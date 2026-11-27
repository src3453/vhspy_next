//! 縦方向 SIMD (ラインを SIMD のレーンに並べる) の共通部品。
//!
//! 信号処理の大半はラインごとに独立した逐次 IIR なので、8 本のラインを 8 レーンに並べて同じ演算を同時に回す。
//! 各レーンはスカラー版と同じ演算を同じ順序で行う (FMA にしない・横方向の総和を取らない) ので、
//! 出力はスカラー版とビット単位で一致する。
//!
//! - [`F32x8`] は `[f32; 8]` を包んだだけの型で、演算はレーンごとのループとして書く。
//!   `#[target_feature(enable = "avx2")]` を付けた関数の中へインライン展開されると 256bit 命令 1 個になり、
//!   aarch64 では NEON 2 レジスタになる
//! - ホットループは [`x8_kernel!`] で AVX2 版にコンパイルし、[`level`] で実行時に選ぶ。
//!   ワークスペース全体には `target-cpu` を掛けない (AVX2 のない CPU でも配布物が動くように)
//! - 環境変数 `VHS_SIMD=scalar` で従来のスカラー経路を強制できる (比較・検証用)

use std::ops::{Add, Mul, Neg, Sub};
use std::sync::OnceLock;

/// 縦方向 SIMD で束ねるライン数
pub const LANES: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C, align(32))]
pub struct F32x8(pub [f32; LANES]);

impl F32x8 {
    #[inline(always)]
    pub fn splat(v: f32) -> Self {
        Self([v; LANES])
    }

    #[inline(always)]
    pub fn from_fn(f: impl FnMut(usize) -> f32) -> Self {
        Self(std::array::from_fn(f))
    }

    /// 連続する 8 個を読む
    #[inline(always)]
    pub fn load(s: &[f32]) -> Self {
        Self(s[..LANES].try_into().unwrap())
    }

    /// 8 本のラインの同じ位置 `n` を 1 つに束ねる
    #[inline(always)]
    pub fn gather(lines: &[&[f32]; LANES], n: usize) -> Self {
        Self::from_fn(|l| lines[l][n])
    }

    /// 8 本のラインの同じ位置 `n` へ書き込む
    #[inline(always)]
    pub fn scatter(self, lines: &mut [&mut [f32]; LANES], n: usize) {
        for l in 0..LANES {
            lines[l][n] = self.0[l];
        }
    }
}

/// 表の線形補間 `t[i] + (t[i + 1] - t[i]) · (pos - i)`、`i = (pos as usize).min(max_i)` をレーンごとに行う
/// (スカラーの同じ式とビット単位で一致する。NaN・負・巨大な pos の添字の扱いも `as usize` と同じ)。
/// x86_64 では AVX2 の gather を使うので、[`x8`] が真の経路 ([`x8_kernel!`] の中) からだけ呼ぶこと
#[inline(always)]
pub fn table_lerp(table: &[f32], pos: F32x8, max_i: usize) -> F32x8 {
    assert!(max_i + 1 < table.len() && max_i < i32::MAX as usize);
    #[cfg(target_arch = "x86_64")]
    // SAFETY: 添字は 0..=max_i に収め、max_i + 1 は表の中。AVX2 は呼び出し側が保証する
    unsafe {
        use std::arch::x86_64::*;
        let p = _mm256_loadu_ps(pos.0.as_ptr());
        let mut j = _mm256_cvttps_epi32(p);
        // 2^31 以上は cvttps が INT_MIN を返すが、as usize では max_i に張り付く
        let big = _mm256_castps_si256(_mm256_cmp_ps::<_CMP_GE_OQ>(p, _mm256_set1_ps(2147483648.0)));
        let maxv = _mm256_set1_epi32(max_i as i32);
        j = _mm256_blendv_epi8(j, maxv, big);
        // 負・NaN は 0
        j = _mm256_min_epi32(_mm256_max_epi32(j, _mm256_setzero_si256()), maxv);
        let fr = _mm256_sub_ps(p, _mm256_cvtepi32_ps(j));
        let t0 = _mm256_i32gather_ps::<4>(table.as_ptr(), j);
        let t1 = _mm256_i32gather_ps::<4>(table.as_ptr().add(1), j);
        let r = _mm256_add_ps(t0, _mm256_mul_ps(_mm256_sub_ps(t1, t0), fr));
        let mut out = F32x8::splat(0.0);
        _mm256_storeu_ps(out.0.as_mut_ptr(), r);
        out
    }
    #[cfg(not(target_arch = "x86_64"))]
    F32x8::from_fn(|l| {
        let i = (pos.0[l] as usize).min(max_i);
        let fr = pos.0[l] - i as f32;
        table[i] + (table[i + 1] - table[i]) * fr
    })
}

