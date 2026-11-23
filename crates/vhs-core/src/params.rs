//! シミュレーションパラメータ。GUI とは JSON (serde) でやり取りする。
//! 全フィールドに `#[serde(default)]` を付け、古いプリセットも読めるようにしている。

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Params {
    pub encoder: EncoderParams,
    pub tape: TapeParams,
    pub transport: TransportParams,
    pub decoder: DecoderParams,
    pub output: OutputParams,
    /// 乱数シード (同じシードなら同じ劣化パターン)
    pub seed: u64,
}

/// 放送/カメラ側の NTSC エンコーダ
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EncoderParams {
    /// 輝度帯域 [MHz]
    pub luma_bw_mhz: f32,
    /// I 信号帯域 [MHz]
    pub i_bw_mhz: f32,
    /// Q 信号帯域 [MHz]
    pub q_bw_mhz: f32,
    /// 色信号の振幅倍率
    pub chroma_gain: f32,
}

impl Default for EncoderParams {
    fn default() -> Self {
        Self { luma_bw_mhz: 4.2, i_bw_mhz: 1.3, q_bw_mhz: 0.6, chroma_gain: 1.0 }
    }
}

/// 磁性体の伝達モデル
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MagneticModel {
    /// 線形 (飽和なし)
    Linear,
    /// tanh による無ヒステリシス飽和
    #[default]
    Tanh,
}

/// 記録・テープ媒体・再生の信号経路
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TapeParams {
    /// false ならテープを通さない (放送をそのまま受像)
    pub enabled: bool,

    // --- 輝度 (FM 記録) 経路 ---
    /// 輝度記録帯域 [MHz] (VHS SP ≒ 3.0, LP/EP はさらに狭い)
    pub luma_bw_mhz: f32,
    /// 記録エンファシス量 [dB]
    pub emphasis_db: f32,
    /// エンファシスのコーナー周波数 [MHz]
    pub emphasis_mhz: f32,
    /// エンファシス後のホワイトクリップ [IRE]
    pub white_clip_ire: f32,
    /// FM 過変調で反転が起きる閾値 [IRE] (ベアディング)
    pub bearding_threshold_ire: f32,
    /// ベアディングの強さ
    pub bearding_strength: f32,
    /// FM 復調ノイズ (rms) [IRE]
    pub luma_noise_ire: f32,
    /// 再生ピーキング [dB] (負で甘く、正でシャープ)
    pub sharpness_db: f32,

    // --- 色 (低域変換記録) 経路 ---
    /// 低域変換クロマの帯域 (片側) [kHz]
    pub chroma_bw_khz: f32,
    /// クロマ AM ノイズ (rms) [IRE]
    pub chroma_noise_ire: f32,
    /// 再生時のクロマ位相誤差 (rms) [deg]
    pub chroma_phase_noise_deg: f32,
    /// 輝度に対するクロマの遅延 [サンプル] (Y/C 遅延誤差)
    pub chroma_delay_samples: i32,
    /// 磁性体モデル
    pub magnetic_model: MagneticModel,
    /// 磁気飽和の起点となるクロマ振幅 [IRE]
    pub saturation_ire: f32,

    // --- ドロップアウト ---
    /// 1 ラインあたりのドロップアウト発生確率
    pub dropout_rate: f32,
    /// ドロップアウト平均長 [サンプル]
    pub dropout_length: f32,
    /// ドロップアウト補償 (1H 前のラインで置換)
    pub dropout_compensation: bool,
}

impl Default for TapeParams {
    fn default() -> Self {
        Self {
            enabled: true,
            luma_bw_mhz: 3.0,
            emphasis_db: 10.0,
            emphasis_mhz: 0.6,
            white_clip_ire: 150.0,
            bearding_threshold_ire: 180.0,
            bearding_strength: 1.0,
            luma_noise_ire: 2.0,
            sharpness_db: 0.0,
            chroma_bw_khz: 500.0,
            chroma_noise_ire: 1.0,
            chroma_phase_noise_deg: 3.0,
            chroma_delay_samples: 3,
            magnetic_model: MagneticModel::Tanh,
            saturation_ire: 50.0,
            dropout_rate: 0.002,
            dropout_length: 40.0,
            dropout_compensation: true,
        }
    }
}

/// テープ走行系・ヘッド (時間軸誤差)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TransportParams {
    /// ライン単位の時間軸ジッタ (rms) [サンプル]
    pub jitter_samples: f32,
    /// ジッタのライン間相関 (0..1)
    pub jitter_correlation: f32,
    /// ワウ・フラッタ振幅 [サンプル]
    pub wow_samples: f32,
    /// ワウ・フラッタ周波数 [Hz]
    pub wow_hz: f32,
    /// ヘッド切替ノイズが及ぶライン数 (画面下端)
    pub head_switch_lines: usize,
    /// ヘッド切替による水平ずれ [サンプル]
    pub head_switch_offset: f32,
    /// ヘッド切替ラインのノイズ量
    pub head_switch_noise: f32,
    /// 画面上部のスキュー (テープ張力誤差) [サンプル]
    pub skew_samples: f32,
    /// トラッキングずれ (0..1)。ノイズバーが現れる
    pub tracking_error: f32,
    /// ノイズバーの移動速度 [ライン/フィールド]
    pub tracking_speed: f32,
}

impl Default for TransportParams {
    fn default() -> Self {
        Self {
            jitter_samples: 0.4,
            jitter_correlation: 0.9,
            wow_samples: 0.0,
            wow_hz: 8.0,
            head_switch_lines: 5,
            head_switch_offset: 10.0,
            head_switch_noise: 1.0,
            skew_samples: 0.0,
            tracking_error: 0.0,
            tracking_speed: 1.5,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum YcSeparation {
    /// トラップ (ノッチ) フィルタ。ドットクロールとクロスカラーが出る
    #[default]
    Notch,
    /// 1H ラインくし形フィルタ
    Comb,
}

/// 受像機 (NTSC デコーダ)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DecoderParams {
    pub yc_separation: YcSeparation,
    /// 復調後の色差帯域 [MHz]
    pub chroma_bw_mhz: f32,
    /// 自動色レベル調整 (バースト振幅で正規化)
    pub acc: bool,
    /// バーストが弱いときに色を消す
    pub color_killer: bool,
    /// 水平 AFC の追従係数 (0 = 追従しない, 1 = 即時追従)
    pub h_pll: f32,
    pub contrast: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub hue_deg: f32,
}

impl Default for DecoderParams {
    fn default() -> Self {
        Self {
            yc_separation: YcSeparation::Notch,
            chroma_bw_mhz: 1.3,
            acc: true,
            color_killer: true,
            h_pll: 0.25,
            contrast: 1.0,
            brightness: 0.0,
            saturation: 1.0,
            hue_deg: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Deinterlace {
    /// 2 フィールドを交互に織り込む (インターレースのまま)
    #[default]
    Weave,
    /// 現フィールドのラインを 2 倍に引き伸ばす
    Bob,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OutputParams {
    pub deinterlace: Deinterlace,
}
