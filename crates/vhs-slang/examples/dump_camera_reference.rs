//! エンジン側 (Unity など) の撮像管カメラの GPU 版を CPU 版と比べるための参照データを書き出す。
//!
//!   cargo run -p vhs-slang --example dump_camera_reference --release -- <出力フォルダ> [ntsc|pal]
//!
//! - `params.json`: Params の JSON (camera を有効にし、比べられるようにプリアンプ雑音を切る)
//! - `scene_<f>.bin`: フィールド f の入力。シーンリニアな HDR (白の 6 倍の光源が動く)、RGBA f32、有効映像の大きさ
//! - `camera_<f>.bin`: CPU 版のカメラ出力 (表示用の値、RGB f32)
//! - `maps.bin`: 感度マップ R/G/B、暗電流マップ R/G/B (f32 の面を 6 枚)。エンジン側の個体差の移植を確かめる
//! - `composite.bin`: カメラ → エンコード → デコードを全フィールド通した最後の出力 (表示用の値、RGBA u8)
//! - `info.txt`: 幅、高さ、フィールド数、方式

use std::path::PathBuf;

use vhs_core::camera::{CameraInput, CameraSim, TubeMaps};
use vhs_core::decoder::Decoder;
use vhs_core::encoder::encode_field;
use vhs_core::frame::{Field, LinearFrame, RgbaFrame};
use vhs_core::params::{CameraParams, Params};
use vhs_core::TvStandard;

const FIELDS: u64 = 6;
const SEED: u64 = 0x1234_5678_9ABC_DEF0;

/// tests/camera.rs と同じ (雑音以外の経路をひととおり通す)
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

/// tests/camera.rs と同じ
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

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: dump_camera_reference <dir> [ntsc|pal]"));
    let standard = match args.next().as_deref() {
        Some("pal") => TvStandard::Pal,
        _ => TvStandard::Ntsc,
    };
    let std = standard.spec();
    let (w, h) = std.image_size();
    std::fs::create_dir_all(&dir)?;

    let params = Params { standard, camera: busy_params(), seed: SEED, ..Params::default() };
    std::fs::write(dir.join("params.json"), serde_json::to_string_pretty(&params).expect("json"))?;
    let p = &params.camera;

    let maps = TubeMaps::new(p, SEED, w, h);
    let planes: Vec<f32> = maps.sens.iter().chain(&maps.dark).flatten().copied().collect();
    std::fs::write(dir.join("maps.bin"), bytemuck::cast_slice(&planes))?;

    let mut cam = CameraSim::default();
    cam.set_field_rate(std.field_rate);
    let mut field = Field::new(std);
    let mut decoder = Decoder::new(std);
    let mut out = RgbaFrame::new(w, h);
    for f in 0..FIELDS {
        let scene = hdr_scene(w, h, f, 6.0);
        let rgba: Vec<f32> = scene.data.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 1.0]).collect();
        std::fs::write(dir.join(format!("scene_{f}.bin")), bytemuck::cast_slice(&rgba))?;
        let video = cam.process(CameraInput::Linear(&scene), p, SEED, f);
        std::fs::write(dir.join(format!("camera_{f}.bin")), bytemuck::cast_slice(&video.data))?;
        field.set_index(f);
        encode_field(video, &mut field, &params.encoder);
        decoder.decode_field(&field, &params.decoder, false, &mut out);
    }
    std::fs::write(dir.join("composite.bin"), &out.data)?;
    std::fs::write(dir.join("info.txt"), format!("{w} {h} {FIELDS} {standard:?}\n"))?;
    println!("wrote {w}x{h} {standard:?} camera reference ({FIELDS} fields) to {}", dir.display());
    Ok(())
}
