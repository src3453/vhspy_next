//! VHS 方式の磁気テープ記録・再生モデル。
//!
//! 処理は実機の信号経路に沿って 3 段階に分かれる。
//!
//! 1. 記録/再生の信号経路 (ライン並列)
//!    - 輝度: Y/C 分離 → 帯域制限 → エンファシス → ホワイトクリップ → FM 伝送 (ノイズ・過変調による
//!      ベアディング) → ディエンファシス → ピーキング
//!    - 色: バンドパス → 低域変換 (複素包絡線で等価的に扱う) → 狭帯域化 → 磁気飽和 → AM/位相ノイズ
//!      → 再生時の高域変換
//! 2. 媒体の欠陥: ドロップアウトとドロップアウト補償 (DOC)、しわ (クリース)、ヘッド目詰まり
//!    (しわと目詰まりは再生 RF レベルの低下として扱い、ノイズ・ドロップアウト・時間軸の乱れに波及させる)
//! 3. 走行系: ジッタ、ワウフラッタ、スキュー、ヘッド切替、トラッキングずれ (時間軸誤差として
//!    ライン全体 = 同期・バーストごとずらす。補正は受像機の AFC/バースト同期に任せる)、垂直方向の揺れ

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
const STREAM_DAMAGE: u64 = 5;

pub struct TapeSim {
    y_buf: Vec<f32>,
    c_buf: Vec<f32>,
    rf_level: Vec<f32>,
    offsets: Vec<f32>,
    /// ジッタのランダムウォーク状態 (フィールドをまたいで連続)
    jitter_state: f32,
    /// 画面下端ヘッド切替位置のゆらぎ状態
    switch_wander: f32,
    /// 垂直揺れの状態 [ライン]
    v_state: f32,
    /// 進行中のしわ: (残りフィールド数, 中心ライン, 1 フィールドあたりの移動量, 帯の高さ)
    crease: Option<(f32, f32, f32, f32)>,
    /// ヘッド目詰まりの残りフィールド数
    clog_left: f32,
}

impl Default for TapeSim {
    fn default() -> Self {
        Self::new(&NTSC)
    }
}

impl TapeSim {
    pub fn new(s: &Standard) -> Self {
        let n = s.field_lines * s.line_samples;
        Self {
            y_buf: vec![0.0; n],
            c_buf: vec![0.0; n],
            rf_level: vec![1.0; s.field_lines],
            offsets: vec![0.0; s.field_lines],
            jitter_state: 0.0,
            switch_wander: 0.0,
            v_state: 0.0,
            crease: None,
            clog_left: 0.0,
        }
    }
}

impl TapeSim {
    pub fn process(&mut self, field: &mut Field, tp: &TapeParams, tr: &TransportParams, seed: u64) {
        if self.y_buf.len() != field.data.len() {
            *self = Self::new(field.std);
        }
        self.compute_rf_level(field, tp, tr, seed);
        self.signal_path(field, tp, seed);
        self.dropouts(field, tp, seed);
        self.transport(field, tr, seed);
    }

    /// トラッキングずれによるライン毎の再生 RF レベル (1 = 正常)
    fn compute_rf_level(&mut self, field: &Field, tp: &TapeParams, tr: &TransportParams, seed: u64) {
        let te = tr.tracking_error.clamp(0.0, 1.0);
        let band_h = 24.0;
        let period = field.std.field_lines as f32 + 2.0 * band_h;
        let center = (field.index as f32 * tr.tracking_speed).rem_euclid(period) - band_h;
        for (l, rf) in self.rf_level.iter_mut().enumerate() {
            let d = (l as f32 - center) / band_h;
            let bump = (-d * d * 2.0).exp();
            // トラッキングがずれると全体の RF もいくらか落ちる
            *rf = (1.0 - te * (0.3 + 0.7 * bump)).max(0.02);
        }
        self.damage(field, tp, seed);
    }

