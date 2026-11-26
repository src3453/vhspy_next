//! クローズドキャプション (EIA/CEA-608) の送出データ列・受像機のデコーダ・画面表示。
//!
//! ライン 21 (第 1 フィールド: CC1/CC2) とライン 284 (第 2 フィールド: CC3/CC4) で、1 フレームに 2 バイト
//! (7 ビット + 奇数パリティ) ずつ送られる。波形のスライスは [`crate::vbi`]。
//!
//! # デコーダ ([`CaptionDecoder`])
//! - 表示モード: ポップオン (非表示メモリに組み立てて EOC で入れ替え)、ロールアップ (2〜4 行、CR で
//!   1 行ずつ上へ)、ペイントオン (表示メモリに直接書く)。テキストモード (TR/RTD) は表示しない
//! - 前置アドレスコード (PAC): 行 (1〜15)・字下げ (4 桁単位)・色・下線・斜体。タブオフセット (TO1〜3)
//! - 行中コード: 色・斜体・下線 (1 桁ぶんの空白になる)。背景属性コード。フラッシュ (FON)
//! - 文字: 標準文字 (ASCII の一部を á é í ó ú ç ÷ Ñ ñ ■ に置き換えたもの)、特殊文字 (® ° ½ ¿ ™ ¢ £ ♪ …)、
//!   拡張文字 (スペイン語・フランス語・ポルトガル語・ドイツ語…。直前の代替文字を自動で消して置き換える)
//! - 制御コードは 2 回続けて送られるので、2 回目は無視する。パリティ誤りの制御コードは捨て、
//!   パリティ誤りの文字は■ (塗りつぶし) で出す。第 2 フィールドの XDS は読み飛ばす
//!
//! # 画面表示 ([`CaptionDecoder::render`])
//! 32 桁 × 15 行の字幕セル (画面の中央 80% ほど) に、黒い背景の箱の上へ 5×7 ドットの文字を描く。
//! 文字の並びの前後に 1 桁ずつ背景を延ばす (実機の字幕デコーダと同じ見え方)。

use crate::frame::RgbaFrame;
use crate::osd::font;
use serde::{Deserialize, Serialize};

pub const COLS: usize = 32;
pub const ROWS: usize = 15;

/// 字幕の送り方 (送出側)
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptionMode {
    /// ポップオン: 1 行ずつ画面下部に中央揃えで出す (映画・収録番組)
    #[default]
    PopOn,
    /// ロールアップ: 3 行の窓で下から流す (生放送)
    RollUp,
    /// ペイントオン: 届いた文字から順に描く
    PaintOn,
}

// ---------------------------------------------------------------- 文字表

/// 標準文字 (0x20〜0x7F)。ASCII のうち 9 文字が置き換わっている
fn standard_char(c: u8) -> char {
    match c {
        0x2A => 'á',
        0x5C => 'é',
        0x5E => 'í',
        0x5F => 'ó',
        0x60 => 'ú',
        0x7B => 'ç',
        0x7C => '÷',
        0x7D => 'Ñ',
        0x7E => 'ñ',
        0x7F => '■',
        _ => c as char,
    }
}

/// 特殊文字 (0x11 0x30〜0x3F)。0x39 は透明な空白 (None)
const SPECIAL: [Option<char>; 16] = [
    Some('®'), Some('°'), Some('½'), Some('¿'), Some('™'), Some('¢'), Some('£'), Some('♪'),
    Some('à'), None, Some('è'), Some('â'), Some('ê'), Some('î'), Some('ô'), Some('û'),
];

/// 拡張文字 (0x12 0x20〜0x3F)
const EXT12: [char; 32] = [
    'Á', 'É', 'Ó', 'Ú', 'Ü', 'ü', '‘', '¡', '*', '\'', '—', '©', '℠', '•', '“', '”',
    'À', 'Â', 'Ç', 'È', 'Ê', 'Ë', 'ë', 'Î', 'Ï', 'ï', 'Ô', 'Ù', 'ù', 'Û', '«', '»',
];

/// 拡張文字 (0x13 0x20〜0x3F)
const EXT13: [char; 32] = [
    'Ã', 'ã', 'Í', 'Ì', 'ì', 'Ò', 'ò', 'Õ', 'õ', '{', '}', '\\', '^', '_', '|', '~',
    'Ä', 'ä', 'Ö', 'ö', 'ß', '¥', '¤', '│', 'Å', 'å', 'Ø', 'ø', '┌', '┐', '└', '┘',
];

/// 字幕の色 (前景)。白・緑・青・シアン・赤・黄・マゼンタ (+ 背景属性用の黒)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CcColor {
    #[default]
    White,
    Green,
    Blue,
    Cyan,
    Red,
    Yellow,
    Magenta,
    Black,
}

impl CcColor {
    fn from_index(i: u8) -> Self {
        [Self::White, Self::Green, Self::Blue, Self::Cyan, Self::Red, Self::Yellow, Self::Magenta, Self::Black]
            [i as usize & 7]
    }

    fn rgb(self) -> [u8; 3] {
        match self {
            Self::White => [235, 235, 235],
            Self::Green => [40, 235, 40],
            Self::Blue => [40, 40, 235],
            Self::Cyan => [40, 235, 235],
            Self::Red => [235, 40, 40],
            Self::Yellow => [235, 235, 40],
            Self::Magenta => [235, 40, 235],
            Self::Black => [16, 16, 16],
        }
    }
}

