//! 撮像管カメラの Slang (wgpu) 版を CPU 版 (vhs_core::camera::CameraSim) と比べる。
//!
//! 入力はシーンリニアな float (白 = 1 を超える光を含む)。動くハイライトで残像・コメットテール・焼き付きの
//! 状態がフィールドをまたいで同じに進むかを見る。プリアンプ雑音は GPU では別の乱数なので、
//! 一致を見るテストでは切り、統計だけを別のテストで比べる。GPU が無い環境ではスキップする。

use vhs_core::camera::{CameraInput, CameraSim};
use vhs_core::decoder::Decoder;
use vhs_core::encoder::encode_field;
use vhs_core::frame::{Field, LinearFrame, RgbaFrame};
use vhs_core::params::{CameraParams, DecoderParams, EncoderParams};
use vhs_core::TvStandard;
use vhs_slang::{CompositeParams, GpuCamera, GpuComposite, GpuContext};

const SEED: u64 = 0x1234_5678_9ABC_DEF0;

fn gpu() -> Option<GpuContext> {
    match GpuContext::new() {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipping GPU comparison: {e}");
            None
        }
    }
}

/// 個体差・劣化・ヒステリシス・ACT など、雑音以外の経路をひととおり通すパラメータ
fn busy_params() -> CameraParams {
    CameraParams {
        enabled: true,
        noise_pct: 0.0,
        aging: 0.3,
        mismatch: 0.5,
        registration_px: 1.2,
        blemishes: 6,
        hysteresis: 0.02,
        hysteresis_fields: 8.0,
        bias_light: 0.01,
        burn_rate: 0.5,
        exposure_ev: 0.5,
        ..CameraParams::default()
    }
}

/// シーンリニアな HDR の入力。グラデーションの上を白の `peak` 倍の光源が横に動く
fn hdr_scene(w: usize, h: usize, field: u64, peak: f32) -> LinearFrame {
    let mut f = LinearFrame::new(w, h);
    let (cx, cy) = (w as f32 * (0.2 + 0.05 * field as f32), h as f32 * 0.4);
    let r = w as f32 * 0.03;
    for y in 0..h {
        for x in 0..w {
            let u = x as f32 / w as f32;
            let v = y as f32 / h as f32;
            let mut c = [0.6 * u, 0.4 * v + 0.1, 0.3 * (1.0 - u) * v];
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            if d < r {
                c = [peak, peak * 0.9, peak * 0.7];
            }
            f.data[(y * w + x) * 3..][..3].copy_from_slice(&c);
        }
    }
    f
}

fn to_rgba(f: &LinearFrame) -> Vec<f32> {
    f.data.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 1.0]).collect()
}

struct Diff {
    max: f32,
    mean: f32,
}

/// 8 bit 単位の差 (CPU は RGB、GPU は RGBA)
fn diff(cpu: &[f32], gpu: &[f32]) -> Diff {
    let n = cpu.len() / 3;
    let mut max = 0f32;
    let mut sum = 0f64;
    for i in 0..n {
        for c in 0..3 {
            let d = (cpu[i * 3 + c] - gpu[i * 4 + c]).abs() * 255.0;
            max = max.max(d);
            sum += d as f64;
        }
    }
    Diff { max, mean: (sum / (n * 3) as f64) as f32 }
}

