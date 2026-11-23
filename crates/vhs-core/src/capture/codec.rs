//! MPEG-2 風のブロック変換符号化 (ビットストリームは作らず、画質への影響だけを再現する)。
//!
//! - 8×8 DCT、MPEG-2 既定のイントラ量子化マトリクス、インターはフラット (16) + デッドゾーン
//! - 16×16 マクロブロック単位の整数画素動き補償 (3 ステップ探索)。予測は再構成画像から行う
//!   (閉ループ) ので、誤差は次の I フレームまで引きずられる
//! - I / P の GOP 構造
//! - レート制御: 発生符号量を VLC 長の近似で見積もり、次のフレームの量子化スケールを決める。
//!   ノイズの多い VHS 素材は符号量を食うので、目標ビットレートが低いとブロックが目立ち、
//!   シーンが変わると画質が「呼吸」する

use super::Plane;
use crate::params::CaptureParams;
use rayon::prelude::*;
use std::sync::OnceLock;

/// MPEG-2 既定のイントラ量子化マトリクス (ラスタ順)
const INTRA_MATRIX: [f32; 64] = [
    8., 16., 19., 22., 26., 27., 29., 34., //
    16., 16., 22., 24., 27., 29., 34., 37., //
    19., 22., 26., 27., 29., 34., 34., 38., //
    22., 22., 26., 27., 29., 34., 37., 40., //
    22., 26., 27., 29., 32., 35., 40., 48., //
    26., 27., 29., 32., 35., 40., 48., 58., //
    26., 27., 29., 34., 38., 46., 56., 69., //
    27., 29., 35., 38., 46., 56., 69., 83., //
];

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21,
    28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61,
    54, 47, 55, 62, 63,
];

const Q_MIN: f32 = 1.0;
const Q_MAX: f32 = 112.0;

fn dct_matrix() -> &'static [[f32; 8]; 8] {
    static M: OnceLock<[[f32; 8]; 8]> = OnceLock::new();
    M.get_or_init(|| {
        let mut m = [[0f32; 8]; 8];
        for (k, row) in m.iter_mut().enumerate() {
            let c = if k == 0 { (1.0f32 / 8.0).sqrt() } else { (2.0f32 / 8.0).sqrt() };
            for (n, v) in row.iter_mut().enumerate() {
                *v = c * ((2 * n + 1) as f32 * k as f32 * std::f32::consts::PI / 16.0).cos();
            }
        }
        m
    })
}

/// 正規直交 2 次元 DCT
fn fdct(x: &[f32; 64]) -> [f32; 64] {
    let m = dct_matrix();
    let mut t = [0f32; 64];
    for r in 0..8 {
        for k in 0..8 {
            let mut s = 0.0;
            for n in 0..8 {
                s += m[k][n] * x[r * 8 + n];
            }
            t[r * 8 + k] = s;
        }
    }
    let mut y = [0f32; 64];
    for c in 0..8 {
        for k in 0..8 {
            let mut s = 0.0;
            for n in 0..8 {
                s += m[k][n] * t[n * 8 + c];
            }
            y[k * 8 + c] = s;
        }
    }
    y
}

fn idct(y: &[f32; 64]) -> [f32; 64] {
    let m = dct_matrix();
    let mut t = [0f32; 64];
    for c in 0..8 {
        for n in 0..8 {
            let mut s = 0.0;
            for k in 0..8 {
                s += m[k][n] * y[k * 8 + c];
            }
            t[n * 8 + c] = s;
        }
    }
    let mut x = [0f32; 64];
    for r in 0..8 {
        for n in 0..8 {
            let mut s = 0.0;
            for k in 0..8 {
                s += m[k][n] * t[r * 8 + k];
            }
            x[r * 8 + n] = s;
        }
    }
    x
}

/// (run, |level|) の VLC 長のおおよその値 [bit] (MPEG-2 表 B.14 の傾向に合わせた近似)
#[inline]
fn vlc_bits(run: u32, level: u32) -> u32 {
    let lv = 32 - level.leading_zeros(); // ceil(log2(level+1)) 相当
    let rn = if run == 0 { 0 } else { 1 + 2 * (32 - run.leading_zeros()) };
    (3 + 2 * (lv - 1) + rn).min(24)
}

