//! VCR の RF モジュレータ → テレビのチューナ (アンテナ端子接続) で生じる劣化。
//!
//! コンポジット信号のまま扱う:
//! - スノー: チューナ帯域 (≒4.2 MHz) で制限した白色雑音。同期にも乗るので AFC も揺れる
//! - ゴースト: 反射波 (遅延した減衰コピー)。ライン境界をまたいで前のラインの尻尾も映り込む
//! - ビート妨害: 他チャンネルや電装品からの正弦波混入 (斜めの縞)。周波数がわずかに揺らぐ
//! - 音声ビート: 色副搬送波と音声搬送波 (4.5 MHz) の差 ≒920 kHz の縞。音声で FM 変調されて揺れる
//! - AGC: 受信レベル (とフラッター) で包絡線の大きさが変わり、チューナの AGC がそれを追う。
//!   負変調 (同期先端 = 搬送波 100%、ブランキング 75%) の包絡線に直して利得をかけ、包絡線検波で戻す。
//!   雑音・妨害は受信機の入口で一定なので、弱電界で AGC が利得を上げるほど相対的に増える

use crate::filter::Cascade;
use crate::frame::Field;
use crate::params::{AgcMode, RfParams};
use crate::rng::Rng;
use crate::timing::Standard;
use rayon::prelude::*;

const STREAM_RF: u64 = 11;

/// 包絡線検波の 1 ラインぶんの測定値 (AGC の検波器が見る量)
#[derive(Clone, Copy, Default)]
struct LineStat {
    /// 同期幅程度の窓で平均した包絡線の最大値 (キード AGC: 同期先端)
    keyed: f32,
    /// 包絡線の最大値 (ピーク AGC)
    peak: f32,
    /// 包絡線の平均 (平均値 AGC)
    mean: f32,
}

#[derive(Default)]
pub struct RfSim {
    src: Vec<f32>,
    /// 音声ビートを揺らす擬似音声の状態
    audio_state: f32,
    /// ラインごとの受信レベル (線形、標準 = 1)
    level: Vec<f32>,
    stats: Vec<LineStat>,
    /// ラインごとの AGC 利得 (末尾に次のフィールド先頭の値を 1 つ足す)
    gain: Vec<f32>,
    /// AGC ループの状態 (ln 利得)。フィールドをまたいで続く
    agc_log_gain: f32,
    /// どの方式で初期化したか (方式を切り替えたら定常値から始め直す)
    agc_init: Option<AgcMode>,
}

/// 負変調の包絡線 (同期先端 = 1, ブランキング = 0.75, 搬送波ゼロ = 0)
#[inline(always)]
fn envelope(s: &Standard, v: f32) -> f32 {
    1.0 - 0.25 * (v - s.ire.sync) / (s.ire.blank - s.ire.sync)
}

/// 包絡線検波 (整流するので、過変調で包絡線が負になると白が折り返す)
#[inline(always)]
fn detect(s: &Standard, e: f32) -> f32 {
    s.ire.sync + (1.0 - e.abs()) * 4.0 * (s.ire.blank - s.ire.sync)
}

/// max の 8 割までは素通しで、その上を滑らかに頭打ちにする
#[inline(always)]
fn soft_clip(x: f32, max: f32) -> f32 {
    let k = 0.8 * max;
    if x <= k {
        x
    } else {
        k + (max - k) * ((x - k) / (max - k)).tanh()
    }
}

/// 受信レベルの揺れ [dB]: 飛行機フラッター。ドップラーで周波数がゆっくり変わる
fn flutter(p: &RfParams, t: f64) -> f32 {
    use std::f64::consts::TAU;
    if p.flutter_db == 0.0 {
        return 0.0;
    }
    let f = p.flutter_hz.max(0.0) as f64;
    // 周波数を 7 秒周期で ±30% 揺らした位相 (瞬時周波数を積分したもの)
    let phase = TAU * f * (t - 0.3 * 7.0 / TAU * (TAU * t / 7.0).cos());
    // 反射波との干渉なので落ち込み側が深い
    let x = phase.sin();
    (p.flutter_db as f64 * (0.5 * x - 0.5 * x * x.abs())) as f32
}