    /// しわ・ヘッド目詰まりのイベントを進め、RF レベルに反映する
    fn damage(&mut self, field: &Field, tp: &TapeParams, seed: u64) {
        let mut rng = Rng::derive(seed, field.index, 0, STREAM_DAMAGE);
        let s = field.std;
        let per_field = |per_min: f32| (per_min / 60.0 / s.field_rate).clamp(0.0, 1.0);

        if self.crease.is_none() && rng.uniform() < per_field(tp.crease_rate) {
            let len = tp.crease_fields.max(1.0) * (0.5 + rng.uniform());
            let center = rng.uniform() * s.field_lines as f32;
            // しわはテープを斜めに横切るので、再生中は画面上をゆっくり上下に流れる
            let drift = (rng.uniform() - 0.5) * 3.0;
            let height = 4.0 + rng.uniform() * 14.0;
            self.crease = Some((len, center, drift, height));
        }
        if let Some((left, center, drift, height)) = self.crease.as_mut() {
            // 出始めと終わりは弱い
            let env = (*left / 3.0).min(1.0);
            for (l, rf) in self.rf_level.iter_mut().enumerate() {
                let d = (l as f32 - *center).abs() / *height;
                if d < 1.5 {
                    let depth = env * (1.0 - (d / 1.5).powi(2)) * (0.7 + 0.3 * rng.uniform());
                    *rf = (*rf * (1.0 - 0.97 * depth)).max(0.02);
                }
            }
            *center += *drift + rng.normal() * 0.7;
            *left -= 1.0;
            if *left <= 0.0 {
                self.crease = None;
            }
        }

        if self.clog_left <= 0.0 && rng.uniform() < per_field(tp.head_clog_rate) {
            self.clog_left = 3.0 + rng.exponential(12.0);
        }
        if self.clog_left > 0.0 {
            // 片側ヘッドだけ詰まることが多いので、フィールドごとに程度が変わる
            let depth = if field.parity == 0 { 0.98 } else { 0.6 + 0.35 * rng.uniform() };
            for rf in self.rf_level.iter_mut() {
                *rf = (*rf * (1.0 - depth)).max(0.02);
            }
            self.clog_left -= 1.0;
        }
    }

