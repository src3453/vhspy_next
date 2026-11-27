//! 輝度の FM 記録・再生を実際に計算するモデル。
//!
//! 1 ラインごとに次の順で処理する (2 倍にオーバーサンプリング)。
//!
//! 1. オーバーサンプリング: Hi8 (白 7.7 MHz) でも搬送波と側波帯がナイキスト周波数に収まるようにする。
//!    J-A では記録 (変調と磁性体) だけを 4〜16 倍 ([`FmConfig::ja_oversample`]) で計算し、ローパスを掛けて
//!    2 倍に間引いてから再生側に渡す。J-A で飽和した搬送波は方形波に近く高調波が多いので、低いレートで解くと
//!    折り返しが復調出力の揺れになる (平坦な画で 4 倍は 1〜2 IRE、8 倍は 0.2〜0.4 IRE、16 倍は 0.2 IRE 以下)
//! 2. 周波数変調: シンクチップ〜ホワイトピークを各フォーマットの搬送波周波数に割り当てる。
//!    エンファシスで白のエッジが行き過ぎると、ホワイトピークより高い周波数まで振れる
//! 3. 低域変換クロマの重畳と磁性体の伝達特性: 実機では色は FM 信号をバイアス代わりに同じトラックへ
//!    記録される。和の瞬時値が磁性体で飽和すると、搬送波とクロマの相互変調積が搬送波の近くに立ち
//!    (伝送路の傾きで AM が PM に変わり)、輝度のモアレになる
//! 4. 再生の伝送路: ヘッド・テープの高域損失 (ローパス)、クロマを分けるハイパス、RF ノイズ。
//!    RF レベル (トラッキング・しわ・ドロップアウト) は搬送波の振幅として効く。
//!    ローパスは再生 EQ で補った後の正味の特性で、EQ が想定していない余分なスペーシング (ヘッドの汚れ・
//!    テープの浮き) は [`super::head`] のスペーシング損失 e^{-kd} として記録信号にだけ掛ける (ノイズは後から足す)
//! 5. リミッタ + 周期計測型の復調: ゼロ交差の間隔から瞬時周波数を求めて輝度に戻す。
//!    高い周波数ほど伝送路で搬送波が弱まり、下側波帯が勝つとゼロ交差を見落として周波数が落ちる
//!    (白のエッジの右に黒い筋 = ベアディング)。搬送波が消えるとノイズのゼロ交差を拾い、白黒の砂嵐になる

use crate::filter::Cascade;
use crate::simd::LANES;
use super::magnetic::MagneticTransfer;
use crate::rng::Rng;
use crate::timing::{Levels, MAX_LINE_SAMPLES};

/// オーバーサンプリング倍率
pub const OVERSAMPLE: usize = 2;
/// ライン先頭の前に流す助走 (元のレートのサンプル数)。フィルタと復調器を落ち着かせる
const PRE_ROLL: usize = 48;
/// J-A の記録を計算するオーバーサンプリング倍率の上限
pub const MAX_JA_OVERSAMPLE: usize = 16;
/// 再生側 (伝送路・復調) のバッファの長さ
const MAX_LEN: usize = (MAX_LINE_SAMPLES + PRE_ROLL) * OVERSAMPLE;
/// スペーシング損失の FIR の片側の最大タップ数
const SPACING_HALF_MAX: usize = 160;

/// FM 伝送路の設定 (フィールドごとに作る)
#[derive(Clone)]
pub struct FmChannel {
    /// 記録を計算するオーバーサンプリング倍率 (J-A は 4〜16 倍、それ以外は 2 倍)
    oversample: usize,
    /// 記録を再生側のレート (2 倍) に間引くときのアンチエイリアスのローパス (J-A のみ)
    decimate: Option<Cascade>,
    /// 再生側 (2 倍オーバーサンプリング) のサンプリング周波数 [Hz]
    fs2: f32,
    /// シンクチップ・ホワイトピークの搬送波周波数 [Hz]
    f_sync: f32,
    f_white: f32,
    lv: Levels,
    rf_lp: Cascade,
    rf_hp: Cascade,
    post_lp: Cascade,
    /// RF ノイズ (搬送波振幅 1 に対する rms、伝送路の帯域で制限する前の値)
    noise: f32,
    /// 低域変換クロマの周波数 [Hz] と、振幅の倍率 [1/IRE]
    cu_freq: f32,
    cu_gain: f32,
    /// 磁性体の伝達特性 (搬送波の振幅 = 1 の尺度)
    mag: MagneticTransfer,
    /// 伝送路と復調後のフィルタの遅れ [オーバーサンプリング後のサンプル]。出力を前にずらして打ち消す
    delay: f32,
    /// 余分なスペーシングの損失 (零位相 FIR、長さ 2 * spacing_half + 1)。spacing_half = 0 なら掛けない
    spacing: [f32; 2 * SPACING_HALF_MAX + 1],
    spacing_half: usize,
    /// 2π / 書き込み速度 [s/m]。周波数 f、スペーシング d の損失は e^{-spacing_k f d}
    spacing_k: f32,
}

/// `x[m-1]` と `x[m]` の間のゼロ交差の位置 (0..1)。
///
/// 線形補間や 3 次補間では、1 周期 7〜8 サンプルの正弦波の曲がりで交差位置が搬送波の位相に応じてずれ、
/// 復調出力に細かい揺れが残る。帯域制限した補間 (Lanczos 窓付き sinc、16 タップ) の値ではさみうち法で詰める
fn cross_fraction(x: &[f32], m: usize) -> f32 {
    let (p1, p2) = (x[m - 1], x[m]);
    let base = (m - 1) as isize;
    let eval = |t: f32| lanczos_at(x, base, t);
    // 線形補間の推定から始めて、交差を挟んだまま 2 回詰める (はさみうち法)
    let (mut t0, mut f0) = (0.0f32, p1);
    let (mut t1, mut f1) = (1.0f32, p2);
    let mut t = p1 / (p1 - p2);
    for _ in 0..2 {
        let f = eval(t);
        if (f < 0.0) == (f0 < 0.0) {
            (t0, f0) = (t, f);
        } else {
            (t1, f1) = (t, f);
        }
        let d = f1 - f0;
        if d.abs() < 1e-12 {
            break;
        }
        t = t0 - f0 * (t1 - t0) / d;
    }
    t.clamp(0.0, 1.0)
}

