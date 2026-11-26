//! データグループ (告示 別表第二〜四号)。
//!
//! ```text
//! 最初の単位 (1 データブロック) = SOH + データグループヘッダ DB2〜DB7 + データヘッダ + 終了符号 + CRC
//! 2 番目以降の単位             = STX + データユニット + (NUL 詰め) + 終了符号 + CRC   (1 以上のデータブロック)
//! ```
//!
//! - データブロックは 22 バイト (データパケット 1 個ぶん)。単位の最初のデータブロックを運ぶパケットは
//!   「誤り検出符号化区間識別フラグ」を 1 にする。受信側はこのフラグで単位の切れ目を知る
//! - 終了符号: ページの最後は ETX (最終ページ・番組索引は EOT)、それ以外は ETB
//! - CRC-16 (G′(X) = X¹⁶ + X¹² + X⁵ + 1) は単位の開始符号から終了符号までの**送出順のビット列** (各バイト b1 から)
//!   に対する剰余で、s15 から順に送る

use crate::packet::BLOCK_BYTES;

pub const SOH: u8 = 0x01;
pub const STX: u8 = 0x02;
pub const ETX: u8 = 0x03;
pub const EOT: u8 = 0x04;
pub const ETB: u8 = 0x17;
/// 情報分離符号 (データヘッダの始まり)
pub const RS: u8 = 0x1E;
/// データユニット分離符号
pub const US: u8 = 0x1F;

/// データグループ識別符号: 番組管理データ・ページデータ
pub const GROUP_PROGRAM: u8 = 0;
/// データグループ識別符号: 番組索引データ
pub const GROUP_INDEX: u8 = 15;

type Block = [u8; BLOCK_BYTES];

/// CRC-16 の剰余 (s15..s0 を上位から並べた値)。ビットは送出順 (各バイトの b1 = LSB から)
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut r: u16 = 0;
    for &b in bytes {
        for i in 0..8 {
            let bit = (b >> i & 1) as u16;
            let fb = (r >> 15) ^ bit;
            r <<= 1;
            if fb == 1 {
                r ^= 0x1021;
            }
        }
    }
    r
}

/// CRC を送出順 (s15 が D21 の b1) の 2 バイトにする
fn crc_bytes(r: u16) -> [u8; 2] {
    let rev = r.reverse_bits();
    [(rev & 0xFF) as u8, (rev >> 8) as u8]
}

/// 単位の最後の 2 バイトが、その前までの CRC になっているか
pub fn crc_ok(unit: &[u8]) -> bool {
    unit.len() >= 3 && crc_bytes(crc16(&unit[..unit.len() - 2])) == unit[unit.len() - 2..]
}

/// データユニット (別表第十一号): US + パラメタ + リンクフラグ・データ長 (12 ビット) + データ
pub fn data_unit(param: u8, data: &[u8]) -> Vec<u8> {
    let data = &data[..data.len().min(4095)];
    let n = data.len();
    let mut out = vec![US, param, (n >> 8) as u8 & 0x7F, n as u8];
    out.extend_from_slice(data);
    out
}

/// 送出するデータグループ
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataGroup {
    /// データグループ識別符号
    pub id: u8,
    /// データヘッダ (RS + パラメタ + データ長 + データ。最初の単位に収まる 12 バイト以下)
    pub header: Vec<u8>,
    /// データユニット (それぞれ 1 つの単位にする)
    pub units: Vec<Vec<u8>>,
    /// 最後の終了符号 (ETX か EOT)
    pub end: u8,
}

impl DataGroup {
    /// データブロックの列。(ブロック, 単位の最初か)
    pub fn blocks(&self) -> Vec<(Block, bool)> {
        let mut units: Vec<Vec<u8>> = Vec::new();
        // 最初の単位: SOH DB2..DB7 + ヘッダ + 終了符号 + CRC。サイズ (DB4〜DB5) は後で埋める
        let mut first = vec![SOH, self.id << 4, 0, 0, 0, 0, 0];
        first.extend_from_slice(&self.header[..self.header.len().min(BLOCK_BYTES - 10)]);
        first.resize(BLOCK_BYTES - 3, 0);
        first.push(if self.units.is_empty() { self.end } else { ETB });
        units.push(first);
        for (k, u) in self.units.iter().enumerate() {
            let mut v = vec![STX];
            v.extend_from_slice(u);
            // 開始符号 + データ + 終了符号 + CRC がデータブロックの整数倍になるよう NUL を詰める
            let len = (v.len() + 3).div_ceil(BLOCK_BYTES) * BLOCK_BYTES;
            v.resize(len - 3, 0);
            v.push(if k + 1 == self.units.len() { self.end } else { ETB });
            units.push(v);
        }
        let total: usize = units.iter().map(|u| (u.len() + 2) / BLOCK_BYTES).sum();
        units[0][3] = (total >> 8) as u8;
        units[0][4] = total as u8;
        let mut out = Vec::with_capacity(total);
        for mut u in units {
            let c = crc_bytes(crc16(&u));
            u.extend_from_slice(&c);
            for (i, chunk) in u.chunks(BLOCK_BYTES).enumerate() {
                out.push((chunk.try_into().unwrap(), i == 0));
            }
        }
        out
    }
}

/// 受信したデータグループ
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Received {
    pub id: u8,
    /// データヘッダパラメタ (02/0 番組、02/1 ページ、02/2 番組索引、02/3 継続)
    pub header_param: u8,
    /// データヘッダデータ
    pub header: Vec<u8>,
    /// (データユニットパラメタ, データ)
    pub units: Vec<(u8, Vec<u8>)>,
}

