//! 低域変換方式 (VHS / ベータ / 8mm とその高画質版) の磁気テープ記録・再生モデル。
//!
//! 処理は実機の信号経路に沿って 3 段階に分かれる。
//!
//! 1. 記録/再生の信号経路 (ライン並列)
//!    - 輝度: Y/C 分離 → 帯域制限 → エンファシス → ホワイトクリップ → FM 伝送 → ディエンファシス → ピーキング。
//!      FM 伝送は 2 通り: Simple (加算ノイズと確率的なベアディングで近似) と Fm (変調・伝送路・復調を計算、[`fm`])
//!    - 色: バンドパス → 低域変換 (複素包絡線で等価的に扱う) → 狭帯域化 → 磁気飽和 → AM/位相ノイズ
//!      → 再生時の高域変換
//!    - 磁性体の伝達特性 ([`magnetic`]) は記録するすべての信号に掛ける: クロマの包絡線の圧縮に加えて、
//!      FM の RF (搬送波 + 低域変換クロマ) の瞬時値にも掛ける。Fm モデルでは実際に RF に通し、
//!      Simple モデルでは搬送波の圧縮 (S/N の低下) と相互変調 (2 × 低域変換周波数のモアレ) で近似する
//! 2. 媒体の欠陥: ドロップアウトとドロップアウト補償 (DOC)、しわ (クリース)、ヘッド目詰まり。
//!    どれもヘッドとテープの間隔 (スペーシング) が広がる現象として扱い、損失 e^{-kd} ([`head`]) で
//!    周波数の高い信号ほど大きく落とす (FM では搬送波の高い白から先に崩れ、浅いドロップアウトは白い部分だけに出る)。
//!    トラッキングずれは再生できるトラック幅が減るだけなので、周波数によらない RF レベルの低下にする。
//!    RF レベルの低下はノイズ・ドロップアウト・時間軸の乱れに波及させる
//! 3. 走行系: ジッタ、ワウフラッタ、スキュー、ヘッド切替、トラッキングずれ (時間軸誤差として
//!    ライン全体 = 同期・バーストごとずらす。補正は受像機の AFC/バースト同期に任せる)、垂直方向の揺れ

pub mod fm;
pub mod head;
pub mod magnetic;

use crate::filter::{Biquad, Cascade, fractional_shift};
use crate::frame::Field;
use crate::params::{LumaModel, TapeParams, TransportParams};
use fm::{FmChannel, FmConfig};
use crate::rng::Rng;
use crate::timing::*;
use magnetic::MagneticTransfer;
use rayon::prelude::*;

const STREAM_PATH: u64 = 1;
const STREAM_DROPOUT: u64 = 2;
const STREAM_TRANSPORT: u64 = 3;
const STREAM_DAMAGE: u64 = 5;
const STREAM_DROPOUT_NOISE: u64 = 6;

/// しわの中心でテープが浮く量 [m]。VHS の中心搬送波でおよそ −30 dB
const CREASE_SPACING: f32 = 0.85e-6;
/// 目詰まりでヘッドのギャップに付いた汚れの厚み [m] (ひどい側 / 軽い側の範囲)
const CLOG_SPACING_FULL: f32 = 0.95e-6;
const CLOG_SPACING_PARTIAL: (f32, f32) = (0.2e-6, 0.7e-6);
/// ドロップアウトの縁 (スペーシングが立ち上がるまで) の長さ [サンプル]
const DROPOUT_EDGE: usize = 6;

/// ドロップアウト: [開始, 終了) [サンプル] と、その間のスペーシング [m]
pub type Dropout = (usize, usize, f32);

/// スペーシング損失を見積もるための値 (フィールドごとに作る)
#[derive(Clone, Copy)]
struct SpacingModel {
    /// 書き込み速度 [m/s]
    speed: f32,
    /// 搬送波の中心周波数 [Hz] (DOC の検出と RF レベルの目安)
    f_mid: f32,
    /// 輝度レベル [IRE] → 搬送波周波数 [Hz]: f_sync + (y - sync) · hz_per_ire
    f_sync: f32,
    hz_per_ire: f32,
    sync_ire: f32,
    /// 低域変換周波数 [Hz]
    f_cu: f32,
}