crate::simd::x8_kernel! {
    /// [`FmChannel::process_line`] の変調 (無記憶の磁性体・2 倍オーバーサンプリング) を段に分けた版。
    /// 補間・周波数・cos の表引き・磁性体はサンプルごとに独立なのでベクトル化し、搬送波と低域変換クロマの
    /// 位相の積算だけを逐次に回す。各サンプルの演算はもとのループと同じ順序なので、結果はビット単位で一致する
    #[allow(clippy::too_many_arguments)]
    fn modulate_memoryless_x8<'a>(
        ch: &FmChannel,
        luma: &[f32],
        cu: Option<(&'a [f32], &'a [f32])>,
        amp: f32,
        line_spacing: f32,
        drop: &[super::Dropout],
        phase0: f32,
        cu_phase0: f32,
        rf: &mut [f32],
    ) {
        use crate::simd::{F32x8, LANES, table_lerp};
        use std::f32::consts::{FRAC_PI_2, TAU};
        const OS: usize = OVERSAMPLE;
        let n = luma.len();
        let len = rf.len();
        let n_src = n + PRE_ROLL;
        debug_assert_eq!(len, n_src * OS);
        let pre = PRE_ROLL * OS;

        // 元のレートの値 (助走はシンクチップ、先頭の 1 つ前と末尾の先はクランプ) を at(i - 1) から並べる
        let mut pad = [0f32; MAX_LINE_SAMPLES + PRE_ROLL + 3];
        for (k, p) in pad[..n_src + 3].iter_mut().enumerate() {
            let j = k as isize - 1 - PRE_ROLL as isize;
            *p = if j < 0 { ch.lv.sync } else { luma[(j as usize).min(n - 1)] };
        }
        // 1. 補間 (Catmull-Rom、frac = 0.5) と瞬時周波数
        let mut f = [0f32; MAX_LEN];
        let f = &mut f[..len];
        let frac = 1.0 / OS as f32;
        let (t2, t3) = (frac * frac, frac * frac * frac);
        for (i, fo) in f.chunks_exact_mut(OS).enumerate() {
            let (p0, p1, p2, p3) = (pad[i], pad[i + 1], pad[i + 2], pad[i + 3]);
            fo[0] = ch.freq_of(p1);
            let v = 0.5
                * ((2.0 * p1)
                    + (-p0 + p2) * frac
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                    + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3);
            fo[1] = ch.freq_of(v);
        }
        // 2. 位相の積算 (逐次)
        let fs_gen = ch.fs2;
        let (dphi_k, dcu) = (TAU / fs_gen, TAU * ch.cu_freq / fs_gen);
        let mut ph = [0f32; MAX_LEN];
        let ph = &mut ph[..len];
        let mut phase = phase0;
        for (p, &fi) in ph.iter_mut().zip(f.iter()) {
            phase += dphi_k * fi;
            if phase > TAU {
                phase -= TAU;
            }
            *p = phase;
        }
        // 3. 搬送波の振幅 (スペーシング損失があるときだけ。exp は逐次のまま)
        let mut a = [amp; MAX_LEN];
        let a = &mut a[..len];
        if line_spacing > 0.0 || !drop.is_empty() {
            for (m, am) in a.iter_mut().enumerate() {
                let src = (m / OS).saturating_sub(PRE_ROLL);
                let mut spacing = line_spacing;
                if m >= pre {
                    for &(s, e, depth) in drop {
                        let d = if src < s { s - src } else if src >= e { src + 1 - e } else { 0 };
                        if d < super::DROPOUT_EDGE {
                            spacing += depth * (1.0 - d as f32 / super::DROPOUT_EDGE as f32);
                        }
                    }
                }
                if spacing > 0.0 {
                    *am *= (-ch.spacing_k * f[m].max(0.0) * spacing).exp();
                }
            }
        }
        // 4. 搬送波 (+ 低域変換クロマ) の表引き
        let ct = cos_table();
        let kpos = F32x8::splat(COS_N as f32 / TAU);
        let full = len / LANES * LANES;
        for m0 in (0..full).step_by(LANES) {
            let c = table_lerp(ct, F32x8::load(&ph[m0..]) * kpos, COS_N - 1);
            rf[m0..m0 + LANES].copy_from_slice(&c.0);
        }
        for m in full..len {
            rf[m] = fast_cos(ct, ph[m]);
        }
        if let Some((cu_i, cu_q)) = cu {
            // 低域変換クロマの位相は助走の後から回る
            let mut cph = [0f32; MAX_LEN];
            let cph = &mut cph[..len];
            let mut cu_phase = cu_phase0;
            for p in cph[pre..].iter_mut() {
                cu_phase += dcu;
                if cu_phase > TAU {
                    cu_phase -= TAU;
                }
                *p = cu_phase;
            }
            let gain = F32x8::splat(ch.cu_gain);
            let (q2, tau) = (F32x8::splat(FRAC_PI_2), F32x8::splat(TAU));
            let mut m0 = pre;
            while m0 + LANES <= len {
                let p = F32x8::load(&cph[m0..]);
                let cos = table_lerp(ct, p * kpos, COS_N - 1);
                // fast_sin: p - π/2 が負なら 2π を足す
                let ps = p - q2;
                let ps = F32x8::from_fn(|l| if ps.0[l] < 0.0 { ps.0[l] + tau.0[l] } else { ps.0[l] });
                let sin = table_lerp(ct, ps * kpos, COS_N - 1);
                let ci = F32x8::from_fn(|l| cu_i[(m0 + l) / OS - PRE_ROLL]);
                let cq = F32x8::from_fn(|l| cu_q[(m0 + l) / OS - PRE_ROLL]);
                let r = F32x8::load(&rf[m0..]) + gain * (ci * cos - cq * sin);
                rf[m0..m0 + LANES].copy_from_slice(&r.0);
                m0 += LANES;
            }
            for m in m0..len {
                let j = m / OS - PRE_ROLL;
                rf[m] += ch.cu_gain * (cu_i[j] * fast_cos(ct, cph[m]) - cu_q[j] * fast_sin(ct, cph[m]));
            }
        }
        // 5. 磁性体 (無記憶) と振幅
        for (r, &am) in rf.iter_mut().zip(a.iter()) {
            *r = am * ch.mag.transfer(*r);
        }
    }
}

