//! RGB 画像 → コンポジット信号 (1 フィールド分、帰線区間込みの全ラスタ)。
//!
//! ライン構造 (等化パルス・切り込み付き垂直同期・バーストの有無) は [`crate::raster`]、
//! VBI に重畳するデータ・試験信号は [`crate::vbi`] で決める。
//!
//! 色は副搬送波の直交変調で載せる。どちらの方式も `U sin(ωt) + V cos(ωt)` の形になる。
//! - NTSC: U/V 軸から 33° 回した I/Q で帯域を変えて (I 広め・Q 狭め) 載せる。バーストは -U 軸 (180°)
//! - PAL: U/V をそのまま同じ帯域で載せ、V の符号をラインごとに反転する (PAL スイッチ)。
//!   バーストも V 成分ごと反転するので、-U 軸から ±45° (135° / 225°) に振れる (スウィンギングバースト)

use crate::filter::Cascade;
use crate::frame::{Field, VideoFrame, YiqFrame};
use crate::params::{EncoderParams, SignalParams, VbiParams};
use crate::raster;
use crate::rng::Rng;
use crate::timing::*;
use crate::vbi;
use rayon::prelude::*;

/// I/Q 軸は U/V 軸から 33° 回転している
const IQ_ROT: f32 = 33.0 * std::f32::consts::PI / 180.0;
const STREAM_VSYNC: u64 = 0x5653_594e_4300_0001;
const STREAM_HSYNC: u64 = 0x4853_594e_4300_0002;
const STREAM_BURST: u64 = 0x4255_5253_5400_0003;
const STREAM_CHROMA: u64 = 0x4348_524f_4d00_0004;

fn field_row(frame: &VideoFrame, line: isize, parity: u8) -> Option<&[f32]> {
    if !(0..(frame.height / 2) as isize).contains(&line) {
        return None;
    }
    let start = (2 * line as usize + parity as usize) * frame.width * 3;
    Some(&frame.data[start..start + frame.width * 3])
}

static NO_VBI: std::sync::LazyLock<VbiParams> =
    std::sync::LazyLock::new(|| VbiParams { enabled: false, ..VbiParams::default() });

/// `frame` は `field.std.image_size()` の大きさであること。コンポジット信号 (全ラスタ) を `field.data` に書く。
/// VBI のデータ信号は載せない
pub fn encode_field(frame: &VideoFrame, field: &mut Field, p: &EncoderParams) {
    encode_field_with_signal(frame, field, p, &SignalParams::default(), 0);
}

/// 信号要素ごとの劣化を加えてコンポジット信号を作る (VBI のデータ信号なし)。
pub fn encode_field_with_signal(
    frame: &VideoFrame,
    field: &mut Field,
    p: &EncoderParams,
    signal: &SignalParams,
    seed: u64,
) {
    encode(frame, None, field, p, signal, &NO_VBI, seed, false);
}

/// YIQ の電気信号を RGB 変換・クリップせずコンポジットへ符号化する。
pub fn encode_field_full_yiq(
    frame: &YiqFrame,
    field: &mut Field,
    p: &EncoderParams,
    signal: &SignalParams,
    vbi: &VbiParams,
    seed: u64,
) {
    let rgb_placeholder = VideoFrame { width: 0, height: 0, data: Vec::new() };
    let split = field.separate;
    encode(&rgb_placeholder, Some(frame), field, p, signal, vbi, seed, split);
}

/// 信号要素ごとの劣化と VBI のデータ・試験信号を加えて書く。
/// `field.separate` なら Y/C を分けたまま (S 映像) 書く: `data` = Y (同期付き)、`chroma` = C (バースト付き)
pub fn encode_field_full(
    frame: &VideoFrame,
    field: &mut Field,
    p: &EncoderParams,
    signal: &SignalParams,
    vbi: &VbiParams,
    seed: u64,
) {
    let split = field.separate;
    encode(frame, None, field, p, signal, vbi, seed, split);
}

