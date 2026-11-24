//! Progressive reconstruction from the fields produced by the video pipeline.
//!
//! Field history is kept here so every output mode (composite receiver and
//! digital bypass) uses the same temporal deinterlacing implementation.

use crate::frame::RgbaFrame;
use crate::params::{Deinterlace, DeinterlaceParams};
use rayon::prelude::*;
use std::collections::VecDeque;

const HISTORY_FIELDS: usize = 4;

#[derive(Clone)]
struct FieldImage {
    index: u64,
    parity: usize,
    /// Packed RGBA rows, one row per source field line.
    data: Vec<u8>,
}

#[derive(Clone, Copy, Default)]
struct MotionVector {
    /// Offset from the current block into the previous same-parity field.
    dx: i8,
    dy: i8,
    sad: u16,
}

pub struct Deinterlacer {
    width: usize,
    height: usize,
    history: VecDeque<FieldImage>,
}

impl Deinterlacer {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            history: VecDeque::with_capacity(HISTORY_FIELDS),
        }
    }

    /// Complete the progressive output after the current field has been written
    /// into its matching even/odd rows of `frame`.
    pub fn process_field(
        &mut self,
        frame: &mut RgbaFrame,
        index: u64,
        parity: u8,
        mode: Deinterlace,
        params: &DeinterlaceParams,
    ) {
        let (width, height) = (frame.width, frame.height);
        assert_eq!(frame.data.len(), width * height * 4);
        assert_eq!(height % 2, 0);
        let parity = (parity & 1) as usize;
        let field_height = height / 2;
        let row_bytes = width * 4;

        if (self.width, self.height) != (width, height) {
            self.width = width;
            self.height = height;
            self.history.clear();
        }
        if self
            .history
            .back()
            .is_some_and(|last| last.index + 1 != index)
        {
            self.history.clear();
        }

        let mut current = FieldImage {
            index,
            parity,
            data: vec![0; field_height * row_bytes],
        };
        for line in 0..field_height {
            let src_row = 2 * line + parity;
            current.data[line * row_bytes..(line + 1) * row_bytes]
                .copy_from_slice(&frame.data[src_row * row_bytes..(src_row + 1) * row_bytes]);
        }

        let previous = index.checked_sub(1).and_then(|wanted| {
            self.history
                .iter()
                .find(|field| field.index == wanted && field.parity != parity)
        });
        let previous_same = index.checked_sub(2).and_then(|wanted| {
            self.history
                .iter()
                .find(|field| field.index == wanted && field.parity == parity)
        });
        let qtgmc_block_size = params.qtgmc_block_size.clamp(8, 32) as usize;
        let qtgmc_block_height = (qtgmc_block_size / 2).max(4);
        let qtgmc_search_radius = params.qtgmc_search_radius.min(6) as usize;
        let motion = (mode == Deinterlace::Qtgmc).then(|| {
            previous_same.map(|previous| {
                estimate_motion(
                    &current,
                    previous,
                    width,
                    field_height,
                    qtgmc_block_size,
                    qtgmc_block_height,
                    qtgmc_search_radius,
                )
            })
        });
        let motion = motion.flatten();
        let current_data = current.data.as_slice();
        let previous_data = previous.map(|field| field.data.as_slice());
        let previous_same_data = previous_same.map(|field| field.data.as_slice());

        frame
            .data
            .par_chunks_mut(row_bytes)
            .enumerate()
            .for_each(|(row, dst)| {
                let output_parity = row & 1;
                let line = row / 2;
                if output_parity == parity {
                    dst.copy_from_slice(&current_data[line * row_bytes..(line + 1) * row_bytes]);
                    return;
                }

                match mode {
                    Deinterlace::Weave => {
                        weave_line(
                            current_data,
                            previous_data,
                            width,
                            field_height,
                            line,
                            parity,
                            params.weave_temporal_weight_pct,
                            dst,
                        );
                    }
                    Deinterlace::Bob => blend_line(
                        current_data,
                        width,
                        field_height,
                        line,
                        parity,
                        params.bob_blend_pct,
                        dst,
                    ),
                    Deinterlace::Blend => blend_line(
                        current_data,
                        width,
                        field_height,
                        line,
                        parity,
                        params.blend_strength_pct,
                        dst,
                    ),
                    Deinterlace::MotionAdaptiveBlend => {
                        motion_adaptive_line(
                            current_data,
                            previous_data,
                            previous_same_data,
                            width,
                            field_height,
                            line,
                            parity,
                            params.motion_adaptive_threshold,
                            params.motion_adaptive_blend_pct,
                            dst,
                        );
                    }
                    Deinterlace::Yadif => yadif_line(
                        current_data,
                        previous_data,
                        previous_same_data,
                        width,
                        field_height,
                        line,
                        parity,
                        params.yadif_motion_threshold,
                        params.yadif_search_radius,
                        params.yadif_temporal_weight_pct,
                        dst,
                    ),
                    Deinterlace::Qtgmc => qtgmc_line(
                        current_data,
                        previous_data,
                        width,
                        field_height,
                        line,
                        parity,
                        motion.as_deref(),
                        qtgmc_block_size,
                        qtgmc_block_height,
                        params.qtgmc_temporal_weight_pct,
                        params.qtgmc_sharpness_pct,
                        dst,
                    ),
                }
            });

        self.history.push_back(current);
        while self.history.len() > HISTORY_FIELDS {
            self.history.pop_front();
        }
    }
}