/// 交差 `ms` それぞれの [`cross_fraction`] を `out` に書く
fn cross_fractions(x: &[f32], ms: &[usize], out: &mut [f32]) {
    use crate::simd::LANES;
    let full = ms.len() / LANES * LANES;
    if crate::simd::x8() {
        cross_fractions_x8(x, &ms[..full], &mut out[..full]);
    } else {
        for (o, &m) in out[..full].iter_mut().zip(&ms[..full]) {
            *o = cross_fraction(x, m);
        }
    }
    for (o, &m) in out[full..].iter_mut().zip(&ms[full..]) {
        *o = cross_fraction(x, m);
    }
}

crate::simd::x8_kernel! {
    /// [`cross_fraction`] を 8 交差ずつ同時に解く (交差 = レーン)。各レーンの演算・分岐はスカラー版と同じ
    fn cross_fractions_x8(x: &[f32], ms: &[usize], out: &mut [f32]) {
        use crate::simd::{F32x8, LANES};
        for (mc, oc) in ms.chunks_exact(LANES).zip(out.chunks_exact_mut(LANES)) {
            let p1 = F32x8::from_fn(|l| x[mc[l] - 1]);
            let p2 = F32x8::from_fn(|l| x[mc[l]]);
            let base: [isize; LANES] = std::array::from_fn(|l| mc[l] as isize - 1);
            let (mut t0, mut f0) = ([0f32; LANES], p1.0);
            let (mut t1, mut f1) = ([1f32; LANES], p2.0);
            let mut t = F32x8::from_fn(|l| p1.0[l] / (p1.0[l] - p2.0[l])).0;
            let mut done = [false; LANES];
            for _ in 0..2 {
                let f = lanczos_at_x8(x, &base, F32x8(t)).0;
                for l in 0..LANES {
                    if done[l] {
                        continue;
                    }
                    if (f[l] < 0.0) == (f0[l] < 0.0) {
                        (t0[l], f0[l]) = (t[l], f[l]);
                    } else {
                        (t1[l], f1[l]) = (t[l], f[l]);
                    }
                    let d = f1[l] - f0[l];
                    if d.abs() < 1e-12 {
                        done[l] = true;
                        continue;
                    }
                    t[l] = t0[l] - f0[l] * (t1[l] - t0[l]) / d;
                }
            }
            for l in 0..LANES {
                oc[l] = t[l].clamp(0.0, 1.0);
            }
        }
    }
}

/// [`lanczos_at`] の 8 レーン版 (レーンごとに別の位置)。各レーンの足し算の順序は同じ
#[inline(always)]
fn lanczos_at_x8(x: &[f32], base: &[isize; crate::simd::LANES], t: crate::simd::F32x8) -> crate::simd::F32x8 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: AVX2 の経路 (x8_kernel の中) からだけ呼ばれる
        if let Some(v) = unsafe { lanczos_at_avx2(x, base, t) } {
            return v;
        }
        let mut r = crate::simd::F32x8::splat(0.0);
        for l in 0..crate::simd::LANES {
            r.0[l] = lanczos_at(x, base[l], t.0[l]);
        }
        r
    }
    #[cfg(not(target_arch = "x86_64"))]
    lanczos_at_x8_portable(x, base, t)
}

/// [`lanczos_at_x8`] の AVX2 版: 信号と窓の表を gather で引く。
/// 窓の表の添字がどれかのレーンで範囲外 (異常な t) なら None を返し、呼び出し側がスカラーで計算する
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn lanczos_at_avx2(x: &[f32], base: &[isize; crate::simd::LANES], t: crate::simd::F32x8) -> Option<crate::simd::F32x8> {
    use std::arch::x86_64::*;
    const A: i32 = LANCZOS_A as i32;
    let table = lanczos_table();
    debug_assert!(x.len() < i32::MAX as usize);
    unsafe {
        let tv = _mm256_loadu_ps(t.0.as_ptr());
        let res = _mm256_set1_ps(LANCZOS_RES as f32);
        let sign = _mm256_set1_ps(-0.0);
        // 表の添字 j と j + 1 が範囲に収まるか (pos < len - 1。NaN は外れる)
        let limit = _mm256_set1_ps((table.len() - 1) as f32);
        let mut ok = true;
        let base_v = _mm256_setr_epi32(
            base[0] as i32, base[1] as i32, base[2] as i32, base[3] as i32,
            base[4] as i32, base[5] as i32, base[6] as i32, base[7] as i32,
        );
        let (lo, hi) = (_mm256_setzero_si256(), _mm256_set1_epi32(x.len() as i32 - 1));
        let mut acc = _mm256_setzero_ps();
        for k in (1 - A)..=A {
            let i = _mm256_min_epi32(_mm256_max_epi32(_mm256_add_epi32(base_v, _mm256_set1_epi32(k)), lo), hi);
            let xv = _mm256_i32gather_ps::<4>(x.as_ptr(), i);
            let pos = _mm256_mul_ps(_mm256_andnot_ps(sign, _mm256_sub_ps(tv, _mm256_set1_ps(k as f32))), res);
            ok &= _mm256_movemask_ps(_mm256_cmp_ps::<_CMP_LT_OQ>(pos, limit)) == 0xFF;
            if !ok {
                return None;
            }
            let j = _mm256_cvttps_epi32(pos);
            let fr = _mm256_sub_ps(pos, _mm256_cvtepi32_ps(j));
            let w0 = _mm256_i32gather_ps::<4>(table.as_ptr(), j);
            let w1 = _mm256_i32gather_ps::<4>(table.as_ptr().add(1), j);
            let w = _mm256_add_ps(w0, _mm256_mul_ps(_mm256_sub_ps(w1, w0), fr));
            acc = _mm256_add_ps(acc, _mm256_mul_ps(xv, w));
        }
        let mut r = crate::simd::F32x8::splat(0.0);
        _mm256_storeu_ps(r.0.as_mut_ptr(), acc);
        Some(r)
    }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
fn lanczos_at_x8_portable(x: &[f32], base: &[isize; crate::simd::LANES], t: crate::simd::F32x8) -> crate::simd::F32x8 {
    use crate::simd::{F32x8, LANES};
    const A: isize = LANCZOS_A as isize;
    let table = lanczos_table();
    let last = x.len() as isize - 1;
    let mut acc = F32x8::splat(0.0);
    for k in (1 - A)..=A {
        let xv = F32x8::from_fn(|l| x[(base[l] + k).clamp(0, last) as usize]);
        let pos = F32x8::from_fn(|l| (t.0[l] - k as f32).abs() * LANCZOS_RES as f32);
        let j: [usize; LANES] = std::array::from_fn(|l| pos.0[l] as usize);
        let fr = F32x8::from_fn(|l| pos.0[l] - j[l] as f32);
        let w0 = F32x8::from_fn(|l| table[j[l]]);
        let w1 = F32x8::from_fn(|l| table[j[l] + 1]);
        let w = w0 + (w1 - w0) * fr;
        acc = acc + xv * w;
    }
    acc
}

/// cos の表の大きさ (1 周期)
const COS_N: usize = 4096;

/// 1 周期分の cos の表 (最後に 1 点余分に持ち、線形補間する)。
/// 搬送波・低域変換クロマでサンプルごとに 3 回三角関数を使うので表引きにする (誤差 < 1e-6)
fn cos_table() -> &'static [f32] {
    use std::sync::OnceLock;
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| (0..=COS_N).map(|i| (std::f64::consts::TAU * i as f64 / COS_N as f64).cos() as f32).collect())
}