/// `host_size` の HDR 入力を `fields` フィールド通し、各フィールドの後のカメラ出力を比べる
fn compare_camera(standard: TvStandard, host_size: Option<(usize, usize)>, p: &CameraParams, fields: u64) {
    let Some(gpu) = gpu() else { return };
    let std = standard.spec();
    let (w, h) = std.image_size();
    let (hw, hh) = host_size.unwrap_or((w, h));
    let mut cpu = CameraSim::default();
    cpu.set_field_rate(std.field_rate);
    let mut cam = GpuCamera::new(&gpu, standard, p, SEED);
    let mut worst = Diff { max: 0.0, mean: 0.0 };
    for f in 0..fields {
        let scene = hdr_scene(hw, hh, f, 6.0);
        // CPU は Pipeline::process_field_linear と同じく、有効映像の大きさに縮小してからカメラに入れる
        let resized = scene.resized(w, h);
        let expect = cpu.process(CameraInput::Linear(&resized), p, SEED, f).data.clone();
        cam.upload_input(&gpu, &to_rgba(&scene), hw as u32, hh as u32, true);
        cam.process_field(&gpu, f);
        let got = cam.read_output(&gpu);
        let d = diff(&expect, &got);
        worst.max = worst.max.max(d.max);
        worst.mean = worst.mean.max(d.mean);
    }
    eprintln!(
        "camera {standard:?} input {:?}: max {:.3} LSB, worst field mean {:.4} LSB on {}",
        (hw, hh),
        worst.max,
        worst.mean,
        gpu.adapter_name
    );
    assert!(worst.max <= 1.0, "max error {} LSB", worst.max);
    assert!(worst.mean < 0.05, "mean error {} LSB", worst.mean);
}

#[test]
fn hdr_float_input_matches_cpu() {
    for standard in [TvStandard::Ntsc, TvStandard::Pal] {
        compare_camera(standard, None, &busy_params(), 12);
    }
}

#[test]
fn default_params_match_cpu() {
    let p = CameraParams { enabled: true, noise_pct: 0.0, ..CameraParams::default() };
    compare_camera(TvStandard::Ntsc, None, &p, 6);
}

/// 1920×1080 の float 入力を GPU で縮小してからカメラに入れる
#[test]
fn downscaled_hdr_float_input_matches_cpu() {
    compare_camera(TvStandard::Ntsc, Some((1920, 1080)), &busy_params(), 6);
}

/// 白を超える float の光が切られずに届く: 白の 8 倍の光源は、白 (1.0) で頭打ちの光源より長く尾を引く
#[test]
fn over_white_float_light_reaches_target() {
    let Some(gpu) = gpu() else { return };
    let standard = TvStandard::Ntsc;
    let (w, h) = standard.spec().image_size();
    let p = CameraParams {
        enabled: true,
        noise_pct: 0.0,
        mismatch: 0.0,
        registration_px: 0.0,
        beam_current: 1.5,
        target_saturation: 20.0,
        camera_gamma: 1.0,
        ..CameraParams::default()
    };
    let tail = |level: f32| {
        let mut cam = GpuCamera::new(&gpu, standard, &p, 1);
        let on = vec![level; w * h * 4];
        let off = vec![0.0; w * h * 4];
        cam.upload_input(&gpu, &on, w as u32, h as u32, true);
        for f in 0..10 {
            cam.process_field(&gpu, f);
        }
        cam.upload_input(&gpu, &off, w as u32, h as u32, true);
        let mut n = 0;
        for f in 10..60u64 {
            cam.process_field(&gpu, f);
            // 偶数フィールドが書く行 0 の中央
            if f & 1 == 0 && cam.read_output(&gpu)[(w / 2) * 4] > 0.08 {
                n += 1;
            }
        }
        n
    };
    let clipped = tail(1.0);
    let hdr = tail(8.0);
    eprintln!("comet tail fields: white {clipped}, 8x white {hdr}");
    assert!(hdr >= clipped + 3, "clipped {clipped} hdr {hdr}");
}

