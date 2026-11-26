//! 音声の劣化: テープの音声トラック → 再生機 → キャプチャ機器。
//!
//! 映像と違って信号そのもの (サンプル列) を順に処理する。ステレオのインターリーブ f32 を受け取る。
//!
//! リニア (ノーマル) トラック:
//!   モノラル化 → 録音レベル (磁気飽和。映像と同じ磁性体モデル `tape.magnetic_model`) → 帯域制限 (低域カット・高域カット・ヘッドバンプ)
//!   → ワウフラッタ (変調遅延線によるピッチの揺れ) → テープヒス → ドロップアウト (こもり・レベル低下)
//!   → 再生ヘッドの損失 (テープ速度・スペーシング・ギャップ。SP 用の再生 EQ で補いきれないぶん、[`crate::tape::head`])
//! Hi-Fi トラック:
//!   ステレオ・広帯域 → ワウフラッタ (弱め) → コンパンダによるノイズの浮き沈み
//!   → ヘッド切替バズ (59.94 Hz の周期的な欠落とスパイク。トラッキングずれで悪化) → FM 欠落
//! 周辺機器:
//!   電源ハム (グラウンドループ) → AGC (ポンピング) → ADC (サンプリング周波数・量子化) → モノラル化

use crate::filter::{Biquad, Cascade};
use crate::params::{AudioMode, AudioParams, Params};
use crate::rng::Rng;
use crate::tape::head::{Fir, design_fir, playback_loss};
use crate::tape::magnetic::MagneticTransfer;
use crate::timing::TvStandard;
use std::f32::consts::TAU;

const DELAY_LEN: usize = 1 << 15;
/// 再生ヘッドの損失の FIR の長さ
const HEAD_TAPS: usize = 255;
/// 再生 EQ が想定している音声ヘッドとテープの間隔 [m]
const NOMINAL_SPACING: f32 = 0.2e-6;

/// リニアトラックの SP のテープ速度 [m/s]
fn sp_speed(standard: TvStandard) -> f32 {
    match standard {
        TvStandard::Ntsc => 33.35e-3,
        TvStandard::Pal => 23.39e-3,
    }
}

/// 再生ヘッドの損失のうち、SP 用の再生 EQ で補われないぶん (FIR)。補うべきものがなければ None。
/// 厚み損失は速度ごとに EQ の時定数で補われるので含めない (式 (13) の δ = 0)
fn head_loss_fir(p: &AudioParams, standard: TvStandard, fs: f32) -> Option<Fir> {
    let extra = p.head_spacing_um.max(0.0) * 1e-6;
    let ratio = p.tape_speed.clamp(0.05, 4.0);
    if extra <= 0.0 && (ratio - 1.0).abs() < 1e-4 {
        return None;
    }
    let v_ref = sp_speed(standard);
    let v = v_ref * ratio;
    let gap = p.head_gap_um.max(0.0) * 1e-6;
    let gain = |f: f32| {
        let eq = playback_loss(f, v_ref, NOMINAL_SPACING, gap, 0.0);
        playback_loss(f, v, NOMINAL_SPACING + extra, gap, 0.0) / eq.max(0.05)
    };
    Some(Fir::new(design_fir(HEAD_TAPS, fs, gain)))
}

fn db(v: f32) -> f32 {
    if v <= -89.9 { 0.0 } else { 10f32.powf(v / 20.0) }
}

#[derive(Clone)]
struct Channel {
    band: Cascade,
    bump: Biquad,
    hiss_lp: Cascade,
    delay: Vec<f32>,
    /// ドロップアウト中のこもり (1 次 LPF の状態)
    dull: f32,
    adc_pre: Cascade,
    adc_post: Cascade,
    held: f32,
    /// 再生ヘッドの損失 (リニアトラックで速度・スペーシングが SP 標準と違うとき)
    head: Option<Fir>,
}

