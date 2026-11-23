//! VHS 方式の磁気テープ記録・再生モデル。
//!
//! 処理は実機の信号経路に沿って 3 段階に分かれる。
//!
//! 1. 記録/再生の信号経路 (ライン並列)
//!    - 輝度: Y/C 分離 → 帯域制限 → エンファシス → ホワイトクリップ → FM 伝送 (ノイズ・過変調による
//!      ベアディング) → ディエンファシス → ピーキング
//!    - 色: バンドパス → 低域変換 (複素包絡線で等価的に扱う) → 狭帯域化 → 磁気飽和 → AM/位相ノイズ
//!      → 再生時の高域変換
//! 2. 媒体の欠陥: ドロップアウトとドロップアウト補償 (DOC)
//! 3. 走行系: ジッタ、ワウフラッタ、スキュー、ヘッド切替、トラッキングずれ (時間軸誤差として
//!    ライン全体 = 同期・バーストごとずらす。補正は受像機の AFC/バースト同期に任せる)

pub mod magnetic;

use crate::filter::{Biquad, Cascade, fractional_shift};
use crate::frame::Field;
use crate::params::{TapeParams, TransportParams};
use crate::rng::Rng;
use crate::timing::*;
use magnetic::MagneticTransfer;
use rayon::prelude::*;

const STREAM_PATH: u64 = 1;
const STREAM_DROPOUT: u64 = 2;
const STREAM_TRANSPORT: u64 = 3;

pub struct TapeSim {
    y_buf: Vec<f32>,
    c_buf: Vec<f32>,
    rf_level: Vec<f32>,
    offsets: Vec<f32>,
    /// ジッタのランダムウォーク状態 (フィールドをまたいで連続)
    jitter_state: f32,
    /// 画面下端ヘッド切替位置のゆらぎ状態
    switch_wander: f32,
}

impl Default for TapeSim {
    fn default() -> Self {
        let n = FIELD_LINES * LINE_SAMPLES;
        Self {
            y_buf: vec![0.0; n],
            c_buf: vec![0.0; n],
            rf_level: vec![1.0; FIELD_LINES],
            offsets: vec![0.0; FIELD_LINES],
            jitter_state: 0.0,
            switch_wander: 0.0,
        }
    }
}

impl TapeSim {
    pub fn process(&mut self, field: &mut Field, tp: &TapeParams, tr: &TransportParams, seed: u64) {
        self.compute_rf_level(field, tr);
        self.signal_path(field, tp, seed);
        self.dropouts(field, tp, seed);
        self.transport(field, tr, seed);
    }

    /// トラッキングずれによるライン毎の再生 RF レベル (1 = 正常)
    fn compute_rf_level(&mut self, field: &Field, tr: &TransportParams) {
        let te = tr.tracking_error.clamp(0.0, 1.0);
        let band_h = 24.0;
        let period = FIELD_LINES as f32 + 2.0 * band_h;
        let center = (field.index as f32 * tr.tracking_speed).rem_euclid(period) - band_h;
        for (l, rf) in self.rf_level.iter_mut().enumerate() {
            let d = (l as f32 - center) / band_h;
            let bump = (-d * d * 2.0).exp();
            // トラッキングがずれると全体の RF もいくらか落ちる
            *rf = (1.0 - te * (0.3 + 0.7 * bump)).max(0.02);
        }
    }

