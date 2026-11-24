//! DV (民生用デジタルビデオ。IEC 61834 / SMPTE 314M の 25 Mbps 方式) の記録と再生。
//!
//! ビットストリームは作らず、画質に効く仕組みだけを再現する:
//!
//! 1. 13.5 MHz で標本化し直した 720 画素の YCbCr。NTSC は 4:1:1、PAL は 4:2:0 (色差はフィールドごとに間引く)
//! 2. 8×8 DCT。動きのあるブロック (隣のラインが別フィールドで食い違う) は 2-4-8 DCT
//!    (上下のラインの和と差を、それぞれ 4×8 で変換) に切り替える
//! 3. 係数の重み付けと、クラス (ブロックの細かさ) × QNO (量子化番号) × エリア (周波数帯) による量子化
//! 4. **ビデオセグメント単位の固定長符号化**: 画面のあちこちから集めた 5 マクロブロックを 1 セグメントとし、
//!    セグメントの符号量が一定 (25 Mbps) に収まる最も細かい QNO を選ぶ。複雑な絵柄の周りだけ粗くなる。
//!    収まりきらない分は、ブロック自身の領域 → 同じマクロブロックの空き → セグメント内の空き の順に詰め、
//!    それでも入らない高域の係数は捨てる
//! 5. テープのエラーはノイズではなくマクロブロック単位の欠落になる。再生機はそれを前のフレームの
//!    同じ位置で埋める (エラー修整) ので、モザイク状の残像になる。片方のヘッドが詰まると
//!    1 本おきのトラックが読めず、両方なら画面が止まる
//! 6. バーストノイズ (データ化け): 読み取り順に連続した区間でバイト誤りが起きる。マクロブロックごとの
//!    誤り訂正 (内符号) で直せる数なら影響なし、多すぎれば検出して修整する。検出を逃した誤り (または
//!    誤り訂正なし) はそのまま復号され、ブロックの途中から可変長符号の同期が外れてでたらめな係数になる
//!    (縞・格子模様のブロック)。DV のブロックは固定位置から始まるので、化けるのは当たったブロックだけ
//!
//! 処理はフィールドごとに呼ばれる。DV はフレーム (2 フィールド) 単位で符号化するので、
//! 今のフィールドのラインと、もう一方のフィールドの直前のラインを織り込んだフレームを毎回符号化する
//! (遅延なし)。エラーの発生はフレーム単位で決め、同じフレームの 2 フィールドで揃える。
//!
//! 量子化の表やハフマン符号の長さは規格の傾向に合わせた近似。

use crate::capture::codec::{ZIGZAG, fdct, idct};
use crate::frame::VideoFrame;
use crate::params::DvParams;
use crate::rng::Rng;
use crate::timing::Standard;
use rayon::prelude::*;
use std::f32::consts::{FRAC_1_SQRT_2, PI};
use std::sync::OnceLock;

const STREAM_DV: u64 = 41;
const STREAM_DV_GARBAGE: u64 = 42;

/// 1 シンクブロック (マクロブロック 1 つぶん) のデータのバイト数
const MB_BYTES: usize = 77;
/// 内符号 (リード・ソロモン (85, 77)) で訂正できるバイト誤りの数
const ECC_CORRECTABLE: u32 = 4;

/// フレームごとのマクロブロックの読み取りの状態
#[derive(Clone, Copy, Debug, PartialEq)]
enum MbState {
    Ok,
    /// 読めない (誤りを検出した) → 修整する
    Lost,
    /// 誤りを見逃して化けたまま復号する (値はバイト誤り率)
    Corrupt(f32),
}

/// DV の標本化周波数 (BT.601) [Hz]
const DV_FS: f32 = 13.5e6;
const DV_WIDTH: usize = 720;
/// 1 セグメントのマクロブロック数
const SEG_MBS: usize = 5;
/// 1 マクロブロックのブロック数 (輝度 4 + Cb + Cr)
const MB_BLOCKS: usize = 6;
const SEG_BLOCKS: usize = SEG_MBS * MB_BLOCKS;
/// 1 スーパーブロックのマクロブロック数
const SB_MBS: usize = 27;
/// 画面の横に並ぶスーパーブロックの数
const SB_COLS: usize = 5;
/// スーパーブロック 1 段の高さ [ライン]
const SB_ROW_LINES: usize = 48;
/// ブロックの領域のうち AC 係数に使える量 [bit]。輝度 14 バイト、色差 10 バイトから
/// 固定部分 (DC 9 bit + DCT モード 1 bit + クラス 2 bit) を除いたもの
const LUMA_AREA: f32 = 14.0 * 8.0 - 12.0;
const CHROMA_AREA: f32 = 10.0 * 8.0 - 12.0;
const EOB_BITS: u32 = 4;

/// QNO + クラスのオフセットから、エリア 0..3 の量子化ステップ (2 の累乗の指数) を引く表
const QUANT_SHIFTS: [[u8; 4]; 22] = [
    [3, 3, 4, 4],
    [3, 3, 4, 4],
    [2, 3, 3, 4],
    [2, 3, 3, 4],
    [2, 2, 3, 3],
    [2, 2, 3, 3],
    [1, 2, 2, 3],
    [1, 2, 2, 3],
    [1, 1, 2, 2],
    [1, 1, 2, 2],
    [0, 1, 1, 2],
    [0, 1, 1, 2],
    [0, 0, 1, 1],
    [0, 0, 1, 1],
    [0, 0, 0, 1],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
    [0, 0, 0, 0],
];
/// クラス 0 (平坦) 〜 3 (細かい) のオフセット。値が大きいほど細かく量子化する。クラス 3 はさらに 1 段粗い
const CLASS_OFFSET: [usize; 4] = [6, 3, 0, 1];
/// クラスを決める AC 係数の最大値のしきい値 (QNO 15 での量子化値)
const CLASS_THRESH: [f32; 3] = [12.0, 36.0, 96.0];
const QNO_MAX: usize = 15;

