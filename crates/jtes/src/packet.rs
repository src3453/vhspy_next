//! データラインとデータパケットのビット配置 (告示 別表第一号)。
//!
//! ビット列はすべて**送出順**の `bool`。バイトは b1 (最下位) から送る。

use crate::fec;

/// ビット同期符号 (クロックラン、B1〜B2) の送出順
pub const CLOCK_RUN_IN: [bool; 16] = {
    let mut a = [false; 16];
    let mut i = 0;
    while i < 16 {
        a[i] = i % 2 == 0;
        i += 1;
    }
    a
};

/// バイト同期符号 (フレーミングコード、B3) の送出順 b1〜b8 = 1110 0101
pub const FRAMING_CODE: [bool; 8] = [true, true, true, false, false, true, false, true];

/// データラインのビット数 (296 = 16 + 8 + 272)
pub const LINE_BITS: usize = CLOCK_RUN_IN.len() + FRAMING_CODE.len() + fec::N;

/// データブロックのバイト数
pub const BLOCK_BYTES: usize = 22;

/// サービス識別符号 (B4)。送出モードの区分
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Service {
    /// 送出第1モード (逐次受信処理)
    Mode1,
    /// 送出第2モード (逐次受信処理)
    Mode2,
    /// 送出第3モード (記録受信処理)
    Mode3,
    /// 送出第4モード (記録受信処理)
    Mode4,
    /// 補助信号 (番組索引データ)
    Auxiliary,
    /// 運用信号 (放送局の運用)
    Operation,
}

impl Service {
    pub const ALL: [Service; 6] =
        [Service::Mode1, Service::Mode2, Service::Mode3, Service::Mode4, Service::Auxiliary, Service::Operation];

    /// 送出順 b1〜b8 (情報 4 ビット + (8,4) 拡大ハミングのチェック 4 ビット)。告示の表そのまま
    pub const fn code(self) -> [bool; 8] {
        let s = match self {
            Service::Mode1 => b"01001001",
            Service::Mode2 => b"01010100",
            Service::Mode3 => b"01100010",
            Service::Mode4 => b"01111111",
            Service::Auxiliary => b"11101100",
            Service::Operation => b"11110001",
        };
        let mut a = [false; 8];
        let mut i = 0;
        while i < 8 {
            a[i] = s[i] == b'1';
            i += 1;
        }
        a
    }

    /// 受信した 8 ビットに最も近い符号 (1 ビットの誤りまで)
    pub fn decode(bits: &[bool]) -> Option<Service> {
        Service::ALL.into_iter().find(|s| s.code().iter().zip(bits).filter(|(a, b)| a != b).count() <= 1)
    }
}

/// データパケット (272 ビット) の中身
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub service: Service,
    /// 連続性指標 (0〜15 を巡回。送出単位内で連続)
    pub continuity: u8,
    /// 伝送制御フラグ (送出単位の最初のパケット)
    pub transmission_start: bool,
    /// 誤り検出符号化区間識別フラグ (SOH か STX を含むパケット)
    pub crc_section_start: bool,
    /// データブロック D1〜D22
    pub data: [u8; BLOCK_BYTES],
}

impl Packet {
    /// 中身のない (全 NUL の) パケット
    pub fn empty(service: Service, continuity: u8) -> Self {
        Packet { service, continuity, transmission_start: false, crc_section_start: false, data: [0; BLOCK_BYTES] }
    }

    /// 情報ビット 190 個 (B4 b1 〜 B27 b6、送出順)
    pub fn info_bits(&self) -> [bool; fec::K] {
        let mut out = [false; fec::K];
        let mut w = BitWriter { out: &mut out, at: 0 };
        w.bits(&self.service.code());
        w.lsb(self.continuity as u32 & 0xF, 4);
        w.bit(self.transmission_start);
        w.bit(self.crc_section_start);
        for &b in &self.data {
            w.lsb(b as u32, 8);
        }
        debug_assert_eq!(w.at, fec::K);
        out
    }

    /// チェック符号を付けた 272 ビット
    pub fn encode(&self) -> [bool; fec::N] {
        fec::encode(&self.info_bits())
    }

