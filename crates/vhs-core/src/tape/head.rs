//! 再生ヘッドの損失 (Chowdhury, DAFx-19 の式 (13)。元は Bertram / Kadis)。
//!
//! 正弦波の記録 (波長 λ = v / f、波数 k = 2π / λ = 2π f / v) を再生したときの振幅は
//!
//! ```text
//! V = V0 · e^{-k d} · (1 - e^{-k δ}) / (k δ) · sin(k g / 2) / (k g / 2)
//!          スペーシング   厚み (記録深さ)         ギャップ
//! ```
//!
//! - d: ヘッドとテープの間隔。汚れ・テープの浮き・摩耗で広がる。54.6 d/λ [dB] と波長に対して急に効く
//! - δ: 磁化している層の厚み (実効の記録深さ)
//! - g: 再生ヘッドのギャップ長。λ = g で最初のヌルになる
//! - v: ヘッドとテープの相対速度 (映像はドラムの書き込み速度、音声はテープ速度)
//!
//! 実機の再生アンプは設計どおりの損失をイコライザで補っているので、ここでは損失そのものではなく
//! 「EQ が想定した状態」との比を掛ける (映像: 余分なスペーシングだけ、音声: SP 用の EQ のまま速度を落としたぶん)。

use std::f32::consts::{PI, TAU};

/// 式 (13) の損失 (振幅比、符号付き: ギャップヌルを越えると極性が反転する)。
/// 長さはすべて [m]、速度は [m/s]。`depth` = 0 は厚み損失なし (EQ で補われている) として扱う
pub fn playback_loss(f_hz: f32, speed: f32, spacing: f32, gap: f32, depth: f32) -> f32 {
    let k = TAU * f_hz.abs() / speed.max(1e-6);
    let spacing_loss = (-k * spacing.max(0.0)).exp();
    let kd = k * depth.max(0.0);
    let thickness_loss = if kd < 1e-4 { 1.0 - 0.5 * kd } else { (1.0 - (-kd).exp()) / kd };
    let x = 0.5 * k * gap.max(0.0);
    let gap_loss = if x.abs() < 1e-4 { 1.0 } else { x.sin() / x };
    spacing_loss * thickness_loss * gap_loss
}

/// スペーシング損失 e^{-k d} の振幅比 (1 以下)
#[inline]
pub fn spacing_loss(f_hz: f32, speed: f32, spacing: f32) -> f32 {
    (-TAU * f_hz.abs() * spacing.max(0.0) / speed.max(1e-6)).exp()
}

/// スペーシング損失 e^{-|ω| τ} を零位相の FIR にする (τ はサンプル単位、`taps` の長さは奇数)。
///
/// 空間ではローレンツ型 (コーシー分布) のぼかしで、離散時間の逆変換は閉じた形になる:
/// h[n] = τ (1 - (-1)^n e^{-π τ}) / (π (τ² + n²))。裾が 1/n² と重いので打ち切ったぶんは直流で正規化する
pub fn spacing_kernel(tau: f32, taps: &mut [f32]) {
    debug_assert!(taps.len() % 2 == 1);
    let half = (taps.len() / 2) as isize;
    let alias = (-PI * tau).exp();
    for (i, h) in taps.iter_mut().enumerate() {
        let n = i as isize - half;
        let sign = if n % 2 == 0 { 1.0 } else { -1.0 };
        *h = tau * (1.0 - sign * alias) / (PI * (tau * tau + (n * n) as f32));
    }
    let sum: f32 = taps.iter().sum();
    taps.iter_mut().for_each(|h| *h /= sum);
}

