//! (272,190) 短縮化差集合巡回符号 (告示 別表第一号 注 7)。
//!
//! 送出順のビット列 `bits[0..272]` を多項式 a271 X²⁷¹ + … + a0 の係数とみなす (bits[i] = a(271−i))。
//! 情報 190 ビットが a271〜a82、チェック 82 ビットが a81〜a0 (= 剰余 r81〜r0)。
//!
//! 復号は (273,191) 差集合巡回符号としての 1 段多数決論理 (直交チェック和 17 個、8 ビット以下を訂正)。
//! 短縮で消えた次数 272 の係数は 0 とみなす。

/// 生成多項式 G(X) = X⁸² + X⁷⁷ + X⁷⁶ + X⁷¹ + X⁶⁷ + X⁶⁶ + X⁵⁶ + X⁵² + X⁴⁸ + X⁴⁰ + X³⁶ + X³⁴ + X²⁴ + X²² + X¹⁸ + X¹⁰ + X⁴ + 1
pub const GENERATOR_EXPONENTS: [u32; 18] = [82, 77, 76, 71, 67, 66, 56, 52, 48, 40, 36, 34, 24, 22, 18, 10, 4, 0];

/// 符号長 (短縮後)
pub const N: usize = 272;
/// 情報ビット数
pub const K: usize = 190;
/// チェックビット数
pub const R: usize = N - K;
/// 短縮前の巡回符号の長さ (= 16² + 16 + 1)
pub const CYCLIC_N: usize = 273;
/// 多数決で必ず訂正できるビット数
pub const T: usize = 8;

/// 法 273 の完全差集合 (17 元)。台 {X^d} の多項式が双対符号に属する (符号語と直交する)。
/// GF(2¹²) の Singer 差集合から作り、G(X) の符号と直交するものを選んだ (テストで検査する)
pub const DIFFERENCE_SET: [usize; 17] = [5, 10, 20, 39, 40, 47, 78, 80, 91, 94, 103, 139, 156, 160, 182, 188, 206];

const G: u128 = {
    let mut g = 0u128;
    let mut i = 0;
    while i < GENERATOR_EXPONENTS.len() {
        g |= 1 << GENERATOR_EXPONENTS[i];
        i += 1;
    }
    g
};
const MASK: u128 = (1 << R) - 1;

/// 最上位の係数から順にビットを入れて G(X) で割った剰余
fn remainder(bits: impl IntoIterator<Item = bool>) -> u128 {
    let mut r = 0u128;
    for b in bits {
        r = (r << 1) | b as u128;
        if r >> R & 1 == 1 {
            r ^= G;
        }
    }
    r & MASK
}

/// 情報 190 ビット (送出順) からチェック 82 ビット (送出順、r81 が先) を作る
pub fn parity(info: &[bool; K]) -> [bool; R] {
    let r = remainder(info.iter().copied().chain(std::iter::repeat_n(false, R)));
    std::array::from_fn(|i| r >> (R - 1 - i) & 1 == 1)
}

/// 情報 190 ビットに符号化した 272 ビットの符号語
pub fn encode(info: &[bool; K]) -> [bool; N] {
    let p = parity(info);
    std::array::from_fn(|i| if i < K { info[i] } else { p[i - K] })
}

/// 符号語なら true (G(X) で割り切れる)
pub fn is_codeword(bits: &[bool; N]) -> bool {
    remainder(bits.iter().copied()) == 0
}

/// 復号できなかった (誤りが訂正能力を超えた)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Uncorrectable;

