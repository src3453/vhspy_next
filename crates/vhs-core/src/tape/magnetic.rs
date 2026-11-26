//! 磁性体の伝達特性 (記録電流 → 残留磁化)。テープに記録するすべての信号に掛ける。
//!
//! - 低域変換クロマ: FM 輝度信号を交流バイアスとして直接記録されるため、記録レベルが大きいと
//!   磁化が飽和して振幅が圧縮される ([`MagneticTransfer::amplitude`]、複素包絡線に掛ける)
//! - FM 輝度の RF: 搬送波 + 低域変換クロマの瞬時値に掛ける ([`MagneticTransfer::state`])。
//!   FM なので輝度のレベルは振幅の圧縮では変わらないが、搬送波とクロマの相互変調積が
//!   輝度帯域に落ちてモアレになる
//! - リニア音声トラック: 交流バイアス録音の音声波形の瞬時値に掛ける ([`MagneticTransfer::transfer`])
//!
//! モデルは 3 通り。
//!
//! - Linear: 飽和しない
//! - Tanh: 無ヒステリシスの飽和 (`saturation` が飽和の起点)
//! - Jiles–Atherton (Chowdhury, DAFx-19 の式 (6)〜(10)): 磁化の履歴を持つ。係数はテープの種類
//!   ([`MagneticMedia`]) ごとに保磁力と角形比 (Mr/Ms) に合わせて当てはめた値 (Ms = 1 に正規化)。
//!   記録磁界は保磁力 Hc を単位に指定する。
//!   - FM の RF は状態を持って 1 サンプルずつ解く (ラインごとに初期化し、同期の助走でループを落ち着かせる)
//!   - クロマの包絡線と音声は、バイアス (クロマには FM 搬送波、音声には交流バイアス) を掛けた J-A を
//!     前もって解いて作った表を引く。バイアスの 1 周期で平均した磁化が入力に対してどう曲がるか
//!     (バイアスが足りないとデッドゾーンと歪み、多すぎると感度が落ちる) を無記憶の曲線にしたもの

use crate::params::{MagneticMedia, MagneticModel};
use std::sync::{Arc, Mutex};

/// Jiles–Atherton の係数 (Ms = 1 に正規化)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JaCoeffs {
    /// 無履歴磁化曲線の形 (Langevin 関数の幅)
    pub a: f32,
    /// ピン止め (ループの幅、おおよそ保磁力)
    pub k: f32,
    /// 磁区間の結合 (平均場)。大きいほどループが角張る
    pub alpha: f32,
    /// 可逆な磁化の割合
    pub c: f32,
    /// この係数で ±3 Hc の主ループを描いたときの保磁力 (記録磁界の単位)
    pub hc: f32,
}

impl JaCoeffs {
    /// テープの種類ごとの係数。保磁力 Hc / Ms と角形比 Mr/Mmax (±3 Hc の主ループ) を目安に当てはめ、
    /// 丸めた係数での主ループは: コバルト被着酸化鉄 (VHS・ベータ) Hc 0.342 Ms・角形比 0.85、
    /// 高保磁力コバルト (S-VHS) 0.397 / 0.895、メタル (8mm・Hi8) 0.374 / 0.917。
    /// どれも実際のビデオテープの角形比 (0.8〜0.9) の範囲で、メタルほど角張る
    pub fn of(media: MagneticMedia) -> Self {
        match media {
            MagneticMedia::Cobalt => Self { a: 0.1373, k: 0.4228, alpha: 0.2255, c: 0.0004, hc: 0.3417 },
            MagneticMedia::HighCobalt => Self { a: 0.2228, k: 0.4652, alpha: 0.6278, c: 0.0093, hc: 0.3971 },
            MagneticMedia::MetalParticle => Self { a: 0.1757, k: 0.4448, alpha: 0.5165, c: 0.0, hc: 0.3740 },
            // Chowdhury (2019) の音声用酸化鉄テープ (Ms = 350 kA/m, a = 22 kA/m, k = 27 kA/m)。角形比は 0.26 と低い
            MagneticMedia::FerricAudio => {
                Self { a: 22_000.0 / 350_000.0, k: 27_000.0 / 350_000.0, alpha: 1.6e-3, c: 0.17, hc: 0.0578 }
            }
        }
    }
}

