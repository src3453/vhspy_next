//! 8 単位符号 (告示 別表第十八〜二十号): 本文・ヘッダ文のデータユニットの中身。
//!
//! - 送出側 [`encode`]: 簡易マークアップの文 → 符号列
//! - 受信側 [`Decoder`]: 符号列 → [`Screen`] (ユニットスクリーン上の表示区画の並び)
//!
//! 初期状態 (別表第三十号): G0 = 漢字系、G1 = 英数、G2 = 平仮名、G3 = マクロ、GL = G0 (LS0)、GR = G2 (LS2R)。
//! 前景色は白 (WHF)、背景色は透明 (COL 05/8)、文字の大きさは本文が標準、ヘッダ文が小形。
//!
//! 対応しているもの: 漢字系・英数・平仮名・片仮名集合、モザイク (2 × 3 の近似)、C0 の動作位置制御・CS・CAN・PAPF・APS、
//! C1 の前景色・COL・SSZ/MSZ/NSZ/SZX・FLC・POL・RPC。CDC・WMM・HLC・MACRO・TIME・CSI などはパラメタを読み飛ばす。
//! DRCS・DJCS (外字) とマクロの展開は未対応 (外字は表示しない)。

use encoding_rs::EUC_JP;

// ---------------------------------------------------------------- 符号値

pub const NUL: u8 = 0x00;
pub const APB: u8 = 0x08;
pub const APF: u8 = 0x09;
pub const APD: u8 = 0x0A;
pub const APU: u8 = 0x0B;
pub const CS: u8 = 0x0C;
pub const APR: u8 = 0x0D;
pub const LS1: u8 = 0x0E;
pub const LS0: u8 = 0x0F;
pub const PAPF: u8 = 0x16;
pub const CAN: u8 = 0x18;
pub const SS2: u8 = 0x19;
pub const ESC: u8 = 0x1B;
pub const APS: u8 = 0x1C;
pub const SS3: u8 = 0x1D;
pub const SP: u8 = 0x20;
pub const DEL: u8 = 0x7F;
/// 前景色 黒〜白 (BKF〜WHF) は 0x80〜0x87
pub const BKF: u8 = 0x80;
pub const SSZ: u8 = 0x88;
pub const MSZ: u8 = 0x89;
pub const NSZ: u8 = 0x8A;
pub const SZX: u8 = 0x8B;
pub const MCS: u8 = 0x8C;
pub const MCT: u8 = 0x8D;
pub const COL: u8 = 0x90;
pub const FLC: u8 = 0x91;
pub const CDC: u8 = 0x92;
pub const POL: u8 = 0x93;
pub const WMM: u8 = 0x94;
pub const MACRO: u8 = 0x95;
pub const HLC: u8 = 0x97;
pub const RPC: u8 = 0x98;
pub const SPL: u8 = 0x99;
pub const STL: u8 = 0x9A;
pub const CSI: u8 = 0x9B;
pub const TIME: u8 = 0x9D;

/// 色 (カラーマップ下位アドレス CMLA)。0 黒、1〜7 最大輝度の赤緑黄青マゼンタシアン白、8 透明、9〜15 半輝度
pub const TRANSPARENT: u8 = 8;
/// ラスタの既定色 (スーパーでないとき、半輝度の青)
pub const DEFAULT_RASTER: u8 = 12;

/// CMLA の RGB。透明 (8) は None
pub fn cmla_rgb(c: u8) -> Option<[u8; 3]> {
    let c = c & 15;
    if c == TRANSPARENT {
        return None;
    }
    let level = if c < 8 { 255 } else { 128 };
    // 1 赤 2 緑 3 黄 4 青 5 マゼンタ 6 シアン 7 白 (ビット 0 = 赤、1 = 緑、2 = 青)。0 は黒
    let bits = c & 7;
    let ch = |b: u8| if bits >> b & 1 == 1 { level } else { 0 };
    Some([ch(0), ch(1), ch(2)])
}

// ---------------------------------------------------------------- 表示区画の大きさ

/// 文字の大きさ (標準密度の横書、別表第十二号)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Normal,
    Medium,
    Small,
    Wide,
    Tall,
    Double,
    Special1,
    Special2,
    Tiny,
}

impl Size {
    /// (横, 縦) [ユニットスクリーンの 1/256]
    pub fn dims(self) -> (i32, i32) {
        match self {
            Size::Normal => (16, 24),
            Size::Medium => (8, 24),
            Size::Small => (8, 12),
            Size::Wide => (32, 24),
            Size::Tall => (16, 48),
            Size::Double => (32, 48),
            Size::Special1 => (16, 20),
            Size::Special2 => (20, 20),
            Size::Tiny => (4, 4),
        }
    }
}