/// cos(phase)。phase は 0..2π
#[inline(always)]
fn fast_cos(table: &[f32], phase: f32) -> f32 {
    let pos = phase * (COS_N as f32 / std::f32::consts::TAU);
    let i = (pos as usize).min(COS_N - 1);
    let fr = pos - i as f32;
    table[i] + (table[i + 1] - table[i]) * fr
}

/// sin(phase) = cos(phase - π/2)。phase は 0..2π
#[inline(always)]
fn fast_sin(table: &[f32], phase: f32) -> f32 {
    let p = phase - std::f32::consts::FRAC_PI_2;
    fast_cos(table, if p < 0.0 { p + std::f32::consts::TAU } else { p })
}

/// Lanczos 窓の半幅 (タップ数はこの 2 倍)
const LANCZOS_A: usize = 8;
/// 窓関数の表の分解能 [点 / サンプル]
const LANCZOS_RES: usize = 512;

/// Lanczos-8 の重みの表 (|d| = 0..8 を LANCZOS_RES 刻み)。sin を毎回計算すると FM 復調が重くなるので表引きにする
fn lanczos_table() -> &'static [f32] {
    use std::sync::OnceLock;
    static T: OnceLock<Vec<f32>> = OnceLock::new();
    T.get_or_init(|| {
        use std::f64::consts::PI;
        let a = LANCZOS_A as f64;
        (0..=LANCZOS_A * LANCZOS_RES + 1)
            .map(|i| {
                let d = i as f64 / LANCZOS_RES as f64;
                if d < 1e-9 {
                    1.0
                } else if d >= a {
                    0.0
                } else {
                    let pd = PI * d;
                    (a * pd.sin() * (pd / a).sin() / (pd * pd)) as f32
                }
            })
            .collect()
    })
}

/// `x[base]` から t (0..1) だけ進んだ位置の値 (Lanczos-8 補間)
#[inline]
fn lanczos_at(x: &[f32], base: isize, t: f32) -> f32 {
    const A: isize = LANCZOS_A as isize;
    let table = lanczos_table();
    let last = x.len() as isize - 1;
    let mut acc = 0f32;
    for k in (1 - A)..=A {
        let i = (base + k).clamp(0, last) as usize;
        let pos = (t - k as f32).abs() * LANCZOS_RES as f32;
        let j = pos as usize;
        let fr = pos - j as f32;
        let w = table[j] + (table[j + 1] - table[j]) * fr;
        acc += x[i] * w;
    }
    acc
}

/// フィルタの直流での群遅延 (インパルス応答の重心) [サンプル]
fn dc_delay(f: Cascade) -> f32 {
    let mut f = f;
    let (mut s, mut sn) = (0f32, 0f32);
    for n in 0..4096 {
        let h = f.process(if n == 0 { 1.0 } else { 0.0 });
        s += h;
        sn += n as f32 * h;
    }
    if s.abs() > 1e-6 { sn / s } else { 0.0 }
}

pub struct FmConfig {
    /// 元の (4fsc) サンプリング周波数 [Hz]
    pub fs: f32,
    pub f_sync_hz: f32,
    pub f_white_hz: f32,
    pub levels: Levels,
    /// 再生伝送路の帯域 [Hz]
    pub channel_bw_hz: f32,
    /// 復調後の輝度帯域 [Hz]
    pub luma_bw_hz: f32,
    /// RF ノイズ。伝送路の帯域内の rms が搬送波振幅 1 に対してこの値になる
    pub noise: f32,
    pub color_under_hz: f32,
    /// バースト振幅の色信号が、搬送波振幅 1 に対してこの振幅で記録される
    pub color_under_level: f32,
    /// 磁性体の伝達特性 (飽和レベルは搬送波の振幅 = 1 の尺度)
    pub mag: MagneticTransfer,
    /// ヘッドとテープの相対速度 (書き込み速度) [m/s]
    pub writing_speed: f32,
    /// 再生 EQ が想定していない余分なスペーシング [m]
    pub extra_spacing: f32,
    /// J-A の記録を計算するオーバーサンプリング倍率 ([`OVERSAMPLE`] の倍数に丸め、上限 [`MAX_JA_OVERSAMPLE`])
    pub ja_oversample: usize,
}

