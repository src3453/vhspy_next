//! vhs-core: コンポジット映像信号 (NTSC など、[`timing::Standard`]) と磁気テープ記録のシミュレータ。
//!
//! 信号の流れ:
//! ```text
//! RgbFrame / LinearFrame(HDR) ─ camera (撮像管) ─ VideoFrame(f32) ─ encoder ─▶ Field(コンポジット, 4fsc) ─ (rf) ─ tape ─▶ (rf) ─▶ Field ─ decoder ─▶ RgbaFrame
//!          ─ capture (YCbCr・色差間引き・DCT 圧縮) ─▶ RgbaFrame
//! ```
//! 音声は `audio` (テープ音声トラック → 再生機 → キャプチャ ADC) で別に処理する。
//! GUI や入出力には依存しない純粋なライブラリ。

pub mod audio;
pub mod camera;
pub mod capture;
pub mod decoder;
pub mod encoder;
pub mod filter;
pub mod frame;
pub mod osd;
pub mod params;
pub mod pattern;
pub mod pipeline;
pub mod rf;
pub mod rng;
pub mod tape;
pub mod timing;

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
        let line = pl.composite().line(s.field_lines / 2);
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
}
