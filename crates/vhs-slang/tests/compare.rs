//! Slang (wgpu) 版と CPU 版 (vhs-core) の出力を比べる。
//!
//! 同じテストパターンを同じフィールド数だけ通し、最後のフレームを 8 ビット単位で比べる。
//! GPU の sin/cos・FMA の差で係数や IIR の結果がわずかにずれるので、完全一致ではなく誤差の上限で判定する。
//! GPU が無い環境ではスキップする。

use vhs_core::frame::{LinearFrame, VideoFrame};
use vhs_core::params::{DecoderParams, EncoderParams, YcSeparation};
use vhs_core::TvStandard;
use vhs_slang::reference::{cpu_decode, test_pattern};
use vhs_slang::{CompositeParams, GpuComposite, GpuContext};

struct Case {
    standard: TvStandard,
    dec: DecoderParams,
    /// Some なら、この大きさのシーンリニアな入力を有効映像の大きさへ縮小して通す
    /// (CPU は Pipeline::process_field_linear と同じく LinearFrame::resized → copy_from_linear)
    host_size: Option<(usize, usize)>,
}

struct Stats {
    max: f32,
    mean: f32,
}

fn compare(case: &Case, fields: usize) -> Option<Stats> {
    let gpu = match GpuContext::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipping GPU comparison: {e}");
            return None;
        }
    };
    let standard = case.standard;
    let (w, h) = standard.spec().image_size();
    let enc = EncoderParams::default();
    let mut params = CompositeParams::new(standard, &enc, &case.dec);

    let (gpu_input, gpu_size, cpu_px) = match case.host_size {
        None => {
            let px = test_pattern(w, h);
            let rgba: Vec<f32> = px.iter().flat_map(|c| [c[0], c[1], c[2], 1.0]).collect();
            (rgba, (w, h), px)
        }
        Some((hw, hh)) => {
            let linear: Vec<f32> =
                test_pattern(hw, hh).iter().flat_map(|c| [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2)]).collect();
            let frame = LinearFrame::from_raw(hw, hh, linear.clone()).unwrap();
            let mut video = VideoFrame::new(w, h);
            video.copy_from_linear(&frame.resized(w, h));
            let px: Vec<[f32; 3]> = video.data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            let rgba: Vec<f32> = linear.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 1.0]).collect();
            params.input_linear = 1;
            (rgba, (hw, hh), px)
        }
    };

    let cpu = cpu_decode(standard, &enc, &case.dec, &cpu_px, fields);
    let mut composite = GpuComposite::new(&gpu, standard, params);
    let out = composite.process(&gpu, &gpu_input, gpu_size.0 as u32, gpu_size.1 as u32, fields);

    let mut max = 0f32;
    let mut sum = 0f64;
    let mut worst = (0, 0, 0);
    for y in 0..h {
        for x in 0..w {
            for ch in 0..3 {
                let i = (y * w + x) * 4 + ch;
                let d = (out[i] * 255.0 - cpu.data[i] as f32).abs();
                sum += d as f64;
                if d > max {
                    max = d;
                    worst = (x, y, ch);
                }
            }
        }
    }
    let mean = (sum / (w * h * 3) as f64) as f32;
    eprintln!(
        "{standard:?} {:?} pll {} afc {} input {:?} on {}: max {max:.2} LSB at {worst:?}, mean {mean:.4} LSB",
        case.dec.yc_separation, case.dec.burst_pll, case.dec.h_pll, gpu_size, gpu.adapter_name
    );
    Some(Stats { max, mean })
}

fn check(case: Case) {
    // 4 フィールド通して、AFC がフィールドをまたいで状態を持つ経路も比べる
    if let Some(s) = compare(&case, 4) {
        assert!(s.mean < 0.35, "mean error {} LSB", s.mean);
        assert!(s.max <= 1.0, "max error {} LSB", s.max);
    }
}

fn native(standard: TvStandard, yc: YcSeparation) -> Case {
    Case { standard, dec: DecoderParams { yc_separation: yc, ..DecoderParams::default() }, host_size: None }
}

#[test]
fn ntsc_notch_matches_cpu() {
    check(native(TvStandard::Ntsc, YcSeparation::Notch));
}

#[test]
fn ntsc_comb_matches_cpu() {
    check(native(TvStandard::Ntsc, YcSeparation::Comb));
}

#[test]
fn pal_notch_matches_cpu() {
    check(native(TvStandard::Pal, YcSeparation::Notch));
}

#[test]
fn pal_comb_matches_cpu() {
    check(native(TvStandard::Pal, YcSeparation::Comb));
}

/// 副搬送波 PLL の追従係数が 1 未満 (逐次の経路)、AFC の係数と色の帯域も既定から変える
#[test]
fn slow_pll_matches_cpu() {
    for standard in [TvStandard::Ntsc, TvStandard::Pal] {
        let dec = DecoderParams { burst_pll: 0.3, h_pll: 0.6, chroma_bw_mhz: 0.5, ..DecoderParams::default() };
        check(Case { standard, dec, host_size: None });
    }
}

/// 1920×1080 のシーンリニア入力を縮小して通す (vhsPrepare の三角フィルタと 1/2.2 乗)
#[test]
fn downscaled_linear_input_matches_cpu() {
    for standard in [TvStandard::Ntsc, TvStandard::Pal] {
        check(Case { standard, dec: DecoderParams::default(), host_size: Some((1920, 1080)) });
    }
}
