//! 静止画にシミュレーションをかけて PNG 出力、および速度計測。
//!
//! cargo run -p vhs-core --release --example render -- in.png out.png [params.json] [fields] [pan_px]
//!   pan_px: 1 フィールドごとに画像を横へずらす量 (動き補償・圧縮の確認用)
//! cargo run -p vhs-core --release --example render -- --bench [fields]

use std::time::{Duration, Instant};
use vhs_core::{INPUT_HEIGHT, INPUT_WIDTH, Params, Pipeline, RgbFrame};

fn load(path: &str) -> RgbFrame {
    let img = image::open(path).expect("open image").to_rgb8();
    let img = image::imageops::resize(&img, INPUT_WIDTH as u32, INPUT_HEIGHT as u32, image::imageops::FilterType::Triangle);
    RgbFrame::from_raw(INPUT_WIDTH, INPUT_HEIGHT, img.into_raw()).unwrap()
}

fn test_pattern() -> RgbFrame {
    vhs_core::pattern::color_bars()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("--bench") {
        let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(600);
        let input = test_pattern();
        let mut p = Pipeline::new(Params::default());
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

    let input = match args.first().map(|s| s.as_str()) {
        Some("--pattern") | None => test_pattern(),
        Some(path) => load(path),
    };
    let out_path = args.get(1).cloned().unwrap_or_else(|| "out.png".into());
    let params: Params = match args.get(2) {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
        None => Params::default(),
    };
    let mut p = Pipeline::new(params);
    // 状態 (AFC, ジッタ, レート制御) が落ち着くまで数フィールド回す
    let fields: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
    let pan: f32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    for i in 0..fields {
        if pan != 0.0 {
            p.process_field(&shifted(&input, (pan * i as f32) as isize));
        } else {
            p.process_field(&input);
        }
        if let Some(c) = p.codec_stats().filter(|_| i + 16 >= fields) {
            let kind = if c.intra_frame { 'I' } else { 'P' };
            println!("field {i}: {kind} q={:.1} {:.0} kbit", c.q, c.bits as f64 / 1e3);
        }
    }
    let out = p.output();
    // 表示用に 4:3 (640x480) へリサイズ
    let img = image::RgbaImage::from_raw(out.width as u32, out.height as u32, out.data.clone()).unwrap();
    let img = image::imageops::resize(&img, 640, 480, image::imageops::FilterType::Triangle);
    img.save(&out_path).unwrap();
    println!("wrote {out_path}");
}

/// 横方向に巡回シフトした画像
fn shifted(f: &RgbFrame, dx: isize) -> RgbFrame {
    let mut o = f.clone();
    let w = f.width as isize;
    for y in 0..f.height {
        for x in 0..f.width {
            let sx = (x as isize - dx).rem_euclid(w) as usize;
            let (d, s) = ((y * f.width + x) * 3, (y * f.width + sx) * 3);
            o.data[d..d + 3].copy_from_slice(&f.data[s..s + 3]);
        }
    }
    o
}
