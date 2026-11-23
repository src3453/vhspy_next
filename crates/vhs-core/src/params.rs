//! シミュレーションパラメータ。GUI とは JSON (serde) でやり取りする。
//! 全フィールドに `#[serde(default)]` を付け、古いプリセットも読めるようにしている。

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Params {
    /// 撮像管カメラ (入力映像の直後)
    pub camera: CameraParams,
    pub encoder: EncoderParams,
    pub tape: TapeParams,
    pub transport: TransportParams,
    /// VCR → TV の RF (アンテナ) 接続
    pub rf: RfParams,
    pub decoder: DecoderParams,
    /// デジタルキャプチャ (ADC・色差間引き・圧縮)
    pub capture: CaptureParams,
    /// 音声トラック (書き出し時に適用)
    pub audio: AudioParams,
    pub output: OutputParams,
    pub osd: OsdParams,
    /// 乱数シード (同じシードなら同じ劣化パターン)
    pub seed: u64,
}

/// 撮像管 (ビジコン・プランビコン・サチコンなど) を使ったカラーカメラ。
/// 入力画像を「被写体の光」とみなし、3 管式カメラの光学系 → 光導電ターゲット → 電子ビーム読み出し →
/// 映像増幅・プロセス回路を通す。モデルの詳細は `camera` モジュールを参照。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CameraParams {
    pub enabled: bool,

    // --- 光学系 ---
    /// 露出 (アイリス) [EV]
    pub exposure_ev: f32,
    /// 色分解プリズムの混色 (隣の色の漏れ込み, 0..0.3)
    pub crosstalk: f32,
    /// レンズ・プリズムのフレア (画面平均の光が全体にかぶる量)。赤管は 2 倍
    pub flare: f32,
    /// レジストレーション誤差 [画素] (RGB 3 管の像の重ね合わせずれ)
    pub registration_px: f32,
    /// 周辺の感度低下 (シェーディング) 0..0.5
    pub shading: f32,
    /// レンズ・ターゲット・ビームスポットによるぼけ [画素]
    pub resolution_px: f32,

    // --- 光導電ターゲット ---
    /// ターゲットの γ (光量 → 信号電流)。ビジコン ≒ 0.65、プランビコン ≒ 0.95、サチコン ≒ 1.0
    pub target_gamma: f32,
    /// 暗電流 (白レベル比)
    pub dark_current: f32,
    /// 光導電性残像の量 (0..1)。トラップによる遅い応答
    pub photo_lag: f32,
    /// 光導電性残像の減衰時定数 [フィールド] (立ち上がりはこの半分)
    pub photo_lag_fields: f32,
    /// 容量性 (ビーム) 残像の強さ。低照度ほど読み残しが増える
    pub capacitive_lag: f32,
    /// バイアス光 (白レベル比)。残像を減らす
    pub bias_light: f32,
    /// ビーム電流の上限 (白レベル比)。超えた電荷は読み残されてコメットテールになる
    pub beam_current: f32,
    /// ターゲットに蓄えられる電荷の上限 (白レベル比)
    pub target_saturation: f32,
    /// ACT (アンチコメットテール): 帰線期間に過大な電荷を放電する
    pub act: bool,
    /// ACT が働き始めるレベル (白レベル比)
    pub act_level: f32,
    /// ターゲットのヒステリシス幅 (白レベル比)。明→暗と暗→明で残る像が変わる
    pub hysteresis: f32,
    /// ヒステリシスが解ける時定数 [フィールド]
    pub hysteresis_fields: f32,
    /// 焼き付きの進む速さ [/秒] (白を超えた部分の感度が落ちる)
    pub burn_rate: f32,
    /// 焼き付きが回復する速さ [/秒]
    pub burn_recovery: f32,
    /// ターゲットのしみ・きず (ブレミッシュ) の数
    pub blemishes: u32,

    // --- 映像増幅 (真空管プリアンプ) ---
    /// 非線形性の強さ (0 = 線形)
    pub amp_drive: f32,
    /// 正負の非対称性 (-1..1)。正で白側、負で黒側がより圧縮される
    pub amp_asymmetry: f32,
    /// プリアンプ雑音 (rms) [%]。周波数が高いほど大きい三角雑音
    pub noise_pct: f32,

    // --- 3 管のばらつき・劣化 ---
    /// RGB 各撮像管の特性のばらつき (0..1)。γ・残像・感度・黒レベルなどがずれる
    pub mismatch: f32,
    /// 撮像管の劣化 (0..1)。感度低下・ビーム電流低下・残像増加・焼き付き・しみ・雑音
    pub aging: f32,

    // --- プロセス回路 ---
    /// 輪郭補償 (アパーチャ補正) の量
    pub aperture: f32,
    /// ニーの開始レベル (白レベル比)
    pub knee: f32,
    /// ニーより上の傾き
    pub knee_slope: f32,
    /// ホワイトクリップ (白レベル比)
    pub white_clip: f32,
    /// ガンマ補正 (出力 = 入力^γ)。受像管 γ=2.2 の逆数 ≒ 0.45
    pub camera_gamma: f32,
}

