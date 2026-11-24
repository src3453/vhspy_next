//! コンポジット (NTSC / PAL) → RGB の受像機モデル。
//!
//! - 水平 AFC: 同期の後縁を検出し、時定数付きで追従する (遅い時間軸変動は吸収、速い変動は残る)
//! - Y/C 分離: ノッチ (ドットクロール/クロスカラーが出る)、くし形 (NTSC 1H / PAL 2H)、
//!   または 3 次元 (画面バッファとのフレームくし形。動く部分はラインくし形に切り替える動き適応型)
//! - 色副搬送波 PLL: ラインごとのバースト位相に追従係数付きで同期する
//! - NTSC は I/Q、PAL は U/V を同期検波。ACC とカラーキラー付き
//! - PAL: スウィンギングバーストから PAL スイッチの向きを決め (アイデント)、
//!   1H 遅延線で前のラインと色差を平均する (位相誤差 → 彩度低下)。遅延線なしならハノーバーバーが出る

use crate::filter::{Biquad, Cascade, fractional_shift};
use crate::frame::{Field, RgbaFrame};
use crate::nr::NoiseReduction;
use crate::params::{DecoderParams, YcSeparation};
use crate::timing::*;
use rayon::prelude::*;

pub struct Decoder {
    /// AFC が推定している同期後縁位置 (フィールドをまたいで保持)
    h_sync_pos: f32,
    /// AFC 補正後のフィールド (S 映像なら Y)
    aligned: Vec<f32>,
    /// AFC 補正後の C (S 映像のときだけ使う)
    aligned_c: Vec<f32>,
    corrections: Vec<f32>,
    /// Y/C 分離後の輝度・色 (field_lines × line_samples)
    luma: Vec<f32>,
    chroma: Vec<f32>,
    /// 同期検波後の色差 (field_lines × active_width)。a = cos 側 (I / ±V)、b = sin 側 (Q / U)
    det_a: Vec<f32>,
    det_b: Vec<f32>,
    /// ラインごとのバースト検出結果
    bursts: Vec<Burst>,
    /// ラインごとの検波条件
    lines: Vec<LineRef>,
    /// 3 次元 Y/C 分離の画面バッファ: 過去のフィールドの AFC 補正後のコンポジット信号
    /// (新しい順、フィールド番号付き。最大 [`FIELD_STORE`] フィールド)
    history: Vec<(u64, Vec<f32>)>,
    /// 復調した輝度・色差信号 (active_width × field_lines)。NR と RGB 変換で共有する
    nr_y: Vec<f32>,
    nr_u: Vec<f32>,
    nr_v: Vec<f32>,
    noise_reduction: NoiseReduction,
}

/// 画面バッファに持つフィールド数 (PAL の 2 フレーム = 4 フィールド前まで)
const FIELD_STORE: usize = 4;

#[derive(Clone, Copy, Default)]
struct Burst {
    /// 副搬送波との相関 (位相を表す複素数)
    zr: f32,
    zi: f32,
    /// 包絡線のピーク振幅
    amp: f32,
}

#[derive(Clone, Copy, Default)]
struct LineRef {
    /// PLL が出す副搬送波の基準位相 [rad]
    phase: f32,
    /// PAL スイッチ (V の符号)。NTSC は常に +1
    sv: f32,
    /// 色差のゲイン (彩度 × ACC、カラーキラーで 0)
    gain: f32,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new(&NTSC)
    }
}

impl Decoder {
    pub fn new(s: &Standard) -> Self {
        let n = s.field_lines * s.line_samples;
        let na = s.field_lines * s.active_width;
        Self {
            h_sync_pos: s.sync_end as f32,
            aligned: vec![0.0; n],
            aligned_c: Vec::new(),
            corrections: vec![0.0; s.field_lines],
            luma: vec![0.0; n],
            chroma: vec![0.0; n],
            det_a: vec![0.0; na],
            det_b: vec![0.0; na],
            bursts: vec![Burst::default(); s.field_lines],
            lines: vec![LineRef::default(); s.field_lines],
            history: Vec::new(),
            nr_y: vec![0.0; na],
            nr_u: vec![0.0; na],
            nr_v: vec![0.0; na],
            noise_reduction: NoiseReduction::default(),
        }
    }
}