/// FM の RF の出力を正規化する基準の記録磁界 [Hc]。このとき搬送波の基本波の振幅が 1 になる
const NOMINAL_RF_DRIVE: f32 = 2.0;
/// 音声のフルスケール (1.0) に対する信号磁界 [Hc]
const AUDIO_SIGNAL_HC: f32 = 1.2;
/// 表の点数
const LUT_N: usize = 65;
/// 表を引く音声の入力の上限 (録音レベル +18 dB でも収まる)
const AUDIO_MAX: f32 = 4.0;
/// 表を引くクロマの包絡線の上限 [IRE]
const CHROMA_MAX_IRE: f32 = 200.0;

#[derive(Clone, Debug)]
pub struct MagneticTransfer {
    model: MagneticModel,
    sat: f32,
    ja: JaCoeffs,
    /// J-A: 入力 1 あたりの記録磁界 (Ms = 1 の単位)
    field_per_unit: f32,
    /// J-A: 磁化 → 出力の倍率
    out_scale: f32,
    /// J-A の無記憶近似の表 (0..=lut_max を等分。奇関数として負側に広げる)
    lut: Option<Arc<[f32]>>,
    lut_max: f32,
}

impl MagneticTransfer {
    /// `saturation` は飽和の起点 (信号と同じ単位)。J-A でも表を作らない (tanh で近似する) 汎用の形
    pub fn new(model: MagneticModel, saturation: f32) -> Self {
        Self {
            model,
            sat: saturation.max(1e-3),
            ja: JaCoeffs::of(MagneticMedia::default()),
            field_per_unit: 0.0,
            out_scale: 1.0,
            lut: None,
            lut_max: 1.0,
        }
    }

    /// FM の RF 用。入力は搬送波の振幅 = 1 の尺度。J-A では搬送波の振幅 1 が `drive_hc` × Hc の記録磁界になり、
    /// 出力は記録磁界が [`NOMINAL_RF_DRIVE`] のときの搬送波の基本波が 1 になるように正規化する
    /// (記録電流が足りないと再生 RF が弱く、多すぎると飽和して伸びない)
    pub fn rf(model: MagneticModel, media: MagneticMedia, saturation: f32, drive_hc: f32) -> Self {
        let mut t = Self::new(model, saturation);
        if model == MagneticModel::JilesAtherton {
            t.ja = JaCoeffs::of(media);
            t.field_per_unit = drive_hc.max(0.01) * t.ja.hc;
            t.out_scale = 1.0 / nominal_fundamental(t.ja).max(1e-3);
        }
        t
    }

    /// 低域変換クロマの包絡線用 (入力は IRE)。J-A では FM 搬送波 (記録磁界 `drive_hc` × Hc) をバイアスとして、
    /// 搬送波に対して `carrier_per_ire` の振幅で記録した色の振幅の圧縮を表にする。小信号の利得は 1
    pub fn chroma(
        model: MagneticModel,
        media: MagneticMedia,
        saturation_ire: f32,
        drive_hc: f32,
        carrier_per_ire: f32,
    ) -> Self {
        let mut t = Self::new(model, saturation_ire);
        if model == MagneticModel::JilesAtherton {
            let ja = JaCoeffs::of(media);
            let g = carrier_per_ire.max(1e-4);
            let field = drive_hc.max(0.01) * ja.hc;
            let key = cache_key(0, media, &[drive_hc, g]);
            t.ja = ja;
            t.lut = Some(cached(key, || chroma_lut(ja, field, g)));
            t.lut_max = CHROMA_MAX_IRE;
        }
        t
    }