impl RfSim {
    pub fn process(&mut self, field: &mut Field, p: &RfParams, seed: u64) {
        self.src.clear();
        self.src.extend_from_slice(&field.data);
        let src = &self.src;
        let s = field.std;
        let (fs, ls) = (s.fs, s.line_samples);
        let lines = field.data.len() / ls;
        let line_time = ls as f64 / fs as f64;
        let sound_beat_hz = s.sound_carrier_hz - s.fsc as f64;

        let delay = (p.ghost_delay_us.max(0.0) * 1e-6 * fs).max(0.0);
        let (d0, dfrac) = (delay.floor() as usize, delay.fract());
        let gain = p.ghost_gain;
        let snow_lp = Cascade::butter_lowpass_at(2, s.video_bw_hz, fs);
        let snow_norm = (0.5 * fs / s.video_bw_hz).sqrt() * p.snow_ire;
        let (index, frame, parity) = (field.index, field.frame, field.parity);

        // 擬似音声: フィールドごとに変わる (声や音楽の抑揚の代わり)
        let mut frng = Rng::derive(seed, index, 0, STREAM_RF + 1);
        self.audio_state = 0.85 * self.audio_state + 0.5 * frng.normal();
        let audio = self.audio_state;

        let interf_w = 2.0 * std::f64::consts::PI * p.interference_mhz as f64 * 1e6 / fs as f64;
        let beat_w = 2.0 * std::f64::consts::PI * sound_beat_hz / fs as f64;

        // 受信レベル: ラインごとに一定とみなす (フラッターはライン周波数よりずっと遅い)
        let agc = p.agc_enabled;
        self.level.clear();
        if agc {
            self.level.extend((0..lines).map(|l| {
                let t = s.stream_line(l, frame, parity) as f64 * line_time;
                db_to_lin(p.signal_db + flutter(p, t))
            }));
        }
        let level = &self.level;
        let overload = db_to_lin(p.overload_db.max(-20.0));
        // キード AGC の窓: 等化パルスにも収まる幅
        let key_w = ((0.8 * s.eq_width) as usize).clamp(4, s.sync_end.max(4));

        // 1 パス目: 受信した信号 (AGC 有効なら包絡線) と、AGC が見る測定値
        let mut stats = std::mem::take(&mut self.stats);
        field
            .data
            .par_chunks_mut(ls)
            .enumerate()
            .map(|(l, out)| {
                let mut rng = Rng::derive(seed, index, l as u64, STREAM_RF);
                let mut lp = snow_lp;
                // 通算サンプル位置 (位相をフィールド・ラインをまたいで連続させる)
                let t0 = s.stream_line(l, frame, parity) as f64 * ls as f64;
                // 音声ビートの位相は音声で FM 変調されるので、ラインごとに揺れる
                let beat_phase = 2.5 * audio as f64 + 0.3 * rng.normal() as f64;
                let base = l * ls;
                // 雑音・妨害は受信機の入口で一定なので、信号に対しては受信レベルに反比例する
                let lv = if agc { level[l] } else { 1.0 };
                let inv = 1.0 / lv;

                for n in 0..ls {
                    let i = base + n;
                    let mut v = src[i];
                    if gain != 0.0 && i > d0 {
                        let a = src[i - d0];
                        let b = src[i - d0 - 1];
                        v += gain * (a + (b - a) * dfrac);
                    }
                    let t = t0 + n as f64;
                    if p.interference_ire > 0.0 {
                        // 周波数がゆっくり揺らぐので縞の傾きが少しずつ変わる
                        v += p.interference_ire * inv * (interf_w * t + 40.0 * (t * 1e-7).sin()).sin() as f32;
                    }
                    if p.sound_beat > 0.0 {
                        let amp = p.sound_beat * 4.0 * (0.6 + 0.4 * audio.abs().min(1.0));
                        v += amp * (beat_w * t + beat_phase).sin() as f32;
                    }
                    if p.snow_ire > 0.0 {
                        v += lp.process(rng.normal()) * snow_norm * inv;
                    }
                    out[n] = if agc {
                        // アンテナ入力の包絡線。強電界ではフロントエンドが飽和して同期先端が潰れる
                        soft_clip(lv * envelope(s, v), overload)
                    } else {
                        v
                    };
                }
                if agc { line_stat(out, key_w) } else { LineStat::default() }
            })
            .collect_into_vec(&mut stats);
        self.stats = stats;
        if !agc {
            return;
        }

        // 2 パス目: AGC ループ (ラインを順に。前のラインまでの測定で今のラインの利得が決まる)
        self.run_agc(s, p, line_time);

        // 3 パス目: IF 増幅 (利得をライン内で補間) → 包絡線検波 → 映像段のクランプ (DC 再生)。
        // クランプはブランキングを基準に戻すので、利得のずれはブランキングを中心にした振幅 (コントラスト・同期の深さ) に出る
        let (g, level) = (&self.gain, &self.level);
        // IF・映像増幅の出力の頭打ち (同期先端が標準の 1.4 倍まで)
        let if_max = 1.4;
        let blank_env = envelope(s, s.ire.blank);
        field.data.par_chunks_mut(ls).enumerate().for_each(|(l, out)| {
            let (g0, g1) = (g[l], g[l + 1]);
            let dg = (g1 - g0) / ls as f32;
            // このラインのブランキングの包絡線 (フロントエンドの圧縮込み)
            let a_blank = soft_clip(level[l] * blank_env, overload);
            for (n, x) in out.iter_mut().enumerate() {
                let gn = g0 + dg * n as f32;
                let e = gn * *x;
                let clamp = s.ire.blank - detect(s, soft_clip(gn * a_blank, if_max));
                *x = detect(s, e.signum() * soft_clip(e.abs(), if_max)) + clamp;
            }
        });
    }

