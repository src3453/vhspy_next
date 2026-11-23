//! vhs-core: NTSC コンポジット信号と磁気テープ記録のシミュレータ。
//!
//! 信号の流れ:
//! ```text
//! RgbFrame ─ encoder ─▶ Field(コンポジット, 4fsc) ─ tape ─▶ rf ─▶ Field ─ decoder ─▶ RgbaFrame
//!          ─ capture (YCbCr・色差間引き・DCT 圧縮) ─▶ RgbaFrame
//! ```
//! 音声は `audio` (テープ音声トラック → 再生機 → キャプチャ ADC) で別に処理する。
//! GUI や入出力には依存しない純粋なライブラリ。

pub mod audio;
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

pub use frame::{RgbFrame, RgbaFrame};
pub use params::Params;
pub use pipeline::{INPUT_HEIGHT, INPUT_WIDTH, OUTPUT_HEIGHT, OUTPUT_WIDTH, Pipeline, StageTimings};

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(r: u8, g: u8, b: u8) -> RgbFrame {
        let mut f = RgbFrame::new(INPUT_WIDTH, INPUT_HEIGHT);
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
            let i = (OUTPUT_HEIGHT / 2 * OUTPUT_WIDTH + OUTPUT_WIDTH / 2) * 4;
            let got = &out.data[i..i + 3];
            for (c, want) in [r, g, b].iter().enumerate() {
                let d = (got[c] as i32 - *want as i32).abs();
                assert!(d <= 10, "input {:?} got {:?}", (r, g, b), got);
            }
        }
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
