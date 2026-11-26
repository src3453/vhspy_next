//! 番組データ (告示 別表第五〜十一号) の送出と受信。
//!
//! 送出 ([`cycle`]): 非一括形・反復形の番組を、ページ番号の順に繰り返し送る。
//!
//! ```text
//! 送出単位 1: [番組管理データ (番組データヘッダ)] [ページ 1 (ページデータヘッダ + ヘッダ文 + 本文)]
//! 送出単位 2: [ページ 2] …
//! ```
//!
//! 各送出単位の最初のパケットで伝送制御フラグを立てる。ページの送出間隔は別表第六号どおり
//! 「直前のページの送出時間 × 3 + 1 秒」以上空け、その間は全 NUL のパケットを送る。
//!
//! 受信 ([`Receiver`]): 選んだ番組のページデータヘッダと本文・ヘッダ文を蓄え、選んだページの画面を組み立てる。

use std::collections::BTreeMap;

use crate::code::{self, Area, Screen};
use crate::group::{self, Assembler, DataGroup, Event};
use crate::packet::{Packet, Service};

/// データヘッダパラメタ
pub const HEADER_PROGRAM: u8 = 0x20;
pub const HEADER_PAGE: u8 = 0x21;
/// データユニットパラメタ
pub const UNIT_BODY: u8 = 0x20;
pub const UNIT_HEADER_TEXT: u8 = 0x24;

/// 送るページ
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageSpec {
    /// ページ番号 (1〜99)
    pub number: u8,
    /// ヘッダ文 (簡易マークアップ、[`code::encode`])
    pub header: String,
    /// 本文
    pub body: String,
}

/// 送る番組
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramSpec {
    /// マガジン番号 (0〜9)
    pub magazine: u8,
    /// 番組番号 (0〜999。000 は総目次、999 は字幕)
    pub number: u16,
    /// スーパー (ラスタを透明にしてテレビ映像に重ねる)
    pub superimpose: bool,
    pub pages: Vec<PageSpec>,
}

fn bcd3(n: u16) -> [u8; 3] {
    let n = n.min(999);
    [(n / 100) as u8, (n / 10 % 10) as u8, (n % 10) as u8]
}

fn header(param: u8, data: [u8; 9]) -> Vec<u8> {
    let mut h = vec![group::RS, param, 9];
    h.extend_from_slice(&data);
    h
}

/// 番組データヘッダ (別表第七号)。番組形態は反復形・非一括形・独立形、提示デバイスはディスプレー、手動進行
fn program_header(p: &ProgramSpec, total_blocks: usize) -> Vec<u8> {
    let [a, b, c] = bcd3(p.number);
    header(
        HEADER_PROGRAM,
        [
            (p.magazine % 10) << 4 | a,
            b << 4 | c,
            p.pages.len().min(100) as u8,
            0,
            1,
            0,
            0,
            (total_blocks >> 8) as u8,
            total_blocks as u8,
        ],
    )
}

/// ページデータヘッダ (別表第八号)
fn page_header(p: &ProgramSpec, page: u8) -> Vec<u8> {
    let [a, b, c] = bcd3(p.number);
    let page = page.clamp(1, 99);
    header(
        HEADER_PAGE,
        [
            (p.magazine % 10) << 4 | a,
            b << 4 | c,
            ((page / 10) << 4) | (page % 10),
            0,
            1,
            // 表示モード制御: b5 スーパー
            (p.superimpose as u8) << 4,
            // 提示更新制御: b1 表示書換
            1,
            // 初期提示制御: 標準密度の横書
            0,
            // ヘッダラスタ色・ラスタ色: 変更しない (8)
            0x88,
        ],
    )
}

fn page_group(p: &ProgramSpec, page: &PageSpec, last: bool) -> DataGroup {
    DataGroup {
        id: group::GROUP_PROGRAM,
        header: page_header(p, page.number),
        units: vec![
            group::data_unit(UNIT_HEADER_TEXT, &code::encode(&page.header)),
            group::data_unit(UNIT_BODY, &code::encode(&page.body)),
        ],
        end: if last { group::EOT } else { group::ETX },
    }
}

