//! OSD (オンスクリーン表示) のシミュレーション。
//!
//! 文字はマークアップで記述する (書式は [`markup`] を参照)。合成先は 2 通り:
//!
//! - [`OsdMode::Tape`](crate::params::OsdMode::Tape): エンコード前の RGB 画像に焼き込む。カメラの日付表示のように
//!   テープに記録された文字を再現し、テープの劣化 (にじみ・ノイズ・ジッタ) を一緒に受ける。
//! - [`OsdMode::Composite`](crate::params::OsdMode::Composite): テープ再生後のコンポジット信号に差し込む。デッキの
//!   キャラクタジェネレータが出力に重畳する表示を再現する。文字はくっきりしているが、
//!   再生信号の水平同期に追従するので時間軸の揺れは画面と一緒に受ける。

pub mod face;
mod font;
pub mod markup;

use crate::decoder::{find_sync_edge, pal_ident};
use crate::frame::{Field, RgbaFrame, VideoFrame};
use crate::params::OsdParams;
use crate::timing::*;
use face::{Face, Mask};
use markup::{Context, Glyph, Rgba};
use std::collections::HashMap;
use std::sync::Arc;

const IQ_ROT: f32 = 33.0 * std::f32::consts::PI / 180.0;

/// OSD のオーバーレイ (乗算済みアルファの RGBA, 0..1) とその状態
pub struct Osd {
    std: &'static Standard,
    /// オーバーレイの大きさ (= 方式の画像サイズ)
    w: usize,
    h: usize,
    overlay: Vec<[f32; 4]>,
    /// 前回描画した表示リスト (変化がなければ描き直さない)
    last: Vec<Glyph>,
    /// 描画済み範囲 (行) — 空なら None
    rows: Option<(usize, usize)>,
    face: Face,
    /// 読み込み済みフォントの設定 (パス, 番号, ドット数, 補間)
    face_key: Option<(String, u32, u32, bool)>,
    face_error: Option<String>,
    /// 書体を固定する (`OsdParams` のフォント設定を無視する。デッキのメニュー表示用)
    fixed_face: bool,
    /// (文字, サイズ) → (文字マスク, 縁取りマスク)
    cache: HashMap<(char, u32), Arc<(Mask, Mask)>>,
}

impl Default for Osd {
    fn default() -> Self {
        Self::new(&NTSC)
    }
}

impl Osd {
    pub fn new(std: &'static Standard) -> Self {
        let (w, h) = std.image_size();
        Self {
            std,
            w,
            h,
            overlay: vec![[0.0; 4]; w * h],
            last: Vec::new(),
            rows: None,
            face: Face::Builtin,
            face_key: None,
            face_error: None,
            fixed_face: false,
            cache: HashMap::new(),
        }
    }

    /// 書体を固定した OSD (デッキのメニュー表示用。内蔵 VCR-JP フォントを使う)
    pub fn with_vcr_face(std: &'static Standard) -> Self {
        Self { face: face::vcr(std.sample_aspect(), true), fixed_face: true, ..Self::new(std) }
    }
}

impl Osd {
    /// 表示内容を更新する。戻り値は何か描画すべきものがあるか。
    pub fn update(&mut self, p: &OsdParams, field_index: u64) -> bool {
        let font_changed = self.update_face(p);
        let ctx = Context::new(&p.start_datetime, field_index, self.std.field_rate);
        let glyphs: Vec<Glyph> = markup::layout(&p.markup, p.scale.clamp(1, 8), &ctx, &self.face)
            .into_iter()
            .filter(|g| !g.style.blink || ctx.blink_on())
            .collect();
        if font_changed || glyphs != self.last || (self.rows.is_none() && !glyphs.is_empty()) {
            self.render(&glyphs);
            self.last = glyphs;
        }
        self.rows.is_some()
    }

    /// 外部フォントの読み込みに失敗していればその理由 (内蔵フォントで代用中)
    pub fn font_error(&self) -> Option<&str> {
        self.face_error.as_deref()
    }

