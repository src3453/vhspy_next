//! 全ラスタ (帰線区間込み) のライン構造。
//!
//! 1 フィールドのバッファ ([`crate::frame::Field`]) は有効映像だけでなく、等化パルス・切り込み付きの垂直同期・
//! VBI (垂直帰線消去期間のデータライン) を含む。ライン番号は 1 始まりのフレーム内の番号 (NTSC 1..=525、
//! PAL 1..=625)。各フィールドのバッファは垂直同期の後 (NTSC 10/272、PAL 6/318) から始まり、
//! 次のフィールドの等化パルス・垂直同期を末尾に含む ([`Standard::raster_first_line`])。
//!
//! NTSC (SMPTE 170M):
//! ```text
//!   1-3   等化パルス ×2/ライン        264-265 等化        266   等化 | 幅広
//!   4-6   幅広パルス (垂直同期)       267-268 幅広        269   幅広 | 等化
//!   7-9   等化パルス                  270-271 等化        272   等化 | なし
//!  10-21  VBI (21: 字幕)              273-284 VBI (284: 字幕 第 2 フィールド)
//!  22-262 有効映像  263 前半映像 | 等化   285-525 有効映像
//! ```
//! PAL (ITU-R BT.470):
//! ```text
//!   1-2   幅広    3 幅広 | 等化    4-5 等化     6-22 VBI   23 WSS | 後半映像   24-310 有効映像
//! 311-312 等化  313 等化 | 幅広  314-315 幅広  316-317 等化  318 等化 | なし  319-335 VBI
//! 336-622 有効映像  623 前半映像 | 等化  624-625 等化
//! ```
//! バーストは NTSC では垂直同期の前後 9H (1-9, 264-272) にない。PAL はブルッフ・ブランキング
//! (4 フィールドの周期で 9 ラインずつバーストを抜き、垂直同期後の最初のバーストの位相を揃える)。

use crate::timing::{Standard, TvStandard};

/// 半ライン単位の同期パルスの種類
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pulse {
    /// パルスなし (ブランキングのまま)
    None,
    /// 水平同期 (4.7 µs)
    HSync,
    /// 等化パルス (2.3 µs、半ライン周期)
    Equalizing,
    /// 幅広パルス (垂直同期。半ラインから切り込みの幅を引いた長さ)
    Broad,
}

/// ラインの映像区間
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Video {
    /// 映像なし (VBI のブランキング。データ信号はここに載る)
    None,
    Full,
    /// ラインの前半だけ映像 (NTSC 263、PAL 623)
    FirstHalf,
    /// ラインの後半だけ映像 (PAL 23)
    SecondHalf,
}

/// 1 ラインの構造
#[derive(Clone, Copy, Debug)]
pub struct LineInfo {
    /// 通算ライン番号
    pub stream: u64,
    /// このラインが属するフレーム番号
    pub frame: u64,
    /// フレーム内のライン番号 (1 始まり)
    pub number: usize,
    /// 前半・後半の同期パルス
    pub halves: [Pulse; 2],
    pub video: Video,
    /// カラーバーストがある
    pub burst: bool,
    /// 映像の入るフレーム画像の行 (有効映像のライン)
    pub image_row: Option<usize>,
}

impl LineInfo {
    /// 映像区間 [開始, 終了) [サンプル]。映像がなければ None
    pub fn video_span(&self, s: &Standard) -> Option<(usize, usize)> {
        let ls = s.line_samples;
        let mid = half_video_split(s);
        match self.video {
            Video::None => None,
            Video::Full => Some((s.active_start, s.active_end())),
            // 半ラインの映像は中央の等化パルスの手前 (フロントポーチ 1.5 µs) で終わる
            Video::FirstHalf => {
                let end = (ls as f32 * 0.5 - 1.5e-6 * s.fs) as usize;
                Some((s.active_start, end.min(mid)))
            }
            Video::SecondHalf => Some((mid, s.active_end())),
        }
    }

    /// 同期パルスのあるライン (水平同期で始まるライン) か
    pub fn has_hsync(&self) -> bool {
        self.halves[0] == Pulse::HSync
    }
}