/// 送出 1 周ぶんのパケット (送出第1モード)。連続性指標は呼び出し側が送出順に振る。
/// `packets_per_second` は 1 秒あたりに送れるパケット数 (1 ライン対なら毎秒のフィールド数)
pub fn cycle(p: &ProgramSpec, packets_per_second: usize) -> Vec<Packet> {
    let mut pages = p.pages.clone();
    pages.sort_by_key(|q| q.number);
    let groups: Vec<DataGroup> =
        pages.iter().enumerate().map(|(i, q)| page_group(p, q, i + 1 == pages.len())).collect();
    let page_blocks: Vec<_> = groups.iter().map(DataGroup::blocks).collect();
    let total = 1 + page_blocks.iter().map(Vec::len).sum::<usize>();
    let program = DataGroup { id: group::GROUP_PROGRAM, header: program_header(p, total), units: vec![], end: group::ETX };

    let mut out = Vec::new();
    let empty = Packet::empty(Service::Mode1, 0);
    for (i, blocks) in page_blocks.iter().enumerate() {
        let mut unit = if i == 0 { program.blocks() } else { Vec::new() };
        unit.extend_from_slice(blocks);
        let n = unit.len();
        for (k, (data, start)) in unit.into_iter().enumerate() {
            out.push(Packet { transmission_start: k == 0, crc_section_start: start, data, ..empty.clone() });
        }
        // 次のページまでの間隔 (送出時間 × 3 + 1 秒) は空のパケット
        out.extend(std::iter::repeat_n(empty.clone(), 3 * n + packets_per_second));
    }
    if out.is_empty() {
        out.push(empty);
    }
    out
}

// ---------------------------------------------------------------- 受信

/// 受信したページ
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Page {
    superimpose: bool,
    header_raster: u8,
    raster: u8,
    header_text: Vec<u8>,
    body: Vec<u8>,
}

/// 受信の統計
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// 受け取ったパケット (選んだ送出モード)
    pub packets: u64,
    /// 誤り訂正したビット数の合計
    pub corrected_bits: u64,
    /// 正しく受けたデータグループ
    pub groups: u64,
    pub crc_errors: u64,
    pub lost: u64,
}

/// 文字放送受信機 (番組選択とページの蓄積)
pub struct Receiver {
    service: Service,
    magazine: u8,
    program: u16,
    page: u8,
    asm: Assembler,
    pages: BTreeMap<u8, Page>,
    /// 番組データヘッダから読んだページ総数
    total_pages: Option<u8>,
    /// 画面の内容が変わるたびに増える
    version: u64,
    pub stats: Stats,
}

impl Default for Receiver {
    fn default() -> Self {
        Receiver {
            service: Service::Mode1,
            magazine: 1,
            program: 100,
            page: 1,
            asm: Assembler::default(),
            pages: BTreeMap::new(),
            total_pages: None,
            version: 0,
            stats: Stats::default(),
        }
    }
}

fn from_bcd(b: u8) -> u16 {
    (b >> 4).min(9) as u16 * 10 + (b & 15).min(9) as u16
}

impl Receiver {
    /// 受信状態を捨てる (チャンネル切り替え・信号の途切れ)
    pub fn reset(&mut self) {
        self.asm.reset();
        self.pages.clear();
        self.total_pages = None;
        self.stats = Stats::default();
        self.version += 1;
    }

