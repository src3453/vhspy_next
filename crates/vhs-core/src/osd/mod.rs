//! OSD (オンスクリーン表示) のシミュレーション。
//!
//! 文字はマークアップで記述する (書式は [`markup`] を参照)。合成先は 2 通り:
//!
//! - [`OsdMode::Tape`](crate::params::OsdMode::Tape): エンコード前の RGB 画像に焼き込む。カメラの日付表示のように
//!   テープに記録された文字を再現し、テープの劣化 (にじみ・ノイズ・ジッタ) を一緒に受ける。
//! - [`OsdMode::Composite`](crate::params::OsdMode::Composite): テープ再生後のコンポジット信号に差し込む。デッキの
//!   キャラクタジェネレータが出力に重畳する表示を再現する。文字はくっきりしているが、
//!   再生信号の水平同期に追従するので時間軸の揺れは画面と一緒に受ける。

mod font;
pub mod markup;

use crate::decoder::find_sync_edge;
use crate::frame::{Field, RgbFrame};
use crate::params::OsdParams;
use crate::timing::*;
use font::{GLYPH_H, GLYPH_W, glyph};
use markup::{Context, Glyph, Rgba};

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
}

impl Default for Osd {
    fn default() -> Self {
        Self { overlay: vec![[0.0; 4]; W * H], last: Vec::new(), rows: None }
    }
}

impl Osd {
    /// 表示内容を更新する。戻り値は何か描画すべきものがあるか。
    pub fn update(&mut self, p: &OsdParams, field_index: u64) -> bool {
        let ctx = Context::new(&p.start_datetime, field_index);
        let glyphs: Vec<Glyph> = markup::layout(&p.markup, p.scale.clamp(1, 8), &ctx)
            .into_iter()
            .filter(|g| !g.style.blink || ctx.blink_on())
            .collect();
        if glyphs != self.last || (self.rows.is_none() && !glyphs.is_empty()) {
            self.render(&glyphs);
            self.last = glyphs;
        }
        self.rows.is_some()
    }

    fn render(&mut self, glyphs: &[Glyph]) {
        if let Some((y0, y1)) = self.rows.take() {
            self.overlay[y0 * W..y1 * W].fill([0.0; 4]);
        }
        let mut bounds = Bounds::default();
        // 背景 → 影 → 縁取り → 文字 の順に重ねる
        for g in glyphs {
            if let Some(bg) = g.style.bg {
                let (dw, dh) = dot_size(g.style.size);
                let x0 = g.x - dw / 2;
                let y0 = g.y - dh;
                self.fill(x0, y0, x0 + dw * (GLYPH_W as i32 + 1), y0 + dh * (GLYPH_H as i32 + 2), bg, &mut bounds);
            }
        }
        for g in glyphs {
            if let Some(c) = g.style.shadow {
                let (dw, dh) = dot_size(g.style.size);
                self.dots(g, dw, dh, 0, c, &mut bounds);
            }
        }
        for g in glyphs {
            if let Some(c) = g.style.edge {
                self.dots(g, 0, 0, 1, c, &mut bounds);
            }
        }
        for g in glyphs {
            self.dots(g, 0, 0, 0, g.style.color, &mut bounds);
        }
        self.rows = bounds.rows();
    }

    /// グリフのドットを描く。`grow` > 0 で各ドットを膨らませる (縁取り)。
    fn dots(&mut self, g: &Glyph, ox: i32, oy: i32, grow: i32, c: Rgba, b: &mut Bounds) {
        let (dw, dh) = dot_size(g.style.size);
        // 縁取りは横 2 サンプル、縦 2 ライン (= 1 フィールドライン) 単位で太らせる
        let (gx, gy) = (grow * 2, grow * 2);
        let bits = glyph(g.ch);
        for (r, row) in bits.iter().enumerate() {
            for col in 0..GLYPH_W {
                if row & (1 << (GLYPH_W - 1 - col)) == 0 {
                    continue;
                }
                let x0 = g.x + ox + col as i32 * dw;
                let y0 = g.y + oy + r as i32 * dh;
                self.fill(x0 - gx, y0 - gy, x0 + dw + gx, y0 + dh + gy, c, b);
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

/// サイズ 1 あたりのドット: 横 2 サンプル × 縦 2 ライン (1 フィールドライン)
fn dot_size(size: u32) -> (i32, i32) {
    (2 * size as i32, 2 * size as i32)
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
