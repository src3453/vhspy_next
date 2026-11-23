//! OSD の書体: 内蔵 5×7 ドットフォントと、外部の TrueType / OpenType フォント。
//!
//! 外部フォントは 2 通りで描ける。
//! - ドット表示 (`dots` > 0): 1 em を `dots` ドットでラスタライズして 2 値化し、ドットを
//!   画面上で正方形になるよう拡大する。漢字 ROM を持つキャラクタジェネレータ (16 ドット等) 風。
//! - なめらか (`dots` = 0): 画素単位でアンチエイリアスして描く。
//!
//! どちらも最後は「セル左上を原点とした、サンプル × ライン単位のカバレッジマスク」にする。

use super::font::{GLYPH_H, GLYPH_W, glyph};
use super::markup::Metrics;
use crate::timing::{ACTIVE_WIDTH, FRAME_LINES};

/// 画面上の正方形 1 つが何サンプル幅になるか (754 サンプル = 4:3 の 480 ライン相当の幅)
pub const SAMPLE_ASPECT: f32 = ACTIVE_WIDTH as f32 / (FRAME_LINES as f32 * 4.0 / 3.0);

/// 外部フォントの em の高さ [ライン / サイズ]。大文字の高さが内蔵フォント (14 ライン/サイズ) と同程度になる
const EM_LINES_PER_SIZE: f32 = 18.0;

/// セル左上を原点とするカバレッジ (0..1) のマスク
#[derive(Clone, Debug, Default)]
pub struct Mask {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl Mask {
    /// 各方向に (gx, gy) 画素だけ太らせたマスク (縁取り用)
    pub fn dilate(&self, gx: usize, gy: usize) -> Mask {
        let (w, h) = (self.w + 2 * gx, self.h + 2 * gy);
        // 横方向の最大値 → 縦方向の最大値
        let mut tmp = vec![0f32; w * self.h];
        for y in 0..self.h {
            let src = &self.data[y * self.w..(y + 1) * self.w];
            for x in 0..w {
                let lo = x.saturating_sub(2 * gx);
                let hi = (x + 1).min(self.w);
                tmp[y * w + x] = src.get(lo..hi).map_or(0.0, |s| s.iter().fold(0f32, |a, &b| a.max(b)));
            }
        }
        let mut data = vec![0f32; w * h];
        for y in 0..h {
            let lo = y.saturating_sub(2 * gy);
            let hi = (y + 1).min(self.h);
            for x in 0..w {
                data[y * w + x] = (lo..hi).map(|r| tmp[r * w + x]).fold(0f32, f32::max);
            }
        }
        Mask { x: self.x - gx as i32, y: self.y - gy as i32, w, h, data }
    }
}

pub enum Face {
    Builtin,
    External { font: Box<fontdue::Font>, dots: u32, ascent: f32, descent: f32 },
}

impl Default for Face {
    fn default() -> Self {
        Face::Builtin
    }
}

/// フォントファイルを読む。`index` は .ttc 内のフォント番号。
pub fn load(path: &str, index: u32, dots: u32) -> Result<Face, String> {
    let data = std::fs::read(path).map_err(|e| format!("フォントを読めません: {e}"))?;
    let settings = fontdue::FontSettings { collection_index: index, scale: 64.0, ..Default::default() };
    let font = fontdue::Font::from_bytes(data, settings).map_err(|e| format!("フォントを解釈できません: {e}"))?;
    let (ascent, descent) = font
        .horizontal_line_metrics(1.0)
        .map(|m| (m.ascent, -m.descent))
        .filter(|(a, d)| *a > 0.0 && *d >= 0.0)
        .unwrap_or((0.8, 0.2));
    Ok(Face::External { font: Box::new(font), dots, ascent, descent })
}

/// フォント名 (分かれば) を返す。GUI での確認用。
pub fn describe(path: &str, index: u32) -> Result<String, String> {
    match load(path, index, 0)? {
        Face::External { font, .. } => Ok(font.name().unwrap_or("(名前なし)").to_string()),
        Face::Builtin => unreachable!(),
    }
}

/// 外部フォントのラスタ 1 画素の大きさ [サンプル, ライン] とラスタライズ時の px
fn raster_unit(size: u32, dots: u32) -> (f32, f32, f32) {
    let em = EM_LINES_PER_SIZE * size as f32;
    if dots == 0 {
        // 横 1 サンプル = 1 画素で描き、縦は後で縮める
        (1.0, 1.0 / SAMPLE_ASPECT, em * SAMPLE_ASPECT)
    } else {
        let uh = em / dots as f32;
        (uh * SAMPLE_ASPECT, uh, dots as f32)
    }
}

fn builtin_advance(size: u32) -> i32 {
    (GLYPH_W as i32 + 1) * 2 * size as i32
}

impl Face {
    /// 外部フォントにこの文字があるか (なければ内蔵フォントで代用する)
    fn has(&self, ch: char) -> bool {
        match self {
            Face::Builtin => false,
            Face::External { font, .. } => ch == ' ' || font.lookup_glyph_index(ch) != 0,
        }
    }

