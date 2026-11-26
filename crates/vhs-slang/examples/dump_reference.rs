//! エンジン側 (Unity など) の GPU 版を CPU 版と比べるための参照データを書き出す。
//!
//!   cargo run -p vhs-slang --example dump_reference --release -- <出力フォルダ> [ntsc|pal] [幅x高さ]
//!
//! - `pattern.bin`: テストパターン (シーンリニア = 表示用の値の 2.2 乗、RGBA f32、行は上から)。
//!   大きさを指定すると、その大きさで作る (GPU 側は有効映像の大きさへ縮小して読む)
//! - `cpu.bin`: CPU 版で 4 フィールド通した出力 (表示用の値、RGBA u8)。入力の縮小は
//!   Pipeline::process_field_linear と同じ LinearFrame::resized → VideoFrame::copy_from_linear
//! - `info.txt`: 入力の幅・高さ、フィールド数、方式、出力の幅・高さ

use std::path::PathBuf;

use vhs_core::frame::{LinearFrame, VideoFrame};
use vhs_core::params::{DecoderParams, EncoderParams};
use vhs_core::TvStandard;
use vhs_slang::reference::{cpu_decode, test_pattern};

const FIELDS: usize = 4;

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: dump_reference <dir> [ntsc|pal] [WxH]"));
    let standard = match args.next().as_deref() {
        Some("pal") => TvStandard::Pal,
        _ => TvStandard::Ntsc,
    };
    let (w, h) = standard.spec().image_size();
    let (iw, ih) = args
        .next()
        .and_then(|s| {
            let (a, b) = s.split_once('x')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .unwrap_or((w, h));
    std::fs::create_dir_all(&dir)?;

    let linear: Vec<f32> =
        test_pattern(iw, ih).iter().flat_map(|c| [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2)]).collect();
    let rgba: Vec<f32> = linear.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 1.0]).collect();
    std::fs::write(dir.join("pattern.bin"), bytemuck::cast_slice(&rgba))?;

    let frame = LinearFrame::from_raw(iw, ih, linear).expect("frame size");
    let mut video = VideoFrame::new(w, h);
    video.copy_from_linear(&frame.resized(w, h));
    let px: Vec<[f32; 3]> = video.data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    let cpu = cpu_decode(standard, &EncoderParams::default(), &DecoderParams::default(), &px, FIELDS);
    std::fs::write(dir.join("cpu.bin"), &cpu.data)?;
    std::fs::write(dir.join("info.txt"), format!("{iw} {ih} {FIELDS} {standard:?} {w} {h}\n"))?;
    println!("wrote {iw}x{ih} -> {w}x{h} {standard:?} reference to {}", dir.display());
    Ok(())
}