/// 1 ブロックを量子化・逆量子化し、再構成した残差と発生符号量を返す
fn code_block(src: &[f32; 64], intra: bool, q: f32) -> ([f32; 64], u32) {
    let coef = fdct(src);
    let mut rec = [0f32; 64];
    let mut bits = 0u32;
    let mut run = 0u32;
    let start = if intra {
        // DC は 8 ビット精度 (ステップ 8)
        let l = (coef[0] / 8.0).round();
        rec[0] = l * 8.0;
        let size = 32 - (l.abs() as u32).leading_zeros();
        bits += 3 + size;
        1
    } else {
        0
    };
    let mut any = false;
    for &zi in &ZIGZAG[start..] {
        let f = coef[zi];
        let level = if intra {
            let w = INTRA_MATRIX[zi] * q;
            let l = (16.0 * f / w).round();
            rec[zi] = l * w / 16.0;
            l
        } else {
            // フラットマトリクス + デッドゾーン付き切り捨て
            let l = (f / q).trunc();
            rec[zi] = if l != 0.0 { (2.0 * l + l.signum()) * q / 2.0 } else { 0.0 };
            l
        };
        if level != 0.0 {
            bits += vlc_bits(run, level.abs() as u32);
            run = 0;
            any = true;
        } else {
            run += 1;
        }
    }
    if any || intra {
        bits += 2; // EOB
    }
    if !any && !intra {
        // 係数なしブロックは CBP で飛ばされる
        return (rec, 0);
    }
    (idct(&rec), bits)
}

#[derive(Clone, Copy, Debug, Default)]
struct MbInfo {
    mv: (i32, i32),
    intra: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CodecStats {
    /// 直前のフレームの発生符号量 [bit]
    pub bits: u64,
    /// 直前のフレームの量子化スケール
    pub q: f32,
    pub intra_frame: bool,
}

pub struct Codec {
    reference: Option<[Plane; 3]>,
    gop_pos: u32,
    q_i: f32,
    q_p: f32,
    mbs: Vec<MbInfo>,
    recon: [Plane; 3],
    pub stats: CodecStats,
}

impl Default for Codec {
    fn default() -> Self {
        Self {
            reference: None,
            gop_pos: 0,
            q_i: 6.0,
            q_p: 8.0,
            mbs: Vec::new(),
            recon: Default::default(),
            stats: CodecStats::default(),
        }
    }
}

fn sad16(cur: &Plane, refp: &Plane, x0: usize, y0: usize, dx: i32, dy: i32, limit: u32) -> u32 {
    let mut s = 0f32;
    for y in 0..16 {
        let cy = y0 + y;
        let ry = cy as isize + dy as isize;
        for x in 0..16 {
            let cx = x0 + x;
            let r = refp.at_clamped(cx as isize + dx as isize, ry);
            s += (cur.data[cy * cur.w + cx] - r).abs();
        }
        if s as u32 > limit {
            return u32::MAX;
        }
    }
    s as u32
}

impl Codec {
    pub fn reset(&mut self) {
        self.reference = None;
        self.gop_pos = 0;
    }

