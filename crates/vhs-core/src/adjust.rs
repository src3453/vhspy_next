//! 入力映像の画質調整 (パイプラインの先頭、撮像管カメラやエンコーダより前)。
//!
//! カメラの有無に関わらず、入ってきた映像すべてに掛かる。昔のビデオカメラの質感
//! (甘いレンズ・CCD のぼけ、強い輪郭強調によるハロー、色温度のずれ、浅い/濃い発色) を作るためのもの。
//!
//! ```text
//! 入力 ─ ぼけ (レンズ・光学 LPF) ─ ホワイトバランス (色温度・色かぶり) ─ 明るさ/コントラスト/ガンマ
//!      ─ 彩度・色相 ─ 輪郭強調 (輝度のみ・コアリング付き) ─▶ 次段へ
//! ```
//!
//! 処理はガンマ補正済み (表示用, ≒ 1/2.2) の値で行う。HDR (シーンリニア) の入力は
//! 一度 1/2.2 でエンコードして調整し、元のリニアに戻す (白を超える値は保つ)。

use crate::frame::{LinearFrame, RgbFrame, sanitize};
use crate::params::InputParams;
use rayon::prelude::*;

/// Rec.601 の輝度係数
const KR: f32 = 0.299;
const KG: f32 = 0.587;
const KB: f32 = 0.114;

#[derive(Default)]
pub struct InputAdjust {
    w: usize,
    h: usize,
    /// 作業用の RGB (ガンマ補正済み, インターリーブ)
    rgb: Vec<f32>,
    /// 輝度と、そのぼかし
    luma: Vec<f32>,
    luma_blur: Vec<f32>,
    plane: Vec<f32>,
    tmp: Vec<f32>,
    out_rgb: Option<RgbFrame>,
    out_linear: Option<LinearFrame>,
}

impl InputAdjust {
    /// 調整が何もしない設定か (無効、またはすべて既定値)
    pub fn is_identity(p: &InputParams) -> bool {
        !p.enabled || *p == InputParams { enabled: true, ..InputParams::default() }
    }

    fn allocate(&mut self, w: usize, h: usize) {
        if self.w == w && self.h == h && self.rgb.len() == w * h * 3 {
            return;
        }
        self.w = w;
        self.h = h;
        self.rgb = vec![0.0; w * h * 3];
        self.luma = vec![0.0; w * h];
        self.luma_blur = vec![0.0; w * h];
        self.plane = vec![0.0; w * h];
        self.tmp = vec![0.0; w * h];
        self.out_rgb = None;
        self.out_linear = None;
    }