/// ジグザグ順の位置 → エリア番号
#[inline(always)]
fn area_of(zz: usize) -> usize {
    match zz {
        0..=5 => 0,
        6..=20 => 1,
        21..=42 => 2,
        _ => 3,
    }
}

/// (ゼロの連続数, 振幅) のハフマン符号の長さ [bit] (符号ビット込み) の近似
#[inline]
fn vlc_len(run: u32, amp: u32) -> u32 {
    let a = 32 - amp.leading_zeros();
    if run == 0 {
        // 大きい振幅はエスケープ (7 bit + 振幅 8 bit + 符号)
        return if amp <= 22 { 1 + 2 * a } else { 16 };
    }
    let r = 32 - run.leading_zeros();
    let len = 2 + 2 * a + 2 * r;
    if run > 14 || len > 16 {
        // ゼロの連続だけの符号 (13 bit) + 振幅の符号
        13 + vlc_len(0, amp)
    } else {
        len
    }
}

fn dct_mat<const N: usize>() -> [[f32; N]; N] {
    let mut m = [[0f32; N]; N];
    for (k, row) in m.iter_mut().enumerate() {
        let c = if k == 0 { (1.0 / N as f32).sqrt() } else { (2.0 / N as f32).sqrt() };
        for (n, v) in row.iter_mut().enumerate() {
            *v = c * ((2 * n + 1) as f32 * k as f32 * PI / (2 * N) as f32).cos();
        }
    }
    m
}

fn m8() -> &'static [[f32; 8]; 8] {
    static M: OnceLock<[[f32; 8]; 8]> = OnceLock::new();
    M.get_or_init(dct_mat::<8>)
}

fn m4() -> &'static [[f32; 4]; 4] {
    static M: OnceLock<[[f32; 4]; 4]> = OnceLock::new();
    M.get_or_init(dct_mat::<4>)
}

/// 2-4-8 DCT: 上下 2 ライン (別フィールド) の和と差を、縦 4 点 × 横 8 点で変換する。
/// 出力の上 4 行が和、下 4 行が差の係数。正規直交
fn fdct248(x: &[f32; 64]) -> [f32; 64] {
    let (m4, m8) = (m4(), m8());
    let mut t = [0f32; 64];
    for c in 0..8 {
        let mut s = [0f32; 4];
        let mut d = [0f32; 4];
        for k in 0..4 {
            let (a, b) = (x[2 * k * 8 + c], x[(2 * k + 1) * 8 + c]);
            s[k] = (a + b) * FRAC_1_SQRT_2;
            d[k] = (a - b) * FRAC_1_SQRT_2;
        }
        for k in 0..4 {
            let (mut vs, mut vd) = (0.0, 0.0);
            for n in 0..4 {
                vs += m4[k][n] * s[n];
                vd += m4[k][n] * d[n];
            }
            t[k * 8 + c] = vs;
            t[(4 + k) * 8 + c] = vd;
        }
    }
    let mut y = [0f32; 64];
    for r in 0..8 {
        for k in 0..8 {
            let mut v = 0.0;
            for n in 0..8 {
                v += m8[k][n] * t[r * 8 + n];
            }
            y[r * 8 + k] = v;
        }
    }
    y
}

fn idct248(y: &[f32; 64]) -> [f32; 64] {
    let (m4, m8) = (m4(), m8());
    let mut t = [0f32; 64];
    for r in 0..8 {
        for n in 0..8 {
            let mut v = 0.0;
            for k in 0..8 {
                v += m8[k][n] * y[r * 8 + k];
            }
            t[r * 8 + n] = v;
        }
    }
    let mut x = [0f32; 64];
    for c in 0..8 {
        for n in 0..4 {
            let (mut s, mut d) = (0.0, 0.0);
            for k in 0..4 {
                s += m4[k][n] * t[k * 8 + c];
                d += m4[k][n] * t[(4 + k) * 8 + c];
            }
            x[2 * n * 8 + c] = (s + d) * FRAC_1_SQRT_2;
            x[(2 * n + 1) * 8 + c] = (s - d) * FRAC_1_SQRT_2;
        }
    }
    x
}

/// 係数の重み (8-8 用, 2-4-8 用)。高い周波数ほど小さくして粗く量子化する
fn weights() -> &'static [[f32; 64]; 2] {
    static W: OnceLock<[[f32; 64]; 2]> = OnceLock::new();
    W.get_or_init(|| {
        let cs = |m: f32| (m * PI / 16.0).cos();
        let w = [
            1.0,
            cs(4.0) / (4.0 * cs(7.0) * cs(2.0)),
            cs(4.0) / (2.0 * cs(6.0)),
            1.0 / (2.0 * cs(5.0)),
            7.0 / 8.0,
            cs(4.0) / cs(3.0),
            cs(4.0) / cs(2.0),
            cs(4.0) / cs(1.0),
        ];
        let mut out = [[0f32; 64]; 2];
        for r in 0..8 {
            for c in 0..8 {
                out[0][r * 8 + c] = w[r] * w[c];
                // 2-4-8: 和の行は縦 2k 番目、差の行はその間の周波数とみなす
                let wv = if r < 4 { w[2 * r] } else { w[2 * (r - 4) + 1] };
                out[1][r * 8 + c] = wv * w[c];
            }
        }
        out
    })
}

/// 隣のライン (別フィールド) どうしの差が、同じフィールドのライン間の差より大きければ動きがあるとみなす
fn interlaced_motion(px: &[f32; 64]) -> bool {
    let (mut inter, mut intra) = (0.0f32, 0.0f32);
    for c in 0..8 {
        for r in 0..7 {
            inter += (px[r * 8 + c] - px[(r + 1) * 8 + c]).abs();
        }
        for r in 0..6 {
            intra += (px[r * 8 + c] - px[(r + 2) * 8 + c]).abs();
        }
    }
    inter / 56.0 > 1.6 * intra / 48.0 + 2.0
}

/// 8 bit の画像平面 (値は 0..255 の f32)
#[derive(Clone, Debug, Default)]
struct Plane {
    w: usize,
    data: Vec<f32>,
}

