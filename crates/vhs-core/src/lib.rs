//! vhs-core: コンポジット映像信号 (NTSC など、[`timing::Standard`]) と磁気テープ記録のシミュレータ。
//!
//! 信号の流れ:
//! ```text
//! RgbFrame / LinearFrame(HDR) ─ adjust (画質調整) ─ camera (撮像管) ─ VideoFrame(f32) ─ encoder ─▶ Field(コンポジット, 4fsc) ─ (rf) ─ tape ─ (tbc) ─▶ (rf) ─▶ Field ─ decoder ─▶ RgbaFrame
//!          ─ capture (YCbCr・色差間引き・DCT 圧縮) ─▶ RgbaFrame
//! ```
//! 音声は `audio` (テープ音声トラック → 再生機 → キャプチャ ADC) で別に処理する。
//! GUI や入出力には依存しない純粋なライブラリ。

pub mod adjust;
pub mod audio;
pub mod camera;
pub mod caption;
pub mod capture;
pub mod decoder;
pub mod deinterlace;
pub mod dv;
pub mod encoder;
pub mod filter;
pub mod frame;
pub mod osd;
pub mod params;
pub mod pattern;
pub mod pipeline;
mod nr;
pub mod raster;
pub mod rf;
pub mod rng;
pub mod tape;
pub mod tbc;
pub mod timing;
pub mod vbi;