    /// リニア音声トラック用 (フルスケール 1)。J-A ではフルスケールが [`AUDIO_SIGNAL_HC`] × Hc の信号磁界、
    /// 交流バイアスが `bias_hc` × Hc の記録をバイアスの 1 周期で平均した曲線を表にする
    pub fn audio(model: MagneticModel, media: MagneticMedia, bias_hc: f32) -> Self {
        let mut t = Self::new(model, 1.0);
        if model == MagneticModel::JilesAtherton {
            let ja = JaCoeffs::of(media);
            let bias = bias_hc.max(0.0) * ja.hc;
            let key = cache_key(1, media, &[bias_hc]);
            t.ja = ja;
            t.lut = Some(cached(key, || {
                biased_curve(ja, bias, AUDIO_SIGNAL_HC * ja.hc, AUDIO_MAX).into_iter().collect()
            }));
            t.lut_max = AUDIO_MAX;
        }
        t
    }

    /// 線形 (飽和しない) モデルか
    pub fn is_linear(&self) -> bool {
        self.model == MagneticModel::Linear
    }

    /// FM RF で状態付き Jiles–Atherton 処理を使うか
    pub fn is_jiles_atherton(&self) -> bool {
        self.model == MagneticModel::JilesAtherton
    }

    /// サンプル列の処理に使う磁化状態を作る
    pub fn state(&self) -> MagneticState {
        MagneticState {
            model: self.model,
            sat: self.sat,
            ja: self.ja,
            field_per_unit: self.field_per_unit,
            out_scale: self.out_scale,
            magnetization: 0.0,
            previous_field: 0.0,
        }
    }

    /// 飽和レベル
    pub fn saturation(&self) -> f32 {
        self.sat
    }

    /// 瞬時値 `x` (符号付き) の伝達。Tanh は小信号の傾きが 1
    #[inline(always)]
    pub fn transfer(&self, x: f32) -> f32 {
        match self.model {
            MagneticModel::Linear => x,
            MagneticModel::JilesAtherton if self.lut.is_some() => x.signum() * self.lookup(x.abs()),
            // 表のない J-A は無記憶の tanh で近似する
            MagneticModel::Tanh | MagneticModel::JilesAtherton => self.sat * fast_tanh(x / self.sat),
        }
    }

    /// 包絡線振幅 `a` (≥0) に対する再生振幅
    #[inline(always)]
    pub fn amplitude(&self, a: f32) -> f32 {
        match self.model {
            MagneticModel::Linear => a,
            MagneticModel::JilesAtherton if self.lut.is_some() => self.lookup(a),
            MagneticModel::Tanh | MagneticModel::JilesAtherton => self.sat * fast_tanh(a / self.sat),
        }
    }

    /// 表の線形補間 (範囲外は最後の点で頭打ち)
    #[inline]
    fn lookup(&self, x: f32) -> f32 {
        let lut = self.lut.as_deref().unwrap_or(&[]);
        if lut.is_empty() {
            return x;
        }
        let pos = (x / self.lut_max * (lut.len() - 1) as f32).max(0.0);
        let i = (pos as usize).min(lut.len() - 2);
        let fr = (pos - i as f32).min(1.0);
        lut[i] + (lut[i + 1] - lut[i]) * fr
    }

    /// J-A の RF: 振幅 1 の搬送波に直流 `offset` (搬送波の尺度) を重ねて記録したときの、搬送波の基本波の振幅
    /// (出力の尺度)。Simple モデルで搬送波の圧縮と相互変調を見積もるのに使う
    pub fn carrier_fundamental(&self, offset: f32) -> f32 {
        let mut st = self.state();
        fundamental_of(|th| st.process(th.cos() + offset))
    }
}

/// 1 本の信号列に沿った磁化履歴
#[derive(Clone, Copy, Debug)]
pub struct MagneticState {
    model: MagneticModel,
    sat: f32,
    ja: JaCoeffs,
    field_per_unit: f32,
    out_scale: f32,
    magnetization: f32,
    previous_field: f32,
}

