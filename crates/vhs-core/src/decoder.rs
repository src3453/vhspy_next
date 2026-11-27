//! コンポジット (NTSC / PAL) → RGB の受像機モデル。
//!
//! 入力はフィールドごとの全ラスタ (帰線区間込み) だが、受像機はそれを途切れのないライン列 (ストリーム) として受け、
//! フィールドの区切りは自分で同期信号から見つける。
//!
//! - 水平 AFC: 同期の後縁を検出し、時定数付きで追従する (遅い時間軸変動は吸収、速い変動は残る)。
//!   帰線区間の等化パルス (幅が半分) でも引っ張られるので、画面の上端がわずかに曲がる
//! - 垂直同期分離: 同期分離した信号を RC 積分し、幅広パルス (垂直同期) が続いて積分値がしきい値を超えたら
//!   垂直発振器をトリガする。発振器は自走周期 (垂直ホールド) を持ち、自走周期の終わり近くの窓の中でだけ
//!   同期を受け付ける。同期が弱い・雑音が多い・ホールドがずれていると自走になり、画面が上下に流れて
//!   帰線区間 (黒帯・VBI のデータ) が見えてくる。第 2 フィールドは半ライン遅れてトリガされるので、
//!   インターレースの上下位置 (出力の偶数/奇数行) も信号から決まる
//! - Y/C 分離: ノッチ (ドットクロール/クロスカラーが出る)、くし形 (NTSC 1H / PAL 2H)、
//!   または 3 次元 (フレームメモリとのフレームくし形。動く部分はラインくし形に切り替える動き適応型)。
//!   くし形はストリームの直前のラインを使うので、有効映像の 1 行目には VBI のデータが漏れ込む
//! - 色副搬送波 PLL: ラインごとのバースト位相に追従係数付きで同期する。バーストのない帰線区間は惰性で回る
//! - NTSC は I/Q、PAL は U/V を同期検波。ACC とカラーキラー付き
//! - PAL: スウィンギングバーストから PAL スイッチの向きを決め (アイデント)、
//!   1H 遅延線で前のラインと色差を平均する (位相誤差 → 彩度低下)。遅延線なしならハノーバーバーが出る
//! - 発振器の誤差: 色副搬送波 VCXO・水平発振器・垂直発振器それぞれの周波数のずれ・ドリフト・ワウ・フラッタ・ジッタ
//!   ([`crate::params::ReceiverClocks`])
//! - 字幕デコーダ: 垂直同期から数えたライン 21 のクローズドキャプションを解読する ([`crate::vbi`])

use crate::filter::{Biquad, Cascade, fractional_shift};
use crate::frame::{Field, RgbaFrame};
use crate::nr::NoiseReduction;
use crate::params::{ChromaDemod, DecoderParams, YcSeparation};
use crate::raster;
use crate::timing::*;
use crate::caption::CaptionDecoder;
use crate::clock::{CLOCK_CHROMA, CLOCK_HORIZONTAL, CLOCK_VERTICAL, ClockSim};
use crate::vbi;
use rayon::prelude::*;
use std::collections::VecDeque;

/// ラインを通算ライン番号で引くリングバッファ
struct Ring {
    ls: usize,
    cap: usize,
    data: Vec<f32>,
}

impl Ring {
    fn new(ls: usize, cap: usize) -> Self {
        Self { ls, cap, data: vec![0.0; ls * cap] }
    }

    #[inline]
    fn line(&self, sl: u64) -> &[f32] {
        let i = (sl % self.cap as u64) as usize;
        &self.data[i * self.ls..(i + 1) * self.ls]
    }

    #[inline]
    fn line_mut(&mut self, sl: u64) -> &mut [f32] {
        let i = (sl % self.cap as u64) as usize;
        &mut self.data[i * self.ls..(i + 1) * self.ls]
    }
}