impl SpacingModel {
    fn new(tp: &TapeParams, s: &Standard) -> Self {
        let lv = s.ire;
        Self {
            speed: tp.writing_speed_mps.max(0.1),
            f_mid: 0.5 * (tp.fm_sync_mhz + tp.fm_white_mhz) * 1e6,
            f_sync: tp.fm_sync_mhz * 1e6,
            hz_per_ire: (tp.fm_white_mhz - tp.fm_sync_mhz) * 1e6 / (lv.white - lv.sync),
            sync_ire: lv.sync,
            f_cu: tp.color_under_khz * 1e3,
        }
    }

    /// 輝度レベル [IRE] を記録した搬送波の周波数 [Hz]
    #[inline]
    fn carrier(&self, y: f32) -> f32 {
        (self.f_sync + (y - self.sync_ire) * self.hz_per_ire).max(0.0)
    }

    /// 周波数 f でスペーシング d のときの損失 (振幅比)
    #[inline]
    fn loss(&self, f: f32, d: f32) -> f32 {
        head::spacing_loss(f, self.speed, d)
    }
}

pub struct TapeSim {
    y_buf: Vec<f32>,
    c_buf: Vec<f32>,
    /// トラッキングずれによる RF レベル (周波数によらない、1 = 正常)
    rf_track: Vec<f32>,
    /// しわ・目詰まりによるラインごとの余分なスペーシング [m]
    line_spacing: Vec<f32>,
    /// 実効の RF レベル (トラッキング × 中心搬送波でのスペーシング損失)。ノイズ・ドロップアウト・時間軸の乱れの目安
    rf_level: Vec<f32>,
    offsets: Vec<f32>,
    /// ジッタのランダムウォーク状態 (フィールドをまたいで連続)
    jitter_state: f32,
    /// 画面下端ヘッド切替位置のゆらぎ状態
    switch_wander: f32,
    /// 垂直揺れの状態 [ライン]
    v_state: f32,
    /// 次のヘッドの信号の遅れ [ライン] (ヘッド切替から先、次のフィールドの垂直同期を含む部分に掛ける)
    v_next: Option<usize>,
    /// 進行中のしわ: (残りフィールド数, 中心ライン, 1 フィールドあたりの移動量, 帯の高さ)
    crease: Option<(f32, f32, f32, f32)>,
    /// ヘッド目詰まりの残りフィールド数
    clog_left: f32,
    /// ラインごとのドロップアウト (区間とスペーシング)
    drops: Vec<Vec<Dropout>>,
    /// 直前のフィールドの再生信号 (垂直揺れでストリームの前のラインが入ってくるとき使う) と、その偶奇
    prev_y: Vec<f32>,
    prev_c: Vec<f32>,
    prev_parity: Option<u8>,
}

/// 色の低域変換記録 → 再生 (複素包絡線で等価的に扱う)
#[derive(Clone)]
struct ChromaRec {
    c_bp: Cascade,
    lp_i: Cascade,
    lp_q: Cascade,
    n_i: Cascade,
    n_q: Cascade,
    mag: MagneticTransfer,
    /// クロマノイズの rms [IRE]
    cn: f32,
}

impl ChromaRec {
    /// 1 サンプル進めて、再生した複素包絡線 (I, Q) を返す。`k` は副搬送波の位相象限
    #[inline(always)]
    fn step(&mut self, xv: f32, k: usize, rng: &mut Rng) -> (f32, f32) {
        let cr = self.c_bp.process(xv);
        let mut bi = self.lp_i.process(2.0 * cr * COS4[k]);
        let mut bq = self.lp_q.process(2.0 * cr * SIN4[k]);
        let a = (bi * bi + bq * bq).sqrt();
        if a > 1e-4 {
            let g = self.mag.amplitude(a) / a;
            bi *= g;
            bq *= g;
        }
        bi += self.n_i.process(rng.normal()) * self.cn;
        bq += self.n_q.process(rng.normal()) * self.cn;
        (bi, bq)
    }
}