/// レーンごとの `(pos as usize).min(max_i)` (NaN・負は 0、巨大な値は max_i)。
/// x86_64 では AVX2 の命令を使うので、[`x8`] が真の経路からだけ呼ぶこと
#[inline(always)]
pub fn trunc_index(pos: F32x8, max_i: usize) -> [i32; LANES] {
    assert!(max_i < i32::MAX as usize);
    #[cfg(target_arch = "x86_64")]
    // SAFETY: AVX2 は呼び出し側が保証する
    unsafe {
        use std::arch::x86_64::*;
        let p = _mm256_loadu_ps(pos.0.as_ptr());
        let mut j = _mm256_cvttps_epi32(p);
        let big = _mm256_castps_si256(_mm256_cmp_ps::<_CMP_GE_OQ>(p, _mm256_set1_ps(2147483648.0)));
        let maxv = _mm256_set1_epi32(max_i as i32);
        j = _mm256_blendv_epi8(j, maxv, big);
        j = _mm256_min_epi32(_mm256_max_epi32(j, _mm256_setzero_si256()), maxv);
        let mut out = [0i32; LANES];
        _mm256_storeu_si256(out.as_mut_ptr() as *mut __m256i, j);
        out
    }
    #[cfg(not(target_arch = "x86_64"))]
    std::array::from_fn(|l| (pos.0[l] as usize).min(max_i) as i32)
}

/// レーンごとに `table[idx[l] + off]` を読む (範囲は確かめる)。
/// x86_64 では AVX2 の gather を使うので、[`x8`] が真の経路からだけ呼ぶこと
#[inline(always)]
pub fn gather_idx(table: &[f32], idx: &[i32; LANES], off: usize) -> F32x8 {
    check_idx(table, idx, off + 1);
    #[cfg(target_arch = "x86_64")]
    // SAFETY: 添字は上で確かめた。AVX2 は呼び出し側が保証する
    unsafe {
        use std::arch::x86_64::*;
        let i = _mm256_loadu_si256(idx.as_ptr() as *const __m256i);
        let v = _mm256_i32gather_ps::<4>(table.as_ptr().add(off), i);
        let mut out = F32x8::splat(0.0);
        _mm256_storeu_ps(out.0.as_mut_ptr(), v);
        out
    }
    #[cfg(not(target_arch = "x86_64"))]
    F32x8::from_fn(|l| table[idx[l] as usize + off])
}

/// 添字がどれも 0 以上で、`idx + n` までが表に収まるか確かめる
#[inline(always)]
fn check_idx(table: &[f32], idx: &[i32; LANES], n: usize) {
    let (lo, hi) = idx.iter().fold((i32::MAX, i32::MIN), |(a, b), &i| (a.min(i), b.max(i)));
    assert!(lo >= 0 && hi as usize + n <= table.len() && table.len() < i32::MAX as usize);
}

