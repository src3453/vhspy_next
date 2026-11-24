//! 静止画にシミュレーションをかけて PNG 出力、および速度計測。
//!
//! cargo run -p vhs-core --release --example render -- in.png out.png [params.json] [fields] [pan_px]
//!   pan_px: 1 フィールドごとに画像を横へずらす量 (動き補償・圧縮の確認用)
//!   環境変数 VHS_MENU: デッキのメニュー表示 (OSD マークアップ) を重ねる
//!   in が .hdr / .exr ならシーンリニアな HDR 入力として扱う (撮像管カメラに白を超える光が届く)
//! cargo run -p vhs-core --release --example render -- --bench [fields] [params.json]

use std::time::{Duration, Instant};
use vhs_core::{LinearFrame, Params, Pipeline, RgbFrame, Standard};

fn load(path: &str, std: &Standard) -> RgbFrame {
    let (w, h) = std.image_size();
    let img = image::open(path).expect("open image").to_rgb8();
    let img = image::imageops::resize(&img, w as u32, h as u32, image::imageops::FilterType::Triangle);
    RgbFrame::from_raw(w, h, img.into_raw()).unwrap()
}

/// 入力画像 (8 bit またはシーンリニア HDR)
enum Input {
    Rgb(RgbFrame),
    Linear(LinearFrame),
}

fn load_input(path: &str, std: &Standard) -> Input {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".hdr") || lower.ends_with(".exr") {
        let img = image::open(path).expect("open image").to_rgb32f();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let f = LinearFrame::from_raw(w, h, img.into_raw()).unwrap();
        // 露出合わせの目安: 輝度の分布
        let mut lum: Vec<f32> = f
            .data
            .chunks_exact(3)
            .map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])
            .filter(|v| v.is_finite())
            .collect();
        lum.sort_by(f32::total_cmp);
        let q = |r: f64| lum[((lum.len() - 1) as f64 * r) as usize];
        let mean = lum.iter().map(|&v| v as f64).sum::<f64>() / lum.len() as f64;
        eprintln!(
            "{w}x{h} HDR: mean {mean:.3}  p50 {:.3}  p90 {:.3}  p99 {:.3}  p99.9 {:.3}  max {:.1}",
            q(0.5),
            q(0.9),
            q(0.99),
            q(0.999),
            q(1.0)
        );
        let (w, h) = std.image_size();
        Input::Linear(f.resized(w, h))
    } else {
        Input::Rgb(load(path, std))
    }
}

fn test_pattern(std: &Standard) -> RgbFrame {
    vhs_core::pattern::color_bars(std)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("--bench") {
        let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(600);
        let params: Params = match args.get(2) {
            Some(p) => serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
            None => Params::default(),
        };
        let input = test_pattern(params.standard.spec());
        let mut p = Pipeline::new(params);
        for _ in 0..30 {
            p.process_field(&input);
        }
        let (mut enc, mut tape, mut dec) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
        let t = Instant::now();
        for _ in 0..n {
            p.process_field(&input);
            enc += p.timings.encode;
            tape += p.timings.tape;
            dec += p.timings.decode;
        }
        let el = t.elapsed();
        let ms = |d: Duration| d.as_secs_f64() * 1e3 / n as f64;
        println!(
            "{n} fields: {:.1} fields/s  (encode {:.2} ms, tape {:.2} ms, decode {:.2} ms)",
            n as f64 / el.as_secs_f64(),
            ms(enc),
            ms(tape),
            ms(dec)
        );
        return;
    }

    let params: Params = match args.get(2) {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
        None => Params::default(),
    };
    let std = params.standard.spec();
    let input = match args.first().map(|s| s.as_str()) {
        Some("--pattern") | None => Input::Rgb(test_pattern(std)),
        Some(path) => load_input(path, std),
    };
    let out_path = args.get(1).cloned().unwrap_or_else(|| "out.png".into());
    let mut p = Pipeline::new(params);
    // VHS_MENU にマークアップを入れるとデッキのメニュー表示を重ねる
    p.set_menu(std::env::var("VHS_MENU").ok());
    // 状態 (AFC, ジッタ, レート制御) が落ち着くまで数フィールド回す
    let fields: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
    let pan: f32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    for i in 0..fields {
        let dx = (pan * i as f32) as isize;
        match &input {
            Input::Rgb(f) => {
                p.process_field(&RgbFrame { data: shifted(&f.data, f.width, f.height, dx), ..*f });
            }
            Input::Linear(f) => {
                p.process_field_linear(&LinearFrame { data: shifted(&f.data, f.width, f.height, dx), ..*f });
            }
        }
        if let Some(c) = p.codec_stats().filter(|_| i + 16 >= fields) {
            let kind = if c.intra_frame { 'I' } else { 'P' };
            println!("field {i}: {kind} q={:.1} {:.0} kbit", c.q, c.bits as f64 / 1e3);
        }
        if let Some(d) = p.dv_stats().filter(|_| i + 4 >= fields) {
            println!("field {i}: DV qno={:.2} truncated={} lost_mbs={} corrupt_mbs={}", d.qno_mean, d.truncated_blocks, d.lost_mbs, d.corrupt_mbs);
        }
    }
    let out = p.output();
    // 表示用に 4:3 (NTSC なら 640x480) へリサイズ
    let img = image::RgbaImage::from_raw(out.width as u32, out.height as u32, out.data.clone()).unwrap();
    let (dw, dh) = (out.height as u32 * 4 / 3, out.height as u32);
    let img = image::imageops::resize(&img, dw, dh, image::imageops::FilterType::Triangle);
    img.save(&out_path).unwrap();
    println!("wrote {out_path}");
}

/// 横方向に巡回シフトした画像 (RGB 3 チャンネル)
fn shifted<T: Copy>(data: &[T], width: usize, height: usize, dx: isize) -> Vec<T> {
    let mut o = data.to_vec();
    if dx == 0 {
        return o;
    }
    let w = width as isize;
    for y in 0..height {
        for x in 0..width {
            let sx = (x as isize - dx).rem_euclid(w) as usize;
            let (d, s) = ((y * width + x) * 3, (y * width + sx) * 3);
            o[d..d + 3].copy_from_slice(&data[s..s + 3]);
        }
    }
    o
}