/// 後半だけ映像のライン (PAL 23) の映像の開始位置。WSS (11.0〜38.4 µs) と重ならない位置にする
fn half_video_split(s: &Standard) -> usize {
    match s.kind {
        TvStandard::Ntsc => s.active_start + s.active_width / 2,
        TvStandard::Pal => ((38.5e-6 * s.fs) as usize).max(s.active_start + s.active_width / 2),
    }
}

/// 通算ライン番号のラインの構造
pub fn line_info(s: &Standard, stream: u64) -> LineInfo {
    use Pulse::*;
    let (frame, n) = s.line_number(stream);
    let (halves, video) = match s.kind {
        TvStandard::Ntsc => match n {
            1..=3 | 7..=9 | 264..=265 | 270..=271 => ([Equalizing, Equalizing], Video::None),
            4..=6 | 267..=268 => ([Broad, Broad], Video::None),
            266 => ([Equalizing, Broad], Video::None),
            269 => ([Broad, Equalizing], Video::None),
            272 => ([Equalizing, None], Video::None),
            10..=21 | 273..=284 => ([HSync, None], Video::None),
            263 => ([HSync, Equalizing], Video::FirstHalf),
            _ => ([HSync, None], Video::Full),
        },
        TvStandard::Pal => match n {
            1..=2 | 314..=315 => ([Broad, Broad], Video::None),
            3 => ([Broad, Equalizing], Video::None),
            313 => ([Equalizing, Broad], Video::None),
            4..=5 | 311..=312 | 316..=317 | 624..=625 => ([Equalizing, Equalizing], Video::None),
            318 => ([Equalizing, None], Video::None),
            6..=22 | 319..=335 => ([HSync, None], Video::None),
            23 => ([HSync, None], Video::SecondHalf),
            623 => ([HSync, Equalizing], Video::FirstHalf),
            _ => ([HSync, None], Video::Full),
        },
    };
    let burst = halves[0] == HSync
        && match s.kind {
            TvStandard::Ntsc => true,
            TvStandard::Pal => !bruch_blanked(frame, n),
        };
    let image_row = (0..2).find_map(|f| {
        let a = n.checked_sub(s.active_first_line[f])?;
        (a < s.field_lines).then_some(2 * a + f)
    });
    LineInfo { stream, frame, number: n, halves, video, burst, image_row }
}

/// PAL のブルッフ・ブランキング: バーストを抜くラインか。
/// 第 1〜4 フィールド (2 フレーム周期) で 623-6 / 310-318 / 622-5 / 311-319。
/// 622〜625 は次のフレームの第 1 フィールドの垂直同期の前なので、次のフレームの番号で決める
fn bruch_blanked(frame: u64, n: usize) -> bool {
    let even = frame % 2 == 0;
    match n {
        1..=6 => if even { n <= 6 } else { n <= 5 },
        310..=319 => if even { n <= 318 } else { n >= 311 },
        622..=625 => {
            // 次のフレームが偶数 (= 第 1 フィールド) なら 623〜、奇数 (第 3 フィールド) なら 622〜
            if !even { n >= 623 } else { n >= 622 }
        }
        _ => false,
    }
}

/// サンプル n の区間 [n, n+1) を区間 [a, b) が覆う割合 (同期パルスの縁を小数位置で描く)。
/// 整数の位置の縁は `n < b` の硬い縁と同じになる
#[inline(always)]
pub fn coverage(n: usize, a: f32, b: f32) -> f32 {
    let x = n as f32;
    ((x + 1.0).min(b) - x.max(a)).clamp(0.0, 1.0)
}

