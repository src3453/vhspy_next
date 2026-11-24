//! 磁性体の伝達特性 (記録電流 → 残留磁化)。テープに記録するすべての信号に掛ける。
//!
//! - 低域変換クロマ: FM 輝度信号を交流バイアスとして直接記録されるため、記録レベルが大きいと
//!   磁化が飽和して振幅が圧縮される ([`MagneticTransfer::amplitude`]、複素包絡線に掛ける)
//! - FM 輝度の RF: 搬送波 + 低域変換クロマの瞬時値に掛ける ([`MagneticTransfer::transfer`])。
//!   FM なので輝度のレベルは振幅の圧縮では変わらないが、搬送波とクロマの相互変調積が
//!   輝度帯域に落ちてモアレになる
//! - リニア音声トラック: 交流バイアス録音の音声波形の瞬時値に掛ける (テープコンプレッション)
//!
//! tanh は無ヒステリシスの近似。Jiles–Atherton は FM RF 用に状態を持つモデルで、
//! ラインごとに磁化状態を初期化し、同期キャリアの助走区間でループを落ち着かせる。

use crate::params::MagneticModel;

#[derive(Clone, Copy, Debug)]
pub struct MagneticTransfer {
    model: MagneticModel,
    sat: f32,
}

impl MagneticTransfer {
    /// `saturation` は飽和の起点 (信号と同じ単位)
    pub fn new(model: MagneticModel, saturation: f32) -> Self {
        Self { model, sat: saturation.max(1e-3) }
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
    pub fn state(self) -> MagneticState {
        MagneticState { model: self.model, sat: self.sat, magnetization: 0.0, previous_field: 0.0 }
    }

    /// 飽和レベル
    pub fn saturation(&self) -> f32 {
        self.sat
    }

    /// 瞬時値 `x` (符号付き) の伝達。小信号の傾きは 1
    #[inline(always)]
    pub fn transfer(&self, x: f32) -> f32 {
        match self.model {
            MagneticModel::Linear => x,
            // Envelope/simple/audio paths have no per-sample J-A state. Keep their
            // existing memoryless approximation; the FM RF path uses `state()`.
            MagneticModel::Tanh | MagneticModel::JilesAtherton => self.sat * fast_tanh(x / self.sat),
        }
    }

    /// 包絡線振幅 `a` (≥0) に対する再生振幅
    #[inline(always)]
    pub fn amplitude(&self, a: f32) -> f32 {
        match self.model {
            MagneticModel::Linear => a,
            MagneticModel::Tanh | MagneticModel::JilesAtherton => self.sat * fast_tanh(a / self.sat),
        }
    }
}

/// 1 本の信号列に沿った磁化履歴。J-A の係数は論文のフェライト系テープ値を
/// Ms=1 に正規化した初期値で、VHS テープに対する測定済み定数ではない。
#[derive(Clone, Copy, Debug)]
pub struct MagneticState {
    model: MagneticModel,
    sat: f32,
    magnetization: f32,
    previous_field: f32,
}

impl MagneticState {
    /// 記録信号の次サンプルを処理し、残留磁化を返す (入力と同じ尺度)
    #[inline]
    pub fn process(&mut self, x: f32, sample_rate: f32) -> f32 {
        match self.model {
            MagneticModel::Linear => x,
            MagneticModel::Tanh => self.sat * fast_tanh(x / self.sat),
            MagneticModel::JilesAtherton => self.process_jiles_atherton(x, sample_rate),
        }
    }

    #[inline]
    fn process_jiles_atherton(&mut self, x: f32, sample_rate: f32) -> f32 {
        // Map the existing saturation control to a normalized field. At x=sat,
        // the anhysteretic curve is about 75% of Ms (H/a = 4).
        let a = 22_000.0 / 350_000.0;
        let field = (x / self.sat) * (4.0 * a);
        let dt = 1.0 / sample_rate.max(1.0);
        let field_rate = (field - self.previous_field) / dt;
        let mid_field = 0.5 * (self.previous_field + field);
        let m = self.magnetization;

        let k1 = ja_rate(m, self.previous_field, field_rate) * dt;
        let k2 = ja_rate(m + 0.5 * k1, mid_field, field_rate) * dt;
        let k3 = ja_rate(m + 0.5 * k2, mid_field, field_rate) * dt;
        let k4 = ja_rate(m + k3, field, field_rate) * dt;
        self.magnetization = (m + (k1 + 2.0 * k2 + 2.0 * k3 + k4) / 6.0).clamp(-1.0, 1.0);
        self.previous_field = field;
        self.sat * self.magnetization
    }
}

// Jiles-Atherton coefficients from Chowdhury (2019), normalized by Ms.
const JA_MS: f32 = 1.0;
const JA_A: f32 = 22_000.0 / 350_000.0;
const JA_K: f32 = 27_000.0 / 350_000.0;
const JA_ALPHA: f32 = 1.6e-3;
const JA_C: f32 = 0.17;

#[inline(always)]
fn ja_rate(m: f32, h: f32, h_rate: f32) -> f32 {
    if h_rate.abs() < 1e-6 {
        return 0.0;
    }

    let q = (h + JA_ALPHA * m) / JA_A;
    let man = JA_MS * langevin(q);
    let dman_dh = (JA_MS / JA_A) * langevin_prime(q);
    let direction = h_rate.signum();
    let irreversible = if direction * (man - m) > 0.0 {
        let denominator = (1.0 - JA_C) * direction * JA_K - JA_ALPHA * (man - m);
        let denominator = if denominator.abs() < 1e-6 {
            if denominator < 0.0 { -1e-6 } else { 1e-6 }
        } else {
            denominator
        };
        (1.0 - JA_C) * (man - m) / denominator
    } else {
        0.0
    };
    let feedback = (1.0 - JA_C * JA_ALPHA * dman_dh).max(1e-4);
    h_rate * (irreversible + JA_C * dman_dh) / feedback
}

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