/// 背景: 色と不透明度 (0 = 透明、1 = 半透明、2 = 不透明)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Background {
    pub color: CcColor,
    pub opacity: u8,
}

impl Default for Background {
    fn default() -> Self {
        Self { color: CcColor::Black, opacity: 2 }
    }
}

/// 文字の属性
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: CcColor,
    pub bg: Background,
    pub italic: bool,
    pub underline: bool,
    pub flash: bool,
}

/// 字幕セル 1 つ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

type Screen = [[Option<Cell>; COLS]; ROWS];

// ---------------------------------------------------------------- 送出側のデータ列

/// 1 文字の送り方
enum Code {
    /// 標準文字 1 バイト
    Std(u8),
    /// 特殊文字・拡張文字・行中コードなどの 2 バイトの制御コード (拡張文字は直前に代替の標準文字を送る)
    Pair([u8; 2]),
}

/// 文字 → 送り方。標準文字で送れないものは特殊・拡張文字にする
fn encode_char(ch: char, dc: u8, out: &mut Vec<Code>) {
    let ch1 = 0x10 | dc << 3;
    // 標準文字 (置き換わっている ASCII 9 文字は拡張文字で送る)
    for c in 0x20u8..=0x7F {
        if standard_char(c) == ch && !matches!(c, 0x7F) {
            out.push(Code::Std(c));
            return;
        }
    }
    if let Some(i) = SPECIAL.iter().position(|&c| c == Some(ch)) {
        out.push(Code::Pair([ch1 | 0x01, 0x30 + i as u8]));
        return;
    }
    for (first, table) in [(0x02u8, &EXT12), (0x03, &EXT13)] {
        if let Some(i) = table.iter().position(|&c| c == ch) {
            // 拡張文字を解釈できない古いデコーダ向けの代替文字 (対応するデコーダは自動で消す)
            out.push(Code::Std(fallback(ch)));
            out.push(Code::Pair([ch1 | first, 0x20 + i as u8]));
            return;
        }
    }
    out.push(Code::Std(b'?'));
}

/// 拡張文字の代替 (アクセントを取った文字か空白)
fn fallback(ch: char) -> u8 {
    let base = match ch {
        'Á' | 'À' | 'Â' | 'Ã' | 'Ä' | 'Å' => 'A',
        'É' | 'È' | 'Ê' | 'Ë' => 'E',
        'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
        'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => 'O',
        'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
        'Ç' => 'C',
        'ã' | 'ä' | 'å' => 'a',
        'ë' => 'e',
        'ì' | 'ï' => 'i',
        'ò' | 'õ' | 'ö' | 'ø' => 'o',
        'ù' | 'ü' => 'u',
        'ß' => 's',
        '‘' | '\'' => '\'',
        '“' | '”' | '«' | '»' => '"',
        '¡' => '!',
        '—' => '-',
        '©' => 'c',
        '•' => '.',
        '[' | '{' => '[',
        ']' | '}' => ']',
        _ => ' ',
    };
    base as u8
}

/// PAC (行 1〜15、字下げまたは色、下線)
fn pac(dc: u8, row: usize, attr: u8) -> [u8; 2] {
    const ROW: [(u8, u8); 15] = [
        (0x11, 0), (0x11, 1), (0x12, 0), (0x12, 1), (0x15, 0), (0x15, 1), (0x16, 0), (0x16, 1),
        (0x17, 0), (0x17, 1), (0x10, 0), (0x13, 0), (0x13, 1), (0x14, 0), (0x14, 1),
    ];
    let (c1, hi) = ROW[row.clamp(1, ROWS) - 1];
    [c1 | dc << 3, 0x40 | hi << 5 | (attr & 0x1F)]
}

/// 1 行の文を送り方の列にする。`*斜体*` `_下線_` `{red}` などで属性を変えられる (行中コード、1 桁の空白になる)。
/// 戻り値は (送り方, 画面上の桁数)
fn encode_line(line: &str, dc: u8) -> (Vec<Code>, usize) {
    let ch1 = 0x11 | dc << 3;
    let mut out = Vec::new();
    let (mut cols, mut color, mut italic, mut underline) = (0usize, 0u8, false, false);
    let mut chars = line.chars().peekable();
    let mid_row = |out: &mut Vec<Code>, color: u8, italic: bool, underline: bool| {
        let attr = if italic { 7 } else { color };
        out.push(Code::Pair([ch1, 0x20 | attr << 1 | underline as u8]));
    };
    while let Some(ch) = chars.next() {
        if cols >= COLS {
            break;
        }
        match ch {
            '*' => {
                italic = !italic;
                mid_row(&mut out, color, italic, underline);
                cols += 1;
            }
            '_' => {
                underline = !underline;
                mid_row(&mut out, color, italic, underline);
                cols += 1;
            }
            '{' => {
                let name: String = chars.by_ref().take_while(|&c| c != '}').collect();
                let c = match name.as_str() {
                    "white" => 0,
                    "green" => 1,
                    "blue" => 2,
                    "cyan" => 3,
                    "red" => 4,
                    "yellow" => 5,
                    "magenta" => 6,
                    _ => color,
                };
                color = c;
                italic = false;
                mid_row(&mut out, color, italic, underline);
                cols += 1;
            }
            _ => {
                encode_char(ch, dc, &mut out);
                cols += 1;
            }
        }
    }
    (out, cols)
}