/// 受信の結果 (統計用)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Group,
    CrcError,
    /// 連続性指標が飛んだ (パケットの抜け) ので組み立て中のものを捨てた
    Lost,
}

/// 1 つの送出モードのパケットからデータグループを組み立てる
#[derive(Default)]
pub struct Assembler {
    last_ci: Option<u8>,
    /// 組み立て中の単位の列と、データグループのブロック数
    units: Vec<Vec<u8>>,
    size: usize,
    blocks: usize,
}

impl Assembler {
    pub fn reset(&mut self) {
        *self = Assembler::default();
    }

    /// パケット 1 個 (誤り訂正済み) を入れる。データグループができたら返す
    pub fn feed(&mut self, packet: &crate::Packet) -> Option<Result<Received, Event>> {
        let ci = packet.continuity & 15;
        let lost = self.last_ci.is_some_and(|l| (l + 1) & 15 != ci);
        self.last_ci = Some(ci);
        let mut result = None;
        if lost && self.size > 0 {
            self.units.clear();
            self.size = 0;
            result = Some(Err(Event::Lost));
        }
        let d = &packet.data;
        if packet.crc_section_start && d[0] == SOH {
            self.units = vec![d.to_vec()];
            self.size = (d[3] as usize) << 8 | d[4] as usize;
            self.blocks = 1;
        } else if self.size > 0 {
            if packet.crc_section_start {
                self.units.push(Vec::new());
            }
            self.units.last_mut().unwrap().extend_from_slice(d);
            self.blocks += 1;
        } else {
            return result;
        }
        if self.size > 0 && self.blocks >= self.size {
            let units = std::mem::take(&mut self.units);
            self.size = 0;
            return Some(parse(&units).ok_or(Event::CrcError));
        }
        result
    }
}

fn parse(units: &[Vec<u8>]) -> Option<Received> {
    if !units.iter().all(|u| crc_ok(u)) {
        return None;
    }
    let first = &units[0];
    let id = first[1] >> 4;
    let (header_param, header) = if first[7] == RS {
        let n = (first[9] as usize).min(BLOCK_BYTES - 10 - 3);
        (first[8], first[10..10 + n].to_vec())
    } else {
        (0, Vec::new())
    };
    let mut out = Vec::new();
    for u in &units[1..] {
        // STX の後のデータユニットを順に読む (NUL 詰め・終了符号・CRC は読まない)
        let body = &u[1..u.len() - 3];
        let mut i = 0;
        while i + 4 <= body.len() && body[i] == US {
            let n = ((body[i + 2] & 0x7F) as usize) << 8 | body[i + 3] as usize;
            let end = (i + 4 + n).min(body.len());
            out.push((body[i + 1], body[i + 4..end].to_vec()));
            i = end;
        }
    }
    Some(Received { id, header_param, header, units: out })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{Packet, Service};

    #[test]
    fn crc_known_value() {
        // 生成多項式で割り切れる: データ + CRC 全体の剰余は 0
        let mut u = vec![STX, 0x41, 0x42, 0x43, ETX];
        let c = crc_bytes(crc16(&u));
        u.extend_from_slice(&c);
        assert!(crc_ok(&u));
        assert_eq!(crc16(&u), 0);
        u[1] ^= 4;
        assert!(!crc_ok(&u));
    }

    fn sample() -> DataGroup {
        let mut header = vec![RS, 0x21, 9];
        header.extend_from_slice(&[0x11, 0x00, 0x01, 0, 0, 1, 0, 0, 0x88]);
        DataGroup {
            id: GROUP_PROGRAM,
            header,
            units: vec![data_unit(0x24, b"hdr"), data_unit(0x20, &[0x55; 40])],
            end: ETX,
        }
    }

    #[test]
    fn blocks_layout_and_roundtrip() {
        let g = sample();
        let blocks = g.blocks();
        // 最初の単位 1 + "hdr" の単位 1 + 40 バイトの単位 (1+4+40+3 = 48 → 3)
        assert_eq!(blocks.len(), 5);
        assert_eq!(blocks[0].0[0], SOH);
        assert_eq!(blocks[0].0[4], 5);
        assert_eq!(blocks.iter().filter(|b| b.1).count(), 3);
        let mut asm = Assembler::default();
        let mut got = None;
        for (k, (b, start)) in blocks.iter().enumerate() {
            let p = Packet { data: *b, crc_section_start: *start, ..Packet::empty(Service::Mode1, k as u8) };
            if let Some(r) = asm.feed(&p) {
                got = Some(r);
            }
        }
        let r = got.unwrap().unwrap();
        assert_eq!(r.header_param, 0x21);
        assert_eq!(r.header.len(), 9);
        assert_eq!(r.units, vec![(0x24, b"hdr".to_vec()), (0x20, vec![0x55; 40])]);
    }

    #[test]
    fn lost_packet_drops_group() {
        let blocks = sample().blocks();
        let mut asm = Assembler::default();
        let mut results = Vec::new();
        for (k, (b, start)) in blocks.iter().enumerate() {
            if k == 2 {
                continue;
            }
            let p = Packet { data: *b, crc_section_start: *start, ..Packet::empty(Service::Mode1, k as u8) };
            results.extend(asm.feed(&p));
        }
        assert_eq!(results, vec![Err(Event::Lost)]);
    }
}