    /// `frame_rate`: 符号量を割り当てるフレーム周波数 [Hz]
    /// (1 出力フレーム = 2 フィールドを織り込んだ 1 フレーム。NTSC なら 29.97)
    pub fn encode(&mut self, planes: &mut [Plane; 3], p: &CaptureParams, frame_rate: f32) {
        let dims_ok = self.reference.as_ref().is_some_and(|r| (0..3).all(|i| r[i].w == planes[i].w && r[i].h == planes[i].h));
        let gop = p.gop.max(1);
        if !dims_ok || self.gop_pos >= gop {
            self.gop_pos = 0;
        }
        let intra_frame = self.gop_pos == 0 || !dims_ok;
        let q = if intra_frame { self.q_i } else { self.q_p };
        let search = p.motion_search.min(32) as i32;

        for i in 0..3 {
            self.recon[i].resize(planes[i].w, planes[i].h);
        }
        let (yw, yh) = (planes[0].w, planes[0].h);
        let (mbw, mbh) = (yw / 16, yh / 16);
        self.mbs.resize(mbw * mbh, MbInfo::default());
        let reference = if intra_frame { None } else { self.reference.as_ref() };

        // ---- 輝度: マクロブロック単位 (動き探索・イントラ判定込み) ----
        let cur_y = &planes[0];
        let luma_bits: u64 = self.recon[0]
            .data
            .par_chunks_mut(16 * yw)
            .zip(self.mbs.par_chunks_mut(mbw))
            .enumerate()
            .map(|(my, (rows, infos))| {
                let mut bits = 0u64;
                for (mx, info) in infos.iter_mut().enumerate() {
                    let (x0, y0) = (mx * 16, my * 16);
                    let mut mb = MbInfo { mv: (0, 0), intra: true };
                    if let Some(rf) = reference {
                        let rfy = &rf[0];
                        // 3 ステップ探索
                        let mut best = sad16(cur_y, rfy, x0, y0, 0, 0, u32::MAX);
                        let zero_sad = best;
                        let mut c = (0i32, 0i32);
                        let mut step = ((search + 1) / 2).max(1);
                        while search > 0 && step >= 1 {
                            let center = c;
                            for dy in [-step, 0, step] {
                                for dx in [-step, 0, step] {
                                    let v = (center.0 + dx, center.1 + dy);
                                    if (dx == 0 && dy == 0) || v.0.abs() > search || v.1.abs() > search {
                                        continue;
                                    }
                                    let s = sad16(cur_y, rfy, x0, y0, v.0, v.1, best);
                                    if s < best {
                                        best = s;
                                        c = v;
                                    }
                                }
                            }
                            step /= 2;
                        }
                        // 動きベクトルにも符号量がかかるので、僅差ならゼロベクトルを選ぶ
                        if zero_sad <= best + 100 {
                            c = (0, 0);
                            best = zero_sad;
                        }
                        let mut mean = 0.0;
                        for y in 0..16 {
                            for x in 0..16 {
                                mean += cur_y.data[(y0 + y) * yw + x0 + x];
                            }
                        }
                        mean /= 256.0;
                        let mut dev = 0.0;
                        for y in 0..16 {
                            for x in 0..16 {
                                dev += (cur_y.data[(y0 + y) * yw + x0 + x] - mean).abs();
                            }
                        }
                        mb = MbInfo { mv: c, intra: dev + 500.0 < best as f32 };
                        bits += if mb.intra { 4 } else { 1 + mv_bits(c) };
                    } else {
                        bits += 2;
                    }
                    *info = mb;

                    for b in 0..4 {
                        let (bx, by) = (x0 + (b & 1) * 8, (b >> 1) * 8);
                        let mut src = [0f32; 64];
                        let mut pred = [0f32; 64];
                        for y in 0..8 {
                            for x in 0..8 {
                                let cv = cur_y.data[(y0 + by + y) * yw + bx + x];
                                let pv = match reference {
                                    Some(rf) if !mb.intra => rf[0].at_clamped(
                                        (bx + x) as isize + mb.mv.0 as isize,
                                        (y0 + by + y) as isize + mb.mv.1 as isize,
                                    ),
                                    _ => 128.0,
                                };
                                pred[y * 8 + x] = pv;
                                src[y * 8 + x] = cv - pv;
                            }
                        }
                        let (res, b_bits) = code_block(&src, mb.intra, q);
                        bits += b_bits as u64;
                        for y in 0..8 {
                            for x in 0..8 {
                                rows[(by + y) * yw + bx + x] = (pred[y * 8 + x] + res[y * 8 + x]).clamp(0.0, 255.0);
                            }
                        }
                    }
                }
                bits
            })
            .sum();

        // ---- 色差: 8×8 ブロックごとに、対応するマクロブロックの判定と動きベクトルを流用 ----
        let mut chroma_bits = 0u64;
        for ci in 1..3 {
            let cur = &planes[ci];
            let (cw, ch) = (cur.w, cur.h);
            let (sx, sy) = (yw as f32 / cw as f32, yh as f32 / ch as f32);
            let mbs = &self.mbs;
            let rf = reference.map(|r| &r[ci]);
            chroma_bits += self.recon[ci]
                .data
                .par_chunks_mut(8 * cw)
                .enumerate()
                .map(|(byi, rows)| {
                    let mut bits = 0u64;
                    for bxi in 0..cw / 8 {
                        let (bx, by) = (bxi * 8, byi * 8);
                        let lmx = (((bx as f32 + 4.0) * sx) as usize / 16).min(mbw - 1);
                        let lmy = (((by as f32 + 4.0) * sy) as usize / 16).min(mbh - 1);
                        let mb = mbs[lmy * mbw + lmx];
                        let mv = ((mb.mv.0 as f32 / sx).round() as isize, (mb.mv.1 as f32 / sy).round() as isize);
                        let mut src = [0f32; 64];
                        let mut pred = [0f32; 64];
                        for y in 0..8 {
                            for x in 0..8 {
                                let cv = cur.data[(by + y) * cw + bx + x];
                                let pv = match rf {
                                    Some(r) if !mb.intra => {
                                        r.at_clamped((bx + x) as isize + mv.0, (by + y) as isize + mv.1)
                                    }
                                    _ => 128.0,
                                };
                                pred[y * 8 + x] = pv;
                                src[y * 8 + x] = cv - pv;
                            }
                        }
                        let (res, b_bits) = code_block(&src, mb.intra, q);
                        bits += b_bits as u64;
                        for y in 0..8 {
                            for x in 0..8 {
                                rows[y * cw + bx + x] = (pred[y * 8 + x] + res[y * 8 + x]).clamp(0.0, 255.0);
                            }
                        }
                    }
                    bits
                })
                .sum::<u64>();
        }

        // VLC 長の近似は実際より長めに出るので補正する。300 bit はヘッダ類
        let bits = ((luma_bits + chroma_bits) as f32 * 0.7) as u64 + 300;
        self.rate_control(bits, intra_frame, p, gop, frame_rate);
        self.stats = CodecStats { bits, q, intra_frame };

        // 再構成画像を出力し、次のフレームの参照にする
        for i in 0..3 {
            std::mem::swap(&mut planes[i].data, &mut self.recon[i].data);
        }
        match self.reference.as_mut() {
            Some(r) if dims_ok => {
                for i in 0..3 {
                    r[i].data.copy_from_slice(&planes[i].data);
                }
            }
            _ => self.reference = Some(planes.clone()),
        }
        self.gop_pos += 1;
    }