impl MagneticState {
    /// 記録信号の次サンプルを処理し、残留磁化を返す (入力と同じ尺度)
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        match self.model {
            MagneticModel::Linear => x,
            MagneticModel::Tanh => self.sat * fast_tanh(x / self.sat),
            MagneticModel::JilesAtherton => {
                let field = x * self.field_per_unit;
                self.magnetization = ja_step(self.ja, self.magnetization, self.previous_field, field);
                self.previous_field = field;
                self.out_scale * self.magnetization
            }
        }
    }
}

/// 磁界を h0 → h1 と動かしたときの磁化 (表づくり用の RK4)
#[inline]
fn ja_step(ja: JaCoeffs, m: f32, h0: f32, h1: f32) -> f32 {
    ja_step_rk::<4>(ja, m, h0, h1, 0.5)
}

/// H について Runge–Kutta で積分する (ORDER = 2: 中点法、4: 古典的 RK4)。
/// 角形比の高いテープは保磁力付近で dM/dH が急なので、ΔH を `max_dh_k` × k ごとに刻む (粗いと発散する)
#[inline]
fn ja_step_rk<const ORDER: usize>(ja: JaCoeffs, m: f32, h0: f32, h1: f32, max_dh_k: f32) -> f32 {
    let dh_total = h1 - h0;
    if dh_total == 0.0 {
        return m;
    }
    let steps = ((dh_total.abs() / (max_dh_k * ja.k)).ceil() as usize).clamp(1, 64);
    let dh = dh_total / steps as f32;
    let dir = dh.signum();
    let mut m = m;
    for s in 0..steps {
        let h = h0 + dh * s as f32;
        let mid = h + 0.5 * dh;
        let k1 = ja_dm_dh(ja, m, h, dir) * dh;
        let k2 = ja_dm_dh(ja, m + 0.5 * k1, mid, dir) * dh;
        m = if ORDER == 2 {
            m + k2
        } else {
            let k3 = ja_dm_dh(ja, m + 0.5 * k2, mid, dir) * dh;
            let k4 = ja_dm_dh(ja, m + k3, h + dh, dir) * dh;
            m + (k1 + 2.0 * k2 + 2.0 * k3 + k4) / 6.0
        }
        .clamp(-1.0, 1.0);
    }
    m
}

/// dM/dH (Holters & Zölzer の形。`dir` は H の動く向き = δ)
#[inline(always)]
fn ja_dm_dh(ja: JaCoeffs, m: f32, h: f32, dir: f32) -> f32 {
    let inv_a = 1.0 / ja.a;
    let q = (h + ja.alpha * m) * inv_a;
    let (man, l_prime) = langevin_pair(q);
    let dman_dh = l_prime * inv_a;
    let irreversible = if dir * (man - m) > 0.0 {
        let denominator = (1.0 - ja.c) * dir * ja.k - ja.alpha * (man - m);
        let denominator = if denominator.abs() < 1e-6 { 1e-6f32.copysign(denominator) } else { denominator };
        (1.0 - ja.c) * (man - m) / denominator
    } else {
        0.0
    };
    let feedback = (1.0 - ja.c * ja.alpha * dman_dh).max(1e-4);
    (irreversible + ja.c * dman_dh) / feedback
}

/// 1 周期を `STEPS` 点で何周期か回し、最後の 1 周期の基本波の振幅を返す
fn fundamental_of(mut f: impl FnMut(f32) -> f32) -> f32 {
    use std::f32::consts::TAU;
    const STEPS: usize = 48;
    const CYCLES: usize = 5;
    let (mut re, mut im) = (0f32, 0f32);
    for n in 0..STEPS * CYCLES {
        let th = TAU * (n % STEPS) as f32 / STEPS as f32;
        let y = f(th);
        if n >= STEPS * (CYCLES - 1) {
            re += y * th.cos();
            im += y * th.sin();
        }
    }
    2.0 * (re * re + im * im).sqrt() / STEPS as f32
}

/// 記録磁界が [`NOMINAL_RF_DRIVE`] × Hc の正弦波を記録したときの磁化の基本波の振幅
fn nominal_fundamental(ja: JaCoeffs) -> f32 {
    let field = NOMINAL_RF_DRIVE * ja.hc;
    let (mut m, mut h0) = (0f32, 0f32);
    fundamental_of(|th| {
        let h = field * th.cos();
        m = ja_step(ja, m, h0, h);
        h0 = h;
        m
    })
}

