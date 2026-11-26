//! 受信機の文字発生器: JIS C 6226-1983 (JIS X 0208-1983) の 16 × 16 ドット字形 (jiskan16)。
//!
//! 字形の出典とライセンスは `fonts/README.md`。

static JISKAN16: &[u8; 94 * 94 * 32] = include_bytes!("../fonts/jiskan16.bin");

/// 区点 (0x21〜0x7E の 2 バイト) の字形。16 行、各行は最上位ビットが左。字形がなければ None
pub fn glyph16(row: u8, cell: u8) -> Option<[u16; 16]> {
    if !(0x21..=0x7E).contains(&row) || !(0x21..=0x7E).contains(&cell) {
        return None;
    }
    let o = ((row - 0x21) as usize * 94 + (cell - 0x21) as usize) * 32;
    let g: [u16; 16] = std::array::from_fn(|i| u16::from_be_bytes([JISKAN16[o + 2 * i], JISKAN16[o + 2 * i + 1]]));
    // 字形のない位置はすべて 0 (全角空白 1 区 1 点も 0 だが、空白なので同じ扱いでよい)
    g.iter().any(|&r| r != 0).then_some(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_kanji_and_kana() {
        // あ (4 区 2 点)、亜 (16 区 1 点)、瑤 (84 区 4 点: 1983 年版で追加された字)
        for (r, c) in [(0x24, 0x22), (0x30, 0x21), (0x74, 0x24)] {
            assert!(glyph16(r, c).is_some(), "{r:x} {c:x}");
        }
        // 未定義の位置 (2 区 15 点) と、1990 年版で追加された 84 区 5 点
        assert!(glyph16(0x22, 0x2F).is_none());
        assert!(glyph16(0x74, 0x25).is_none());
    }
}