/// ユニットスクリーン (標準密度) の幅と高さ。座標は左上原点・下向き
pub const SCREEN_W: i32 = 248;
pub const SCREEN_H: i32 = 204;
/// ヘッダ文表示領域の高さ (上端の 12)。本文表示領域はその下の 192
pub const HEADER_H: i32 = 12;

/// 表示領域
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Header,
    Body,
}

impl Area {
    /// (上端, 下端)
    fn span(self) -> (i32, i32) {
        match self {
            Area::Header => (0, HEADER_H),
            Area::Body => (HEADER_H, SCREEN_H),
        }
    }
}

// ---------------------------------------------------------------- 画面

/// 表示区画に描くもの
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    /// 空白 (背景色だけ)
    Space,
    /// 漢字系集合の区点 (0x21〜0x7E)。英数・平仮名・片仮名もここに写して描く
    Jis(u8, u8),
    /// モザイク (b1〜b6 を 2 × 3 の升に割り当てた近似)
    Mosaic(u8),
}

/// 1 つの表示区画
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// 左上 [1/256]
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub glyph: Glyph,
    pub fg: u8,
    pub bg: u8,
    /// フラッシング (None = なし、Some(false) = 正相、Some(true) = 逆相)
    pub flash: Option<bool>,
}

/// 1 ページの表示画面
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    pub cells: Vec<Cell>,
    /// スーパー (ラスタが透明でテレビ映像が見える)
    pub superimpose: bool,
    /// ヘッダラスタ色・ラスタ色 (CMLA)
    pub header_raster: u8,
    pub raster: u8,
}

impl Screen {
    pub fn new(superimpose: bool) -> Self {
        Screen { cells: Vec::new(), superimpose, header_raster: DEFAULT_RASTER, raster: DEFAULT_RASTER }
    }

    fn put(&mut self, c: Cell) {
        // 重なる区画は新しいもので上書きする
        self.cells.retain(|o| o.x + o.w <= c.x || c.x + c.w <= o.x || o.y + o.h <= c.y || c.y + c.h <= o.y);
        self.cells.push(c);
    }

    fn clear(&mut self, area: Area) {
        let (top, bottom) = area.span();
        self.cells.retain(|c| c.y + c.h <= top || c.y >= bottom);
    }

    /// 表示される文字を行ごとに並べた文字列 (テスト・GUI 表示用)
    pub fn text(&self) -> String {
        let mut cells: Vec<&Cell> = self.cells.iter().filter(|c| c.glyph != Glyph::Space).collect();
        cells.sort_by_key(|c| (c.y + c.h, c.x));
        let mut out = String::new();
        let mut last_base = None;
        for c in cells {
            let base = c.y + c.h;
            if last_base.is_some_and(|b| b != base) {
                out.push('\n');
            }
            last_base = Some(base);
            out.push(glyph_char(c.glyph));
        }
        out
    }
}

/// 区点 → Unicode (EUC-JP を通す)。モザイクは ▚
pub fn glyph_char(g: Glyph) -> char {
    match g {
        Glyph::Space => ' ',
        Glyph::Mosaic(_) => '▚',
        Glyph::Jis(r, c) => {
            let bytes = [r | 0x80, c | 0x80];
            let (s, _, bad) = EUC_JP.decode(&bytes);
            if bad { '〓' } else { s.chars().next().unwrap_or('〓') }
        }
    }
}

/// Unicode → 区点 (JIS X 0208 にない字は None)
pub fn jis_of(ch: char) -> Option<(u8, u8)> {
    let mut buf = [0u8; 4];
    let (b, _, bad) = EUC_JP.encode(ch.encode_utf8(&mut buf));
    match *b {
        [a, c] if !bad && a >= 0xA1 && c >= 0xA1 => Some((a & 0x7F, c & 0x7F)),
        _ => None,
    }
}

// ---------------------------------------------------------------- 符号集合

/// G0〜G3 に指示する符号集合
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Set {
    Kanji,
    Alnum,
    Hiragana,
    Katakana,
    Mosaic(u8),
    /// DRCS-0 (2 バイト)
    Drcs0,
    /// DRCS-1〜15 (1 バイト)
    Drcs,
    Macro,
    /// 未知の終端符号
    Unknown { two_bytes: bool },
}

