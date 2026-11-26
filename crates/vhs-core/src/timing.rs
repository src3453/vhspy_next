//! テレビ方式 (NTSC など) のタイミングと信号レベル。
//!
//! 信号は 4fsc でサンプリングした 1 次元のコンポジット信号として扱う。単位は IRE
//! (ブランキング = 0、白 = 100。PAL は 700 mV = 100 とした同じ尺度)。
//! 方式ごとの値は [`Standard`] にまとめ、`Field` が参照を持つ。
//!
//! 1 フィールドは帰線区間 (等化パルス・垂直同期・VBI) を含む全ラスタで扱う ([`crate::raster`])。
//!
//! PAL の簡略化: 実際の PAL の副搬送波は 1 ラインあたり 283.7516 周期 (25 Hz オフセット) で、
//! 4fsc ではライン同期しない。ここでは 283.75 周期 (= 1135 サンプル) に丸め、ライン同期として扱う。
//! 半ライン (567.5 サンプル) の位置にある等化パルス・幅広パルスは小数位置のまま描く。

use serde::{Deserialize, Serialize};

/// テレビ方式の選択 (パラメータとして保存する値)
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TvStandard {
    /// NTSC-M (525 ライン / 59.94 Hz)
    #[default]
    Ntsc,
    /// PAL-B/G (625 ライン / 50 Hz)
    Pal,
}

