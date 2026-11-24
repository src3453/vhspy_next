//! 輝度の FM 記録・再生を実際に計算するモデル。
//!
//! 1 ラインごとに次の順で処理する (通常 2 倍、J-A は 4 倍にオーバーサンプリング)。
//!
//! 1. オーバーサンプリング: Hi8 (白 7.7 MHz) でも搬送波と側波帯がナイキスト周波数に収まるようにする
//! 2. 周波数変調: シンクチップ〜ホワイトピークを各フォーマットの搬送波周波数に割り当てる。
//!    エンファシスで白のエッジが行き過ぎると、ホワイトピークより高い周波数まで振れる
//! 3. 低域変換クロマの重畳と磁性体の伝達特性: 実機では色は FM 信号をバイアス代わりに同じトラックへ
//!    記録される。和の瞬時値が磁性体で飽和すると、搬送波とクロマの相互変調積が搬送波の近くに立ち
//!    (伝送路の傾きで AM が PM に変わり)、輝度のモアレになる
//! 4. 再生の伝送路: ヘッド・テープの高域損失 (ローパス)、クロマを分けるハイパス、RF ノイズ。
//!    RF レベル (トラッキング・しわ・ドロップアウト) は搬送波の振幅として効く
//! 5. リミッタ + 周期計測型の復調: ゼロ交差の間隔から瞬時周波数を求めて輝度に戻す。
//!    高い周波数ほど伝送路で搬送波が弱まり、下側波帯が勝つとゼロ交差を見落として周波数が落ちる
//!    (白のエッジの右に黒い筋 = ベアディング)。搬送波が消えるとノイズのゼロ交差を拾い、白黒の砂嵐になる

use crate::filter::Cascade;
use super::magnetic::MagneticTransfer;
use crate::rng::Rng;
use crate::timing::{Levels, MAX_LINE_SAMPLES};

/// オーバーサンプリング倍率
pub const OVERSAMPLE: usize = 2;
/// ライン先頭の前に流す助走 (元のレートのサンプル数)。フィルタと復調器を落ち着かせる
const PRE_ROLL: usize = 48;
const JA_OVERSAMPLE: usize = 4;
const MAX_OVERSAMPLE: usize = JA_OVERSAMPLE;
const MAX_LEN: usize = (MAX_LINE_SAMPLES + PRE_ROLL) * MAX_OVERSAMPLE;

/// FM 伝送路の設定 (フィールドごとに作る)
#[derive(Clone, Copy)]
pub struct FmChannel {
    /// J-A は 4 倍、それ以外は従来どおり 2 倍で処理する
    oversample: usize,
    /// オーバーサンプリング後のサンプリング周波数 [Hz]
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
}

