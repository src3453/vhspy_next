//! IIR フィルタ (RBJ biquad とその縦続接続)。
//!
//! アナログ回路の因果的な振る舞い (群遅延・リンギング) を再現するため、
//! ゼロ位相フィルタではなく素直な因果 IIR を使う。

use std::f32::consts::PI;

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Default for Biquad {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Biquad {
    pub const IDENTITY: Biquad = Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: 0.0, z2: 0.0 };

    fn norm(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z1: 0.0, z2: 0.0 }
    }

    fn w0(fc: f32, fs: f32) -> (f32, f32) {
        let fc = fc.clamp(1e-6 * fs, 0.45 * fs);
        let w = 2.0 * PI * fc / fs;
        (w.cos(), w.sin())
    }

    /// ノッチ (帯域除去)
    pub fn notch_at(fc: f32, q: f32, fs: f32) -> Self {
        let (c, s) = Self::w0(fc, fs);
        let alpha = s / (2.0 * q);
        Self::norm(1.0, -2.0 * c, 1.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    // ---- サンプリング周波数 fs を指定する (映像は Standard::fs、音声はその fs) ----

    pub fn lowpass_at(fc: f32, q: f32, fs: f32) -> Self {
        let (c, s) = Self::w0(fc, fs);
        let alpha = s / (2.0 * q);
        Self::norm((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn highpass_at(fc: f32, q: f32, fs: f32) -> Self {
        let (c, s) = Self::w0(fc, fs);
        let alpha = s / (2.0 * q);
        Self::norm((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    pub fn bandpass_at(fc: f32, q: f32, fs: f32) -> Self {
        let (c, s) = Self::w0(fc, fs);
        let alpha = s / (2.0 * q);
        Self::norm(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
    }

    /// ピーキング EQ
    pub fn peaking_at(fc: f32, q: f32, gain_db: f32, fs: f32) -> Self {
        if gain_db.abs() < 1e-4 {
            return Self::IDENTITY;
        }
        let (c, s) = Self::w0(fc, fs);
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = s / (2.0 * q);
        Self::norm(1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a)
    }

    pub fn high_shelf_at(fc: f32, gain_db: f32, fs: f32) -> Self {
        if gain_db.abs() < 1e-4 {
            return Self::IDENTITY;
        }
        let (c, s) = Self::w0(fc, fs);
        let a = 10f32.powf(gain_db / 40.0);
        let alpha = s / 2.0 * std::f32::consts::SQRT_2;
        let sa = 2.0 * a.sqrt() * alpha;
        Self::norm(
            a * ((a + 1.0) + (a - 1.0) * c + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - sa),
            (a + 1.0) - (a - 1.0) * c + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - sa,
        )
    }

    #[inline(always)]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

const MAX_SECTIONS: usize = 4;

/// biquad の縦続接続 (最大 8 次)。`Copy` なのでラインごとに安価に複製できる。
#[derive(Clone, Copy, Debug)]
pub struct Cascade {
    s: [Biquad; MAX_SECTIONS],
    n: usize,
}

impl Default for Cascade {
    fn default() -> Self {
        Self { s: [Biquad::IDENTITY; MAX_SECTIONS], n: 0 }
    }
}

impl Cascade {
    pub fn new(sections: &[Biquad]) -> Self {
        let mut c = Self::default();
        for b in sections.iter().take(MAX_SECTIONS) {
            c.s[c.n] = *b;
            c.n += 1;
        }
        c
    }

    fn butter_qs(order: usize) -> impl Iterator<Item = f32> {
        let pairs = (order.max(2) / 2).min(MAX_SECTIONS);
        let n = 2 * pairs;
        (1..=pairs).map(move |k| {
            let theta = PI * (2 * k - 1) as f32 / (2 * n) as f32;
            1.0 / (2.0 * theta.cos())
        })
    }

    /// Butterworth ローパス (偶数次、最大 8 次)
    pub fn butter_lowpass_at(order: usize, fc: f32, fs: f32) -> Self {
        let v: Vec<Biquad> = Self::butter_qs(order).map(|q| Biquad::lowpass_at(fc, q, fs)).collect();
        Self::new(&v)
    }

    /// Butterworth ハイパス (偶数次、最大 8 次)
    pub fn butter_highpass_at(order: usize, fc: f32, fs: f32) -> Self {
        let v: Vec<Biquad> = Self::butter_qs(order).map(|q| Biquad::highpass_at(fc, q, fs)).collect();
        Self::new(&v)
    }

    #[inline(always)]
    pub fn process(&mut self, x: f32) -> f32 {
        let mut y = x;
        for s in &mut self.s[..self.n] {
            y = s.process(y);
        }
        y
    }

    pub fn sections(&self) -> &[Biquad] {
        &self.s[..self.n]
    }

    pub fn process_slice(&mut self, buf: &mut [f32]) {
        for v in buf {
            *v = self.process(*v);
        }
    }

    pub fn reset(&mut self) {
        for s in &mut self.s[..self.n] {
            s.reset();
        }
    }
}

/// 線形補間による小数サンプル遅延。範囲外は `fill`。
pub fn fractional_shift(src: &[f32], dst: &mut [f32], shift: f32, fill: f32) {
    let len = src.len() as isize;
    let i0 = shift.floor();
    let frac = shift - i0;
    let i0 = i0 as isize;
    for (n, d) in dst.iter_mut().enumerate() {
        let k = n as isize - i0;
        let a = if (0..len).contains(&k) { src[k as usize] } else { fill };
        let b = if (0..len).contains(&(k - 1)) { src[(k - 1) as usize] } else { fill };
        *d = a + (b - a) * frac;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::NTSC;
    const FS: f32 = NTSC.fs;

    fn gain_at(mut f: Cascade, freq: f32) -> f32 {
        let n = 20000;
        let mut peak = 0f32;
        for i in 0..n {
            let x = (2.0 * PI * freq * i as f32 / FS).sin();
            let y = f.process(x);
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn butterworth_lowpass_response() {
        let f = Cascade::butter_lowpass_at(4, 1.0e6, FS);
        assert!((gain_at(f, 50e3) - 1.0).abs() < 0.02);
        assert!((gain_at(f, 1.0e6) - 0.7071).abs() < 0.03);
        assert!(gain_at(f, 4.0e6) < 0.01);
    }

    #[test]
    fn shelf_inverse_is_flat() {
        let f = Cascade::new(&[Biquad::high_shelf_at(0.5e6, 10.0, FS), Biquad::high_shelf_at(0.5e6, -10.0, FS)]);
        for fr in [10e3, 0.5e6, 2e6, 5e6] {
            assert!((gain_at(f, fr) - 1.0).abs() < 0.02, "{fr}");
        }
    }
}