/// 同期の後縁 (-20 IRE を下から上へ横切る位置) を小数精度で探す。
///
/// 実機の同期分離回路と同じく、スライスの前に積分 (ここでは左右対称の移動平均) して
/// 雑音で誤検出しにくくする。対称な平均なのでエッジ位置は動かない。
pub(crate) fn find_sync_edge(line: &[f32], s: &Standard) -> Option<f32> {
    const R: usize = 4;
    let thr = s.ire.sync / 2.0;
    let mut acc: f32 = line[..2 * R + 1].iter().sum();
    let norm = 1.0 / (2 * R + 1) as f32;
    // prev = n-1 を中心とした平均
    let mut prev = acc * norm;
    let mut low_run = 0;
    for n in (R + 1)..(s.active_start - 10) {
        acc += line[n + R] - line[n - R - 1];
        let cur = acc * norm;
        if prev < thr {
            low_run += 1;
        } else {
            low_run = 0;
        }
        // 同期パルス幅の 1/4 以上低レベルが続いたあとの立ち上がりだけを採る
        if low_run >= 16 && prev < thr && cur >= thr {
            let t = (thr - prev) / (cur - prev);
            return Some(n as f32 - 1.0 + t);
        }
        prev = cur;
    }
    None
}

/// PAL アイデント: ライン l の PAL スイッチを sV = s0·(-1)^l としたときの s0 (±1) を返す。
///
/// `mus` はラインごとのバースト相関の位相 μ = φ - 45°·sV (バーストが弱いラインは None)。
/// 隣り合うラインの μ の差は (ライン当たりの位相の進み) - 90°·sV になるので、フィールド全体で多数決を取る
/// (実機の PAL スイッチのフリップフロップをアイデント信号で正しい向きに揃えるのに相当)。
pub(crate) fn pal_ident(mus: &[Option<f32>], s: &Standard) -> f32 {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, TAU};
    let wrap = |a: f32| a - TAU * (a / TAU).round();
    let step = s.line_phase_step() as f32 * FRAC_PI_2;
    let mut vote = 0.0;
    for l in 1..mus.len() {
        if let (Some(a), Some(b)) = (mus[l - 1], mus[l]) {
            let d = wrap(b - a - step);
            // d ≒ -90°·sV。±45° 以内のずれなら判定に使う
            if d.abs() > FRAC_PI_4 && d.abs() < 3.0 * FRAC_PI_4 {
                let sv = -d.signum();
                vote += if l % 2 == 0 { sv } else { -sv };
            }
        }
    }
    if vote < 0.0 { -1.0 } else { 1.0 }
}