    /// 文字のマスクを作る (セル左上が原点)
    pub fn mask(&self, ch: char, size: u32) -> Mask {
        match self {
            Face::External { font, dots, .. } if self.has(ch) => {
                let (uw, uh, px) = raster_unit(size, *dots);
                let (m, bytes) = font.rasterize(ch, px);
                let raster: Vec<f32> = if *dots > 0 {
                    bytes.iter().map(|&b| if b >= 128 { 1.0 } else { 0.0 }).collect()
                } else {
                    bytes.iter().map(|&b| b as f32 / 255.0).collect()
                };
                let base = self.baseline(size) as f32;
                let ox = m.xmin as f32 * uw;
                let oy = base - (m.ymin + m.height as i32) as f32 * uh;
                resample(&raster, m.width, m.height, uw, uh, ox, oy)
            }
            _ => {
                // 内蔵フォント (外部フォントの代用時はベースラインを揃える)
                let s = 2.0 * size as f32;
                let bits = glyph(ch);
                let raster: Vec<f32> = bits
                    .iter()
                    .flat_map(|row| (0..GLYPH_W).map(move |c| ((row >> (GLYPH_W - 1 - c)) & 1) as f32))
                    .collect();
                let oy = (self.baseline(size) - (GLYPH_H as i32 * 2 * size as i32)) as f32;
                resample(&raster, GLYPH_W, GLYPH_H, s, s, 0.0, oy)
            }
        }
    }
}

impl Metrics for Face {
    fn advance(&self, ch: char, size: u32) -> i32 {
        match self {
            Face::External { font, dots, .. } if self.has(ch) => {
                let (uw, _, px) = raster_unit(size, *dots);
                (font.metrics(ch, px).advance_width * uw).round() as i32
            }
            _ => builtin_advance(size),
        }
    }

    fn spacing(&self, ch: char, size: u32) -> i32 {
        if self.has(ch) { 0 } else { 2 * size as i32 }
    }

    fn baseline(&self, size: u32) -> i32 {
        match self {
            Face::Builtin => GLYPH_H as i32 * 2 * size as i32,
            Face::External { ascent, .. } => (ascent * EM_LINES_PER_SIZE * size as f32).round() as i32,
        }
    }

    fn cell_height(&self, size: u32) -> i32 {
        match self {
            Face::Builtin => GLYPH_H as i32 * 2 * size as i32,
            Face::External { ascent, descent, .. } => {
                ((ascent + descent) * EM_LINES_PER_SIZE * size as f32).round() as i32
            }
        }
    }
}

/// 1 画素が (uw, uh) の大きさを持つラスタを、左上 (ox, oy) に置いて 1×1 の格子へ面積平均で写す
fn resample(src: &[f32], sw: usize, sh: usize, uw: f32, uh: f32, ox: f32, oy: f32) -> Mask {
    if sw == 0 || sh == 0 {
        return Mask { x: ox.floor() as i32, y: oy.floor() as i32, ..Default::default() };
    }
    let (x0, wx) = weights(sw, uw, ox);
    let (y0, wy) = weights(sh, uh, oy);
    let (w, h) = (wx.len(), wy.len());
    // 横 → 縦の分離型
    let mut tmp = vec![0f32; w * sh];
    for r in 0..sh {
        let row = &src[r * sw..(r + 1) * sw];
        for (tx, ws) in wx.iter().enumerate() {
            tmp[r * w + tx] = ws.iter().map(|&(k, a)| row[k] * a).sum();
        }
    }
    let mut data = vec![0f32; w * h];
    for (ty, ws) in wy.iter().enumerate() {
        for tx in 0..w {
            data[ty * w + tx] = ws.iter().map(|&(k, a)| tmp[k * w + tx] * a).sum::<f32>().min(1.0);
        }
    }
    Mask { x: x0, y: y0, w, h, data }
}

/// 長さ u のセル n 個 (原点 o) と単位格子との重なり
fn weights(n: usize, u: f32, o: f32) -> (i32, Vec<Vec<(usize, f32)>>) {
    let start = o.floor() as i32;
    let end = (o + n as f32 * u - 1e-4).ceil() as i32;
    let out = (start..end)
        .map(|t| {
            let t = t as f32;
            let kmin = (((t - o) / u).floor().max(0.0)) as usize;
            let kmax = ((((t + 1.0 - o) / u).ceil()) as usize).min(n);
            (kmin..kmax)
                .filter_map(|k| {
                    let a = (t + 1.0).min(o + (k + 1) as f32 * u) - t.max(o + k as f32 * u);
                    (a > 1e-6).then_some((k, a))
                })
                .collect()
        })
        .collect();
    (start, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_mask_is_exact_dots() {
        let m = Face::Builtin.mask('I', 2);
        assert_eq!((m.x, m.y, m.w, m.h), (0, 0, 20, 28));
        // 'I' の 1 行目は 01110 → 4..16 サンプルが 1
        let want: Vec<f32> = (0..20).map(|x| if (4..16).contains(&x) { 1.0 } else { 0.0 }).collect();
        assert_eq!(&m.data[0..20], &want[..]);
    }

    #[test]
    fn resample_area() {
        // 1.5 画素幅の 1 セルを 0.25 から置く → [0.75, 0.75]
        let m = resample(&[1.0], 1, 1, 1.5, 1.0, 0.25, 0.0);
        assert_eq!((m.x, m.w), (0, 2));
        assert!((m.data[0] - 0.75).abs() < 1e-5 && (m.data[1] - 0.75).abs() < 1e-5);
    }

    #[test]
    fn dilate_grows() {
        let m = Mask { x: 0, y: 0, w: 1, h: 1, data: vec![1.0] }.dilate(2, 1);
        assert_eq!((m.x, m.y, m.w, m.h), (-2, -1, 5, 3));
        assert!(m.data.iter().all(|&v| v == 1.0));
    }
}
