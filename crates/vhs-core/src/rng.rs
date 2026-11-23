//! 高速・決定論的な乱数 (SplitMix64)。
//!
//! ライン単位で並列処理するので、(シード, フィールド, ライン, ストリーム) から
//! 独立した系列を派生させる。同じシードなら同じ劣化が再現される。

#[derive(Clone, Debug)]
pub struct Rng(u64);

#[inline(always)]
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(mix(seed ^ 0x9E37_79B9_7F4A_7C15))
    }

    pub fn derive(seed: u64, field: u64, line: u64, stream: u64) -> Self {
        let h = mix(seed ^ mix(field.wrapping_add(0x1234_5678) ^ mix(line ^ (stream << 48))));
        Self(h)
    }

    #[inline(always)]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.0)
    }

    /// [0, 1)
    #[inline(always)]
    pub fn uniform(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / (1u64 << 24) as f32)
    }

    /// 平均 0・分散 1 の近似正規乱数 (一様乱数 4 個の和)
    #[inline(always)]
    pub fn normal(&mut self) -> f32 {
        let v = self.next_u64();
        let a = (v & 0xFFFF) as f32 + ((v >> 16) & 0xFFFF) as f32 + ((v >> 32) & 0xFFFF) as f32 + (v >> 48) as f32;
        // 和の平均 = 2*65535, 分散 = 4 * 65536^2/12
        (a - 2.0 * 65535.0) * (1.0 / (65536.0 * 0.577_350_3))
    }

    /// 平均 `mean` の指数分布
    #[inline]
    pub fn exponential(&mut self, mean: f32) -> f32 {
        -mean * (1.0 - self.uniform()).max(1e-7).ln()
    }
}