/// Simple モデルで FM の RF に掛かる磁性体の伝達特性を近似する係数
#[derive(Clone, Copy)]
struct SimpleRf {
    /// 搬送波が飽和で縮んだぶんの S/N の低下 (輝度ノイズの倍率)
    noise_gain: f32,
    /// 相互変調による輝度の揺れの係数 [1/IRE]。クロマの複素包絡線の 2 乗に掛ける
    imd: f32,
    /// 低域変換周波数の 1 サンプルあたりの位相の進み [rad]
    dcu: f32,
}

impl SimpleRf {
    fn new(mag: &MagneticTransfer, tp: &TapeParams, s: &Standard) -> Self {
        use std::f32::consts::TAU;
        let dcu = TAU * tp.color_under_khz * 1e3 / s.fs;
        if mag.is_linear() {
            return Self { noise_gain: 1.0, imd: 0.0, dcu };
        }
        let k_cu = tp.color_under_level.max(0.0) / s.ire.burst.max(1e-3);
        let dev_hz = (tp.fm_white_mhz - tp.fm_sync_mhz).max(0.1) * 1e6;
        let ire_per_hz = (s.ire.white - s.ire.sync) / dev_hz;
        if mag.is_jiles_atherton() {
            // 状態付きの J-A で搬送波を記録して測る。基本波 F(c) は直流 c を重ねると偶関数で縮み
            // (F(c) ≈ F0 - F2 c²)、振幅 b の色を重ねると a ± 2c に (F0 - F(b)) / 4 の側波帯が立つ。
            // 小さい b は J-A の数値の揺れに埋もれるので、b = 0.1 で測って b² に比例させる
            let f0 = mag.carrier_fundamental(0.0).max(0.05);
            let b = k_cu.max(0.1);
            let side = (f0 - mag.carrier_fundamental(b)).abs() / 4.0 * (k_cu / b).powi(2) / f0;
            let imd = 0.5 * side * 2.0 * tp.color_under_khz * 1e3 * ire_per_hz;
            return Self { noise_gain: 1.0 / f0, imd, dcu };
        }
        // 搬送波 (振幅 1) の基本波の振幅 = 飽和後の再生 RF レベル
        const N: usize = 64;
        let fund = (0..N)
            .map(|k| {
                let th = TAU * (k as f32 + 0.5) / N as f32;
                mag.transfer(th.cos()) * th.cos()
            })
            .sum::<f32>()
            * 2.0
            / N as f32;
        // 3 次の相互変調: x = cos a + b cos c を S tanh(x / S) に通すと、a ± 2c に振幅 b² / (4 S²) の側波帯が立つ。
        // 対称な側波帯は AM でリミッタに消されるが、伝送路の傾きで半分ほどが PM になり、
        // 周波数偏移 r · 2 f_cu として復調される
        let sat = mag.saturation();
        let imd = 0.5 * k_cu * k_cu / (4.0 * sat * sat) * 2.0 * tp.color_under_khz * 1e3 * ire_per_hz;
        Self { noise_gain: 1.0 / fund.max(0.05), imd, dcu }
    }
}

impl Default for TapeSim {
    fn default() -> Self {
        Self::new(&NTSC)
    }
}

impl TapeSim {
    pub fn new(s: &Standard) -> Self {
        let n = s.raster_lines * s.line_samples;
        Self {
            y_buf: vec![0.0; n],
            c_buf: vec![0.0; n],
            rf_track: vec![1.0; s.raster_lines],
            line_spacing: vec![0.0; s.raster_lines],
            rf_level: vec![1.0; s.raster_lines],
            offsets: vec![0.0; s.raster_lines],
            jitter_state: 0.0,
            switch_wander: 0.0,
            v_state: 0.0,
            v_next: None,
            crease: None,
            clog_left: 0.0,
            drops: vec![Vec::new(); s.raster_lines],
            prev_y: vec![0.0; n],
            prev_c: vec![0.0; n],
            prev_parity: None,
        }
    }
}

