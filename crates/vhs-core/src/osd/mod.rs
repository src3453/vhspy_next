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

use crate::decoder::find_sync_edge;
use crate::frame::{Field, RgbFrame};
use crate::params::OsdParams;
use crate::timing::*;
use face::{Face, Mask};
use markup::{Context, Glyph, Rgba};
use std::collections::HashMap;
use std::sync::Arc;

const W: usize = ACTIVE_WIDTH;
const H: usize = FRAME_LINES;
const IQ_ROT: f32 = 33.0 * std::f32::consts::PI / 180.0;

/// OSD のオーバーレイ (乗算済みアルファの RGBA, 0..1) とその状態
pub struct Osd {
    overlay: Vec<[f32; 4]>,
    /// 前回描画した表示リスト (変化がなければ描き直さない)
    last: Vec<Glyph>,
    /// 描画済み範囲 (行) — 空なら None
    rows: Option<(usize, usize)>,
    face: Face,
    /// 読み込み済みフォントの設定 (パス, 番号, ドット数)
    face_key: Option<(String, u32, u32)>,
    face_error: Option<String>,
    /// (文字, サイズ) → (文字マスク, 縁取りマスク)
    cache: HashMap<(char, u32), Arc<(Mask, Mask)>>,
}

impl Default for Osd {
    fn default() -> Self {
        Self {
            overlay: vec![[0.0; 4]; W * H],
            last: Vec::new(),
            rows: None,
            face: Face::Builtin,
            face_key: None,
            face_error: None,
            cache: HashMap::new(),
        }
    }
}

impl Osd {
    /// 表示内容を更新する。戻り値は何か描画すべきものがあるか。
    pub fn update(&mut self, p: &OsdParams, field_index: u64) -> bool {
        let font_changed = self.update_face(p);
        let ctx = Context::new(&p.start_datetime, field_index);
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
        let path = p.font.trim();
        let key = (path.to_string(), p.font_index, p.font_dots.min(64));
        if self.face_key.as_ref() == Some(&key) {
            return false;
        }
        (self.face, self.face_error) = if path.is_empty() {
            (Face::Builtin, None)
        } else {
            match face::load(path, key.1, key.2) {
                Ok(f) => (f, None),
                Err(e) => (Face::Builtin, Some(e)),
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
        if let Some((y0, y1)) = self.rows.take() {
            self.overlay[y0 * W..y1 * W].fill([0.0; 4]);
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
        let ys = my.max(0)..(my + m.h as i32).min(H as i32);
        let xs = mx.max(0)..(mx + m.w as i32).min(W as i32);
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
                let o = &mut self.overlay[py as usize * W + px as usize];
                let src = [c.r * a, c.g * a, c.b * a, a];
                for k in 0..4 {
                    o[k] = src[k] + o[k] * (1.0 - a);
                }
            }
        }
    }

    fn fill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba, b: &mut Bounds) {
        let (x0, x1) = (x0.clamp(0, W as i32) as usize, x1.clamp(0, W as i32) as usize);
        let (y0, y1) = (y0.clamp(0, H as i32) as usize, y1.clamp(0, H as i32) as usize);
        if x0 >= x1 || y0 >= y1 || c.a <= 0.0 {
            return;
        }
        b.add(y0, y1);
        let a = c.a.min(1.0);
        let src = [c.r * a, c.g * a, c.b * a, a];
        for y in y0..y1 {
            for px in &mut self.overlay[y * W + x0..y * W + x1] {
                for k in 0..4 {
                    px[k] = src[k] + px[k] * (1.0 - a);
                }
            }
        }
    }

    /// エンコード前の画像に焼き込む
    pub fn burn_into(&self, frame: &mut RgbFrame, opacity: f32) {
        let Some((y0, y1)) = self.rows else { return };
        debug_assert_eq!((frame.width, frame.height), (W, H));
        for y in y0..y1 {
            let src = &self.overlay[y * W..(y + 1) * W];
            let dst = &mut frame.data[y * W * 3..(y + 1) * W * 3];
            for (o, px) in src.iter().zip(dst.chunks_exact_mut(3)) {
                let a = o[3] * opacity;
                if a <= 0.0 {
                    continue;
                }
                for k in 0..3 {
                    let v = px[k] as f32 * (1.0 - a) + o[k] * opacity * 255.0;
                    px[k] = (v + 0.5).clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    /// 再生後のコンポジット信号に差し込む。各ラインの同期位置に追従する。
    pub fn insert_composite(&self, field: &mut Field, opacity: f32) {
        let Some((y0, y1)) = self.rows else { return };
        for l in 0..FIELD_LINES {
            let row = 2 * l + field.parity as usize;
            if row < y0 || row >= y1 {
                continue;
            }
            let line = field.line_mut(l);
            // キャラクタジェネレータは再生信号の H 同期でタイミングを取る
            let d = find_sync_edge(line)
                .map(|e| e - SYNC_END as f32)
                .filter(|d| d.abs() < 40.0)
                .unwrap_or(0.0);
            let psi = burst_phase(line, d);
            let src = &self.overlay[row * W..(row + 1) * W];
            for x in 0..W {
                // 文字発生器の立ち上がり (≒100 ns) を 3 タップで近似
                let at = |i: isize| src[i.clamp(0, W as isize - 1) as usize];
                let (a, b, c) = (at(x as isize - 1), at(x as isize), at(x as isize + 1));
                let mut o = [0f32; 4];
                for k in 0..4 {
                    o[k] = (0.25 * a[k] + 0.5 * b[k] + 0.25 * c[k]) * opacity;
                }
                if o[3] <= 1e-4 {
                    continue;
                }
                let pos = ACTIVE_START as f32 + x as f32 + d;
                let m = pos.round() as isize;
                if m < 0 || m >= LINE_SAMPLES as isize {
                    continue;
                }
                let (r, g, bb) = (o[0], o[1], o[2]);
                let y = ire::BLACK * o[3] + ire::SPAN * (0.299 * r + 0.587 * g + 0.114 * bb);
                let i = ire::SPAN * (0.5959 * r - 0.2746 * g - 0.3213 * bb);
                let q = ire::SPAN * (0.2115 * r - 0.5227 * g + 0.3112 * bb);
                // 色副搬送波は再生バーストに位相同期させる (デッキ内の APC 出力を使う想定)
                let theta = m as f32 * std::f32::consts::FRAC_PI_2 + psi + IQ_ROT;
                let s = y + i * theta.cos() + q * theta.sin();
                let v = &mut line[m as usize];
                *v = *v * (1.0 - o[3]) + s;
            }
        }
    }
}

/// ラインのバーストから副搬送波位相 φ0 を求める (バースト = -sin(nπ/2 + φ0))。
/// テープの輝度経路の遅延で同期位置とバースト位相の関係は崩れるので、同期からは求めない。
fn burst_phase(line: &[f32], d: f32) -> f32 {
    let start = (BURST_START as f32 + d).round().max(0.0) as usize & !3;
    let end = ((BURST_END + 16) as f32 + d).round().clamp(0.0, ACTIVE_START as f32) as usize;
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