/// レーンごとに連続する 4 個 `table[idx[l]..idx[l] + 4]` を読み、k 番目どうしを束ねた 4 つを返す (範囲は確かめる)。
/// x86_64 では AVX2 の命令を使うので、[`x8`] が真の経路からだけ呼ぶこと
#[inline(always)]
pub fn gather4(table: &[f32], idx: &[i32; LANES]) -> [F32x8; 4] {
    check_idx(table, idx, 4);
    #[cfg(target_arch = "x86_64")]
    // SAFETY: 添字は上で確かめた。AVX2 は呼び出し側が保証する
    unsafe {
        use std::arch::x86_64::*;
        let p = table.as_ptr();
        // 下半分にレーン k、上半分にレーン k + 4 の 4 個を置き、128bit ごとに 4×4 を転置する
        let row = |k: usize| {
            _mm256_set_m128(_mm_loadu_ps(p.add(idx[k + 4] as usize)), _mm_loadu_ps(p.add(idx[k] as usize)))
        };
        let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
        let (t0, t1) = (_mm256_unpacklo_ps(r0, r1), _mm256_unpackhi_ps(r0, r1));
        let (t2, t3) = (_mm256_unpacklo_ps(r2, r3), _mm256_unpackhi_ps(r2, r3));
        let c = [
            _mm256_shuffle_ps::<0x44>(t0, t2),
            _mm256_shuffle_ps::<0xEE>(t0, t2),
            _mm256_shuffle_ps::<0x44>(t1, t3),
            _mm256_shuffle_ps::<0xEE>(t1, t3),
        ];
        let mut out = [F32x8::splat(0.0); 4];
        for k in 0..4 {
            _mm256_storeu_ps(out[k].0.as_mut_ptr(), c[k]);
        }
        out
    }
    #[cfg(not(target_arch = "x86_64"))]
    std::array::from_fn(|k| F32x8::from_fn(|l| table[idx[l] as usize + k]))
}

/// `buf` にライン長 `ls` おきに並んだ 8 本のライン (長さはちょうど `ls`)
#[inline(always)]
pub fn rows(buf: &[f32], ls: usize) -> [&[f32]; LANES] {
    std::array::from_fn(|l| &buf[l * ls..][..ls])
}

/// [`rows`] の書き込み用
#[inline(always)]
pub fn rows_mut(buf: &mut [f32], ls: usize) -> [&mut [f32]; LANES] {
    let mut it = buf[..LANES * ls].chunks_exact_mut(ls);
    std::array::from_fn(|_| it.next().unwrap())
}

macro_rules! lanewise {
    ($tr:ident, $f:ident, $op:tt) => {
        impl $tr for F32x8 {
            type Output = Self;
            #[inline(always)]
            fn $f(self, o: Self) -> Self {
                let mut r = self.0;
                for l in 0..LANES {
                    r[l] = r[l] $op o.0[l];
                }
                Self(r)
            }
        }
    };
}
lanewise!(Add, add, +);
lanewise!(Sub, sub, -);
lanewise!(Mul, mul, *);

impl Neg for F32x8 {
    type Output = Self;
    #[inline(always)]
    fn neg(self) -> Self {
        Self(self.0.map(|v| -v))
    }
}

/// スカラー (`f32`) と [`F32x8`] に共通の演算。フィルタをレーン型でジェネリックに書くために使う
pub trait Lane: Copy + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> + Neg<Output = Self> {
    fn splat(v: f32) -> Self;
}

impl Lane for f32 {
    #[inline(always)]
    fn splat(v: f32) -> Self {
        v
    }
}

impl Lane for F32x8 {
    #[inline(always)]
    fn splat(v: f32) -> Self {
        F32x8::splat(v)
    }
}

/// 実行時に選んだ経路
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// 従来のスカラー経路 (AVX2 のない x86_64、または `VHS_SIMD=scalar`)
    Scalar,
    /// 8 レーン (x86_64 は AVX2、aarch64 は NEON)
    X8,
}

/// 今の CPU で使う経路 (最初の呼び出しで決めて保持する)
pub fn level() -> Level {
    static LEVEL: OnceLock<Level> = OnceLock::new();
    *LEVEL.get_or_init(|| {
        let forced = std::env::var("VHS_SIMD").ok().map(|s| s.to_ascii_lowercase());
        if matches!(forced.as_deref(), Some("scalar" | "off" | "0")) {
            return Level::Scalar;
        }
        if x8_supported() { Level::X8 } else { Level::Scalar }
    })
}

#[inline]
pub fn x8() -> bool {
    level() == Level::X8
}

fn x8_supported() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2")
    }
    #[cfg(target_arch = "aarch64")]
    {
        true
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        false
    }
}

