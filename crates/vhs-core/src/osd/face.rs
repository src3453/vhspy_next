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
    /// `aspect`: 画面上の正方形 1 つが何サンプル幅になるか ([`Standard::sample_aspect`](crate::timing::Standard::sample_aspect))
    /// `em_lines`: サイズ 1 での em の高さ [ライン]。
    /// `rounding`: ドット表示で SAA5050 風の文字補間 (斜めの段差を半ドットで埋める) をする
    External { font: Box<fontdue::Font>, dots: u32, ascent: f32, descent: f32, aspect: f32, em_lines: f32, rounding: bool },
}

/// 内蔵の VCR 風ドットフォント (VCR-JP: ASCII・半角カナ・ブロック要素・矢印)
const VCR_FONT: &[u8] = include_bytes!("../../fonts/vcr-jp.ttf");

/// デッキのメニュー表示用の書体。1 em = 8 ドット、サイズ 1 で 1 ドット = 1.5 ライン
/// (サイズ 2 で 1 ドット = 3 ライン、サイズ 4 で 6 ライン)
pub fn vcr(aspect: f32) -> Face {
    let mut f = load_bytes(VCR_FONT.to_vec(), 0, 8, aspect).expect("内蔵 VCR フォント");
    if let Face::External { em_lines, rounding, .. } = &mut f {
        *em_lines = 12.0;
        *rounding = true;
    }
    f
}

impl Default for Face {
    fn default() -> Self {
        Face::Builtin
    }
}

/// フォントファイルを読む。`index` は .ttc 内のフォント番号。
pub fn load(path: &str, index: u32, dots: u32, aspect: f32) -> Result<Face, String> {
    let data = std::fs::read(path).map_err(|e| format!("フォントを読めません: {e}"))?;
    load_bytes(data, index, dots, aspect)
}

/// メモリ上のフォントデータから読む
pub fn load_bytes(data: Vec<u8>, index: u32, dots: u32, aspect: f32) -> Result<Face, String> {
    let settings = fontdue::FontSettings { collection_index: index, scale: 64.0, ..Default::default() };
    let font = fontdue::Font::from_bytes(data, settings).map_err(|e| format!("フォントを解釈できません: {e}"))?;
    let (ascent, descent) = font
        .horizontal_line_metrics(1.0)
        .map(|m| (m.ascent, -m.descent))
        .filter(|(a, d)| *a > 0.0 && *d >= 0.0)
        .unwrap_or((0.8, 0.2));
    Ok(Face::External { font: Box::new(font), dots, ascent, descent, aspect, em_lines: EM_LINES_PER_SIZE, rounding: false })
}

/// フォント名 (分かれば) を返す。GUI での確認用。
pub fn describe(path: &str, index: u32) -> Result<String, String> {
    match load(path, index, 0, 1.0)? {
        Face::External { font, .. } => Ok(font.name().unwrap_or("(名前なし)").to_string()),
        Face::Builtin => unreachable!(),
    }
}

/// 外部フォントのラスタ 1 画素の大きさ [サンプル, ライン] とラスタライズ時の px
fn raster_unit(size: u32, dots: u32, aspect: f32, em_lines: f32) -> (f32, f32, f32) {
    let em = em_lines * size as f32;
    if dots == 0 {
        // 横 1 サンプル = 1 画素で描き、縦は後で縮める
        (1.0, 1.0 / aspect, em * aspect)
    } else {
        let uh = em / dots as f32;
        (uh * aspect, uh, dots as f32)
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
            Face::External { font, dots, aspect, em_lines, rounding, .. } if self.has(ch) => {
                let (uw, uh, px) = raster_unit(size, *dots, *aspect, *em_lines);
                let (m, bytes) = font.rasterize(ch, px);
                let raster: Vec<f32> = if *dots > 0 {
                    bytes.iter().map(|&b| if b >= 128 { 1.0 } else { 0.0 }).collect()
                } else {
                    bytes.iter().map(|&b| b as f32 / 255.0).collect()
                };
                let base = self.baseline(size) as f32;
                let ox = m.xmin as f32 * uw;
                let oy = base - (m.ymin + m.height as i32) as f32 * uh;
                if *dots > 0 && *rounding {
                    let r = round_diagonals(&raster, m.width, m.height);
                    resample(&r, 2 * m.width, 2 * m.height, uw / 2.0, uh / 2.0, ox, oy)
                } else {
                    resample(&raster, m.width, m.height, uw, uh, ox, oy)
                }
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
            Face::External { font, dots, aspect, em_lines, .. } if self.has(ch) => {
                let (uw, _, px) = raster_unit(size, *dots, *aspect, *em_lines);
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
            Face::External { ascent, em_lines, .. } => (ascent * em_lines * size as f32).round() as i32,
        }
    }

    fn cell_height(&self, size: u32) -> i32 {
        match self {
            Face::Builtin => GLYPH_H as i32 * 2 * size as i32,
            Face::External { ascent, descent, em_lines, .. } => {
                ((ascent + descent) * em_lines * size as f32).round() as i32
            }
        }
    }
}

/// SAA5050 (テレテキストの文字発生器) 風の文字補間。ドットを縦横 2 倍に分け、
/// 斜めにだけつながる 2 ドット (間の 2 ドットが空) の内側の角を半ドットずつ埋めて斜線を滑らかにする。
fn round_diagonals(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    let (w2, h2) = (2 * w, 2 * h);
    let mut out = vec![0f32; w2 * h2];
    let at = |x: isize, y: isize| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && src[y as usize * w + x as usize] > 0.5;
    for y in 0..h {
        for x in 0..w {
            let (xi, yi) = (x as isize, y as isize);
            let set = |o: &mut Vec<f32>, sx: usize, sy: usize| o[(2 * y + sy) * w2 + 2 * x + sx] = 1.0;
            if at(xi, yi) {
                for (sx, sy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    set(&mut out, sx, sy);
                }
                continue;
            }
            // 空のドットの 4 つの角: その角の側の縦横の隣がともに点いていて、
            // 対角の隣が消えていれば (= 斜めだけでつながっている) 角を埋める
            for (dx, dy, sx, sy) in [(-1, -1, 0, 0), (1, -1, 1, 0), (-1, 1, 0, 1), (1, 1, 1, 1)] {
                if at(xi + dx, yi) && at(xi, yi + dy) && !at(xi + dx, yi + dy) {
                    set(&mut out, sx, sy);
                }
            }
        }
    }
    out
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
    fn rounding_fills_diagonal_corners() {
        // 斜めの 2 ドット
        // #.
        // .#
        let r = round_diagonals(&[1.0, 0.0, 0.0, 1.0], 2, 2);
        let row = |y: usize| r[y * 4..(y + 1) * 4].to_vec();
        assert_eq!(row(0), vec![1.0, 1.0, 0.0, 0.0]);
        assert_eq!(row(1), vec![1.0, 1.0, 1.0, 0.0]);
        assert_eq!(row(2), vec![0.0, 1.0, 1.0, 1.0]);
        assert_eq!(row(3), vec![0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn dilate_grows() {
        let m = Mask { x: 0, y: 0, w: 1, h: 1, data: vec![1.0] }.dilate(2, 1);
        assert_eq!((m.x, m.y, m.w, m.h), (-2, -1, 5, 3));
        assert!(m.data.iter().all(|&v| v == 1.0));
    }
}