    fn rate_control(&mut self, bits: u64, intra: bool, p: &CaptureParams, gop: u32, frame_rate: f32) {
        let target = p.bitrate_kbps.max(50.0) * 1000.0 / frame_rate;
        // GOP 内で I に 3 倍の符号量を割り当てる
        let g = gop as f32;
        let tp = if gop > 1 { g * target / (g + 2.0) } else { target };
        let ti = if gop > 1 { 3.0 * tp } else { target };
        // 1 フレーム遅れの帰還なので、ゲインが高すぎると 1 フレームおきに画質が振動する
        let k = 0.05 + 0.45 * p.rate_control.clamp(0.0, 1.0);
        let ratio = (bits as f32 / if intra { ti } else { tp }).max(1e-3);
        let f = ratio.powf(k).clamp(0.5, 2.0);
        if intra {
            self.q_i = (self.q_i * f).clamp(Q_MIN, Q_MAX);
            // P の量子化も I に引きずられる
            self.q_p = (self.q_p * f.sqrt()).clamp(Q_MIN, Q_MAX);
        } else {
            self.q_p = (self.q_p * f).clamp(Q_MIN, Q_MAX);
        }
    }
}

fn mv_bits(mv: (i32, i32)) -> u64 {
    let c = |v: i32| if v == 0 { 1 } else { 2 + 2 * (32 - v.unsigned_abs().leading_zeros()) as u64 };
    c(mv.0) + c(mv.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dct_roundtrip() {
        let mut x = [0f32; 64];
        for (i, v) in x.iter_mut().enumerate() {
            *v = ((i * 37) % 255) as f32 - 128.0;
        }
        let y = idct(&fdct(&x));
        for i in 0..64 {
            assert!((x[i] - y[i]).abs() < 1e-3);
        }
        // DC = 8 × 平均
        let flat = [10f32; 64];
        assert!((fdct(&flat)[0] - 80.0).abs() < 1e-3);
    }
}
