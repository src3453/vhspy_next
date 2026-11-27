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
    ($(#[$m:meta])* $vis:vis fn $name:ident $(<$($lt:lifetime),*>)? ($($arg:ident : $ty:ty),* $(,)?) $body:block) => {
        $(#[$m])*
        $vis fn $name $(<$($lt),*>)? ($($arg: $ty),*) {
            #[inline(always)]
            fn inner $(<$($lt),*>)? ($($arg: $ty),*) $body
            #[cfg(target_arch = "x86_64")]
            {
                #[target_feature(enable = "avx2")]
                fn avx2 $(<$($lt),*>)? ($($arg: $ty),*) {
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
