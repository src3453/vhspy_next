//! 静止画入力。画像を読み込み、配置 (引き伸ばし・覆う・収める)・拡大・パンを掛けて 1 フィールドぶんの入力を作る。
//! OpenEXR などの浮動小数点画像はシーンリニアな HDR としてそのまま撮像管カメラへ渡す。

use image::DynamicImage;
use std::path::Path;
use vhs_core::params::{ImageFit, ImageParams};
use vhs_core::{LinearFrame, RgbFrame, Standard};

/// 1 フィールドぶんの入力
pub enum StillFrame<'a> {
    Rgb(&'a RgbFrame),
    Linear(&'a LinearFrame),
}

/// 画素の型 (8 bit / f32) をまとめて扱う
#[derive(Clone)]
enum Pixels {
    Rgb(RgbFrame),
    Linear(LinearFrame),
}

impl Pixels {
    fn size(&self) -> (usize, usize) {
        match self {
            Pixels::Rgb(f) => (f.width, f.height),
            Pixels::Linear(f) => (f.width, f.height),
        }
    }

    fn resized(&self, w: usize, h: usize) -> Pixels {
        match self {
            Pixels::Rgb(f) if (f.width, f.height) == (w, h) => self.clone(),
            Pixels::Rgb(f) => {
                let img = image::RgbImage::from_raw(f.width as u32, f.height as u32, f.data.clone()).unwrap();
                let img = image::imageops::resize(&img, w as u32, h as u32, image::imageops::FilterType::CatmullRom);
                Pixels::Rgb(RgbFrame::from_raw(w, h, img.into_raw()).unwrap())
            }
            // 光量を保つリサイズ (小さな光源が縮小で消えない)
            Pixels::Linear(f) => Pixels::Linear(f.resized(w, h)),
        }
    }
}

/// 拡大後の画像のキャッシュを作り直す条件
#[derive(Clone, Copy, PartialEq)]
struct ScaleKey {
    fit: ImageFit,
    zoom: f32,
    frame: (usize, usize),
}

pub struct Still {
    original: Pixels,
    scaled: Option<(ScaleKey, Pixels)>,
    /// 今の表示位置 (-1..1) と動く向き
    pos: [f32; 2],
    dir: [f32; 2],
    last_pan: [f32; 2],
    out_rgb: RgbFrame,
    out_linear: LinearFrame,
}

/// 画像の拡大率の上限 (拡大後の画像をメモリに持つため)
const MAX_ZOOM: f32 = 4.0;

impl Still {
    pub fn load(path: &Path) -> Result<Self, String> {
        let img = image::open(path).map_err(|e| format!("画像を開けません: {e}"))?;
        let (w, h) = (img.width() as usize, img.height() as usize);
        let original = match img {
            // 浮動小数点の画像 (OpenEXR) はシーンリニアとみなす。アルファは乗算済みなので捨てれば黒背景になる
            DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_) => {
                Pixels::Linear(LinearFrame::from_raw(w, h, img.to_rgb32f().into_raw()).unwrap())
            }
            _ => Pixels::Rgb(RgbFrame::from_raw(w, h, img.to_rgb8().into_raw()).unwrap()),
        };
        Ok(Self {
            original,
            scaled: None,
            pos: [0.0; 2],
            dir: [1.0; 2],
            last_pan: [f32::NAN; 2],
            out_rgb: RgbFrame::new(0, 0),
            out_linear: LinearFrame::new(0, 0),
        })
    }

    /// 画面上での画像の大きさ [画素] (画面の画素は正方でない)
    fn scaled_size(&self, p: &ImageParams, tv: &Standard) -> (usize, usize) {
        let (fw, fh) = tv.image_size();
        let (w, h) = self.original.size();
        let sa = tv.sample_aspect();
        let zoom = p.zoom.clamp(1.0, MAX_ZOOM);
        // 正方画素での画面の幅
        let fw_sq = fw as f32 / sa;
        let (kx, ky) = match p.fit {
            ImageFit::Stretch => (fw_sq / w as f32, fh as f32 / h as f32),
            ImageFit::Cover => {
                let k = (fw_sq / w as f32).max(fh as f32 / h as f32);
                (k, k)
            }
            ImageFit::Contain => {
                let k = (fw_sq / w as f32).min(fh as f32 / h as f32);
                (k, k)
            }
        };
        let sw = (w as f32 * kx * sa * zoom).round().max(1.0) as usize;
        let sh = (h as f32 * ky * zoom).round().max(1.0) as usize;
        (sw, sh)
    }

    /// 1 フィールドぶんの入力を作る。`dt` はフィールドの時間 [s] (パンを進める)
    pub fn render(&mut self, p: &ImageParams, tv: &Standard, dt: f32) -> StillFrame<'_> {
        let frame = tv.image_size();
        let key = ScaleKey { fit: p.fit, zoom: p.zoom.clamp(1.0, MAX_ZOOM), frame };
        if self.scaled.as_ref().is_none_or(|(k, _)| *k != key) {
            let (sw, sh) = self.scaled_size(p, tv);
            self.scaled = Some((key, self.original.resized(sw, sh)));
        }
        let scaled = &self.scaled.as_ref().unwrap().1;
        let (sw, sh) = scaled.size();

        // ---- パン ----
        let pan = [p.pan_x.clamp(-1.0, 1.0), p.pan_y.clamp(-1.0, 1.0)];
        let speed = [p.speed_x, p.speed_y];
        let fsize = [frame.0 as f32, frame.1 as f32];
        let ssize = [sw as f32, sh as f32];
        let mut origin = [0f32; 2];
        for i in 0..2 {
            // 位置を動かしたとき・止めているときは指定の位置へ
            if pan[i] != self.last_pan[i] || speed[i] == 0.0 {
                self.pos[i] = pan[i];
                self.last_pan[i] = pan[i];
            } else {
                let half = (fsize[i] - ssize[i]).abs() / 2.0;
                if half >= 0.5 {
                    self.pos[i] += self.dir[i] * speed[i] * fsize[i] * dt / half;
                    // 端で折り返す
                    if self.pos[i] > 1.0 {
                        self.pos[i] = (2.0 - self.pos[i]).max(-1.0);
                        self.dir[i] = -self.dir[i];
                    } else if self.pos[i] < -1.0 {
                        self.pos[i] = (-2.0 - self.pos[i]).min(1.0);
                        self.dir[i] = -self.dir[i];
                    }
                }
            }
            // 画像の左上の画面上の位置。画像が大きければ pos=1 で右端が見え、小さければ画像が右に寄る
            origin[i] = (fsize[i] - ssize[i]) * (1.0 + self.pos[i]) / 2.0;
        }

        match scaled {
            Pixels::Rgb(src) => {
                if (self.out_rgb.width, self.out_rgb.height) != frame {
                    self.out_rgb = RgbFrame::new(frame.0, frame.1);
                }
                place(&src.data, (sw, sh), &mut self.out_rgb.data, frame, origin, 1.0);
                StillFrame::Rgb(&self.out_rgb)
            }
            Pixels::Linear(src) => {
                if (self.out_linear.width, self.out_linear.height) != frame {
                    self.out_linear = LinearFrame::new(frame.0, frame.1);
                }
                let gain = 2f32.powf(p.exposure_ev);
                place(&src.data, (sw, sh), &mut self.out_linear.data, frame, origin, gain);
                StillFrame::Linear(&self.out_linear)
            }
        }
    }
}