impl Decoder {
    pub fn decode_field(
        &mut self,
        field: &Field,
        p: &DecoderParams,
        playback_nr: bool,
        out: &mut RgbaFrame,
    ) {
        let s = field.std;
        let (aw, fl, ls) = (s.active_width, s.field_lines, s.line_samples);
        assert_eq!((out.width, out.height), s.image_size());
        if self.aligned.len() != fl * ls || self.det_a.len() != fl * aw {
            *self = Self::new(s);
        }
        let lv = s.ire;
        let pal = s.is_pal();

        // --- 水平 AFC (逐次) ---
        let k = p.h_pll.clamp(0.0, 1.0);
        for l in 0..fl {
            if let Some(pos) = find_sync_edge(field.line(l), s) {
                self.h_sync_pos += k * (pos - self.h_sync_pos);
            }
            self.corrections[l] = self.h_sync_pos - s.sync_end as f32;
        }
        let corr = &self.corrections;
        self.aligned.par_chunks_mut(ls).enumerate().for_each(|(l, dst)| {
            fractional_shift(field.line(l), dst, -corr[l], lv.blank);
        });
        // S 映像: C も同じ補正で揃える
        let separate = field.separate;
        if separate {
            self.aligned_c.resize(fl * ls, 0.0);
            self.aligned_c.par_chunks_mut(ls).enumerate().for_each(|(l, dst)| {
                fractional_shift(field.chroma_line(l).unwrap(), dst, -corr[l], 0.0);
            });
        }

        // --- Y/C 分離とバースト検出 (ライン並列) ---
        let notch = Cascade::new(&[Biquad::notch_at(s.fsc, 1.0, s.fs)]);
        let c_bp = Cascade::new(&[Biquad::bandpass_at(s.fsc, 1.5, s.fs)]);
        // くし形の相手: 副搬送波が 180° ずれたライン (NTSC は 1H、PAL は V の符号も揃う 2H 前)
        let comb_lag = if pal { 2 } else { 1 };
        let use_3d = p.yc_separation == YcSeparation::Comb3d && !separate;
        let use_comb = p.yc_separation == YcSeparation::Comb;
        // フレームくし形の相手: 副搬送波が 180° ずれた同じ位置のライン
        // (NTSC は 1 フレーム = 2 フィールド前、PAL は 2 フレーム = 4 フィールド前で V の符号も揃う)。
        // 早送りやシークでフィールドが飛んだときは使わない (ラインくし形のまま)
        let frame_lag: u64 = if pal { 4 } else { 2 };
        let frame_ref = if use_3d {
            field.index.checked_sub(frame_lag).and_then(|want| {
                self.history.iter().find(|(i, _)| *i == want).map(|(_, v)| v.as_slice())
            })
        } else {
            None
        };
        // 動き適応なし (None) なら常にフレームくし形
        let motion_thr = p.motion_adaptive.then(|| p.motion_threshold_ire.max(1e-3));
        let (aligned, aligned_c) = (&self.aligned, &self.aligned_c);
        let burst_gate = s.burst_start..s.burst_end + 16;
        self.luma
            .par_chunks_mut(ls)
            .zip(self.chroma.par_chunks_mut(ls))
            .zip(self.bursts.par_iter_mut())
            .enumerate()
            .for_each(|(l, ((y, c), burst))| {
                let x = &aligned[l * ls..(l + 1) * ls];
                if separate {
                    // S 映像: Y/C 分離は要らない (C は帯域外の雑音だけ落とす)
                    let xc = &aligned_c[l * ls..(l + 1) * ls];
                    let mut bp = c_bp;
                    for n in 0..ls {
                        y[n] = x[n];
                        c[n] = bp.process(xc[n]);
                    }
                } else if let Some(fr) = frame_ref {
                    let xf = &fr[l * ls..(l + 1) * ls];
                    let prev = (l >= comb_lag).then(|| &aligned[(l - comb_lag) * ls..(l - comb_lag + 1) * ls]);
                    separate_3d(x, xf, prev, y, c, c_bp, motion_thr);
                } else if (use_comb || use_3d) && l >= comb_lag {
                    let prev = &aligned[(l - comb_lag) * ls..(l - comb_lag + 1) * ls];
                    let mut bp = c_bp;
                    for n in 0..ls {
                        c[n] = bp.process(0.5 * (x[n] - prev[n]));
                        y[n] = x[n] - c[n];
                    }
                } else {
                    let (mut nf, mut bp) = (notch, c_bp);
                    for n in 0..ls {
                        y[n] = nf.process(x[n]);
                        c[n] = bp.process(x[n]);
                    }
                }

                // バースト位相・振幅の検出
                // バーストゲートはテープ再生時の包絡線の遅れも含むよう広めに取る。
                // 位相は全体の和から、振幅 (ACC 用) は 1 周期ごとの包絡線のピークから求める。
                let (mut zr, mut zi) = (0f32, 0f32);
                let mut amp = 0f32;
                for cyc in burst_gate.clone().step_by(4) {
                    let (mut cr, mut ci) = (0f32, 0f32);
                    for n in cyc..cyc + 4 {
                        cr += c[n] * COS4[n & 3];
                        ci -= c[n] * SIN4[n & 3];
                    }
                    zr += cr;
                    zi += ci;
                    amp = amp.max(0.5 * (cr * cr + ci * ci).sqrt());
                }
                *burst = Burst { zr, zi, amp };
            });

        // 画面バッファを更新する (3 次元 Y/C 分離のときだけ持つ)
        if use_3d {
            let mut buf = if self.history.len() >= FIELD_STORE { self.history.pop().unwrap().1 } else { Vec::new() };
            buf.clear();
            buf.extend_from_slice(&self.aligned);
            quantize_buffer(&mut buf, p.frame_buffer_bits, &lv);
            self.history.insert(0, (field.index, buf));
        } else {
            self.history.clear();
        }

        // --- 色副搬送波 PLL と PAL アイデント (逐次) ---
        self.lock_subcarrier(s, p);

        // --- 同期検波 (ライン並列) ---
        let c_lp = Cascade::butter_lowpass_at(2, p.chroma_bw_mhz * 1e6, s.fs);
        // NTSC は I/Q 軸 (U/V から 33°) で検波する。PAL は U/V 軸のまま
        let hue = if pal { p.hue_deg.to_radians() } else { p.hue_deg.to_radians() + 33f32.to_radians() };
        let (active_start, active_end) = (s.active_start, s.active_end());
        let (chroma, lines) = (&self.chroma, &self.lines);
        self.det_a.par_chunks_mut(aw).zip(self.det_b.par_chunks_mut(aw)).enumerate().for_each(|(l, (da, db))| {
            let c = &chroma[l * ls..(l + 1) * ls];
            let psi = lines[l].phase;
            let mut cos_t = [0f32; 4];
            let mut sin_t = [0f32; 4];
            for k in 0..4 {
                let a = k as f32 * std::f32::consts::FRAC_PI_2 + psi + hue;
                cos_t[k] = a.cos();
                sin_t[k] = a.sin();
            }
            let (mut alp, mut blp) = (c_lp, c_lp);
            for n in 0..active_end {
                let a = alp.process(2.0 * c[n] * cos_t[n & 3]);
                let b = blp.process(2.0 * c[n] * sin_t[n & 3]);
                if n >= active_start {
                    da[n - active_start] = a;
                    db[n - active_start] = b;
                }
            }
        });

        // --- VCR playback NR in recovered baseband Y/C ---
        for l in 0..fl {
            let y_line = &self.luma[l * ls..(l + 1) * ls];
            let (da, db) = (&self.det_a[l * aw..(l + 1) * aw], &self.det_b[l * aw..(l + 1) * aw]);
            let LineRef { sv, gain, .. } = self.lines[l];
            let prev = (pal && p.pal_delay_line && l > 0).then(|| {
                let r = self.lines[l - 1];
                (&self.det_a[(l - 1) * aw..l * aw], &self.det_b[(l - 1) * aw..l * aw], r.sv * r.gain, r.gain)
            });
            for x in 0..aw {
                let i = l * aw + x;
                self.nr_y[i] = y_line[x + active_start] - lv.black;
                if pal {
                    let (mut v, mut u) = (da[x] * sv * gain, db[x] * gain);
                    if let Some((pa, pb, pvg, pug)) = prev {
                        v = 0.5 * (v + pa[x] * pvg);
                        u = 0.5 * (u + pb[x] * pug);
                    }
                    self.nr_u[i] = u;
                    self.nr_v[i] = v;
                } else {
                    self.nr_u[i] = da[x] * gain;
                    self.nr_v[i] = db[x] * gain;
                }
            }
        }
        if playback_nr {
            self.noise_reduction.process(
                field.index,
                aw,
                fl,
                &mut self.nr_y,
                &mut self.nr_u,
                &mut self.nr_v,
                p.ynr_strength,
                p.dnr_strength,
                p.dnr_motion_threshold_ire,
                p.nr_bits,
            );
        } else {
            self.noise_reduction.reset();
        }

        // --- RGB へ変換して書き込む ---
        let parity = field.parity as usize;
        let row_bytes = aw * 4;
        let (nr_y, nr_u, nr_v) = (&self.nr_y, &self.nr_u, &self.nr_v);
        out.data.par_chunks_mut(row_bytes).enumerate().for_each(|(row, dst)| {
            if row % 2 != parity {
                return;
            }
            let l = row / 2;
            let y = &nr_y[l * aw..(l + 1) * aw];
            let u = &nr_u[l * aw..(l + 1) * aw];
            let v = &nr_v[l * aw..(l + 1) * aw];
            for x in 0..aw {
                let yy = y[x] / lv.span() * p.contrast + p.brightness;
                let (r, g, b) = if pal {
                    (yy + 1.140 * v[x], yy - 0.395 * u[x] - 0.581 * v[x], yy + 2.032 * u[x])
                } else {
                    (yy + 0.956 * u[x] + 0.619 * v[x], yy - 0.272 * u[x] - 0.647 * v[x], yy - 1.106 * u[x] + 1.703 * v[x])
                };
                let o = x * 4;
                dst[o] = to_u8(r);
                dst[o + 1] = to_u8(g);
                dst[o + 2] = to_u8(b);
                dst[o + 3] = 255;
            }
        });
    }