pub struct AudioProcessor {
    p: AudioParams,
    /// リニアトラックの磁性体の伝達特性 (フルスケール = 1 の尺度)
    mag: MagneticTransfer,
    fs: f32,
    /// 映像のフィールド周波数 [Hz] (Hi-Fi のヘッド切替の周期)
    field_rate: f32,
    rng: Rng,
    ch: [Channel; 2],
    t: u64,
    write: usize,
    base_delay: f32,
    /// 再生ヘッドの損失の FIR の遅れ [サンプル]
    head_delay: f32,
    wow_amp: f32,
    flutter_amp: [f32; 3],
    flutter_phase: [f32; 3],
    wow_phase: f32,
    buzz: f32,
    /// 次のヘッド切替までのサンプル数
    switch_left: f32,
    glitch_left: u32,
    glitch_sign: f32,
    /// ドロップアウト: (残りサンプル, 全長, 深さ)
    dropout: (u32, u32, f32),
    agc_env: f32,
    agc_gain: f32,
    comp_env: f32,
    hold_phase: f32,
}

impl AudioProcessor {
    pub fn new(params: &Params, sample_rate: u32) -> Self {
        let p = params.audio.clone();
        let fs = sample_rate as f32;
        let field_rate = params.tv().field_rate;
        let linear = p.mode == AudioMode::Linear;
        let band = if linear {
            let lo = Cascade::butter_highpass_at(2, p.low_cut_hz.max(5.0), fs);
            let hi = Cascade::butter_lowpass_at(4, p.high_cut_hz.max(200.0), fs);
            // 2 つの縦続を 1 つにまとめる (2 + 4 次 = 3 セクション)
            let mut v = Vec::new();
            v.extend(sections(&lo));
            v.extend(sections(&hi));
            Cascade::new(&v)
        } else {
            Cascade::new(&[Biquad::highpass_at(15.0, 0.707, fs), Biquad::lowpass_at(20000.0, 0.707, fs)])
        };
        // ヘッドバンプ: 固定ヘッドの低域のうねり
        let bump = if linear { Biquad::peaking_at(p.low_cut_hz.max(5.0) * 1.6, 1.2, 3.0, fs) } else { Biquad::IDENTITY };
        let hiss_lp = Cascade::butter_lowpass_at(2, if linear { p.high_cut_hz * 1.2 } else { 18000.0 }, fs);
        let sr = p.sample_rate_hz.clamp(2000.0, fs);
        let adc_pre = Cascade::butter_lowpass_at(2, 0.45 * sr, fs);
        let adc_post = Cascade::butter_lowpass_at(4, 0.45 * sr, fs);
        let chan = Channel {
            band,
            bump,
            hiss_lp,
            delay: vec![0.0; DELAY_LEN],
            dull: 0.0,
            adc_pre,
            adc_post,
            held: 0.0,
            head: if linear { head_loss_fir(&p, params.standard, fs) } else { None },
        };
        let head_delay = chan.head.as_ref().map_or(0, Fir::delay) as f32;

        // ワウ (≒0.6 Hz) とフラッタ (キャプスタン・ピンチローラ由来の数 Hz〜十数 Hz) の遅延振幅
        // 速度偏差 pct [%] の正弦波 → 遅延振幅 A = pct / 100 / (2π f)
        let speed_scale = if linear { 1.0 } else { 0.35 };
        let wow_amp = p.wow_pct.max(0.0) / 100.0 * speed_scale / (TAU * 0.6) * fs;
        let fl_f = [6.3f32, 9.7, 13.1];
        let fl_w = [0.6f32, 0.3, 0.1];
        let fl_amp = std::array::from_fn(|i| p.flutter_pct.max(0.0) / 100.0 * speed_scale * fl_w[i] / (TAU * fl_f[i]) * fs);
        let base_delay = (wow_amp + fl_amp.iter().sum::<f32>()) * 1.5 + 0.002 * fs;
        let mut rng = Rng::new(params.seed ^ 0xA0D10);
        let flutter_phase = std::array::from_fn(|_| rng.uniform() * TAU);

        let buzz = if linear { 0.0 } else { p.hifi_buzz.max(0.0) + 1.5 * params.transport.tracking_error.clamp(0.0, 1.0) };

        Self {
            fs,
            field_rate,
            rng,
            ch: [chan.clone(), chan],
            t: 0,
            write: 0,
            base_delay,
            head_delay,
            wow_amp,
            flutter_amp: fl_amp,
            flutter_phase,
            wow_phase: 0.0,
            buzz,
            switch_left: fs / field_rate,
            glitch_left: 0,
            glitch_sign: 1.0,
            dropout: (0, 1, 0.0),
            agc_env: 0.1,
            agc_gain: 1.0,
            comp_env: 0.0,
            hold_phase: 1.0,
            mag: MagneticTransfer::audio(params.tape.magnetic_model, params.tape.magnetic_media, p.bias_level),
            p,
        }
    }