/// 送り方の列を 2 バイト組に詰める。制御コード (特殊・拡張文字、行中コード) は 2 回送る
fn pack(codes: Vec<Code>, seq: &mut Vec<[u8; 2]>) {
    let mut pending: Option<u8> = None;
    for code in codes {
        match code {
            Code::Std(c) => match pending.take() {
                Some(a) => seq.push([a, c]),
                None => pending = Some(c),
            },
            Code::Pair(p) => {
                if let Some(a) = pending.take() {
                    seq.push([a, 0]);
                }
                seq.push(p);
                seq.push(p);
            }
        }
    }
    if let Some(a) = pending {
        seq.push([a, 0]);
    }
}

/// 字幕の送出列 (2 バイト組、パリティなし)。`channel` は CC1〜CC4
pub fn sequence(lines: &[String], mode: CaptionMode, channel: u8) -> Vec<[u8; 2]> {
    let ch = channel.clamp(1, 4) - 1;
    let (field2, dc) = (ch >= 2, ch & 1);
    // 雑制御コードの 1 バイト目 (第 2 フィールドは 0x15 / 0x1D)
    let misc = |code: u8| [(if field2 { 0x15 } else { 0x14 }) | dc << 3, code];
    let ctrl = |seq: &mut Vec<[u8; 2]>, p: [u8; 2]| {
        seq.push(p);
        seq.push(p);
    };
    let nulls = |seq: &mut Vec<[u8; 2]>, n: usize| seq.extend(std::iter::repeat_n([0, 0], n));
    const RCL: u8 = 0x20;
    const RU3: u8 = 0x26;
    const RDC: u8 = 0x29;
    const EDM: u8 = 0x2C;
    const CR: u8 = 0x2D;
    const ENM: u8 = 0x2E;
    const EOC: u8 = 0x2F;
    let mut seq = Vec::new();
    // 中央揃えの位置へ: PAC の字下げ (4 桁単位) + タブオフセット (1〜3 桁)
    let place = |seq: &mut Vec<[u8; 2]>, row: usize, cols: usize| {
        let start = (COLS - cols.min(COLS)) / 2;
        ctrl(seq, pac(dc, row, ((start / 4 + 8) as u8) << 1));
        if start % 4 > 0 {
            ctrl(seq, [0x17 | dc << 3, 0x20 + (start % 4) as u8]);
        }
    };
    match mode {
        CaptionMode::PopOn => {
            for line in lines {
                let (codes, cols) = encode_line(line, dc);
                ctrl(&mut seq, misc(RCL));
                ctrl(&mut seq, misc(ENM));
                place(&mut seq, 14, cols);
                pack(codes, &mut seq);
                ctrl(&mut seq, misc(EOC));
                nulls(&mut seq, 75);
            }
            ctrl(&mut seq, misc(EDM));
            nulls(&mut seq, 30);
        }
        CaptionMode::RollUp => {
            ctrl(&mut seq, misc(RU3));
            ctrl(&mut seq, pac(dc, 15, 0x10));
            for line in lines {
                ctrl(&mut seq, misc(CR));
                let (codes, _) = encode_line(line, dc);
                pack(codes, &mut seq);
                nulls(&mut seq, 45);
            }
        }
        CaptionMode::PaintOn => {
            ctrl(&mut seq, misc(RDC));
            for line in lines {
                ctrl(&mut seq, misc(EDM));
                let (codes, cols) = encode_line(line, dc);
                place(&mut seq, 14, cols);
                pack(codes, &mut seq);
                nulls(&mut seq, 60);
            }
            ctrl(&mut seq, misc(EDM));
            nulls(&mut seq, 30);
        }
    }
    if lines.is_empty() {
        nulls(&mut seq, 1);
    }
    seq
}

// ---------------------------------------------------------------- デコーダ

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// まだ字幕のモードに入っていない
    None,
    PopOn,
    RollUp(usize),
    PaintOn,
    /// テキストモード (表示しない)
    Text,
}

/// 受像機の字幕デコーダ (EIA-608)
#[derive(Clone, Debug)]
pub struct CaptionDecoder {
    /// 選んでいるチャンネル (1〜4)
    channel: u8,
    displayed: Screen,
    nondisplayed: Screen,
    mode: Mode,
    row: usize,
    col: usize,
    style: Style,
    /// ロールアップの最下行
    base_row: usize,
    /// 直前に受けた制御コード (フィールドごと。2 回続けて送られるので 2 回目を無視する)
    last_ctrl: [Option<[u8; 2]>; 2],
    /// フィールドごとに、今の文字が属するデータチャンネル (直前の制御コードで決まる)
    data_channel: [u8; 2],
    /// 第 2 フィールドの XDS パケットの途中
    in_xds: bool,
    /// 表示が変わるたびに増える (描き直しの判定用)
    pub version: u64,
}

impl Default for CaptionDecoder {
    fn default() -> Self {
        Self {
            channel: 1,
            displayed: [[None; COLS]; ROWS],
            nondisplayed: [[None; COLS]; ROWS],
            mode: Mode::None,
            row: ROWS - 1,
            col: 0,
            style: Style::default(),
            base_row: ROWS - 1,
            last_ctrl: [None; 2],
            data_channel: [0; 2],
            in_xds: false,
            version: 0,
        }
    }
}