    /// バーストからラインごとの基準位相・PAL スイッチ・色のゲインを決める。
    ///
    /// バースト相関の位相 μ は NTSC では副搬送波位相 φ そのもの、PAL では φ - 45°·sV (スウィンギングバースト)。
    /// - PAL アイデント: 隣り合うラインの μ の差は (ライン当たりの位相の進み) - 90°·sV になるので、
    ///   フィールド全体の多数決で PAL スイッチ (フリップフロップ) の向きを決める
    /// - PLL: 前のラインの基準位相 + ライン当たりの進みを予測とし、バーストとの差に `burst_pll` を掛けて追従する。
    ///   バーストが弱いラインは予測のまま惰性で回る
    fn lock_subcarrier(&mut self, s: &Standard, p: &DecoderParams) {
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, TAU};
        let wrap = |a: f32| a - TAU * (a / TAU).round();
        let step = s.line_phase_step() as f32 * FRAC_PI_2;
        let pal = s.is_pal();
        let mu = |b: &Burst| b.zi.atan2(b.zr) - FRAC_PI_2;
        let valid = |b: &Burst| b.amp >= 3.0;

        let s0 = if pal {
            let mus: Vec<Option<f32>> = self.bursts.iter().map(|b| valid(b).then(|| mu(b))).collect();
            pal_ident(&mus, s)
        } else {
            1.0
        };

