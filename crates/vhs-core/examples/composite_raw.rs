//! 静止画 (またはカラーバー) をシミュレーションに通し、コンポジット信号を raw サンプル列で書き出す。
//! FL2000 系 USB-VGA アダプタ + osmo-fl2k で実機のテレビへ入力するためのもの。
//!
//! cargo run -p vhs-core --release --example composite_raw -- [options] <in.png | --pattern> <out.raw>
//!   --params p.json   パラメータ (アプリで保存したプリセット)
//!   --fields N        書き出すフィールド数 (既定 600 = NTSC 約 10 秒)
//!   --format F        s8 (既定, fl2k_file 用) / u8 / s16
//!   --upsample K      4fsc の K 倍で出す (1/2/4/8、既定 2)
//!   --lo IRE --hi IRE 最小・最大コードにする IRE (既定 sync−5 / 白+35)
//!   --vfs V           DAC の最大コードの電圧 [V, 75Ω 負荷]。sync〜白 が 1.0 V になるよう --hi を決める
//!   --yc              S 映像: <out>.y.raw / <out>.c.raw に分けて書く (Y→R, C→G など)
//!
//! 再生例: fl2k_file -s <表示されるサンプルレート> out.raw

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use vhs_core::rawout::{RawChannel, RawOptions, RawWriter, SampleFormat, describe};
use vhs_core::{Params, Pipeline, RgbFrame};

fn usage() -> ! {
    eprintln!(
        "usage: composite_raw [--params p.json] [--fields N] [--format s8|u8|s16] [--upsample K]\n\
         \x20                    [--lo IRE] [--hi IRE] [--vfs V] [--yc] <in.png|--pattern> <out.raw>"
    );
    std::process::exit(2)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    path.with_file_name(format!("{stem}.{suffix}{ext}"))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut params_path, mut fields, mut format, mut upsample) = (None, 600usize, SampleFormat::S8, 2usize);
    let (mut lo, mut hi, mut vfs, mut yc) = (None::<f32>, None::<f32>, None::<f32>, false);
    let mut pos = Vec::new();
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--params" => params_path = Some(val()),
            "--fields" => fields = val().parse().unwrap_or_else(|_| usage()),
            "--format" => format = SampleFormat::from_name(&val()).unwrap_or_else(|| usage()),
            "--upsample" => upsample = val().parse().unwrap_or_else(|_| usage()),
            "--lo" => lo = Some(val().parse().unwrap_or_else(|_| usage())),
            "--hi" => hi = Some(val().parse().unwrap_or_else(|_| usage())),
            "--vfs" => vfs = Some(val().parse().unwrap_or_else(|_| usage())),
            "--yc" => yc = true,
            "-h" | "--help" => usage(),
            _ => pos.push(a),
        }
    }
    let [input, out] = <[String; 2]>::try_from(pos).unwrap_or_else(|_| usage());

    let params: Params = match &params_path {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p).expect("read params")).expect("parse params"),
        None => Params::default(),
    };
    if !params.encoder.enabled {
        eprintln!("encoder が無効なのでコンポジット信号がありません");
        std::process::exit(1);
    }
    let std = params.tv();
    let frame = if input == "--pattern" {
        vhs_core::pattern::color_bars(std)
    } else {
        let (w, h) = std.image_size();
        let img = image::open(&input).expect("open image").to_rgb8();
        let img = image::imageops::resize(&img, w as u32, h as u32, image::imageops::FilterType::Triangle);
        RgbFrame::from_raw(w, h, img.into_raw()).unwrap()
    };

    let mut opts = RawOptions::new(std, format, upsample);
    if let Some(v) = lo {
        opts.ire_lo = v;
    }
    if let Some(v) = vfs {
        opts = opts.with_full_scale_volts(std, v);
    }
    if let Some(v) = hi {
        opts.ire_hi = v;
    }
    if opts.clips_white(std) {
        eprintln!("警告: 白 ({} IRE) が最大コード ({:.1} IRE) を超えて切れます", std.ire.white, opts.ire_hi);
    }

    let out = PathBuf::from(out);
    let open = |p: &Path| BufWriter::with_capacity(1 << 20, File::create(p).expect("create output"));
    let mut writers = if yc {
        vec![
            (with_suffix(&out, "y"), RawWriter::new(open(&with_suffix(&out, "y")), std, opts, RawChannel::Luma)),
            (with_suffix(&out, "c"), RawWriter::new(open(&with_suffix(&out, "c")), std, opts, RawChannel::Chroma)),
        ]
    } else {
        vec![(out.clone(), RawWriter::new(open(&out), std, opts, RawChannel::Composite))]
    };

    let mut pipe = Pipeline::new(params);
    let mut separate_seen = false;
    for i in 0..fields {
        pipe.process_field(&frame);
        separate_seen |= pipe.composite().separate;
        for (_, w) in &mut writers {
            w.write_field(pipe.composite()).expect("write");
        }
        if i % 60 == 59 {
            eprint!("\r{}/{fields} fields", i + 1);
        }
    }
    eprintln!();
    if yc && !separate_seen {
        eprintln!("警告: S 映像になっていません (出力の接続を S 映像にし、テープを有効に)。C は無信号です");
    }
    let rate = writers[0].1.sample_rate();
    for (path, mut w) in writers {
        w.flush().expect("flush");
        println!("wrote {} ({} samples)", path.display(), w.samples());
    }
    print!("{}", describe(std, &opts, rate));
    println!("play: fl2k_file -s {} {}", rate.round() as u64, out.display());
}