    /// 誤り訂正済みの 272 ビットからパケットを読む。サービス識別が読めなければ None
    pub fn parse(bits: &[bool; fec::N]) -> Option<Packet> {
        let service = Service::decode(&bits[0..8])?;
        let lsb = |at: usize, n: usize| (0..n).fold(0u32, |acc, i| acc | (bits[at + i] as u32) << i);
        Some(Packet {
            service,
            continuity: lsb(8, 4) as u8,
            transmission_start: bits[12],
            crc_section_start: bits[13],
            data: std::array::from_fn(|i| lsb(14 + 8 * i, 8) as u8),
        })
    }

    /// 受信した 272 ビットを誤り訂正して読む。(パケット, 訂正したビット数)
    pub fn decode(bits: &[bool; fec::N]) -> Option<(Packet, usize)> {
        let mut b = *bits;
        let n = fec::decode(&mut b).ok()?;
        Some((Packet::parse(&b)?, n))
    }
}

struct BitWriter<'a> {
    out: &'a mut [bool],
    at: usize,
}

impl BitWriter<'_> {
    fn bit(&mut self, b: bool) {
        self.out[self.at] = b;
        self.at += 1;
    }
    fn bits(&mut self, bs: &[bool]) {
        for &b in bs {
            self.bit(b);
        }
    }
    fn lsb(&mut self, v: u32, n: usize) {
        for i in 0..n {
            self.bit(v >> i & 1 == 1);
        }
    }
}

/// データライン (296 ビット) の組み立て
pub struct DataLine;

impl DataLine {
    /// クロックラン + フレーミングコード + 符号化済みパケット
    pub fn bits(packet: &Packet) -> [bool; LINE_BITS] {
        Self::from_packet_bits(&packet.encode())
    }

    /// 符号化済みの 272 ビットからデータラインにする
    pub fn from_packet_bits(packet: &[bool; fec::N]) -> [bool; LINE_BITS] {
        let mut out = [false; LINE_BITS];
        out[..16].copy_from_slice(&CLOCK_RUN_IN);
        out[16..24].copy_from_slice(&FRAMING_CODE);
        out[24..].copy_from_slice(packet);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_codes_are_distinct_by_3_or_more() {
        for a in Service::ALL {
            assert_eq!(Service::decode(&a.code()), Some(a));
            for b in Service::ALL {
                if a != b {
                    let d = a.code().iter().zip(b.code()).filter(|(x, y)| **x != *y).count();
                    assert!(d >= 3, "{a:?} {b:?} {d}");
                }
            }
            // 1 ビットの誤りは直る
            for i in 0..8 {
                let mut c = a.code();
                c[i] = !c[i];
                assert_eq!(Service::decode(&c), Some(a));
            }
        }
    }

    #[test]
    fn bit_layout() {
        let mut p = Packet::empty(Service::Mode3, 0b1010);
        p.transmission_start = true;
        p.data[0] = 0x01;
        p.data[21] = 0x80;
        let line = DataLine::bits(&p);
        assert_eq!(line.len(), 296);
        // B1〜B2 クロックラン、B3 フレーミングコード
        let byte = |k: usize| (0..8).fold(0u8, |acc, i| acc | (line[8 * k + i] as u8) << i);
        assert_eq!([byte(0), byte(1), byte(2)], [0x55, 0x55, 0xA7]);
        // B4 サービス識別 (送出順 01100010)
        assert_eq!(&line[24..32], &Service::Mode3.code());
        // B5 b1〜b4 = CI (LSB 先)、b5 伝送制御、b6 誤り検出区間
        assert_eq!(&line[32..38], &[false, true, false, true, true, false]);
        // D1 は B5 b7 から、D22 の b8 は B27 b6
        assert!(line[38]);
        assert!(!line[39]);
        assert!(line[8 * 26 + 5]);
        assert!(fec::is_codeword(line[24..].try_into().unwrap()));
    }

    #[test]
    fn roundtrip() {
        let mut p = Packet::empty(Service::Auxiliary, 7);
        p.crc_section_start = true;
        p.data = std::array::from_fn(|i| (i * 37 + 1) as u8);
        let mut bits = p.encode();
        bits[3] = !bits[3];
        bits[100] = !bits[100];
        bits[271] = !bits[271];
        assert_eq!(Packet::decode(&bits), Some((p, 3)));
    }
}