impl TvStandard {
    pub fn spec(self) -> &'static Standard {
        match self {
            TvStandard::Ntsc => &NTSC,
            TvStandard::Pal => &PAL,
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
    /// 1 フレームの総ライン数 (525 / 625)
    pub frame_total_lines: usize,
    /// 1 フィールドのバッファ ([`crate::frame::Field`]) のライン数 (帰線区間込み)。
    /// 第 1 フィールドの末尾 1 ラインは次のフィールドの先頭と重なる (ld-decode と同じく両フィールドを同じ大きさにする)
    pub raster_lines: usize,
    /// フィールドバッファの先頭のライン番号 (1 始まり) [第 1, 第 2 フィールド]。
    /// 垂直同期の後の等化パルスが終わったところから始め、次のフィールドの垂直同期をバッファの末尾に含める
    /// (受像機は前のバッファで見つけた垂直同期から画面を描くので、テープの垂直揺れで遅れても有効映像が収まる)
    pub raster_first_line: [usize; 2],
    /// 垂直同期 (最初の幅広パルス) の始まる位置 [ライン番号、半ラインは .5] [第 1, 第 2 フィールド]
    pub vsync_line: [f32; 2],
    /// 有効映像の 1 行目のライン番号 [第 1, 第 2 フィールド]
    pub active_first_line: [usize; 2],
    /// 等化パルスの幅 [サンプル]
    pub eq_width: f32,
    /// 垂直同期の切り込み (セレーション) の幅 [サンプル]。幅広パルスは半ラインからこれを引いた長さ
    pub serration_width: f32,
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

    /// フィールド (frame, parity) のバッファの `raster_line` ライン目の通算ライン番号 (ストリームの先頭 = 0)。
    /// ストリームは帰線区間も含めて途切れなく続くライン列で、副搬送波位相・PAL スイッチ・ライン番号はこれで決まる
    #[inline]
    pub fn stream_line(&self, raster_line: usize, frame: u64, parity: u8) -> u64 {
        frame * self.frame_total_lines as u64 + (self.raster_first_line[parity as usize & 1] - 1 + raster_line) as u64
    }

    /// 通算ライン番号 → (フレーム番号, フレーム内のライン番号 1..=frame_total_lines)
    #[inline]
    pub fn line_number(&self, stream_line: u64) -> (u64, usize) {
        let t = self.frame_total_lines as u64;
        (stream_line / t, (stream_line % t) as usize + 1)
    }

    /// そのフィールドがストリームに足すライン数 (バッファ末尾の重なりを除く)。NTSC 262 / 263、PAL 312 / 313
    #[inline]
    pub fn stream_lines(&self, parity: u8) -> usize {
        let half = self.frame_total_lines / 2;
        if parity & 1 == 0 { half } else { self.frame_total_lines - half }
    }

    /// フィールド (frame, parity) の垂直同期の始まる通算ライン位置 (第 2 フィールドは半ライン)
    #[inline]
    pub fn vsync_stream_pos(&self, frame: u64, parity: u8) -> f64 {
        (frame * self.frame_total_lines as u64) as f64 + self.vsync_line[parity as usize & 1] as f64 - 1.0
    }

    /// 垂直同期の始まりから有効映像の 1 行目までのライン数 (第 1 フィールドで数える)
    #[inline]
    pub fn vsync_to_active(&self) -> f64 {
        self.active_first_line[0] as f64 - self.vsync_line[0] as f64
    }

    /// 有効映像がバッファの末尾 (ストリームに足すぶん) までに残す余裕のライン数
    #[inline]
    pub fn raster_slack(&self, parity: u8) -> usize {
        self.stream_lines(parity) - (self.active_raster_index(parity) + self.field_lines)
    }

    /// 有効映像の 1 行目がフィールドバッファの何ライン目か
    #[inline]
    pub fn active_raster_index(&self, parity: u8) -> usize {
        let p = parity as usize & 1;
        self.active_first_line[p] - self.raster_first_line[p]
    }

    /// ラインごとのサブキャリア位相オフセット (90° 単位)。`raster_line` はフィールドバッファのライン番号。
    /// サンプル n の位相象限は `(n + line_phase_base(..)) & 3`。
    ///
    /// 副搬送波は 4fsc のサンプリングに同期しているので、通算ライン番号 × ラインのサンプル数で決まる。
    /// NTSC: 1 ライン = 227.5 周期なのでラインごとに 180° 反転、1 フレーム (525 ライン) でも 180° 反転する。
    /// PAL: 1 ライン = 283.75 周期なのでラインごとに 270° 進む。
    #[inline]
    pub fn line_phase_base(&self, raster_line: usize, frame: u64, parity: u8) -> usize {
        self.line_phase_base_at(self.stream_line(raster_line, frame, parity))
    }

    /// 通算ライン番号からのサブキャリア位相オフセット (90° 単位)
    #[inline]
    pub fn line_phase_base_at(&self, stream_line: u64) -> usize {
        ((stream_line & 3) as usize * (self.line_samples & 3)) & 3
    }

    /// 1 ライン進んだときの副搬送波位相の進み (90° 単位)。受像機の PLL がバーストの来る位相を予測するのに使う
    #[inline(always)]
    pub const fn line_phase_step(&self) -> usize {
        self.line_samples & 3
    }

    /// 色の V 成分の符号 (PAL スイッチ)。NTSC は常に +1。`raster_line` はフィールドバッファのライン番号
    #[inline(always)]
    pub fn v_switch(&self, raster_line: usize, frame: u64, parity: u8) -> f32 {
        self.v_switch_at(self.stream_line(raster_line, frame, parity))
    }

    /// 通算ライン番号からの PAL スイッチ。PAL は 1 ラインごとに V を反転する
    /// (1 フレームは奇数ラインなので、同じライン番号でもフレームごとに反転する)
    #[inline(always)]
    pub fn v_switch_at(&self, stream_line: u64) -> f32 {
        match self.kind {
            TvStandard::Ntsc => 1.0,
            TvStandard::Pal => {
                if stream_line & 1 == 0 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }

    #[inline(always)]
    pub fn is_pal(&self) -> bool {
        self.kind == TvStandard::Pal
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
    frame_total_lines: 525,
    raster_lines: 263,
    raster_first_line: [10, 272],
    vsync_line: [4.0, 266.5],
    active_first_line: [22, 285],
    // 2.3 µs / 4.7 µs
    eq_width: 33.0,
    serration_width: 67.0,
    field_rate: 60000.0 / 1001.0,
    field_rate_ratio: (60000, 1001),
    sound_carrier_hz: 4.5e6,
    video_bw_hz: 4.2e6,
    ire: Levels { sync: -40.0, blank: 0.0, black: 7.5, white: 100.0, burst: 20.0 },
};

/// PAL-B/G。4fsc = 17.734475 MHz、1 ライン 64 µs = 1135 サンプル。
/// 同期 4.7 µs、バースト 10 周期 (同期の立ち下がりから 5.6 µs)、有効映像 52 µs (10.5 µs から)
pub static PAL: Standard = Standard {
    kind: TvStandard::Pal,
    fsc: 4_433_618.75,
    fs: 4.0 * 4_433_618.75,
    line_samples: 1135,
    sync_end: 83,
    burst_start: 100,
    burst_end: 140,
    active_start: 186,
    active_width: 922,
    field_lines: 288,
    total_lines_per_field: 312.5,
    frame_total_lines: 625,
    raster_lines: 313,
    raster_first_line: [6, 318],
    vsync_line: [1.0, 313.5],
    active_first_line: [23, 336],
    // 2.35 µs / 4.7 µs
    eq_width: 42.0,
    serration_width: 83.0,
    field_rate: 50.0,
    field_rate_ratio: (50, 1),
    sound_carrier_hz: 5.5e6,
    video_bw_hz: 5.0e6,
    // 同期 300 mV、バースト 300 mVp-p (700 mV = 100)。黒 = ブランキング (セットアップなし)
    ire: Levels { sync: -300.0 / 7.0, blank: 0.0, black: 0.0, white: 100.0, burst: 150.0 / 7.0 },
};

const _: () = assert!(NTSC.line_samples <= MAX_LINE_SAMPLES && NTSC.active_width <= MAX_ACTIVE_WIDTH);
const _: () = assert!(PAL.line_samples <= MAX_LINE_SAMPLES && PAL.active_width <= MAX_ACTIVE_WIDTH);

/// サブキャリアの 4 象限位相テーブル (4fsc サンプリングでは位相は 90° 刻みになる)
pub const COS4: [f32; 4] = [1.0, 0.0, -1.0, 0.0];
pub const SIN4: [f32; 4] = [0.0, 1.0, 0.0, -1.0];
