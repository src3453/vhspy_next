//! NTSC コンポジット → RGB の受像機モデル。
//!
//! - 水平 AFC: 同期の後縁を検出し、時定数付きで追従する (遅い時間軸変動は吸収、速い変動は残る)
//! - Y/C 分離: ノッチ (ドットクロール/クロスカラーが出る) または 1H くし形
//! - バースト位相に同期して I/Q を同期検波、ACC とカラーキラー付き

use crate::filter::{Biquad, Cascade, fractional_shift};
use crate::frame::{Field, RgbaFrame};
use crate::params::{Deinterlace, DecoderParams, YcSeparation};
use crate::timing::*;
use rayon::prelude::*;

pub struct Decoder {
    /// AFC が推定している同期後縁位置 (フィールドをまたいで保持)
    h_sync_pos: f32,
    /// AFC 補正後のフィールド
    aligned: Vec<f32>,
    corrections: Vec<f32>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self {
            h_sync_pos: SYNC_END as f32,
            aligned: vec![0.0; FIELD_LINES * LINE_SAMPLES],
            corrections: vec![0.0; FIELD_LINES],
        }
    }
}

/// 同期の後縁 (-20 IRE を下から上へ横切る位置) を小数精度で探す
fn find_sync_edge(line: &[f32]) -> Option<f32> {
    let thr = ire::SYNC / 2.0;
    let mut low_seen = false;
    for n in 1..(ACTIVE_START - 10) {
        if line[n - 1] < thr {
            low_seen = true;
        }
        if low_seen && line[n - 1] < thr && line[n] >= thr {
            let t = (thr - line[n - 1]) / (line[n] - line[n - 1]);
            return Some(n as f32 - 1.0 + t);
        }
    }
    None
}

impl Decoder {
    pub fn decode_field(&mut self, field: &Field, p: &DecoderParams, deint: Deinterlace, out: &mut RgbaFrame) {
        assert_eq!(out.width, ACTIVE_WIDTH);
        assert_eq!(out.height, FRAME_LINES);

        // --- 水平 AFC (逐次) ---
        let k = p.h_pll.clamp(0.0, 1.0);
        for l in 0..FIELD_LINES {
            if let Some(pos) = find_sync_edge(field.line(l)) {
                self.h_sync_pos += k * (pos - self.h_sync_pos);
            }
            self.corrections[l] = self.h_sync_pos - SYNC_END as f32;
        }
        let corr = &self.corrections;
        self.aligned.par_chunks_mut(LINE_SAMPLES).enumerate().for_each(|(l, dst)| {
            fractional_shift(field.line(l), dst, -corr[l], ire::BLANK);
        });

        // --- フィルタ準備 ---
        let notch = Cascade::new(&[Biquad::notch(FSC, 1.0)]);
        let c_bp = Cascade::new(&[Biquad::bandpass(FSC, 1.5)]);
        let c_lp = Cascade::butter_lowpass(2, p.chroma_bw_mhz * 1e6);
        let hue = p.hue_deg.to_radians() + 33f32.to_radians();
        let aligned = &self.aligned;
        let parity = field.parity as usize;

        let row_bytes = ACTIVE_WIDTH * 4;
        let rows: Vec<(usize, &mut [u8])> = out.data.chunks_mut(row_bytes).enumerate().collect();
        // 書き込み先のフィールドライン番号ごとに行をまとめる
        let mut targets: Vec<Vec<&mut [u8]>> = (0..FIELD_LINES).map(|_| Vec::new()).collect();
        for (row, buf) in rows {
            match deint {
                Deinterlace::Weave if row % 2 == parity => targets[row / 2].push(buf),
                Deinterlace::Bob => targets[row / 2].push(buf),
                _ => {}
            }
        }

        targets.into_par_iter().enumerate().for_each(|(l, mut dsts)| {
            let x = &aligned[l * LINE_SAMPLES..(l + 1) * LINE_SAMPLES];
            let mut y = [0f32; LINE_SAMPLES];
            let mut c = [0f32; LINE_SAMPLES];

            // Y/C 分離
            let use_comb = p.yc_separation == YcSeparation::Comb && l > 0;
            if use_comb {
                let prev = &aligned[(l - 1) * LINE_SAMPLES..l * LINE_SAMPLES];
                let mut bp = c_bp;
                for n in 0..LINE_SAMPLES {
                    c[n] = bp.process(0.5 * (x[n] - prev[n]));
                    y[n] = x[n] - c[n];
                }
            } else {
                let (mut nf, mut bp) = (notch, c_bp);
                for n in 0..LINE_SAMPLES {
                    y[n] = nf.process(x[n]);
                    c[n] = bp.process(x[n]);
                }
            }

            // バースト位相・振幅の検出
            // バーストゲートはテープ再生時の包絡線の遅れも含むよう広めに取る。
            // 位相は全体の和から、振幅 (ACC 用) は 1 周期ごとの包絡線のピークから求める。
            let (mut zr, mut zi) = (0f32, 0f32);
            let mut burst_amp = 0f32;
            for cyc in (BURST_START..BURST_END + 16).step_by(4) {
                let (mut cr, mut ci) = (0f32, 0f32);
                for n in cyc..cyc + 4 {
                    cr += c[n] * COS4[n & 3];
                    ci -= c[n] * SIN4[n & 3];
                }
                zr += cr;
                zi += ci;
                burst_amp = burst_amp.max(0.5 * (cr * cr + ci * ci).sqrt());
            }
            let psi = zi.atan2(zr) - std::f32::consts::FRAC_PI_2;

            let mut gain = p.saturation / ire::SPAN;
            if p.acc {
                gain *= (ire::BURST / burst_amp.max(1e-3)).min(8.0);
            }
            if p.color_killer && burst_amp < 3.0 {
                gain = 0.0;
            }

            let mut cos_t = [0f32; 4];
            let mut sin_t = [0f32; 4];
            for k in 0..4 {
                let a = k as f32 * std::f32::consts::FRAC_PI_2 + psi + hue;
                cos_t[k] = a.cos();
                sin_t[k] = a.sin();
            }

            let (mut ilp, mut qlp) = (c_lp, c_lp);
            let mut rgba = [0u8; ACTIVE_WIDTH * 4];
            for n in 0..ACTIVE_END {
                let i = ilp.process(2.0 * c[n] * cos_t[n & 3]);
                let q = qlp.process(2.0 * c[n] * sin_t[n & 3]);
                if n < ACTIVE_START {
                    continue;
                }
                let yy = (y[n] - ire::BLACK) / ire::SPAN * p.contrast + p.brightness;
                let (i, q) = (i * gain, q * gain);
                let r = yy + 0.956 * i + 0.619 * q;
                let g = yy - 0.272 * i - 0.647 * q;
                let b = yy - 1.106 * i + 1.703 * q;
                let o = (n - ACTIVE_START) * 4;
                rgba[o] = to_u8(r);
                rgba[o + 1] = to_u8(g);
                rgba[o + 2] = to_u8(b);
                rgba[o + 3] = 255;
            }
            for d in dsts.iter_mut() {
                d.copy_from_slice(&rgba);
            }
        });
    }
}

#[inline(always)]
fn to_u8(v: f32) -> u8 {
    (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}