#[inline]
fn parity_ok(b: u8) -> bool {
    b.count_ones() % 2 == 1
}

impl CaptionDecoder {
    pub fn reset(&mut self) {
        let channel = self.channel;
        let version = self.version + 1;
        *self = Self { channel, version, ..Self::default() };
    }

    /// 表示するチャンネル (CC1〜CC4) を選ぶ。変わったら表示を消す
    pub fn set_channel(&mut self, channel: u8) {
        let channel = channel.clamp(1, 4);
        if channel != self.channel {
            self.channel = channel;
            self.reset();
        }
    }

    /// 表示中の画面 (行 × 桁)
    pub fn screen(&self) -> &[[Option<Cell>; COLS]; ROWS] {
        &self.displayed
    }

    /// 表示中の各行の文字列 (空の行は除く。テスト・デバッグ用)
    pub fn rows(&self) -> Vec<String> {
        self.displayed
            .iter()
            .map(|r| r.iter().map(|c| c.map_or(' ', |c| c.ch)).collect::<String>().trim().to_string())
            .filter(|r| !r.is_empty())
            .collect()
    }

    /// 1 フレームぶんの 2 バイト (パリティ付き) を入れる。`field` は 0 = ライン 21、1 = ライン 284。
    /// スライスできなかったら None
    pub fn feed(&mut self, field: u8, pair: Option<[u8; 2]>) {
        let f = field as usize & 1;
        let Some(pair) = pair else {
            self.last_ctrl[f] = None;
            return;
        };
        let (c1, c2) = (pair[0] & 0x7f, pair[1] & 0x7f);
        let my_field = (self.channel - 1) / 2 == field;
        let my_dc = (self.channel - 1) & 1;

        if (0x10..=0x1f).contains(&c1) {
            // 制御コード: パリティが合わなければ捨てる
            if !parity_ok(pair[0]) || !parity_ok(pair[1]) {
                self.last_ctrl[f] = None;
                return;
            }
            if self.last_ctrl[f] == Some([c1, c2]) {
                self.last_ctrl[f] = None;
                return;
            }
            self.last_ctrl[f] = Some([c1, c2]);
            self.in_xds = false;
            let dc = (c1 >> 3) & 1;
            self.data_channel[f] = dc;
            if my_field && dc == my_dc {
                self.control(c1 & 0x17, c2);
            }
            return;
        }
        self.last_ctrl[f] = None;
        // XDS (第 2 フィールド): 0x01〜0x0E で始まり 0x0F で終わる
        if f == 1 && (0x01..=0x0f).contains(&c1) && parity_ok(pair[0]) {
            self.in_xds = c1 != 0x0f;
            return;
        }
        if !my_field || self.data_channel[f] != my_dc || (f == 1 && self.in_xds) {
            return;
        }
        for (b, c) in [(pair[0], c1), (pair[1], c2)] {
            if c == 0 && parity_ok(b) {
                continue;
            }
            if !parity_ok(b) {
                // パリティ誤りの文字は塗りつぶしで出す
                self.put(Some('■'));
            } else if c >= 0x20 {
                self.put(Some(standard_char(c)));
            }
        }
    }

    /// 文字を書くメモリ (ポップオンは非表示メモリ、それ以外は表示メモリ)
    fn target(&mut self) -> &mut Screen {
        if self.mode == Mode::PopOn { &mut self.nondisplayed } else { &mut self.displayed }
    }

    fn touched(&mut self) {
        if self.mode != Mode::PopOn {
            self.version += 1;
        }
    }

    /// 今の位置に 1 文字書いて進む (None は透明な空白)
    fn put(&mut self, ch: Option<char>) {
        if matches!(self.mode, Mode::None | Mode::Text) {
            return;
        }
        let (row, col, style) = (self.row, self.col.min(COLS - 1), self.style);
        self.target()[row][col] = ch.map(|ch| Cell { ch, style });
        self.col = (col + 1).min(COLS);
        self.touched();
    }

    fn backspace(&mut self) {
        if self.col > 0 {
            self.col -= 1;
            let (row, col) = (self.row, self.col);
            self.target()[row][col] = None;
            self.touched();
        }
    }

    /// `c1` はデータチャンネルのビットを落としたもの (0x10〜0x17)
    fn control(&mut self, c1: u8, c2: u8) {
        match (c1, c2) {
            // 雑制御コード (第 2 フィールドは 0x15 を使う局もある)
            (0x14 | 0x15, 0x20..=0x2f) => self.misc(c2),
            // タブオフセット
            (0x17, 0x21..=0x23) => self.col = (self.col + (c2 - 0x20) as usize).min(COLS - 1),
            // 行中コード: 色・斜体・下線。1 桁の空白になる
            (0x11, 0x20..=0x2f) => {
                let attr = (c2 >> 1) & 7;
                if attr == 7 {
                    self.style.italic = true;
                } else {
                    self.style.fg = CcColor::from_index(attr);
                    self.style.italic = false;
                }
                self.style.underline = c2 & 1 == 1;
                self.style.flash = false;
                self.put(Some(' '));
            }
            // 特殊文字
            (0x11, 0x30..=0x3f) => self.put(SPECIAL[(c2 - 0x30) as usize]),
            // 拡張文字: 直前の代替文字を消して置き換える
            (0x12 | 0x13, 0x20..=0x3f) => {
                let table = if c1 == 0x12 { &EXT12 } else { &EXT13 };
                self.backspace();
                self.put(Some(table[(c2 - 0x20) as usize]));
            }
            // 背景属性 (色・半透明)
            (0x10, 0x20..=0x2f) => {
                self.style.bg = Background { color: CcColor::from_index((c2 >> 1) & 7), opacity: 2 - (c2 & 1) };
                self.attribute_space();
            }
            (0x17, 0x2d) => {
                self.style.bg = Background { color: CcColor::Black, opacity: 0 };
                self.attribute_space();
            }
            (0x17, 0x2e | 0x2f) => {
                self.style.fg = CcColor::Black;
                self.style.underline = c2 == 0x2f;
                self.attribute_space();
            }
            // 前置アドレスコード
            (_, 0x40..=0x7f) => self.preamble(c1, c2),
            _ => {}
        }
    }

