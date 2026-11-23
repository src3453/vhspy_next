//! RGB 画像 → コンポジット信号 (1 フィールド分)。
//!
//! 色は副搬送波の直交変調で載せる。どちらの方式も `U sin(ωt) + V cos(ωt)` の形になる。
//! - NTSC: U/V 軸から 33° 回した I/Q で帯域を変えて (I 広め・Q 狭め) 載せる。バーストは -U 軸 (180°)
//! - PAL: U/V をそのまま同じ帯域で載せ、V の符号をラインごとに反転する (PAL スイッチ)。
//!   バーストも V 成分ごと反転するので、-U 軸から ±45° (135° / 225°) に振れる (スウィンギングバースト)

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
    let pal = s.is_pal();

    let y_lp = Cascade::butter_lowpass_at(4, p.luma_bw_mhz * 1e6, s.fs);
    // NTSC: (I, Q)、PAL: (V, U) の帯域
    let (a_bw, b_bw) = if pal { (p.uv_bw_mhz, p.uv_bw_mhz) } else { (p.i_bw_mhz, p.q_bw_mhz) };
    let i_lp = Cascade::butter_lowpass_at(2, a_bw * 1e6, s.fs);
    let q_lp = Cascade::butter_lowpass_at(2, b_bw * 1e6, s.fs);
    let (parity, frame_no) = (field.parity, field.frame);
    let lv = s.ire;
    let span = lv.span();
    let (sync_end, active, burst) = (s.sync_end, s.active_start..s.active_end(), s.burst_start..s.burst_end);

    // cos(φ+θ0), sin(φ+θ0) の 4 象限テーブル (NTSC は θ0 = 33°、PAL は 0°)
    let rot = if pal { 0.0 } else { IQ_ROT };
    let mut cos_iq = [0f32; 4];
    let mut sin_iq = [0f32; 4];
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_2 + rot;
        cos_iq[k] = a.cos();
        sin_iq[k] = a.sin();
    }
    let cg = p.chroma_gain;
    let burst_half = lv.burst * std::f32::consts::FRAC_1_SQRT_2;

    field.data.par_chunks_mut(s.line_samples).enumerate().for_each(|(l, out)| {
        let row = 2 * l + parity as usize;
        let src = &frame.data[row * aw * 3..(row + 1) * aw * 3];
        let base = s.line_phase_base(l, frame_no, parity);
        let sv = s.v_switch(l, frame_no, parity);
        let (mut y_lp, mut i_lp, mut q_lp) = (y_lp, i_lp, q_lp);

        for (n, o) in out.iter_mut().enumerate() {
            // NTSC: (i, q) = (I, Q)、PAL: (i, q) = (±V, U)。どちらも i は cos 側、q は sin 側に載る
            let (mut y, mut i, mut q) = (lv.blank, 0.0, 0.0);
            if n < sync_end {
                y = lv.sync;
            } else if active.contains(&n) {
                let px = &src[(n - active.start) * 3..][..3];
                let (r, g, b) = (px[0], px[1], px[2]);
                y = lv.black + span * (0.299 * r + 0.587 * g + 0.114 * b);
                if pal {
                    i = sv * span * (0.615 * r - 0.515 * g - 0.100 * b);
                    q = span * (-0.147 * r - 0.289 * g + 0.436 * b);
                } else {
                    i = span * (0.5959 * r - 0.2746 * g - 0.3213 * b);
                    q = span * (0.2115 * r - 0.5227 * g + 0.3112 * b);
                }
            }
            let y = y_lp.process(y);
            let i = i_lp.process(i);
            let q = q_lp.process(q);

            let k = (n + base) & 3;
            let mut s = y + cg * (q * sin_iq[k] + i * cos_iq[k]);
            if burst.contains(&n) {
                if pal {
                    // -U 軸から V 側へ ±45°
                    s += burst_half * (sv * COS4[k] - SIN4[k]);
                } else {
                    s -= lv.burst * SIN4[k];
                }
            }
            *o = s;
        }
    });
}
