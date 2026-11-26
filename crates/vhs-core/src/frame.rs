//! 画像バッファとコンポジット信号のフィールドバッファ。

use crate::timing::{NTSC, Standard};
use rayon::prelude::*;

/// RGB24 (R, G, B の順) の画像
#[derive(Clone, Debug)]
pub struct RgbFrame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl RgbFrame {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, data: vec![0; width * height * 3] }
    }

    pub fn from_raw(width: usize, height: usize, data: Vec<u8>) -> Option<Self> {
        (data.len() == width * height * 3).then_some(Self { width, height, data })
    }

    /// バイリニア補間でリサイズ (縮小時は簡易的に 2x2 平均)。
    pub fn resized(&self, width: usize, height: usize) -> RgbFrame {
        if width == self.width && height == self.height {
            return self.clone();
        }
        let mut out = RgbFrame::new(width, height);
        let sx = self.width as f32 / width as f32;
        let sy = self.height as f32 / height as f32;
        let (sw, sh) = (self.width, self.height);
        out.data.par_chunks_mut(width * 3).enumerate().for_each(|(y, row)| {
            let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (sh - 1) as f32);
            let y0 = fy as usize;
            let y1 = (y0 + 1).min(sh - 1);
            let wy = fy - y0 as f32;
            for x in 0..width {
                let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (sw - 1) as f32);
                let x0 = fx as usize;
                let x1 = (x0 + 1).min(sw - 1);
                let wx = fx - x0 as f32;
                for c in 0..3 {
                    let p = |xx: usize, yy: usize| self.data[(yy * sw + xx) * 3 + c] as f32;
                    let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * wx;
                    let bot = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * wx;
                    row[x * 3 + c] = (top + (bot - top) * wy + 0.5) as u8;
                }
            }
        });
        out
    }
}

/// シーンリニアな RGB 画像 (HDR)。3DCG のレンダラなどから直接渡す。
///
/// - 原色は Rec.709 / sRGB、値は光量に比例する (ガンマなし)。
/// - 1.0 が露出 0 EV でのカメラの白 (撮像管の白レベル電流) にあたる。1.0 を超える値も
///   そのまま扱い、ビーム電流の上限を超えた分がコメットテールやブルーミングになる。
/// - 負の値と NaN は 0 として扱う。
#[derive(Clone, Debug)]
pub struct LinearFrame {
    pub width: usize,
    pub height: usize,
    /// R, G, B の順に並んだ f32
    pub data: Vec<f32>,
}

impl LinearFrame {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, data: vec![0.0; width * height * 3] }
    }

    pub fn from_raw(width: usize, height: usize, data: Vec<f32>) -> Option<Self> {
        (data.len() == width * height * 3).then_some(Self { width, height, data })
    }

    /// 8 bit の画像を表示ガンマ 2.2 として線形化する (撮像管カメラが 8 bit 入力に使う変換と同じ)
    pub fn from_rgb(f: &RgbFrame) -> Self {
        let lut: [f32; 256] = std::array::from_fn(|i| (i as f32 / 255.0).powf(2.2));
        Self { width: f.width, height: f.height, data: f.data.iter().map(|&v| lut[v as usize]).collect() }
    }

    /// 表示用の 8 bit 画像にする (0..1 で切り、ガンマ 1/2.2)
    pub fn to_rgb(&self) -> RgbFrame {
        let data = self
            .data
            .par_iter()
            .map(|&v| {
                let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
                (v.powf(1.0 / 2.2) * 255.0 + 0.5) as u8
            })
            .collect();
        RgbFrame { width: self.width, height: self.height, data }
    }

    /// 光量を保つリサイズ (三角フィルタ。縮小時はフィルタ幅を広げて面積平均に近づける)。
    /// 小さな光源を縮小しても、明るさが失われずに面積に応じて薄まる。
    pub fn resized(&self, width: usize, height: usize) -> LinearFrame {
        if width == self.width && height == self.height {
            return self.clone();
        }
        let horiz = resample_weights(self.width, width);
        let vert = resample_weights(self.height, height);
        let sw = self.width;
        // 横方向
        let mut tmp = vec![0f32; width * self.height * 3];
        tmp.par_chunks_mut(width * 3).enumerate().for_each(|(y, row)| {
            let src = &self.data[y * sw * 3..(y + 1) * sw * 3];
            for (x, (start, ws)) in horiz.iter().enumerate() {
                let mut acc = [0f32; 3];
                for (j, wv) in ws.iter().enumerate() {
                    let px = &src[(start + j) * 3..][..3];
                    for c in 0..3 {
                        acc[c] += sanitize(px[c]) * wv;
                    }
                }
                row[x * 3..x * 3 + 3].copy_from_slice(&acc);
            }
        });
        // 縦方向
        let mut out = LinearFrame::new(width, height);
        out.data.par_chunks_mut(width * 3).enumerate().for_each(|(y, row)| {
            let (start, ws) = &vert[y];
            for (j, wv) in ws.iter().enumerate() {
                let src = &tmp[(start + j) * width * 3..][..width * 3];
                for (o, s) in row.iter_mut().zip(src) {
                    *o += s * wv;
                }
            }
        });
        out
    }
}