        let k = p.burst_pll.clamp(0.0, 1.0);
        let mut prev: Option<f32> = None;
        for l in 0..self.bursts.len() {
            let b = self.bursts[l];
            let sv = if !pal {
                1.0
            } else if l % 2 == 0 {
                s0
            } else {
                -s0
            };
            // バーストから測った副搬送波位相
            let measured = if pal { mu(&b) + FRAC_PI_4 * sv } else { mu(&b) };
            let phase = match prev {
                // 即時同期 (ラインごとにバーストへ合わせる)
                _ if k >= 1.0 => measured,
                None => measured,
                Some(ph) => {
                    let pred = wrap(ph + step);
                    if valid(&b) { pred + k * wrap(measured - pred) } else { pred }
                }
            };
            prev = Some(phase);

            let mut gain = p.saturation / s.ire.span();
            if p.acc {
                gain *= (s.ire.burst / b.amp.max(1e-3)).min(8.0);
            }
            if p.color_killer && b.amp < 3.0 {
                gain = 0.0;
            }
            self.lines[l] = LineRef { phase, sv, gain };
        }
    }
}

/// 画面バッファへ書き込むときの量子化 (フレームメモリの A/D 変換)。
/// 範囲は同期先端から、白 + 色の振幅のぶんのピーク (NTSC なら約 -40〜150 IRE) まで。範囲外は張り付く
fn quantize_buffer(buf: &mut [f32], bits: u32, lv: &Levels) {
    if bits == 0 {
        return;
    }
    let bits = bits.min(16);
    let lo = lv.sync;
    let hi = lv.white + 0.35 * (lv.white - lv.sync);
    let steps = ((1u32 << bits) - 1) as f32;
    let (scale, inv) = (steps / (hi - lo), (hi - lo) / steps);
    buf.par_iter_mut().for_each(|v| {
        let q = ((*v - lo) * scale).round().clamp(0.0, steps);
        *v = lo + q * inv;
    });
}