impl Set {
    fn from_final(f: u8, two_bytes: bool, drcs: bool) -> Set {
        match (two_bytes, drcs, f) {
            (true, false, 0x42) => Set::Kanji,
            (true, true, 0x40) => Set::Drcs0,
            (false, true, 0x41..=0x4F) => Set::Drcs,
            (false, false, 0x4A) => Set::Alnum,
            (false, false, 0x30) => Set::Hiragana,
            (false, false, 0x31) => Set::Katakana,
            (false, false, 0x32..=0x35) => Set::Mosaic(f - 0x32),
            (false, false, 0x70) => Set::Macro,
            _ => Set::Unknown { two_bytes },
        }
    }

    fn two_bytes(self) -> bool {
        matches!(self, Set::Kanji | Set::Drcs0 | Set::Unknown { two_bytes: true })
    }
}

/// 平仮名・片仮名集合の 07/7〜07/14 (ゝゞ / ヽヾ、ー。「」、・) の区点
fn kana_tail(katakana: bool, b: u8) -> Option<(u8, u8)> {
    Some(match b {
        0x77 => (0x21, if katakana { 0x33 } else { 0x35 }),
        0x78 => (0x21, if katakana { 0x34 } else { 0x36 }),
        0x79 => (0x21, 0x3C),
        0x7A => (0x21, 0x23),
        0x7B => (0x21, 0x56),
        0x7C => (0x21, 0x57),
        0x7D => (0x21, 0x22),
        0x7E => (0x21, 0x26),
        _ => return None,
    })
}

/// 英数集合の 1 文字 (別記第2) の区点。全角形で描く
fn alnum_jis(b: u8) -> Option<(u8, u8)> {
    let ch = match b {
        0x5C => '￥',
        0x7E => '￣',
        0x22 => '”',
        0x27 => '’',
        0x2D => '－',
        _ => char::from_u32(0xFF01 + (b as u32 - 0x21))?,
    };
    jis_of(ch)
}

// ---------------------------------------------------------------- 送出側

/// 簡易マークアップの文を本文・ヘッダ文のデータユニットデータにする。
///
/// - 改行 → APR (動作位置改行)
/// - `{red}` `{green}` `{yellow}` `{blue}` `{magenta}` `{cyan}` `{white}` `{black}`: 前景色。
///   `{dim-red}` などで半輝度
/// - `{bg:blue}` などで背景色、`{bg:none}` で透明
/// - `{small}` `{medium}` `{normal}` `{wide}` `{tall}` `{double}`: 文字の大きさ
/// - `{flash}` `{/flash}`: フラッシング、`{reverse}` `{/reverse}`: 反転 (POL)
/// - `{cls}`: 画面消去
/// - そのほかの `{...}` はそのまま文字として送る
///
/// 平仮名は GR の平仮名集合 (1 バイト)、ASCII の英数字・記号は英数集合 (LS1 で GL に呼ぶ)、
/// それ以外は漢字系集合 (2 バイト)。JIS X 0208 にない字は〓 (2 区 14 点) にする
pub fn encode(src: &str) -> Vec<u8> {
    let mut out = Vec::new();
    // GL に呼び出している集合 (true = G1 英数)
    let mut gl_alnum = false;
    let mut rest = src;
    while let Some(ch) = rest.chars().next() {
        if ch == '{'
            && let Some(end) = rest.find('}')
            && let Some(code) = tag(&rest[1..end])
        {
            out.extend_from_slice(&code);
            rest = &rest[end + 1..];
            continue;
        }
        rest = &rest[ch.len_utf8()..];
        match ch {
            '\r' => {}
            '\n' => out.push(APR),
            ' ' => out.push(SP),
            '!'..='~' => {
                if !gl_alnum {
                    out.push(LS1);
                    gl_alnum = true;
                }
                out.push(ch as u8);
            }
            _ => {
                if let Some(b) = hiragana_byte(ch) {
                    out.push(b | 0x80);
                    continue;
                }
                let (r, c) = jis_of(ch).unwrap_or((0x22, 0x2E));
                if gl_alnum {
                    out.push(LS0);
                    gl_alnum = false;
                }
                out.extend_from_slice(&[r, c]);
            }
        }
    }
    out
}

/// 平仮名集合 (別記第3) にある字の符号 (GL 側の値)
fn hiragana_byte(ch: char) -> Option<u8> {
    let (r, c) = jis_of(ch)?;
    if r == 0x24 && c <= 0x73 {
        return Some(c);
    }
    (0x77..=0x7E).find(|&b| kana_tail(false, b) == Some((r, c)))
}

