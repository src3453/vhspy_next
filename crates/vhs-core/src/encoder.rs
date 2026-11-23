//! RGB 画像 → NTSC コンポジット信号 (1 フィールド分)。

use crate::filter::Cascade;
use crate::frame::{Field, RgbFrame};
use crate::params::EncoderParams;
use crate::timing::*;
use rayon::prelude::*;

/// I/Q 軸は U/V 軸から 33° 回転している
const IQ_ROT: f32 = 33.0 * std::f32::consts::PI / 180.0;

/// `frame` は ACTIVE_WIDTH × FRAME_LINES であること。
pub fn encode_field(frame: &RgbFrame, field: &mut Field, p: &EncoderParams) {
    assert_eq!(frame.width, ACTIVE_WIDTH);
    assert_eq!(frame.height, FRAME_LINES);

    let y_lp = Cascade::butter_lowpass(4, p.luma_bw_mhz * 1e6);
    let i_lp = Cascade::butter_lowpass(2, p.i_bw_mhz * 1e6);
    let q_lp = Cascade::butter_lowpass(2, p.q_bw_mhz * 1e6);
    let (parity, frame_no) = (field.parity, field.frame);

    // cos(φ+33°), sin(φ+33°) の 4 象限テーブル
    let mut cos_iq = [0f32; 4];
    let mut sin_iq = [0f32; 4];
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_2 + IQ_ROT;
        cos_iq[k] = a.cos();
        sin_iq[k] = a.sin();
    }
    let cg = p.chroma_gain;

    field.data.par_chunks_mut(LINE_SAMPLES).enumerate().for_each(|(l, out)| {
        let row = 2 * l + parity as usize;
        let src = &frame.data[row * ACTIVE_WIDTH * 3..(row + 1) * ACTIVE_WIDTH * 3];
        let base = line_phase_base(l, frame_no, parity);
        let (mut y_lp, mut i_lp, mut q_lp) = (y_lp, i_lp, q_lp);

        for (n, o) in out.iter_mut().enumerate() {
            let (mut y, mut i, mut q) = (ire::BLANK, 0.0, 0.0);
            if n < SYNC_END {
                y = ire::SYNC;
            } else if (ACTIVE_START..ACTIVE_END).contains(&n) {
                let px = &src[(n - ACTIVE_START) * 3..][..3];
                let (r, g, b) = (px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0);
                y = ire::BLACK + ire::SPAN * (0.299 * r + 0.587 * g + 0.114 * b);
                i = ire::SPAN * (0.5959 * r - 0.2746 * g - 0.3213 * b);
                q = ire::SPAN * (0.2115 * r - 0.5227 * g + 0.3112 * b);
            }
            let y = y_lp.process(y);
            let i = i_lp.process(i);
            let q = q_lp.process(q);

            let k = (n + base) & 3;
            let mut s = y + cg * (q * sin_iq[k] + i * cos_iq[k]);
            if (BURST_START..BURST_END).contains(&n) {
                s -= ire::BURST * SIN4[k];
            }
            *o = s;
        }
    });
}
