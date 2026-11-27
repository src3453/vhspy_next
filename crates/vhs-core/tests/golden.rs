//! リファクタ用の回帰テスト: 全段を有効にしたパイプラインの出力を基準ファイルと比べる。
//!
//! 基準は `target/golden/` に置く (リポジトリには入れない)。無ければ今回の出力を基準として保存する。
//! 意図して出力を変えたときは `target/golden/` を消して作り直す。

use std::path::PathBuf;
use vhs_core::{Params, Pipeline, pattern};

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/golden")
}

fn run(name: &str, mut p: Params) {
    p.seed = 1234;
    let input = pattern::color_bars(p.standard.spec());
    let mut pl = Pipeline::new(p);
    let mut out = Vec::new();
    for _ in 0..12 {
        pl.process_field(&input);
        out.extend_from_slice(&pl.output().data);
    }
    let comp: Vec<u8> = pl.composite().data.iter().flat_map(|v| v.to_le_bytes()).collect();

    let dir = golden_dir();
    std::fs::create_dir_all(&dir).unwrap();
    for (suffix, data) in [("rgba", &out), ("composite", &comp)] {
        let path = dir.join(format!("{name}.{suffix}"));
        match std::fs::read(&path) {
            Ok(want) => {
                assert_eq!(want.len(), data.len(), "{name}.{suffix}: size changed");
                let diffs = want.iter().zip(data.iter()).filter(|(a, b)| a != b).count();
                assert_eq!(diffs, 0, "{name}.{suffix}: {diffs} bytes differ");
            }
            Err(_) => {
                std::fs::write(&path, data).unwrap();
                eprintln!("wrote golden {}", path.display());
            }
        }
    }
}

fn full() -> Params {
    let mut p = Params::default();
    p.tape.enabled = true;
    p.tape.dropout_rate = p.tape.dropout_rate.max(0.01);
    p.transport.tracking_error = 0.2;
    p.rf.enabled = true;
    p.rf.snow_ire = 3.0;
    p.rf.ghost_gain = 0.1;
    p.rf.interference_ire = 1.0;
    p.rf.sound_beat = 0.5;
    p.osd.enabled = true;
    p.capture.enabled = true;
    p.capture.codec = true;
    p
}

#[test]
fn golden_ntsc_full() {
    run("ntsc_full", full());
}

#[test]
fn golden_ntsc_camera() {
    let mut p = full();
    p.camera.enabled = true;
    p.osd.mode = vhs_core::params::OsdMode::Composite;
    run("ntsc_camera", p);
}

#[test]
fn golden_pal_full() {
    let mut p = full();
    p.standard = vhs_core::TvStandard::Pal;
    p.decoder.burst_pll = 0.5;
    p.decoder.yc_separation = vhs_core::params::YcSeparation::Comb;
    run("pal_full", p);
}

#[test]
fn golden_ntsc_fm_svideo() {
    let mut p = full();
    p.tape.luma_model = vhs_core::params::LumaModel::Fm;
    p.tape.fm_channel_mhz = 4.5;
    p.output.connection = vhs_core::params::Connection::SVideo;
    p.rf.enabled = false;
    run("ntsc_fm_svideo", p);
}

#[test]
fn golden_ntsc_tbc() {
    let mut p = full();
    p.transport.jitter_samples = 2.0;
    p.tbc.enabled = true;
    p.tbc.error_rate = 0.005;
    run("ntsc_tbc", p);
}

#[test]
fn golden_ntsc_comb3d() {
    let mut p = full();
    p.decoder.yc_separation = vhs_core::params::YcSeparation::Comb3d;
    p.decoder.luma_shelf_db = 3.0;
    p.decoder.chroma_shelf_db = 2.0;
    p.decoder.chroma_demod = vhs_core::params::ChromaDemod::IqWide;
    run("ntsc_comb3d", p);
}

#[test]
fn golden_ntsc_comb() {
    let mut p = full();
    p.decoder.yc_separation = vhs_core::params::YcSeparation::Comb;
    p.decoder.gamma = 2.5;
    run("ntsc_comb", p);
}