impl Default for CameraParams {
    fn default() -> Self {
        Self {
            enabled: false,
            exposure_ev: 0.0,
            crosstalk: 0.03,
            flare: 0.02,
            registration_px: 0.4,
            shading: 0.08,
            resolution_px: 0.7,
            target_gamma: 0.95,
            dark_current: 0.005,
            photo_lag: 0.05,
            photo_lag_fields: 6.0,
            capacitive_lag: 0.08,
            bias_light: 0.0,
            beam_current: 2.0,
            target_saturation: 12.0,
            act: false,
            act_level: 1.5,
            hysteresis: 0.0,
            hysteresis_fields: 30.0,
            burn_rate: 0.02,
            burn_recovery: 0.05,
            blemishes: 0,
            amp_drive: 0.3,
            amp_asymmetry: 0.4,
            noise_pct: 0.8,
            mismatch: 0.2,
            aging: 0.0,
            aperture: 0.5,
            knee: 0.9,
            knee_slope: 0.3,
            white_clip: 1.08,
            camera_gamma: 0.45,
        }
    }
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

    // --- テープの物理的な傷み ---
    /// しわ・折れ目 (クリース) の発生頻度 [回/分]
    pub crease_rate: f32,
    /// 1 回のしわが画面に現れる長さ [フィールド]
    pub crease_fields: f32,
    /// ヘッドの目詰まり (画面全体が一瞬ノイズに埋もれる) の発生頻度 [回/分]
    pub head_clog_rate: f32,
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
            crease_rate: 0.0,
            crease_fields: 20.0,
            head_clog_rate: 0.0,
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
    /// 垂直方向の揺れ (rms) [ライン]。安物デッキの V ジッタ
    pub vertical_jitter: f32,
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
            vertical_jitter: 0.0,
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

/// OSD の合成先
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OsdMode {
    /// デッキが再生出力 (コンポジット) に重畳する
    #[default]
    Composite,
    /// 録画時にテープへ焼き込まれている (カメラの日付表示など)
    Tape,
}

/// OSD (オンスクリーン表示)。書式は `osd::markup` を参照。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OsdParams {
    pub enabled: bool,
    pub mode: OsdMode,
    /// 表示内容のマークアップ
    pub markup: String,
    /// 既定の文字サイズ (1 = 2×2 ドット/画素, 1〜8)
    pub scale: u32,
    /// 不透明度 (0..1)
    pub opacity: f32,
    /// `{date}` `{time}` などの起点となる日時 ("YYYY-MM-DD HH:MM:SS")
    pub start_datetime: String,
    /// フォントファイル (.ttf / .otf / .ttc) のパス。空なら内蔵の 5×7 ドットフォント
    pub font: String,
    /// .ttc 内のフォント番号
    pub font_index: u32,
    /// 外部フォントを 1 em あたり何ドットで表示するか (0 = なめらかに描く)
    pub font_dots: u32,
}

impl Default for OsdParams {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: OsdMode::Composite,
            markup: "<pos=56,48><edge=black>PLAY ▶</edge>
<pos=56,400><edge=black>{counter}   <color=#9cf>SP</color></edge>
<pos=698,400><align=right><edge=black>{MON}.{D} {Y}</edge>".into(),
            scale: 2,
            opacity: 1.0,
            start_datetime: "1995-08-12 14:30:00".into(),
            font: String::new(),
            font_index: 0,
            font_dots: 16,
        }
    }
}

/// VCR の RF モジュレータ → テレビのチューナ経由で見たときの劣化
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RfParams {
    pub enabled: bool,
    /// 受信ノイズ (スノー) の rms [IRE]
    pub snow_ire: f32,
    /// ゴースト (反射波) の遅延 [µs]
    pub ghost_delay_us: f32,
    /// ゴーストの強さ (負なら反転ゴースト)
    pub ghost_gain: f32,
    /// ビート妨害 (斜め縞) の振幅 [IRE]
    pub interference_ire: f32,
    /// ビート妨害の周波数 [MHz]
    pub interference_mhz: f32,
    /// 音声搬送波が映像に漏れる量 (920 kHz ビート / 音声連動の縞)
    pub sound_beat: f32,
}

impl Default for RfParams {
    fn default() -> Self {
        Self {
            enabled: false,
            snow_ire: 3.0,
            ghost_delay_us: 1.2,
            ghost_gain: 0.12,
            interference_ire: 0.0,
            interference_mhz: 2.7,
            sound_beat: 0.0,
        }
    }
}