fn tag(name: &str) -> Option<Vec<u8>> {
    const COLORS: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
    let color = |s: &str| -> Option<u8> {
        if let Some(d) = s.strip_prefix("dim-") {
            return COLORS.iter().position(|c| *c == d).filter(|&i| i > 0).map(|i| 8 + i as u8);
        }
        if s == "none" || s == "transparent" {
            return Some(TRANSPARENT);
        }
        COLORS.iter().position(|c| *c == s).map(|i| i as u8)
    };
    Some(match name {
        "small" => vec![SSZ],
        "medium" => vec![MSZ],
        "normal" => vec![NSZ],
        "wide" => vec![SZX, 0x44],
        "tall" => vec![SZX, 0x41],
        "double" => vec![SZX, 0x45],
        "flash" => vec![FLC, 0x40],
        "/flash" => vec![FLC, 0x4F],
        "reverse" => vec![POL, 0x41],
        "/reverse" => vec![POL, 0x40],
        "cls" => vec![CS],
        _ => {
            if let Some(bg) = name.strip_prefix("bg:") {
                vec![COL, 0x50 | color(bg)?]
            } else {
                let c = color(name)?;
                if c < 8 { vec![BKF + c] } else { vec![COL, 0x40 | c] }
            }
        }
    })
}

// ---------------------------------------------------------------- 受信側

/// 本文・ヘッダ文のデータユニット 1 つを画面に描く復号器
pub struct Decoder<'a> {
    screen: &'a mut Screen,
    area: Area,
    g: [Set; 4],
    gl: usize,
    gr: usize,
    size: Size,
    fg: u8,
    bg: u8,
    flash: Option<bool>,
    reverse: bool,
    /// 動作位置 (表示区画の基準点 = 左下) [1/256]
    x: i32,
    y: i32,
}

impl<'a> Decoder<'a> {
    /// データユニットごとに文字の符号化に関して初期化する (別表第三十号の (3))
    pub fn new(screen: &'a mut Screen, area: Area) -> Self {
        let size = if area == Area::Header { Size::Small } else { Size::Normal };
        let (top, _) = area.span();
        Decoder {
            screen,
            area,
            g: [Set::Kanji, Set::Alnum, Set::Hiragana, Set::Macro],
            gl: 0,
            gr: 2,
            size,
            fg: 7,
            bg: TRANSPARENT,
            flash: None,
            reverse: false,
            x: 0,
            y: top + size.dims().1,
        }
    }

    fn dims(&self) -> (i32, i32) {
        self.size.dims()
    }

    fn apd(&mut self) {
        let (top, bottom) = self.area.span();
        let h = self.dims().1;
        self.y += h;
        if self.y > bottom {
            self.y = top + h;
        }
    }

    fn apu(&mut self) {
        let (top, bottom) = self.area.span();
        let h = self.dims().1;
        self.y -= h;
        if self.y - h < top {
            self.y = bottom - (bottom - top) % h;
        }
    }

    fn apf(&mut self) {
        let w = self.dims().0;
        self.x += w;
        if self.x + w > SCREEN_W {
            self.x = 0;
            self.apd();
        }
    }

    fn apb(&mut self) {
        let w = self.dims().0;
        self.x -= w;
        if self.x < 0 {
            self.x = (SCREEN_W / w - 1) * w;
            self.apu();
        }
    }

    fn apr(&mut self) {
        self.x = 0;
        self.apd();
    }

    fn draw(&mut self, glyph: Glyph) {
        let (w, h) = self.dims();
        if self.x + w > SCREEN_W {
            self.apr();
        }
        // 最初の行で縦倍・2 倍の文字を出すと表示区画が領域の上にはみ出すので、領域の中まで下げる
        let top = self.area.span().0;
        if self.y - h < top {
            self.y = top + h;
        }
        let (fg, bg) = if self.reverse { (self.bg, self.fg) } else { (self.fg, self.bg) };
        self.screen.put(Cell { x: self.x, y: self.y - h, w, h, glyph, fg, bg, flash: self.flash });
        self.apf();
    }