fn neighbor_lines(line: usize, parity: usize, field_height: usize) -> (usize, usize) {
    let upper = line.saturating_sub(parity);
    let lower = (line + 1 - parity).min(field_height - 1);
    (upper, lower)
}

fn weave_line(
    current: &[u8],
    previous: Option<&[u8]>,
    width: usize,
    field_height: usize,
    line: usize,
    parity: usize,
    temporal_weight_pct: f32,
    dst: &mut [u8],
) {
    let Some(previous) = previous else {
        blend_line(current, width, field_height, line, parity, 0.0, dst);
        return;
    };
    let row_bytes = width * 4;
    let old_row = &previous[line * row_bytes..(line + 1) * row_bytes];
    let (upper, lower) = neighbor_lines(line, parity, field_height);
    let upper_row = &current[upper * row_bytes..(upper + 1) * row_bytes];
    let lower_row = &current[lower * row_bytes..(lower + 1) * row_bytes];
    let weight = (temporal_weight_pct / 100.0).clamp(0.0, 1.0);
    for x in 0..width {
        let p = x * 4;
        for c in 0..3 {
            let spatial = 0.5 * (upper_row[p + c] as f32 + lower_row[p + c] as f32);
            dst[p + c] = mix(spatial, old_row[p + c] as f32, weight);
        }
        dst[p + 3] = 255;
    }
}

fn blend_line(
    current: &[u8],
    width: usize,
    field_height: usize,
    line: usize,
    parity: usize,
    strength_pct: f32,
    dst: &mut [u8],
) {
    let (upper, lower) = neighbor_lines(line, parity, field_height);
    let row_bytes = width * 4;
    let center_row = &current[line * row_bytes..(line + 1) * row_bytes];
    let a = &current[upper * row_bytes..(upper + 1) * row_bytes];
    let b = &current[lower * row_bytes..(lower + 1) * row_bytes];
    let weight = (strength_pct / 100.0).clamp(0.0, 1.0);
    for x in 0..width {
        let p = x * 4;
        for c in 0..3 {
            let average = 0.5 * (a[p + c] as f32 + b[p + c] as f32);
            dst[p + c] = mix(center_row[p + c] as f32, average, weight);
        }
        dst[p + 3] = 255;
    }
}

fn motion_adaptive_line(
    current: &[u8],
    previous: Option<&[u8]>,
    previous_same: Option<&[u8]>,
    width: usize,
    field_height: usize,
    line: usize,
    parity: usize,
    motion_threshold: f32,
    blend_strength_pct: f32,
    dst: &mut [u8],
) {
    let Some(previous) = previous else {
        blend_line(
            current,
            width,
            field_height,
            line,
            parity,
            blend_strength_pct,
            dst,
        );
        return;
    };
    let row_bytes = width * 4;
    let old_row = &previous[line * row_bytes..(line + 1) * row_bytes];
    let Some(previous_same) = previous_same else {
        blend_line(
            current,
            width,
            field_height,
            line,
            parity,
            blend_strength_pct,
            dst,
        );
        return;
    };
    let same_row = &previous_same[line * row_bytes..(line + 1) * row_bytes];
    let (upper, lower) = neighbor_lines(line, parity, field_height);
    let upper_row = &current[upper * row_bytes..(upper + 1) * row_bytes];
    let lower_row = &current[lower * row_bytes..(lower + 1) * row_bytes];
    let current_row = &current[line * row_bytes..(line + 1) * row_bytes];
    let spatial_weight = (blend_strength_pct / 100.0).clamp(0.0, 1.0);
    let threshold = motion_threshold.clamp(0.0, 255.0);
    for x in 0..width {
        let p = x * 4;
        let in_motion = (luma(current_row, p) - luma(same_row, p)).abs() as f32 > threshold;
        for c in 0..3 {
            if in_motion {
                let average = 0.5 * (upper_row[p + c] as f32 + lower_row[p + c] as f32);
                dst[p + c] = mix(current_row[p + c] as f32, average, spatial_weight);
            } else {
                dst[p + c] = old_row[p + c];
            }
        }
        dst[p + 3] = 255;
    }
}

