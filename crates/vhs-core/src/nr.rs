//! VCR playback noise reduction in the recovered Y/C domain.
//!
//! YNR models the recursive 1H-correlated luminance filter used by VHS HQ.
//! Digital 3DNR keeps a one-frame history and blends only where a smoothed
//! frame difference indicates little motion. This is a practical approximation
//! of period 3DNR silicon, not a reconstruction of its Hadamard/vector logic.

/// VCR playback noise-reduction state. The history is one frame old, represented
/// by the field with the same parity (two fields earlier).
#[derive(Default)]
struct FieldHistory {
    index: Option<u64>,
    y: Vec<f32>,
    u: Vec<f32>,
    v: Vec<f32>,
}

#[derive(Default)]
pub(crate) struct NoiseReduction {
    /// One slot per interlaced field parity; each is revisited once per frame.
    history: [FieldHistory; 2],
    motion: Vec<f32>,
}

impl NoiseReduction {
    pub(crate) fn reset(&mut self) {
        for history in &mut self.history {
            history.index = None;
        }
    }

    pub(crate) fn process(
        &mut self,
        field_index: u64,
        width: usize,
        height: usize,
        y: &mut [f32],
        u: &mut [f32],
        v: &mut [f32],
        ynr_strength: f32,
        dnr_strength: f32,
        motion_threshold_ire: f32,
        adc_dac_bits: u32,
    ) {
        let len = width * height;
        debug_assert_eq!(y.len(), len);
        debug_assert_eq!(u.len(), len);
        debug_assert_eq!(v.len(), len);

        let ynr = ynr_strength.clamp(0.0, 1.0);
        let strength = dnr_strength.clamp(0.0, 1.0);
        let bits = adc_dac_bits.min(10);
        if strength > 0.0 {
            for history in &mut self.history {
                if history.y.len() != len {
                    history.y.resize(len, 0.0);
                    history.u.resize(len, 0.0);
                    history.v.resize(len, 0.0);
                    history.index = None;
                }
            }
        } else {
            self.reset();
        }
        if bits > 0 && (ynr > 0.0 || strength > 0.0) {
            quantize_yuv(y, u, v, bits);
        }

        apply_ynr(y, width, height, ynr);

        let slot = (field_index & 1) as usize;
        let previous = &self.history[slot];
        let has_previous_frame = field_index >= 2 && previous.index == Some(field_index - 2);
        if strength > 0.0 && has_previous_frame {
            self.motion.resize(len, 0.0);
            let threshold = motion_threshold_ire.max(0.1);
            for row in 0..height {
                for x in 0..width {
                    let i = row * width + x;
                    let delta = |j: usize| y[j] - previous.y[j];
                    let center = delta(i);
                    let left = delta(row * width + x.saturating_sub(1));
                    let right = delta(row * width + (x + 1).min(width - 1));
                    let above = delta(row.saturating_sub(1) * width + x);
                    let below = delta((row + 1).min(height - 1) * width + x);
                    // A cross-shaped low-pass on the difference suppresses isolated
                    // tape noise in the motion detector while retaining moving edges.
                    let motion = ((4.0 * center + left + right + above + below) / 8.0).abs();
                    self.motion[i] = 1.0 - smoothstep(threshold, threshold * 3.0, motion);
                }
            }

            for i in 0..len {
                let amount = strength * self.motion[i];
                y[i] += (previous.y[i] - y[i]) * amount;
                u[i] += (previous.u[i] - u[i]) * amount;
                v[i] += (previous.v[i] - v[i]) * amount;
            }
        }

        if bits > 0 && (ynr > 0.0 || strength > 0.0) {
            quantize_yuv(y, u, v, bits);
        }

        // Keep the filtered result for a recursive frame-cyclic response. A seek
        // or skipped field naturally disables NR until a matching field returns.
        if strength > 0.0 {
            let history = &mut self.history[slot];
            history.y.copy_from_slice(y);
            history.u.copy_from_slice(u);
            history.v.copy_from_slice(v);
            history.index = Some(field_index);
        }
    }
}

/// Model the shared-bit-depth ADC and DAC around the digital NR block. Y is
/// carried in IRE relative to black; U/V use white-level-normalized units.
fn quantize_yuv(y: &mut [f32], u: &mut [f32], v: &mut [f32], bits: u32) {
    const Y_MIN_IRE: f32 = -40.0;
    const Y_MAX_IRE: f32 = 120.0;
    const CHROMA_PEAK: f32 = 0.5;
    let levels = ((1u32 << bits) - 1) as f32;
    let y_step = (Y_MAX_IRE - Y_MIN_IRE) / levels;
    let c_step = (2.0 * CHROMA_PEAK) / levels;
    for sample in y {
        let code = ((*sample - Y_MIN_IRE) / y_step).round().clamp(0.0, levels);
        *sample = Y_MIN_IRE + code * y_step;
    }
    for sample in u.iter_mut().chain(v.iter_mut()) {
        let code = ((*sample + CHROMA_PEAK) / c_step)
            .round()
            .clamp(0.0, levels);
        *sample = -CHROMA_PEAK + code * c_step;
    }
}

/// Recursive line-correlated low-pass with a limiter around vertical edges.
/// Signal values are in IRE relative to black; the limiter protects picture
/// detail while allowing the low-level noise on flat regions to average out.
fn apply_ynr(y: &mut [f32], width: usize, height: usize, strength: f32) {
    if strength <= 0.0 || width == 0 || height < 2 {
        return;
    }

    const CORING_IRE: f32 = 2.0;
    const EDGE_IRE: f32 = 12.0;
    const LIMIT_IRE: f32 = 6.0;

    for x in 0..width {
        let mut previous_input = y[x];
        let mut previous_output = previous_input;
        for row in 1..height {
            let i = row * width + x;
            let current = y[i];
            let edge = (current - previous_input).abs();
            let correlation = 1.0 - smoothstep(CORING_IRE, EDGE_IRE, edge);
            let correction = (previous_output - current).clamp(-LIMIT_IRE, LIMIT_IRE);
            let output = current + strength * correlation * correction;
            y[i] = output;
            previous_input = current;
            previous_output = output;
        }
    }
}

#[inline]
fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo).max(f32::EPSILON)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