#[inline]
pub(crate) fn sanitize(v: f32) -> f32 {
    if v > 0.0 { v } else { 0.0 } // NaN もここで 0 になる
}

/// 出力画素ごとの (最初の入力画素, 重み列)。重みの和は 1。
fn resample_weights(src: usize, dst: usize) -> Vec<(usize, Vec<f32>)> {
    let scale = src as f32 / dst as f32;
    let support = scale.max(1.0);
    (0..dst)
        .map(|i| {
            let center = (i as f32 + 0.5) * scale;
            let lo = ((center - support).floor().max(0.0)) as usize;
            let hi = ((center + support).ceil() as usize).min(src);
            let mut ws: Vec<f32> =
                (lo..hi).map(|j| (1.0 - ((j as f32 + 0.5 - center) / support).abs()).max(0.0)).collect();
            let sum: f32 = ws.iter().sum();
            if sum > 0.0 {
                ws.iter_mut().for_each(|w| *w /= sum);
            } else {
                // 端の念のため: 最も近い画素
                let j = (center as usize).min(src - 1);
                return (j, vec![1.0]);
            }
            (lo, ws)
        })
        .collect()
}

/// ガンマ補正済みの映像信号 (エンコーダの入力)。
///
/// R, G, B の f32 で 0.0 = 黒 (7.5 IRE)、1.0 = 白 (100 IRE)。カメラのニー・ホワイトクリップを
/// 通った 1.0 を超える値はスーパーホワイトとしてそのままコンポジット信号になる。
#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

/// コンポジットエンコーダへ直接渡す YIQ テスト信号。Y/I/Q は白レベルを 1 とする。
#[derive(Clone, Debug)]
pub struct YiqFrame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<[f32; 3]>,
}

impl YiqFrame {
    pub fn new(width: usize, height: usize, color: [f32; 3]) -> Self {
        Self { width, height, data: vec![color; width * height] }
    }
}

impl VideoFrame {
    pub fn new(width: usize, height: usize) -> Self {
        Self { width, height, data: vec![0.0; width * height * 3] }
    }

    fn ensure(&mut self, width: usize, height: usize) {
        if self.width != width || self.height != height {
            *self = Self::new(width, height);
        }
    }

    /// 8 bit の画像 (0..255 → 0..1) を写す
    pub fn copy_from_rgb(&mut self, f: &RgbFrame) {
        self.ensure(f.width, f.height);
        self.data.par_chunks_mut(f.width * 3).zip(f.data.par_chunks(f.width * 3)).for_each(|(o, s)| {
            for (o, &s) in o.iter_mut().zip(s) {
                *o = s as f32 * (1.0 / 255.0);
            }
        });
    }

    /// シーンリニアな画像を 0..1 で切ってガンマ 1/2.2 で写す (カメラを通さないとき)
    pub fn copy_from_linear(&mut self, f: &LinearFrame) {
        self.ensure(f.width, f.height);
        self.data.par_chunks_mut(f.width * 3).zip(f.data.par_chunks(f.width * 3)).for_each(|(o, s)| {
            for (o, &s) in o.iter_mut().zip(s) {
                *o = sanitize(s).min(1.0).powf(1.0 / 2.2);
            }
        });
    }