    /// 背景属性コードは直前の空白に自動で戻って重なる
    fn attribute_space(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        }
        self.put(Some(' '));
    }

    fn preamble(&mut self, c1: u8, c2: u8) {
        let hi = (c2 >> 5) & 1;
        let row = match (c1, hi) {
            (0x11, 0) => 1,
            (0x11, _) => 2,
            (0x12, 0) => 3,
            (0x12, _) => 4,
            (0x15, 0) => 5,
            (0x15, _) => 6,
            (0x16, 0) => 7,
            (0x16, _) => 8,
            (0x17, 0) => 9,
            (0x17, _) => 10,
            (0x10, 0) => 11,
            (0x13, 0) => 12,
            (0x13, _) => 13,
            (0x14, 0) => 14,
            (0x14, _) => 15,
            _ => return,
        } - 1;
        let attr = c2 & 0x1f;
        let mut style = Style { underline: attr & 1 == 1, bg: Background::default(), ..Style::default() };
        let a = attr >> 1;
        let col = if a >= 8 {
            (a as usize - 8) * 4
        } else {
            if a == 7 {
                style.italic = true;
            } else {
                style.fg = CcColor::from_index(a);
            }
            0
        };
        self.style = style;
        self.col = col;
        if let Mode::RollUp(n) = self.mode {
            // ロールアップの窓を新しい最下行へ動かす
            let new_base = row.max(n - 1);
            if new_base != self.base_row {
                let mut moved = [[None; COLS]; ROWS];
                for k in 0..n {
                    if let (Some(src), Some(dst)) = (self.base_row.checked_sub(k), new_base.checked_sub(k)) {
                        moved[dst] = self.displayed[src];
                    }
                }
                self.displayed = moved;
                self.base_row = new_base;
                self.version += 1;
            }
            self.row = self.base_row;
        } else {
            self.row = row;
        }
    }

    fn misc(&mut self, c2: u8) {
        match c2 {
            // RCL: ポップオン
            0x20 => self.mode = Mode::PopOn,
            // BS
            0x21 => self.backspace(),
            // DER: 行末まで消す
            0x24 => {
                let (row, col) = (self.row, self.col.min(COLS));
                for c in &mut self.target()[row][col..] {
                    *c = None;
                }
                self.touched();
            }
            // RU2〜RU4: ロールアップ
            0x25..=0x27 => {
                let n = (c2 - 0x25 + 2) as usize;
                if !matches!(self.mode, Mode::RollUp(_)) {
                    self.displayed = [[None; COLS]; ROWS];
                    self.nondisplayed = [[None; COLS]; ROWS];
                    self.base_row = ROWS - 1;
                    self.col = 0;
                    self.version += 1;
                }
                self.mode = Mode::RollUp(n);
                self.base_row = self.base_row.max(n - 1);
                self.row = self.base_row;
                // 窓の外の行は消す
                for r in 0..ROWS {
                    if r + n <= self.base_row || r > self.base_row {
                        self.displayed[r] = [None; COLS];
                    }
                }
            }
            // FON: 点滅
            0x28 => self.style.flash = true,
            // RDC: ペイントオン
            0x29 => self.mode = Mode::PaintOn,
            // TR / RTD: テキストモード (表示しない)
            0x2a | 0x2b => self.mode = Mode::Text,
            // EDM: 表示メモリを消す
            0x2c => {
                self.displayed = [[None; COLS]; ROWS];
                self.version += 1;
            }
            // CR: ロールアップなら 1 行上へ流す
            0x2d => {
                if let Mode::RollUp(n) = self.mode {
                    let top = self.base_row + 1 - n;
                    for r in top..self.base_row {
                        self.displayed[r] = self.displayed[r + 1];
                    }
                    self.displayed[self.base_row] = [None; COLS];
                    self.row = self.base_row;
                    self.col = 0;
                    self.style = Style::default();
                    self.version += 1;
                }
            }
            // ENM: 非表示メモリを消す
            0x2e => self.nondisplayed = [[None; COLS]; ROWS],
            // EOC: 表示と非表示を入れ替える (ポップオンの字幕が出る)
            0x2f => {
                std::mem::swap(&mut self.displayed, &mut self.nondisplayed);
                self.mode = Mode::PopOn;
                self.version += 1;
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ 画面表示

    /// 字幕を画面に描く。`field_index` は点滅の位相に使う
    pub fn render(&self, frame: &mut RgbaFrame, field_index: u64) {
        let (w, h) = (frame.width, frame.height);
        // 1 桁の幅 [サンプル] と 1 行の高さ [ライン] (画面の中央 80% ほど)
        let cw = (w * 77 / 100 / COLS).max(6);
        let rh = (h * 81 / 100 / ROWS).max(10);
        let left = (w - cw * COLS) / 2;
        let top = (h - rh * ROWS) / 2;
        // ドットの大きさ (小数): 横はセル幅の 80% に 5 ドット、縦はセルの高さの 92% に
        // アクセント 2 行 + 本体 7 行 + セディーユ 1 行 + 下線 1 行の 11 行
        let dw = cw as f32 * 0.8 / 5.0;
        let dh = rh as f32 * 0.92 / 11.0;
        let flash_off = (field_index / 30) % 2 == 1;
        for (r, row) in self.displayed.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let x0 = left + c * cw;
                let y0 = top + r * rh;
                // 文字の並びの前後に 1 桁ずつ背景を延ばす
                let pad = cell.is_none()
                    && ((c > 0 && row[c - 1].is_some_and(|p| p.style.bg.opacity > 0))
                        || (c + 1 < COLS && row[c + 1].is_some_and(|n| n.style.bg.opacity > 0)));
                if pad {
                    let left_cell = if c > 0 { row[c - 1] } else { None };
                    let bg = left_cell.or(row.get(c + 1).copied().flatten()).map(|p| p.style.bg);
                    fill_rect(frame, x0, y0, cw, rh, bg.unwrap_or_default());
                    continue;
                }
                let Some(cell) = cell else { continue };
                fill_rect(frame, x0, y0, cw, rh, cell.style.bg);
                if cell.style.flash && flash_off {
                    continue;
                }
                let g = caption_glyph(cell.ch);
                let fg = cell.style.fg.rgb();
                let gx0 = x0 as f32 + (cw as f32 - 5.0 * dw) * 0.5;
                // 縦: アクセント 2 行 + 本体 7 行 + 下の 1 行 (セディーユ) + 下線
                let gy0 = y0 as f32 + (rh as f32 - 11.0 * dh) * 0.5;
                // 小数の位置のドットを画素の境目に丸めて描く (隣のドットと隙間ができない)
                let dot = |frame: &mut RgbaFrame, fx: f32, fy: f32, fw: f32, fh: f32| {
                    let (xa, ya) = (fx.round() as usize, fy.round() as usize);
                    let (xb, yb) = ((fx + fw).round() as usize, (fy + fh).round() as usize);
                    put_rect(frame, xa, ya, xb.saturating_sub(xa).max(1), yb.saturating_sub(ya).max(1), fg);
                };
                for (gy, bits) in g.rows.iter().enumerate() {
                    // 斜体: 上ほど右へずらす
                    let shear = if cell.style.italic { (9 - gy.min(9)) as f32 * dw / 3.0 } else { 0.0 };
                    for gx in 0..5 {
                        if bits >> (4 - gx) & 1 == 1 {
                            dot(frame, gx0 + gx as f32 * dw + shear, gy0 + gy as f32 * dh, dw, dh);
                        }
                    }
                }
                if cell.style.underline {
                    dot(frame, x0 as f32, gy0 + 10.0 * dh, cw as f32, dh);
                }
            }
        }
    }
}

