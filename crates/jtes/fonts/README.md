# jiskan16.bin

JIS X 0208-1983 (= JIS C 6226-1983) の 16 × 16 ドット字形。文字放送の漢字系集合と同じ版。

- 出典: GNU intlfonts 1.4.2 (`https://ftp.gnu.org/gnu/intlfonts/intlfonts-1.4.2.tar.gz`) の `Japanese.X/jiskan16.bdf`
  (X11 付属。JIS X 9051-1984「表示装置用 16 ドット字形」由来、`COPYRIGHT "from JIS X 9051-1984, by permission to use"`。
  intlfonts の README では "Public Domain or ... freely usable")
- 形式: 区 1〜94 × 点 1〜94 の順に 1 字 32 バイト (16 行 × 2 バイト、各行は上位ビットが左)。字形のない位置は 0。
  BDF の BBX はすべて `16 16 0 -2` なので、行 0 が字形の上端、行 13 が基線の上の行
