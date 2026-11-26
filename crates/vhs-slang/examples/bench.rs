//! GPU 版と CPU 版の 1 フィールドあたりの処理時間の目安。
//!
//!   cargo run -p vhs-slang --example bench --release
//!
//! - GPU (通し): 入力を送ったあと 240 フィールドを続けて流し、終わるまで待った時間をフィールド数で割る。
//!   括弧内は同じ処理の GPU タイムライン上の時間 (CPU 側のコマンド作成を含まない、5 回の中央値)。
//!   どちらも先に 2000 フィールド流して GPU のクロックを上げてから測る
//! - GPU (カーネル別): 1 フィールドをカーネルごとのパスに分けてタイムスタンプで測る (TIMESTAMP_QUERY があれば)
//! - CPU: 同じ範囲 (エンコード + 受像機、劣化なし) を vhs-core (rayon) で回す
//!
//! 入力の大きさは BENCH_SIZE=幅x高さ で変えられる (既定 1920x1080)。

use std::time::Instant;

use vhs_core::params::{DecoderParams, EncoderParams};
use vhs_core::TvStandard;
use vhs_slang::reference::{cpu_decode, test_pattern};
use vhs_slang::{CompositeParams, GpuComposite, GpuContext};

const FIELDS: usize = 240;

fn main() {
    let enc = EncoderParams::default();
    let dec = DecoderParams::default();
    let gpu = GpuContext::new().expect("GPU");
    let (w, h) = std::env::var("BENCH_SIZE")
        .ok()
        .and_then(|s| {
            let (a, b) = s.split_once('x')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .unwrap_or((1920u32, 1080u32));

    for standard in [TvStandard::Ntsc, TvStandard::Pal] {
        let px = test_pattern(w as usize, h as usize);
        let rgba: Vec<f32> = px.iter().flat_map(|c| [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2), 1.0]).collect();
        let mut params = CompositeParams::new(standard, &enc, &dec);
        params.input_linear = 1;
        let mut composite = GpuComposite::new(&gpu, standard, params);
        composite.process(&gpu, &rgba, w, h, 4); // パイプライン生成・入力の転送
        // GPU のクロックが省電力で下がっていると数倍ぶれるので、負荷をかけて上げてから測る
        composite.run(&gpu, 2000);
        composite.wait(&gpu);

        let t = Instant::now();
        composite.run(&gpu, FIELDS);
        composite.wait(&gpu);
        let gpu_ms = t.elapsed().as_secs_f64() * 1e3 / FIELDS as f64;
        let mut timeline: Vec<f64> = (0..5)
            .filter_map(|_| {
                composite.run(&gpu, 200);
                composite.run_timed(&gpu, FIELDS).map(|ms| ms / FIELDS as f64)
            })
            .collect();
        timeline.sort_by(f64::total_cmp);
        let timeline = timeline.get(timeline.len() / 2).map_or(String::new(), |ms| format!(" (GPU timeline {ms:.3})"));

        let (aw, ah) = standard.spec().image_size();
        let small = test_pattern(aw, ah);
        let n = 60;
        let t = Instant::now();
        cpu_decode(standard, &enc, &dec, &small, n);
        let cpu_ms = t.elapsed().as_secs_f64() * 1e3 / n as f64;

        println!(
            "{standard:?} {w}x{h} on {}: GPU {gpu_ms:.3}{timeline} ms/field, CPU (vhs-core, rayon) {cpu_ms:.3} ms/field",
            gpu.adapter_name
        );
        composite.run(&gpu, 1000);
        // 1 回目はクロックが上がりきらないことがあるので、何回か測って最小を取る
        let mut best: Option<Vec<(&str, f64)>> = None;
        for _ in 0..8 {
            if let Some(p) = composite.profile_field(&gpu) {
                best = Some(match best {
                    None => p,
                    Some(b) => b.into_iter().zip(p).map(|((n, x), (_, y))| (n, x.min(y))).collect(),
                });
            }
        }
        if let Some(p) = best {
            let total: f64 = p.iter().map(|(_, ms)| ms).sum();
            let parts: Vec<String> = p.iter().map(|(n, ms)| format!("{n} {ms:.3}")).collect();
            println!("  kernels (ms): {} | sum {total:.3}", parts.join(", "));
        }
    }
}