    /// 番組とページを選ぶ。番組が変わればページの蓄積を捨てる
    pub fn select(&mut self, magazine: u8, program: u16, page: u8) {
        if (magazine, program) != (self.magazine, self.program) {
            self.magazine = magazine;
            self.program = program;
            self.pages.clear();
            self.total_pages = None;
            self.version += 1;
        }
        if page != self.page {
            self.page = page;
            self.version += 1;
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn selected(&self) -> (u8, u16, u8) {
        (self.magazine, self.program, self.page)
    }

    /// 蓄えているページ番号
    pub fn page_numbers(&self) -> Vec<u8> {
        self.pages.keys().copied().collect()
    }

    pub fn total_pages(&self) -> Option<u8> {
        self.total_pages
    }

    /// 誤り訂正済みのパケットを入れる。(パケット, 訂正したビット数)
    pub fn feed(&mut self, packet: &Packet, corrected: usize) {
        if packet.service != self.service {
            return;
        }
        self.stats.packets += 1;
        self.stats.corrected_bits += corrected as u64;
        match self.asm.feed(packet) {
            Some(Ok(g)) => {
                self.stats.groups += 1;
                self.on_group(g);
            }
            Some(Err(Event::CrcError)) => self.stats.crc_errors += 1,
            Some(Err(Event::Lost)) => self.stats.lost += 1,
            Some(Err(Event::Group)) | None => {}
        }
    }

    fn matches(&self, h: &[u8]) -> bool {
        h.len() >= 2 && h[0] >> 4 == self.magazine && from_bcd(h[0] & 15) * 100 + from_bcd(h[1]) == self.program
    }

    fn on_group(&mut self, g: group::Received) {
        if g.id != group::GROUP_PROGRAM || !self.matches(&g.header) {
            return;
        }
        match g.header_param {
            HEADER_PROGRAM if g.header.len() >= 3 => self.total_pages = Some(g.header[2]),
            HEADER_PAGE if g.header.len() >= 9 => {
                let h = &g.header;
                let number = from_bcd(h[2]) as u8;
                let mut page = Page {
                    superimpose: h[5] >> 4 & 1 == 1,
                    header_raster: h[8] >> 4,
                    raster: h[8] & 15,
                    ..Page::default()
                };
                for (param, data) in g.units {
                    match param {
                        UNIT_BODY => page.body = data,
                        UNIT_HEADER_TEXT => page.header_text = data,
                        _ => {}
                    }
                }
                if self.pages.get(&number) != Some(&page) {
                    if number == self.page {
                        self.version += 1;
                    }
                    self.pages.insert(number, page);
                }
            }
            _ => {}
        }
    }

    /// 選んだページの画面。まだ受けていなければ None
    pub fn screen(&self) -> Option<Screen> {
        let page = self.pages.get(&self.page)?;
        let mut s = Screen::new(page.superimpose);
        let raster = |c: u8| if c == code::TRANSPARENT { code::DEFAULT_RASTER } else { c };
        s.header_raster = raster(page.header_raster);
        s.raster = raster(page.raster);
        code::Decoder::new(&mut s, Area::Header).run(&page.header_text);
        code::Decoder::new(&mut s, Area::Body).run(&page.body);
        Some(s)
    }

    /// ページを待っている間の表示 (ヘッダ文表示領域に番組・ページ番号を出す。ラスタは透明)
    pub fn waiting_screen(&self) -> Screen {
        let mut s = Screen::new(true);
        let text = format!("{{yellow}}{:03}-{:02} {{white}}さがしています", self.program, self.page);
        code::Decoder::new(&mut s, Area::Header).run(&code::encode(&text));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fec;

    fn spec() -> ProgramSpec {
        ProgramSpec {
            magazine: 1,
            number: 123,
            superimpose: false,
            pages: vec![
                PageSpec { number: 2, header: "ヘッダ".into(), body: "二ページ目".into() },
                PageSpec { number: 1, header: "ヘッダ".into(), body: "{red}一ページ目\nABC".into() },
            ],
        }
    }

    /// 送出 1 周ぶんを、連続性指標を振ってデータラインの符号化・復号を通して受信する
    fn receive(rx: &mut Receiver, cyc: &[Packet]) {
        for (k, p) in cyc.iter().enumerate() {
            let p = Packet { continuity: (k % 16) as u8, ..p.clone() };
            let bits: [bool; fec::N] = p.encode();
            let (q, n) = Packet::decode(&bits).unwrap();
            rx.feed(&q, n);
        }
    }

    #[test]
    fn transmit_and_receive_pages() {
        let cyc = cycle(&spec(), 60);
        assert!(cyc[0].transmission_start && cyc[0].crc_section_start);
        let mut rx = Receiver::default();
        rx.select(1, 123, 1);
        assert!(rx.screen().is_none());
        receive(&mut rx, &cyc);
        assert_eq!(rx.stats.crc_errors, 0);
        assert_eq!(rx.total_pages(), Some(2));
        assert_eq!(rx.page_numbers(), vec![1, 2]);
        let s = rx.screen().unwrap();
        assert_eq!(s.text(), "ヘッダ\n一ページ目\nＡＢＣ");
        assert_eq!(s.raster, code::DEFAULT_RASTER);
        rx.select(1, 123, 2);
        assert!(rx.screen().unwrap().text().ends_with("二ページ目"));
        // 別の番組は受けない
        let mut other = Receiver::default();
        other.select(1, 124, 1);
        receive(&mut other, &cyc);
        assert!(other.page_numbers().is_empty());
    }

    #[test]
    fn page_interval() {
        // 1 ページ目の送出単位 (番組管理データ + ページ) の後に 3 倍 + 1 秒の空き
        let cyc = cycle(&spec(), 60);
        let second = cyc.iter().enumerate().skip(1).find(|(_, p)| p.transmission_start).unwrap().0;
        let first_len = cyc.iter().skip(1).position(|p| p.data == [0; 22]).unwrap() + 1;
        assert_eq!(second, first_len + 3 * first_len + 60);
    }
}
