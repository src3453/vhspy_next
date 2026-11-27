//! IIR フィルタ (RBJ biquad とその縦続接続)。
//!
//! アナログ回路の因果的な振る舞い (群遅延・リンギング) を再現するため、
//! ゼロ位相フィルタではなく素直な因果 IIR を使う。

use crate::simd::Lane;
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

    /// 周波数 f での群遅延 [サンプル]。伝達関数の零点の真上 (バンドパスの直流など) では意味を持たない
    pub fn group_delay(&self, f: f32, fs: f32) -> f32 {
        // 多項式 Σ p_n z^-n の群遅延 = Re(Σ n p_n e^{-jωn} / Σ p_n e^{-jωn})
        let poly = |p: [f32; 3]| {
            let w = 2.0 * PI * f / fs;
            let (mut nr, mut ni, mut dr, mut di) = (0f32, 0f32, 0f32, 0f32);
            for (n, &c) in p.iter().enumerate() {
                let (s, co) = (w * n as f32).sin_cos();
                dr += c * co;
                di -= c * s;
                nr += n as f32 * c * co;
                ni -= n as f32 * c * s;
            }
            (nr * dr + ni * di) / (dr * dr + di * di).max(1e-20)
        };
        poly([self.b0, self.b1, self.b2]) - poly([1.0, self.a1, self.a2])
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

    /// 周波数 f での群遅延 [サンプル] (各段の和)
    pub fn group_delay(&self, f: f32, fs: f32) -> f32 {
        self.sections().iter().map(|b| b.group_delay(f, fs)).sum()
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

    /// 同じ係数・同じ状態のフィルタをレーン型 `V` で回す版 (縦方向 SIMD 用。各レーンが 1 本のライン)
    #[inline(always)]
    pub fn lanes<V: Lane>(&self) -> CascadeX<V> {
        let mut c = CascadeX { s: [BiquadX::from(&Biquad::IDENTITY); MAX_SECTIONS], n: self.n };
        for (d, s) in c.s.iter_mut().zip(&self.s[..self.n]) {
            *d = BiquadX::from(s);
        }
        c
    }
}

/// [`Biquad`] のレーン型版。係数は全レーン共通、状態はレーンごと。
/// 演算の順序は [`Biquad::process`] と同じなので、各レーンの結果はスカラー版とビット単位で一致する
#[derive(Clone, Copy, Debug)]
pub struct BiquadX<V> {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: V,
    z2: V,
}

impl<V: Lane> BiquadX<V> {
    #[inline(always)]
    fn from(b: &Biquad) -> Self {
        Self { b0: b.b0, b1: b.b1, b2: b.b2, a1: b.a1, a2: b.a2, z1: V::splat(b.z1), z2: V::splat(b.z2) }
    }

    #[inline(always)]
    pub fn process(&mut self, x: V) -> V {
        let y = V::splat(self.b0) * x + self.z1;
        self.z1 = V::splat(self.b1) * x - V::splat(self.a1) * y + self.z2;
        self.z2 = V::splat(self.b2) * x - V::splat(self.a2) * y;
        y
    }
}

/// [`Cascade`] のレーン型版 ([`Cascade::lanes`] で作る)
#[derive(Clone, Copy, Debug)]
pub struct CascadeX<V> {
    s: [BiquadX<V>; MAX_SECTIONS],
    n: usize,
}

impl<V: Lane> CascadeX<V> {
    #[inline(always)]
    pub fn process(&mut self, x: V) -> V {
        let mut y = x;
        for s in &mut self.s[..self.n] {
            y = s.process(y);
        }
        y
    }
}

/// 小数サンプル遅延の補間の片側タップ数 (Lanczos 窓の半径)
const SHIFT_TAPS: isize = 4;

/// 小数サンプル遅延 (`dst[n] = src(n - shift)`)。範囲外は `fill`。
/// Lanczos (a = 4) 窓付き sinc で補間する。線形補間だと半サンプルずらしたとき 4fsc の副搬送波 (fs/4) が
/// cos 45° ≒ 0.707 倍に落ち、バーストも色も薄くなる (ACC なしで彩度が下がる)。整数ずらしならそのままコピーになる
pub fn fractional_shift(src: &[f32], dst: &mut [f32], shift: f32, fill: f32) {
    let len = src.len() as isize;
    let i0 = shift.floor();
    let frac = shift - i0;
    let i0 = i0 as isize;
    let get = |k: isize| if (0..len).contains(&k) { src[k as usize] } else { fill };
    if frac == 0.0 {
        for (n, d) in dst.iter_mut().enumerate() {
            *d = get(n as isize - i0);
        }
        return;
    }
    // 出力 n の位置は src の (n - i0) - frac。src[n - i0 - j] (j = 1 - a ..= a) の重み w[j] = L(frac - j)
    let lanczos = |t: f32| {
        let a = SHIFT_TAPS as f32;
        if t.abs() < 1e-6 {
            1.0
        } else if t.abs() >= a {
            0.0
        } else {
            let pt = std::f32::consts::PI * t;
            a * pt.sin() * (pt / a).sin() / (pt * pt)
        }
    };
    let mut w = [0f32; 2 * SHIFT_TAPS as usize];
    for (i, wi) in w.iter_mut().enumerate() {
        *wi = lanczos(frac - (i as isize + 1 - SHIFT_TAPS) as f32);
    }
    let sum: f32 = w.iter().sum();
    w.iter_mut().for_each(|wi| *wi /= sum);
    // 全タップが src の中に収まる出力の範囲 (内側) と、範囲外を fill で埋める端に分ける
    let dlen = dst.len() as isize;
    let lo = (i0 + SHIFT_TAPS).clamp(0, dlen) as usize;
    let hi = (len + i0 - SHIFT_TAPS).clamp(lo as isize, dlen) as usize;
    for n in (0..lo).chain(hi..dst.len()) {
        let k = n as isize - i0;
        let mut acc = 0.0;
        for (i, wi) in w.iter().enumerate() {
            acc += wi * get(k - (i as isize + 1 - SHIFT_TAPS));
        }
        dst[n] = acc;
    }
    if lo < hi {
        // 出力 n の窓は src[n - i0 - a ..= n - i0 + a - 1] を逆順に重み付けしたもの
        let base = (lo as isize - i0 - SHIFT_TAPS) as usize;
        let src = &src[base..base + (hi - lo) + 2 * SHIFT_TAPS as usize - 1];
        if crate::simd::x8() {
            shift_inner_x8(src, &mut dst[lo..hi], &w);
        } else {
            shift_inner(src, &mut dst[lo..hi], &w);
        }
    }
}

/// [`fractional_shift`] の内側: `dst[m] = Σ_i w[i]·src[m + 2a - 1 - i]` (i の順に足す)
#[inline(always)]
fn shift_inner_impl(src: &[f32], dst: &mut [f32], w: &[f32; 2 * SHIFT_TAPS as usize]) {
    const T: usize = 2 * SHIFT_TAPS as usize;
    let n = dst.len();
    let src = &src[..n + T - 1];
    for (m, d) in dst.iter_mut().enumerate() {
        let s = &src[m..m + T];
        let mut acc = 0.0;
        for i in 0..T {
            acc += w[i] * s[T - 1 - i];
        }
        *d = acc;
    }
}

fn shift_inner(src: &[f32], dst: &mut [f32], w: &[f32; 2 * SHIFT_TAPS as usize]) {
    shift_inner_impl(src, dst, w)
}

crate::simd::x8_kernel! {
    /// [`shift_inner`] を AVX2 でコンパイルしたもの (出力サンプルの方向に自動ベクトル化される。各出力の足し算の順序は同じ)
    fn shift_inner_x8(src: &[f32], dst: &mut [f32], w: &[f32; 2 * SHIFT_TAPS as usize]) {
        shift_inner_impl(src, dst, w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::NTSC;
    const FS: f32 = NTSC.fs;

    /// 半サンプルずらしても 4fsc の副搬送波 (fs/4) の振幅がほぼ保たれ、整数ずらしはそのままコピーになる
    #[test]
    fn fractional_shift_keeps_subcarrier() {
        let src: Vec<f32> = (0..400).map(|n| (PI / 2.0 * n as f32 + 0.3).sin()).collect();
        let mut dst = vec![0f32; 400];
        for shift in [0.5f32, 0.25, -0.5, 1.75] {
            fractional_shift(&src, &mut dst, shift, 0.0);
            // fs/4 なので隣り合う 2 サンプルが直交成分になる: 振幅 = sqrt(x[n]² + x[n+1]²)
            for k in 100..300 {
                let amp = (dst[k] * dst[k] + dst[k + 1] * dst[k + 1]).sqrt();
                assert!((amp - 1.0).abs() < 0.03, "shift {shift}: {amp} at {k}");
            }
            let want = (PI / 2.0 * (200.0 - shift) + 0.3).sin();
            assert!((dst[200] - want).abs() < 0.03, "shift {shift}: {} vs {want}", dst[200]);
        }
        fractional_shift(&src, &mut dst, 3.0, -1.0);
        assert_eq!(&dst[3..], &src[..397]);
        assert!(dst[..3].iter().all(|&v| v == -1.0));
    }

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
