//! コンポジット (NTSC / PAL) → RGB の受像機モデル。
//!
//! - 水平 AFC: 同期の後縁を検出し、時定数付きで追従する (遅い時間軸変動は吸収、速い変動は残る)
//! - Y/C 分離: ノッチ (ドットクロール/クロスカラーが出る) またはくし形 (NTSC 1H / PAL 2H)
//! - 色副搬送波 PLL: ラインごとのバースト位相に追従係数付きで同期する
//! - NTSC は I/Q、PAL は U/V を同期検波。ACC とカラーキラー付き
//! - PAL: スウィンギングバーストから PAL スイッチの向きを決め (アイデント)、
//!   1H 遅延線で前のラインと色差を平均する (位相誤差 → 彩度低下)。遅延線なしならハノーバーバーが出る

use crate::filter::{Biquad, Cascade, fractional_shift};
use crate::frame::{Field, RgbaFrame};
use crate::params::{Deinterlace, DecoderParams, YcSeparation};
use crate::timing::*;
use rayon::prelude::*;

pub struct Decoder {
    /// AFC が推定している同期後縁位置 (フィールドをまたいで保持)
    h_sync_pos: f32,
    /// AFC 補正後のフィールド
    aligned: Vec<f32>,
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
}

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
            corrections: vec![0.0; s.field_lines],
            luma: vec![0.0; n],
            chroma: vec![0.0; n],
            det_a: vec![0.0; na],
            det_b: vec![0.0; na],
            bursts: vec![Burst::default(); s.field_lines],
            lines: vec![LineRef::default(); s.field_lines],
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
    pub fn decode_field(&mut self, field: &Field, p: &DecoderParams, deint: Deinterlace, out: &mut RgbaFrame) {
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

        // --- Y/C 分離とバースト検出 (ライン並列) ---
        let notch = Cascade::new(&[Biquad::notch_at(s.fsc, 1.0, s.fs)]);
        let c_bp = Cascade::new(&[Biquad::bandpass_at(s.fsc, 1.5, s.fs)]);
        // くし形の相手: 副搬送波が 180° ずれたライン (NTSC は 1H、PAL は V の符号も揃う 2H 前)
        let comb_lag = if pal { 2 } else { 1 };
        let use_comb = p.yc_separation == YcSeparation::Comb;
        let aligned = &self.aligned;
        let burst_gate = s.burst_start..s.burst_end + 16;
        self.luma
            .par_chunks_mut(ls)
            .zip(self.chroma.par_chunks_mut(ls))
            .zip(self.bursts.par_iter_mut())
            .enumerate()
            .for_each(|(l, ((y, c), burst))| {
                let x = &aligned[l * ls..(l + 1) * ls];
                if use_comb && l >= comb_lag {
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

        // --- RGB へ変換して書き込む ---
        let parity = field.parity as usize;
        let row_bytes = aw * 4;
        let rows: Vec<(usize, &mut [u8])> = out.data.chunks_mut(row_bytes).enumerate().collect();
        // 書き込み先のフィールドライン番号ごとに行をまとめる
        let mut targets: Vec<Vec<&mut [u8]>> = (0..fl).map(|_| Vec::new()).collect();
        for (row, buf) in rows {
            match deint {
                Deinterlace::Weave if row % 2 == parity => targets[row / 2].push(buf),
                Deinterlace::Bob => targets[row / 2].push(buf),
                _ => {}
            }
        }
        let (luma, det_a, det_b) = (&self.luma, &self.det_a, &self.det_b);
        let delay_line = pal && p.pal_delay_line;
        targets.into_par_iter().enumerate().for_each(|(l, mut dsts)| {
            let y = &luma[l * ls..(l + 1) * ls];
            let (da, db) = (&det_a[l * aw..(l + 1) * aw], &det_b[l * aw..(l + 1) * aw]);
            let LineRef { sv, gain, .. } = lines[l];
            // 1H 遅延線: 前のラインの色差 (V の符号はそのラインの PAL スイッチで戻す)
            let prev = (delay_line && l > 0).then(|| {
                let r = lines[l - 1];
                (&det_a[(l - 1) * aw..l * aw], &det_b[(l - 1) * aw..l * aw], r.sv * r.gain, r.gain)
            });
            let mut rgba_buf = [0u8; MAX_ACTIVE_WIDTH * 4];
            let rgba = &mut rgba_buf[..aw * 4];
            for x in 0..aw {
                let yy = (y[x + active_start] - lv.black) / lv.span() * p.contrast + p.brightness;
                let (r, g, b) = if pal {
                    let (mut v, mut u) = (da[x] * sv * gain, db[x] * gain);
                    if let Some((pa, pb, pvg, pug)) = prev {
                        v = 0.5 * (v + pa[x] * pvg);
                        u = 0.5 * (u + pb[x] * pug);
                    }
                    (yy + 1.140 * v, yy - 0.395 * u - 0.581 * v, yy + 2.032 * u)
                } else {
                    let (i, q) = (da[x] * gain, db[x] * gain);
                    (yy + 0.956 * i + 0.619 * q, yy - 0.272 * i - 0.647 * q, yy - 1.106 * i + 1.703 * q)
                };
                let o = x * 4;
                rgba[o] = to_u8(r);
                rgba[o + 1] = to_u8(g);
                rgba[o + 2] = to_u8(b);
                rgba[o + 3] = 255;
            }
            for d in dsts.iter_mut() {
                d.copy_from_slice(rgba);
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

#[inline(always)]
fn to_u8(v: f32) -> u8 {
    (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}