    /// ワウフラッタの遅延線による遅れ [サンプル]。書き出し時に頭を詰めるのに使う
    pub fn latency(&self) -> usize {
        (self.base_delay + self.head_delay).round() as usize
    }

    /// インターリーブのステレオ f32 (−1..1) をその場で処理する
    pub fn process(&mut self, buf: &mut [f32]) {
        let p = &self.p;
        let fs = self.fs;
        let linear = p.mode == AudioMode::Linear;
        let drive = 0.5 * db(p.drive_db).max(1e-3);
        let mag = self.mag.clone();
        let drive_norm = 1.0 / mag.transfer(drive).max(1e-3);
        let hiss = if linear { db(p.hiss_db) } else { db(p.hiss_db - 25.0) };
        // 帯域制限した白色雑音の rms を戻す係数
        let hiss_norm = hiss * if linear { (0.5 * fs / (p.high_cut_hz * 1.2)).sqrt() } else { 1.2 };
        let hum = db(p.hum_db);
        let agc = p.agc.clamp(0.0, 1.0);
        let (atk, rel) = (1.0 - (-1.0 / (0.005 * fs)).exp(), 1.0 - (-1.0 / (0.4 * fs)).exp());
        let sr = p.sample_rate_hz.clamp(2000.0, fs);
        let resample = sr < fs * 0.99;
        let levels = 2f32.powi(p.bits.clamp(2, 24) as i32 - 1);
        let quantize = p.bits < 24;
        let dropout_prob = p.dropout_rate.max(0.0) / fs;
        let field_period = fs / self.field_rate;
        let glitch_len = (0.0006 * fs) as u32;

        for frame in buf.chunks_exact_mut(2) {
            let t = self.t as f32 / fs;
            self.t += 1;

            // ---- 入力・録音 ----
            let (mut l, mut r) = (frame[0], frame[1]);
            if linear {
                let m = 0.5 * (l + r);
                l = m;
                r = m;
            }
            if linear {
                l = mag.transfer(l * drive) * drive_norm;
                r = mag.transfer(r * drive) * drive_norm;
            }

            // ---- テープ速度の揺れ (遅延変調) ----
            self.wow_phase += TAU * (0.6 + 0.15 * (t * 0.13).sin()) / fs;
            let mut d = self.base_delay + self.wow_amp * self.wow_phase.sin();
            for k in 0..3 {
                let f = [6.3f32, 9.7, 13.1][k];
                d += self.flutter_amp[k] * (TAU * f * t + self.flutter_phase[k]).sin();
            }
            let w = self.write;
            self.write = (w + 1) & (DELAY_LEN - 1);

            // ---- ドロップアウト ----
            if self.dropout.0 == 0 && self.rng.uniform() < dropout_prob {
                let len = if linear { self.rng.exponential(0.08) } else { self.rng.exponential(0.006) };
                let n = ((len + 0.005) * fs) as u32;
                self.dropout = (n, n, 0.3 + 0.6 * self.rng.uniform());
            }
            let mut do_gain = 1.0;
            let mut do_dull = 0.0;
            if self.dropout.0 > 0 {
                let (left, total, depth) = self.dropout;
                let x = 1.0 - left as f32 / total as f32;
                let env = (std::f32::consts::PI * x).sin();
                if linear {
                    do_gain = 1.0 - depth * env;
                    do_dull = env * depth;
                } else {
                    do_gain = 1.0 - env.min(1.0) * depth.max(0.8);
                }
                self.dropout.0 -= 1;
            }

            // ---- Hi-Fi ヘッド切替 ----
            let mut glitch = 0.0;
            let mut glitch_mute = 1.0;
            if !linear && self.buzz > 0.0 {
                self.switch_left -= 1.0;
                if self.switch_left <= 0.0 {
                    self.switch_left += field_period;
                    self.glitch_left = glitch_len;
                    self.glitch_sign = -self.glitch_sign;
                }
                if self.glitch_left > 0 {
                    let x = self.glitch_left as f32 / glitch_len.max(1) as f32;
                    glitch = self.glitch_sign * self.buzz * 0.04 * x * x;
                    glitch_mute = 1.0 - (self.buzz * 0.6).min(0.95) * x;
                    self.glitch_left -= 1;
                }
            }

            // コンパンダの包絡線 (Hi-Fi のノイズは信号に合わせて浮き沈みする)
            let lvl = 0.5 * (l.abs() + r.abs());
            self.comp_env += (lvl - self.comp_env) * if lvl > self.comp_env { atk } else { rel * 4.0 };
            let noise_mod = if linear { 1.0 } else { 0.3 + 6.0 * self.comp_env };

            let hum_v = if hum > 0.0 {
                let ph = TAU * p.hum_hz * t;
                hum * (ph.sin() + 0.5 * (2.0 * ph).sin() + 0.3 * (3.0 * ph).sin() + 0.15 * (5.0 * ph).sin())
            } else {
                0.0
            };

            let mut out = [l, r];
            for (c, o) in out.iter_mut().enumerate() {
                let ch = &mut self.ch[c];
                // 帯域制限は記録時 (遅延線の前)
                let x = ch.bump.process(ch.band.process(*o));
                ch.delay[w] = x;
                let mut y = read_hermite(&ch.delay, w, d);
                // ドロップアウトでヘッドとテープが離れると高域が抜ける
                let a = 1.0 - 0.95 * do_dull;
                ch.dull += (y - ch.dull) * a;
                y = ch.dull * do_gain * glitch_mute + glitch;
                if hiss > 0.0 {
                    y += ch.hiss_lp.process(self.rng.normal()) * hiss_norm * noise_mod;
                }
                // 再生ヘッド: テープから出る信号 (ヒスを含む) が速度・スペーシング・ギャップで削られる
                if let Some(head) = ch.head.as_mut() {
                    y = head.process(y);
                }
                y += hum_v;
                *o = y;
            }

            // ---- AGC (キャプチャ機器 / VCR の自動レベル調整) ----
            if agc > 0.0 {
                let lvl = 0.5 * (out[0].abs() + out[1].abs());
                self.agc_env += (lvl - self.agc_env) * if lvl > self.agc_env { atk } else { rel };
                let want = (0.25 / self.agc_env.max(1e-4)).clamp(0.3, 10.0).powf(agc);
                // ゲインはゆっくり動かす (速いとひずむ)
                self.agc_gain += (want - self.agc_gain) * (1.0 / (0.05 * fs));
                out[0] *= self.agc_gain;
                out[1] *= self.agc_gain;
            }

            // ---- ADC ----
            if resample {
                self.hold_phase += sr / fs;
                let take = self.hold_phase >= 1.0;
                if take {
                    self.hold_phase -= 1.0;
                }
                for (c, o) in out.iter_mut().enumerate() {
                    let ch = &mut self.ch[c];
                    let pre = ch.adc_pre.process(*o);
                    if take {
                        ch.held = pre;
                    }
                    *o = ch.adc_post.process(ch.held);
                }
            }
            if quantize {
                for o in out.iter_mut() {
                    *o = (o.clamp(-1.0, 1.0) * levels).round() / levels;
                }
            }
            if p.mono {
                let m = 0.5 * (out[0] + out[1]);
                out = [m, m];
            }
            frame[0] = out[0].clamp(-1.0, 1.0);
            frame[1] = out[1].clamp(-1.0, 1.0);
        }
    }
}