trait Sample: Copy {
    const ZERO: Self;
    fn to_f(self) -> f32;
    fn from_f(v: f32) -> Self;
}

impl Sample for u8 {
    const ZERO: Self = 0;
    fn to_f(self) -> f32 {
        self as f32
    }
    fn from_f(v: f32) -> Self {
        (v + 0.5).clamp(0.0, 255.0) as u8
    }
}

impl Sample for f32 {
    const ZERO: Self = 0.0;
    fn to_f(self) -> f32 {
        self
    }
    fn from_f(v: f32) -> Self {
        v
    }
}

/// `src` (RGB) を左上が `origin` に来るように `dst` へ置く (はみ出しは黒、端数はバイリニア補間)。`gain` を掛ける
fn place<T: Sample>(src: &[T], (sw, sh): (usize, usize), dst: &mut [T], (dw, dh): (usize, usize), origin: [f32; 2], gain: f32) {
    let (ox, oy) = (origin[0], origin[1]);
    // 整数位置で倍率 1 なら行ごとにコピーする (止めている引き伸ばし表示は元と同じ画素になる)
    if ox.fract() == 0.0 && oy.fract() == 0.0 && gain == 1.0 {
        let (ox, oy) = (ox as isize, oy as isize);
        for y in 0..dh {
            let row = &mut dst[y * dw * 3..(y + 1) * dw * 3];
            let sy = y as isize - oy;
            if sy < 0 || sy >= sh as isize {
                row.fill(T::ZERO);
                continue;
            }
            for x in 0..dw {
                let sx = x as isize - ox;
                let px = &mut row[x * 3..x * 3 + 3];
                if sx < 0 || sx >= sw as isize {
                    px.fill(T::ZERO);
                } else {
                    px.copy_from_slice(&src[(sy as usize * sw + sx as usize) * 3..][..3]);
                }
            }
        }
        return;
    }
    // 画像の外は黒として補間する
    let get = |x: isize, y: isize, c: usize| -> f32 {
        if x < 0 || y < 0 || x >= sw as isize || y >= sh as isize {
            0.0
        } else {
            src[(y as usize * sw + x as usize) * 3 + c].to_f()
        }
    };
    for y in 0..dh {
        let fy = y as f32 - oy;
        let y0 = fy.floor();
        let wy = fy - y0;
        let y0 = y0 as isize;
        for x in 0..dw {
            let fx = x as f32 - ox;
            let x0 = fx.floor();
            let wx = fx - x0;
            let x0 = x0 as isize;
            for c in 0..3 {
                let top = get(x0, y0, c) * (1.0 - wx) + get(x0 + 1, y0, c) * wx;
                let bot = get(x0, y0 + 1, c) * (1.0 - wx) + get(x0 + 1, y0 + 1, c) * wx;
                dst[(y * dw + x) * 3 + c] = T::from_f((top * (1.0 - wy) + bot * wy) * gain);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_integer_and_fractional() {
        let src: Vec<u8> = (0..2 * 2 * 3).map(|i| i as u8 * 10).collect();
        let mut dst = vec![0u8; 3 * 2 * 3];
        place(&src, (2, 2), &mut dst, (3, 2), [1.0, 0.0], 1.0);
        assert_eq!(&dst[0..3], &[0, 0, 0]);
        assert_eq!(&dst[3..9], &src[0..6]);
        place(&src, (2, 2), &mut dst, (3, 2), [0.5, 0.0], 1.0);
        // 左端は黒と最初の画素の中間
        assert_eq!(dst[0], 0);
        assert_eq!(dst[3], ((0.0 + 30.0) / 2.0 + 0.5) as u8);
    }

    #[test]
    fn exr_loads_as_hdr_and_pans() {
        // 横長 (2:1) の HDR 画像: 左半分 0.25、右半分 4.0 (白を超える)
        let (w, h) = (200u32, 100u32);
        let img = image::Rgb32FImage::from_fn(w, h, |x, _| {
            let v = if x < w / 2 { 0.25 } else { 4.0 };
            image::Rgb([v, v, v])
        });
        let path = std::env::temp_dir().join(format!("vhs_still_test_{}.exr", std::process::id()));
        DynamicImage::ImageRgb32F(img).save(&path).unwrap();
        let mut still = Still::load(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let tv = vhs_core::TvStandard::Ntsc.spec();
        let (fw, fh) = tv.image_size();
        let mut p = ImageParams { fit: ImageFit::Cover, pan_x: -1.0, ..Default::default() };
        let mid = |f: &LinearFrame| f.data[(fh / 2 * fw + fw / 2) * 3];
        // 左端に寄せると暗い側だけ、右端に寄せると明るい側 (1.0 超えのまま) が見える
        let StillFrame::Linear(f) = still.render(&p, tv, 1.0 / 60.0) else { panic!("HDR のはず") };
        assert_eq!((f.width, f.height), (fw, fh));
        assert!((mid(f) - 0.25).abs() < 1e-3);
        p.pan_x = 1.0;
        let StillFrame::Linear(f) = still.render(&p, tv, 1.0 / 60.0) else { unreachable!() };
        assert!((mid(f) - 4.0).abs() < 1e-3);
        // 露出
        p.exposure_ev = -2.0;
        let StillFrame::Linear(f) = still.render(&p, tv, 1.0 / 60.0) else { unreachable!() };
        assert!((mid(f) - 1.0).abs() < 1e-3);
        // パン: 右端から左向きに動き始め、端で折り返す
        p.speed_x = 1.0;
        let _ = still.render(&p, tv, 0.1);
        assert!(still.pos[0] < 1.0);
        for _ in 0..100 {
            let _ = still.render(&p, tv, 0.1);
            assert!((-1.0..=1.0).contains(&still.pos[0]));
        }
    }
}
