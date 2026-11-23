//! NTSC のタイミングと信号レベル定数。
//!
//! 信号は 4fsc (≒14.318 MHz) でサンプリングした 1 次元のコンポジット信号として扱う。
//! 1 ライン = 910 サンプル (227.5 サブキャリア周期)。単位は IRE。

/// カラーサブキャリア周波数 [Hz]
pub const FSC: f32 = 315.0e6 / 88.0;
/// サンプリング周波数 (4fsc) [Hz]
pub const FS: f32 = 4.0 * FSC;

/// 1 水平ラインあたりのサンプル数
pub const LINE_SAMPLES: usize = 910;
/// 水平同期パルスの終端 (ライン先頭 = 同期の立ち下がり)
pub const SYNC_END: usize = 67;
/// カラーバースト区間 (9 周期 = 36 サンプル)
pub const BURST_START: usize = 76;
pub const BURST_END: usize = 112;
/// 有効映像区間
pub const ACTIVE_START: usize = 135;
pub const ACTIVE_WIDTH: usize = 754;
pub const ACTIVE_END: usize = ACTIVE_START + ACTIVE_WIDTH;

/// 1 フィールドあたりの有効ライン数
pub const FIELD_LINES: usize = 240;
/// 1 フレームあたりの有効ライン数
pub const FRAME_LINES: usize = 2 * FIELD_LINES;

/// 1 ラインの時間 [s]
pub const LINE_TIME: f32 = LINE_SAMPLES as f32 / FS;
/// フィールド周波数 [Hz]
pub const FIELD_RATE: f32 = 60000.0 / 1001.0;

/// 信号レベル [IRE]
pub mod ire {
    pub const SYNC: f32 = -40.0;
    pub const BLANK: f32 = 0.0;
    pub const BLACK: f32 = 7.5;
    pub const WHITE: f32 = 100.0;
    /// バースト振幅 (片側)
    pub const BURST: f32 = 20.0;
    /// 黒〜白のスパン
    pub const SPAN: f32 = WHITE - BLACK;
}

/// サブキャリアの 4 象限位相テーブル (4fsc サンプリングでは位相は 90° 刻みになる)
pub const COS4: [f32; 4] = [1.0, 0.0, -1.0, 0.0];
pub const SIN4: [f32; 4] = [0.0, 1.0, 0.0, -1.0];

/// ラインごとのサブキャリア位相オフセット (90° 単位)。
///
/// 1 ライン = 227.5 周期なのでラインごとに 180° 反転、1 フレーム = 119437.5 周期なので
/// フレームごとにも 180° 反転する。第 2 フィールドは 262.5 ライン後に始まるので 270° ずれる。
/// サンプル n の位相象限は `(n + line_phase_base(..)) & 3`。
#[inline]
pub fn line_phase_base(field_line: usize, frame: u64, parity: u8) -> usize {
    (2 * field_line + 2 * (frame as usize & 1) + 3 * parity as usize) & 3
}
