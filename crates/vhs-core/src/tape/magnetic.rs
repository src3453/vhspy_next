//! 磁性体の伝達特性。
//!
//! 低域変換クロマは FM 輝度信号を交流バイアスとして直接記録されるため、
//! 記録レベルが大きいと磁化が飽和して振幅が圧縮される。
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
    pub fn new(model: MagneticModel, saturation_ire: f32) -> Self {
        Self { model, sat: saturation_ire.max(1.0) }
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