    /// フォント設定が変わっていれば読み直す
    fn update_face(&mut self, p: &OsdParams) -> bool {
        if self.fixed_face {
            return false;
        }
        let path = p.font.trim();
        let key = (path.to_string(), p.font_index, p.font_dots.min(64), p.font_rounding);
        if self.face_key.as_ref() == Some(&key) {
            return false;
        }
        (self.face, self.face_error) = if path.is_empty() {
            (face::vcr(self.std.sample_aspect(), p.font_rounding), None)
        } else {
            match face::load(path, key.1, key.2, self.std.sample_aspect()) {
                Ok(mut f) => {
                    face::set_rounding(&mut f, p.font_rounding);
                    (f, None)
                }
                Err(e) => (face::vcr(self.std.sample_aspect(), p.font_rounding), Some(e)),
            }
        };
        self.face_key = Some(key);
        self.cache.clear();
        true
    }

    fn glyph_masks(&mut self, ch: char, size: u32) -> Arc<(Mask, Mask)> {
        let face = &self.face;
        self.cache
            .entry((ch, size))
            .or_insert_with(|| {
                let m = face.mask(ch, size);
                // 縁取りは横 2 サンプル、縦 2 ライン (= 1 フィールドライン) 太らせる
                let e = m.dilate(2, 2);
                Arc::new((m, e))
            })
            .clone()
    }

    fn render(&mut self, glyphs: &[Glyph]) {
        let w = self.w;
        if let Some((y0, y1)) = self.rows.take() {
            self.overlay[y0 * w..y1 * w].fill([0.0; 4]);
        }
        let mut bounds = Bounds::default();
        // 背景 → 影 → 縁取り → 文字 の順に重ねる
        for g in glyphs {
            if let Some(bg) = g.style.bg {
                let pad = 2 * g.style.size as i32;
                let x0 = g.x - g.spacing / 2;
                self.fill(x0, g.y - pad, x0 + g.advance, g.y + g.height + pad, bg, &mut bounds);
            }
        }
        let masks: Vec<_> = glyphs.iter().map(|g| self.glyph_masks(g.ch, g.style.size)).collect();
        for (g, m) in glyphs.iter().zip(&masks) {
            if let Some(c) = g.style.shadow {
                let d = 2 * g.style.size as i32;
                self.blit(&m.0, g.x + d, g.y + d, c, &mut bounds);
            }
        }
        for (g, m) in glyphs.iter().zip(&masks) {
            if let Some(c) = g.style.edge {
                self.blit(&m.1, g.x, g.y, c, &mut bounds);
            }
        }
        for (g, m) in glyphs.iter().zip(&masks) {
            self.blit(&m.0, g.x, g.y, g.style.color, &mut bounds);
        }
        self.rows = bounds.rows();
    }

    /// カバレッジマスクを (x, y) を原点として色 c で重ねる
    fn blit(&mut self, m: &Mask, x: i32, y: i32, c: Rgba, b: &mut Bounds) {
        if c.a <= 0.0 || m.w == 0 {
            return;
        }
        let (mx, my) = (x + m.x, y + m.y);
        let (w, h) = (self.w, self.h);
        let ys = my.max(0)..(my + m.h as i32).min(h as i32);
        let xs = mx.max(0)..(mx + m.w as i32).min(w as i32);
        if ys.is_empty() || xs.is_empty() {
            return;
        }
        b.add(ys.start as usize, ys.end as usize);
        let ca = c.a.min(1.0);
        for py in ys {
            let mrow = &m.data[(py - my) as usize * m.w..][..m.w];
            for px in xs.clone() {
                let a = ca * mrow[(px - mx) as usize];
                if a <= 0.0 {
                    continue;
                }
                let o = &mut self.overlay[py as usize * w + px as usize];
                let src = [c.r * a, c.g * a, c.b * a, a];
                for k in 0..4 {
                    o[k] = src[k] + o[k] * (1.0 - a);
                }
            }
        }
    }

