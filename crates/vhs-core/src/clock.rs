//! 受像機の発振器の周波数誤差と揺れ ([`ClockParams`]) をライン単位で発生させる。
//!
//! - オフセット: 一定の周波数のずれ
//! - ドリフト: 相関時間 τ の 1 次の不規則過程 (Ornstein–Uhlenbeck)。ラインごとに更新する
//! - ワウ・フラッタ: 正弦波の周波数変調。位相は通算ライン番号から決まるのでシークしても連続している
//! - ジッタ: ラインごとに独立な時間のずれ (周波数ではなく位相に乗る)
//!
//! 乱数は通算ライン番号と発振器ごとの番号から派生させるので、同じ信号を通せば同じ揺れになる。

use crate::params::ClockParams;
use crate::rng::Rng;

/// 発振器を区別する乱数ストリームの番号
pub(crate) const CLOCK_CHROMA: u64 = 1;
pub(crate) const CLOCK_HORIZONTAL: u64 = 2;
pub(crate) const CLOCK_VERTICAL: u64 = 3;

const SEED: u64 = 0xC10C_4D21_F7E5_0001;

/// あるラインでの発振器の状態
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ClockSample {
    /// 公称周波数に対する瞬時の周波数誤差 (比。1e-6 = 1 ppm)
    pub freq_err: f64,
    /// このラインの時間ジッタ [s]
    pub jitter_s: f64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ClockSim {
    /// ドリフトの今の値 [ppm]
    drift: f64,
    /// 直前に進めた通算ライン (途切れたらドリフトを引き直す)
    last: Option<u64>,
}

impl ClockSim {
    /// 通算ライン `sl` での周波数誤差とジッタ。ラインは 1 本ずつ順に呼ぶ
    pub fn step(&mut self, p: &ClockParams, sl: u64, line_time: f64, id: u64) -> ClockSample {
        use std::f64::consts::TAU;
        if p.is_ideal() {
            self.last = None;
            return ClockSample::default();
        }
        let mut rng = Rng::derive(SEED, id, sl, 0);
        let sigma = p.drift_ppm.max(0.0) as f64;
        if sigma > 0.0 {
            let n = rng.normal() as f64;
            if self.last.is_some_and(|l| l + 1 == sl) {
                let a = (-line_time / (p.drift_time_s.max(0.01) as f64)).exp();
                self.drift = a * self.drift + (1.0 - a * a).sqrt() * sigma * n;
            } else if self.last != Some(sl) {
                // 最初・シーク直後: 定常分布から引く
                self.drift = sigma * n;
            }
        } else {
            self.drift = 0.0;
        }
        self.last = Some(sl);

        let t = sl as f64 * line_time;
        let wow = p.wow_ppm as f64 * (TAU * p.wow_hz.max(0.0) as f64 * t).sin();
        // フラッタはワウと同期しないよう初期位相をずらす
        let flutter = p.flutter_ppm as f64 * (TAU * p.flutter_hz.max(0.0) as f64 * t + 1.3).sin();
        let freq_err = (p.offset_ppm as f64 + self.drift + wow + flutter) * 1e-6;
        let jitter_s = if p.jitter_ns > 0.0 { rng.normal() as f64 * p.jitter_ns as f64 * 1e-9 } else { 0.0 };
        ClockSample { freq_err, jitter_s }
    }

    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LT: f64 = 63.555e-6;

    #[test]
    fn ideal_clock_is_zero() {
        let mut c = ClockSim::default();
        let p = ClockParams::default();
        for sl in 0..100 {
            assert_eq!(c.step(&p, sl, LT, 1), ClockSample::default());
        }
    }

    #[test]
    fn offset_and_wow_add_up() {
        let mut c = ClockSim::default();
        let p = ClockParams { offset_ppm: 10.0, wow_ppm: 5.0, wow_hz: 1.0, ..Default::default() };
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        // 1 秒ぶん
        for sl in 0..15734 {
            let e = c.step(&p, sl, LT, 1).freq_err * 1e6;
            lo = lo.min(e);
            hi = hi.max(e);
        }
        assert!((lo - 5.0).abs() < 0.01 && (hi - 15.0).abs() < 0.01, "{lo} {hi}");
    }

    #[test]
    fn drift_and_jitter_have_requested_rms() {
        let mut c = ClockSim::default();
        let p = ClockParams { drift_ppm: 3.0, drift_time_s: 0.01, jitter_ns: 20.0, ..Default::default() };
        let n = 200_000u64;
        let (mut d2, mut j2) = (0.0, 0.0);
        for sl in 0..n {
            let s = c.step(&p, sl, LT, 2);
            d2 += (s.freq_err * 1e6).powi(2);
            j2 += (s.jitter_s * 1e9).powi(2);
        }
        let (d, j) = ((d2 / n as f64).sqrt(), (j2 / n as f64).sqrt());
        assert!((d - 3.0).abs() < 0.4, "drift rms {d}");
        assert!((j - 20.0).abs() < 1.0, "jitter rms {j}");
    }

    #[test]
    fn same_lines_give_same_samples() {
        let p = ClockParams { drift_ppm: 3.0, jitter_ns: 20.0, flutter_ppm: 2.0, ..Default::default() };
        let run = || {
            let mut c = ClockSim::default();
            (1000..1100).map(|sl| c.step(&p, sl, LT, 3)).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}