impl TapeSim {
    /// 1 フィールドを記録・再生する。`field.separate` が立っていれば Y/C を分けたまま (S 映像) 出力する
    pub fn process(&mut self, field: &mut Field, tp: &TapeParams, tr: &TransportParams, seed: u64) {
        if self.y_buf.len() != field.data.len() {
            *self = Self::new(field.std);
        }
        let sm = SpacingModel::new(tp, field.std);
        self.compute_rf_level(field, tp, tr, seed, &sm);
        // ドロップアウトはスペーシングの急な広がり。Fm モデルでは RF の損失として FM 復調に通す
        self.plan_dropouts(field, tp, seed);
        self.signal_path(field, tp, seed, &sm);
        self.apply_dropouts(field, tp, seed, &sm);
        self.transport(field, tr, seed);
    }

    /// ライン毎の再生 RF レベル: トラッキングずれ (周波数によらない) と、しわ・目詰まりのスペーシング
    fn compute_rf_level(&mut self, field: &Field, tp: &TapeParams, tr: &TransportParams, seed: u64, sm: &SpacingModel) {
        let te = tr.tracking_error.clamp(0.0, 1.0);
        let band_h = 24.0;
        let period = field.std.raster_lines as f32 + 2.0 * band_h;
        let center = (field.index as f32 * tr.tracking_speed).rem_euclid(period) - band_h;
        for (l, rf) in self.rf_track.iter_mut().enumerate() {
            let d = (l as f32 - center) / band_h;
            let bump = (-d * d * 2.0).exp();
            // トラッキングがずれると全体の RF もいくらか落ちる
            *rf = (1.0 - te * (0.3 + 0.7 * bump)).max(0.02);
        }
        self.line_spacing.fill(0.0);
        self.damage(field, tp, seed);
        for ((rf, &track), &d) in self.rf_level.iter_mut().zip(&self.rf_track).zip(&self.line_spacing) {
            *rf = (track * sm.loss(sm.f_mid, d)).max(1e-3);
        }
    }