    fn run_agc(&mut self, s: &Standard, p: &RfParams, line_time: f64) {
        let mode = p.agc_mode;
        let offset = p.agc_offset_db / 20.0 * std::f32::consts::LN_10;
        let range = p.agc_range_db.max(0.0) / 20.0 * std::f32::consts::LN_10;
        let tau = (p.agc_time_ms.max(0.01) * 1e-3) as f64;
        let alpha = (1.0 - (-line_time / tau).exp()) as f32;
        // ピーク AGC は速く充電し、ゆっくり放電する
        let alpha_attack = (1.0 - (-line_time * 20.0 / tau).exp()) as f32;
        let reference = agc_reference(s, mode).ln();
        let measure = |st: &LineStat| match mode {
            AgcMode::Keyed | AgcMode::Manual => st.keyed,
            AgcMode::Peak => st.peak,
            AgcMode::Average => st.mean,
        };

        // 初回・方式の切り替え時は、今の受信レベルでの定常値から始める
        if self.agc_init != Some(mode) {
            let m = self.stats.first().map(measure).unwrap_or(1.0).max(1e-4);
            self.agc_log_gain = if mode == AgcMode::Manual { 0.0 } else { (reference - m.ln()).clamp(-range, range) };
            self.agc_init = Some(mode);
        }

        self.gain.clear();
        let mut lg = self.agc_log_gain;
        for st in &self.stats {
            if mode == AgcMode::Manual {
                // 固定利得: 調整ずれだけがかかる
                self.gain.push(offset.exp());
                continue;
            }
            self.gain.push((lg + offset).exp());
            // 検波器が見るのは利得をかけた後の包絡線。基準との比 (対数) で利得を動かす
            let err = reference - (lg + measure(st).max(1e-4).ln());
            let a = if mode == AgcMode::Peak && err < 0.0 { alpha_attack } else { alpha };
            lg = (lg + a * err).clamp(-range, range);
        }
        self.agc_log_gain = lg;
        self.gain.push(if mode == AgcMode::Manual { offset.exp() } else { (lg + offset).exp() });
    }
}

#[inline]
fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// 標準の受信レベル・標準の絵で検波器が見る値 (AGC はこれに合わせる)
fn agc_reference(s: &Standard, mode: AgcMode) -> f32 {
    match mode {
        AgcMode::Keyed | AgcMode::Peak | AgcMode::Manual => 1.0,
        AgcMode::Average => {
            // 同期 + ブランキング + 平均輝度 35% の絵
            let ls = s.line_samples as f32;
            let sync = s.sync_end as f32 / ls;
            let active = s.active_width as f32 / ls;
            let apl = s.ire.black + 0.35 * s.ire.span();
            sync * envelope(s, s.ire.sync)
                + active * envelope(s, apl)
                + (1.0 - sync - active) * envelope(s, s.ire.blank)
        }
    }
}