/// 交流バイアス (振幅 `bias`、磁界の単位) に直流の入力 x (0..=x_max、1 あたり `field_per_unit`) を重ねて、
/// バイアスの 1 周期で平均した磁化 T(x) の表 ([`LUT_N`] 点)
fn biased_curve(ja: JaCoeffs, bias: f32, field_per_unit: f32, x_max: f32) -> Vec<f32> {
    use std::f32::consts::TAU;
    const STEPS: usize = 32;
    const CYCLES: usize = 6;
    (0..LUT_N)
        .map(|i| {
            let c = x_max * i as f32 / (LUT_N - 1) as f32 * field_per_unit;
            let (mut m, mut h0, mut acc) = (0f32, 0f32, 0f32);
            for n in 0..STEPS * CYCLES {
                let h = c + bias * (TAU * n as f32 / STEPS as f32).cos();
                m = ja_step(ja, m, h0, h);
                h0 = h;
                // 最後の 2 周期で平均する
                if n >= STEPS * (CYCLES - 2) {
                    acc += m;
                }
            }
            acc / (2 * STEPS) as f32
        })
        .collect()
}

/// クロマの包絡線の表 [IRE → IRE]。バイアス付きの曲線 T に振幅 A の正弦波を通した基本波の振幅を、
/// 小信号の利得が 1 になるように正規化する
fn chroma_lut(ja: JaCoeffs, bias: f32, carrier_per_ire: f32) -> Arc<[f32]> {
    use std::f32::consts::PI;
    let x_max = CHROMA_MAX_IRE * carrier_per_ire;
    let curve = biased_curve(ja, bias, bias, x_max);
    let t = |x: f32| {
        let pos = (x.abs() / x_max * (LUT_N - 1) as f32).min((LUT_N - 1) as f32);
        let i = (pos as usize).min(LUT_N - 2);
        let fr = pos - i as f32;
        x.signum() * (curve[i] + (curve[i + 1] - curve[i]) * fr)
    };
    // 基本波 (2/π) ∫₀^π T(A sin φ) sin φ dφ (中点則)
    const Q: usize = 64;
    let fundamental = |a: f32| {
        (0..Q)
            .map(|j| {
                let phi = PI * (j as f32 + 0.5) / Q as f32;
                t(a * phi.sin()) * phi.sin()
            })
            .sum::<f32>()
            * 2.0
            / Q as f32
    };
    // 小信号の利得: 最初の格子点 (と同じ大きさの振幅) で測る
    let a1 = x_max / (LUT_N - 1) as f32;
    let gain0 = (fundamental(a1) / a1).max(1e-6);
    (0..LUT_N)
        .map(|i| {
            let a = x_max * i as f32 / (LUT_N - 1) as f32;
            fundamental(a) / gain0 / carrier_per_ire
        })
        .collect()
}

fn cache_key(kind: u8, media: MagneticMedia, values: &[f32]) -> Vec<u32> {
    let mut k = vec![kind as u32, media as u32];
    k.extend(values.iter().map(|v| v.to_bits()));
    k
}

/// 表は作るのに数 ms かかるので、最近使ったものを取っておく (パラメータを動かしている間も作り直さない)
fn cached(key: Vec<u32>, build: impl FnOnce() -> Arc<[f32]>) -> Arc<[f32]> {
    static CACHE: Mutex<Vec<(Vec<u32>, Arc<[f32]>)>> = Mutex::new(Vec::new());
    if let Some((_, v)) = CACHE.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return v.clone();
    }
    let v = build();
    let mut cache = CACHE.lock().unwrap();
    if cache.len() >= 8 {
        cache.remove(0);
    }
    cache.push((key, v.clone()));
    v
}

