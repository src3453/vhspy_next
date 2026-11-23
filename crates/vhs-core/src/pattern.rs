//! 調整用のテストパターン。

use crate::frame::RgbFrame;
use crate::pipeline::{INPUT_HEIGHT, INPUT_WIDTH};

/// 75% カラーバー + 輝度ランプ + fsc 付近の細かい縦縞 (クロスカラー確認用)
pub fn color_bars() -> RgbFrame {
    let mut f = RgbFrame::new(INPUT_WIDTH, INPUT_HEIGHT);
    let bars: [[u8; 3]; 7] =
        [[191, 191, 191], [191, 191, 0], [0, 191, 191], [0, 191, 0], [191, 0, 191], [191, 0, 0], [0, 0, 191]];
    for y in 0..INPUT_HEIGHT {
        for x in 0..INPUT_WIDTH {
            let px = if y < INPUT_HEIGHT * 2 / 3 {
                bars[x * 7 / INPUT_WIDTH]
            } else if y < INPUT_HEIGHT * 5 / 6 {
                let v = (x * 255 / INPUT_WIDTH) as u8;
                [v, v, v]
            } else {
                // 左から右へ周波数が上がる縦縞 (およそ 0.5〜5 MHz)
                let t = x as f32 / INPUT_WIDTH as f32;
                let phase = std::f32::consts::PI * (0.07 * t + 0.33 * t * t) * x as f32;
                let v = (126.0 + 100.0 * phase.sin()) as u8;
                [v, v, v]
            };
            f.data[(y * INPUT_WIDTH + x) * 3..][..3].copy_from_slice(&px);
        }
    }
    f
}
