//! デジタルキャプチャのシミュレーション。
//!
//! アナログ映像 (受像機の出力) を家庭用キャプチャ機器で取り込み、MPEG 系の形式で保存した
//! ときの見た目を作る。処理は実際の機器の順序に沿う:
//!
//! 1. ADC: RGB → YCbCr (BT.601)。レベル (16-235 / 0-255) の取り違え、量子化ビット数
//! 2. 色差間引き: 4:2:2 / 4:2:0 / 4:1:1。4:2:0 はインターレース素材の上下フィールドの色を
//!    混ぜてしまうので、動きのある所で色がにじむ (クロマアップサンプリングエラー)
//! 3. 圧縮: 8×8 DCT・量子化・動き補償予測・GOP・レート制御 (`codec`)
//! 4. 再生側: 色差の補間 (水平は線形、垂直は最近傍) と RGB への変換
//!
//! コマ落ち (取り込みが間に合わず前のフレームを繰り返す) もここで扱う。

pub(crate) mod codec;

use crate::frame::RgbaFrame;
use crate::params::{CaptureLevels, CaptureParams, ChromaSubsampling};
use crate::rng::Rng;
use codec::Codec;
use rayon::prelude::*;

pub use codec::CodecStats;

const STREAM_CAPTURE: u64 = 21;

/// 1 枚の画像平面 (0..255 の f32)
#[derive(Clone, Debug, Default)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl Plane {
    fn resize(&mut self, w: usize, h: usize) {
        self.w = w;
        self.h = h;
        self.data.resize(w * h, 0.0);
    }

    #[inline(always)]
    fn at_clamped(&self, x: isize, y: isize) -> f32 {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.data[y * self.w + x]
    }
}

/// 色差平面の縮小率 (水平, 垂直)
fn chroma_factor(s: ChromaSubsampling) -> (usize, usize) {
    match s {
        ChromaSubsampling::S444 => (1, 1),
        ChromaSubsampling::S422 => (2, 1),
        ChromaSubsampling::S420 => (2, 2),
        ChromaSubsampling::S411 => (4, 1),
    }
}

#[derive(Default)]
pub struct CaptureSim {
    out: Option<RgbaFrame>,
    /// Y, Cb, Cr
    planes: [Plane; 3],
    full_cb: Plane,
    full_cr: Plane,
    codec: Codec,
}

impl CaptureSim {
    pub fn output(&self) -> Option<&RgbaFrame> {
        self.out.as_ref()
    }

    pub(crate) fn output_mut(&mut self) -> Option<&mut RgbaFrame> {
        self.out.as_mut()
    }

    pub fn codec_stats(&self) -> CodecStats {
        self.codec.stats
    }

    /// `frame_rate`: 映像のフレーム周波数 [Hz] (圧縮の符号量の割り当てに使う)
    pub fn process(&mut self, input: &RgbaFrame, p: &CaptureParams, seed: u64, index: u64, frame_rate: f32) -> &RgbaFrame {
        let mut rng = Rng::derive(seed, index, 0, STREAM_CAPTURE);
        let drop = rng.uniform() < p.frame_drop_rate.clamp(0.0, 1.0);
        if drop && self.out.as_ref().is_some_and(|o| o.width == input.width && o.height == input.height) {
            return self.out.as_ref().unwrap();
        }

        let (w, h) = (input.width, input.height);
        // マクロブロック (16×16) の倍数に広げる
        let pw = w.div_ceil(16) * 16;
        let ph = h.div_ceil(16) * 16;
        let (fx, fy) = chroma_factor(p.chroma_subsampling);

        self.adc(input, pw, ph, p);
        let (cw, ch) = (pw / fx, ph / fy);
        downsample(&self.full_cb, &mut self.planes[1], cw, ch, fx, fy);
        downsample(&self.full_cr, &mut self.planes[2], cw, ch, fx, fy);

        if p.codec {
            self.codec.encode(&mut self.planes, p, frame_rate);
        } else {
            self.codec.reset();
        }

        let out = self.out.get_or_insert_with(|| RgbaFrame::new(w, h));
        if out.width != w || out.height != h {
            *out = RgbaFrame::new(w, h);
        }
        reconstruct(&self.planes, out, p.levels, fx, fy);
        out
    }