    pub fn from_rgb(f: &RgbFrame) -> Self {
        let mut v = Self::new(f.width, f.height);
        v.copy_from_rgb(f);
        v
    }
}

/// RGBA32 の画像 (Canvas の ImageData にそのまま渡せる形式)
#[derive(Clone, Debug)]
pub struct RgbaFrame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl RgbaFrame {
    pub fn new(width: usize, height: usize) -> Self {
        let mut data = vec![0; width * height * 4];
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Self { width, height, data }
    }
}

/// 1 フィールド分の映像信号 (raster_lines × line_samples, 単位 IRE)。
///
/// 有効映像だけでなく、等化パルス・垂直同期・VBI を含む全ラスタを持つ ([`crate::raster`])。
/// バッファの先頭は垂直同期の始まるライン、有効映像の 1 行目は [`Field::active_index`] で求める。
///
/// 通常はコンポジット信号 (`data`)。S 映像 (Y/C 分離) で送るときは `separate` が立ち、
/// `data` が Y (同期付き)、`chroma` が C (バースト付き) になる。
#[derive(Clone, Debug)]
pub struct Field {
    /// この信号のテレビ方式
    pub std: &'static Standard,
    pub data: Vec<f32>,
    /// S 映像のとき true (`data` = Y, `chroma` = C)
    pub separate: bool,
    /// S 映像の C 信号 (`separate` のときだけ使う)
    pub chroma: Vec<f32>,
    /// 0 = 第 1 (偶数ライン) フィールド, 1 = 第 2 (奇数ライン) フィールド
    pub parity: u8,
    /// フレーム番号
    pub frame: u64,
    /// 通算フィールド番号
    pub index: u64,
}

impl Default for Field {
    fn default() -> Self {
        Self::new(&NTSC)
    }
}

impl Field {
    pub fn new(std: &'static Standard) -> Self {
        Self {
            std,
            data: vec![0.0; std.raster_lines * std.line_samples],
            separate: false,
            chroma: Vec::new(),
            parity: 0,
            frame: 0,
            index: 0,
        }
    }

    pub fn set_index(&mut self, index: u64) {
        self.index = index;
        self.parity = (index & 1) as u8;
        self.frame = index / 2;
    }

    /// バッファのライン数 (帰線区間込み)
    #[inline]
    pub fn lines(&self) -> usize {
        self.std.raster_lines
    }

    /// 有効映像の l 行目 (フレーム画像の行 2l + parity) がバッファの何ライン目か
    #[inline]
    pub fn active_index(&self, l: usize) -> usize {
        self.std.active_raster_index(self.parity) + l
    }

    /// バッファの l ライン目の通算ライン番号
    #[inline]
    pub fn stream_line(&self, l: usize) -> u64 {
        self.std.stream_line(l, self.frame, self.parity)
    }

    /// バッファの l ライン目 (帰線区間込み)
    pub fn line(&self, l: usize) -> &[f32] {
        &self.data[l * self.std.line_samples..(l + 1) * self.std.line_samples]
    }

    pub fn line_mut(&mut self, l: usize) -> &mut [f32] {
        &mut self.data[l * self.std.line_samples..(l + 1) * self.std.line_samples]
    }

    /// S 映像に切り替え、C 信号の領域を用意する
    pub fn set_separate(&mut self, separate: bool) {
        self.separate = separate;
        if separate && self.chroma.len() != self.data.len() {
            self.chroma = vec![0.0; self.data.len()];
        }
    }

    /// S 映像の C 信号のライン (コンポジットなら None)
    pub fn chroma_line(&self, l: usize) -> Option<&[f32]> {
        let ls = self.std.line_samples;
        self.separate.then(|| &self.chroma[l * ls..(l + 1) * ls])
    }

    /// Y (またはコンポジット) と C (S 映像のときだけ) のラインを同時に借りる
    pub fn lines_mut(&mut self, l: usize) -> (&mut [f32], Option<&mut [f32]>) {
        let ls = self.std.line_samples;
        let y = &mut self.data[l * ls..(l + 1) * ls];
        let c = if self.separate { Some(&mut self.chroma[l * ls..(l + 1) * ls]) } else { None };
        (y, c)
    }
}