/// Langevin 関数 L(x) = coth x - 1/x と導関数 L'(x) = 1/x² - coth² x + 1 を、exp 1 回で同時に求める
#[inline(always)]
fn langevin_pair(x: f32) -> (f32, f32) {
    let ax = x.abs();
    let x2 = x * x;
    if ax < 0.1 {
        let l = x * (1.0 / 3.0 - x2 / 45.0 + 2.0 * x2 * x2 / 945.0);
        (l, 1.0 / 3.0 - x2 / 15.0 + 2.0 * x2 * x2 / 189.0)
    } else if ax > 10.0 {
        (x.signum() - 1.0 / x, 1.0 / x2)
    } else {
        // coth |x| = 1 + 2 / (e^{2|x|} - 1)
        let coth_abs = 1.0 + 2.0 / ((2.0 * ax).exp() - 1.0);
        let inv = 1.0 / x;
        (coth_abs.copysign(x) - inv, inv * inv - coth_abs * coth_abs + 1.0)
    }
}

#[cfg(test)]
#[inline(always)]
fn langevin(x: f32) -> f32 {
    let x2 = x * x;
    if x.abs() < 0.1 {
        x * (1.0 / 3.0 - x2 / 45.0 + 2.0 * x2 * x2 / 945.0)
    } else if x.abs() > 10.0 {
        x.signum() - 1.0 / x
    } else {
        1.0 / x.tanh() - 1.0 / x
    }
}

#[cfg(test)]
#[inline(always)]
fn langevin_prime(x: f32) -> f32 {
    let x2 = x * x;
    if x.abs() < 0.1 {
        1.0 / 3.0 - x2 / 15.0 + 2.0 * x2 * x2 / 189.0
    } else if x.abs() > 10.0 {
        1.0 / x2
    } else {
        let coth = 1.0 / x.tanh();
        1.0 / x2 - coth * coth + 1.0
    }
}