    /// RGB → YCbCr (4:4:4)、量子化
    fn adc(&mut self, input: &RgbaFrame, pw: usize, ph: usize, p: &CaptureParams) {
        let (w, h) = (input.width, input.height);
        self.planes[0].resize(pw, ph);
        self.full_cb.resize(pw, ph);
        self.full_cr.resize(pw, ph);
        let full_range = p.levels == CaptureLevels::Crushed;
        let (y_off, y_span, c_span) = if full_range { (0.0, 255.0, 255.0) } else { (16.0, 219.0, 224.0) };
        let step = 2f32.powi(8 - p.adc_bits.clamp(1, 8) as i32);

        let q = |v: f32| ((v / step).round() * step).clamp(0.0, 255.0);
        self.planes[0]
            .data
            .par_chunks_mut(pw)
            .zip(self.full_cb.data.par_chunks_mut(pw))
            .zip(self.full_cr.data.par_chunks_mut(pw))
            .enumerate()
            .for_each(|(y, ((py, pcb), pcr))| {
                let sy = y.min(h - 1);
                for x in 0..pw {
                    let sx = x.min(w - 1);
                    let px = &input.data[(sy * w + sx) * 4..][..3];
                    let (r, g, b) = (px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0);
                    let yy = 0.299 * r + 0.587 * g + 0.114 * b;
                    py[x] = q(y_off + y_span * yy);
                    pcb[x] = q(128.0 + c_span * (b - yy) / 1.772);
                    pcr[x] = q(128.0 + c_span * (r - yy) / 1.402);
                }
            });
    }
}

/// 箱型平均で色差を間引く
fn downsample(src: &Plane, dst: &mut Plane, cw: usize, ch: usize, fx: usize, fy: usize) {
    dst.resize(cw, ch);
    let norm = 1.0 / (fx * fy) as f32;
    dst.data.par_chunks_mut(cw).enumerate().for_each(|(y, row)| {
        for (x, d) in row.iter_mut().enumerate() {
            let mut s = 0.0;
            for dy in 0..fy {
                let r = &src.data[(y * fy + dy) * src.w..];
                for dx in 0..fx {
                    s += r[x * fx + dx];
                }
            }
            *d = s * norm;
        }
    });
}

/// 色差を補間して RGB に戻す (再生ソフト側のデコーダ)
fn reconstruct(planes: &[Plane; 3], out: &mut RgbaFrame, levels: CaptureLevels, fx: usize, fy: usize) {
    let w = out.width;
    // 送り側が 16-235 なのに 0-255 として表示する (WashedOut) などの取り違え
    let decode_full = levels == CaptureLevels::WashedOut;
    let (y_off, y_span, c_span) = if decode_full { (0.0, 255.0, 255.0) } else { (16.0, 219.0, 224.0) };
    let (py, pcb, pcr) = (&planes[0], &planes[1], &planes[2]);
    out.data.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let cy = (y / fy) as isize;
        for x in 0..w {
            // 水平は線形補間 (色差サンプルは左端の輝度サンプルと同位置)
            let cxf = x as f32 / fx as f32;
            let cx0 = cxf.floor() as isize;
            let t = cxf - cx0 as f32;
            let cb = pcb.at_clamped(cx0, cy) * (1.0 - t) + pcb.at_clamped(cx0 + 1, cy) * t;
            let cr = pcr.at_clamped(cx0, cy) * (1.0 - t) + pcr.at_clamped(cx0 + 1, cy) * t;
            let yy = (py.data[y * py.w + x] - y_off) / y_span;
            let cb = (cb - 128.0) / c_span;
            let cr = (cr - 128.0) / c_span;
            let r = yy + 1.402 * cr;
            let g = yy - 0.344_136 * cb - 0.714_136 * cr;
            let b = yy + 1.772 * cb;
            let o = &mut row[x * 4..x * 4 + 4];
            o[0] = to_u8(r);
            o[1] = to_u8(g);
            o[2] = to_u8(b);
            o[3] = 255;
        }
    });
}

#[inline(always)]
fn to_u8(v: f32) -> u8 {
    (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}