/// 多数決論理復号。訂正したビット数を返し、`bits` をその場で直す。
/// 8 ビット以下の誤りは必ず訂正する。訂正後も符号語にならなければ `Err` (このとき `bits` は元のまま)
pub fn decode(bits: &mut [bool; N]) -> Result<usize, Uncorrectable> {
    // 次数 j の係数。次数 272 は短縮で消えた 0
    let mut c = [false; CYCLIC_N];
    for (i, &b) in bits.iter().enumerate() {
        c[N - 1 - i] = b;
    }
    let orig = c;
    // 1 段で 8 ビット以下は直る。超えたときのため、変化がなくなるまで数回くり返す (反復多数決)
    for _ in 0..4 {
        // チェック s: 次数 {d + s} の係数の和 (符号語なら 0)
        let fail: [bool; CYCLIC_N] = std::array::from_fn(|s| {
            DIFFERENCE_SET.iter().fold(false, |acc, &d| acc ^ c[(d + s) % CYCLIC_N])
        });
        let mut changed = false;
        let mut next = c;
        for j in 0..N {
            // j を含む 17 個のチェック (s = j − d) は j について直交する
            let votes = DIFFERENCE_SET.iter().filter(|&&d| fail[(j + CYCLIC_N - d) % CYCLIC_N]).count();
            if votes > T {
                next[j] = !next[j];
                changed = true;
            }
        }
        c = next;
        if !changed {
            break;
        }
    }
    let out: [bool; N] = std::array::from_fn(|i| c[N - 1 - i]);
    if c[N] || !is_codeword(&out) {
        return Err(Uncorrectable);
    }
    let corrected = (0..N).filter(|&j| c[j] != orig[j]).count();
    *bits = out;
    Ok(corrected)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn info(&mut self) -> [bool; K] {
            std::array::from_fn(|_| self.next() & 1 == 1)
        }
    }

    fn poly_mod(mut a: Vec<bool>, m: &[bool]) -> Vec<bool> {
        // a, m は次数の昇順
        let dm = m.len() - 1;
        for i in (dm..a.len()).rev() {
            if a[i] {
                for (k, &mk) in m.iter().enumerate() {
                    a[i - dm + k] ^= mk;
                }
            }
        }
        a.truncate(dm);
        a
    }

    #[test]
    fn generator_divides_x273_plus_1() {
        let mut g = vec![false; R + 1];
        for &e in &GENERATOR_EXPONENTS {
            g[e as usize] = true;
        }
        let mut x = vec![false; CYCLIC_N + 1];
        x[0] = true;
        x[CYCLIC_N] = true;
        assert!(poly_mod(x, &g).iter().all(|&b| !b));
    }

    #[test]
    fn difference_set_is_perfect_and_dual() {
        let mut seen = [false; CYCLIC_N];
        for &a in &DIFFERENCE_SET {
            for &b in &DIFFERENCE_SET {
                if a != b {
                    let d = (a + CYCLIC_N - b) % CYCLIC_N;
                    assert!(!seen[d], "difference {d} repeats");
                    seen[d] = true;
                }
            }
        }
        assert_eq!(seen.iter().filter(|&&s| s).count(), CYCLIC_N - 1);
        // 生成多項式の巡回シフト (= 符号語) すべてと直交する
        for s in 0..CYCLIC_N {
            let par = DIFFERENCE_SET
                .iter()
                .filter(|&&d| GENERATOR_EXPONENTS.contains(&(((d + CYCLIC_N - s) % CYCLIC_N) as u32)))
                .count();
            assert_eq!(par % 2, 0, "shift {s}");
        }
    }

    #[test]
    fn encoded_is_codeword_and_systematic() {
        let mut rng = Rng(1);
        for _ in 0..50 {
            let info = rng.info();
            let c = encode(&info);
            assert!(is_codeword(&c));
            assert_eq!(&c[..K], &info[..]);
        }
        // 全 0 の情報はチェックも 0、最後の情報ビットだけ 1 なら剰余は X⁸² mod G
        let mut info = [false; K];
        assert!(parity(&info).iter().all(|&b| !b));
        info[K - 1] = true;
        let p = parity(&info);
        for k in 0..R {
            let expect = GENERATOR_EXPONENTS[1..].contains(&(k as u32));
            assert_eq!(p[R - 1 - k], expect, "r{k}");
        }
    }

    #[test]
    fn corrects_up_to_8_errors() {
        let mut rng = Rng(0x1234_5678);
        for trial in 0..400 {
            let c = encode(&rng.info());
            let mut r = c;
            let n = trial % (T + 1);
            let mut flipped = Vec::new();
            while flipped.len() < n {
                let i = (rng.next() % N as u64) as usize;
                if !flipped.contains(&i) {
                    flipped.push(i);
                    r[i] = !r[i];
                }
            }
            assert_eq!(decode(&mut r), Ok(n), "trial {trial}");
            assert_eq!(r, c);
        }
    }

    #[test]
    fn heavy_errors_are_rejected_or_corrected() {
        // 訂正能力を超えた誤りで、別の符号語に化けずに検出されることが多いこと
        let mut rng = Rng(99);
        let mut wrong = 0;
        for _ in 0..200 {
            let c = encode(&rng.info());
            let mut r = c;
            for _ in 0..40 {
                let i = (rng.next() % N as u64) as usize;
                r[i] = !r[i];
            }
            if let Ok(_) = decode(&mut r) {
                if r != c {
                    wrong += 1;
                }
            }
        }
        assert!(wrong < 5, "{wrong}");
    }
}