fn yadif_line(
    current: &[u8],
    previous: Option<&[u8]>,
    previous_same: Option<&[u8]>,
    width: usize,
    field_height: usize,
    line: usize,
    parity: usize,
    motion_threshold: f32,
    search_radius: u32,
    temporal_weight_pct: f32,
    dst: &mut [u8],
) {
    let (upper, lower) = neighbor_lines(line, parity, field_height);
    let row_bytes = width * 4;
    let upper_row = &current[upper * row_bytes..(upper + 1) * row_bytes];
    let lower_row = &current[lower * row_bytes..(lower + 1) * row_bytes];
    let old_row = previous.map(|data| &data[line * row_bytes..(line + 1) * row_bytes]);
    let same_row = previous_same.map(|data| &data[line * row_bytes..(line + 1) * row_bytes]);
    let current_row = &current[line * row_bytes..(line + 1) * row_bytes];
    let radius = search_radius.clamp(1, 8) as isize;
    let threshold = motion_threshold.clamp(0.0, 255.0);
    let temporal_weight = (temporal_weight_pct / 100.0).clamp(0.0, 1.0);

    for x in 0..width {
        let p = x * 4;
        let mut best_dir = 0isize;
        let mut best_score = i32::MAX;
        for dir in -radius..=radius {
            let left = clamp_x(x as isize + dir, width);
            let right = clamp_x(x as isize - dir, width);
            let left2 = clamp_x(x as isize + dir - 1, width);
            let right2 = clamp_x(x as isize - dir + 1, width);
            let score = (luma(upper_row, left * 4) - luma(lower_row, right * 4)).abs()
                + (luma(upper_row, left2 * 4) - luma(lower_row, right2 * 4)).abs();
            if score < best_score {
                best_score = score;
                best_dir = dir;
            }
        }
        let upper_x = clamp_x(x as isize + best_dir, width) * 4;
        let lower_x = clamp_x(x as isize - best_dir, width) * 4;
        let motion = same_row.map_or(255, |old| (luma(current_row, p) - luma(old, p)).abs()) as f32;
        for c in 0..3 {
            let spatial = (upper_row[upper_x + c] as u16 + lower_row[lower_x + c] as u16 + 1) / 2;
            // YADIF keeps temporal samples in static regions and switches to its
            // edge-directed spatial prediction where same-parity fields move.
            dst[p + c] = if motion <= threshold {
                old_row.map_or(spatial as u8, |old| {
                    mix(spatial as f32, old[p + c] as f32, temporal_weight)
                })
            } else {
                spatial as u8
            };
        }
        dst[p + 3] = 255;
    }
}