fn fill_rect(frame: &mut RgbaFrame, x: usize, y: usize, w: usize, h: usize, bg: Background) {
    let a = match bg.opacity {
        0 => return,
        1 => 0.5,
        _ => 1.0,
    };
    let rgb = bg.color.rgb();
    let (fw, fh) = (frame.width, frame.height);
    for yy in y..(y + h).min(fh) {
        for xx in x..(x + w).min(fw) {
            let o = (yy * fw + xx) * 4;
            for k in 0..3 {
                let v = frame.data[o + k] as f32 * (1.0 - a) + rgb[k] as f32 * a;
                frame.data[o + k] = v as u8;
            }
        }
    }
}

fn put_rect(frame: &mut RgbaFrame, x: usize, y: usize, w: usize, h: usize, rgb: [u8; 3]) {
    let (fw, fh) = (frame.width, frame.height);
    for yy in y..(y + h).min(fh) {
        for xx in x..(x + w).min(fw) {
            let o = (yy * fw + xx) * 4;
            frame.data[o..o + 3].copy_from_slice(&rgb);
        }
    }
}

// ---------------------------------------------------------------- 字形

/// 字幕の文字の字形: 11 行 × 5 ドット (上 2 行がアクセント、7 行が本体、最後がセディーユ)
struct CcGlyph {
    rows: [u8; 10],
}

fn bits(rows: [&str; 7]) -> [u8; 7] {
    let mut g = [0u8; 7];
    for (d, r) in g.iter_mut().zip(rows) {
        *d = r.bytes().fold(0, |a, b| (a << 1) | (b == b'1') as u8);
    }
    g
}

