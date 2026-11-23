//! VCR の RF モジュレータ → テレビのチューナ (アンテナ端子接続) で生じる劣化。
//!
//! コンポジット信号のまま扱う:
//! - スノー: チューナ帯域 (≒4.2 MHz) で制限した白色雑音。同期にも乗るので AFC も揺れる
//! - ゴースト: 反射波 (遅延した減衰コピー)。ライン境界をまたいで前のラインの尻尾も映り込む
//! - ビート妨害: 他チャンネルや電装品からの正弦波混入 (斜めの縞)。周波数がわずかに揺らぐ
//! - 音声ビート: 色副搬送波と音声搬送波 (4.5 MHz) の差 ≒920 kHz の縞。音声で FM 変調されて揺れる

use crate::filter::Cascade;
use crate::frame::Field;
use crate::params::RfParams;
use crate::rng::Rng;
use rayon::prelude::*;

const STREAM_RF: u64 = 11;

#[derive(Default)]
pub struct RfSim {
    src: Vec<f32>,
    /// 音声ビートを揺らす擬似音声の状態
    audio_state: f32,
}

impl RfSim {
    pub fn process(&mut self, field: &mut Field, p: &RfParams, seed: u64) {
        self.src.clear();
        self.src.extend_from_slice(&field.data);
        let src = &self.src;
        let s = field.std;
        let (fs, ls) = (s.fs, s.line_samples);
        // 位相をフィールド・ラインをまたいで連続させるのに総ライン数 (帰線区間込み) を使う
        let lines_per_field = s.total_lines_per_field;
        let sound_beat_hz = s.sound_carrier_hz - s.fsc as f64;

        let delay = (p.ghost_delay_us.max(0.0) * 1e-6 * fs).max(0.0);
        let (d0, dfrac) = (delay.floor() as usize, delay.fract());
        let gain = p.ghost_gain;
        let snow_lp = Cascade::butter_lowpass_at(2, s.video_bw_hz, fs);
        let snow_norm = (0.5 * fs / s.video_bw_hz).sqrt() * p.snow_ire;
        let index = field.index;

        // 擬似音声: フィールドごとに変わる (声や音楽の抑揚の代わり)
        let mut frng = Rng::derive(seed, index, 0, STREAM_RF + 1);
        self.audio_state = 0.85 * self.audio_state + 0.5 * frng.normal();
        let audio = self.audio_state;

        let interf_w = 2.0 * std::f64::consts::PI * p.interference_mhz as f64 * 1e6 / fs as f64;
        let beat_w = 2.0 * std::f64::consts::PI * sound_beat_hz / fs as f64;

        field.data.par_chunks_mut(ls).enumerate().for_each(|(l, out)| {
            let mut rng = Rng::derive(seed, index, l as u64, STREAM_RF);
            let mut lp = snow_lp;
            // 通算サンプル位置 (位相をフィールド・ラインをまたいで連続させる)
            let t0 = ((index as f64 * lines_per_field) + l as f64) * ls as f64;
            // 音声ビートの位相は音声で FM 変調されるので、ラインごとに揺れる
            let beat_phase = 2.5 * audio as f64 + 0.3 * rng.normal() as f64;
            let base = l * ls;

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
                    v += p.interference_ire * (interf_w * t + 40.0 * (t * 1e-7).sin()).sin() as f32;
                }
                if p.sound_beat > 0.0 {
                    let amp = p.sound_beat * 4.0 * (0.6 + 0.4 * audio.abs().min(1.0));
                    v += amp * (beat_w * t + beat_phase).sin() as f32;
                }
                if p.snow_ire > 0.0 {
                    v += lp.process(rng.normal()) * snow_norm;
                }
                out[n] = v;
            }
        });
    }
}
