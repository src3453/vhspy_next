//! 調整用のテストパターン。

use crate::frame::RgbFrame;
use crate::timing::Standard;

/// 75% カラーバー + 輝度ランプ + fsc 付近の細かい縦縞 (クロスカラー確認用)。大きさは方式の画像サイズ
pub fn color_bars(std: &Standard) -> RgbFrame {
    let (w, h) = std.image_size();
    let mut f = RgbFrame::new(w, h);
    let bars: [[u8; 3]; 7] =
        [[191, 191, 191], [191, 191, 0], [0, 191, 191], [0, 191, 0], [191, 0, 191], [191, 0, 0], [0, 0, 191]];
    for y in 0..h {
        for x in 0..w {
            let px = if y < h * 2 / 3 {
                bars[x * 7 / w]
            } else if y < h * 5 / 6 {
                let v = (x * 255 / w) as u8;
                [v, v, v]
            } else {
                // 左から右へ周波数が上がる縦縞 (およそ 0.5〜5 MHz)
                let t = x as f32 / w as f32;
                let phase = std::f32::consts::PI * (0.07 * t + 0.33 * t * t) * x as f32;
                let v = (126.0 + 100.0 * phase.sin()) as u8;
                [v, v, v]
            };
            f.data[(y * w + x) * 3..][..3].copy_from_slice(&px);
        }
    }
    f
}