impl FmChannel {
    pub fn new(c: &FmConfig) -> Self {
        let oversample = if c.mag.is_jiles_atherton() { JA_OVERSAMPLE } else { OVERSAMPLE };
        let fs2 = c.fs * oversample as f32;
        let bw = c.channel_bw_hz.clamp(0.5e6, 0.45 * fs2);
        // 低域変換クロマ (〜1.2 MHz) と搬送波の下側波帯の間で切る
        let hp = (0.4 * c.f_sync_hz).clamp(0.3e6, 0.4 * bw);
        let rf_lp = Cascade::butter_lowpass_at(4, bw, fs2);
        let post_lp = Cascade::butter_lowpass_at(4, c.luma_bw_hz.clamp(0.3e6, 0.4 * c.fs), fs2);
        Self {
            oversample,
            fs2,
            f_sync: c.f_sync_hz,
            f_white: c.f_white_hz.max(c.f_sync_hz + 0.1e6),
            lv: c.levels,
            rf_lp,
            rf_hp: Cascade::butter_highpass_at(2, hp, fs2),
            post_lp,
            // 周期計測の復調は直前の半周期の平均を出すので、さらに約 1/4 周期遅れる
            delay: dc_delay(rf_lp) + dc_delay(post_lp) + 0.25 * fs2 / c.f_sync_hz,
            // 白色雑音を帯域 bw で切ると rms は sqrt(2 bw / fs2) 倍になるので戻しておく
            noise: c.noise * (0.5 * fs2 / bw).sqrt(),
            cu_freq: c.color_under_hz,
            cu_gain: c.color_under_level / c.levels.burst.max(1e-3),
            mag: c.mag,
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
    /// - `amp`: 搬送波の振幅 (RF レベル)。ドロップアウト区間は `drop` で 0 近くまで落とす
    /// - `out`: 復調した輝度 [IRE] (ディエンファシス前)
    pub fn process_line(
        &self,
        luma: &[f32],
        cu_i: &[f32],
        cu_q: &[f32],
        amp: f32,
        drop: &[(usize, usize)],
        rng: &mut Rng,
        out: &mut [f32],
    ) {
        use std::f32::consts::TAU;
        let n = luma.len();
        debug_assert!(n <= MAX_LINE_SAMPLES && out.len() == n);
        let os = self.oversample;
        let len = (n + PRE_ROLL) * os;
        let has_cu = cu_i.len() == n && cu_q.len() == n && self.cu_gain > 0.0;

        // 元のレートでの値 (助走区間はシンクチップ)
        let at = |i: isize| -> f32 {
            let j = i - PRE_ROLL as isize;
            if j < 0 { self.lv.sync } else { luma[(j as usize).min(n - 1)] }
        };

        // --- 変調 + 伝送路 ---
        let mut rf = [0f32; MAX_LEN];
        let mut phase = rng.uniform() * TAU;
        let mut cu_phase = rng.uniform() * TAU;
        let (dphi_k, dcu) = (TAU / self.fs2, TAU * self.cu_freq / self.fs2);
        let (mut lp, mut hp) = (self.rf_lp, self.rf_hp);
        // Line-local state keeps this PoC compatible with the existing parallel line renderer.
        // The sync-carrier pre-roll gives the hysteresis loop several cycles to settle.
        let mut magnetic = self.mag.state();
        let ct = cos_table();
        for m in 0..len {
            let i = (m / os) as isize;
            // Catmull-Rom interpolation. At 2x this is the previous midpoint formula;
            // the J-A model uses quarter-sample points for its 4x RF oversampling.
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
            phase += dphi_k * self.freq_of(v);
            if phase > TAU {
                phase -= TAU;
            }
            // 搬送波の振幅: ドロップアウト区間は端をなだらかにして落とす
            let src = (m / os).saturating_sub(PRE_ROLL);
            let mut a = amp;
            if m >= PRE_ROLL * os {
                for &(s, e) in drop {
                    let d = if src < s { s - src } else if src >= e { src + 1 - e } else { 0 };
                    if d < 6 {
                        a *= 0.03 + 0.97 * d as f32 / 6.0;
                    }
                }
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
            let recorded = magnetic.process(rec, self.fs2);
            let x = a * recorded + self.noise * rng.normal();
            rf[m] = hp.process(lp.process(x));
        }

        // --- リミッタ + 周期計測型の復調 ---
        // ゼロ交差 (両方向) の間隔 Δt から f = 1 / (2Δt)。区間内のサンプルにはその区間の周波数を入れる
        let mut freq = [0f32; MAX_LEN];
        let mut last_cross = 0f32;
        let mut filled = 0usize;
        let mut f_prev = self.f_sync;
        for m in 1..len {
            let (a, b) = (rf[m - 1], rf[m]);
            if (a < 0.0) != (b < 0.0) {
                let t = (m - 1) as f32 + cross_fraction(&rf[..len], m);
                let dt = (t - last_cross).max(1e-3);
                let f = self.fs2 / (2.0 * dt);
                for v in &mut freq[filled..m] {
                    *v = f;
                }
                filled = m;
                last_cross = t;
                f_prev = f;
            }
        }
        // 最後の交差以降は、その時点までの経過から周波数の上限を見積もる (交差がなければ下がっていく)
        for (m, v) in freq.iter_mut().enumerate().take(len).skip(filled) {
            let dt = (m as f32 - last_cross).max(1.0);
            *v = f_prev.min(self.fs2 / (2.0 * dt));
        }

        // --- 復調後のローパス ---
        let mut post = self.post_lp;
        for v in freq.iter_mut().take(len) {
            *v = post.process(*v);
        }
        // --- 遅れを打ち消して間引く ---
        for (j, o) in out.iter_mut().enumerate() {
            let t = ((j + PRE_ROLL) * os) as f32 + self.delay;
            let i0 = (t.floor() as usize).min(len - 2);
            let fr = (t - i0 as f32).clamp(0.0, 1.0);
            *o = self.level_of(freq[i0] + (freq[i0 + 1] - freq[i0]) * fr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::NTSC;

    fn channel(noise: f32) -> FmChannel {
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
        })
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
            ch.process_line(&input, &[], &[], 1.0, &[], &mut rng, &mut out);
            let (mean, sd, _, _) = stats(&out[100..800]);
            assert!((mean - level).abs() < 0.5 && sd < 0.3, "level {level}: mean {mean} sd {sd}");
        }
    }
}
