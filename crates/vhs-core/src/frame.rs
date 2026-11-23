//! 画像バッファとコンポジット信号のフィールドバッファ。

use crate::timing::{FIELD_LINES, LINE_SAMPLES};
use rayon::prelude::*;

/// RGB24 (R, G, B の順) の画像
#[derive(Clone, Debug)]
pub struct RgbFrame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl RgbFrame {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, data: vec![0; width * height * 3] }
    }

    pub fn from_raw(width: usize, height: usize, data: Vec<u8>) -> Option<Self> {
        (data.len() == width * height * 3).then_some(Self { width, height, data })
    }

    /// バイリニア補間でリサイズ (縮小時は簡易的に 2x2 平均)。
    pub fn resized(&self, width: usize, height: usize) -> RgbFrame {
        if width == self.width && height == self.height {
            return self.clone();
        }
        let mut out = RgbFrame::new(width, height);
        let sx = self.width as f32 / width as f32;
        let sy = self.height as f32 / height as f32;
        let (sw, sh) = (self.width, self.height);
        out.data.par_chunks_mut(width * 3).enumerate().for_each(|(y, row)| {
            let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (sh - 1) as f32);
            let y0 = fy as usize;
            let y1 = (y0 + 1).min(sh - 1);
            let wy = fy - y0 as f32;
            for x in 0..width {
                let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (sw - 1) as f32);
                let x0 = fx as usize;
                let x1 = (x0 + 1).min(sw - 1);
                let wx = fx - x0 as f32;
                for c in 0..3 {
                    let p = |xx: usize, yy: usize| self.data[(yy * sw + xx) * 3 + c] as f32;
                    let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * wx;
                    let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * wx;
                    row[x * 3 + c] = (top + (bot - top) * wy + 0.5) as u8;
                }
            }
        });
        out
    }
}

/// RGBA32 の画像 (Canvas の ImageData にそのまま渡せる形式)
#[derive(Clone, Debug)]
pub struct RgbaFrame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl RgbaFrame {
    pub fn new(width: usize, height: usize) -> Self {
        let mut data = vec![0; width * height * 4];
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Self { width, height, data }
    }
}

/// 1 フィールド分のコンポジット信号 (FIELD_LINES × LINE_SAMPLES, 単位 IRE)
#[derive(Clone, Debug)]
pub struct Field {
    pub data: Vec<f32>,
    /// 0 = 第 1 (偶数ライン) フィールド, 1 = 第 2 (奇数ライン) フィールド
    pub parity: u8,
    /// フレーム番号
    pub frame: u64,
    /// 通算フィールド番号
    pub index: u64,
}

impl Default for Field {
    fn default() -> Self {
        Self { data: vec![0.0; FIELD_LINES * LINE_SAMPLES], parity: 0, frame: 0, index: 0 }
    }
}

impl Field {
    pub fn set_index(&mut self, index: u64) {
        self.index = index;
        self.parity = (index & 1) as u8;
        self.frame = index / 2;
    }

    pub fn line(&self, l: usize) -> &[f32] {
        &self.data[l * LINE_SAMPLES..(l + 1) * LINE_SAMPLES]
    }

    pub fn line_mut(&mut self, l: usize) -> &mut [f32] {
        &mut self.data[l * LINE_SAMPLES..(l + 1) * LINE_SAMPLES]
    }
}
