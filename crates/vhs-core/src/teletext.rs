//! 受像機の文字放送 (JTES) 受信機: VBI のデータラインを読んで、選んだ番組・ページを画面に重ねる。
//!
//! パケットの組み立て・番組選択・8 単位符号の解釈は [`jtes::program::Receiver`]。ここでは
//! ユニットスクリーン (248 × 204、1/256 単位) を画面の中央に写し、16 ドットの漢字 ROM (jiskan16) で文字を描く。

use jtes::code::{self, Cell, Glyph, Screen};

use crate::frame::RgbaFrame;

/// ユニットスクリーンが占める割合 (縦は画面の高さ、横は 4:3 の画面幅に対して)。
/// 248:204 の画面を、文字の縦横比が崩れないよう 4:3 の画面の内側に置く
const HEIGHT_FRACTION: f32 = 0.86;
const WIDTH_FRACTION: f32 = HEIGHT_FRACTION * (code::SCREEN_W as f32 / code::SCREEN_H as f32) / (4.0 / 3.0);

#[derive(Default)]
pub struct TeletextDecoder {
    pub receiver: jtes::program::Receiver,
    /// 読んだ直前の帰線 (同じフィールドを 2 回読まない)
    pub(crate) last: Option<i64>,
    /// 表示する画面 (受信機の版が変わったときだけ組み立て直す)
    screen: Option<(u64, Screen)>,
}

impl TeletextDecoder {
    pub fn reset(&mut self) {
        self.receiver.reset();
        self.last = None;
        self.screen = None;
    }

    /// 1 フィールドぶんのパケット (ラインの順) を入れる
    pub fn feed(&mut self, packets: impl IntoIterator<Item = (jtes::Packet, usize)>) {
        for (p, n) in packets {
            self.receiver.feed(&p, n);
        }
        let v = self.receiver.version();
        if self.screen.as_ref().is_none_or(|(k, _)| *k != v) {
            let s = self.receiver.screen().unwrap_or_else(|| self.receiver.waiting_screen());
            self.screen = Some((v, s));
        }
    }

    pub fn screen(&self) -> Option<&Screen> {
        self.screen.as_ref().map(|(_, s)| s)
    }

    /// 画面に描く。`field_index` はフラッシング (1 秒周期、明滅 1:1) の位相に使う
    pub fn render(&self, frame: &mut RgbaFrame, field_index: u64) {
        if let Some(s) = self.screen() {
            render_screen(s, frame, field_index);
        }
    }
}

/// ユニットスクリーン座標 → 画素
struct Map {
    x0: f32,
    y0: f32,
    sx: f32,
    sy: f32,
}

impl Map {
    fn new(frame: &RgbaFrame) -> Self {
        let (w, h) = (frame.width as f32, frame.height as f32);
        let sx = w * WIDTH_FRACTION / code::SCREEN_W as f32;
        let sy = h * HEIGHT_FRACTION / code::SCREEN_H as f32;
        Map { x0: (w - sx * code::SCREEN_W as f32) / 2.0, y0: (h - sy * code::SCREEN_H as f32) / 2.0, sx, sy }
    }

    /// 矩形 (1/256 単位、小数可) → 画素の範囲 [x0, x1) × [y0, y1)
    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> (usize, usize, usize, usize) {
        let px = |u: f32| (self.x0 + u * self.sx).round().max(0.0) as usize;
        let py = |u: f32| (self.y0 + u * self.sy).round().max(0.0) as usize;
        (px(x), py(y), px(x + w), py(y + h))
    }
}

fn fill(frame: &mut RgbaFrame, (x0, y0, x1, y1): (usize, usize, usize, usize), rgb: [u8; 3]) {
    let (fw, fh) = (frame.width, frame.height);
    for y in y0..y1.min(fh) {
        let row = &mut frame.data[y * fw * 4..(y + 1) * fw * 4];
        for x in x0..x1.min(fw) {
            row[x * 4..x * 4 + 3].copy_from_slice(&rgb);
        }
    }
}