/// ラインの同期パルスの区間。`h_end` は水平同期の後縁 (ジッタ込み)。
/// 戻り値の各要素は (開始, 終了, 垂直同期か)
pub fn sync_pulses(s: &Standard, halves: [Pulse; 2], h_end: f32) -> [(f32, f32, bool); 2] {
    let half = s.line_samples as f32 * 0.5;
    let mut out = [(0.0, 0.0, false); 2];
    for (k, p) in halves.iter().enumerate() {
        let t0 = k as f32 * half;
        out[k] = match p {
            Pulse::None => (t0, t0, false),
            Pulse::HSync => (t0, t0 + h_end, false),
            Pulse::Equalizing => (t0, t0 + s.eq_width, false),
            Pulse::Broad => (t0, t0 + half - s.serration_width, true),
        };
    }
    out
}

/// 同期・ブランキングだけの理想的なライン (受像機側の基準や TBC の同期の作り直しに使う)
pub fn render_blanking(s: &Standard, info: &LineInfo, out: &mut [f32]) {
    let lv = s.ire;
    let pulses = sync_pulses(s, info.halves, s.sync_end as f32);
    for (n, o) in out.iter_mut().enumerate() {
        let cov: f32 = pulses.iter().map(|&(a, b, _)| coverage(n, a, b)).sum();
        *o = lv.blank + (lv.sync - lv.blank) * cov.min(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::{NTSC, NTSC_228, PAL};

    /// バッファは垂直同期の後から始まり、次のフィールドの垂直同期を末尾に含む。ストリームは途切れない
    #[test]
    fn field_buffers_frame_the_picture() {
        for s in [&NTSC, &NTSC_228, &PAL] {
            for p in 0..2u8 {
                let first = line_info(s, s.stream_line(0, 0, p));
                assert!(first.halves.iter().all(|h| *h != Pulse::Broad), "{:?} parity {p}", s.kind);
                // 次のフィールドの最初の幅広パルス
                let (nf, np) = if p == 0 { (0, 1) } else { (1, 0) };
                let v = s.vsync_stream_pos(nf, np);
                let i = (v.floor() as u64 - s.stream_line(0, 0, p)) as usize;
                assert!(i < s.stream_lines(p), "{:?} parity {p}: vsync at {i}", s.kind);
                let info = line_info(s, v.floor() as u64);
                let half = (v.fract() > 0.25) as usize;
                assert_eq!(info.halves[half], Pulse::Broad, "{:?} parity {p}", s.kind);
                if half == 1 {
                    assert_ne!(info.halves[0], Pulse::Broad);
                } else {
                    assert_ne!(line_info(s, v.floor() as u64 - 1).halves[1], Pulse::Broad);
                }
                // 有効映像はバッファに収まり、垂直同期から数えた位置は第 2 フィールドが半ライン下
                let a = s.active_raster_index(p);
                assert!(s.raster_slack(p) >= 6, "{:?} parity {p}", s.kind);
                let info = line_info(s, s.stream_line(a, 0, p));
                assert_eq!(info.image_row, Some(p as usize));
                let d = s.stream_line(a, 0, p) as f64 - s.vsync_stream_pos(0, p);
                assert_eq!(d, s.vsync_to_active() + 0.5 * p as f64);
            }
            assert_eq!(s.stream_line(0, 0, 0) + s.stream_lines(0) as u64, s.stream_line(0, 0, 1));
            assert_eq!(s.stream_line(0, 0, 1) + s.stream_lines(1) as u64, s.stream_line(0, 1, 0));
        }
    }

    /// 1 フィールドの幅広パルス・等化パルスの数
    #[test]
    fn vertical_interval_pulse_counts() {
        for s in [&NTSC, &NTSC_228, &PAL] {
            for p in 0..2u8 {
                let (mut eq, mut broad) = (0, 0);
                // 垂直同期の前 6 ライン〜後 8 ライン
                let start = s.vsync_stream_pos(1, p).floor() as u64 - 6;
                for sl in start..start + 14 {
                    for h in line_info(s, sl).halves {
                        match h {
                            Pulse::Equalizing => eq += 1,
                            Pulse::Broad => broad += 1,
                            _ => {}
                        }
                    }
                }
                let want = if s.kind == TvStandard::Ntsc { (12, 6) } else { (10, 5) };
                assert_eq!((eq, broad), want, "{:?} parity {p}", s.kind);
            }
        }
    }
}