pub use frame::{LinearFrame, RgbFrame, RgbaFrame, VideoFrame};
pub use params::Params;
pub use pipeline::{Pipeline, StageTimings};
pub use timing::{Standard, TvStandard};

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(r: u8, g: u8, b: u8) -> RgbFrame {
        let (w, h) = timing::NTSC.image_size();
        let mut f = RgbFrame::new(w, h);
        for px in f.data.chunks_exact_mut(3) {
            px.copy_from_slice(&[r, g, b]);
        }
        f
    }

    /// テープなし・ノイズなしでは色がほぼ保たれること (エンコーダ/デコーダの符号規約の確認)
    #[test]
    fn clean_roundtrip_preserves_color() {
        let mut p = Params::default();
        p.tape.enabled = false;
        p.decoder.yc_separation = params::YcSeparation::Comb;
        for (r, g, b) in [(200, 40, 40), (40, 200, 40), (40, 40, 200), (230, 200, 30), (128, 128, 128)] {
            let mut pl = Pipeline::new(p.clone());
            let input = flat(r, g, b);
            for _ in 0..4 {
                pl.process_field(&input);
            }
            let out = pl.output();
            // 中央付近の画素
            let (w, h) = pl.output_size();
            let i = (h / 2 * w + w / 2) * 4;
            let got = &out.data[i..i + 3];
            for (c, want) in [r, g, b].iter().enumerate() {
                let d = (got[c] as i32 - *want as i32).abs();
                assert!(d <= 10, "input {:?} got {:?}", (r, g, b), got);
            }
        }
    }

    /// RF の配置 (テープの前 / 後 / 両方) で出力が変わり、両方では 2 回とも効くこと
    #[test]
    fn digital_paths_preserve_color() {
        // コンポジットなし (カメラ・OSD だけ) と DV (FireWire / S 映像のアナログ出力) で色が保たれること
        for standard in [TvStandard::Ntsc, TvStandard::Pal] {
            let (w, h) = standard.spec().image_size();
            let mut input = RgbFrame::new(w, h);
            for px in input.data.chunks_exact_mut(3) {
                px.copy_from_slice(&[200, 120, 40]);
            }
            for (dv, composite, tol) in [(false, false, 1), (true, false, 3), (true, true, 12)] {
                let mut p = Params { standard, ..Params::default() };
                p.encoder.enabled = composite;
                p.tape.enabled = dv;
                p.tape.format = params::TapeFormat::Dv;
                p.output.connection = params::Connection::SVideo;
                p.decoder.yc_separation = params::YcSeparation::Comb;
                let mut pl = Pipeline::new(p);
                for _ in 0..4 {
                    pl.process_field(&input);
                }
                assert_eq!(pl.dv_stats().is_some(), dv);
                let out = pl.output();
                let i = (h / 2 * w + w / 2) * 4;
                for (c, want) in [200, 120, 40].iter().enumerate() {
                    let d = (out.data[i + c] as i32 - want).abs();
                    assert!(d <= tol, "{standard:?} dv={dv} composite={composite}: ch{c} {} vs {want}", out.data[i + c]);
                }
            }
        }
    }

    #[test]
    fn rf_placement_changes_output() {
        use params::RfPlacement::*;
        let input = flat(120, 90, 60);
        let render = |placement: Option<params::RfPlacement>| {
            let mut p = Params::default();
            p.tape.enabled = true;
            p.rf.enabled = placement.is_some();
            p.rf.placement = placement.unwrap_or_default();
            p.rf.snow_ire = 6.0;
            let mut pl = Pipeline::new(p);
            pl.process_field(&input);
            pl.output().data.clone()
        };
        let noise = |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(x, y)| (*x as f64 - *y as f64).powi(2)).sum::<f64>();
        let off = render(None);
        let (before, after, both) = (render(Some(BeforeTape)), render(Some(AfterTape)), render(Some(Both)));
        assert_ne!(before, after);
        assert_ne!(both, before);
        assert_ne!(both, after);
        // 独立な雑音が 2 回乗るので、片方だけより明らかに荒れる
        let (e_before, e_after, e_both) = (noise(&before, &off), noise(&after, &off), noise(&both, &off));
        assert!(e_both > 1.3 * e_before.max(e_after), "{e_before} {e_after} {e_both}");
    }

    fn flat_in(s: &Standard, rgb: [u8; 3]) -> RgbFrame {
        let (w, h) = s.image_size();
        let mut f = RgbFrame::new(w, h);
        for px in f.data.chunks_exact_mut(3) {
            px.copy_from_slice(&rgb);
        }
        f
    }

    /// 出力の (行, 列) の RGB
    fn pixel(pl: &Pipeline, row: usize, col: usize) -> [i32; 3] {
        let (w, _) = pl.output_size();
        let i = (row * w + col) * 4;
        let d = &pl.output().data;
        [d[i] as i32, d[i + 1] as i32, d[i + 2] as i32]
    }

    fn run_flat(p: &Params, rgb: [u8; 3], fields: usize) -> Pipeline {
        let mut pl = Pipeline::new(p.clone());
        let input = flat_in(p.standard.spec(), rgb);
        for _ in 0..fields {
            pl.process_field(&input);
        }
        pl
    }

    fn pal_clean() -> Params {
        let mut p = Params::default();
        p.standard = TvStandard::Pal;
        p.encoder.luma_bw_mhz = 5.0;
        p.tape.enabled = false;
        p
    }

    /// アーティファクトカラー: 4fsc に同期した白黒の縞 (2 サンプル白・2 サンプル黒) は、
    /// 228 周期ラインなら全ラインで同じ色になり、227.5 周期ならラインごとに補色へ入れ替わる
    #[test]
    fn artifact_color_needs_228_cycle_lines() {
        let row_diff = |line_228: bool| {
            let mut p = Params::default();
            p.ntsc_228 = line_228;
            p.tape.enabled = false;
            let s = p.tv();
            let (w, h) = s.image_size();
            let mut input = RgbFrame::new(w, h);
            for (i, px) in input.data.chunks_exact_mut(3).enumerate() {
                let v = if ((i % w) + s.active_start) & 2 == 0 { 255 } else { 0 };
                px.fill(v);
            }
            let mut pl = Pipeline::new(p);
            for _ in 0..4 {
                pl.process_field(&input);
            }
            let (ow, oh) = pl.output_size();
            let (a, b) = (pixel(&pl, oh / 2, ow / 2), pixel(&pl, oh / 2 + 1, ow / 2));
            let chroma = (a[0] - a[1]).abs().max((a[1] - a[2]).abs()).max((a[0] - a[2]).abs());
            let diff = (0..3).map(|c| (a[c] - b[c]).abs()).max().unwrap();
            (chroma, diff, a, b)
        };
        let (chroma, diff, a, b) = row_diff(true);
        assert!(chroma > 60 && diff < 12, "228: {a:?} / {b:?}");
        let (_, diff, a, b) = row_diff(false);
        assert!(diff > 60, "227.5: {a:?} / {b:?}");
    }

    const COLORS: [[u8; 3]; 5] = [[200, 40, 40], [40, 200, 40], [40, 40, 200], [230, 200, 30], [128, 128, 128]];

    /// PAL: テープなしで色が保たれる (ノッチ / 2H くし形、遅延線あり / なし)
    #[test]
    fn pal_clean_roundtrip_preserves_color() {
        for comb in [false, true] {
            for delay in [false, true] {
                let mut p = pal_clean();
                p.decoder.yc_separation = if comb { params::YcSeparation::Comb } else { params::YcSeparation::Notch };
                p.decoder.pal_delay_line = delay;
                for rgb in COLORS {
                    let pl = run_flat(&p, rgb, 4);
                    let (w, h) = pl.output_size();
                    assert_eq!((w, h), (922, 576));
                    // 隣り合う 2 行とも (PAL スイッチの両方の向き)
                    for row in [h / 2, h / 2 + 1, h / 2 + 2, h / 2 + 3] {
                        let got = pixel(&pl, row, w / 2);
                        for c in 0..3 {
                            assert!(
                                (got[c] - rgb[c] as i32).abs() <= 10,
                                "comb={comb} delay={delay} input {rgb:?} row {row} got {got:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// 色相 (位相) の誤差: PAL は遅延線で彩度低下に変わり、遅延線なしではライン交互に色相がずれる
    /// (ハノーバーバー)。NTSC はそのまま色相ずれになる
    #[test]
    fn pal_phase_error_behaviour() {
        let rgb = [200u8, 40, 40];
        let hue_of = |c: [i32; 3]| {
            let (r, g, b) = (c[0] as f32, c[1] as f32, c[2] as f32);
            let y = 0.299 * r + 0.587 * g + 0.114 * b;
            let (u, v) = (0.492 * (b - y), 0.877 * (r - y));
            (v.atan2(u).to_degrees(), (u * u + v * v).sqrt())
        };
        let (h0, s0) = hue_of(rgb.map(i32::from));

        // 遅延線あり: 色相は保たれ、彩度が cos(30°) 倍ほどに下がる
        let mut p = pal_clean();
        p.decoder.hue_deg = 30.0;
        let pl = run_flat(&p, rgb, 4);
        let (w, h) = pl.output_size();
        for row in [h / 2, h / 2 + 2] {
            let (hh, ss) = hue_of(pixel(&pl, row, w / 2));
            assert!((hh - h0).abs() < 4.0, "delay line: hue {hh} vs {h0}");
            assert!((ss / s0 - 0.866).abs() < 0.08, "delay line: saturation ratio {}", ss / s0);
        }

        // 遅延線なし: 上下で逆向きに色相がずれる
        p.decoder.pal_delay_line = false;
        let pl = run_flat(&p, rgb, 4);
        let (ha, _) = hue_of(pixel(&pl, h / 2, w / 2));
        let (hb, _) = hue_of(pixel(&pl, h / 2 + 2, w / 2));
        assert!((ha - h0).abs() > 20.0 && (hb - h0).abs() > 20.0, "simple PAL: {ha} {hb} vs {h0}");
        assert!((ha - h0).signum() != (hb - h0).signum(), "simple PAL: {ha} {hb} vs {h0}");

        // NTSC: 全ラインが同じ向きに色相ずれ
        let mut p = Params::default();
        p.tape.enabled = false;
        p.decoder.hue_deg = 30.0;
        let pl = run_flat(&p, rgb, 4);
        let (w, h) = pl.output_size();
        let (ha, _) = hue_of(pixel(&pl, h / 2, w / 2));
        let (hb, _) = hue_of(pixel(&pl, h / 2 + 2, w / 2));
        assert!((ha - h0).abs() > 20.0 && (ha - hb).abs() < 4.0, "NTSC: {ha} {hb} vs {h0}");
    }

    /// 3 次元 Y/C 分離: 静止画では色が保たれ、ラインごとに変わる細かい輝度の模様もクロスカラーにならない。
    /// 急に絵が変わっても (動き検出でラインくし形に切り替わり) 前の色が残らない
    #[test]
    fn comb3d_separation() {
        for pal in [false, true] {
            let mut p = if pal { pal_clean() } else { Params::default() };
            p.tape.enabled = false;
            p.decoder.yc_separation = params::YcSeparation::Comb3d;
            for rgb in COLORS {
                let pl = run_flat(&p, rgb, 10);
                let (w, h) = pl.output_size();
                for row in [h / 2, h / 2 + 1] {
                    let got = pixel(&pl, row, w / 2);
                    for c in 0..3 {
                        assert!((got[c] - rgb[c] as i32).abs() <= 10, "pal={pal} input {rgb:?} got {got:?}");
                    }
                }
            }

            // ラインごとに位相の変わる副搬送波付近の灰色の模様 (くし形でも取り切れない)
            let s = p.standard.spec();
            let (w, h) = s.image_size();
            let period = s.fs / (s.fsc * 0.9);
            let mut tex = RgbFrame::new(w, h);
            for (i, px) in tex.data.chunks_exact_mut(3).enumerate() {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                let v = (128.0 + 60.0 * (std::f32::consts::TAU * x / period + 1.3 * y * y).sin()) as u8;
                px.copy_from_slice(&[v, v, v]);
            }
            let false_color = |sep: params::YcSeparation| {
                let mut q = p.clone();
                q.decoder.yc_separation = sep;
                let mut pl = Pipeline::new(q);
                for _ in 0..10 {
                    pl.process_field(&tex);
                }
                let mut sum = 0.0;
                let mut n = 0.0;
                for row in h / 3..h * 2 / 3 {
                    for col in w / 3..w * 2 / 3 {
                        let c = pixel(&pl, row, col);
                        sum += ((c[0] - c[1]).abs() + (c[1] - c[2]).abs() + (c[2] - c[0]).abs()) as f64;
                        n += 1.0;
                    }
                }
                sum / n
            };
            let (c3, c2) = (false_color(params::YcSeparation::Comb3d), false_color(params::YcSeparation::Comb));
            assert!(c3 < c2 * 0.3, "pal={pal}: cross color 3d {c3} vs line comb {c2}");

            // 赤 → 青へ切り替えた直後のフィールドも青
            let mut pl = Pipeline::new(p.clone());
            let (red, blue) = (flat_in(s, [200, 40, 40]), flat_in(s, [40, 40, 200]));
            for _ in 0..8 {
                pl.process_field(&red);
            }
            pl.process_field(&blue);
            let row = h / 2 + (pl.field_index() as usize - 1) % 2;
            let got = pixel(&pl, row, w / 2);
            assert!(got[2] > 150 && got[0] < 90, "pal={pal}: after cut got {got:?}");

            // 動き適応なし: 静止画は正しく出る
            let mut q = p.clone();
            q.decoder.motion_adaptive = false;
            for rgb in COLORS {
                let pl = run_flat(&q, rgb, 10);
                let got = pixel(&pl, h / 2 + 1, w / 2);
                for c in 0..3 {
                    assert!((got[c] - rgb[c] as i32).abs() <= 10, "pal={pal} no motion adaptation: {rgb:?} got {got:?}");
                }
            }
            // 切り替え直後は前の絵とのフレームくし形になり、色が崩れる (ただし黒く潰れはしない)
            let mut pl = Pipeline::new(q);
            for _ in 0..8 {
                pl.process_field(&red);
            }
            pl.process_field(&blue);
            let bad = pixel(&pl, row, w / 2);
            let dist = (0..3).map(|c| (bad[c] - [40, 40, 200][c]).abs()).max().unwrap();
            assert!(dist > 30, "pal={pal}: no motion adaptation, after cut got {bad:?}");
            assert!(bad.iter().sum::<i32>() > 120, "pal={pal}: no motion adaptation, after cut got {bad:?}");

            // バッファのビット数が少ないと静止画でも量子化雑音が乗る (ゆるやかな傾斜で見る)
            let mut ramp = RgbFrame::new(w, h);
            for (i, px) in ramp.data.chunks_exact_mut(3).enumerate() {
                let v = (60.0 + 120.0 * (i % w) as f32 / w as f32) as u8;
                px.copy_from_slice(&[v, v / 2 + 40, 200 - v / 2]);
            }
            let roughness = |bits: u32| {
                let mut q = p.clone();
                q.decoder.frame_buffer_bits = bits;
                let mut pl = Pipeline::new(q);
                for _ in 0..10 {
                    pl.process_field(&ramp);
                }
                let row = h / 2 + (pl.field_index() as usize - 1) % 2;
                let px: Vec<[i32; 3]> = (w / 4..w * 3 / 4).map(|x| pixel(&pl, row, x)).collect();
                px.windows(2).map(|a| (0..3).map(|c| (a[1][c] - a[0][c]).pow(2)).sum::<i32>() as f64).sum::<f64>()
            };
            let (r4, r10) = (roughness(4), roughness(10));
            assert!(r4 > r10 * 2.0, "pal={pal}: 4-bit {r4} vs 10-bit {r10}");
        }
    }

    /// 色副搬送波 PLL をゆっくりにしても、位相の乱れがなければライン当たりの位相の進みを正しく予測して色が保たれる
    #[test]
    fn slow_burst_pll_keeps_color() {
        for pal in [false, true] {
            let mut p = if pal { pal_clean() } else { Params::default() };
            p.tape.enabled = false;
            p.decoder.burst_pll = 0.2;
            for rgb in COLORS {
                let pl = run_flat(&p, rgb, 4);
                let (w, h) = pl.output_size();
                for row in [h / 2, h / 2 + 2] {
                    let got = pixel(&pl, row, w / 2);
                    for c in 0..3 {
                        assert!((got[c] - rgb[c] as i32).abs() <= 10, "pal={pal} input {rgb:?} got {got:?}");
                    }
                }
            }
        }
    }

    /// FM モデル: ノイズ・クロマ重畳なしなら、変調 → 伝送路 → 復調で輝度と色がほぼ保たれる (NTSC / PAL)
    #[test]
    fn fm_luma_clean_roundtrip() {
        for pal in [false, true] {
            let mut p = if pal { pal_clean() } else { Params::default() };
            p.tape.enabled = true;
            p.tape.luma_model = params::LumaModel::Fm;
            p.tape.fm_noise = 0.0;
            p.tape.color_under_level = 0.0;
            p.tape.chroma_noise_ire = 0.0;
            p.tape.chroma_phase_noise_deg = 0.0;
            p.tape.dropout_rate = 0.0;
            p.transport.jitter_samples = 0.0;
            p.transport.wow_samples = 0.0;
            p.transport.vertical_jitter = 0.0;
            p.transport.head_switch_noise = 0.0;
            // 比較対象: 同じテープ条件で FM を近似しない (ノイズなし) Simple モデル。
            // 色は FM モデルの方が濃く残る (復調後のローパスで輝度経路に残る副搬送波が減るため) ので、
            // 輝度と色相だけ比べる
            let mut simple = p.clone();
            simple.tape.luma_model = params::LumaModel::Simple;
            simple.tape.luma_noise_ire = 0.0;
            simple.tape.bearding_strength = 0.0;
            let luma = |c: [i32; 3]| 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
            let hue = |c: [i32; 3]| {
                let y = luma(c);
                (0.877 * (c[0] as f32 - y)).atan2(0.492 * (c[2] as f32 - y)).to_degrees()
            };
            for rgb in [[16u8, 16, 16], [128, 128, 128], [235, 235, 235], [200, 40, 40], [40, 40, 200]] {
                let pl = run_flat(&p, rgb, 6);
                let (w, h) = pl.output_size();
                let got = pixel(&pl, h / 2 + 1, w / 2);
                let want = pixel(&run_flat(&simple, rgb, 6), h / 2 + 1, w / 2);
                let msg = format!("pal={pal} input {rgb:?} fm {got:?} simple {want:?}");
                assert!((luma(got) - luma(want)).abs() <= 2.0, "{msg}");
                if rgb[0] != rgb[1] {
                    assert!((hue(got) - hue(rgb.map(i32::from))).abs() <= 6.0, "{msg}");
                }
                // 搬送波の漏れ・復調の揺れが残っていないこと (平坦な入力は平坦なまま)
                let row: Vec<f32> = (w / 8..w * 7 / 8).map(|x| pixel(&pl, h / 2 + 1, x)[1] as f32).collect();
                let mean = row.iter().sum::<f32>() / row.len() as f32;
                let sd = (row.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / row.len() as f32).sqrt();
                assert!(sd < 0.6, "{msg}: sd {sd}");
            }
        }
    }

    /// S 映像: 色はコンポジットと同じように再生され、輝度の細かい縞はノッチで削られずに残る
    #[test]
    fn svideo_keeps_color_and_luma_detail() {
        for pal in [false, true] {
            let mut p = if pal { pal_clean() } else { Params::default() };
            p.tape.enabled = true;
            p.tape.luma_noise_ire = 0.0;
            p.tape.chroma_noise_ire = 0.0;
            p.tape.chroma_phase_noise_deg = 0.0;
            p.tape.dropout_rate = 0.0;
            p.transport.jitter_samples = 0.0;
            p.transport.wow_samples = 0.0;
            p.transport.vertical_jitter = 0.0;
            p.output.connection = params::Connection::SVideo;
            for rgb in COLORS {
                let pl = run_flat(&p, rgb, 6);
                let (w, h) = pl.output_size();
                let got = pixel(&pl, h / 2 + 1, w / 2);
                for c in 0..3 {
                    // テープの色の飽和で少し落ちる
                    assert!((got[c] - rgb[c] as i32).abs() <= 28, "pal={pal} input {rgb:?} got {got:?}");
                }
            }

            // 約 2.3 MHz の縦縞 (テープの輝度帯域内、受像機のノッチの帯域内)
            let s = p.standard.spec();
            let (w, h) = s.image_size();
            let period = s.fs / 2.3e6;
            let mut stripes = RgbFrame::new(w, h);
            for (i, px) in stripes.data.chunks_exact_mut(3).enumerate() {
                let x = (i % w) as f32;
                let v = (128.0 + 90.0 * (std::f32::consts::TAU * x / period).sin()) as u8;
                px.copy_from_slice(&[v, v, v]);
            }
            let contrast = |p: &Params| {
                let mut pl = Pipeline::new(p.clone());
                for _ in 0..6 {
                    pl.process_field(&stripes);
                }
                // コンポジットではクロスカラー (偽色) が出るので、RGB から輝度を求めて比べる
                let row: Vec<f32> = (w / 3..w * 2 / 3)
                    .map(|x| {
                        let c = pixel(&pl, h / 2 + 1, x);
                        0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32
                    })
                    .collect();
                row.iter().cloned().fold(f32::MIN, f32::max) - row.iter().cloned().fold(f32::MAX, f32::min)
            };
            let mut composite = p.clone();
            composite.output.connection = params::Connection::Composite;
            let (sv, cv) = (contrast(&p), contrast(&composite));
            assert!(sv > cv * 1.1, "pal={pal}: S-video contrast {sv} vs composite {cv}");
        }
    }

    /// PAL をテープに通しても色が保たれる。垂直揺れでライン順がずれても PAL スイッチの向きを取り違えない
    #[test]
    fn pal_through_tape_keeps_color() {
        let mut p = pal_clean();
        p.tape.enabled = true;
        p.tape.luma_noise_ire = 0.0;
        p.tape.chroma_noise_ire = 0.0;
        p.tape.chroma_phase_noise_deg = 0.0;
        p.tape.dropout_rate = 0.0;
        p.transport.vertical_jitter = 4.0;
        let rgb = [200u8, 40, 40];
        let mut pl = Pipeline::new(p.clone());
        let input = flat_in(p.standard.spec(), rgb);
        for f in 0..24 {
            pl.process_field(&input);
            if f < 4 {
                continue;
            }
            let (w, h) = pl.output_size();
            // Weave なので今のフィールドの行を見る
            let row = h / 2 + (f & 1);
            let got = pixel(&pl, row, w / 2);
            // テープの色飽和・帯域で多少落ちるが、赤のまま (V を取り違えると色相が大きく回る)
            assert!(got[0] > 130 && got[1] < 110 && got[2] < 110, "field {f}: got {got:?}");
        }
    }

    /// カメラのホワイトクリップまでのスーパーホワイトがコンポジット信号に残る (8 bit で切られない)
    #[test]
    fn camera_super_white_reaches_composite() {
        let mut p = Params::default();
        p.tape.enabled = false;
        p.camera.enabled = true;
        p.camera.white_clip = 1.08;
        p.camera.knee = 2.0;
        p.camera.noise_pct = 0.0;
        let (w, h) = timing::NTSC.image_size();
        let mut hdr = LinearFrame::new(w, h);
        hdr.data.fill(4.0);
        let mut pl = Pipeline::new(p);
        for _ in 0..4 {
            pl.process_field_linear(&hdr);
        }
        let s = pl.standard();
        let c = pl.composite();
        let line = c.line(c.active_index(s.field_lines / 2));
        let mid = (s.active_start + s.active_end()) / 2;
        let peak = line[mid - 50..mid + 50].iter().sum::<f32>() / 100.0;
        // 7.5 + 92.5 × 1.08^0.45 ≒ 103.3 IRE
        assert!(peak > 102.0 && peak < 105.0, "peak {peak} IRE");
    }

    #[test]
    fn params_json_roundtrip() {
        let p = Params::default();
        let s = serde_json_like(&p);
        assert!(s.contains("tape"));
    }

    fn serde_json_like(p: &Params) -> String {
        // dev-dependency の serde_json で確認
        serde_json::to_string(p).unwrap()
    }
    /// 画面の中ほどに帰線区間の黒帯 (行全体がほぼ黒) が見えたフィールドがあるか
    fn black_band_seen(p: Params, fields: usize) -> bool {
        let s = p.standard.spec();
        let input = flat_in(s, [180, 180, 180]);
        let mut pl = Pipeline::new(p);
        let (w, h) = pl.output_size();
        let mut seen = false;
        for f in 0..fields {
            pl.process_field(&input);
            if f < 4 {
                continue;
            }
            let out = pl.output();
            for row in h / 8..h * 7 / 8 {
                let sum: u32 = (0..w).step_by(8).map(|x| out.data[(row * w + x) * 4] as u32).sum();
                if sum / (w as u32 / 8) < 30 {
                    seen = true;
                }
            }
        }
        seen
    }

    fn clean_ntsc() -> Params {
        let mut p = Params::default();
        p.tape.enabled = false;
        p
    }

    #[test]
    fn locked_picture_does_not_roll() {
        assert!(!black_band_seen(clean_ntsc(), 40));
        let mut p = pal_clean();
        p.tape.enabled = false;
        assert!(!black_band_seen(p, 40));
    }

    /// 垂直ホールドを速い側にずらすと同期で引き込めず、画面が流れて帰線区間が見える
    #[test]
    fn vertical_hold_off_rolls() {
        let mut p = clean_ntsc();
        p.decoder.v_hold_pct = 10.0;
        assert!(black_band_seen(p, 40));
    }

    /// 垂直同期 (幅広パルス) が弱いと積分回路がトリガされず、自走して流れる
    #[test]
    fn weak_vertical_sync_rolls() {
        let mut p = clean_ntsc();
        p.signal.vertical_sync.attenuation_pct = 80.0;
        assert!(black_band_seen(p, 40));
    }

    /// 画面を下げると、帰線区間の VITS (白バー) が画面の上に見えてくる
    #[test]
    fn vbi_shows_when_picture_is_lowered() {
        let bright_top_rows = |vpos: f32| {
            let mut p = clean_ntsc();
            p.decoder.vertical_position = vpos;
            let input = flat_in(&timing::NTSC, [0, 0, 0]);
            let mut pl = Pipeline::new(p);
            for _ in 0..4 {
                pl.process_field(&input);
            }
            // VITS の白バーは同期から 12〜30 µs (有効映像の左寄り)
            (0..16).filter(|&row| pixel(&pl, row, 150)[0] > 180).count()
        };
        assert_eq!(bright_top_rows(0.0), 0);
        assert!(bright_top_rows(-8.0) >= 2);
    }

    /// 受像機の字幕デコーダがライン 21 から字幕を組み立てる
    #[test]
    fn caption_decoder_reads_line21() {
        let s = &timing::NTSC;
        let mut p = Params::default();
        p.decoder.caption_display = true;
        let (w, h) = s.image_size();
        let video = VideoFrame::new(w, h);
        let mut field = frame::Field::new(s);
        let mut dec = decoder::Decoder::new(s);
        let mut out = RgbaFrame::new(w, h);
        for i in 0..160 {
            field.set_index(i);
            encoder::encode_field_full(&video, &mut field, &p.encoder, &p.signal, &p.vbi, 1);
            dec.decode_field(&field, &p.decoder, false, &mut out);
        }
        let rows = dec.captions().rows();
        assert!(rows.iter().any(|r| r.contains("VHS NEXT CLOSED CAPTION TEST")), "{rows:?}");
    }
}