impl FmChannel {
    pub fn new(c: &FmConfig) -> Self {
        let oversample = if c.mag.is_jiles_atherton() {
            (c.ja_oversample.div_ceil(OVERSAMPLE) * OVERSAMPLE).clamp(OVERSAMPLE, MAX_JA_OVERSAMPLE)
        } else {
            OVERSAMPLE
        };
        let fs2 = c.fs * OVERSAMPLE as f32;
        let bw = c.channel_bw_hz.clamp(0.5e6, 0.45 * fs2);
        // 間引きのローパス: 伝送路の帯域は通し、間引いた後に帯域へ折り返す成分 (fs2 - bw 以上) を十分落とす
        let decimate = (oversample > OVERSAMPLE).then(|| {
            Cascade::butter_lowpass_at(8, (1.15 * bw).min(0.42 * fs2), c.fs * oversample as f32)
        });
        let decimate_delay = decimate.map_or(0.0, |f| dc_delay(f) / (oversample / OVERSAMPLE) as f32);
        // 低域変換クロマ (〜1.2 MHz) と搬送波の下側波帯の間で切る
        let hp = (0.4 * c.f_sync_hz).clamp(0.3e6, 0.4 * bw);
        let rf_lp = Cascade::butter_lowpass_at(4, bw, fs2);
        let post_lp = Cascade::butter_lowpass_at(4, c.luma_bw_hz.clamp(0.3e6, 0.4 * c.fs), fs2);
        // e^{-k d} = e^{-2π f d / v} → 離散時間では e^{-|ω| τ}、τ = d / v · fs2 [サンプル]
        let mut spacing = [0f32; 2 * SPACING_HALF_MAX + 1];
        let tau = c.extra_spacing.max(0.0) / c.writing_speed.max(0.1) * fs2;
        let spacing_half = if tau > 1e-3 { ((16.0 * tau).ceil() as usize).clamp(4, SPACING_HALF_MAX) } else { 0 };
        if spacing_half > 0 {
            super::head::spacing_kernel(tau, &mut spacing[..2 * spacing_half + 1]);
        }
        Self {
            oversample,
            decimate,
            fs2,
            f_sync: c.f_sync_hz,
            f_white: c.f_white_hz.max(c.f_sync_hz + 0.1e6),
            lv: c.levels,
            rf_lp,
            rf_hp: Cascade::butter_highpass_at(2, hp, fs2),
            post_lp,
            // 周期計測の復調は直前の半周期の平均を出すので、さらに約 1/4 周期遅れる
            delay: dc_delay(rf_lp) + dc_delay(post_lp) + 0.25 * fs2 / c.f_sync_hz + decimate_delay,
            // 白色雑音を帯域 bw で切ると rms は sqrt(2 bw / fs2) 倍になるので戻しておく
            noise: c.noise * (0.5 * fs2 / bw).sqrt(),
            cu_freq: c.color_under_hz,
            cu_gain: c.color_under_level / c.levels.burst.max(1e-3),
            mag: c.mag.clone(),
            spacing,
            spacing_half,
            spacing_k: std::f32::consts::TAU / c.writing_speed.max(0.1),
        }
    }

    /// 輝度レベル [IRE] → 搬送波周波数 [Hz]
    #[inline(always)]
    fn freq_of(&self, v: f32) -> f32 {
        self.f_sync + (v - self.lv.sync) / (self.lv.white - self.lv.sync) * (self.f_white - self.f_sync)
    }

    /// 搬送波周波数 [Hz] → 輝度レベル [IRE]
    #[inline(always)]
    fn level_of(&self, f: f32) -> f32 {
        self.lv.sync + (f - self.f_sync) / (self.f_white - self.f_sync) * (self.lv.white - self.lv.sync)
    }

    /// 1 ラインを記録・再生する。
    ///
    /// - `luma`: 記録する輝度 (エンファシス・クリップ後) [IRE]
    /// - `cu_i`, `cu_q`: 低域変換クロマの複素包絡線 [IRE] (色がなければ空でよい)
    /// - `amp`: 搬送波の振幅 (トラッキングずれによる RF レベル。周波数によらない)
    /// - `line_spacing`: このラインの余分なスペーシング (しわ・目詰まり) [m]
    /// - `drop`: ドロップアウト ([開始, 終了) [サンプル] とスペーシング [m])。縁はなだらかに立ち上げる
    /// - `out`: 復調した輝度 [IRE] (ディエンファシス前)
    ///
    /// ライン・ドロップアウトのスペーシングは時間とともに変わるので FIR にせず、その瞬間の搬送波周波数での
    /// 損失 e^{-2π f d / v} を振幅に掛ける (準静的な近似。側波帯ごとの傾きは無視する)
    /// 1 ラインを記録・再生する。
    ///
    /// - `luma`: 記録する輝度 (エンファシス・クリップ後) [IRE]
    /// - `cu_i`, `cu_q`: 低域変換クロマの複素包絡線 [IRE] (色がなければ空でよい)
    /// - `amp`: 搬送波の振幅 (トラッキングずれによる RF レベル。周波数によらない)
    /// - `line_spacing`: このラインの余分なスペーシング (しわ・目詰まり) [m]
    /// - `drop`: ドロップアウト ([開始, 終了) [サンプル] とスペーシング [m])。縁はなだらかに立ち上げる
    /// - `out`: 復調した輝度 [IRE] (ディエンファシス前)
    ///
    /// ライン・ドロップアウトのスペーシングは時間とともに変わるので FIR にせず、その瞬間の搬送波周波数での
    /// 損失 e^{-2π f d / v} を振幅に掛ける (準静的な近似。側波帯ごとの傾きは無視する)
    #[allow(clippy::too_many_arguments)]
    pub fn process_line(
        &self,
        luma: &[f32],
        cu_i: &[f32],
        cu_q: &[f32],
        amp: f32,
        line_spacing: f32,
        drop: &[super::Dropout],
        rng: &mut Rng,
        out: &mut [f32],
    ) {
        let len = rf_len(luma.len());
        let mut rf = [0f32; MAX_LEN];
        let rf = &mut rf[..len];
        self.record(luma, cu_i, cu_q, amp, line_spacing, drop, rng, rf);
        let (mut lp, mut hp) = (self.rf_lp, self.rf_hp);
        for v in rf.iter_mut() {
            *v = hp.process(lp.process(*v));
        }
        let mut freq = [0f32; MAX_LEN];
        let freq = &mut freq[..len];
        self.demodulate(rf, freq);
        let mut post = self.post_lp;
        for v in freq.iter_mut() {
            *v = post.process(*v);
        }
        self.resample(freq, out);
    }