    fn signal_path(&mut self, field: &Field, tp: &TapeParams, seed: u64) {
        let s = field.std;
        let (fs, ls) = (s.fs, s.line_samples);
        let lv = s.ire;
        let rec_notch = Biquad::notch_at(s.fsc, 1.2, fs);
        let rec_lp = Cascade::butter_lowpass_at(4, tp.luma_bw_mhz * 1e6, fs);
        let pre = Biquad::high_shelf_at(tp.emphasis_mhz * 1e6, tp.emphasis_db, fs);
        let de = Biquad::high_shelf_at(tp.emphasis_mhz * 1e6, -tp.emphasis_db, fs);
        let noise_lp = Cascade::butter_lowpass_at(2, tp.luma_bw_mhz * 1e6, fs);
        let sharp = Biquad::high_shelf_at(2.0e6, tp.sharpness_db, fs);
        let c_bp = Cascade::new(&[Biquad::bandpass_at(s.fsc, 2.5, fs)]);
        let c_lp = Cascade::butter_lowpass_at(2, tp.chroma_bw_khz * 1e3, fs);
        let cn_lp = Cascade::butter_lowpass_at(2, tp.chroma_bw_khz * 1e3, fs);
        let mag = MagneticTransfer::new(tp.magnetic_model, tp.saturation_ire);

        // 帯域制限した白色雑音の rms を 1 に戻す係数
        let luma_noise_norm = (0.5 * fs / (tp.luma_bw_mhz * 1e6)).sqrt();
        let chroma_noise_norm = (0.5 * fs / (tp.chroma_bw_khz * 1e3)).sqrt();
        let phase_noise = tp.chroma_phase_noise_deg.to_radians();
        let delay = tp.chroma_delay_samples;
        let (parity, frame, index) = (field.parity, field.frame, field.index);
        let rf_level = &self.rf_level;

        self.y_buf
            .par_chunks_mut(ls)
            .zip(self.c_buf.par_chunks_mut(ls))
            .enumerate()
            .for_each(|(l, (yo, co))| {
                let x = field.line(l);
                let mut rng = Rng::derive(seed, index, l as u64, STREAM_PATH);
                let base = s.line_phase_base(l, frame, parity);
                let noise_gain = 1.0 / rf_level[l];
                let (mut rec_notch, mut rec_lp, mut pre, mut de, mut noise_lp, mut sharp) =
                    (rec_notch, rec_lp, pre, de, noise_lp, sharp);
                let (mut c_bp, mut lp_i, mut lp_q, mut n_i, mut n_q) = (c_bp, c_lp, c_lp, cn_lp, cn_lp);

                // 再生時の低域変換クロマの位相誤差 (APC で取り切れない残留分)
                let dphi = rng.normal() * phase_noise * noise_gain.min(4.0);
                let (cd, sd) = (dphi.cos(), dphi.sin());
                let mut beard_left = 0u32;

                co.fill(0.0);
                for n in 0..ls {
                    let xv = x[n];

                    // ---- 輝度 (FM) ----
                    let yr = rec_lp.process(rec_notch.process(xv));
                    let pe = pre.process(yr).clamp(lv.sync - 20.0, tp.white_clip_ire);
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
                    if (0..ls as i32).contains(&out_n) {
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
        let ls = field.std.line_samples;
        for l in 0..field.std.field_lines {
            let mut rng = Rng::derive(seed, field.index, l as u64, STREAM_DROPOUT);
            let rf = self.rf_level[l];
            let prob = (tp.dropout_rate * (1.0 + 30.0 * (1.0 - rf))).min(1.0);
            let mut events = 0;
            while events < 3 && rng.uniform() < prob {
                events += 1;
                let start = (rng.uniform() * ls as f32) as usize;
                let len = rng.exponential(tp.dropout_length * (1.0 + 2.0 * (1.0 - rf))) as usize + 2;
                let end = (start + len).min(ls);
                for n in start..end {
                    let i = l * ls + n;
                    if tp.dropout_compensation && l > 0 {
                        self.y_buf[i] = self.y_buf[i - ls];
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
        let s = field.std;
        let (fl, ls, lv) = (s.field_lines, s.line_samples, s.ire);
        let rho = tr.jitter_correlation.clamp(0.0, 0.999);
        let innov = tr.jitter_samples * (1.0 - rho * rho).sqrt();
        // ヘッド切替位置はフィールドごとに少しふらつく
        self.switch_wander = 0.8 * self.switch_wander + rng.normal() * 0.6;
        let hs_lines = tr.head_switch_lines.min(fl) as f32 + self.switch_wander.max(-(tr.head_switch_lines as f32));
        let hs_start = (fl as f32 - hs_lines.max(0.0)).round() as usize;
        // 垂直揺れ: フィールド単位でライン全体が上下にずれる (整数ライン)
        self.v_state = 0.7 * self.v_state + rng.normal() * tr.vertical_jitter * 0.714;
        let v_shift = self.v_state.round() as isize;

        for l in 0..fl {
            self.jitter_state = rho * self.jitter_state + innov * rng.normal();
            let t = (field.index as f32 / s.field_rate) + l as f32 * s.line_time();
            let mut off = self.jitter_state + tr.wow_samples * (2.0 * std::f32::consts::PI * tr.wow_hz * t).sin();
            off += tr.skew_samples * (-(l as f32) / 10.0).exp();
            if l >= hs_start {
                let f = (l - hs_start + 1) as f32 / (fl - hs_start).max(1) as f32;
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
        field.data.par_chunks_mut(ls).enumerate().for_each(|(l, out)| {
            let mut comp_buf = [0f32; MAX_LINE_SAMPLES];
            let comp = &mut comp_buf[..ls];
            let src = l as isize - v_shift;
            if (0..fl as isize).contains(&src) {
                let base = src as usize * ls;
                for n in 0..ls {
                    comp[n] = y_buf[base + n] + c_buf[base + n];
                }
            } else {
                // 垂直帰線区間が見えている: 同期だけの黒いライン
                for (n, v) in comp.iter_mut().enumerate() {
                    *v = if n < s.sync_end {
                        lv.sync
                    } else if n >= s.active_start {
                        lv.black
                    } else {
                        lv.blank
                    };
                }
            }
            if l == hs_start && hs_noise > 0.0 && hs_start < fl {
                // ヘッド切替の瞬間: ライン途中から乱れる
                let mut rng = Rng::derive(seed, index, l as u64, STREAM_TRANSPORT + 1);
                let from = (rng.uniform() * ls as f32 * 0.6) as usize;
                for v in &mut comp[from..] {
                    *v += rng.normal() * 25.0 * hs_noise;
                }
            }
            fractional_shift(comp, out, offsets[l], lv.blank);
        });
    }
}
