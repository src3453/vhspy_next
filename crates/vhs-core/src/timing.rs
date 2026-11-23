//! テレビ方式 (NTSC など) のタイミングと信号レベル。
//!
//! 信号は 4fsc でサンプリングした 1 次元のコンポジット信号として扱う。単位は IRE
//! (ブランキング = 0、白 = 100)。方式ごとの値は [`Standard`] にまとめ、`Field` が参照を持つ。

use serde::{Deserialize, Serialize};

/// テレビ方式の選択 (パラメータとして保存する値)
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TvStandard {
    /// NTSC-M (525 ライン / 59.94 Hz)
    #[default]
    Ntsc,
}

impl TvStandard {
    pub fn spec(self) -> &'static Standard {
        match self {
            TvStandard::Ntsc => &NTSC,
        }
    }
}

/// 信号レベル [IRE]
#[derive(Clone, Copy, Debug)]
pub struct Levels {
    pub sync: f32,
    pub blank: f32,
    pub black: f32,
    pub white: f32,
    /// バースト振幅 (片側)
    pub burst: f32,
}

impl Levels {
    /// 黒〜白のスパン
    #[inline(always)]
    pub const fn span(&self) -> f32 {
        self.white - self.black
    }
}

/// 1 つのテレビ方式のタイミングとレベル
#[derive(Debug)]
pub struct Standard {
    pub kind: TvStandard,
    /// カラーサブキャリア周波数 [Hz]
    pub fsc: f32,
    /// サンプリング周波数 (4fsc) [Hz]
    pub fs: f32,
    /// 1 水平ラインあたりのサンプル数
    pub line_samples: usize,
    /// 水平同期パルスの終端 (ライン先頭 = 同期の立ち下がり)
    pub sync_end: usize,
    /// カラーバースト区間
    pub burst_start: usize,
    pub burst_end: usize,
    /// 有効映像区間
    pub active_start: usize,
    pub active_width: usize,
    /// 1 フィールドあたりの有効ライン数
    pub field_lines: usize,
    /// 1 フィールドの総ライン数 (帰線区間込み)
    pub total_lines_per_field: f64,
    /// フィールド周波数 [Hz]
    pub field_rate: f32,
    /// フィールド周波数の分数表記 (ffmpeg 用、分子/分母)
    pub field_rate_ratio: (u32, u32),
    /// 音声搬送波と映像搬送波の間隔 [Hz]
    pub sound_carrier_hz: f64,
    /// 映像の伝送帯域 (放送・チューナ) [Hz]
    pub video_bw_hz: f32,
    pub ire: Levels,
}

impl Standard {
    #[inline(always)]
    pub const fn active_end(&self) -> usize {
        self.active_start + self.active_width
    }

    /// 1 フレームあたりの有効ライン数
    #[inline(always)]
    pub const fn frame_lines(&self) -> usize {
        2 * self.field_lines
    }

    /// 画面上の正方形 1 つが何サンプル幅になるか (4:3 の画面で、高さ 1 ライン = 1 単位)
    #[inline(always)]
    pub fn sample_aspect(&self) -> f32 {
        self.active_width as f32 / (self.frame_lines() as f32 * 4.0 / 3.0)
    }

    /// 入出力画像の大きさ (幅, 高さ)。有効映像区間のサンプル数 × フレームの有効ライン数
    #[inline(always)]
    pub const fn image_size(&self) -> (usize, usize) {
        (self.active_width, self.frame_lines())
    }

    /// 1 ラインの時間 [s]
    #[inline(always)]
    pub fn line_time(&self) -> f32 {
        self.line_samples as f32 / self.fs
    }

    /// フレーム周波数 [Hz]
    #[inline(always)]
    pub fn frame_rate(&self) -> f32 {
        self.field_rate * 0.5
    }

    /// ラインごとのサブキャリア位相オフセット (90° 単位)。
    /// サンプル n の位相象限は `(n + line_phase_base(..)) & 3`。
    ///
    /// NTSC: 1 ライン = 227.5 周期なのでラインごとに 180° 反転、1 フレーム = 119437.5 周期なので
    /// フレームごとにも 180° 反転する。第 2 フィールドは 262.5 ライン後に始まるので 270° ずれる。
    #[inline]
    pub fn line_phase_base(&self, field_line: usize, frame: u64, parity: u8) -> usize {
        match self.kind {
            TvStandard::Ntsc => (2 * field_line + 2 * (frame as usize & 1) + 3 * parity as usize) & 3,
        }
    }
}

/// どの方式でも収まるライン・有効区間の最大サンプル数 (スタック上の作業領域用)
pub const MAX_LINE_SAMPLES: usize = 1152;
pub const MAX_ACTIVE_WIDTH: usize = 1024;

pub static NTSC: Standard = Standard {
    kind: TvStandard::Ntsc,
    fsc: 315.0e6 / 88.0,
    fs: 4.0 * (315.0e6 / 88.0),
    line_samples: 910,
    sync_end: 67,
    // 9 周期 = 36 サンプル
    burst_start: 76,
    burst_end: 112,
    active_start: 135,
    active_width: 754,
    field_lines: 240,
    total_lines_per_field: 262.5,
    field_rate: 60000.0 / 1001.0,
    field_rate_ratio: (60000, 1001),
    sound_carrier_hz: 4.5e6,
    video_bw_hz: 4.2e6,
    ire: Levels { sync: -40.0, blank: 0.0, black: 7.5, white: 100.0, burst: 20.0 },
};

const _: () = assert!(NTSC.line_samples <= MAX_LINE_SAMPLES && NTSC.active_width <= MAX_ACTIVE_WIDTH);

/// サブキャリアの 4 象限位相テーブル (4fsc サンプリングでは位相は 90° 刻みになる)
pub const COS4: [f32; 4] = [1.0, 0.0, -1.0, 0.0];
pub const SIN4: [f32; 4] = [0.0, 1.0, 0.0, -1.0];
