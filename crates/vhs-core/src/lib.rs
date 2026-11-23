//! vhs-core: コンポジット映像信号 (NTSC など、[`timing::Standard`]) と磁気テープ記録のシミュレータ。
//!
//! 信号の流れ:
//! ```text
//! RgbFrame / LinearFrame(HDR) ─ camera (撮像管) ─ VideoFrame(f32) ─ encoder ─▶ Field(コンポジット, 4fsc) ─ tape ─▶ rf ─▶ Field ─ decoder ─▶ RgbaFrame
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