/// `field.separate` なら Y/C を分けたまま (S 映像) 書く: `data` = Y (同期付き)、`chroma` = C (バースト付き)。
/// テープを通さずに S 映像で出す機器 (DV デッキなど) 用
pub fn encode_field_yc(frame: &VideoFrame, field: &mut Field, p: &EncoderParams) {
    encode_field_yc_with_signal(frame, field, p, &SignalParams::default(), 0);
}

/// `field.separate` なら Y/C を分けたまま、信号要素ごとの劣化を加えて書く。
pub fn encode_field_yc_with_signal(
    frame: &VideoFrame,
    field: &mut Field,
    p: &EncoderParams,
    signal: &SignalParams,
    seed: u64,
) {
    let split = field.separate;
    encode(frame, None, field, p, signal, &NO_VBI, seed, split);
}

fn encode(
    frame: &VideoFrame,
    yiq_frame: Option<&YiqFrame>,
    field: &mut Field,
    p: &EncoderParams,
    signal: &SignalParams,
    vbi_p: &VbiParams,
    seed: u64,
    split: bool,
) {
    let s = field.std;
    let (aw, fl) = s.image_size();
    assert_eq!((yiq_frame.map_or(frame.width, |f| f.width), yiq_frame.map_or(frame.height, |f| f.height)), (aw, fl));
    let pal = s.is_pal();

    let y_lp = Cascade::butter_lowpass_at(4, p.luma_bw_mhz * 1e6, s.fs);
    // NTSC: (I, Q)、PAL: (V, U) の帯域
    let (a_bw, b_bw) = if pal { (p.uv_bw_mhz, p.uv_bw_mhz) } else { (p.i_bw_mhz, p.q_bw_mhz) };
    let i_lp = Cascade::butter_lowpass_at(2, a_bw * 1e6, s.fs);
    let q_lp = Cascade::butter_lowpass_at(2, b_bw * 1e6, s.fs);
    // 実機のエンコーダは帯域の狭い I/Q (U/V) のフィルタの遅れに合わせて Y (と I) を遅らせ、3 つを揃える。
    // ここでは同じことを、色差を群遅延のぶん先の画素から読むことで行う
    let iq_lead = |lp: &Cascade| {
        let d = lp.group_delay(1e3, s.fs) - y_lp.group_delay(1e3, s.fs);
        d.round().max(0.0) as usize
    };
    let (i_lead, q_lead) = (iq_lead(&i_lp), iq_lead(&q_lp));
    let (parity, frame_no, field_index) = (field.parity, field.frame, field.index);
    let lv = s.ire;
    let span = lv.span();
    let burst = s.burst_start..s.burst_end;

    // 垂直同期: 幅広パルスの振幅減衰・雑音は信号そのものに掛ける (受像機の垂直同期分離が取り損ねると
    // 垂直発振器が自走して画面が流れる)。ジッタは同期に対して映像だけを上下にずらす (フィールド単位)
    let vertical = &signal.vertical_sync;
    let mut vertical_rng = Rng::derive(seed, field.index, 0, STREAM_VSYNC);
    let vertical_offset = vertical_rng.normal() * vertical.jitter.max(0.0);
    let vertical_attenuation = (vertical.attenuation_pct / 100.0).clamp(0.0, 1.0);

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
    let burst_amp = lv.burst * (p.burst_level_pct / 100.0).max(0.0);
    let burst_half = burst_amp * std::f32::consts::FRAC_1_SQRT_2;
    let hs = &signal.horizontal_sync;
    let hs_attenuation = (hs.attenuation_pct / 100.0).clamp(0.0, 1.0);
    let burst_signal = &signal.color_burst;
    let burst_attenuation = (burst_signal.attenuation_pct / 100.0).clamp(0.0, 1.0);
    let clean_burst = burst_signal.jitter <= 0.0 && burst_attenuation == 0.0;
    let chroma_signal = &signal.chroma_subcarrier;
    let chroma_attenuation = (chroma_signal.attenuation_pct / 100.0).clamp(0.0, 1.0);
    let clean_chroma = chroma_signal.jitter <= 0.0 && chroma_attenuation == 0.0;

    let ls = s.line_samples;
    let mut no_chroma = Vec::new();
    let chroma_out: Vec<Option<&mut [f32]>> = if split {
        field.chroma.chunks_mut(ls).map(Some).collect()
    } else {
        no_chroma.resize_with(field.data.len() / ls, || None);
        no_chroma
    };
    field.data.par_chunks_mut(ls).zip(chroma_out).enumerate().for_each(|(l, (out, mut c_out))| {
        let sl = s.stream_line(l, frame_no, parity);
        let info = raster::line_info(s, sl);
        let base = s.line_phase_base_at(sl);
        let sv = s.v_switch_at(sl);
        // 映像の入るラインなら、このフィールドの何行目の画像か
        let image_line = info.image_row.filter(|r| r % 2 == parity as usize).map(|r| r / 2);
        let video = info.video_span(s);
        // V-sync のずれは同期波形・バーストを保ったまま有効映像だけを上下させる。
        let shifted = vertical_offset.abs() > 1e-5;
        let source = image_line.map_or(0.0, |a| a as f32 - vertical_offset);
        let source_lo = source.floor() as isize;
        let source_frac = source - source_lo as f32;
        let source_a = yiq_frame.is_none().then(|| field_row(frame, source_lo, parity)).flatten();
        let source_b = yiq_frame.is_none().then(|| field_row(frame, source_lo + 1, parity)).flatten();
        let (mut y_lp, mut i_lp, mut q_lp) = (y_lp, i_lp, q_lp);

        let mut hs_rng = Rng::derive(seed, field_index, l as u64, STREAM_HSYNC);
        let mut vs_rng = Rng::derive(seed, field_index, l as u64, STREAM_VSYNC);
        let mut burst_rng = Rng::derive(seed, field_index, l as u64, STREAM_BURST);
        let mut chroma_rng = Rng::derive(seed, field_index, l as u64, STREAM_CHROMA);
        let sync_end_pos =
            (s.sync_end as f32 + hs_rng.normal() * hs.jitter.max(0.0)).clamp(1.0, s.burst_start as f32);
        let pulses = raster::sync_pulses(s, info.halves, sync_end_pos);
        let burst_phase = burst_rng.normal() * burst_signal.jitter.max(0.0).to_radians();
        let (burst_sin, burst_cos) = burst_phase.sin_cos();
        let chroma_phase = chroma_rng.normal() * chroma_signal.jitter.max(0.0).to_radians();
        let (chroma_sin, chroma_cos) = chroma_phase.sin_cos();

        // VBI のデータ・試験信号 (映像のないラインと、半ラインの映像のない側)
        let mut vbi_y = [0f32; MAX_LINE_SAMPLES];
        let mut vbi_c = [0f32; MAX_LINE_SAMPLES];
        let has_vbi = info.video != raster::Video::Full
            && vbi::render_line(s, &info, vbi_p, base, &mut vbi_y[..ls], &mut vbi_c[..ls]);

        // 有効映像の画素 n の [Y, I, Q] (映像のない位置なら None)
        let video_yiq = |n: usize| -> Option<[f32; 3]> {
            if !video.is_some_and(|(a, b)| (a..b).contains(&n)) {
                return None;
            }
            let a = image_line?;
            let px = n - s.active_start;
            if let Some(yiq) = yiq_frame {
                let yiq = yiq.data[(2 * a + parity as usize) * aw + px];
                return Some([lv.black + span * yiq[0], span * yiq[1], span * yiq[2]]);
            }
            let px = px * 3;
            let (r, g, b) = if shifted {
                let sample = |c: usize| {
                    let a = source_a.map_or(0.0, |line| line[px + c]);
                    let b = source_b.map_or(0.0, |line| line[px + c]);
                    a + (b - a) * source_frac
                };
                (sample(0), sample(1), sample(2))
            } else {
                let row = &frame.data[(2 * a + parity as usize) * aw * 3..];
                (row[px], row[px + 1], row[px + 2])
            };
            let y = lv.black + span * (0.299 * r + 0.587 * g + 0.114 * b);
            Some(if pal {
                [y, sv * span * (0.615 * r - 0.515 * g - 0.100 * b), span * (-0.147 * r - 0.289 * g + 0.436 * b)]
            } else {
                [y, span * (0.5959 * r - 0.2746 * g - 0.3213 * b), span * (0.2115 * r - 0.5227 * g + 0.3112 * b)]
            })
        };

        for (n, o) in out.iter_mut().enumerate() {
            // NTSC: (i, q) = (I, Q)、PAL: (i, q) = (±V, U)。どちらも i は cos 側、q は sin 側に載る
            let (mut y, mut i, mut q) = (lv.blank, 0.0, 0.0);
            let (mut h_cov, mut v_cov) = (0f32, 0f32);
            for &(a, b, vert) in &pulses {
                let cv = raster::coverage(n, a, b);
                if vert {
                    v_cov += cv;
                } else {
                    h_cov += cv;
                }
            }
            let in_video = video.is_some_and(|(a, b)| (a..b).contains(&n));
            if h_cov > 0.0 || v_cov > 0.0 {
                y += (lv.sync - lv.blank) * (h_cov * (1.0 - hs_attenuation) + v_cov * (1.0 - vertical_attenuation));
            } else if in_video {
                y = video_yiq(n).map_or(lv.black, |v| v[0]);
            }
            if h_cov == 0.0 && v_cov == 0.0 {
                i = video_yiq(n + i_lead).map_or(0.0, |v| v[1]);
                q = video_yiq(n + q_lead).map_or(0.0, |v| v[2]);
            }
            if has_vbi && !in_video {
                y += vbi_y[n];
            }
            let mut y = y_lp.process(y);
            let i = i_lp.process(i);
            let q = q_lp.process(q);

            if h_cov > 0.0 && hs.noise_ire > 0.0 {
                y += hs_rng.normal() * hs.noise_ire;
            }
            if v_cov > 0.0 && vertical.noise_ire > 0.0 {
                y += vs_rng.normal() * vertical.noise_ire;
            }

            let k = (n + base) & 3;
            let mut c = if clean_chroma {
                cg * (q * sin_iq[k] + i * cos_iq[k])
            } else {
                let i_rot = i * chroma_cos + q * chroma_sin;
                let q_rot = q * chroma_cos - i * chroma_sin;
                cg * (1.0 - chroma_attenuation) * (q_rot * sin_iq[k] + i_rot * cos_iq[k])
            };
            if info.burst && burst.contains(&n) {
                if clean_burst {
                    if pal {
                        // -U 軸から V 側へ ±45°
                        c += burst_half * (sv * COS4[k] - SIN4[k]);
                    } else {
                        c -= burst_amp * SIN4[k];
                    }
                } else {
                    let (burst_i, burst_q) = if pal {
                        // -U 軸から V 側へ ±45°
                        (sv * burst_half, -burst_half)
                    } else {
                        (0.0, -burst_amp)
                    };
                    let burst_i_rot = burst_i * burst_cos + burst_q * burst_sin;
                    let burst_q_rot = burst_q * burst_cos - burst_i * burst_sin;
                    c += (1.0 - burst_attenuation) * (burst_i_rot * COS4[k] + burst_q_rot * SIN4[k]);
                }
                if burst_signal.noise_ire > 0.0 {
                    c += burst_rng.normal() * burst_signal.noise_ire;
                }
            }
            if has_vbi && !in_video {
                c += vbi_c[n];
            }
            if in_video && chroma_signal.noise_ire > 0.0 {
                c += chroma_rng.normal() * chroma_signal.noise_ire;
            }
            match c_out.as_deref_mut() {
                Some(co) => {
                    *o = y;
                    co[n] = c;
                }
                None => *o = y + c,
            }
        }
    });
}