fn qtgmc_line(
    current: &[u8],
    previous: Option<&[u8]>,
    width: usize,
    field_height: usize,
    line: usize,
    parity: usize,
    motion: Option<&[MotionVector]>,
    block_width: usize,
    block_height: usize,
    temporal_weight_pct: f32,
    sharpness_pct: f32,
    dst: &mut [u8],
) {
    let (upper, lower) = neighbor_lines(line, parity, field_height);
    let row_bytes = width * 4;
    let upper_row = &current[upper * row_bytes..(upper + 1) * row_bytes];
    let lower_row = &current[lower * row_bytes..(lower + 1) * row_bytes];
    let Some(previous) = previous else {
        blend_line(current, width, field_height, line, parity, 100.0, dst);
        return;
    };
    let columns = width.div_ceil(block_width);

    for x in 0..width {
        let p = x * 4;
        let mut best_dir = 0isize;
        let mut best_score = i32::MAX;
        for dir in -3isize..=3 {
            let left = clamp_x(x as isize + dir, width);
            let right = clamp_x(x as isize - dir, width);
            let score = (luma(upper_row, left * 4) - luma(lower_row, right * 4)).abs();
            if score < best_score {
                best_score = score;
                best_dir = dir;
            }
        }
        let upper_x = clamp_x(x as isize + best_dir, width) * 4;
        let lower_x = clamp_x(x as isize - best_dir, width) * 4;
        let block = (line / block_height) * columns + x / block_width;
        let vector = motion.and_then(|vectors| vectors.get(block)).copied();
        let vector = vector.unwrap_or_default();
        let source_x = x as f32 + vector.dx as f32 * 0.5;
        let source_y = line as f32 + vector.dy as f32 * 0.5;
        let temporal_weight = if motion.is_some() {
            (temporal_weight_pct / 100.0 * (1.0 - vector.sad as f32 / 160.0)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let sharpness = (sharpness_pct / 100.0).clamp(0.0, 1.0);
        for c in 0..3 {
            let spatial = 0.5 * (upper_row[upper_x + c] as f32 + lower_row[lower_x + c] as f32);
            // Half of the two-field motion vector aligns the prior opposite
            // field with this field; edge-directed spatial detail fills gaps.
            let temporal = sample_channel(previous, width, field_height, source_x, source_y, c);
            let sharpened = spatial
                + sharpness * (spatial - 0.5 * (upper_row[p + c] as f32 + lower_row[p + c] as f32));
            dst[p + c] = (sharpened * (1.0 - temporal_weight) + temporal * temporal_weight + 0.5)
                .clamp(0.0, 255.0) as u8;
        }
        dst[p + 3] = 255;
    }
}

fn estimate_motion(
    current: &FieldImage,
    previous: &FieldImage,
    width: usize,
    height: usize,
    block_width: usize,
    block_height: usize,
    search_radius: usize,
) -> Vec<MotionVector> {
    let columns = width.div_ceil(block_width);
    let rows = height.div_ceil(block_height);
    (0..columns * rows)
        .into_par_iter()
        .map(|block| {
            let bx = block % columns;
            let by = block / columns;
            let x0 = bx * block_width;
            let y0 = by * block_height;
            let x1 = (x0 + block_width).min(width);
            let y1 = (y0 + block_height).min(height);
            let mut best = MotionVector {
                sad: u16::MAX,
                ..MotionVector::default()
            };
            let vertical_radius = (search_radius + 1) / 2;
            for dy in -(vertical_radius as isize)..=(vertical_radius as isize) {
                for dx in -(search_radius as isize)..=(search_radius as isize) {
                    let mut sad = 0u32;
                    let mut count = 0u32;
                    for y in (y0..y1).step_by(2) {
                        let py = y as isize + dy;
                        if !(0..height as isize).contains(&py) {
                            continue;
                        }
                        for x in (x0..x1).step_by(2) {
                            let px = x as isize + dx;
                            if !(0..width as isize).contains(&px) {
                                continue;
                            }
                            let a = pixel_luma(&current.data, width, x, y);
                            let b = pixel_luma(&previous.data, width, px as usize, py as usize);
                            sad += (a - b).unsigned_abs();
                            count += 1;
                        }
                    }
                    let score = if count == 0 {
                        u16::MAX
                    } else {
                        (sad / count).min(u16::MAX as u32) as u16
                    };
                    if score < best.sad {
                        best = MotionVector {
                            dx: dx as i8,
                            dy: dy as i8,
                            sad: score,
                        };
                    }
                }
            }
            best
        })
        .collect()
}

fn pixel_luma(data: &[u8], width: usize, x: usize, y: usize) -> i32 {
    let p = (y * width + x) * 4;
    luma(data, p)
}

fn luma(row: &[u8], p: usize) -> i32 {
    (77 * row[p] as i32 + 150 * row[p + 1] as i32 + 29 * row[p + 2] as i32) >> 8
}

fn mix(a: f32, b: f32, weight: f32) -> u8 {
    (a + (b - a) * weight + 0.5).clamp(0.0, 255.0) as u8
}

fn clamp_x(x: isize, width: usize) -> usize {
    x.clamp(0, width as isize - 1) as usize
}

fn sample_channel(data: &[u8], width: usize, height: usize, x: f32, y: f32, channel: usize) -> f32 {
    let x = x.clamp(0.0, (width - 1) as f32);
    let y = y.clamp(0.0, (height - 1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let wx = x - x0 as f32;
    let wy = y - y0 as f32;
    let at = |xx: usize, yy: usize| data[(yy * width + xx) * 4 + channel] as f32;
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * wx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * wx;
    top + (bottom - top) * wy
}