    /// しわ・ヘッド目詰まりのイベントを進め、ラインごとのスペーシングに足す
    fn damage(&mut self, field: &Field, tp: &TapeParams, seed: u64) {
        let mut rng = Rng::derive(seed, field.index, 0, STREAM_DAMAGE);
        let s = field.std;
        let per_field = |per_min: f32| (per_min / 60.0 / s.field_rate).clamp(0.0, 1.0);

        if self.crease.is_none() && rng.uniform() < per_field(tp.crease_rate) {
            let len = tp.crease_fields.max(1.0) * (0.5 + rng.uniform());
            let center = rng.uniform() * s.raster_lines as f32;
            // しわはテープを斜めに横切るので、再生中は画面上をゆっくり上下に流れる
            let drift = (rng.uniform() - 0.5) * 3.0;
            let height = 4.0 + rng.uniform() * 14.0;
            self.crease = Some((len, center, drift, height));
        }
        if let Some((left, center, drift, height)) = self.crease.as_mut() {
            // 出始めと終わりは弱い
            let env = (*left / 3.0).min(1.0);
            // しわの山でテープがヘッドから浮く
            for (l, sp) in self.line_spacing.iter_mut().enumerate() {
                let d = (l as f32 - *center).abs() / *height;
                if d < 1.5 {
                    let depth = env * (1.0 - (d / 1.5).powi(2)) * (0.7 + 0.3 * rng.uniform());
                    *sp += CREASE_SPACING * depth;
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
            // ギャップに付いた汚れの厚みがスペーシングになる。片側ヘッドだけ詰まることが多いので、フィールドごとに程度が変わる
            let (lo, hi) = CLOG_SPACING_PARTIAL;
            let d = if field.parity == 0 { CLOG_SPACING_FULL } else { lo + (hi - lo) * rng.uniform() };
            for sp in self.line_spacing.iter_mut() {
                *sp += d;
            }
            self.clog_left -= 1.0;
        }
    }

    fn signal_path(&mut self, field: &Field, tp: &TapeParams, seed: u64, sm: &SpacingModel) {
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
        // 色の包絡線の伝達特性。J-A では FM 搬送波がバイアスになり、色は搬送波に対して
        // color_under_level / バースト振幅 の割合で記録される
        let cu_per_ire = tp.color_under_level.max(0.0) / lv.burst.max(1e-3);
        let mag = MagneticTransfer::chroma(
            tp.magnetic_model,
            tp.magnetic_media,
            tp.saturation_ire.max(1.0),
            tp.rf_drive_hc,
            cu_per_ire,
        );
        // FM の RF の伝達特性 (搬送波の振幅 = 1 の尺度)
        let rf_mag = MagneticTransfer::rf(
            tp.magnetic_model,
            tp.magnetic_media,
            tp.rf_saturation.max(0.05),
            tp.rf_drive_hc,
            tp.ja_solver,
        );
        let simple_rf = SimpleRf::new(&rf_mag, tp, s);
        let fm = (tp.luma_model == LumaModel::Fm).then(|| {
            FmChannel::new(&FmConfig {
                fs,
                f_sync_hz: tp.fm_sync_mhz * 1e6,
                f_white_hz: tp.fm_white_mhz * 1e6,
                levels: lv,
                channel_bw_hz: tp.fm_channel_mhz * 1e6,
                luma_bw_hz: tp.luma_bw_mhz * 1e6,
                noise: tp.fm_noise.max(0.0),
                color_under_hz: tp.color_under_khz * 1e3,
                color_under_level: tp.color_under_level.max(0.0),
                mag: rf_mag.clone(),
                writing_speed: tp.writing_speed_mps,
                extra_spacing: tp.head_spacing_um * 1e-6,
            })
        });
        // 余分なスペーシング (再生 EQ が想定していないぶん: ヘッドの汚れ + しわ・目詰まり) の損失。
        // ヘッドアンプのノイズは減らないので、信号が弱まったぶんだけ S/N が落ちる
        // (再生側の AGC/ACC がレベルを戻すとノイズが持ち上がる)
        let base_spacing = tp.head_spacing_um.max(0.0) * 1e-6;
        let sm = *sm;
        let drops = &self.drops;
        let (rf_track, line_spacing) = (&self.rf_track, &self.line_spacing);

        // 帯域制限した白色雑音の rms を 1 に戻す係数
        let luma_noise_norm = (0.5 * fs / (tp.luma_bw_mhz * 1e6)).sqrt();
        let chroma_noise_norm = (0.5 * fs / (tp.chroma_bw_khz * 1e3)).sqrt();
        let phase_noise = tp.chroma_phase_noise_deg.to_radians();
        // VCR の Y/C 遅延の整合: 色の経路 (バンドパス + 低域変換のローパス) の遅れを輝度の経路
        // (ノッチ + ローパス) との差だけ打ち消し、残りの位置ずれは chroma_delay_samples だけにする
        let chroma_path = c_bp.group_delay(s.fsc, fs) + c_lp.group_delay(1e3, fs);
        let luma_path = rec_notch.group_delay(1e3, fs) + rec_lp.group_delay(1e3, fs);
        let delay = tp.chroma_delay_samples - (chroma_path - luma_path).round().max(0.0) as i32;
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
                // トラッキング (周波数によらない) とスペーシング (周波数ごと) は分けて掛ける
                let noise_gain = 1.0 / rf_track[l];
                let spacing = base_spacing + line_spacing[l];
                let (mut rec_notch, mut rec_lp, mut pre, mut de, mut noise_lp, mut sharp) =
                    (rec_notch, rec_lp, pre, de, noise_lp, sharp);
                let cn = tp.chroma_noise_ire * chroma_noise_norm * noise_gain / sm.loss(sm.f_cu, spacing);
                let mut chroma = ChromaRec { c_bp, lp_i: c_lp, lp_q: c_lp, n_i: cn_lp, n_q: cn_lp, mag: mag.clone(), cn };

                // 再生時の低域変換クロマの位相誤差 (APC で取り切れない残留分)
                let dphi = rng.normal() * phase_noise * (1.0 / rf_level[l]).min(4.0);
                let (cd, sd) = (dphi.cos(), dphi.sin());
                let mut beard_left = 0u32;
                let mut cu_phase = rng.uniform() * std::f32::consts::TAU;

                co.fill(0.0);
                // 再生クロマを (遅延をつけて) 書き込む
                let put_chroma = |co: &mut [f32], n: usize, bi: f32, bq: f32| {
                    let (ri, rq) = (bi * cd - bq * sd, bi * sd + bq * cd);
                    let out_n = n as i32 + delay;
                    if (0..ls as i32).contains(&out_n) {
                        let m = out_n as usize;
                        let km = (m + base) & 3;
                        co[m] = ri * COS4[km] + rq * SIN4[km];
                    }
                };

                if let Some(fm) = &fm {
                    // ---- Fm モデル: 記録する輝度と色を 1 ライン分作ってから FM で伝送する ----
                    let mut rec = [0f32; MAX_LINE_SAMPLES];
                    let mut cu_i = [0f32; MAX_LINE_SAMPLES];
                    let mut cu_q = [0f32; MAX_LINE_SAMPLES];
                    for n in 0..ls {
                        let xv = x[n];
                        let yr = rec_lp.process(rec_notch.process(xv));
                        rec[n] = pre.process(yr).clamp(lv.sync - 20.0, tp.white_clip_ire);
                        let (bi, bq) = chroma.step(xv, (n + base) & 3, &mut rng);
                        cu_i[n] = bi;
                        cu_q[n] = bq;
                        put_chroma(co, n, bi, bq);
                    }
                    let mut demod = [0f32; MAX_LINE_SAMPLES];
                    fm.process_line(
                        &rec[..ls],
                        &cu_i[..ls],
                        &cu_q[..ls],
                        rf_track[l],
                        line_spacing[l],
                        &drops[l],
                        &mut rng,
                        &mut demod[..ls],
                    );
                    for n in 0..ls {
                        yo[n] = sharp.process(de.process(demod[n]));
                    }
                    return;
                }

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
                    let mut noise = noise_lp.process(rng.normal())
                        * luma_noise_norm
                        * tp.luma_noise_ire
                        * noise_gain
                        * simple_rf.noise_gain;
                    if spacing > 0.0 {
                        // その瞬間の搬送波周波数でのスペーシング損失ぶんノイズが増える (白ほど大きい)。
                        // 搬送波がノイズに埋もれる先 (−34 dB) は砂嵐と同じなので頭打ちにする
                        noise /= sm.loss(sm.carrier(pe), spacing).max(0.02);
                    }

                    // ---- 色 (低域変換) ----
                    let (bi, bq) = chroma.step(xv, (n + base) & 3, &mut rng);
                    put_chroma(co, n, bi, bq);

                    // 搬送波とクロマの相互変調: (bi + j bq)² e^{j 2 ω_cu t} の実部に比例する輝度の揺れ
                    cu_phase += simple_rf.dcu;
                    if cu_phase > std::f32::consts::TAU {
                        cu_phase -= std::f32::consts::TAU;
                    }
                    let imd = if simple_rf.imd > 0.0 {
                        let (s2, c2) = (2.0 * cu_phase).sin_cos();
                        simple_rf.imd * ((bi * bi - bq * bq) * c2 - 2.0 * bi * bq * s2)
                    } else {
                        0.0
                    };
                    let mut yd = de.process(pe + noise + imd);
                    if beard_left > 0 {
                        yd = yd * 0.15 - 5.0;
                        beard_left -= 1;
                    }
                    yo[n] = sharp.process(yd);
                }
            });
    }

    /// ラインごとのドロップアウトを決める。ドロップアウトはテープの傷・磁性体の欠け・ゴミでテープが
    /// ヘッドから浮くこと (スペーシングの急な広がり) として扱う。浅いものは白い部分だけが崩れ、深いものは RF が消える
    fn plan_dropouts(&mut self, field: &Field, tp: &TapeParams, seed: u64) {
        let ls = field.std.line_samples;
        for (l, segs) in self.drops.iter_mut().enumerate() {
            segs.clear();
            if tp.dropout_rate <= 0.0 {
                continue;
            }
            let mut rng = Rng::derive(seed, field.index, l as u64, STREAM_DROPOUT);
            let rf = self.rf_level[l];
            let prob = (tp.dropout_rate * (1.0 + 30.0 * (1.0 - rf))).min(1.0);
            while segs.len() < 3 && rng.uniform() < prob {
                let start = (rng.uniform() * ls as f32) as usize;
                let len = rng.exponential(tp.dropout_length * (1.0 + 2.0 * (1.0 - rf))) as usize + 2;
                // 浮きの量: 最低でも平均の 0.3 倍、裾の長い分布 (平均 1.3 倍)
                let depth = tp.dropout_depth_um.max(0.0) * 1e-6 * (0.3 + rng.exponential(1.0));
                segs.push((start, (start + len).min(ls), depth));
            }
        }
    }

    /// ドロップアウトの再生側の処理。
    ///
    /// - RF レベルが DOC のしきい値 (中心搬送波で `doc_threshold_db`) より落ちたところは RF の欠落として検出し、
    ///   DOC が有効なら 1H 前のラインで置き換え、色は止める
    /// - しきい値より浅いところは検出されず、そのまま再生される: Fm モデルでは RF の損失がすでに復調に効いている
    ///   (白い部分だけノイズ・黒い筋が出る)。Simple モデルでは搬送波周波数ごとのノイズの増加で近似する。
    ///   低域変換クロマは損失のぶん薄くなる
    /// - 深いのに補償しないときは、FM 復調器がノイズのゼロ交差を拾った砂嵐になる (Fm モデルでは計算済み)
    fn apply_dropouts(&mut self, field: &Field, tp: &TapeParams, seed: u64, sm: &SpacingModel) {
        let ls = field.std.line_samples;
        let fm = tp.luma_model == LumaModel::Fm;
        for l in 0..field.std.raster_lines {
            if self.drops[l].is_empty() {
                continue;
            }
            let mut rng = Rng::derive(seed, field.index, l as u64, STREAM_DROPOUT_NOISE);
            let rf = self.rf_level[l];
            for &(start, end, depth) in &self.drops[l] {
                let level = rf * sm.loss(sm.f_mid, depth);
                let detected = -20.0 * level.max(1e-9).log10() >= tp.doc_threshold_db;
                let cu_loss = sm.loss(sm.f_cu, depth);
                for n in start..end {
                    let i = l * ls + n;
                    if detected {
                        if tp.dropout_compensation && l > 0 {
                            self.y_buf[i] = self.y_buf[i - ls];
                        } else if !fm {
                            // RF が途切れると FM 復調器はリミッタ雑音を出す (白っぽいスパークル)
                            self.y_buf[i] = 75.0 + 30.0 * rng.normal();
                        }
                        self.c_buf[i] = 0.0;
                    } else {
                        if !fm {
                            let gain = 1.0 / sm.loss(sm.carrier(self.y_buf[i]), depth).max(0.02);
                            self.y_buf[i] += tp.luma_noise_ire * (gain - 1.0) * rng.normal();
                        }
                        self.c_buf[i] *= cu_loss;
                    }
                }
            }
        }
    }

    fn transport(&mut self, field: &mut Field, tr: &TransportParams, seed: u64) {
        let mut rng = Rng::derive(seed, field.index, 0, STREAM_TRANSPORT);
        let s = field.std;
        let (rl, ls, lv) = (s.raster_lines, s.line_samples, s.ire);
        let rho = tr.jitter_correlation.clamp(0.0, 0.999);
        let innov = tr.jitter_samples * (1.0 - rho * rho).sqrt();
        // ヘッド切替は次のフィールドの垂直同期の少し前 (有効映像の下端) で起き、そこから先は次のヘッドの信号になる。
        // 切替位置はフィールドごとに少しふらつく
        self.switch_wander = 0.8 * self.switch_wander + rng.normal() * 0.6;
        let hs_lines = (tr.head_switch_lines as f32 + self.switch_wander.max(-(tr.head_switch_lines as f32))).max(0.0);
        let active_end = (field.active_index(0) + s.field_lines) as f32;
        let hs_start = ((active_end - hs_lines).round().max(0.0) as usize).min(rl);
        // 垂直揺れ: フィールド単位で同期ごと信号全体が遅れる (整数ライン)。受像機は垂直同期に追従するので
        // 画面全体が上下に揺れる。進む側には動かせない (もう出力した前のフィールドに垂直同期が入ってしまう) ので
        // 遅れだけにし、有効映像がバッファに収まる範囲に抑える
        // 遅れが変わるのはヘッド切替のところ (次のヘッドの信号から)。ここで変えれば垂直同期のパルス列は切れない
        self.v_state = 0.7 * self.v_state + rng.normal() * tr.vertical_jitter * 0.714;
        let next = (self.v_state.abs().round() as usize).min(s.raster_slack(1 - field.parity));
        let v_cur = self.v_next.replace(next).unwrap_or(next).min(s.raster_slack(field.parity)) as isize;
        let v_next = next as isize;

        for l in 0..rl {
            self.jitter_state = rho * self.jitter_state + innov * rng.normal();
            let t = (field.index as f32 / s.field_rate) + l as f32 * s.line_time();
            let mut off = self.jitter_state + tr.wow_samples * (2.0 * std::f32::consts::PI * tr.wow_hz * t).sin();
            off += tr.skew_samples * (-(l as f32) / 10.0).exp();
            if l >= hs_start {
                let f = ((l - hs_start + 1) as f32 / hs_lines.max(1.0)).min(1.5);
                off += tr.head_switch_offset * f * f;
            }
            let rf = self.rf_level[l];
            off += (1.0 - rf) * 6.0 * rng.normal();
            self.offsets[l] = off;
        }

        let offsets = &self.offsets;
        let (y_buf, c_buf) = (&self.y_buf, &self.c_buf);
        // 前のフィールドの再生信号のうち、ストリームでこのフィールドの直前に来るライン
        let prev = self.prev_parity.map(|pp| (s.stream_lines(pp), &self.prev_y, &self.prev_c));
        let hs_noise = tr.head_switch_noise;
        let index = field.index;
        let separate = field.separate;
        // S 映像なら C も同じ時間軸誤差で別に書き出す
        let mut c_dummy = Vec::new();
        let c_out = if separate { &mut field.chroma } else { &mut c_dummy };
        let c_chunks: Vec<Option<&mut [f32]>> = if separate {
            c_out.chunks_mut(ls).map(Some).collect()
        } else {
            (0..rl).map(|_| None).collect()
        };
        field.data.par_chunks_mut(ls).zip(c_chunks.into_par_iter()).enumerate().for_each(|(l, (out, c_out))| {
            let mut comp_buf = [0f32; MAX_LINE_SAMPLES];
            let comp = &mut comp_buf[..ls];
            let src = l as isize - if l >= hs_start { v_next } else { v_cur };
            // (Y, C) のライン
            let (ys, cs): (&[f32], &[f32]) = if src < 0 {
                match prev {
                    Some((pn, py, pc)) if pn as isize + src >= 0 => {
                        let b = (pn as isize + src) as usize * ls;
                        (&py[b..b + ls], &pc[b..b + ls])
                    }
                    _ => (&y_buf[..ls], &c_buf[..ls]),
                }
            } else {
                // ストリームの先 (次のフィールド) はまだないので最後のラインを繰り返す
                let b = (src as usize).min(rl - 1) * ls;
                (&y_buf[b..b + ls], &c_buf[b..b + ls])
            };
            for n in 0..ls {
                comp[n] = if separate { ys[n] } else { ys[n] + cs[n] };
            }
            if l == hs_start && hs_noise > 0.0 && hs_start < rl {
                // ヘッド切替の瞬間: ライン途中から乱れる
                let mut rng = Rng::derive(seed, index, l as u64, STREAM_TRANSPORT + 1);
                let from = (rng.uniform() * ls as f32 * 0.6) as usize;
                for v in &mut comp[from..] {
                    *v += rng.normal() * 25.0 * hs_noise;
                }
            }
            fractional_shift(comp, out, offsets[l], lv.blank);
            if let Some(c_out) = c_out {
                fractional_shift(cs, c_out, offsets[l], 0.0);
            }
        });
        // 次のフィールドのために今の再生信号を取っておく (作業領域と入れ替えるだけ)
        std::mem::swap(&mut self.y_buf, &mut self.prev_y);
        std::mem::swap(&mut self.c_buf, &mut self.prev_c);
        self.prev_parity = Some(field.parity);
    }
}