/// OSD のフォントにない字幕用の記号
fn special_glyph(ch: char) -> Option<[u8; 7]> {
    Some(bits(match ch {
        '®' => ["01110", "10001", "11101", "11011", "11101", "10001", "01110"],
        '©' => ["01110", "10001", "10111", "10101", "10111", "10001", "01110"],
        '™' => ["11111", "00100", "00100", "00000", "11011", "10101", "10101"],
        '℠' => ["01100", "10000", "01000", "00100", "11011", "10101", "10101"],
        '½' => ["10000", "10000", "10000", "01110", "00001", "00110", "01111"],
        '¿' => ["00100", "00000", "00100", "01000", "10000", "10001", "01110"],
        '¡' => ["00100", "00000", "00100", "00100", "00100", "00100", "00100"],
        '¢' => ["00100", "01110", "10100", "10100", "10101", "01110", "00100"],
        '£' => ["00110", "01001", "01000", "11100", "01000", "01001", "11110"],
        '¥' => ["10001", "01010", "11111", "00100", "11111", "00100", "00100"],
        '¤' => ["00000", "10001", "01110", "01010", "01110", "10001", "00000"],
        '♪' => ["00011", "00011", "00010", "00010", "01110", "11110", "01100"],
        '•' => ["00000", "00000", "01110", "01110", "01110", "00000", "00000"],
        '«' => ["00000", "00101", "01010", "10100", "01010", "00101", "00000"],
        '»' => ["00000", "10100", "01010", "00101", "01010", "10100", "00000"],
        '—' => ["00000", "00000", "00000", "11111", "00000", "00000", "00000"],
        '‘' => ["00010", "00100", "00110", "00000", "00000", "00000", "00000"],
        '’' => ["01100", "00100", "01000", "00000", "00000", "00000", "00000"],
        '“' => ["01001", "10010", "11011", "00000", "00000", "00000", "00000"],
        '”' => ["11011", "01001", "10010", "00000", "00000", "00000", "00000"],
        '÷' => ["00000", "00100", "00000", "11111", "00000", "00100", "00000"],
        'ß' => ["01100", "10010", "10010", "10110", "10001", "10001", "10110"],
        'Ø' => ["01111", "10011", "10101", "10101", "10101", "11001", "11110"],
        'ø' => ["00000", "00001", "01110", "10011", "10101", "11001", "01110"],
        '│' => ["00100", "00100", "00100", "00100", "00100", "00100", "00100"],
        '┌' => ["00000", "00000", "00000", "00111", "00100", "00100", "00100"],
        '┐' => ["00000", "00000", "00000", "11100", "00100", "00100", "00100"],
        '└' => ["00100", "00100", "00100", "00111", "00000", "00000", "00000"],
        '┘' => ["00100", "00100", "00100", "11100", "00000", "00000", "00000"],
        _ => return None,
    }))
}

/// アクセント付きの文字 → (基の文字, アクセント 2 行, セディーユ)
fn decompose(ch: char) -> Option<(char, [u8; 2], bool)> {
    const ACUTE: [u8; 2] = [0b00010, 0b00100];
    const GRAVE: [u8; 2] = [0b01000, 0b00100];
    const CIRC: [u8; 2] = [0b00100, 0b01010];
    const DIAER: [u8; 2] = [0b00000, 0b01010];
    const TILDE: [u8; 2] = [0b01101, 0b10110];
    const RING: [u8; 2] = [0b00100, 0b01010];
    let (base, acc, ced) = match ch {
        'á' => ('a', ACUTE, false),
        'é' => ('e', ACUTE, false),
        'í' => ('i', ACUTE, false),
        'ó' => ('o', ACUTE, false),
        'ú' => ('u', ACUTE, false),
        'à' => ('a', GRAVE, false),
        'è' => ('e', GRAVE, false),
        'ì' => ('i', GRAVE, false),
        'ò' => ('o', GRAVE, false),
        'ù' => ('u', GRAVE, false),
        'â' => ('a', CIRC, false),
        'ê' => ('e', CIRC, false),
        'î' => ('i', CIRC, false),
        'ô' => ('o', CIRC, false),
        'û' => ('u', CIRC, false),
        'ä' => ('a', DIAER, false),
        'ë' => ('e', DIAER, false),
        'ï' => ('i', DIAER, false),
        'ö' => ('o', DIAER, false),
        'ü' => ('u', DIAER, false),
        'ã' => ('a', TILDE, false),
        'õ' => ('o', TILDE, false),
        'ñ' => ('n', TILDE, false),
        'å' => ('a', RING, false),
        'ç' => ('c', [0, 0], true),
        'Á' => ('A', ACUTE, false),
        'É' => ('E', ACUTE, false),
        'Í' => ('I', ACUTE, false),
        'Ó' => ('O', ACUTE, false),
        'Ú' => ('U', ACUTE, false),
        'À' => ('A', GRAVE, false),
        'È' => ('E', GRAVE, false),
        'Ì' => ('I', GRAVE, false),
        'Ò' => ('O', GRAVE, false),
        'Ù' => ('U', GRAVE, false),
        'Â' => ('A', CIRC, false),
        'Ê' => ('E', CIRC, false),
        'Î' => ('I', CIRC, false),
        'Ô' => ('O', CIRC, false),
        'Û' => ('U', CIRC, false),
        'Ä' => ('A', DIAER, false),
        'Ë' => ('E', DIAER, false),
        'Ï' => ('I', DIAER, false),
        'Ö' => ('O', DIAER, false),
        'Ü' => ('U', DIAER, false),
        'Ã' => ('A', TILDE, false),
        'Õ' => ('O', TILDE, false),
        'Ñ' => ('N', TILDE, false),
        'Å' => ('A', RING, false),
        'Ç' => ('C', [0, 0], true),
        _ => return None,
    };
    Some((base, acc, ced))
}