/// 8 レーンのホットループを定義する。x86_64 では本体を AVX2 を有効にした関数としてコンパイルし、
/// それ以外ではそのまま呼ぶ (aarch64 は NEON が必須なので常に有効)。
/// 呼び出し側は [`x8`] が真のときだけ呼ぶこと
macro_rules! x8_kernel {
    ($(#[$m:meta])* $vis:vis fn $name:ident $(<$($lt:lifetime),*>)? ($($arg:ident : $ty:ty),* $(,)?) $(-> $ret:ty)? $body:block) => {
        $(#[$m])*
        $vis fn $name $(<$($lt),*>)? ($($arg: $ty),*) $(-> $ret)? {
            #[inline(always)]
            fn inner $(<$($lt),*>)? ($($arg: $ty),*) $(-> $ret)? $body
            #[cfg(target_arch = "x86_64")]
            {
                #[target_feature(enable = "avx2")]
                fn avx2 $(<$($lt),*>)? ($($arg: $ty),*) $(-> $ret)? {
                    inner($($arg),*)
                }
                debug_assert!($crate::simd::x8());
                // SAFETY: 呼び出し側は simd::x8() (= AVX2 を検出済み) のときだけここへ来る
                unsafe { avx2($($arg),*) }
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                inner($($arg),*)
            }
        }
    };
}
pub(crate) use x8_kernel;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::Cascade;

    x8_kernel! {
        fn run_x8(f: &Cascade, lines: &[Vec<f32>], out: &mut [f32], ls: usize) {
            let mut v = f.lanes::<F32x8>();
            let refs: [&[f32]; LANES] = std::array::from_fn(|l| lines[l].as_slice());
            let mut out = rows_mut(out, ls);
            for n in 0..ls {
                v.process(F32x8::gather(&refs, n)).scatter(&mut out, n);
            }
        }
    }

    x8_kernel! {
        fn lerp_x8(table: &[f32], pos: F32x8, max_i: usize) -> F32x8 {
            table_lerp(table, pos, max_i)
        }
    }

    /// 表引きは `as usize` の添字の扱い (負・NaN・巨大・無限大) も含めてスカラーの式と一致する
    #[test]
    fn table_lerp_matches_scalar() {
        if !x8() {
            return;
        }
        let table: Vec<f32> = (0..=100).map(|i| (i as f32 * 0.37).sin()).collect();
        let max_i = 99;
        let cases = [
            0.0, 0.5, 1.25, 50.75, 98.9, 99.0, 99.5, 120.0, -0.5, -3.0, f32::NAN, f32::INFINITY, -f32::INFINITY, 3e9, 2147483648.0,
            1e-30,
        ];
        for chunk in cases.chunks(LANES) {
            let mut p = [0f32; LANES];
            p[..chunk.len()].copy_from_slice(chunk);
            let got = lerp_x8(&table, F32x8(p), max_i);
            for l in 0..LANES {
                let i = (p[l] as usize).min(max_i);
                let want = table[i] + (table[i + 1] - table[i]) * (p[l] - i as f32);
                assert_eq!(got.0[l].to_bits(), want.to_bits(), "pos {}", p[l]);
            }
        }
    }

    x8_kernel! {
        fn gather4_x8(table: &[f32], idx: &[i32; LANES]) -> [F32x8; 4] {
            gather4(table, idx)
        }
    }

    #[test]
    fn gather4_reads_four_per_lane() {
        if !x8() {
            return;
        }
        let table: Vec<f32> = (0..100).map(|i| i as f32 * 1.5).collect();
        let idx = [0, 7, 3, 96, 50, 1, 42, 13];
        let g = gather4_x8(&table, &idx);
        for k in 0..4 {
            for l in 0..LANES {
                assert_eq!(g[k].0[l], table[idx[l] as usize + k]);
            }
        }
    }

    /// 8 レーン版の縦続 biquad はラインごとのスカラー版とビット単位で一致する
    #[test]
    fn cascade_x8_matches_scalar() {
        if !x8() {
            return;
        }
        let fs = crate::timing::NTSC.fs;
        let f = Cascade::butter_lowpass_at(4, 1.3e6, fs);
        let ls = 1000;
        let lines: Vec<Vec<f32>> =
            (0..LANES).map(|l| (0..ls).map(|n| ((n * (l + 3)) as f32 * 0.37).sin() * (l as f32 + 0.5)).collect()).collect();
        let mut out = vec![0f32; LANES * ls];
        run_x8(&f, &lines, &mut out, ls);
        for (l, line) in lines.iter().enumerate() {
            let mut s = f;
            for n in 0..ls {
                assert_eq!(s.process(line[n]).to_bits(), out[l * ls + n].to_bits(), "lane {l} n {n}");
            }
        }
    }
}