    fn signal_path(&mut self, field: &Field, tp: &TapeParams, seed: u64) {
        let rec_notch = Biquad::notch(FSC, 1.2);
        let rec_lp = Cascade::butter_lowpass(4, tp.luma_bw_mhz * 1e6);
        let pre = Biquad::high_shelf(tp.emphasis_mhz * 1e6, tp.emphasis_db);
        let de = Biquad::high_shelf(tp.emphasis_mhz * 1e6, -tp.emphasis_db);
        let noise_lp = Cascade::butter_lowpass(2, tp.luma_bw_mhz * 1e6);
        let sharp = Biquad::high_shelf(2.0e6, tp.sharpness_db);
        let c_bp = Cascade::new(&[Biquad::bandpass(FSC, 2.5)]);
        let c_lp = Cascade::butter_lowpass(2, tp.chroma_bw_khz * 1e3);
        let cn_lp = Cascade::butter_lowpass(2, tp.chroma_bw_khz * 1e3);
        let mag = MagneticTransfer::new(tp.magnetic_model, tp.saturation_ire);

        // 帯域制限した白色雑音の rms を 1 に戻す係数
        let luma_noise_norm = (0.5 * FS / (tp.luma_bw_mhz * 1e6)).sqrt();
        let chroma_noise_norm = (0.5 * FS / (tp.chroma_bw_khz * 1e3)).sqrt();
        let phase_noise = tp.chroma_phase_noise_deg.to_radians();
        let delay = tp.chroma_delay_samples;
        let (parity, frame, index) = (field.parity, field.frame, field.index);
        let rf_level = &self.rf_level;

        self.y_buf
            .par_chunks_mut(LINE_SAMPLES)
            .zip(self.c_buf.par_chunks_mut(LINE_SAMPLES))
            .enumerate()
            .for_each(|(l, (yo, co))| {
                let x = field.line(l);
                let mut rng = Rng::derive(seed, index, l as u64, STREAM_PATH);
                let base = line_phase_base(l, frame, parity);
                let noise_gain = 1.0 / rf_level[l];
                let (mut rec_notch, mut rec_lp, mut pre, mut de, mut noise_lp, mut sharp) =
                    (rec_notch, rec_lp, pre, de, noise_lp, sharp);
                let (mut c_bp, mut lp_i, mut lp_q, mut n_i, mut n_q) = (c_bp, c_lp, c_lp, cn_lp, cn_lp);

                // 再生時の低域変換クロマの位相誤差 (APC で取り切れない残留分)
                let dphi = rng.normal() * phase_noise * noise_gain.min(4.0);
                let (cd, sd) = (dphi.cos(), dphi.sin());
                let mut beard_left = 0u32;

                co.fill(0.0);
                for n in 0..LINE_SAMPLES {
                    let xv = x[n];

                    // ---- 輝度 (FM) ----
                    let yr = rec_lp.process(rec_notch.process(xv));
                    let pe = pre.process(yr).clamp(ire::SYNC - 20.0, tp.white_clip_ire);
                    if beard_left == 0
                        && pe > tp.bearding_threshold_ire
                        && rng.uniform() < 0.25 * tp.bearding_strength
                    {
                        // 過変調でキャリアが消え、復調出力が黒側へ跳ねる
                        beard_left = 4 + (rng.uniform() * 24.0 * tp.bearding_strength) as u32;
                    }
                    let noise = noise_lp.process(rng.normal()) * luma_noise_norm * tp.luma_noise_ire * noise_gain;
                    let mut yd = de.process(pe + noise);
                    if beard_left > 0 {
                        yd = yd * 0.15 - 5.0;
                        beard_left -= 1;
                    }
                    yo[n] = sharp.process(yd);

                    // ---- 色 (低域変換) ----
                    let k = (n + base) & 3;
                    let cr = c_bp.process(xv);
                    let mut bi = lp_i.process(2.0 * cr * COS4[k]);
                    let mut bq = lp_q.process(2.0 * cr * SIN4[k]);
                    let a = (bi * bi + bq * bq).sqrt();
                    if a > 1e-4 {
                        let g = mag.amplitude(a) / a;
                        bi *= g;
                        bq *= g;
                    }
                    let cn = tp.chroma_noise_ire * chroma_noise_norm * noise_gain;
                    bi += n_i.process(rng.normal()) * cn;
                    bq += n_q.process(rng.normal()) * cn;
                    let (ri, rq) = (bi * cd - bq * sd, bi * sd + bq * cd);
                    let out_n = n as i32 + delay;
                    if (0..LINE_SAMPLES as i32).contains(&out_n) {
                        let m = out_n as usize;
                        let km = (m + base) & 3;
                        co[m] = ri * COS4[km] + rq * SIN4[km];
                    }
                }
            });
    }