pub struct Decoder {
    std: &'static Standard,
    /// AFC が推定している同期後縁位置 (ストリームをまたいで保持)
    h_sync_pos: f32,
    /// 次に来るはずの通算ライン (フィールドが連続しているかの確認)
    next_stream: Option<u64>,
    /// これより前のラインは (シーク・再初期化で) 途切れているので参照しない
    valid_from: u64,
    /// 今のフィールドのぶんの作業領域 (ストリームに足すライン数 × line_samples)
    corrections: Vec<f32>,
    chunk_aligned: Vec<f32>,
    chunk_aligned_c: Vec<f32>,
    chunk_luma: Vec<f32>,
    chunk_chroma: Vec<f32>,
    chunk_bursts: Vec<Burst>,
    /// AFC 補正後の直前の数ライン (くし形フィルタ用)
    tail: Ring,
    /// 3 次元 Y/C 分離のフレームメモリ (量子化した AFC 補正後のコンポジット信号)
    frame_mem: Ring,
    /// フレームメモリが途切れずに埋まっている最初の通算ライン
    frame_mem_from: Option<u64>,
    /// Y/C 分離後の輝度・色とラインごとの検波条件 (表示する窓を切り出すため数フィールドぶん持つ)
    luma: Ring,
    chroma: Ring,
    line_refs: Vec<LineRef>,
    /// 色副搬送波 PLL の直前のラインの位相
    pll_prev: Option<f32>,
    /// PAL アイデントの投票 (正なら sV = (-1)^通算ライン)
    pal_vote: f32,
    pal_prev_mu: Option<(u64, f32)>,
    /// 垂直同期分離の積分回路の状態 (0..1)
    v_int: f32,
    /// 積分値が下がるまで次のトリガを受け付けない
    v_armed: bool,
    /// 直前の垂直発振器のトリガ (帰線開始) の時刻 [通算ライン]
    v_last: f64,
    /// これより前には自走でトリガしない (電源投入・シーク直後に最初の垂直同期を待つ)
    v_free_hold: f64,
    /// 帰線の位置 (積分回路の遅れを引いた、幅広パルスの始まりに相当する位置) [通算ライン]。新しいものが後ろ
    retraces: VecDeque<f64>,
    /// 積分回路の遅れ (時定数ごと) [ライン]
    v_delay: Option<(f32, f64)>,
    /// 同期検波後の色差 (field_lines × active_width)。a = cos 側 (I / ±V)、b = sin 側 (Q / U)
    det_a: Vec<f32>,
    det_b: Vec<f32>,
    /// 表示するラインの輝度・色差 (active_width × field_lines)。NR と RGB 変換で共有する
    nr_y: Vec<f32>,
    nr_u: Vec<f32>,
    nr_v: Vec<f32>,
    noise_reduction: NoiseReduction,
    caption: CaptionDecoder,
    /// 字幕を読んだ直前の帰線 (同じ画面を 2 回読まない)
    caption_last: Option<i64>,
    teletext: crate::teletext::TeletextDecoder,
    /// 発振器の揺れの状態
    clk_chroma: ClockSim,
    clk_h: ClockSim,
    clk_v: ClockSim,
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

/// 表示用に持つフィールド数 (画面が流れているとき、前のフィールドにまたがる窓を切り出す)
const DISPLAY_FIELDS: usize = 3;

impl Decoder {
    pub fn new(s: &'static Standard) -> Self {
        let ls = s.line_samples;
        let chunk = s.stream_lines(1);
        let cap = DISPLAY_FIELDS * s.raster_lines;
        let na = s.field_lines * s.active_width;
        Self {
            std: s,
            h_sync_pos: s.sync_end as f32,
            next_stream: None,
            valid_from: 0,
            corrections: vec![0.0; chunk],
            chunk_aligned: vec![0.0; chunk * ls],
            chunk_aligned_c: Vec::new(),
            chunk_luma: vec![0.0; chunk * ls],
            chunk_chroma: vec![0.0; chunk * ls],
            chunk_bursts: vec![Burst::default(); chunk],
            tail: Ring::new(ls, 4),
            frame_mem: Ring::new(ls, 0),
            frame_mem_from: None,
            luma: Ring::new(ls, cap),
            chroma: Ring::new(ls, cap),
            line_refs: vec![LineRef::default(); cap],
            pll_prev: None,
            pal_vote: 0.0,
            pal_prev_mu: None,
            v_int: 0.0,
            v_armed: true,
            v_last: f64::NEG_INFINITY,
            v_free_hold: f64::NEG_INFINITY,
            retraces: VecDeque::new(),
            v_delay: None,
            det_a: vec![0.0; na],
            det_b: vec![0.0; na],
            nr_y: vec![0.0; na],
            nr_u: vec![0.0; na],
            nr_v: vec![0.0; na],
            noise_reduction: NoiseReduction::default(),
            caption: CaptionDecoder::default(),
            caption_last: None,
            teletext: Default::default(),
            clk_chroma: ClockSim::default(),
            clk_h: ClockSim::default(),
            clk_v: ClockSim::default(),
        }
    }

    /// 字幕デコーダ (表示中の字幕と、画面への描画)
    pub fn captions(&self) -> &CaptionDecoder {
        &self.caption
    }