    /// 8 bit (ガンマ補正済み) の入力を調整する
    pub fn process_rgb(&mut self, input: &RgbFrame, p: &InputParams) -> &RgbFrame {
        let (w, h) = (input.width, input.height);
        self.allocate(w, h);
        self.rgb.par_iter_mut().zip(input.data.par_iter()).for_each(|(o, &v)| *o = v as f32 / 255.0);
        self.apply(p);
        let out = self.out_rgb.get_or_insert_with(|| RgbFrame::new(w, h));
        out.data.par_iter_mut().zip(self.rgb.par_iter()).for_each(|(o, &v)| *o = (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8);
        out
    }

    /// シーンリニア (HDR) の入力を調整する
    pub fn process_linear(&mut self, input: &LinearFrame, p: &InputParams) -> &LinearFrame {
        let (w, h) = (input.width, input.height);
        self.allocate(w, h);
        self.rgb.par_iter_mut().zip(input.data.par_iter()).for_each(|(o, &v)| *o = sanitize(v).max(0.0).powf(1.0 / 2.2));
        self.apply(p);
        let out = self.out_linear.get_or_insert_with(|| LinearFrame::new(w, h));
        out.data.par_iter_mut().zip(self.rgb.par_iter()).for_each(|(o, &v)| *o = v.max(0.0).powf(2.2));
        out
    }

    fn apply(&mut self, p: &InputParams) {
        let (w, h) = (self.w, self.h);

        // --- ぼけ (レンズ・光学ローパス) ---
        if p.blur_px > 0.05 {
            for c in 0..3 {
                self.plane.par_iter_mut().zip(self.rgb.par_chunks(3)).for_each(|(o, px)| *o = px[c]);
                blur_plane(&mut self.plane, &mut self.tmp, w, h, p.blur_px, p.blur_px);
                self.rgb.par_chunks_mut(3).zip(self.plane.par_iter()).for_each(|(px, &v)| px[c] = v);
            }
        }

        // --- ホワイトバランス: リニアでのゲイン。ガンマ補正済みの値には gain^(1/2.2) を掛ければ同じ ---
        let t = p.color_temp.clamp(-1.0, 1.0);
        let tint = p.tint.clamp(-1.0, 1.0);
        let (gr, gg, gb) = (1.0 + 0.35 * t, 1.0 - 0.25 * tint, 1.0 - 0.35 * t);
        let k = KR * gr + KG * gg + KB * gb; // 明るさを保つ
        let wb = [(gr / k).max(0.0).powf(1.0 / 2.2), (gg / k).max(0.0).powf(1.0 / 2.2), (gb / k).max(0.0).powf(1.0 / 2.2)];

        let contrast = p.contrast.max(0.0);
        let brightness = p.brightness;
        let inv_gamma = 1.0 / p.gamma.max(0.05);
        let sat = p.saturation.max(0.0);
        let (hs, hc) = p.hue_deg.to_radians().sin_cos();
        let tone = move |v: f32| {
            let v = (v - 0.5) * contrast + 0.5 + brightness;
            if v > 0.0 && inv_gamma != 1.0 { v.powf(inv_gamma) } else { v }
        };

        self.rgb.par_chunks_mut(3).zip(self.luma.par_iter_mut()).for_each(|(px, y_out)| {
            let r = tone(px[0] * wb[0]);
            let g = tone(px[1] * wb[1]);
            let b = tone(px[2] * wb[2]);
            // 彩度・色相: (B-Y, R-Y) の回転と拡縮
            let y = KR * r + KG * g + KB * b;
            let (cb, cr) = (b - y, r - y);
            let (cb2, cr2) = ((cb * hc - cr * hs) * sat, (cb * hs + cr * hc) * sat);
            let r = y + cr2;
            let b = y + cb2;
            let g = (y - KR * r - KB * b) / KG;
            px[0] = r;
            px[1] = g;
            px[2] = b;
            *y_out = y;
        });

        // --- 輪郭強調 (アンシャープマスク, 輝度のみ) ---
        // 負の量は細部を削る (甘い画)。
        // 細部を横成分 Y - Bh と縦成分 Bh - Bhv (Bh: 横だけぼかし, Bhv: 縦横ぼかし) に分け、
        // 縦成分に `sharpness_vertical` を掛ける。0 で横方向だけ (1H 遅延線の垂直補正がない機種)、
        // 1 で普通の縦横同じアンシャープマスク (Y - Bhv) と一致し、その間は線形に変わる
        let amount = p.sharpness;
        let r = p.sharpness_radius_px;
        if amount.abs() > 1e-4 && r > 0.05 {
            let v = p.sharpness_vertical.clamp(0.0, 1.0);
            // luma_blur = Bh, plane = Bhv
            self.luma_blur.copy_from_slice(&self.luma);
            blur_plane(&mut self.luma_blur, &mut self.tmp, w, h, r, 0.0);
            if v > 0.0 {
                self.plane.copy_from_slice(&self.luma_blur);
                blur_plane(&mut self.plane, &mut self.tmp, w, h, 0.0, r);
            } else {
                self.plane.copy_from_slice(&self.luma_blur);
            }
            let coring = p.sharpness_coring.max(0.0);
            let limit = if p.sharpness_limit > 0.0 { p.sharpness_limit } else { f32::INFINITY };
            let detail = self.luma.par_iter().zip(self.luma_blur.par_iter()).zip(self.plane.par_iter());
            self.rgb.par_chunks_mut(3).zip(detail).for_each(|(px, ((&y, &bh), &bhv))| {
                let d = (y - bh) + v * (bh - bhv);
                let d = d.signum() * (d.abs() - coring).max(0.0);
                let add = (d * amount).clamp(-limit, limit);
                for v in px.iter_mut() {
                    *v += add;
                }
            });
        }
    }
}

/// 分離型ガウスぼかし (横・縦で別の σ)
fn blur_plane(plane: &mut [f32], tmp: &mut [f32], w: usize, h: usize, sx: f32, sy: f32) {
    fn kernel(sigma: f32) -> (Vec<f32>, isize) {
        let r = (sigma * 3.0).ceil() as isize;
        let mut k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
        let sum: f32 = k.iter().sum();
        k.iter_mut().for_each(|v| *v /= sum);
        (k, r)
    }
    if sx >= 0.05 {
        let (k, r) = kernel(sx);
        tmp.par_chunks_mut(w).zip(plane.par_chunks(w)).for_each(|(o, src)| {
            for x in 0..w {
                let mut acc = 0.0;
                for (j, kv) in k.iter().enumerate() {
                    let xx = (x as isize + j as isize - r).clamp(0, w as isize - 1) as usize;
                    acc += src[xx] * kv;
                }
                o[x] = acc;
            }
        });
        plane.copy_from_slice(tmp);
    }
    if sy >= 0.05 {
        let (k, r) = kernel(sy);
        tmp.copy_from_slice(plane);
        let tmp = &*tmp;
        plane.par_chunks_mut(w).enumerate().for_each(|(y, o)| {
            o.fill(0.0);
            for (j, kv) in k.iter().enumerate() {
                let yy = (y as isize + j as isize - r).clamp(0, h as isize - 1) as usize;
                for (ov, sv) in o.iter_mut().zip(&tmp[yy * w..(yy + 1) * w]) {
                    *ov += sv * kv;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> InputParams {
        InputParams { enabled: true, ..InputParams::default() }
    }

    fn edge(w: usize, h: usize) -> RgbFrame {
        let mut f = RgbFrame::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if x < w / 2 { 64 } else { 192 };
                f.data[(y * w + x) * 3..][..3].fill(v);
            }
        }
        f
    }

    #[test]
    fn defaults_are_transparent() {
        let src = edge(32, 8);
        let mut a = InputAdjust::default();
        assert!(InputAdjust::is_identity(&on()));
        assert_eq!(a.process_rgb(&src, &on()).data, src.data);
    }

    #[test]
    fn sharpness_overshoots_and_blur_softens() {
        let src = edge(32, 8);
        let mut a = InputAdjust::default();
        let p = InputParams { sharpness: 1.5, sharpness_coring: 0.0, ..on() };
        let out = a.process_rgb(&src, &p);
        let row = &out.data[4 * 32 * 3..5 * 32 * 3];
        assert!(row[(15) * 3] < 64, "暗側にアンダーシュート");
        assert!(row[(16) * 3] > 192, "明側にオーバーシュート");
        // 平坦部は変わらない
        assert_eq!(row[0], 64);

        let p = InputParams { blur_px: 2.0, ..on() };
        let out = a.process_rgb(&src, &p);
        let row = &out.data[4 * 32 * 3..5 * 32 * 3];
        assert!(row[15 * 3] > 64 && row[16 * 3] < 192);
    }

    /// 横縞 (縦方向の輪郭) は「横だけ」では強調されず、「縦横」では強調される
    #[test]
    fn vertical_ratio_controls_horizontal_edges() {
        let (w, h) = (8, 32);
        let mut src = RgbFrame::new(w, h);
        for y in 0..h {
            let v = if y < h / 2 { 64 } else { 192 };
            src.data[y * w * 3..(y + 1) * w * 3].fill(v);
        }
        let mut a = InputAdjust::default();
        let base = InputParams { sharpness: 1.5, ..on() };
        let out = a.process_rgb(&src, &InputParams { sharpness_vertical: 0.0, ..base.clone() }).data.clone();
        assert_eq!(out, src.data, "横方向だけなら横縞は変わらない");
        let out = a.process_rgb(&src, &InputParams { sharpness_vertical: 1.0, ..base.clone() });
        assert!(out.data[(h / 2) * w * 3] > 192 && out.data[(h / 2 - 1) * w * 3] < 64);
        // 縦縞 (横方向の輪郭) は比率に関わらず同じ
        let src = edge(32, 8);
        let a0 = a.process_rgb(&src, &InputParams { sharpness_vertical: 0.0, ..base.clone() }).data.clone();
        let a1 = a.process_rgb(&src, &InputParams { sharpness_vertical: 1.0, ..base }).data.clone();
        assert_eq!(a0, a1);
    }

    #[test]
    fn saturation_zero_is_gray_and_keeps_luma() {
        let mut src = RgbFrame::new(4, 4);
        for px in src.data.chunks_exact_mut(3) {
            px.copy_from_slice(&[200, 80, 40]);
        }
        let mut a = InputAdjust::default();
        let out = a.process_rgb(&src, &InputParams { saturation: 0.0, ..on() });
        let px = &out.data[..3];
        assert!((px[0] as i32 - px[1] as i32).abs() <= 1 && (px[1] as i32 - px[2] as i32).abs() <= 1);
        let y = KR * 200.0 + KG * 80.0 + KB * 40.0;
        assert!((px[0] as f32 - y).abs() < 1.5);
    }

    #[test]
    fn warm_white_balance_shifts_red_up_blue_down() {
        let mut src = RgbFrame::new(4, 4);
        src.data.fill(128);
        let mut a = InputAdjust::default();
        let out = a.process_rgb(&src, &InputParams { color_temp: 0.5, ..on() });
        assert!(out.data[0] > 128 && out.data[2] < 128);
    }

    #[test]
    fn linear_keeps_highlights() {
        let mut src = LinearFrame::new(4, 4);
        src.data.fill(4.0);
        let mut a = InputAdjust::default();
        let out = a.process_linear(&src, &on());
        assert!((out.data[0] - 4.0).abs() < 1e-3);
    }
}