/// 振幅特性 `gain(f)` (実数、零位相) から長さ `taps` (奇数) の直線位相 FIR を周波数サンプリング + Hann 窓で作る。
/// 遅れは (taps - 1) / 2 サンプル
pub fn design_fir(taps: usize, fs: f32, gain: impl Fn(f32) -> f32) -> Vec<f32> {
    let taps = taps | 1;
    let half = (taps / 2) as f32;
    // 周波数の格子は細かめ (タップ数の 8 倍) に取り、窓で切る
    let grid = taps * 8;
    let g: Vec<f32> = (0..=grid / 2).map(|k| gain(k as f32 * fs / grid as f32)).collect();
    let h: Vec<f32> = (0..taps)
        .map(|i| {
            let n = i as f32 - half;
            let mut acc = g[0];
            for (k, gk) in g.iter().enumerate().skip(1) {
                let w = if 2 * k == grid { 1.0 } else { 2.0 };
                acc += w * gk * (TAU * k as f32 * n / grid as f32).cos();
            }
            let window = 0.5 + 0.5 * (PI * n / (half + 1.0)).cos();
            acc / grid as f32 * window
        })
        .collect();
    // 直流では正規化しない: e^{-k|f|} は f = 0 で折れていて、窓でなまった直流に合わせると全体がずれる
    h
}

/// 1 チャンネル分の FIR (リングバッファ)
#[derive(Clone)]
pub struct Fir {
    taps: Vec<f32>,
    hist: Vec<f32>,
    pos: usize,
}

impl Fir {
    pub fn new(taps: Vec<f32>) -> Self {
        let n = taps.len();
        Self { taps, hist: vec![0.0; n], pos: 0 }
    }

    /// 遅れ [サンプル] (直線位相なので (長さ - 1) / 2)
    pub fn delay(&self) -> usize {
        self.taps.len() / 2
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let n = self.taps.len();
        self.hist[self.pos] = x;
        let mut acc = 0.0;
        // hist[pos] が最新。taps は対称なので向きは気にしなくてよい
        let (newer, older) = self.hist.split_at(self.pos + 1);
        let mut t = self.taps.iter();
        for (v, h) in newer.iter().rev().chain(older.iter().rev()).zip(&mut t) {
            acc += v * h;
        }
        self.pos = (self.pos + 1) % n;
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 正弦波を FIR に通したときの振幅
    fn response(taps: &[f32], fs: f32, f: f32) -> f32 {
        let mut fir = Fir::new(taps.to_vec());
        let n = 4000;
        let mut peak = 0f32;
        for i in 0..n {
            let y = fir.process((TAU * f * i as f32 / fs).sin());
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn spacing_rule_of_thumb() {
        // 54.6 d/λ [dB]: λ = 1.32 µm (4.4 MHz @ 5.8 m/s), d = 0.5 µm → 約 20.7 dB
        let a = spacing_loss(4.4e6, 5.8, 0.5e-6);
        let db = -20.0 * a.log10();
        assert!((db - 54.6 * 0.5 / (5.8 / 4.4)).abs() < 0.2, "{db}");
        assert!((playback_loss(4.4e6, 5.8, 0.5e-6, 0.0, 0.0) - a).abs() < 1e-6);
    }

    #[test]
    fn gap_null_at_wavelength_equal_gap() {
        // v = 11.1 mm/s, g = 1 µm → 11.1 kHz でヌル
        assert!(playback_loss(11.1e3, 11.1e-3, 0.0, 1e-6, 0.0).abs() < 1e-3);
        assert!(playback_loss(5e3, 11.1e-3, 0.0, 1e-6, 0.0) > 0.5);
    }

    #[test]
    fn spacing_kernel_matches_exponential() {
        let fs = 57.27e6;
        let (d, v) = (0.3e-6f32, 5.8f32);
        let tau = d / v * fs;
        let mut taps = vec![0f32; 2 * (16.0 * tau).ceil() as usize + 1];
        spacing_kernel(tau, &mut taps);
        for f in [2e6f32, 4e6, 6e6] {
            let got = response(&taps, fs, f);
            let want = spacing_loss(f, v, d);
            assert!((got / want - 1.0).abs() < 0.06, "{f}: {got} vs {want}");
        }
    }

    #[test]
    fn designed_fir_follows_gain() {
        let fs = 48000.0;
        let gain = |f: f32| playback_loss(f, 11.1e-3, 0.3e-6, 1e-6, 0.0);
        let taps = design_fir(255, fs, gain);
        for f in [500.0f32, 3000.0, 6000.0] {
            let (got, want) = (response(&taps, fs, f), gain(f).abs());
            assert!((got - want).abs() < 0.03, "{f}: {got} vs {want}");
        }
    }
}