    /// 8 ラインをまとめて記録・再生する ([`process_line`](Self::process_line) を 8 回呼ぶのと同じ結果)。
    /// 伝送路と復調後の IIR は 8 ラインを SIMD のレーンに並べ (縦方向)、変調と復調はラインごとに行う。
    /// `luma`・`cu_i`・`cu_q`・`out` は 8 ラインがライン長 `n` おきに並ぶ (色がなければ `cu_i`・`cu_q` は空)
    #[allow(clippy::too_many_arguments)]
    pub fn process_lines_x8(
        &self,
        n: usize,
        luma: &[f32],
        cu_i: &[f32],
        cu_q: &[f32],
        amp: &[f32; LANES],
        line_spacing: &[f32; LANES],
        drop: &[&[super::Dropout]; LANES],
        rng: &mut [Rng; LANES],
        out: &mut [f32],
    ) {
        let len = rf_len(n);
        let mut rf = vec![0f32; LANES * len];
        let has_cu = !cu_i.is_empty();
        for l in 0..LANES {
            let r = l * n..(l + 1) * n;
            let (ci, cq) = if has_cu { (&cu_i[r.clone()], &cu_q[r.clone()]) } else { (&[][..], &[][..]) };
            self.record(&luma[r], ci, cq, amp[l], line_spacing[l], drop[l], &mut rng[l], &mut rf[l * len..(l + 1) * len]);
        }
        iir2_x8(&self.rf_lp, &self.rf_hp, &mut rf, len);
        let mut freq = vec![0f32; LANES * len];
        for l in 0..LANES {
            let r = l * len..(l + 1) * len;
            self.demodulate(&rf[r.clone()], &mut freq[r]);
        }
        iir2_x8(&self.post_lp, &Cascade::default(), &mut freq, len);
        for l in 0..LANES {
            self.resample(&freq[l * len..(l + 1) * len], &mut out[l * n..(l + 1) * n]);
        }
    }

    /// 記録して伝送路の入口まで: 変調・磁性体・余分なスペーシングの FIR と RF ノイズ。`rf` の長さは [`rf_len`]
    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        luma: &[f32],
        cu_i: &[f32],
        cu_q: &[f32],
        amp: f32,
        line_spacing: f32,
        drop: &[super::Dropout],
        rng: &mut Rng,
        rf: &mut [f32],
    ) {
        use std::f32::consts::TAU;
        let n = luma.len();
        debug_assert!(n <= MAX_LINE_SAMPLES && rf.len() == rf_len(n));
        let os = self.oversample;
        let decim = os / OVERSAMPLE;
        let len_gen = (n + PRE_ROLL) * os;
        let len = rf.len();
        let has_cu = cu_i.len() == n && cu_q.len() == n && self.cu_gain > 0.0;

        // 元のレートでの値 (助走区間はシンクチップ)
        let at = |i: isize| -> f32 {
            let j = i - PRE_ROLL as isize;
            if j < 0 { self.lv.sync } else { luma[(j as usize).min(n - 1)] }
        };

        // --- 変調 + 伝送路 ---
        let mut phase = rng.uniform() * TAU;
        let mut cu_phase = rng.uniform() * TAU;
        let fs_gen = self.fs2 * decim as f32;
        let (dphi_k, dcu) = (TAU / fs_gen, TAU * self.cu_freq / fs_gen);
        let mut aa = self.decimate;
        // Line-local state keeps this PoC compatible with the existing parallel line renderer.
        // The sync-carrier pre-roll gives the hysteresis loop several cycles to settle.
        let mut magnetic = self.mag.state();
        let ct = cos_table();
        let staged = !self.mag.is_jiles_atherton() && crate::simd::x8();
        if staged {
            // 無記憶の磁性体 (2 倍、間引きなし): 位相の積算だけを逐次にし、残りはサンプル方向にベクトル化する
            debug_assert!(os == OVERSAMPLE && aa.is_none());
            let (phase0, cu_phase0) = (phase, cu_phase);
            let cu = has_cu.then_some((cu_i, cu_q));
            modulate_memoryless_x8(self, luma, cu, amp, line_spacing, drop, phase0, cu_phase0, rf);
        }
        for m in 0..if staged { 0 } else { len_gen } {
            let i = (m / os) as isize;
            // Catmull-Rom interpolation. At 2x this is the previous midpoint formula.
            let frac = (m % os) as f32 / os as f32;
            let v = if frac == 0.0 {
                at(i)
            } else {
                let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
                let t2 = frac * frac;
                let t3 = t2 * frac;
                0.5 * ((2.0 * p1)
                    + (-p0 + p2) * frac
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                    + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
            };
            let f_inst = self.freq_of(v);
            phase += dphi_k * f_inst;
            if phase > TAU {
                phase -= TAU;
            }
            // 搬送波の振幅: しわ・目詰まり・ドロップアウトのスペーシング損失 (ドロップアウトは縁をなだらかに)
            let src = (m / os).saturating_sub(PRE_ROLL);
            let mut spacing = line_spacing;
            if m >= PRE_ROLL * os {
                for &(s, e, depth) in drop {
                    let d = if src < s { s - src } else if src >= e { src + 1 - e } else { 0 };
                    if d < super::DROPOUT_EDGE {
                        spacing += depth * (1.0 - d as f32 / super::DROPOUT_EDGE as f32);
                    }
                }
            }
            let mut a = amp;
            if spacing > 0.0 {
                a *= (-self.spacing_k * f_inst.max(0.0) * spacing).exp();
            }
            // 記録: 搬送波 + 低域変換クロマを磁性体の伝達特性に通す
            let mut rec = fast_cos(ct, phase);
            if has_cu && m >= PRE_ROLL * os {
                let j = src.min(n - 1);
                cu_phase += dcu;
                if cu_phase > TAU {
                    cu_phase -= TAU;
                }
                rec += self.cu_gain * (cu_i[j] * fast_cos(ct, cu_phase) - cu_q[j] * fast_sin(ct, cu_phase));
            }
            // 再生: 同じトラックなので、クロマも RF レベルの影響を同じだけ受ける
            let y = a * magnetic.process(rec);
            match aa.as_mut() {
                None => rf[m] = y,
                Some(f) => {
                    let y = f.process(y);
                    if m % decim == 0 {
                        rf[m / decim] = y;
                    }
                }
            }
        }
        // 余分なスペーシング: テープから出る信号だけが弱まり、ヘッドアンプのノイズはそのまま (S/N が落ちる)
        if self.spacing_half > 0 {
            let h = self.spacing_half;
            let taps = &self.spacing[..2 * h + 1];
            let mut src = [0f32; MAX_LEN];
            src[..len].copy_from_slice(rf);
            for (m, o) in rf.iter_mut().enumerate().take(len) {
                let mut acc = 0f32;
                for (j, t) in taps.iter().enumerate() {
                    let i = (m + j).saturating_sub(h).min(len - 1);
                    acc += t * src[i];
                }
                *o = acc;
            }
        }
        for v in rf.iter_mut() {
            *v += self.noise * rng.normal();
        }
    }

    /// リミッタ + 周期計測型の復調: ゼロ交差 (両方向) の間隔 Δt から f = 1 / (2Δt)。
    /// 区間内のサンプルにはその区間の周波数を入れる
    fn demodulate(&self, rf: &[f32], freq: &mut [f32]) {
        let len = rf.len();
        let mut last_cross = 0f32;
        let mut filled = 0usize;
        let mut f_prev = self.f_sync;
        // 交差の位置を先に集め、交差ごとに独立な小数位置の計算をまとめて行う (8 交差ずつ SIMD のレーンに並べる)
        let mut crosses = [0usize; MAX_LEN];
        let mut n_cross = 0;
        for m in 1..len {
            if (rf[m - 1] < 0.0) != (rf[m] < 0.0) {
                crosses[n_cross] = m;
                n_cross += 1;
            }
        }
        let mut fracs = [0f32; MAX_LEN];
        cross_fractions(rf, &crosses[..n_cross], &mut fracs[..n_cross]);
        for (&m, &frac) in crosses[..n_cross].iter().zip(&fracs[..n_cross]) {
            let t = (m - 1) as f32 + frac;
            let dt = (t - last_cross).max(1e-3);
            let f = self.fs2 / (2.0 * dt);
            for v in &mut freq[filled..m] {
                *v = f;
            }
            filled = m;
            last_cross = t;
            f_prev = f;
        }
        // 最後の交差以降は、その時点までの経過から周波数の上限を見積もる (交差がなければ下がっていく)
        for (m, v) in freq.iter_mut().enumerate().skip(filled) {
            let dt = (m as f32 - last_cross).max(1.0);
            *v = f_prev.min(self.fs2 / (2.0 * dt));
        }
    }

    /// 遅れを打ち消して元のレートに間引き、周波数を輝度に戻す
    fn resample(&self, freq: &[f32], out: &mut [f32]) {
        let len = freq.len();
        for (j, o) in out.iter_mut().enumerate() {
            let t = ((j + PRE_ROLL) * OVERSAMPLE) as f32 + self.delay;
            let i0 = (t.floor() as usize).min(len - 2);
            let fr = (t - i0 as f32).clamp(0.0, 1.0);
            *o = self.level_of(freq[i0] + (freq[i0 + 1] - freq[i0]) * fr);
        }
    }
}