#[inline(always)]
fn fast_tanh(x: f32) -> f32 {
    // Padé 近似 (|x| < 3 で誤差 < 2%)、それ以上は飽和
    if x.abs() >= 3.0 {
        return x.signum();
    }
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ±3 Hc の主ループの (最大磁化, 残留磁化, 保磁力)
    fn major_loop(ja: JaCoeffs) -> (f32, f32, f32) {
        use std::f32::consts::TAU;
        let (n, cycles) = (512usize, 3usize);
        let hpk = 3.0 * ja.hc;
        let (mut m, mut h0) = (0f32, 0f32);
        let (mut mmax, mut mr, mut hc) = (0f32, 0f32, 0f32);
        let mut prev_m = 0f32;
        for i in 0..n * cycles {
            let h = hpk * (TAU * i as f32 / n as f32).sin();
            m = ja_step(ja, m, h0, h);
            if i >= n * (cycles - 1) {
                mmax = mmax.max(m);
                if (h0 > 0.0) != (h > 0.0) {
                    mr = mr.max(m.abs());
                }
                if (prev_m > 0.0) != (m > 0.0) {
                    hc = hc.max(h.abs());
                }
            }
            prev_m = m;
            h0 = h;
        }
        (mmax, mr, hc)
    }

    #[test]
    fn media_loops_match_fitted_targets() {
        for (media, square) in
            [(MagneticMedia::Cobalt, 0.850), (MagneticMedia::HighCobalt, 0.895), (MagneticMedia::MetalParticle, 0.917)]
        {
            let ja = JaCoeffs::of(media);
            let (mmax, mr, hc) = major_loop(ja);
            assert!((mr / mmax - square).abs() < 0.02, "{media:?}: squareness {}", mr / mmax);
            assert!((hc / ja.hc - 1.0).abs() < 0.03, "{media:?}: hc {hc} vs {}", ja.hc);
        }
    }

    /// exp 1 回で求める Langevin 関数と導関数が、tanh で求めた値と合う
    #[test]
    fn langevin_pair_matches_reference() {
        for i in -300..=300 {
            let x = i as f32 * 0.05 + 0.0123;
            let (l, lp) = langevin_pair(x);
            assert!((l - langevin(x)).abs() < 1e-5, "L({x}): {l} vs {}", langevin(x));
            assert!((lp - langevin_prime(x)).abs() < 1e-4, "L'({x}): {lp} vs {}", langevin_prime(x));
        }
    }

    /// 粗い刻み (FM の RF: 1 周期 13 サンプル) でも発散せず、細かい刻みと同じ振幅になる
    #[test]
    fn coarse_steps_stay_stable() {
        use std::f32::consts::TAU;
        let ja = JaCoeffs::of(MagneticMedia::MetalParticle);
        let peak = |n: usize| {
            let (mut m, mut h0, mut mx) = (0f32, 0f32, 0f32);
            for i in 0..n * 6 {
                let h = 4.0 * ja.hc * (TAU * i as f32 / n as f32).sin();
                m = ja_step(ja, m, h0, h);
                h0 = h;
                if i >= n * 5 {
                    mx = mx.max(m);
                }
            }
            mx
        };
        let (coarse, fine) = (peak(13), peak(1024));
        assert!(coarse < 0.99 && (coarse - fine).abs() < 0.03, "coarse {coarse}, fine {fine}");
    }

    /// RF: 記録磁界が基準のとき搬送波の基本波は 1。弱いと小さく、強いと飽和して伸びない
    #[test]
    fn rf_drive_sets_carrier_level() {
        let rf = |d: f32| {
            MagneticTransfer::rf(MagneticModel::JilesAtherton, MagneticMedia::Cobalt, 1.0, d).carrier_fundamental(0.0)
        };
        let (lo, nom, hi) = (rf(0.7), rf(NOMINAL_RF_DRIVE), rf(4.0));
        assert!((nom - 1.0).abs() < 0.02, "{nom}");
        assert!(lo < 0.5 * nom, "{lo}");
        assert!(hi > nom && hi < 2.0 * nom, "{hi}");
    }

    /// 音声: バイアスがないと小信号が通らない (デッドゾーン)。適正なバイアスでほぼ直線、足りないと早く飽和する
    #[test]
    fn audio_bias_linearizes() {
        let t = |bias: f32, x: f32| MagneticTransfer::audio(MagneticModel::JilesAtherton, MagneticMedia::Cobalt, bias).transfer(x);
        let lin = |bias: f32| (t(bias, 0.5) / 0.5) / (t(bias, 0.1) / 0.1);
        assert!(t(0.0, 0.1) / 0.1 < 0.2 * t(2.0, 0.1) / 0.1, "dead zone");
        assert!(lin(2.0) > 0.95, "optimal bias linear: {}", lin(2.0));
        assert!(lin(0.5) < lin(2.0), "under-bias compresses: {} vs {}", lin(0.5), lin(2.0));
        assert!(t(2.0, -0.3) == -t(2.0, 0.3));
    }

    /// クロマ: 小信号の利得は 1、大きいと圧縮される。搬送波 (バイアス) が強いほど、
    /// 色の記録レベルが低いほど直線に近い (既定の記録レベルでは圧縮は数 % にとどまる)
    #[test]
    fn chroma_envelope_unity_then_compresses() {
        let chroma = |level: f32, drive: f32| {
            MagneticTransfer::chroma(MagneticModel::JilesAtherton, MagneticMedia::Cobalt, 50.0, drive, level / 20.0)
        };
        let ratio = |m: &MagneticTransfer, a: f32| m.amplitude(a) / a;
        let hot = chroma(0.3, 2.0);
        assert!((ratio(&hot, 5.0) - 1.0).abs() < 0.03, "{}", ratio(&hot, 5.0));
        assert!(ratio(&hot, 150.0) < 0.85, "hot {}", ratio(&hot, 150.0));
        let normal = chroma(0.1, 2.0);
        // 既定の記録レベルでは、無履歴曲線の S 字でわずかに持ち上がってから圧縮が始まる程度
        let r = ratio(&normal, 150.0);
        assert!(r > ratio(&hot, 150.0) && (0.9..1.05).contains(&r), "normal {r}");
        assert!(ratio(&chroma(0.3, 3.0), 150.0) > ratio(&hot, 150.0), "more bias, more linear");
    }
}