impl Plane {
    fn new(w: usize, h: usize) -> Self {
        Self { w, data: vec![0.0; w * h] }
    }
}

/// マクロブロック内の 1 ブロックの画素の位置
#[derive(Clone, Copy, Debug)]
enum BlockPos {
    /// 平面 `plane` の (x, y) から 8×8
    Rect { plane: u8, x: u16, y: u16 },
    /// NTSC の右端のマクロブロックの色差: 4×16 を 8×8 に並べる (上半分 → 左 4 列、下半分 → 右 4 列)
    Split { plane: u8, x: u16, y: u16 },
}

impl BlockPos {
    fn plane(self) -> usize {
        match self {
            BlockPos::Rect { plane, .. } | BlockPos::Split { plane, .. } => plane as usize,
        }
    }

    /// ブロック内の (列 u, 行 v) → 平面上の (x, y)
    #[inline(always)]
    fn coord(self, u: usize, v: usize) -> (usize, usize) {
        match self {
            BlockPos::Rect { x, y, .. } => (x as usize + u, y as usize + v),
            BlockPos::Split { x, y, .. } => {
                if u < 4 {
                    (x as usize + u, y as usize + v)
                } else {
                    (x as usize + u - 4, y as usize + 8 + v)
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct Macroblock {
    blocks: [BlockPos; MB_BLOCKS],
    /// 輝度での左上と大きさ (スーパーブロックへの割り当てに使う)
    x: usize,
    y: usize,
    w: usize,
}

/// マクロブロックの並びとセグメントの構成
#[derive(Debug)]
struct Layout {
    mbs: Vec<Macroblock>,
    /// セグメントごとの 5 マクロブロック。トラック順 (トラック t のセグメントは t*27 .. t*27+27)
    segments: Vec<[usize; SEG_MBS]>,
    /// 1 フレームのトラック数 (= スーパーブロックの段数。NTSC 10、PAL 12)
    tracks: usize,
}

impl Layout {
    fn new(std: &Standard) -> Self {
        let h = std.frame_lines();
        assert!(h % SB_ROW_LINES == 0, "DV: frame lines must be a multiple of 48");
        let r = |plane: u8, x: usize, y: usize| BlockPos::Rect { plane, x: x as u16, y: y as u16 };
        let mut mbs = Vec::new();
        if std.is_pal() {
            // 4:2:0: 16×16 の輝度 + 8×8 の色差
            for y in (0..h).step_by(16) {
                for x in (0..DV_WIDTH).step_by(16) {
                    let blocks = [r(0, x, y), r(0, x + 8, y), r(0, x, y + 8), r(0, x + 8, y + 8), r(1, x / 2, y / 2), r(2, x / 2, y / 2)];
                    mbs.push(Macroblock { blocks, x, y, w: 16 });
                }
            }
        } else {
            // 4:1:1: 32×8 の輝度 + 8×8 の色差。右端の 16 画素は 16×16 のマクロブロック
            for y in (0..h).step_by(8) {
                for x in (0..DV_WIDTH - 16).step_by(32) {
                    let blocks = [r(0, x, y), r(0, x + 8, y), r(0, x + 16, y), r(0, x + 24, y), r(1, x / 4, y), r(2, x / 4, y)];
                    mbs.push(Macroblock { blocks, x, y, w: 32 });
                }
            }
            let x = DV_WIDTH - 16;
            for y in (0..h).step_by(16) {
                let s = |plane: u8| BlockPos::Split { plane, x: (x / 4) as u16, y: y as u16 };
                let blocks = [r(0, x, y), r(0, x + 8, y), r(0, x, y + 8), r(0, x + 8, y + 8), s(1), s(2)];
                mbs.push(Macroblock { blocks, x, y, w: 16 });
            }
        }

        // スーパーブロック: 48 ライン × 画面の 1/5 幅ほどの 27 マクロブロック
        let tracks = h / SB_ROW_LINES;
        let mut rows: Vec<Vec<usize>> = vec![Vec::new(); tracks];
        for (i, mb) in mbs.iter().enumerate() {
            rows[mb.y / SB_ROW_LINES].push(i);
        }
        let mut sb = vec![[[0usize; SB_MBS]; SB_COLS]; tracks];
        for (row, list) in rows.iter_mut().enumerate() {
            assert_eq!(list.len(), SB_MBS * SB_COLS);
            list.sort_by_key(|&i| (2 * mbs[i].x + mbs[i].w, mbs[i].y));
            for (col, chunk) in list.chunks(SB_MBS).enumerate() {
                sb[row][col].copy_from_slice(chunk);
            }
        }
        // シャッフリング: セグメントは 5 列のスーパーブロックから、段をずらして 1 つずつ集める
        let mut segments = Vec::with_capacity(tracks * SB_MBS);
        for t in 0..tracks {
            let rows = [(t + 2) % tracks, (t + 6) % tracks, (t + 8) % tracks, t, (t + 4) % tracks];
            for k in 0..SB_MBS {
                segments.push(std::array::from_fn(|c| sb[rows[c]][c][k]));
            }
        }
        Self { mbs, segments, tracks }
    }
}

/// 変換・重み付けした 1 ブロック
struct Block {
    coef: [f32; 64],
    m248: bool,
    class: usize,
    luma: bool,
}

impl Block {
    #[inline(always)]
    fn step(&self, qno: usize, zz: usize) -> f32 {
        let shift = QUANT_SHIFTS[qno + CLASS_OFFSET[self.class]][area_of(zz)] + (self.class == 3) as u8;
        (2u32 << shift) as f32
    }

    /// AC 係数の発生符号量 [bit] (EOB 込み)
    fn ac_bits(&self, qno: usize) -> u32 {
        let mut bits = EOB_BITS;
        let mut run = 0;
        for zz in 1..64 {
            let l = (self.coef[ZIGZAG[zz]] / self.step(qno, zz)).round().abs().min(255.0) as u32;
            if l == 0 {
                run += 1;
            } else {
                bits += vlc_len(run, l);
                run = 0;
            }
        }
        bits
    }
}

/// 量子化したブロックと、どこまでの係数が届いたか
struct Coded {
    /// ジグザグ順の量子化値 (0 番目は DC)
    levels: [f32; 64],
    /// 符号 (係数 1 つ、または EOB) ごとの (その符号の後のジグザグ位置, 長さ)
    codes: Vec<(u8, u8)>,
    /// 1 回目 (自分の領域)・2 回目 (マクロブロック内)・3 回目 (セグメント内) までに届いたジグザグ位置 (この手前まで)
    limit: [usize; 3],
}

fn quantize(b: &Block, qno: usize) -> Coded {
    let mut levels = [0f32; 64];
    // DC は 9 bit (−256..255)
    levels[0] = (b.coef[0] / 4.0).round().clamp(-256.0, 255.0);
    let mut codes = Vec::with_capacity(16);
    let mut run = 0;
    for zz in 1..64 {
        let l = (b.coef[ZIGZAG[zz]] / b.step(qno, zz)).round().clamp(-255.0, 255.0);
        levels[zz] = l;
        if l == 0.0 {
            run += 1;
        } else {
            codes.push((zz as u8 + 1, vlc_len(run, l.abs() as u32) as u8));
            run = 0;
        }
    }
    codes.push((64, EOB_BITS as u8));
    Coded { levels, codes, limit: [1; 3] }
}

/// セグメント内の符号の詰め込み (3 段階)。届かなかった係数は捨てられる
fn allocate(coded: &mut [Coded], blocks: &[Block], scale: f32) {
    let pos = |c: &Coded, j: usize| if j == 0 { 1 } else { c.codes[j - 1].0 as usize };
    let mut taken = [0usize; SEG_BLOCKS];
    let mut left = [0f32; SEG_BLOCKS];
    for (i, (c, b)) in coded.iter_mut().zip(blocks).enumerate() {
        let mut room = if b.luma { LUMA_AREA } else { CHROMA_AREA } * scale;
        let mut j = 0;
        while j < c.codes.len() && c.codes[j].1 as f32 <= room {
            room -= c.codes[j].1 as f32;
            j += 1;
        }
        taken[i] = j;
        left[i] = room;
        c.limit[0] = pos(c, j);
    }
    let mut seg_pool = 0.0;
    for m in 0..SEG_MBS {
        let range = m * MB_BLOCKS..(m + 1) * MB_BLOCKS;
        let mut pool: f32 = left[range.clone()].iter().sum();
        for i in range {
            let c = &mut coded[i];
            let mut j = taken[i];
            while j < c.codes.len() && c.codes[j].1 as f32 <= pool {
                pool -= c.codes[j].1 as f32;
                j += 1;
            }
            taken[i] = j;
            c.limit[1] = pos(c, j);
        }
        seg_pool += pool;
    }
    for (i, c) in coded.iter_mut().enumerate() {
        let mut j = taken[i];
        while j < c.codes.len() && c.codes[j].1 as f32 <= seg_pool {
            seg_pool -= c.codes[j].1 as f32;
            j += 1;
        }
        c.limit[2] = pos(c, j);
    }
}

/// 量子化値から画素 (0..255) を復元する。`limit` から先の係数は 0
fn reconstruct(b: &Block, c: &Coded, qno: usize, limit: usize) -> [f32; 64] {
    reconstruct_levels(b.m248, b.class, &c.levels, qno, limit)
}

fn reconstruct_levels(m248: bool, class: usize, levels: &[f32; 64], qno: usize, limit: usize) -> [f32; 64] {
    let w = &weights()[m248 as usize];
    let mut coef = [0f32; 64];
    coef[0] = levels[0] * 4.0;
    for zz in 1..limit.min(64) {
        let l = levels[zz];
        if l != 0.0 {
            let i = ZIGZAG[zz];
            let shift = QUANT_SHIFTS[qno + CLASS_OFFSET[class]][area_of(zz)] + (class == 3) as u8;
            coef[i] = l * (2u32 << shift) as f32 / w[i];
        }
    }
    let mut px = if m248 { idct248(&coef) } else { idct(&coef) };
    for v in px.iter_mut() {
        *v = (*v + 128.0).round().clamp(0.0, 255.0);
    }
    px
}

/// 誤りを修整しないときの壊れたブロック (でたらめな DC と低域の係数)
fn garbage(rng: &mut Rng) -> [f32; 64] {
    let mut coef = [0f32; 64];
    coef[0] = (rng.uniform() - 0.5) * 1600.0;
    for &i in &ZIGZAG[1..10] {
        coef[i] = rng.normal() * 120.0;
    }
    let mut px = idct(&coef);
    for v in px.iter_mut() {
        *v = (*v + 128.0).clamp(0.0, 255.0);
    }
    px
}

/// 誤りを見逃したまま復号したブロック。誤りの位置までは正しく、そこから先は可変長符号の
/// 同期が外れてでたらめな (run, 振幅) が続き、偽の EOB で終わる。ヘッダ (DC・モード・クラス) に
/// 当たればブロック全体の明るさ・色や量子化の粗さが変わる
fn corrupt_block(b: &Block, c: &Coded, qno: usize, limit: usize, density: f32, rng: &mut Rng) -> [f32; 64] {
    let area_bits: u32 = if b.luma { 112 } else { 80 };
    let p_hit = 1.0 - (1.0 - density.clamp(0.0, 1.0)).powi(area_bits as i32 / 8);
    if rng.uniform() >= p_hit {
        return reconstruct(b, c, qno, limit);
    }
    let off = (rng.uniform() * area_bits as f32) as u32;
    let mut levels = c.levels;
    let (mut m248, mut class) = (b.m248, b.class);
    let start = if off < 12 {
        levels[0] = (rng.uniform() * 512.0 - 256.0).round();
        if rng.uniform() < 0.25 {
            m248 = !m248;
        }
        class = (rng.next_u64() & 3) as usize;
        1
    } else {
        // 自分の領域の中で誤りの位置にかかる符号から先が化ける
        let mut bits = 12;
        let mut start = limit;
        for (j, &(_, len)) in c.codes.iter().enumerate() {
            if bits + len as u32 > off {
                start = if j == 0 { 1 } else { c.codes[j - 1].0 as usize };
                break;
            }
            bits += len as u32;
        }
        start.min(limit)
    };
    for l in levels[start..].iter_mut() {
        *l = 0.0;
    }
    let mut pos = start;
    loop {
        pos += rng.exponential(1.5) as usize;
        if pos >= 64 {
            break;
        }
        let amp = (1.0 + rng.exponential(3.0)).floor().min(255.0);
        levels[pos] = if rng.next_u64() & 1 == 0 { amp } else { -amp };
        pos += 1;
        if rng.uniform() < 0.12 {
            break;
        }
    }
    reconstruct_levels(m248, class, &levels, qno, 64)
}

/// 平均 `mean` のポアソン乱数 (Knuth の方法。平均は小さい前提)
fn poisson(rng: &mut Rng, mean: f32) -> u32 {
    if mean <= 0.0 {
        return 0;
    }
    let limit = (-mean.min(30.0)).exp();
    let mut k = 0;
    let mut prod = rng.uniform();
    while prod > limit {
        k += 1;
        prod *= rng.uniform();
    }
    k
}

/// 直前のフレームの符号化の状態
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DvStats {
    /// セグメントの QNO の平均 (15 = 最も細かい)
    pub qno_mean: f32,
    /// 高域の係数が入りきらずに捨てられたブロックの数
    pub truncated_blocks: u32,
    /// 読めなかった (修整した) マクロブロックの数
    pub lost_mbs: u32,
    /// 誤りを見逃して化けたまま復号したマクロブロックの数
    pub corrupt_mbs: u32,
}

struct SegOut {
    qno: usize,
    truncated: u32,
    /// ブロックごとの復元画素。None は修整 (前のフレームで埋める) するブロック
    px: Vec<Option<[f32; 64]>>,
}

/// フレーム周辺の標本化の変換表: 出力画素ごとの (最初の入力画素, 重み列)
fn resample_map(dst_n: usize, src_n: usize, ratio: f32) -> Vec<(usize, Vec<f32>)> {
    let support = ratio.max(1.0);
    (0..dst_n)
        .map(|i| {
            let center = (i as f32 + 0.5 - dst_n as f32 / 2.0) * ratio + src_n as f32 / 2.0 - 0.5;
            let lo = (center - support).floor() as isize + 1;
            let hi = (center + support).ceil() as isize;
            let mut sum = 0.0;
            let mut ws = Vec::new();
            let mut start = None;
            for j in lo..hi {
                let w = (1.0 - ((j as f32 - center) / support).abs()).max(0.0);
                sum += w;
                // 範囲外は黒 (重みの和には入れて、端を黒へなじませる)
                if j >= 0 && (j as usize) < src_n {
                    start.get_or_insert(j as usize);
                    ws.push(w);
                }
            }
            if sum > 0.0 {
                ws.iter_mut().for_each(|w| *w /= sum);
            }
            (start.unwrap_or(0), ws)
        })
        .collect()
}

pub struct DvSim {
    std: &'static Standard,
    layout: Layout,
    to_dv: Vec<(usize, Vec<f32>)>,
    from_dv: Vec<(usize, Vec<f32>)>,
    /// 2 フィールドを織り込んだ入力 (エンコーダ側のフレームメモリ)
    weave: VideoFrame,
    has_weave: bool,
    /// 符号化前の Y, Cb, Cr
    planes: [Plane; 3],
    /// 復元した Y, Cb, Cr
    recon: [Plane; 3],
    /// 前のフレームの復元画像 (エラー修整用)
    prev: Option<[Plane; 3]>,
    out: VideoFrame,
    frame: Option<u64>,
    /// フレームごとの、読めなかったマクロブロック
    state: Vec<MbState>,
    clog_left: u32,
    clog_heads: u8,
    pub stats: DvStats,
}

impl DvSim {
    pub fn new(std: &'static Standard) -> Self {
        let (aw, h) = std.image_size();
        let layout = Layout::new(std);
        let (cw, ch) = if std.is_pal() { (DV_WIDTH / 2, h / 2) } else { (DV_WIDTH / 4, h) };
        let planes = [Plane::new(DV_WIDTH, h), Plane::new(cw, ch), Plane::new(cw, ch)];
        Self {
            std,
            to_dv: resample_map(DV_WIDTH, aw, std.fs / DV_FS),
            from_dv: resample_map(aw, DV_WIDTH, DV_FS / std.fs),
            weave: VideoFrame::new(aw, h),
            has_weave: false,
            recon: planes.clone(),
            planes,
            prev: None,
            out: VideoFrame::new(aw, h),
            frame: None,
            state: vec![MbState::Ok; layout.mbs.len()],
            layout,
            clog_left: 0,
            clog_heads: 0,
            stats: DvStats::default(),
        }
    }

    /// 1 フィールドぶん記録・再生する。返す画像はフレーム全体 (今のフィールドのラインが有効)
    pub fn process(&mut self, input: &VideoFrame, parity: u8, frame: u64, p: &DvParams, seed: u64) -> &VideoFrame {
        let (aw, h) = self.std.image_size();
        assert_eq!((input.width, input.height), (aw, h));
        // 今のフィールドのラインを織り込む
        if self.has_weave {
            for row in (parity as usize..h).step_by(2) {
                let r = row * aw * 3..(row + 1) * aw * 3;
                self.weave.data[r.clone()].copy_from_slice(&input.data[r]);
            }
        } else {
            self.weave.data.copy_from_slice(&input.data);
            self.has_weave = true;
        }
        if self.frame != Some(frame) {
            if self.frame.is_some() {
                self.prev = Some(self.recon.clone());
            }
            self.frame = Some(frame);
            self.advance_errors(frame, p, seed);
        }

        self.analyze();
        self.code(p, seed, frame);
        self.synthesize();
        &self.out
    }

    /// フレームごとのテープのエラー (ヘッドの目詰まり・ランダムな誤り・バースト)
    fn advance_errors(&mut self, frame: u64, p: &DvParams, seed: u64) {
        let mut rng = Rng::derive(seed, frame, 0, STREAM_DV);
        let frame_rate = self.std.frame_rate();
        if self.clog_left > 0 {
            self.clog_left -= 1;
        } else {
            self.clog_heads = 0;
            if rng.uniform() < p.head_clog_rate.max(0.0) / 60.0 / frame_rate {
                self.clog_left = (rng.exponential(p.head_clog_frames.max(1.0)).ceil() as u32).max(1) - 1;
                // 片方のヘッドだけ詰まることが多い
                self.clog_heads = if rng.uniform() < 0.3 { 3 } else { 1 << (rng.next_u64() & 1) };
            }
        }

        let state = &mut self.state;
        state.fill(MbState::Ok);
        let segs = &self.layout.segments;
        for (si, seg) in segs.iter().enumerate() {
            let head = (si / SB_MBS) & 1;
            if self.clog_heads & (1 << head) != 0 {
                seg.iter().for_each(|&m| state[m] = MbState::Lost);
            }
        }
        let er = p.error_rate.clamp(0.0, 1.0);
        if er > 0.0 {
            for l in state.iter_mut() {
                if rng.uniform() < er {
                    *l = MbState::Lost;
                }
            }
        }
        let burst_p = p.burst_rate.max(0.0) / frame_rate / self.layout.tracks as f32;
        for t in 0..self.layout.tracks {
            if rng.uniform() < burst_p {
                let start = (rng.uniform() * SB_MBS as f32) as usize;
                let len = rng.exponential(p.burst_length.max(1.0)).ceil() as usize;
                for k in start..(start + len.max(1)).min(SB_MBS) {
                    segs[t * SB_MBS + k].iter().for_each(|&m| state[m] = MbState::Lost);
                }
            }
        }

        // バーストノイズ: 読み取り順 (トラック → セグメント → マクロブロック) に連続した区間のデータが化ける。
        // 区間はトラックの境目をまたいで次のトラックへ続くことがある
        let order_len = segs.len() * SEG_MBS;
        let density = p.noise_density.clamp(0.0, 1.0);
        let bursts = poisson(&mut rng, p.noise_rate.max(0.0) / frame_rate);
        for _ in 0..bursts {
            let start = (rng.uniform() * order_len as f32) as usize;
            let len = (rng.exponential(p.noise_length.max(1.0)).ceil() as usize).max(1);
            for idx in start..(start + len).min(order_len) {
                let mb = segs[idx / SEG_MBS][idx % SEG_MBS];
                let errors = (0..MB_BYTES).filter(|_| rng.uniform() < density).count() as u32;
                if errors == 0 || state[mb] == MbState::Lost {
                    continue;
                }
                if p.ecc {
                    if errors <= ECC_CORRECTABLE {
                        continue;
                    }
                    if rng.uniform() >= p.ecc_miss.clamp(0.0, 1.0) {
                        state[mb] = MbState::Lost;
                        continue;
                    }
                }
                state[mb] = MbState::Corrupt(density);
            }
        }
    }

    /// 織り込んだ RGB → 13.5 MHz 標本化の YCbCr、色差の間引き
    fn analyze(&mut self) {
        let (aw, h) = self.std.image_size();
        let pal = self.std.is_pal();
        let weave = &self.weave;
        let to_dv = &self.to_dv;
        let [py, pcb, pcr] = &mut self.planes;
        // 色差は一旦 720 幅で作る (Cb, Cr を 1 行に並べる)
        let mut full_c = vec![0f32; DV_WIDTH * 2 * h];
        py.data.par_chunks_mut(DV_WIDTH).zip(full_c.par_chunks_mut(DV_WIDTH * 2)).enumerate().for_each(|(y, (oy, oc))| {
            let src = &weave.data[y * aw * 3..(y + 1) * aw * 3];
            for (x, (start, ws)) in to_dv.iter().enumerate() {
                let mut rgb = [0f32; 3];
                for (j, w) in ws.iter().enumerate() {
                    let px = &src[(start + j) * 3..][..3];
                    for c in 0..3 {
                        rgb[c] += px[c] * w;
                    }
                }
                let [r, g, b] = rgb;
                let yy = 0.299 * r + 0.587 * g + 0.114 * b;
                oy[x] = (16.0 + 219.0 * yy).round().clamp(1.0, 254.0);
                oc[x] = (128.0 + 224.0 * (b - yy) / 1.772).round().clamp(1.0, 254.0);
                oc[DV_WIDTH + x] = (128.0 + 224.0 * (r - yy) / 1.402).round().clamp(1.0, 254.0);
            }
        });
        let cw = pcb.w;
        for (ci, plane) in [pcb, pcr].into_iter().enumerate() {
            let off = ci * DV_WIDTH;
            plane.data.par_chunks_mut(cw).enumerate().for_each(|(j, row)| {
                if pal {
                    // 4:2:0 をフィールドごとに: 色差の行 j は同じフィールドの上下 2 ライン
                    let (m, par) = (j >> 1, j & 1);
                    let (r0, r1) = (4 * m + par, (4 * m + 2 + par).min(h - 1));
                    for (x, o) in row.iter_mut().enumerate() {
                        let s = |r: usize| full_c[r * DV_WIDTH * 2 + off + 2 * x] + full_c[r * DV_WIDTH * 2 + off + 2 * x + 1];
                        *o = ((s(r0) + s(r1)) * 0.25).round();
                    }
                } else {
                    for (x, o) in row.iter_mut().enumerate() {
                        let r = &full_c[j * DV_WIDTH * 2 + off + 4 * x..][..4];
                        *o = ((r[0] + r[1] + r[2] + r[3]) * 0.25).round();
                    }
                }
            });
        }
    }

    fn code(&mut self, p: &DvParams, seed: u64, frame: u64) {
        let scale = p.bitrate_scale.clamp(0.05, 4.0);
        let layout = &self.layout;
        let planes = &self.planes;
        let state = &self.state;
        let conceal = p.concealment;
        let outs: Vec<SegOut> = layout
            .segments
            .par_iter()
            .enumerate()
            .map(|(si, seg)| {
                // 変換・重み付け・クラス分け
                let mut blocks = Vec::with_capacity(SEG_BLOCKS);
                for &mi in seg {
                    for (bi, pos) in layout.mbs[mi].blocks.iter().enumerate() {
                        let pl = &planes[pos.plane()];
                        let mut px = [0f32; 64];
                        for v in 0..8 {
                            for u in 0..8 {
                                let (x, y) = pos.coord(u, v);
                                px[v * 8 + u] = pl.data[y * pl.w + x] - 128.0;
                            }
                        }
                        let m248 = p.dct_248 && interlaced_motion(&px);
                        let mut coef = if m248 { fdct248(&px) } else { fdct(&px) };
                        let w = &weights()[m248 as usize];
                        let mut max_ac = 0f32;
                        for i in 0..64 {
                            coef[i] *= w[i];
                            if i > 0 {
                                max_ac = max_ac.max(coef[i].abs());
                            }
                        }
                        let class = CLASS_THRESH.iter().filter(|&&t| max_ac * 0.5 >= t).count();
                        blocks.push(Block { coef, m248, class, luma: bi < 4 });
                    }
                }
                // セグメントの符号量に収まる最も細かい QNO
                let cap = SEG_MBS as f32 * (4.0 * LUMA_AREA + 2.0 * CHROMA_AREA) * scale;
                let fits = |q: usize| blocks.iter().map(|b| b.ac_bits(q)).sum::<u32>() as f32 <= cap;
                let qno = if fits(QNO_MAX) {
                    QNO_MAX
                } else if !fits(0) {
                    0
                } else {
                    let (mut lo, mut hi) = (0, QNO_MAX);
                    while hi - lo > 1 {
                        let mid = (lo + hi) / 2;
                        if fits(mid) { lo = mid } else { hi = mid }
                    }
                    lo
                };
                let mut coded: Vec<Coded> = blocks.iter().map(|b| quantize(b, qno)).collect();
                allocate(&mut coded, &blocks, scale);

                // 同じセグメントのどれかが読めない・化けていれば、セグメント内に溢れた分 (3 回目) も失われる
                let damaged = seg.iter().any(|&m| state[m] != MbState::Ok);
                let mut grng = Rng::derive(seed, frame, si as u64, STREAM_DV_GARBAGE);
                let mut truncated = 0;
                let px = blocks
                    .iter()
                    .zip(&coded)
                    .enumerate()
                    .map(|(k, (b, c))| {
                        if c.limit[2] < 64 {
                            truncated += 1;
                        }
                        let limit = if damaged { c.limit[1] } else { c.limit[2] };
                        match state[seg[k / MB_BLOCKS]] {
                            MbState::Lost if conceal => None,
                            MbState::Lost => Some(garbage(&mut grng)),
                            MbState::Corrupt(d) => Some(corrupt_block(b, c, qno, limit, d, &mut grng)),
                            MbState::Ok => Some(reconstruct(b, c, qno, limit)),
                        }
                    })
                    .collect();
                SegOut { qno, truncated, px }
            })
            .collect();

        // 復元画素を平面に書き戻す。修整するブロックは前のフレームから写す (なければ黒)
        let mut qsum = 0;
        let mut truncated = 0;
        for (seg, out) in layout.segments.iter().zip(&outs) {
            qsum += out.qno;
            truncated += out.truncated;
            for (k, px) in out.px.iter().enumerate() {
                let pos = layout.mbs[seg[k / MB_BLOCKS]].blocks[k % MB_BLOCKS];
                let pi = pos.plane();
                let dst = &mut self.recon[pi];
                for v in 0..8 {
                    for u in 0..8 {
                        let (x, y) = pos.coord(u, v);
                        let i = y * dst.w + x;
                        dst.data[i] = match px {
                            Some(px) => px[v * 8 + u],
                            None => match &self.prev {
                                Some(prev) => prev[pi].data[i],
                                None if pi == 0 => 16.0,
                                None => 128.0,
                            },
                        };
                    }
                }
            }
        }
        self.stats = DvStats {
            qno_mean: qsum as f32 / outs.len() as f32,
            truncated_blocks: truncated,
            lost_mbs: state.iter().filter(|&&s| s == MbState::Lost).count() as u32,
            corrupt_mbs: state.iter().filter(|s| matches!(s, MbState::Corrupt(_))).count() as u32,
        };
    }

    /// 復元した YCbCr → RGB、元の標本化 (4fsc) へ戻す
    fn synthesize(&mut self) {
        let (aw, _) = self.std.image_size();
        let pal = self.std.is_pal();
        let [py, pcb, pcr] = &self.recon;
        let from_dv = &self.from_dv;
        let cw = pcb.w;
        self.out.data.par_chunks_mut(aw * 3).enumerate().for_each(|(y, out)| {
            // 色差を 720 幅に補間 (水平は線形、PAL の垂直は同じフィールドの最寄りの行)
            let (cy, fx) = if pal { (((y >> 1) >> 1) * 2 + (y & 1), 2.0) } else { (y, 4.0) };
            let mut rgb = vec![0f32; DV_WIDTH * 3];
            for x in 0..DV_WIDTH {
                let cxf = ((x as f32 + 0.5) / fx - 0.5).max(0.0);
                let c0 = (cxf as usize).min(cw - 1);
                let c1 = (c0 + 1).min(cw - 1);
                let t = cxf - c0 as f32;
                let cb = pcb.data[cy * cw + c0] * (1.0 - t) + pcb.data[cy * cw + c1] * t;
                let cr = pcr.data[cy * cw + c0] * (1.0 - t) + pcr.data[cy * cw + c1] * t;
                let yy = (py.data[y * DV_WIDTH + x] - 16.0) / 219.0;
                let (cb, cr) = ((cb - 128.0) / 224.0, (cr - 128.0) / 224.0);
                rgb[x * 3] = yy + 1.402 * cr;
                rgb[x * 3 + 1] = yy - 0.344_136 * cb - 0.714_136 * cr;
                rgb[x * 3 + 2] = yy + 1.772 * cb;
            }
            for (n, (start, ws)) in from_dv.iter().enumerate() {
                let mut acc = [0f32; 3];
                for (j, w) in ws.iter().enumerate() {
                    for c in 0..3 {
                        acc[c] += rgb[(start + j) * 3 + c] * w;
                    }
                }
                out[n * 3..n * 3 + 3].copy_from_slice(&acc);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::{NTSC, PAL};

    #[test]
    fn layout_counts() {
        for (std, mbs, tracks) in [(&NTSC, 1350, 10), (&PAL, 1620, 12)] {
            let l = Layout::new(std);
            assert_eq!(l.mbs.len(), mbs);
            assert_eq!(l.tracks, tracks);
            assert_eq!(l.segments.len(), tracks * SB_MBS);
            // 全マクロブロックがちょうど 1 回ずつ使われる
            let mut used = vec![0u32; mbs];
            l.segments.iter().flatten().for_each(|&m| used[m] += 1);
            assert!(used.iter().all(|&u| u == 1));
        }
    }

    #[test]
    fn dct248_roundtrip() {
        let mut x = [0f32; 64];
        for (i, v) in x.iter_mut().enumerate() {
            *v = ((i * 53) % 211) as f32 - 100.0;
        }
        let y = idct248(&fdct248(&x));
        for i in 0..64 {
            assert!((x[i] - y[i]).abs() < 1e-3);
        }
    }

    fn textured(std: &'static Standard, amount: f32) -> VideoFrame {
        let (w, h) = std.image_size();
        let mut f = VideoFrame::new(w, h);
        let mut rng = Rng::new(7);
        for y in 0..h {
            for x in 0..w {
                let base = [x as f32 / w as f32, y as f32 / h as f32, 0.5];
                for c in 0..3 {
                    f.data[(y * w + x) * 3 + c] = (base[c] + amount * (rng.uniform() - 0.5)).clamp(0.0, 1.0);
                }
            }
        }
        f
    }

    fn run(std: &'static Standard, input: &VideoFrame, p: &DvParams) -> (DvStats, f32) {
        let mut dv = DvSim::new(std);
        for i in 0..2u64 {
            dv.process(input, (i & 1) as u8, i / 2, p, 1);
        }
        let out = &dv.out;
        // 端 (標本化し直しで黒になる数画素) を除いた平均絶対誤差
        let (w, h) = std.image_size();
        let mut err = 0.0;
        let mut n = 0;
        for y in 0..h {
            for x in 16..w - 16 {
                for c in 0..3 {
                    let i = (y * w + x) * 3 + c;
                    err += (out.data[i] - input.data[i]).abs();
                    n += 1;
                }
            }
        }
        (dv.stats, err / n as f32)
    }

    #[test]
    fn smooth_image_codes_finely() {
        for std in [&NTSC, &PAL] {
            let (st, err) = run(std, &textured(std, 0.0), &DvParams::default());
            assert!(st.qno_mean > 14.0, "{st:?}");
            assert_eq!(st.truncated_blocks, 0);
            assert!(err < 0.01, "err {err}");
        }
    }

    #[test]
    fn noisy_image_quantizes_coarsely() {
        let (clean, _) = run(&NTSC, &textured(&NTSC, 0.05), &DvParams::default());
        let (noisy, err) = run(&NTSC, &textured(&NTSC, 0.6), &DvParams::default());
        assert!(noisy.qno_mean < clean.qno_mean, "{clean:?} {noisy:?}");
        assert!(err > 0.01);
    }

    #[test]
    fn burst_noise_and_ecc() {
        let input = textured(&NTSC, 0.0);
        let stats = |p: &DvParams| {
            let mut dv = DvSim::new(&NTSC);
            let mut acc = DvStats::default();
            for i in 0..40u64 {
                dv.process(&input, (i & 1) as u8, i / 2, p, 3);
                acc.lost_mbs += dv.stats.lost_mbs;
                acc.corrupt_mbs += dv.stats.corrupt_mbs;
            }
            acc
        };
        let base = DvParams { noise_rate: 200.0, noise_length: 40.0, ..DvParams::default() };
        // 誤りが少なければ訂正しきる
        let light = stats(&DvParams { noise_density: 0.003, ..base.clone() });
        assert_eq!((light.lost_mbs, light.corrupt_mbs), (0, 0), "{light:?}");
        // 多ければ検出して修整し、一部は見逃す
        let heavy = stats(&DvParams { noise_density: 0.2, ecc_miss: 0.2, ..base.clone() });
        assert!(heavy.lost_mbs > 0 && heavy.corrupt_mbs > 0, "{heavy:?}");
        assert!(heavy.lost_mbs > heavy.corrupt_mbs, "{heavy:?}");
        // 誤り訂正なしなら全部化ける
        let raw = stats(&DvParams { noise_density: 0.01, ecc: false, ..base });
        assert!(raw.corrupt_mbs > 0 && raw.lost_mbs == 0, "{raw:?}");
    }

    #[test]
    fn head_clog_loses_alternate_tracks() {
        let p = DvParams { head_clog_rate: 1e9, ..DvParams::default() };
        let mut dv = DvSim::new(&NTSC);
        let input = textured(&NTSC, 0.0);
        dv.process(&input, 0, 0, &p, 1);
        // 片方なら半分、両方なら全部
        assert!(dv.stats.lost_mbs == 675 || dv.stats.lost_mbs == 1350, "{:?}", dv.stats);
    }
}