fn line_stat(e: &[f32], w: usize) -> LineStat {
    let w = w.min(e.len()).max(1);
    let mut acc: f32 = e[..w].iter().sum();
    let mut keyed = acc;
    let (mut peak, mut sum) = (f32::MIN, 0.0f32);
    for (n, &x) in e.iter().enumerate() {
        peak = peak.max(x);
        sum += x;
        if n >= w {
            acc += x - e[n - w];
            keyed = keyed.max(acc);
        }
    }
    LineStat { keyed: keyed / w as f32, peak, mean: sum / e.len() as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::NTSC;

    /// 同期 + ブランキング + 灰色の絵のフィールド
    fn test_field(apl: f32) -> Field {
        let s = &NTSC;
        let mut f = Field::new(s);
        for line in f.data.chunks_mut(s.line_samples) {
            for (n, x) in line.iter_mut().enumerate() {
                *x = if n < s.sync_end {
                    s.ire.sync
                } else if (s.active_start..s.active_end()).contains(&n) {
                    apl
                } else {
                    s.ire.blank
                };
            }
        }
        f
    }

    fn quiet(mode: AgcMode) -> RfParams {
        RfParams {
            enabled: true,
            snow_ire: 0.0,
            ghost_gain: 0.0,
            agc_enabled: true,
            agc_mode: mode,
            ..RfParams::default()
        }
    }

    fn run(sim: &mut RfSim, p: &RfParams, apl: f32, fields: u64) -> Field {
        let mut f = test_field(apl);
        for i in 0..fields {
            f = test_field(apl);
            f.set_index(i);
            sim.process(&mut f, p, 1);
        }
        f
    }

    fn mid(f: &Field) -> f32 {
        let s = f.std;
        f.line(100)[s.active_start + s.active_width / 2]
    }

    #[test]
    fn envelope_roundtrip() {
        let s = &NTSC;
        for v in [-40.0, 0.0, 7.5, 50.0, 100.0] {
            assert!((detect(s, envelope(s, v)) - v).abs() < 1e-3);
        }
    }

    #[test]
    fn keyed_agc_compensates_weak_signal() {
        let mut p = quiet(AgcMode::Keyed);
        p.signal_db = -20.0;
        let f = run(&mut RfSim::default(), &p, 50.0, 4);
        assert!((mid(&f) - 50.0).abs() < 1.0, "{}", mid(&f));
        assert!((f.line(100)[10] - NTSC.ire.sync).abs() < 1.0);
    }

    #[test]
    fn manual_gain_follows_signal_level() {
        let mut p = quiet(AgcMode::Manual);
        p.signal_db = -6.0;
        let f = run(&mut RfSim::default(), &p, 50.0, 2);
        // 包絡線が半分になる → ブランキングを中心にコントラストも同期も半分
        assert!((mid(&f) - 25.0).abs() < 1.0, "{}", mid(&f));
        assert!((f.line(100)[10] + 20.0).abs() < 1.0, "{}", f.line(100)[10]);
        assert!((f.line(100)[NTSC.active_start - 4] - NTSC.ire.blank).abs() < 0.5);
    }

    #[test]
    fn overload_compresses_sync() {
        let mut p = quiet(AgcMode::Keyed);
        p.signal_db = 20.0;
        p.overload_db = 10.0;
        let f = run(&mut RfSim::default(), &p, 50.0, 4);
        // 同期先端が潰れて、AGC が絵の振幅を上げる
        let sync_depth = NTSC.ire.blank - f.line(100)[10];
        assert!(sync_depth < 35.0, "{sync_depth}");
    }

    #[test]
    fn agc_range_limits_weak_signal() {
        let mut p = quiet(AgcMode::Keyed);
        p.signal_db = -30.0;
        p.agc_range_db = 10.0;
        let f = run(&mut RfSim::default(), &p, 50.0, 4);
        assert!(f.line(100)[10] > -10.0, "{}", f.line(100)[10]);
    }

    #[test]
    fn average_agc_depends_on_picture() {
        let p = quiet(AgcMode::Average);
        let dark = run(&mut RfSim::default(), &p, 10.0, 20);
        let bright = run(&mut RfSim::default(), &p, 90.0, 20);
        // 負変調: 暗い絵は包絡線の平均が大きいので利得が下がり、明るい絵は上がる
        assert!(mid(&dark) < 9.0, "{}", mid(&dark));
        assert!(mid(&bright) > 92.0, "{}", mid(&bright));
    }

    #[test]
    fn flutter_modulates_gain_with_slow_agc() {
        let mut p = quiet(AgcMode::Keyed);
        p.flutter_db = 10.0;
        p.flutter_hz = 5.0;
        p.agc_time_ms = 500.0;
        let mut sim = RfSim::default();
        let levels: Vec<f32> = (0..24).map(|i| {
            let mut f = test_field(50.0);
            f.set_index(i);
            sim.process(&mut f, &p, 1);
            mid(&f)
        }).collect();
        let (lo, hi) = levels.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        assert!(hi - lo > 5.0, "{levels:?}");
    }
}
