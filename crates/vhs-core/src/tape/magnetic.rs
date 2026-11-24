//! 磁性体の伝達特性 (記録電流 → 残留磁化)。テープに記録するすべての信号に掛ける。
//!
//! - 低域変換クロマ: FM 輝度信号を交流バイアスとして直接記録されるため、記録レベルが大きいと
//!   磁化が飽和して振幅が圧縮される ([`MagneticTransfer::amplitude`]、複素包絡線に掛ける)
//! - FM 輝度の RF: 搬送波 + 低域変換クロマの瞬時値に掛ける ([`MagneticTransfer::transfer`])。
//!   FM なので輝度のレベルは振幅の圧縮では変わらないが、搬送波とクロマの相互変調積が
//!   輝度帯域に落ちてモアレになる
//! - リニア音声トラック: 交流バイアス録音の音声波形の瞬時値に掛ける (テープコンプレッション)
//!
//! 現状は無ヒステリシス曲線 (tanh) で近似している。
//! 将来は Jiles–Atherton モデル等のヒステリシス付きモデルをここに追加する
//! (状態を持つため `MagneticState` をライン単位で保持する設計にしてある)。

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

    /// 飽和レベル
    pub fn saturation(&self) -> f32 {
        self.sat
    }

    /// 瞬時値 `x` (符号付き) の伝達。小信号の傾きは 1
    #[inline(always)]
    pub fn transfer(&self, x: f32) -> f32 {
        match self.model {
            MagneticModel::Linear => x,
            MagneticModel::Tanh => self.sat * fast_tanh(x / self.sat),
        }
    }

    /// 包絡線振幅 `a` (≥0) に対する再生振幅
    #[inline(always)]
    pub fn amplitude(&self, a: f32) -> f32 {
        match self.model {
            MagneticModel::Linear => a,
            MagneticModel::Tanh => self.sat * fast_tanh(a / self.sat),
        }
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