/// キャプチャ時のレベル (ビデオレンジ/フルレンジ) の扱い
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureLevels {
    /// 正しく変換
    #[default]
    Correct,
    /// 16-235 をフルレンジとして表示 (黒が浮いて白がくすむ)
    WashedOut,
    /// フルレンジを 16-235 として表示 (黒つぶれ・白飛び)
    Crushed,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChromaSubsampling {
    #[serde(rename = "444")]
    S444,
    #[serde(rename = "422")]
    S422,
    /// MPEG-2 / DVD / 多くの USB キャプチャ
    #[default]
    #[serde(rename = "420")]
    S420,
    /// NTSC DV
    #[serde(rename = "411")]
    S411,
}

/// デジタルキャプチャ (アナログキャプチャボード → MPEG 系エンコーダ)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CaptureParams {
    pub enabled: bool,
    pub levels: CaptureLevels,
    pub chroma_subsampling: ChromaSubsampling,
    /// DCT ブロック圧縮 (MPEG-2 風) を行う
    pub codec: bool,
    /// 目標ビットレート [kbps] (29.97 fps 換算)
    pub bitrate_kbps: f32,
    /// GOP 長 (I フレーム間隔) [フレーム]
    pub gop: u32,
    /// 動き探索範囲 [画素] (0 で動き補償なし)
    pub motion_search: u32,
    /// レート制御の追従の速さ (0..1)。大きいほど画質が「呼吸」する
    pub rate_control: f32,
    /// コマ落ち (前フレームの繰り返し) の確率
    pub frame_drop_rate: f32,
    /// ADC の量子化ビット数 (8 が普通)
    pub adc_bits: u32,
}

impl Default for CaptureParams {
    fn default() -> Self {
        Self {
            enabled: false,
            levels: CaptureLevels::Correct,
            chroma_subsampling: ChromaSubsampling::S420,
            codec: true,
            bitrate_kbps: 4000.0,
            gop: 15,
            motion_search: 7,
            rate_control: 0.5,
            frame_drop_rate: 0.0,
            adc_bits: 8,
        }
    }
}

/// 音声トラックの方式
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioMode {
    /// 固定ヘッドによるリニア (ノーマル) 音声トラック: モノラル・狭帯域・ヒス・ワウフラッタ
    #[default]
    Linear,
    /// 回転ヘッドによる Hi-Fi (FM 深層記録) 音声: ステレオ・広帯域だがヘッド切替バズが出る
    Hifi,
}

/// 音声の劣化 (テープ → 再生機 → キャプチャ)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioParams {
    pub enabled: bool,
    pub mode: AudioMode,

    // --- テープ ---
    /// リニアトラックの低域カットオフ [Hz]
    pub low_cut_hz: f32,
    /// リニアトラックの高域カットオフ [Hz] (SP ≒ 10k, EP ≒ 5k)
    pub high_cut_hz: f32,
    /// テープヒス [dBFS]
    pub hiss_db: f32,
    /// 録音レベル (テープ飽和へのドライブ) [dB]
    pub drive_db: f32,
    /// ワウ (遅い速度変動) [%]
    pub wow_pct: f32,
    /// フラッタ (速い速度変動) [%]
    pub flutter_pct: f32,
    /// ドロップアウト (音がこもって痩せる) [回/秒]
    pub dropout_rate: f32,
    /// Hi-Fi のヘッド切替バズ (トラッキングずれでも増える)
    pub hifi_buzz: f32,

    // --- 周辺機器 ---
    /// 電源ハム (グラウンドループ) [dBFS]
    pub hum_db: f32,
    /// 電源周波数 [Hz]
    pub hum_hz: f32,
    /// AGC (自動録音レベル) の強さ (0..1)。ポンピングと無音部のノイズ持ち上げ
    pub agc: f32,
    /// キャプチャ ADC のサンプリング周波数 [Hz]
    pub sample_rate_hz: f32,
    /// キャプチャ ADC の量子化ビット数
    pub bits: u32,
    /// モノラルにする
    pub mono: bool,
    /// 書き出し時の AAC ビットレート [kbps] (低いほど圧縮ノイズ)
    pub codec_kbps: u32,
}

impl Default for AudioParams {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: AudioMode::Linear,
            low_cut_hz: 80.0,
            high_cut_hz: 10000.0,
            hiss_db: -52.0,
            drive_db: 0.0,
            wow_pct: 0.12,
            flutter_pct: 0.08,
            dropout_rate: 0.05,
            hifi_buzz: 0.1,
            hum_db: -90.0,
            hum_hz: 60.0,
            agc: 0.0,
            sample_rate_hz: 48000.0,
            bits: 16,
            mono: false,
            codec_kbps: 192,
        }
    }
}