/// ライン長 `n` に対する再生側のバッファの長さ (助走込み、2 倍オーバーサンプリング)
pub fn rf_len(n: usize) -> usize {
    (n + PRE_ROLL) * OVERSAMPLE
}

crate::simd::x8_kernel! {
    /// 8 ライン (長さ `len` おき) に縦続 IIR `f` → `g` を掛ける (縦方向 SIMD)。
    /// 各レーンは `g.process(f.process(x))` をラインごとに回すのと同じ演算
    fn iir2_x8(f: &Cascade, g: &Cascade, buf: &mut [f32], len: usize) {
        use crate::simd::{F32x8, rows_mut};
        let mut rows = rows_mut(buf, len);
        let (mut f, mut g) = (f.lanes::<F32x8>(), g.lanes::<F32x8>());
        for m in 0..len {
            let x = F32x8::from_fn(|l| rows[l][m]);
            g.process(f.process(x)).scatter(&mut rows, m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::NTSC;

    fn channel(noise: f32) -> FmChannel {
        channel_with_spacing(noise, 0.0)
    }

    fn channel_with_spacing(noise: f32, extra_spacing: f32) -> FmChannel {
        FmChannel::new(&FmConfig {
            fs: NTSC.fs,
            f_sync_hz: 3.4e6,
            f_white_hz: 4.4e6,
            levels: NTSC.ire,
            channel_bw_hz: 5.5e6,
            luma_bw_hz: 3.0e6,
            noise,
            color_under_hz: 629e3,
            color_under_level: 0.0,
            mag: MagneticTransfer::new(crate::params::MagneticModel::Linear, 1.0),
            writing_speed: 5.8,
            extra_spacing,
            ja_oversample: 4,
        })
    }

    /// 余分なスペーシングでは信号だけが弱まるので、高い搬送波 (白) ほど S/N が落ちる。
    /// ノイズがなければ復調したレベルは変わらない
    #[test]
    fn spacing_hurts_white_more_than_black() {
        let quiet = channel_with_spacing(0.0, 0.3e-6);
        for level in [7.5f32, 100.0] {
            let input = vec![level; 910];
            let mut out = vec![0f32; 910];
            quiet.process_line(&input, &[], &[], 1.0, 0.0, &[], &mut Rng::new(1), &mut out);
            let (mean, sd, _, _) = stats(&out[100..800]);
            assert!((mean - level).abs() < 0.5 && sd < 0.3, "level {level}: mean {mean} sd {sd}");
        }
        let sd_at = |spacing: f32, level: f32| {
            let ch = channel_with_spacing(0.08, spacing);
            let input = vec![level; 910];
            let mut out = vec![0f32; 910];
            ch.process_line(&input, &[], &[], 1.0, 0.0, &[], &mut Rng::new(7), &mut out);
            stats(&out[100..800]).1
        };
        let (black0, white0) = (sd_at(0.0, 7.5), sd_at(0.0, 100.0));
        let (black, white) = (sd_at(0.3e-6, 7.5), sd_at(0.3e-6, 100.0));
        assert!(black > black0 && white > white0, "noise must grow: {black0}->{black}, {white0}->{white}");
        assert!(white / white0 > black / black0, "white {white0}->{white}, black {black0}->{black}");
    }

    /// 浅いドロップアウト (スペーシング) は区間の中だけで効き、黒より白を大きく乱す。
    /// ライン全体のスペーシング (しわ・目詰まり) も同じ向きに効く
    #[test]
    fn shallow_dropout_and_line_spacing_hit_white_first() {
        let ch = channel(0.05);
        let sd_in = |level: f32, line_spacing: f32, drop: &[crate::tape::Dropout], range: std::ops::Range<usize>| {
            let input = vec![level; 910];
            let mut out = vec![0f32; 910];
            ch.process_line(&input, &[], &[], 1.0, line_spacing, drop, &mut Rng::new(3), &mut out);
            stats(&out[range]).1
        };
        let drop = [(300usize, 600usize, 0.35e-6f32)];
        for level in [7.5f32, 100.0] {
            // 区間の外は乱れない
            let outside = sd_in(level, 0.0, &drop, 100..250);
            assert!((outside - sd_in(level, 0.0, &[], 100..250)).abs() < 1e-4, "level {level}");
        }
        let ratio = |level: f32| sd_in(level, 0.0, &drop, 330..570) / sd_in(level, 0.0, &[], 330..570);
        let (black, white) = (ratio(7.5), ratio(100.0));
        assert!(black > 1.0 && white > black, "dropout noise ratio: black {black}, white {white}");
        let line = |level: f32| sd_in(level, 0.35e-6, &[], 100..800) / sd_in(level, 0.0, &[], 100..800);
        let (black, white) = (line(7.5), line(100.0));
        assert!(black > 1.0 && white > black, "line spacing noise ratio: black {black}, white {white}");
    }

    /// 8 ラインまとめた処理は、ラインごとの処理とビット単位で一致する (色・ドロップアウト・スペーシング・ノイズ込み)
    #[test]
    fn lines_x8_match_per_line() {
        use crate::params::{JaSolver, MagneticMedia, MagneticModel};
        if !crate::simd::x8() {
            return;
        }
        let n = 910;
        for model in [MagneticModel::Linear, MagneticModel::Tanh, MagneticModel::JilesAtherton] {
            let ch = FmChannel::new(&FmConfig {
                fs: NTSC.fs,
                f_sync_hz: 3.4e6,
                f_white_hz: 4.4e6,
                levels: NTSC.ire,
                channel_bw_hz: 5.5e6,
                luma_bw_hz: 3.0e6,
                noise: 0.05,
                color_under_hz: 629e3,
                color_under_level: 0.1,
                mag: MagneticTransfer::rf(model, MagneticMedia::MetalParticle, 0.8, 3.0, JaSolver::Table),
                writing_speed: 5.8,
                extra_spacing: 0.05e-6,
                ja_oversample: 4,
            });
            let luma: Vec<f32> = (0..LANES * n).map(|i| 50.0 + 45.0 * ((i % n) as f32 * 0.013 * (1 + i / n) as f32).sin()).collect();
            let cu_i: Vec<f32> = (0..LANES * n).map(|i| 20.0 * (i as f32 * 0.002).cos()).collect();
            let cu_q: Vec<f32> = (0..LANES * n).map(|i| 20.0 * (i as f32 * 0.003).sin()).collect();
            let amp: [f32; LANES] = std::array::from_fn(|l| 1.0 - 0.05 * l as f32);
            let spacing: [f32; LANES] = std::array::from_fn(|l| if l % 3 == 0 { 0.1e-6 } else { 0.0 });
            let drops: Vec<Vec<crate::tape::Dropout>> =
                (0..LANES).map(|l| if l % 2 == 1 { vec![(100 + 50 * l, 160 + 50 * l, 0.4e-6)] } else { vec![] }).collect();
            let dr: [&[crate::tape::Dropout]; LANES] = std::array::from_fn(|l| drops[l].as_slice());

            let mut want = vec![0f32; LANES * n];
            for l in 0..LANES {
                let r = l * n..(l + 1) * n;
                let mut rng = Rng::new(l as u64);
                ch.process_line(&luma[r.clone()], &cu_i[r.clone()], &cu_q[r.clone()], amp[l], spacing[l], dr[l], &mut rng, &mut want[r]);
            }
            let mut rngs: [Rng; LANES] = std::array::from_fn(|l| Rng::new(l as u64));
            let mut got = vec![0f32; LANES * n];
            ch.process_lines_x8(n, &luma, &cu_i, &cu_q, &amp, &spacing, &dr, &mut rngs, &mut got);
            for (i, (a, b)) in want.iter().zip(&got).enumerate() {
                assert_eq!(a.to_bits(), b.to_bits(), "{model:?}: line {} sample {}", i / n, i % n);
            }
        }
    }

    fn stats(v: &[f32]) -> (f32, f32, f32, f32) {
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let sd = (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt();
        let (mn, mx) = v.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        (mean, sd, mn, mx)
    }

    #[test]
    fn constant_level_is_flat() {
        let ch = channel(0.0);
        for level in [-40.0f32, 7.5, 50.0, 100.0] {
            let input = vec![level; 910];
            let mut out = vec![0f32; 910];
            let mut rng = Rng::new(1);
            ch.process_line(&input, &[], &[], 1.0, 0.0, &[], &mut rng, &mut out);
            let (mean, sd, _, _) = stats(&out[100..800]);
            assert!((mean - level).abs() < 0.5 && sd < 0.3, "level {level}: mean {mean} sd {sd}");
        }
    }

    /// J-A (メタルテープ、記録磁界 3 Hc) で RF を記録しても、FM なので輝度のレベルは保たれる (表引き・厳密とも)
    #[test]
    fn jiles_atherton_rf_keeps_levels() {
        use crate::params::{JaSolver, MagneticMedia, MagneticModel};
        for (solver, os) in [(JaSolver::Table, 4), (JaSolver::Table, 8), (JaSolver::Table, 16), (JaSolver::Exact, 8)] {
            let ch = FmChannel::new(&FmConfig {
                fs: NTSC.fs,
                f_sync_hz: 3.4e6,
                f_white_hz: 4.4e6,
                levels: NTSC.ire,
                channel_bw_hz: 5.5e6,
                luma_bw_hz: 3.0e6,
                noise: 0.0,
                color_under_hz: 629e3,
                color_under_level: 0.0,
                mag: MagneticTransfer::rf(
                    MagneticModel::JilesAtherton,
                    MagneticMedia::MetalParticle,
                    1.0,
                    3.0,
                    solver,
                ),
                writing_speed: 5.8,
                extra_spacing: 0.0,
                ja_oversample: os,
            });
            for level in [7.5f32, 50.0, 100.0] {
                let input = vec![level; 910];
                let mut out = vec![0f32; 910];
                ch.process_line(&input, &[], &[], 1.0, 0.0, &[], &mut Rng::new(1), &mut out);
                let (mean, sd, _, _) = stats(&out[100..800]);
                // 4 倍では飽和した搬送波の高調波の折り返しが、平坦な画でも 1〜2 IRE の揺れとして残る
                // (既定の RF ノイズ 0.05 での揺れ 7 IRE に比べれば小さい)
                let max_sd = if os < 8 { 2.5 } else { 0.5 };
                assert!(
                    (mean - level).abs() < 1.0 && sd < max_sd,
                    "{solver:?} x{os} level {level}: mean {mean} sd {sd}"
                );
            }
        }
    }
}