    fn dropouts(&mut self, field: &Field, tp: &TapeParams, seed: u64) {
        if tp.dropout_rate <= 0.0 {
            return;
        }
        for l in 0..FIELD_LINES {
            let mut rng = Rng::derive(seed, field.index, l as u64, STREAM_DROPOUT);
            let rf = self.rf_level[l];
            let prob = (tp.dropout_rate * (1.0 + 30.0 * (1.0 - rf))).min(1.0);
            let mut events = 0;
            while events < 3 && rng.uniform() < prob {
                events += 1;
                let start = (rng.uniform() * LINE_SAMPLES as f32) as usize;
                let len = rng.exponential(tp.dropout_length * (1.0 + 2.0 * (1.0 - rf))) as usize + 2;
                let end = (start + len).min(LINE_SAMPLES);
                for n in start..end {
                    let i = l * LINE_SAMPLES + n;
                    if tp.dropout_compensation && l > 0 {
                        self.y_buf[i] = self.y_buf[i - LINE_SAMPLES];
                    } else {
                        // RF が途切れると FM 復調器はリミッタ雑音を出す (白っぽいスパークル)
                        self.y_buf[i] = 75.0 + 30.0 * rng.normal();
                    }
                    self.c_buf[i] = 0.0;
                }
            }
        }
    }

    fn transport(&mut self, field: &mut Field, tr: &TransportParams, seed: u64) {
        let mut rng = Rng::derive(seed, field.index, 0, STREAM_TRANSPORT);
        let rho = tr.jitter_correlation.clamp(0.0, 0.999);
        let innov = tr.jitter_samples * (1.0 - rho * rho).sqrt();
        // ヘッド切替位置はフィールドごとに少しふらつく
        self.switch_wander = 0.8 * self.switch_wander + rng.normal() * 0.6;
        let hs_lines = tr.head_switch_lines.min(FIELD_LINES) as f32 + self.switch_wander.max(-(tr.head_switch_lines as f32));
        let hs_start = (FIELD_LINES as f32 - hs_lines.max(0.0)).round() as usize;

        for l in 0..FIELD_LINES {
            self.jitter_state = rho * self.jitter_state + innov * rng.normal();
            let t = (field.index as f32 / FIELD_RATE) + l as f32 * LINE_TIME;
            let mut off = self.jitter_state + tr.wow_samples * (2.0 * std::f32::consts::PI * tr.wow_hz * t).sin();
            off += tr.skew_samples * (-(l as f32) / 10.0).exp();
            if l >= hs_start {
                let f = (l - hs_start + 1) as f32 / (FIELD_LINES - hs_start).max(1) as f32;
                off += tr.head_switch_offset * f * f;
            }
            let rf = self.rf_level[l];
            off += (1.0 - rf) * 6.0 * rng.normal();
            self.offsets[l] = off;
        }

        let offsets = &self.offsets;
        let (y_buf, c_buf) = (&self.y_buf, &self.c_buf);
        let hs_noise = tr.head_switch_noise;
        let index = field.index;
        field.data.par_chunks_mut(LINE_SAMPLES).enumerate().for_each(|(l, out)| {
            let mut comp = [0f32; LINE_SAMPLES];
            let base = l * LINE_SAMPLES;
            for n in 0..LINE_SAMPLES {
                comp[n] = y_buf[base + n] + c_buf[base + n];
            }
            if l == hs_start && hs_noise > 0.0 && hs_start < FIELD_LINES {
                // ヘッド切替の瞬間: ライン途中から乱れる
                let mut rng = Rng::derive(seed, index, l as u64, STREAM_TRANSPORT + 1);
                let from = (rng.uniform() * LINE_SAMPLES as f32 * 0.6) as usize;
                for v in &mut comp[from..] {
                    *v += rng.normal() * 25.0 * hs_noise;
                }
            }
            fractional_shift(&comp, out, offsets[l], ire::BLANK);
        });
    }
}