    /// 1 バイト (0x21〜0x7E) またはその後続から表示する字形を決める
    fn glyph_in(&self, set: Set, b: u8, b2: Option<u8>) -> Option<Glyph> {
        Some(match set {
            Set::Kanji => Glyph::Jis(b, b2?),
            Set::Alnum => alnum_jis(b).map_or(Glyph::Space, |(r, c)| Glyph::Jis(r, c)),
            Set::Hiragana | Set::Katakana => {
                let kata = set == Set::Katakana;
                match kana_tail(kata, b) {
                    Some((r, c)) => Glyph::Jis(r, c),
                    None if b <= 0x76 => Glyph::Jis(if kata { 0x25 } else { 0x24 }, b),
                    None => Glyph::Space,
                }
            }
            Set::Mosaic(_) => Glyph::Mosaic(b & 0x3F),
            // 外字は未定義として空白にする
            Set::Drcs0 | Set::Drcs | Set::Unknown { .. } => Glyph::Space,
            // マクロは展開しない
            Set::Macro => return None,
        })
    }

    /// データユニットデータ全体を復号する
    pub fn run(mut self, data: &[u8]) {
        let mut i = 0;
        let mut single: Option<usize> = None;
        let mut repeat: Option<i32> = None;
        let next = |i: &mut usize| -> Option<u8> {
            let b = data.get(*i).copied();
            *i += 1;
            b
        };
        while let Some(b) = next(&mut i) {
            match b {
                NUL => {}
                APB => self.apb(),
                APF => self.apf(),
                APD => self.apd(),
                APU => self.apu(),
                APR => self.apr(),
                CS => {
                    self.screen.clear(self.area);
                    let fresh = Decoder::new(self.screen, self.area);
                    let (g, gl, gr, size, x, y) = (fresh.g, fresh.gl, fresh.gr, fresh.size, fresh.x, fresh.y);
                    (self.g, self.gl, self.gr, self.size, self.x, self.y) = (g, gl, gr, size, x, y);
                    (self.fg, self.bg, self.flash, self.reverse) = (7, TRANSPARENT, None, false);
                }
                LS0 => self.gl = 0,
                LS1 => self.gl = 1,
                SS2 => single = Some(2),
                SS3 => single = Some(3),
                PAPF => {
                    let n = next(&mut i).unwrap_or(0x40) & 0x3F;
                    for _ in 0..n {
                        self.apf();
                    }
                }
                APS => {
                    let r = (next(&mut i).unwrap_or(0x40) & 0x3F) as i32;
                    let c = (next(&mut i).unwrap_or(0x40) & 0x3F) as i32;
                    let (w, h) = self.dims();
                    self.x = c * w;
                    self.y = self.area.span().0 + (r + 1) * h;
                }
                CAN => {
                    let (w, h) = self.dims();
                    let width = SCREEN_W - self.x;
                    if width > 0 {
                        let c = Cell { x: self.x, y: self.y - h, w: width, h, glyph: Glyph::Space, fg: self.fg, bg: self.bg, flash: None };
                        self.screen.put(c);
                    }
                    let _ = w;
                }
                ESC => self.escape(data, &mut i),
                0x01..=0x1F => {}
                SP | DEL => {
                    let n = repeat.take().unwrap_or(1);
                    for _ in 0..n {
                        self.draw(Glyph::Space);
                    }
                }
                0x21..=0x7E | 0xA1..=0xFE => {
                    let idx = single.take().unwrap_or(if b < 0x80 { self.gl } else { self.gr });
                    let set = self.g[idx];
                    let b1 = b & 0x7F;
                    let b2 = if set.two_bytes() { next(&mut i).map(|c| c & 0x7F) } else { None };
                    if let Some(g) = self.glyph_in(set, b1, b2) {
                        let n = repeat.take().unwrap_or(1);
                        for _ in 0..n {
                            self.draw(g);
                        }
                    }
                }
                0x80..=0x87 => self.fg = b - BKF,
                SSZ => self.size = Size::Small,
                MSZ => self.size = Size::Medium,
                NSZ => self.size = Size::Normal,
                SZX => {
                    self.size = match next(&mut i) {
                        Some(0x60) => Size::Tiny,
                        Some(0x41) => Size::Tall,
                        Some(0x44) => Size::Wide,
                        Some(0x45) => Size::Double,
                        Some(0x6B) => Size::Special1,
                        Some(0x64) => Size::Special2,
                        _ => self.size,
                    }
                }
                COL => match next(&mut i) {
                    Some(0x20) => {
                        next(&mut i);
                    }
                    Some(n @ 0x48..=0x4F) => self.fg = n & 15,
                    Some(n @ 0x50..=0x5F) => self.bg = n & 15,
                    _ => {}
                },
                FLC => {
                    self.flash = match next(&mut i) {
                        Some(0x40) => Some(false),
                        Some(0x47) => Some(true),
                        _ => None,
                    }
                }
                POL => self.reverse = matches!(next(&mut i), Some(0x41 | 0x42)),
                RPC => {
                    let n = (next(&mut i).unwrap_or(0x41) & 0x3F) as i32;
                    // 0 は行末まで
                    repeat = Some(if n == 0 { ((SCREEN_W - self.x) / self.dims().0).max(1) } else { n });
                }
                CDC | TIME => {
                    if next(&mut i).is_some_and(|n| (0x20..=0x2F).contains(&n)) {
                        next(&mut i);
                    }
                }
                WMM | HLC => {
                    next(&mut i);
                }
                MACRO => {
                    // マクロ定義 (04/0 または 04/1 ... MACRO 04/15) は読み飛ばす
                    if matches!(next(&mut i), Some(0x40 | 0x41)) {
                        while i + 1 < data.len() && !(data[i] == MACRO && data[i + 1] == 0x4F) {
                            i += 1;
                        }
                        i += 2;
                    }
                }
                CSI => {
                    while let Some(c) = next(&mut i) {
                        if (0x40..=0x7E).contains(&c) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// ESC で始まる指示・呼出し
    fn escape(&mut self, data: &[u8], i: &mut usize) {
        let mut take = || {
            let b = data.get(*i).copied();
            *i += 1;
            b
        };
        let Some(b) = take() else { return };
        match b {
            0x6E => self.gl = 2,
            0x6F => self.gl = 3,
            0x7E => self.gr = 1,
            0x7D => self.gr = 2,
            0x7C => self.gr = 3,
            0x28..=0x2B => {
                let g = (b - 0x28) as usize;
                let mut f = take().unwrap_or(0);
                let drcs = f == 0x20;
                if drcs {
                    f = take().unwrap_or(0);
                }
                self.g[g] = Set::from_final(f, false, drcs);
            }
            0x24 => {
                let mut f = take().unwrap_or(0);
                let mut g = 0;
                if (0x29..=0x2B).contains(&f) {
                    g = (f - 0x28) as usize;
                    f = take().unwrap_or(0);
                }
                let drcs = f == 0x20;
                if drcs {
                    f = take().unwrap_or(0);
                }
                self.g[g] = Set::from_final(f, true, drcs);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(src: &str) -> Screen {
        let mut s = Screen::new(false);
        Decoder::new(&mut s, Area::Body).run(&encode(src));
        s
    }

    #[test]
    fn roundtrip_text() {
        let s = decode("文字放送のテスト\nABC 123");
        assert_eq!(s.text(), "文字放送のテスト\nＡＢＣ １２３".replace(' ', ""));
        // 平仮名は GR の 1 バイト、英数は LS1 で GL に呼ぶ
        let e = encode("あA漢");
        assert_eq!(e, vec![0x22 | 0x80, LS1, b'A', LS0, 0x34, 0x41]);
    }

    #[test]
    fn layout_and_wrap() {
        // 標準の大きさで 1 行 15 字。16 字目は次の行の頭
        let s = decode(&"あ".repeat(16));
        let last = s.cells.last().unwrap();
        assert_eq!((last.x, last.y, last.w, last.h), (0, HEADER_H + 24, 16, 24));
        assert_eq!(s.cells[14].x, 14 * 16);
    }

    #[test]
    fn colors_and_sizes() {
        let s = decode("{red}{bg:blue}あ{double}い{dim-cyan}う");
        assert_eq!((s.cells[0].fg, s.cells[0].bg), (1, 4));
        assert_eq!((s.cells[1].w, s.cells[1].h), (32, 48));
        // 最初の行の 2 倍の文字は本文表示領域の上にはみ出さない
        assert_eq!(s.cells[1].y, HEADER_H);
        assert_eq!(s.cells[2].fg, 14);
        assert_eq!(cmla_rgb(12), Some([0, 0, 128]));
        assert_eq!(cmla_rgb(8), None);
    }

    #[test]
    fn katakana_set_and_escape() {
        // G1 に片仮名集合を指示して LS1 で呼ぶ
        let mut s = Screen::new(false);
        Decoder::new(&mut s, Area::Body).run(&[ESC, 0x29, 0x31, LS1, 0x22, 0x79, LS0, 0x30, 0x21]);
        assert_eq!(s.text(), "アー亜");
    }
}