/// 動き適応型 3 次元 Y/C 分離 (1 ライン)。
///
/// - フレームくし形: 静止部分では `x - xf` が色だけの 2 倍になるので C = (x - xf) / 2、Y = (x + xf) / 2。
///   帯域を削らないので、細かい輝度の模様もクロスカラーにならず、色の縦のにじみもない
/// - 動き検出: フレーム間差から副搬送波 (fsc とその付近) を落とした低域成分の大きさ。
///   静止部分では色の差だけになり 0 に近く、動く部分では輝度の差が残る
/// - 動きの量に応じて、ラインくし形 (`prev` がなければバンドパス) の C と混ぜる
fn separate_3d(
    x: &[f32],
    xf: &[f32],
    prev: Option<&[f32]>,
    y: &mut [f32],
    c: &mut [f32],
    c_bp: Cascade,
    thr: Option<f32>,
) {
    let n_len = x.len();
    let Some(thr) = thr else {
        // 動き適応なし: 純粋なフレームくし形
        for n in 0..n_len {
            c[n] = 0.5 * (x[n] - xf[n]);
            y[n] = x[n] - c[n];
        }
        return;
    };
    // フレーム間差を 4 サンプル (副搬送波 1 周期) の箱形フィルタ 2 段 (三角窓 7 タップ、fsc と 2fsc に零点) で
    // 低域だけにし、その絶対値を y に一時的に置く
    const TRI: [f32; 7] = [1.0, 2.0, 3.0, 4.0, 3.0, 2.0, 1.0];
    for n in 0..n_len {
        let mut acc = 0.0;
        for (k, w) in TRI.iter().enumerate() {
            let i = (n + k).saturating_sub(3).min(n_len - 1);
            acc += w * (x[i] - xf[i]);
        }
        y[n] = (acc / 16.0).abs();
    }
    // 動きの量を横に広げて (±6 サンプルの最大値) c に置く。境目でぱらつかないように
    const R: usize = 6;
    for n in 0..n_len {
        let lo = n.saturating_sub(R);
        let hi = (n + R + 1).min(n_len);
        c[n] = y[lo..hi].iter().cloned().fold(0.0, f32::max);
    }
    let mut bp = c_bp;
    for n in 0..n_len {
        // 0: 静止 (フレームくし形)、1: 動き (ラインくし形)
        let k = ((c[n] - thr) / thr).clamp(0.0, 1.0);
        let c2 = match prev {
            Some(pv) => bp.process(0.5 * (x[n] - pv[n])),
            None => bp.process(x[n]),
        };
        let c3 = 0.5 * (x[n] - xf[n]);
        let cc = c3 + k * (c2 - c3);
        c[n] = cc;
        y[n] = x[n] - cc;
    }
}

#[inline(always)]
fn to_u8(v: f32) -> u8 {
    (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}