fn sections(c: &Cascade) -> Vec<Biquad> {
    c.sections().to_vec()
}

/// 遅延線から `d` サンプル前を 3 次エルミート補間で読む
#[inline(always)]
fn read_hermite(buf: &[f32], w: usize, d: f32) -> f32 {
    let mask = DELAY_LEN - 1;
    let di = d.floor();
    let f = d - di;
    let i = (w + DELAY_LEN - di as usize) & mask;
    // i が d サンプル前、さらに古い方向が i-1
    let xm1 = buf[(i + 1) & mask];
    let x0 = buf[i];
    let x1 = buf[(i + mask) & mask];
    let x2 = buf[(i + 2 * mask) & mask];
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * f + c2) * f + c1) * f + x0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(fs: f32, f: f32, n: usize, amp: f32) -> Vec<f32> {
        let mut v = vec![0f32; n * 2];
        for i in 0..n {
            let s = amp * (TAU * f * i as f32 / fs).sin();
            v[2 * i] = s;
            v[2 * i + 1] = s;
        }
        v
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt()
    }

    #[test]
    fn linear_track_cuts_high_frequencies() {
        let mut p = Params::default();
        p.audio.dropout_rate = 0.0;
        p.audio.hiss_db = -90.0;
        let fs = 48000.0;
        let mut lo = sine(fs, 1000.0, 48000, 0.3);
        let mut hi = sine(fs, 16000.0, 48000, 0.3);
        AudioProcessor::new(&p, 48000).process(&mut lo);
        AudioProcessor::new(&p, 48000).process(&mut hi);
        let (a, b) = (rms(&lo[20000..]), rms(&hi[20000..]));
        assert!(a > 0.15, "1 kHz should pass: {a}");
        assert!(b < 0.03, "16 kHz should be cut: {b}");
    }

    /// SP 用の EQ のまま 3 倍モードにすると、スペーシングとギャップの損失で高域が落ちる (低域はほぼそのまま)。
    /// SP の標準状態では損失の FIR を掛けない
    #[test]
    fn slower_tape_loses_treble() {
        let mut p = Params::default();
        p.audio.dropout_rate = 0.0;
        p.audio.hiss_db = -90.0;
        p.audio.wow_pct = 0.0;
        p.audio.flutter_pct = 0.0;
        assert!(AudioProcessor::new(&p, 48000).ch[0].head.is_none());
        let level = |p: &Params, f: f32| {
            let mut v = sine(48000.0, f, 48000, 0.2);
            AudioProcessor::new(p, 48000).process(&mut v);
            rms(&v[20000..])
        };
        let (sp_lo, sp_hi) = (level(&p, 300.0), level(&p, 6000.0));
        p.audio.tape_speed = 1.0 / 3.0;
        let (ep_lo, ep_hi) = (level(&p, 300.0), level(&p, 6000.0));
        assert!((ep_lo / sp_lo - 1.0).abs() < 0.05, "300 Hz: {sp_lo} -> {ep_lo}");
        let drop_db = 20.0 * (sp_hi / ep_hi).log10();
        assert!(drop_db > 6.0 && drop_db < 20.0, "6 kHz drop {drop_db} dB");
        // 余分なスペーシングでも高域が落ちる
        p.audio.tape_speed = 1.0;
        p.audio.head_spacing_um = 0.5;
        assert!(level(&p, 6000.0) < 0.7 * sp_hi);
    }

    #[test]
    fn silence_gets_hiss_and_stays_bounded() {
        let p = Params::default();
        let mut buf = vec![0f32; 96000];
        AudioProcessor::new(&p, 48000).process(&mut buf);
        let r = rms(&buf[10000..]);
        assert!(r > 1e-4 && r < 0.05, "hiss rms {r}");
        assert!(buf.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    }
}