    fn fill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba, b: &mut Bounds) {
        let (w, h) = (self.w, self.h);
        let (x0, x1) = (x0.clamp(0, w as i32) as usize, x1.clamp(0, w as i32) as usize);
        let (y0, y1) = (y0.clamp(0, h as i32) as usize, y1.clamp(0, h as i32) as usize);
        if x0 >= x1 || y0 >= y1 || c.a <= 0.0 {
            return;
        }
        b.add(y0, y1);
        let a = c.a.min(1.0);
        let src = [c.r * a, c.g * a, c.b * a, a];
        for y in y0..y1 {
            for px in &mut self.overlay[y * w + x0..y * w + x1] {
                for k in 0..4 {
                    px[k] = src[k] + px[k] * (1.0 - a);
                }
            }
        }
    }

    /// エンコード前の画像に焼き込む
    pub fn burn_into(&self, frame: &mut VideoFrame, opacity: f32) {
        let Some((y0, y1)) = self.rows else { return };
        let w = self.w;
        debug_assert_eq!((frame.width, frame.height), (w, self.h));
        for y in y0..y1 {
            let src = &self.overlay[y * w..(y + 1) * w];
            let dst = &mut frame.data[y * w * 3..(y + 1) * w * 3];
            for (o, px) in src.iter().zip(dst.chunks_exact_mut(3)) {
                let a = o[3] * opacity;
                if a <= 0.0 {
                    continue;
                }
                for k in 0..3 {
                    px[k] = px[k] * (1.0 - a) + o[k] * opacity;
                }
            }
        }
    }

    /// すべての映像処理後の表示用 RGBA 画像に重ねる
    pub fn burn_into_rgba(&self, frame: &mut RgbaFrame, opacity: f32) {
        let Some((y0, y1)) = self.rows else { return };
        let w = self.w;
        debug_assert_eq!((frame.width, frame.height), (w, self.h));
        for y in y0..y1 {
            let src = &self.overlay[y * w..(y + 1) * w];
            let dst = &mut frame.data[y * w * 4..(y + 1) * w * 4];
            for (o, px) in src.iter().zip(dst.chunks_exact_mut(4)) {
                let a = (o[3] * opacity).clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                for k in 0..3 {
                    let v = o[k] * opacity + px[k] as f32 / 255.0 * (1.0 - a);
                    px[k] = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
    }

    /// 再生後のコンポジット信号に差し込む。各ラインの同期位置に追従する。
    pub fn insert_composite(&self, field: &mut Field, opacity: f32) {
        let Some((y0, y1)) = self.rows else { return };
        let s = field.std;
        debug_assert!(std::ptr::eq(s, self.std));
        let (w, lv) = (self.w, s.ire);
        let span = lv.span();
        let pal = s.is_pal();
        // キャラクタジェネレータは再生信号の H 同期でタイミングを取る
        let sync_offset = |line: &[f32]| {
            find_sync_edge(line, s).map(|e| e - s.sync_end as f32).filter(|d| d.abs() < 40.0).unwrap_or(0.0)
        };
        // PAL: 再生信号のバーストから PAL スイッチの向きを決める (テープの垂直揺れでライン順がずれるため)
        let s0 = if pal {
            let mus: Vec<Option<f32>> = (0..s.field_lines)
                .map(|l| {
                    let line = field.line(l);
                    // S 映像ならバーストは C 側にある
                    Some(burst_phase(field.chroma_line(l).unwrap_or(line), sync_offset(line), s))
                })
                .collect();
            pal_ident(&mus, s)
        } else {
            1.0
        };
        for l in 0..s.field_lines {
            let row = 2 * l + field.parity as usize;
            if row < y0 || row >= y1 {
                continue;
            }
            let (line, mut c_line) = field.lines_mut(l);
            let d = sync_offset(line);
            let sv = if !pal || l % 2 == 0 { s0 } else { -s0 };
            // バーストの位相 μ から副搬送波位相 φ へ (PAL は μ = φ - 45°·sV)
            let burst_src: &[f32] = match &c_line {
                Some(c) => c,
                None => line,
            };
            let psi = burst_phase(burst_src, d, s) + if pal { std::f32::consts::FRAC_PI_4 * sv } else { 0.0 };
            let src = &self.overlay[row * w..(row + 1) * w];
            for x in 0..w {
                // 文字発生器の立ち上がり (≒100 ns) を 3 タップで近似
                let at = |i: isize| src[i.clamp(0, w as isize - 1) as usize];
                let (a, b, c) = (at(x as isize - 1), at(x as isize), at(x as isize + 1));
                let mut o = [0f32; 4];
                for k in 0..4 {
                    o[k] = (0.25 * a[k] + 0.5 * b[k] + 0.25 * c[k]) * opacity;
                }
                if o[3] <= 1e-4 {
                    continue;
                }
                let pos = s.active_start as f32 + x as f32 + d;
                let m = pos.round() as isize;
                if m < 0 || m >= s.line_samples as isize {
                    continue;
                }
                let (r, g, bb) = (o[0], o[1], o[2]);
                let y = lv.black * o[3] + span * (0.299 * r + 0.587 * g + 0.114 * bb);
                // NTSC: (i, q) = (I, Q)、PAL: (i, q) = (±V, U)。どちらも i は cos 側、q は sin 側
                let (i, q, rot) = if pal {
                    (sv * span * (0.615 * r - 0.515 * g - 0.100 * bb), span * (-0.147 * r - 0.289 * g + 0.436 * bb), 0.0)
                } else {
                    (span * (0.5959 * r - 0.2746 * g - 0.3213 * bb), span * (0.2115 * r - 0.5227 * g + 0.3112 * bb), IQ_ROT)
                };
                // 色副搬送波は再生バーストに位相同期させる (デッキ内の APC 出力を使う想定)
                let theta = m as f32 * std::f32::consts::FRAC_PI_2 + psi + rot;
                let m = m as usize;
                match c_line.as_deref_mut() {
                    // S 映像: Y と C に分けて重ねる
                    Some(c) => {
                        line[m] = line[m] * (1.0 - o[3]) + y;
                        c[m] = c[m] * (1.0 - o[3]) + i * theta.cos() + q * theta.sin();
                    }
                    None => {
                        let s = y + i * theta.cos() + q * theta.sin();
                        line[m] = line[m] * (1.0 - o[3]) + s;
                    }
                }
            }
        }
    }
}

/// ラインのバーストから副搬送波位相 φ0 を求める (バースト = -sin(nπ/2 + φ0))。
/// テープの輝度経路の遅延で同期位置とバースト位相の関係は崩れるので、同期からは求めない。
fn burst_phase(line: &[f32], d: f32, s: &Standard) -> f32 {
    let start = (s.burst_start as f32 + d).round().max(0.0) as usize & !3;
    let end = ((s.burst_end + 16) as f32 + d).round().clamp(0.0, s.active_start as f32) as usize;
    let (mut zr, mut zi) = (0f32, 0f32);
    // 4 サンプル (1 周期) 単位で足して直流 (ブランキング) を打ち消す
    for n in (start..end.saturating_sub(3)).step_by(4).flat_map(|c| c..c + 4) {
        zr += line[n] * COS4[n & 3];
        zi -= line[n] * SIN4[n & 3];
    }
    zi.atan2(zr) - std::f32::consts::FRAC_PI_2
}

#[derive(Default)]
struct Bounds(Option<(usize, usize)>);

impl Bounds {
    fn add(&mut self, y0: usize, y1: usize) {
        self.0 = Some(match self.0 {
            Some((a, b)) => (a.min(y0), b.max(y1)),
            None => (y0, y1),
        });
    }
    fn rows(&self) -> Option<(usize, usize)> {
        self.0
    }
}
