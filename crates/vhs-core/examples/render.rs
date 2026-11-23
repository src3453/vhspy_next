//! 静止画にシミュレーションをかけて PNG 出力、および速度計測。
//!
//! cargo run -p vhs-core --release --example render -- in.png out.png [params.json]
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
    // 状態 (AFC, ジッタ) が落ち着くまで数フィールド回す
    for _ in 0..8 {
        p.process_field(&input);
    }
    let out = p.output();
    // 表示用に 4:3 (640x480) へリサイズ
    let img = image::RgbaImage::from_raw(out.width as u32, out.height as u32, out.data.clone()).unwrap();
    let img = image::imageops::resize(&img, 640, 480, image::imageops::FilterType::Triangle);
    img.save(&out_path).unwrap();
    println!("wrote {out_path}");
}
