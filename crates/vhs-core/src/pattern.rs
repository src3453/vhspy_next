//! 調整用のテストパターン。

use crate::frame::{RgbFrame, YiqFrame};
use crate::timing::Standard;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorBarStandard {
    Smpte,
    Ebu,
}

/// YIQ から作るカラーバー。SMPTE は 75% バーと調整信号、EBU は 100% の8色バー。
/// 色は RGB 素材から読み込まず、YIQ の基本ベクトルから直接合成する。
pub fn color_bar_signal(std: &Standard, bars: ColorBarStandard) -> YiqFrame {
    let (w, h) = std.image_size();
    let mut f = YiqFrame::new(w, h, [0.0, 0.0, 0.0]);
    // SMPTE 170M の YIQ 伝達行列を使った 75%/100% カラーバー。
    // Y, I, Q は白を 1 とした値。色順は白・黄・シアン・緑・マゼンタ・赤・青。
    let colors: [[f32; 3]; 7] = match bars {
        ColorBarStandard::Smpte => [
            [0.75, 0.0, 0.0],
            [0.6645, 0.2415, -0.2340],
            [0.52575, -0.4470, -0.1585],
            [0.44025, -0.2055, -0.36975],
            [0.30975, 0.2055, 0.36975],
            [0.22425, 0.4470, 0.15825],
            [0.0855, -0.2415, 0.2340],
        ],
        ColorBarStandard::Ebu => [
            [1.0, 0.0, 0.0],
            [0.8860, 0.3220, -0.3120],
            [0.7010, -0.5960, -0.2110],
            [0.5870, -0.2740, -0.5230],
            [0.4130, 0.2740, 0.5230],
            [0.2990, 0.5960, 0.2110],
            [0.1140, -0.3220, 0.3120],
        ],
    };
    for y in 0..h {
        for x in 0..w {
            let xi = x * 8 / w;
            let yi = y * 6 / h;
            let px = match bars {
                ColorBarStandard::Ebu => {
                    let palette = [colors[0], colors[1], colors[2], colors[3], colors[4], colors[5], colors[6], [0.0, 0.0, 0.0]];
                    palette[xi]
                }
                ColorBarStandard::Smpte if yi < 4 => {
                    if xi == 7 { [0.0, 0.0, 0.0] } else { colors[xi] }
                }
                ColorBarStandard::Smpte if yi == 4 => {
                    [colors[6], [0.0,0.0,0.0], colors[4], [0.0,0.0,0.0], colors[2], [0.0,0.0,0.0], [0.75,0.0,0.0], [0.0,0.0,0.0]][xi]
                }
                ColorBarStandard::Smpte if yi == 5 => {
                    // -I / 100% white / +Q を信号領域として保持する。
                    let band = x * 4 / w;
                    // NTSC カラーバーストは 40 IRE p-p、片振幅 20 IRE。
                    let burst = 20.0 / 92.5;
                    match band {
                        0 => [0.0, -burst, 0.0],
                        1 => [1.0, 0.0, 0.0],
                        2 => [0.0, 0.0, burst],
                        _ => {
                            let patch = (x * 4 % w) * 3 / w;
                            [[-4.0 / 92.5, 0.0, 0.0], [0.0, 0.0, 0.0], [4.0 / 92.5, 0.0, 0.0]][patch]
                        }
                    }
                }
                _ => [0.0, 0.0, 0.0],
            };
            f.data[y * w + x] = px;
        }
    }
    f
}

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