    /// 文字放送受信機 (選んだページの画面と、画面への描画)
    pub fn teletext(&self) -> &crate::teletext::TeletextDecoder {
        &self.teletext
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
/// 隣り合うラインの μ の差は (ライン当たりの位相の進み) - 90°·sV になるので、全体で多数決を取る
/// (実機の PAL スイッチのフリップフロップをアイデント信号で正しい向きに揃えるのに相当)。
pub(crate) fn pal_ident(mus: &[Option<f32>], s: &Standard) -> f32 {
    use std::f32::consts::{FRAC_PI_4, TAU};
    let wrap = |a: f32| a - TAU * (a / TAU).round();
    let step = s.line_phase_step() as f32 * std::f32::consts::FRAC_PI_2;
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

/// 垂直同期分離の積分回路が、幅広パルスの始まりから何ライン遅れてしきい値を超えるか
/// (理想的な信号で求めておき、帰線の位置から差し引く)
fn vsync_delay(s: &'static Standard, alpha: f32) -> f64 {
    let ls = s.line_samples;
    let start = s.vsync_stream_pos(1, 0) as u64;
    let mut line = vec![0f32; ls];
    let mut v = 0f32;
    let thr = s.ire.sync / 2.0;
    for sl in start - 40..start + 12 {
        raster::render_blanking(s, &raster::line_info(s, sl), &mut line);
        for (n, &x) in line.iter().enumerate() {
            v += alpha * ((x < thr) as u8 as f32 - v);
            if sl >= start && v > V_TRIGGER {
                return (sl - start) as f64 + n as f64 / ls as f64;
            }
        }
    }
    0.0
}

/// 積分回路のトリガ・再アームのしきい値
const V_TRIGGER: f32 = 0.5;
const V_REARM: f32 = 0.3;

impl Decoder {
    /// 1 フィールドぶんの信号を受けて画面を描く。戻り値は書いた行の偶奇 (0 = 偶数行、1 = 奇数行)。
    /// 受像機は同期信号から帰線の位置を決めるので、画面が流れているときは入力のフィールドの偶奇と一致しない
    pub fn decode_field(&mut self, field: &Field, p: &DecoderParams, playback_nr: bool, out: &mut RgbaFrame) -> u8 {
        let s = field.std;
        assert_eq!((out.width, out.height), s.image_size());
        if !std::ptr::eq(s, self.std) {
            *self = Self::new(s);
        }
        let ls = s.line_samples;
        let lv = s.ire.with_setup(p.setup);
        let pal = s.is_pal();
        let start = field.stream_line(0);
        let n_new = s.stream_lines(field.parity);
        let end = start + n_new as u64;
        if self.next_stream != Some(start) {
            // 途切れた (最初のフィールド・シーク): 過去のラインは使わない。垂直発振器はすぐ引き込める状態にする
            self.valid_from = start;
            self.frame_mem_from = None;
            self.pll_prev = None;
            self.pal_prev_mu = None;
            self.retraces.clear();
            let period = self.free_period(p);
            self.v_last = start as f64 - period * (1.0 - p.v_sync_window.clamp(0.02, 0.9) as f64);
            self.v_free_hold = start as f64 + period;
            self.caption.reset();
            self.teletext.reset();
            self.clk_chroma.reset();
            self.clk_h.reset();
            self.clk_v.reset();
        }
        self.next_stream = Some(end);

        // --- 水平 AFC (逐次) ---
        // 水平発振器は次の同期の位置を自分の周期で予測し、検出した同期との差 (位相比較器は ±半ラインで折り返す)
        // に追従係数を掛けて引き寄せる。発振器が公称より速い (周期が短い) と予測は前へずれる
        let k = p.h_pll.clamp(0.0, 1.0);
        let line_time = s.line_time() as f64;
        let half = 0.5 * ls as f32;
        let wrap_half = |d: f32| d - ls as f32 * (d / ls as f32).round();
        for i in 0..n_new {
            let clk = self.clk_h.step(&p.clocks.horizontal, start + i as u64, line_time, CLOCK_HORIZONTAL);
            let pred = self.h_sync_pos - (clk.freq_err * ls as f64) as f32;
            let mut pos_h = pred;
            if let Some(pos) = find_sync_edge(field.line(i), s) {
                pos_h = pred + k * wrap_half(pos - pred);
            }
            // 同期が外れて流れていくときも、ラインの頭からの位置として持つ
            let rel = pos_h - s.sync_end as f32;
            if rel.abs() > half {
                pos_h = s.sync_end as f32 + wrap_half(rel);
            }
            self.h_sync_pos = pos_h;
            self.corrections[i] = pos_h + (clk.jitter_s * s.fs as f64) as f32 - s.sync_end as f32;
        }
        let corr = &self.corrections;
        self.chunk_aligned[..n_new * ls].par_chunks_mut(ls).enumerate().for_each(|(i, dst)| {
            let prev = (i > 0).then(|| field.line(i - 1));
            let next = (i + 1 < n_new).then(|| field.line(i + 1));
            shift_across_lines(prev, field.line(i), next, dst, -corr[i], lv.blank);
        });
        // S 映像: C も同じ補正で揃える
        let separate = field.separate;
        if separate {
            self.chunk_aligned_c.resize(self.chunk_aligned.len(), 0.0);
            self.chunk_aligned_c[..n_new * ls].par_chunks_mut(ls).enumerate().for_each(|(i, dst)| {
                let c = |j: usize| field.chroma_line(j).unwrap();
                let prev = (i > 0).then(|| c(i - 1));
                let next = (i + 1 < n_new).then(|| c(i + 1));
                shift_across_lines(prev, c(i), next, dst, -corr[i], 0.0);
            });
        }

        // --- 垂直同期分離と垂直発振器 (逐次) ---
        self.vertical(field, n_new, p);

        // --- Y/C 分離とバースト検出 (ライン並列) ---
        let notch = Cascade::new(&[Biquad::notch_at(s.fsc, 1.0, s.fs)]);
        let c_bp = Cascade::new(&[Biquad::bandpass_at(s.fsc, 1.5, s.fs)]);
        // くし形の相手: 副搬送波が 180° ずれたライン (NTSC は 1H、PAL は V の符号も揃う 2H 前)
        let comb_lag: u64 = if pal { 2 } else { 1 };
        let use_3d = p.yc_separation == YcSeparation::Comb3d && !separate;
        let use_comb = p.yc_separation == YcSeparation::Comb;
        // フレームくし形の相手: 副搬送波が 180° ずれた同じ位置のライン
        // (NTSC は 1 フレーム = 525 ライン前、PAL は 2 フレーム = 1250 ライン前で V の符号も揃う)
        let frame_lag = s.frame_total_lines as u64 * if pal { 2 } else { 1 };
        if use_3d {
            let cap = frame_lag as usize + s.raster_lines;
            if self.frame_mem.cap != cap {
                self.frame_mem = Ring::new(ls, cap);
                self.frame_mem_from = None;
            }
        } else {
            self.frame_mem_from = None;
        }
        // 動き適応なし (None) なら常にフレームくし形
        let motion_thr = p.motion_adaptive.then(|| p.motion_threshold_ire.max(1e-3));
        let valid_from = self.valid_from;
        let frame_ok_from = self.frame_mem_from.map(|f| f.max(valid_from));
        {
            let (chunk_aligned, chunk_aligned_c, tail, frame_mem) =
                (&self.chunk_aligned, &self.chunk_aligned_c, &self.tail, &self.frame_mem);
            let aligned_line = |sl: u64| -> Option<&[f32]> {
                if sl >= start {
                    let i = (sl - start) as usize;
                    Some(&chunk_aligned[i * ls..(i + 1) * ls])
                } else if sl >= valid_from && start - sl <= tail.cap as u64 {
                    Some(tail.line(sl))
                } else {
                    None
                }
            };
            let burst_gate = s.burst_start..s.burst_end + 16;
            self.chunk_luma[..n_new * ls]
                .par_chunks_mut(ls)
                .zip(self.chunk_chroma[..n_new * ls].par_chunks_mut(ls))
                .zip(self.chunk_bursts[..n_new].par_iter_mut())
                .enumerate()
                .for_each(|(i, ((y, c), burst))| {
                    let sl = start + i as u64;
                    let x = &chunk_aligned[i * ls..(i + 1) * ls];
                    let frame_ref = frame_ok_from
                        .filter(|&f| use_3d && sl >= f + frame_lag)
                        .map(|_| frame_mem.line(sl - frame_lag));
                    let prev = (sl >= comb_lag).then(|| aligned_line(sl - comb_lag)).flatten();
                    if separate {
                        // S 映像: Y/C 分離は要らない (C は帯域外の雑音だけ落とす)
                        let xc = &chunk_aligned_c[i * ls..(i + 1) * ls];
                        let mut bp = c_bp;
                        for n in 0..ls {
                            y[n] = x[n];
                            c[n] = bp.process(xc[n]);
                        }
                    } else if let Some(xf) = frame_ref {
                        separate_3d(x, xf, prev, y, c, c_bp, motion_thr);
                    } else if let (true, Some(prev)) = (use_comb || use_3d, prev) {
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
        }

        // --- ストリームのバッファへ移す ---
        for i in 0..n_new {
            let sl = start + i as u64;
            let src = i * ls..(i + 1) * ls;
            self.luma.line_mut(sl).copy_from_slice(&self.chunk_luma[src.clone()]);
            self.chroma.line_mut(sl).copy_from_slice(&self.chunk_chroma[src.clone()]);
            if i + self.tail.cap >= n_new {
                self.tail.line_mut(sl).copy_from_slice(&self.chunk_aligned[src.clone()]);
            }
            if use_3d {
                let dst = self.frame_mem.line_mut(sl);
                dst.copy_from_slice(&self.chunk_aligned[src]);
                quantize_line(dst, p.frame_buffer_bits, &lv);
            }
        }
        if use_3d && self.frame_mem_from.is_none() {
            self.frame_mem_from = Some(start);
        }

        // --- 色副搬送波 PLL と PAL アイデント (逐次) ---
        self.lock_subcarrier(start, n_new, p);

        // --- 表示する窓 (垂直同期から決まる) ---
        let fl = s.field_lines;
        let aw = s.active_width;
        // 幅広パルスの始まりから有効映像の 1 行目までのライン数
        let top = s.vsync_to_active();
        let oldest = end.saturating_sub(self.luma.cap as u64).max(self.valid_from);
        let vpos = p.vertical_position as f64;
        let window = |r: f64| {
            let a = r + top + vpos;
            let j0 = (a - 0.25).ceil();
            let row0 = ((2.0 * (j0 - a) + 0.5).floor() as i64).clamp(0, 1) as u8;
            (j0 as i64, row0)
        };
        let chosen = self
            .retraces
            .iter()
            .rev()
            .map(|&r| (r, window(r)))
            .find(|&(_, (j0, _))| j0 + fl as i64 <= end as i64 && j0 >= oldest as i64)
            .or_else(|| {
                // まだ垂直同期を見ていない (電源投入直後): 信号どおりの位置に描く
                let r = s.vsync_stream_pos(field.frame, field.parity);
                Some((r, window(r)))
            });
        let (retrace, (j0, row0)) = chosen.unwrap();
        let line_ok = |j: i64| j >= oldest as i64 && j < end as i64;

        // --- 同期検波 (ライン並列) ---
        // 色差のローパス。det_a (NTSC: I / PAL: V) と det_b (NTSC: Q / PAL: U) で帯域を変えられる
        let demod = if pal { ChromaDemod::Iq } else { p.chroma_demod };
        let (bw_a, bw_b) = match demod {
            ChromaDemod::Iq => (p.chroma_bw_mhz, p.chroma_bw_mhz),
            ChromaDemod::IqWide => (p.chroma_bw_mhz, p.narrow_bw_mhz),
            ChromaDemod::RyBy | ChromaDemod::Xz => (p.narrow_bw_mhz, p.narrow_bw_mhz),
        };
        let c_lp_a = Cascade::butter_lowpass_at(2, bw_a.max(0.01) * 1e6, s.fs);
        let c_lp_b = if bw_b == bw_a { c_lp_a } else { Cascade::butter_lowpass_at(2, bw_b.max(0.01) * 1e6, s.fs) };
        let matrix = chroma_matrix(demod, p);
        // NTSC は I/Q 軸 (U/V から 33°) で検波する。PAL は U/V 軸のまま
        let hue = if pal { p.hue_deg.to_radians() } else { p.hue_deg.to_radians() + 33f32.to_radians() };
        let (active_start, active_end) = (s.active_start, s.active_end());
        // 輝度遅延線: 実機は色の経路 (バンドパス + 復調後のローパス) の遅れに合わせて輝度を遅らせる。
        // ここでは同じだけ色を前へ出す
        let luma_delay = if separate || use_comb || use_3d { 0.0 } else { notch.group_delay(1e3, s.fs) };
        // I/Q 復調では帯域の広い (遅れの小さい) I を遅延線で Q に揃える。ここではそれぞれの遅れだけ前へ出す
        let lead = |lp: &Cascade| (c_bp.group_delay(s.fsc, s.fs) + lp.group_delay(1e3, s.fs) - luma_delay).round().max(0.0) as usize;
        let (lead_a, lead_b) = (lead(&c_lp_a), lead(&c_lp_b));
        let lead_max = lead_a.max(lead_b);
        let (chroma, line_refs) = (&self.chroma, &self.line_refs);
        self.det_a.par_chunks_mut(aw).zip(self.det_b.par_chunks_mut(aw)).enumerate().for_each(|(k, (da, db))| {
            let j = j0 + k as i64;
            if !line_ok(j) {
                da.fill(0.0);
                db.fill(0.0);
                return;
            }
            let c = chroma.line(j as u64);
            let psi = line_refs[(j as u64 % line_refs.len() as u64) as usize].phase;
            let mut cos_t = [0f32; 4];
            let mut sin_t = [0f32; 4];
            for q in 0..4 {
                let a = q as f32 * std::f32::consts::FRAC_PI_2 + psi + hue;
                cos_t[q] = a.cos();
                sin_t[q] = a.sin();
            }
            let (mut alp, mut blp) = (c_lp_a, c_lp_b);
            let (mut a, mut b) = (0.0, 0.0);
            for n in 0..active_end + lead_max {
                if n < ls {
                    a = alp.process(2.0 * c[n] * cos_t[n & 3]);
                    b = blp.process(2.0 * c[n] * sin_t[n & 3]);
                }
                // ラインの終わりを越えたぶんは最後の値を保つ
                if n >= active_start + lead_a && n < active_end + lead_a {
                    da[n - active_start - lead_a] = a;
                }
                if n >= active_start + lead_b && n < active_end + lead_b {
                    db[n - active_start - lead_b] = b;
                }
            }
        });

        // --- 輝度・色差 (PAL は遅延線で前のラインと平均) ---
        let line_ref = |j: i64| self.line_refs[(j as u64 % self.line_refs.len() as u64) as usize];
        for k in 0..fl {
            let j = j0 + k as i64;
            if !line_ok(j) {
                // ストリームの外 (まだ来ていない・もうない): 何も映らない
                let r = k * aw..(k + 1) * aw;
                self.nr_y[r.clone()].fill(-lv.black);
                self.nr_u[r.clone()].fill(0.0);
                self.nr_v[r].fill(0.0);
                continue;
            }
            let y_line = self.luma.line(j as u64);
            let (da, db) = (&self.det_a[k * aw..(k + 1) * aw], &self.det_b[k * aw..(k + 1) * aw]);
            let LineRef { sv, gain, .. } = line_ref(j);
            let prev = (pal && p.pal_delay_line && k > 0 && line_ok(j - 1)).then(|| {
                let r = line_ref(j - 1);
                (&self.det_a[(k - 1) * aw..k * aw], &self.det_b[(k - 1) * aw..k * aw], r.sv * r.gain, r.gain)
            });
            for x in 0..aw {
                let i = k * aw + x;
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
        let parity = row0 as usize;
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
                    let [mr, mg, mb] = matrix;
                    (yy + mr[0] * u[x] + mr[1] * v[x], yy + mg[0] * u[x] + mg[1] * v[x], yy + mb[0] * u[x] + mb[1] * v[x])
                };
                let o = x * 4;
                dst[o] = to_u8(r);
                dst[o + 1] = to_u8(g);
                dst[o + 2] = to_u8(b);
                dst[o + 3] = 255;
            }
        });

        // --- 字幕デコーダ: 垂直同期から数えたライン 21 (第 1 フィールド) / 284 (第 2 フィールド) を読む ---
        if p.caption_display && !pal {
            self.caption.set_channel(p.caption_channel);
            let key = (2.0 * retrace).round() as i64;
            if self.caption_last != Some(key) {
                self.caption_last = Some(key);
                let j = (retrace + 21.0 - s.vsync_line[0] as f64 + 0.5 * row0 as f64).round() as i64;
                let pair = line_ok(j).then(|| vbi::slice_caption(s, self.luma.line(j as u64))).flatten();
                self.caption.feed(row0, pair);
            }
        } else {
            self.caption.reset();
            self.caption_last = None;
        }

        // --- 文字放送受信機: 14〜16・21 H (第 2 フィールドは 277〜279・284 H) を読む ---
        if p.teletext_display && !pal {
            let page = p.teletext_page.clamp(1, 99);
            self.teletext.receiver.select(1, p.teletext_program.min(999), page);
            let key = (2.0 * retrace).round() as i64;
            if self.teletext.last != Some(key) {
                self.teletext.last = Some(key);
                let packets: Vec<_> = vbi::JTES_LINES[0]
                    .iter()
                    .filter_map(|&n| {
                        let j = (retrace + n as f64 - s.vsync_line[0] as f64 + 0.5 * row0 as f64).round() as i64;
                        line_ok(j).then(|| vbi::slice_jtes(s, self.luma.line(j as u64))).flatten()
                    })
                    .collect();
                self.teletext.feed(packets);
            }
        } else if self.teletext.last.is_some() {
            self.teletext.reset();
        }
        row0
    }

    /// 垂直発振器の自走周期 [ライン]
    fn free_period(&self, p: &DecoderParams) -> f64 {
        self.std.total_lines_per_field / (1.0 + p.v_hold_pct.clamp(-50.0, 50.0) as f64 / 100.0)
    }

    /// 垂直同期分離 (同期分離 → RC 積分 → しきい値) と、窓付きでトリガされる垂直発振器
    fn vertical(&mut self, field: &Field, n_new: usize, p: &DecoderParams) {
        let s = self.std;
        let ls = s.line_samples;
        let alpha = 1.0 - (-1.0 / (p.v_integrator_us.max(1.0) * 1e-6 * s.fs)).exp();
        let delay = match self.v_delay {
            Some((a, d)) if a == alpha => d,
            _ => {
                let d = vsync_delay(s, alpha);
                self.v_delay = Some((alpha, d));
                d
            }
        };
        let nominal = self.free_period(p);
        let window = p.v_sync_window.clamp(0.02, 0.9) as f64;
        let thr = s.ire.sync / 2.0;
        let start = field.stream_line(0);
        let line_time = s.line_time() as f64;
        for i in 0..n_new {
            let line = field.line(i);
            let sl = start + i as u64;
            let base = sl as f64;
            // 垂直発振器の周波数誤差で自走周期が変わり、ジッタで自走のトリガ時刻が揺れる
            let clk = self.clk_v.step(&p.clocks.vertical, sl, line_time, CLOCK_VERTICAL);
            let period = nominal / (1.0 + clk.freq_err);
            let jitter = clk.jitter_s / line_time;
            // 自走のトリガがこのラインの中に来るか
            let mut free_at = (self.v_last + period + jitter).max(self.v_free_hold);
            for (n, &x) in line.iter().enumerate() {
                self.v_int += alpha * ((x < thr) as u8 as f32 - self.v_int);
                let t = base + n as f64 / ls as f64;
                if self.v_int < V_REARM {
                    self.v_armed = true;
                }
                if self.v_armed && self.v_int > V_TRIGGER && t - self.v_last >= period * (1.0 - window) {
                    // 同期によるトリガ
                    self.v_armed = false;
                    self.v_last = t;
                    self.push_retrace(t - delay);
                    free_at = self.v_last + period + jitter;
                } else if t >= free_at {
                    // 自走 (同期が来ない・窓の外)
                    self.v_last = free_at;
                    self.push_retrace(free_at - delay);
                    free_at = self.v_last + period + jitter;
                }
            }
        }
    }

    fn push_retrace(&mut self, r: f64) {
        self.retraces.push_back(r);
        while self.retraces.len() > 8 {
            self.retraces.pop_front();
        }
    }

    /// バーストからラインごとの基準位相・PAL スイッチ・色のゲインを決める。
    ///
    /// バースト相関の位相 μ は NTSC では副搬送波位相 φ そのもの、PAL では φ - 45°·sV (スウィンギングバースト)。
    /// - PAL アイデント: 隣り合うラインの μ の差は (ライン当たりの位相の進み) - 90°·sV になるので、
    ///   投票を積み重ねて PAL スイッチ (フリップフロップ) の向きを決める
    /// - PLL: 前のラインの基準位相 + ライン当たりの進み (受像機の VCXO の周波数誤差ぶんを含む) を予測とし、
    ///   バーストとの差に `burst_pll` を掛けて追従する (1 次ループなので周波数ずれは定常位相誤差として残る)。
    ///   バーストが弱い・ない (帰線区間) ラインは予測のまま惰性で回る。
    ///   VCXO のジッタはループの状態ではなく、そのラインの検波に使う基準位相に乗る
    fn lock_subcarrier(&mut self, start: u64, n_new: usize, p: &DecoderParams) {
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, TAU};
        let s = self.std;
        let wrap = |a: f32| a - TAU * (a / TAU).round();
        let step = s.line_phase_step() as f32 * FRAC_PI_2;
        let pal = s.is_pal();
        let k = p.burst_pll.clamp(0.0, 1.0);
        let line_time = s.line_time() as f64;
        let cap = self.line_refs.len() as u64;
        for i in 0..n_new {
            let sl = start + i as u64;
            let clk = self.clk_chroma.step(&p.clocks.chroma, sl, line_time, CLOCK_CHROMA);
            // VCXO が 1 ラインのあいだに余分に進む位相と、このラインの位相ジッタ
            let drift = (std::f64::consts::TAU * s.fsc as f64 * clk.freq_err * line_time) as f32;
            let jitter = (std::f64::consts::TAU * s.fsc as f64 * clk.jitter_s) as f32;
            let b = self.chunk_bursts[i];
            let valid = b.amp >= 3.0;
            let mu = b.zi.atan2(b.zr) - FRAC_PI_2;
            let alt = if sl & 1 == 0 { 1.0 } else { -1.0 };
            let sv = if pal {
                if valid {
                    if let Some((psl, pmu)) = self.pal_prev_mu {
                        if psl + 1 == sl {
                            let d = wrap(mu - pmu - step);
                            // d ≒ -90°·sV。±45° 以内のずれなら判定に使う
                            if d.abs() > FRAC_PI_4 && d.abs() < 3.0 * FRAC_PI_4 {
                                self.pal_vote = 0.95 * self.pal_vote - d.signum() * alt;
                            }
                        }
                    }
                }
                self.pal_prev_mu = valid.then_some((sl, mu));
                let s0 = if self.pal_vote < 0.0 { -1.0 } else { 1.0 };
                s0 * alt
            } else {
                1.0
            };
            // バーストから測った副搬送波位相
            let measured = if pal { mu + FRAC_PI_4 * sv } else { mu };
            let phase = match self.pll_prev {
                None => measured,
                Some(ph) => {
                    let pred = wrap(ph + step + drift);
                    if !valid {
                        pred
                    } else if k >= 1.0 {
                        // 即時同期 (ラインごとにバーストへ合わせる)
                        measured
                    } else {
                        pred + k * wrap(measured - pred)
                    }
                }
            };
            self.pll_prev = Some(phase);

            let mut gain = p.saturation / s.ire.with_setup(p.setup).span();
            if p.acc {
                gain *= (s.ire.burst / b.amp.max(1e-3)).min(8.0);
            }
            if p.color_killer && !valid {
                gain = 0.0;
            }
            self.line_refs[(sl % cap) as usize] = LineRef { phase: phase + jitter, sv, gain };
        }
    }
}

/// 水平 AFC の補正でラインをずらす。ずれが大きい (同期が外れて流れている) ときは、
/// はみ出したぶんを前後のラインから持ってくる (帰線区間の黒帯と同期が画面に入ってくる)
fn shift_across_lines(prev: Option<&[f32]>, cur: &[f32], next: Option<&[f32]>, dst: &mut [f32], shift: f32, fill: f32) {
    // 小さいずれは 1 ラインだけで済ませる (はみ出すのは帰線区間の端だけ)
    if shift.abs() < 16.0 {
        fractional_shift(cur, dst, shift, fill);
        return;
    }
    let ls = cur.len();
    let mut buf = vec![fill; 3 * ls];
    if let Some(p) = prev {
        buf[..ls].copy_from_slice(p);
    }
    buf[ls..2 * ls].copy_from_slice(cur);
    if let Some(n) = next {
        buf[2 * ls..].copy_from_slice(n);
    }
    let mut out = vec![0f32; 3 * ls];
    fractional_shift(&buf, &mut out, shift, fill);
    dst.copy_from_slice(&out[ls..2 * ls]);
}

/// NTSC: I/Q 検波出力 (I, Q) から色差 (R-Y, G-Y, B-Y) への係数。
///
/// R-Y/B-Y・X/Z 復調は「その軸で検波する」方式だが、検波後のローパスが同じなら
/// I/Q で検波してから軸へ射影したものと同じなので、ここでは係数の違いとして持つ
fn chroma_matrix(demod: ChromaDemod, p: &DecoderParams) -> [[f32; 2]; 3] {
    // B-Y 軸を 0° としたときの角度 θ の軸で検波した出力 (I 軸 = 123°、Q 軸 = 33°)
    let axis = |deg: f32| {
        let t = (deg - 33.0).to_radians();
        [t.sin(), t.cos()]
    };
    let comb = |a: [f32; 2], ka: f32, b: [f32; 2], kb: f32| [ka * a[0] + kb * b[0], ka * a[1] + kb * b[1]];
    match demod {
        ChromaDemod::Iq | ChromaDemod::IqWide => [[0.956, 0.619], [-0.272, -0.647], [-1.106, 1.703]],
        ChromaDemod::RyBy => {
            // V = 0.877(R-Y)、U = 0.493(B-Y)
            let ry = comb(axis(90.0), 1.0 / 0.877, [0.0; 2], 0.0);
            let by = comb(axis(0.0), 1.0 / 0.493, [0.0; 2], 0.0);
            // G-Y 増幅トランジスタのベースへ R-Y・B-Y を足し込むマトリクス
            [ry, comb(ry, -0.51, by, -0.19), by]
        }
        ChromaDemod::Xz => {
            // 色差増幅管 (増幅率 −A、ここでは反転込みで X・Z の利得に含める) のカソードは 3 本共通で、
            // E_K = k(E_X + E_Z)。グリッドから見ると −E_K が加わる:
            //   R-Y = −A(E_X − E_K)、B-Y = −A(E_Z − E_K)、G-Y = −A(−E_K) (G-Y 管のグリッドは無入力)
            let k = p.xz_cathode_k.clamp(0.0, 1.0 / 3.0);
            let x = comb(axis(p.xz_x_axis_deg), p.xz_x_gain, [0.0; 2], 0.0);
            let z = comb(axis(p.xz_z_axis_deg), p.xz_z_gain, [0.0; 2], 0.0);
            [comb(x, 1.0 - k, z, -k), comb(x, -k, z, -k), comb(x, -k, z, 1.0 - k)]
        }
    }
}

/// 画面バッファへ書き込むときの量子化 (フレームメモリの A/D 変換)。
/// 範囲は同期先端から、白 + 色の振幅のぶんのピーク (NTSC なら約 -40〜150 IRE) まで。範囲外は張り付く
fn quantize_line(buf: &mut [f32], bits: u32, lv: &Levels) {
    if bits == 0 {
        return;
    }
    let bits = bits.min(16);
    let lo = lv.sync;
    let hi = lv.white + 0.35 * (lv.white - lv.sync);
    let steps = ((1u32 << bits) - 1) as f32;
    let (scale, inv) = (steps / (hi - lo), (hi - lo) / steps);
    for v in buf.iter_mut() {
        let q = ((*v - lo) * scale).round().clamp(0.0, steps);
        *v = lo + q * inv;
    }
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
