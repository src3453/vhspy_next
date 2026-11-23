//! RGB 画像 → NTSC コンポジット信号 (1 フィールド分)。

use crate::filter::Cascade;
use crate::frame::{Field, VideoFrame};
use crate::params::EncoderParams;
use crate::timing::*;
use rayon::prelude::*;

/// I/Q 軸は U/V 軸から 33° 回転している
const IQ_ROT: f32 = 33.0 * std::f32::consts::PI / 180.0;

/// `frame` は `field.std.image_size()` の大きさであること。
pub fn encode_field(frame: &VideoFrame, field: &mut Field, p: &EncoderParams) {
    let s = field.std;
    let (aw, fl) = s.image_size();
    assert_eq!((frame.width, frame.height), (aw, fl));

    let y_lp = Cascade::butter_lowpass_at(4, p.luma_bw_mhz * 1e6, s.fs);
    let i_lp = Cascade::butter_lowpass_at(2, p.i_bw_mhz * 1e6, s.fs);
    let q_lp = Cascade::butter_lowpass_at(2, p.q_bw_mhz * 1e6, s.fs);
    let (parity, frame_no) = (field.parity, field.frame);
    let lv = s.ire;
    let span = lv.span();
    let (sync_end, active, burst) = (s.sync_end, s.active_start..s.active_end(), s.burst_start..s.burst_end);

    // cos(φ+33°), sin(φ+33°) の 4 象限テーブル
    let mut cos_iq = [0f32; 4];
    let mut sin_iq = [0f32; 4];
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_2 + IQ_ROT;
        cos_iq[k] = a.cos();
        sin_iq[k] = a.sin();
    }
    let cg = p.chroma_gain;

    field.data.par_chunks_mut(s.line_samples).enumerate().for_each(|(l, out)| {
        let row = 2 * l + parity as usize;
        let src = &frame.data[row * aw * 3..(row + 1) * aw * 3];
        let base = s.line_phase_base(l, frame_no, parity);
        let (mut y_lp, mut i_lp, mut q_lp) = (y_lp, i_lp, q_lp);

        for (n, o) in out.iter_mut().enumerate() {
            let (mut y, mut i, mut q) = (lv.blank, 0.0, 0.0);
            if n < sync_end {
                y = lv.sync;
            } else if active.contains(&n) {
                let px = &src[(n - active.start) * 3..][..3];
                let (r, g, b) = (px[0], px[1], px[2]);
                y = lv.black + span * (0.299 * r + 0.587 * g + 0.114 * b);
                i = span * (0.5959 * r - 0.2746 * g - 0.3213 * b);
                q = span * (0.2115 * r - 0.5227 * g + 0.3112 * b);
            }
            let y = y_lp.process(y);
            let i = i_lp.process(i);
            let q = q_lp.process(q);

            let k = (n + base) & 3;
            let mut s = y + cg * (q * sin_iq[k] + i * cos_iq[k]);
            if burst.contains(&n) {
                s -= lv.burst * SIN4[k];
            }
            *o = s;
        }
    });
}