fn caption_glyph(ch: char) -> CcGlyph {
    let mut rows = [0u8; 10];
    if let Some(g) = special_glyph(ch) {
        rows[2..9].copy_from_slice(&g);
    } else if let Some((base, acc, ced)) = decompose(ch) {
        let mut g = font::glyph(base);
        if base.is_lowercase() {
            // 小文字は本体の上 2 行 (i の点を含む) をアクセントに使う
            g[0] = acc[0];
            g[1] = acc[1];
        } else {
            rows[0] = acc[0];
            rows[1] = acc[1];
        }
        rows[2..9].copy_from_slice(&g);
        if ced {
            rows[9] = 0b00100;
        }
    } else {
        rows[2..9].copy_from_slice(&font::glyph(ch));
    }
    CcGlyph { rows }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vbi::odd_parity;

    fn run(lines: &[&str], mode: CaptionMode, channel: u8, frames: usize) -> CaptionDecoder {
        let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        let seq = sequence(&lines, mode, channel);
        let mut d = CaptionDecoder::default();
        d.set_channel(channel);
        let field = (channel - 1) / 2;
        for f in 0..frames.min(seq.len()) {
            let [a, b] = seq[f];
            d.feed(field, Some([odd_parity(a), odd_parity(b)]));
            // もう一方のフィールドはヌル
            d.feed(1 - field, Some([0x80, 0x80]));
        }
        d
    }

    /// 送出列の最初の行が出たところまでのフレーム数
    fn until_first_hold(lines: &[&str], mode: CaptionMode, channel: u8) -> usize {
        let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        let seq = sequence(&lines, mode, channel);
        seq.iter().position(|p| *p == [0, 0]).unwrap() + 1
    }

    #[test]
    fn pop_on_centered_with_accents() {
        let text = ["¿Dónde está? ♪ Ça va"];
        let n = until_first_hold(&text, CaptionMode::PopOn, 1);
        let d = run(&text, CaptionMode::PopOn, 1, n);
        assert_eq!(d.rows(), vec!["¿Dónde está? ♪ Ça va".to_string()]);
        // 中央揃え: 20 文字なら 6 桁目から
        let row = &d.screen()[13];
        assert!(row[5].is_none() && row[6].is_some());
        // EOC の前は何も出ていない
        let d = run(&text, CaptionMode::PopOn, 1, n - 3);
        assert!(d.rows().is_empty());
    }

    #[test]
    fn roll_up_scrolls() {
        let text = ["ONE", "TWO", "THREE", "FOUR"];
        let d = run(&text, CaptionMode::RollUp, 1, usize::MAX);
        assert_eq!(d.rows(), vec!["TWO", "THREE", "FOUR"]);
        assert!(d.screen()[14][0].is_some(), "base row is 15");
    }

    #[test]
    fn paint_on_and_styles() {
        let text = ["{red}RED *ital* _und_"];
        let d = run(&text, CaptionMode::PaintOn, 1, until_first_hold(&text, CaptionMode::PaintOn, 1));
        let row = d.screen()[13].iter().flatten().collect::<Vec<_>>();
        let red = row.iter().find(|c| c.ch == 'R').unwrap();
        assert_eq!(red.style.fg, CcColor::Red);
        assert!(row.iter().find(|c| c.ch == 'l').unwrap().style.italic);
        assert!(row.iter().find(|c| c.ch == 'd').unwrap().style.underline);
    }

    #[test]
    fn channels_are_separate() {
        let text = ["CC3 TEXT"];
        let n = until_first_hold(&text, CaptionMode::PopOn, 3);
        assert_eq!(run(&text, CaptionMode::PopOn, 3, n).rows(), vec!["CC3 TEXT"]);
        // CC3 で送ったものは CC1 には出ない
        let lines = vec!["CC3 TEXT".to_string()];
        let seq = sequence(&lines, CaptionMode::PopOn, 3);
        let mut d = CaptionDecoder::default();
        for p in &seq[..n] {
            d.feed(1, Some([odd_parity(p[0]), odd_parity(p[1])]));
        }
        assert!(d.rows().is_empty());
    }

    #[test]
    fn parity_errors() {
        let mut d = CaptionDecoder::default();
        for p in [[0x14, 0x29], [0x14, 0x29], [0x14, 0x70], [0x14, 0x70]] {
            d.feed(0, Some([odd_parity(p[0]), odd_parity(p[1])]));
        }
        // 2 バイト目のパリティ誤り → ■
        d.feed(0, Some([odd_parity(b'A'), b'B']));
        assert_eq!(d.rows(), vec!["A■".to_string()]);
        // 制御コードのパリティ誤りは無視 (EDM が効かない)
        d.feed(0, Some([0x14, odd_parity(0x2C)]));
        assert_eq!(d.rows(), vec!["A■".to_string()]);
    }

    #[test]
    fn glyphs_exist_for_all_tables() {
        let tofu = font::glyph('\u{E000}');
        for ch in SPECIAL.iter().flatten().chain(EXT12.iter()).chain(EXT13.iter()) {
            let g = caption_glyph(*ch);
            assert_ne!(&g.rows[2..9], &tofu, "{ch}");
        }
        for c in 0x20u8..=0x7F {
            let ch = standard_char(c);
            assert_ne!(&caption_glyph(ch).rows[2..9], &tofu, "{ch}");
        }
    }
}