/// プリアンプ雑音は乱数が違うので、平坦な面の平均と標準偏差で比べる
#[test]
fn preamp_noise_statistics_match_cpu() {
    let Some(gpu) = gpu() else { return };
    let standard = TvStandard::Ntsc;
    let std = standard.spec();
    let (w, h) = std.image_size();
    let p = CameraParams {
        enabled: true,
        noise_pct: 3.0,
        mismatch: 0.0,
        registration_px: 0.0,
        shading: 0.0,
        ..CameraParams::default()
    };
    let flat = LinearFrame::from_raw(w, h, vec![0.18; w * h * 3]).unwrap();
    let mut cpu = CameraSim::default();
    cpu.set_field_rate(std.field_rate);
    let mut cam = GpuCamera::new(&gpu, standard, &p, SEED);
    cam.upload_input(&gpu, &to_rgba(&flat), w as u32, h as u32, true);
    for f in 0..4 {
        cpu.process(CameraInput::Linear(&flat), &p, SEED, f);
        cam.process_field(&gpu, f);
    }
    let expect = cpu.output().unwrap().data.clone();
    let got = cam.read_output(&gpu);
    // 端の画素 (輪郭補償・フレア) を避けて中央の G を見る
    let stats = |get: &dyn Fn(usize) -> f32| {
        let vals: Vec<f32> =
            (h / 4..3 * h / 4).flat_map(|y| (w / 4..3 * w / 4).map(move |x| (y, x))).map(|(y, x)| get(y * w + x)).collect();
        let mean = vals.iter().sum::<f32>() / vals.len() as f32;
        let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / vals.len() as f32;
        (mean, var.sqrt())
    };
    let (cm, cs) = stats(&|i| expect[i * 3 + 1]);
    let (gm, gs) = stats(&|i| got[i * 4 + 1]);
    eprintln!("noise: CPU mean {cm:.4} sd {cs:.4}, GPU mean {gm:.4} sd {gs:.4}");
    assert!((cm - gm).abs() < 0.002, "mean CPU {cm} GPU {gm}");
    assert!((gs / cs - 1.0).abs() < 0.1, "sd CPU {cs} GPU {gs}");
}

/// カメラの出力テクスチャをそのままコンポジットのエンコード → デコードに流し、CPU の同じ経路と比べる
#[test]
fn camera_into_composite_matches_cpu() {
    let Some(gpu) = gpu() else { return };
    for standard in [TvStandard::Ntsc, TvStandard::Pal] {
        let std = standard.spec();
        let (w, h) = std.image_size();
        let p = busy_params();
        let enc = EncoderParams::default();
        let dec = DecoderParams::default();

        let mut cpu_cam = CameraSim::default();
        cpu_cam.set_field_rate(std.field_rate);
        let mut field = Field::new(std);
        let mut decoder = Decoder::new(std);
        let mut cpu_out = RgbaFrame::new(w, h);

        let mut cam = GpuCamera::new(&gpu, standard, &p, SEED);
        let mut composite = GpuComposite::new(&gpu, standard, CompositeParams::new(standard, &enc, &dec));
        let fields = 6u64;
        let mut gpu_out = Vec::new();
        for f in 0..fields {
            let scene = hdr_scene(w, h, f, 6.0);
            let video = cpu_cam.process(CameraInput::Linear(&scene), &p, SEED, f);
            field.set_index(f);
            encode_field(video, &mut field, &enc);
            decoder.decode_field(&field, &dec, false, &mut cpu_out);

            cam.upload_input(&gpu, &to_rgba(&scene), w as u32, h as u32, true);
            cam.process_field(&gpu, f);
            composite.set_input_texture(cam.output_texture());
            composite.run(&gpu, 1);
            if f + 1 == fields {
                gpu_out = composite.read_output(&gpu);
            }
        }
        let mut max = 0f32;
        let mut sum = 0f64;
        for i in 0..w * h {
            for c in 0..3 {
                let d = (gpu_out[i * 4 + c] * 255.0 - cpu_out.data[i * 4 + c] as f32).abs();
                max = max.max(d);
                sum += d as f64;
            }
        }
        let mean = sum / (w * h * 3) as f64;
        eprintln!("camera → composite {standard:?}: max {max:.2} LSB, mean {mean:.4} LSB");
        assert!(max <= 1.0, "max error {max} LSB");
        assert!(mean < 0.35, "mean error {mean} LSB");
    }
}
