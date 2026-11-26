//! エンジンから書き出したカメラ画像を wgpu の参照ホストに通し、出力を書き出す。
//! エンジン側の GPU 版の出力 (同じフィールド数) と突き合わせるのに使う。
//!
//!   cargo run -p vhs-slang --example replay --release -- <入力.bin> <幅> <高さ> <フィールド数> <出力.bin> [ntsc|pal]
//!
//! 入力はシーンリニアな RGBA f32 (行はテクスチャのメモリ順)、出力は有効映像の大きさの RGBA f32 (シーンリニア)。
//! パラメータは既定値 (Unity の VhsGpuComposite に空の JSON を渡したときと同じ)。

use vhs_core::params::{DecoderParams, EncoderParams};
use vhs_core::TvStandard;
use vhs_slang::{CompositeParams, GpuComposite, GpuContext};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert!(args.len() >= 5, "usage: replay <input.bin> <width> <height> <fields> <output.bin> [ntsc|pal]");
    let bytes = std::fs::read(&args[0]).expect("read input");
    let width: u32 = args[1].parse().expect("width");
    let height: u32 = args[2].parse().expect("height");
    let fields: usize = args[3].parse().expect("fields");
    let standard = if args.get(5).map(String::as_str) == Some("pal") { TvStandard::Pal } else { TvStandard::Ntsc };
    let input: Vec<f32> = bytemuck::cast_slice(&bytes).to_vec();

    let gpu = GpuContext::new().expect("GPU");
    let mut params = CompositeParams::new(standard, &EncoderParams::default(), &DecoderParams::default());
    params.input_linear = 1;
    params.output_linear = 1;
    let mut composite = GpuComposite::new(&gpu, standard, params);
    let out = composite.process(&gpu, &input, width, height, fields);
    std::fs::write(&args[4], bytemuck::cast_slice(&out)).expect("write output");
    let (w, h) = composite.output_size();
    println!("{width}x{height}, {fields} fields -> {w}x{h} on {}", gpu.adapter_name);
}