fn render_screen(s: &Screen, frame: &mut RgbaFrame, field_index: u64) {
    let m = Map::new(frame);
    // ラスタ (スーパーでなければ画面全体を塗る)
    if !s.superimpose {
        let w = code::SCREEN_W as f32;
        if let Some(c) = code::cmla_rgb(s.header_raster) {
            fill(frame, m.rect(0.0, 0.0, w, code::HEADER_H as f32), c);
        }
        if let Some(c) = code::cmla_rgb(s.raster) {
            fill(frame, m.rect(0.0, code::HEADER_H as f32, w, (code::SCREEN_H - code::HEADER_H) as f32), c);
        }
    }
    // フラッシングは 1 秒周期で明 (前半) と滅 (後半)
    let dark = (field_index / 30) % 2 == 1;
    for c in &s.cells {
        draw_cell(frame, &m, c, dark);
    }
}

fn draw_cell(frame: &mut RgbaFrame, m: &Map, c: &Cell, dark: bool) {
    let (x, y, w, h) = (c.x as f32, c.y as f32, c.w as f32, c.h as f32);
    if let Some(bg) = code::cmla_rgb(c.bg) {
        fill(frame, m.rect(x, y, w, h), bg);
    }
    // 滅: 文字を背景で隠す (逆相は明と滅が逆)
    if c.flash.is_some_and(|inverse| dark != inverse) {
        return;
    }
    let Some(fg) = code::cmla_rgb(c.fg) else { return };
    match c.glyph {
        Glyph::Space => {}
        Glyph::Jis(r, k) => {
            let Some(g) = jtes::font::glyph16(r, k) else { return };
            // 16 × 16 の字形を表示区画の横幅いっぱい、縦は区画の 2/3 (標準 24 のうち 16) に置き、上下に行間を残す
            let gh = h * 2.0 / 3.0;
            let gy = y + (h - gh) / 2.0;
            let (px0, py0, px1, py1) = m.rect(x, gy, w, gh);
            let (fw, fh) = (frame.width, frame.height);
            let (dw, dh) = ((px1 - px0).max(1) as f32, (py1 - py0).max(1) as f32);
            for py in py0..py1.min(fh) {
                let row = g[(((py - py0) as f32 + 0.5) * 16.0 / dh) as usize % 16];
                if row == 0 {
                    continue;
                }
                for px in px0..px1.min(fw) {
                    let col = (((px - px0) as f32 + 0.5) * 16.0 / dw) as usize % 16;
                    if row >> (15 - col) & 1 == 1 {
                        let o = (py * fw + px) * 4;
                        frame.data[o..o + 3].copy_from_slice(&fg);
                    }
                }
            }
        }
        Glyph::Mosaic(bits) => {
            // b1..b6 を左上から右下へ 2 × 3 の升に割り当てる (近似)
            for k in 0..6 {
                if bits >> k & 1 == 1 {
                    let (cx, cy) = ((k % 2) as f32, (k / 2) as f32);
                    fill(frame, m.rect(x + cx * w / 2.0, y + cy * h / 3.0, w / 2.0, h / 3.0), fg);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_page_onto_frame() {
        let mut rx = jtes::program::Receiver::default();
        let spec = jtes::program::ProgramSpec {
            magazine: 1,
            number: 100,
            superimpose: false,
            pages: vec![jtes::program::PageSpec { number: 1, header: "H".into(), body: "{red}亜".into() }],
        };
        for (k, p) in jtes::program::cycle(&spec, 60).into_iter().enumerate() {
            rx.feed(&jtes::Packet { continuity: (k % 16) as u8, ..p }, 0);
        }
        let mut t = TeletextDecoder { receiver: rx, ..Default::default() };
        t.feed([]);
        let mut f = RgbaFrame::new(640, 480);
        t.render(&mut f, 0);
        let px = |x: usize, y: usize| [f.data[(y * 640 + x) * 4], f.data[(y * 640 + x) * 4 + 1], f.data[(y * 640 + x) * 4 + 2]];
        // ラスタは半輝度の青、画面の外は元のまま
        assert_eq!(px(320, 400), [0, 0, 128]);
        assert_eq!(px(5, 5), [0, 0, 0]);
        // 本文 1 字目 (左上) のあたりに赤い点がある
        let m = Map::new(&f);
        let (x0, y0, x1, y1) = m.rect(0.0, 12.0, 16.0, 24.0);
        let red = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| px(x, y) == [255, 0, 0]).count();
        assert!(red > 20, "{red}");
    }
}
