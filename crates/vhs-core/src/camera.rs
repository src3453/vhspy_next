//! 撮像管カメラのシミュレーション (入力画像 → カメラ出力の RGB)。
//!
//! 入力画像を被写体の光とみなし、RGB 3 管式カラーカメラの信号経路をたどる
//! (川崎・和久井「撮像管とカメラ技術 7-1 概論」テレビ誌 28-11 (1974) の図 1 の系統に沿う):
//!
//! ```text
//! 光 ─ 色分解プリズム (混色・フレア) ─ レジストレーション/シェーディング/ぼけ
//!    ─ 光導電ターゲット (γ・暗電流・光導電性残像・焼き付き・しみ)
//!    ─ 電子ビーム読み出し (容量性残像・ビーム電流制限 → コメットテール・ACT)
//!    ─ ヒステリシス ─ 真空管プリアンプ (正負非対称な非線形・三角雑音)
//!    ─ オートブラック ─ 輪郭補償 ─ ニー/ホワイトクリップ ─ ガンマ補正 ─▶ RGB
//! ```
//!
//! ## ターゲットと読み出しのモデル
//! ターゲットの各画素は電荷 `q` (白レベル 1 フィールド分の光 = 1) を蓄える。
//! 光は毎フィールド全ラインに当たるが、インターレース走査なので各ラインが
//! ビームで読まれるのは 1 フレーム (2 フィールド) に 1 回。定常状態の白は 1 回の読み出しで 2。
//!
//! - 容量性残像: ビームの電子速度分布のため、ターゲット電位が低い (電荷が少ない) ほど
//!   1 回で読み切れない。読み出し量を `q²/(q+K)` とし、低照度ほど読み残し (残像) が増える。
//!   バイアス光で底上げすると改善する。
//! - ビーム電流制限: 1 回の読み出しは `beam` 以下。強い光が当たった部分は読み切れず、
//!   被写体が動いた後も数フレームにわたって白く残る (コメットテール)。
//!   ACT は帰線期間に一定以上の電荷を放電して尾を短くする。
//! - 光導電性残像: トラップの捕獲・放出による遅い応答 (一次遅れ、立ち上がりと減衰で時定数が異なる)。
//! - 焼き付き: 明るい光が当たり続けた部分の感度が徐々に落ち、ゆっくり回復する。
//! - ヒステリシス: 画素ごとの遊び (バックラッシュ) 演算子。明→暗と暗→明で残る値が変わり、時間とともに解ける。
//!
//! 3 本の撮像管はシードから決まる個体差 (γ・感度・残像・暗電流・ビーム電流・焼き付きやすさ・
//! レジストレーション・シェーディングの傾き) を持つ。`aging` はそれらをまとめて悪化させる。

use crate::frame::RgbFrame;
use crate::params::CameraParams;
use crate::rng::Rng;
use rayon::prelude::*;

const FIELD_RATE: f32 = 59.94;
/// 真空管プリアンプの動作点 (白レベル比)
const AMP_BIAS: f32 = 0.35;

/// 撮像管 1 本ぶんの個体差
#[derive(Clone, Copy, Debug)]
struct Tube {
    gamma: f32,
    sens: f32,
    cap_lag: f32,
    photo_lag: f32,
    dark: f32,
    beam: f32,
    burn: f32,
    /// オートブラックで取り切れない黒レベルのずれ
    black: f32,
    dx: f32,
    dy: f32,
    scale: f32,
    shade_x: f32,
    shade_y: f32,
}

impl Tube {
    fn generate(p: &CameraParams, seed: u64) -> [Tube; 3] {
        let a = p.aging.clamp(0.0, 1.0);
        let m = p.mismatch.max(0.0) + 0.5 * a;
        let mut rng = Rng::new(seed ^ 0xCA3E_7A11_5EED_0001);
        std::array::from_fn(|c| {
            let mut n = || rng.normal();
            let gamma = (p.target_gamma * (1.0 + 0.06 * m * n())).clamp(0.2, 2.0);
            let sens = (1.0 + 0.08 * m * n()).max(0.3) * (1.0 - 0.4 * a);
            let cap_lag = (0.4 * m * n()).exp() * (1.0 + 3.0 * a);
            let photo_lag = (0.4 * m * n()).exp();
            let dark = (1.0 + 0.5 * m * n()).max(0.0) * (1.0 + 5.0 * a);
            let beam = (1.0 + 0.15 * m * n()).max(0.2) * (1.0 - 0.6 * a);
            let burn = (0.5 * m * n()).exp() * (1.0 + 4.0 * a);
            let black = 0.01 * m * n();
            let scale = 1.0 + 0.002 * p.registration_px * n();
            let (shade_x, shade_y) = (0.5 * m * n(), 0.5 * m * n());
            // G 管を基準にして R/B 管がずれる
            let angle = rng.uniform() * std::f32::consts::TAU;
            let r = p.registration_px * (0.5 + 0.5 * rng.uniform()) * if c == 1 { 0.3 } else { 1.0 };
            Tube {
                gamma,
                sens,
                cap_lag,
                photo_lag,
                dark,
                beam,
                burn,
                black,
                dx: r * angle.cos(),
                dy: r * angle.sin(),
                scale,
                shade_x,
                shade_y,
            }
        })
    }
}

/// 感度・暗電流マップを作り直すかどうかの判定に使う値
#[derive(Clone, PartialEq)]
struct MapKey {
    seed: u64,
    w: usize,
    h: usize,
    vals: [u32; 8],
    blemishes: u32,
}

impl MapKey {
    fn new(p: &CameraParams, seed: u64, w: usize, h: usize) -> Self {
        let v = [
            p.shading,
            p.aging,
            p.mismatch,
            p.dark_current,
            p.target_gamma,
            p.registration_px,
            0.0,
            0.0,
        ];
        Self { seed, w, h, vals: v.map(f32::to_bits), blemishes: p.blemishes }
    }
}

#[derive(Default)]
pub struct CameraSim {
    w: usize,
    h: usize,
    lin: [Vec<f32>; 3],
    flux: [Vec<f32>; 3],
    tmp: Vec<f32>,
    /// ターゲットの蓄積電荷
    q: [Vec<f32>; 3],
    /// 光導電性残像のトラップ状態
    trap: [Vec<f32>; 3],
    /// ヒステリシスの状態
    hyst: [Vec<f32>; 3],
    /// 焼き付き (感度低下率)
    burn: [Vec<f32>; 3],
    /// 読み出した信号 (白レベル比、オートブラック後)
    sig: [Vec<f32>; 3],
    sens: [Vec<f32>; 3],
    dark: [Vec<f32>; 3],
    tubes: Option<[Tube; 3]>,
    key: Option<MapKey>,
    last_field: Option<u64>,
    out: Option<RgbFrame>,
}

impl CameraSim {
    /// 直前の出力
    pub fn output(&self) -> Option<&RgbFrame> {
        self.out.as_ref()
    }

    fn allocate(&mut self, w: usize, h: usize) {
        let n = w * h;
        self.w = w;
        self.h = h;
        for v in self
            .lin
            .iter_mut()
            .chain(&mut self.flux)
            .chain(&mut self.q)
            .chain(&mut self.trap)
            .chain(&mut self.hyst)
            .chain(&mut self.burn)
            .chain(&mut self.sig)
            .chain(&mut self.sens)
            .chain(&mut self.dark)
        {
            *v = vec![0.0; n];
        }
        self.tmp = vec![0.0; n];
        self.out = Some(RgbFrame::new(w, h));
        self.key = None;
        self.last_field = None;
    }

    /// 感度 (シェーディング・しみ) と暗電流 (傾き・白きず) のマップ
    fn build_maps(&mut self, p: &CameraParams, seed: u64, tubes: &[Tube; 3]) {
        let (w, h) = (self.w, self.h);
        let a = p.aging.clamp(0.0, 1.0);
        let shading = p.shading.max(0.0) + 0.2 * a;
        let dark_level = p.dark_current.max(0.0);
        for c in 0..3 {
            let t = tubes[c];
            self.sens[c].par_chunks_mut(w).zip(self.dark[c].par_chunks_mut(w)).enumerate().for_each(|(y, (sr, dr))| {
                let ny = (y as f32 + 0.5) / h as f32 * 2.0 - 1.0;
                for x in 0..w {
                    let nx = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
                    let r2 = 0.5 * (nx * nx + ny * ny);
                    let tilt = 1.0 + shading * (t.shade_x * nx + t.shade_y * ny);
                    sr[x] = (t.sens * (1.0 - shading * r2) * tilt).max(0.0);
                    // 暗電流は管の片側が多くなりがち (温度・ターゲット厚のむら)
                    dr[x] = dark_level * t.dark * (1.0 + 0.6 * nx * t.shade_x.signum() + 0.4 * r2).max(0.0);
                }
            });
        }

        // ターゲットのしみ (感度が落ちた黒い点) と白きず (局所的な暗電流)
        let count = p.blemishes as usize + (40.0 * a) as usize;
        let mut rng = Rng::new(seed ^ 0xB1E3_15E5_0000_0002);
        for _ in 0..count {
            let c = (rng.uniform() * 3.0) as usize % 3;
            let cx = rng.uniform() * w as f32;
            let cy = rng.uniform() * h as f32;
            let radius = (0.8 + rng.exponential(2.0)).min(12.0);
            let white = rng.uniform() < 0.2;
            let strength = 0.2 + 0.7 * rng.uniform();
            let reach = (radius * 2.5).ceil() as isize;
            for yy in (cy as isize - reach).max(0)..(cy as isize + reach).min(h as isize) {
                for xx in (cx as isize - reach).max(0)..(cx as isize + reach).min(w as isize) {
                    let dx = (xx as f32 + 0.5 - cx) / radius;
                    let dy = (yy as f32 + 0.5 - cy) / radius;
                    let g = (-(dx * dx + dy * dy)).exp();
                    let i = yy as usize * w + xx as usize;
                    if white {
                        self.dark[c][i] += 0.25 * strength * g;
                    } else {
                        self.sens[c][i] *= 1.0 - strength * g;
                    }
                }
            }
        }
    }

    /// 1 フィールドぶん処理する。返す画像のうち、このフィールドの走査線 (`field_index` の偶奇) が更新される。
    pub fn process(&mut self, input: &RgbFrame, p: &CameraParams, seed: u64, field_index: u64) -> &RgbFrame {
        let (w, h) = (input.width, input.height);
        if self.w != w || self.h != h || self.out.is_none() {
            self.allocate(w, h);
        }
        let key = MapKey::new(p, seed, w, h);
        if self.key.as_ref() != Some(&key) || self.tubes.is_none() {
            let tubes = Tube::generate(p, seed);
            self.build_maps(p, seed, &tubes);
            self.tubes = Some(tubes);
            self.key = Some(key);
        }
        let tubes = self.tubes.unwrap();
        // フィールドが連続していなければ (初回・無効化からの復帰) 定常状態から始める
        let prime = self.last_field.is_none_or(|f| f + 1 != field_index);
        self.last_field = Some(field_index);
        let parity = (field_index & 1) as usize;
        let a = p.aging.clamp(0.0, 1.0);

        self.optics(input, p, &tubes);
        for c in 0..3 {
            self.target(c, p, &tubes[c], parity, prime, a);
        }
        self.process_amp(p, seed, field_index, parity, a);
        self.out.as_ref().unwrap()
    }

    /// 光学系: 線形化・露出・混色・フレア・レジストレーション・ぼけ
    fn optics(&mut self, input: &RgbFrame, p: &CameraParams, tubes: &[Tube; 3]) {
        let (w, h) = (self.w, self.h);
        let gain = 2f32.powf(p.exposure_ev);
        let lut: [f32; 256] = std::array::from_fn(|i| (i as f32 / 255.0).powf(2.2) * gain);
        let x = p.crosstalk.clamp(0.0, 0.33);
        {
            let [l0, l1, l2] = &mut self.lin;
            l0.par_chunks_mut(w)
                .zip(l1.par_chunks_mut(w))
                .zip(l2.par_chunks_mut(w))
                .zip(input.data.par_chunks(w * 3))
                .for_each(|(((r0, r1), r2), src)| {
                    for i in 0..w {
                        let (r, g, b) = (lut[src[i * 3] as usize], lut[src[i * 3 + 1] as usize], lut[src[i * 3 + 2] as usize]);
                        // ダイクロイックプリズムの分光特性の裾で隣の色が漏れ込む
                        r0[i] = r * (1.0 - x) + g * x;
                        r1[i] = g * (1.0 - 2.0 * x) + (r + b) * x;
                        r2[i] = b * (1.0 - x) + g * x;
                    }
                });
        }

        let sigma = p.resolution_px.max(0.0) + 0.8 * p.aging.clamp(0.0, 1.0);
        for c in 0..3 {
            let t = tubes[c];
            let lin = &self.lin[c];
            // フレア: 画面全体の光がかぶる (赤管は特に多い)
            let mean = lin.par_iter().sum::<f32>() / lin.len() as f32;
            let flare = p.flare.max(0.0) * if c == 0 { 2.0 } else { 1.0 } * mean;
            let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
            let shifted = t.dx.abs() > 1e-3 || t.dy.abs() > 1e-3 || (t.scale - 1.0).abs() > 1e-6;
            self.flux[c].par_chunks_mut(w).enumerate().for_each(|(y, row)| {
                if !shifted {
                    for (o, v) in row.iter_mut().zip(&lin[y * w..(y + 1) * w]) {
                        *o = v + flare;
                    }
                    return;
                }
                let sy = (cy + (y as f32 + 0.5 - cy) * t.scale - t.dy - 0.5).clamp(0.0, (h - 1) as f32);
                let y0 = sy as usize;
                let y1 = (y0 + 1).min(h - 1);
                let fy = sy - y0 as f32;
                for (x, o) in row.iter_mut().enumerate() {
                    let sx = (cx + (x as f32 + 0.5 - cx) * t.scale - t.dx - 0.5).clamp(0.0, (w - 1) as f32);
                    let x0 = sx as usize;
                    let x1 = (x0 + 1).min(w - 1);
                    let fx = sx - x0 as f32;
                    let top = lin[y0 * w + x0] + (lin[y0 * w + x1] - lin[y0 * w + x0]) * fx;
                    let bot = lin[y1 * w + x0] + (lin[y1 * w + x1] - lin[y1 * w + x0]) * fx;
                    *o = top + (bot - top) * fy + flare;
                }
            });
            blur_plane(&mut self.flux[c], &mut self.tmp, w, h, sigma);
        }
    }

    /// 光導電ターゲットへの蓄積と、このフィールドの走査線の読み出し
    fn target(&mut self, c: usize, p: &CameraParams, t: &Tube, parity: usize, prime: bool, a: f32) {
        let w = self.w;
        let gamma = t.gamma;
        let plag = (p.photo_lag.max(0.0) * t.photo_lag + 0.3 * a).min(0.95);
        let tau = p.photo_lag_fields.max(0.1);
        let k_decay = 1.0 - (-1.0 / tau).exp();
        let k_rise = 1.0 - (-2.0 / tau).exp();
        let kc = p.capacitive_lag.max(0.0) * t.cap_lag;
        // 読み出しは 1 フレーム (2 フィールド) に 1 回なので電荷の単位は 2 倍
        let beam = (p.beam_current.max(0.05) * t.beam * 2.0).max(0.05);
        let qsat = (p.target_saturation.max(p.beam_current) * 2.0).max(beam);
        let act = p.act.then_some(p.act_level.max(0.1) * 2.0);
        let bias = p.bias_light.max(0.0);
        // オートブラックが差し引く黒レベル (暗電流の平均とバイアス光)。むら・個体差は残る
        let black = p.dark_current.max(0.0) * (1.0 + 5.0 * a) + bias - t.black;
        let burn_rate = p.burn_rate.max(0.0) * t.burn / FIELD_RATE;
        let burn_rec = p.burn_recovery.max(0.0) / FIELD_RATE;
        let hw = p.hysteresis.max(0.0);
        let h_relax = 1.0 - (-1.0 / p.hysteresis_fields.max(0.1)).exp();
        let amp = Amp::new(p.amp_drive, p.amp_asymmetry);

        let flux = &self.flux[c];
        let sens = &self.sens[c];
        let dark = &self.dark[c];
        self.q[c]
            .par_chunks_mut(w)
            .zip(self.trap[c].par_chunks_mut(w))
            .zip(self.hyst[c].par_chunks_mut(w))
            .zip(self.burn[c].par_chunks_mut(w))
            .zip(self.sig[c].par_chunks_mut(w))
            .enumerate()
            .for_each(|(y, ((((qr, tr), hr), br), sr))| {
                let read_line = y & 1 == parity;
                let base = y * w;
                for x in 0..w {
                    let f = flux[base + x].max(0.0);
                    let i = sens[base + x] * (1.0 - br[x]) * f.powf(gamma) + dark[base + x] + bias;
                    if prime {
                        tr[x] = i;
                        br[x] = 0.0;
                        // 定常状態: 読み出し直前の電荷 Q は Q²/(Q+K) = 2i を満たす
                        let qeq = i + (i * i + 2.0 * i * kc).sqrt();
                        let residual = (qeq - 2.0 * i).max(0.0);
                        qr[x] = residual + if read_line { i } else { 0.0 };
                    }
                    // 光導電性残像 (トラップ)
                    let trap = tr[x];
                    let k = if i > trap { k_rise } else { k_decay };
                    let trap = trap + (i - trap) * k;
                    tr[x] = trap;
                    let ie = i * (1.0 - plag) + trap * plag;

                    // 焼き付き: 強い光で感度が落ち、ゆっくり戻る
                    let b = br[x] + burn_rate * (f - 0.6).max(0.0) - burn_rec * br[x];
                    br[x] = b.clamp(0.0, 0.95);

                    let mut q = (qr[x] + ie).min(qsat);
                    if read_line {
                        let rd = if kc > 0.0 { q * q / (q + kc) } else { q }.min(beam);
                        q -= rd;
                        if let Some(l) = act {
                            q = q.min(l);
                        }
                        let s = rd * 0.5 - black;
                        // ヒステリシス (遊び演算子 + 緩和)
                        let hv = if prime { s } else { hr[x].clamp(s - hw, s + hw) };
                        let hv = hv + (s - hv) * h_relax;
                        hr[x] = hv;
                        // 真空管プリアンプ (画素ごとの無記憶な非線形なのでここで掛けておく)
                        sr[x] = amp.apply(if hw > 0.0 { hv } else { s });
                    }
                    qr[x] = q;
                }
            });
    }

    /// プリアンプ (非線形・雑音) とプロセス回路 (輪郭補償・ニー・ガンマ)。このフィールドの走査線だけ書く
    fn process_amp(&mut self, p: &CameraParams, seed: u64, field_index: u64, parity: usize, a: f32) {
        let (w, h) = (self.w, self.h);
        // 劣化で落ちた感度はカメラの利得で持ち上げる (雑音も一緒に増える)
        let gain = 1.0 / (1.0 - 0.4 * a);
        let noise = p.noise_pct.max(0.0) * 0.01 * (1.0 + 2.0 * a);
        let ap = p.aperture;
        let (knee, slope, clip) = (p.knee, p.knee_slope.max(0.0), p.white_clip);
        let cg = p.camera_gamma.max(0.05);
        // ガンマ補正の表 (0..clip)
        const N: usize = 4096;
        let top = clip.max(1e-3);
        let lut: Vec<u8> =
            (0..=N).map(|i| ((i as f32 / N as f32 * top).powf(cg) * 255.0 + 0.5).clamp(0.0, 255.0) as u8).collect();
        let lut = &lut;
        let sig = &self.sig;
        let out = self.out.as_mut().unwrap();
        out.data.par_chunks_mut(w * 3).enumerate().filter(|(y, _)| y & 1 == parity).for_each(|(y, row)| {
            // 同じフィールドの上下のライン (1H 遅延線による垂直輪郭補償)
            let yu = if y >= 2 { y - 2 } else { y };
            let yd = if y + 2 < h { y + 2 } else { y };
            for c in 0..3 {
                let s = &sig[c];
                let (rc, ru, rd) = (&s[y * w..][..w], &s[yu * w..][..w], &s[yd * w..][..w]);
                let mut rng = Rng::derive(seed, field_index, y as u64, 0xCA_0000 + c as u64);
                let mut prev = 0.0;
                for x in 0..w {
                    let xl = x.saturating_sub(1);
                    let xr = (x + 1).min(w - 1);
                    let v0 = rc[x];
                    let around = 0.25 * (rc[xl] + rc[xr] + ru[x] + rd[x]);
                    let mut v = v0 + ap * (v0 - around);
                    // 前置増幅器の雑音: 入力容量のため高域ほど大きい (三角雑音)
                    if noise > 0.0 {
                        let n = rng.normal();
                        v += noise * (0.35 * n + 0.75 * (n - prev));
                        prev = n;
                    }
                    let mut v = v * gain;
                    if v > knee {
                        v = knee + (v - knee) * slope;
                    }
                    let v = v.min(clip).max(0.0);
                    row[x * 3 + c] = lut[((v / top * N as f32 + 0.5) as usize).min(N)];
                }
            }
        });
    }
}

/// 真空管プリアンプの正負非対称な飽和特性。黒 (0) と白 (1) が動かないよう正規化してある
#[derive(Clone, Copy)]
struct Amp {
    cp: f32,
    cn: f32,
    f0: f32,
    scale: f32,
}

impl Amp {
    fn new(drive: f32, asym: f32) -> Self {
        let drive = drive.max(0.0);
        let asym = asym.clamp(-1.0, 1.0);
        let mut a = Self { cp: drive * (1.0 + asym), cn: drive * (1.0 - asym), f0: 0.0, scale: 1.0 };
        let f0 = a.raw(0.0);
        let f1 = a.raw(1.0);
        a.f0 = f0;
        a.scale = if (f1 - f0).abs() > 1e-6 { 1.0 / (f1 - f0) } else { 1.0 };
        a
    }

    #[inline]
    fn raw(&self, s: f32) -> f32 {
        let u = s - AMP_BIAS;
        let c = if u > 0.0 { self.cp } else { self.cn };
        if c < 1e-4 { u } else { (c * u).tanh() / c }
    }

    #[inline]
    fn apply(&self, s: f32) -> f32 {
        (self.raw(s) - self.f0) * self.scale
    }
}

/// 分離型ガウスぼかし
fn blur_plane(plane: &mut [f32], tmp: &mut [f32], w: usize, h: usize, sigma: f32) {
    if sigma < 0.05 {
        return;
    }
    let r = (sigma * 3.0).ceil() as isize;
    let mut k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = k.iter().sum();
    k.iter_mut().for_each(|v| *v /= sum);
    let k = &k;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, v: u8) -> RgbFrame {
        RgbFrame { width: w, height: h, data: vec![v; w * h * 3] }
    }

    fn quiet() -> CameraParams {
        CameraParams {
            enabled: true,
            noise_pct: 0.0,
            mismatch: 0.0,
            registration_px: 0.0,
            shading: 0.0,
            flare: 0.0,
            crosstalk: 0.0,
            dark_current: 0.0,
            amp_drive: 0.0,
            aperture: 0.0,
            burn_rate: 0.0,
            target_gamma: 1.0,
            camera_gamma: 1.0 / 2.2,
            ..Default::default()
        }
    }

    /// 静止画は定常状態でほぼそのまま通る
    #[test]
    fn steady_state_is_transparent() {
        let mut cam = CameraSim::default();
        let p = quiet();
        let input = flat(64, 48, 128);
        for f in 0..4 {
            cam.process(&input, &p, 1, f);
        }
        let out = cam.output().unwrap();
        for &v in &out.data[64 * 3 * 10..64 * 3 * 12] {
            assert!((v as i32 - 128).abs() <= 3, "got {v}");
        }
    }

    /// 消灯後も残像が残り、低照度のほうが残りやすい
    #[test]
    fn lag_is_larger_at_low_light() {
        let residual = |level: u8| {
            let mut cam = CameraSim::default();
            let mut p = quiet();
            p.capacitive_lag = 0.1;
            p.photo_lag = 0.0;
            p.camera_gamma = 1.0;
            let on = flat(32, 8, level);
            let off = flat(32, 8, 0);
            for f in 0..20 {
                cam.process(&on, &p, 1, f);
            }
            let lit = cam.output().unwrap().data[16 * 3] as f32;
            // 消灯直後のフレームは前フィールドの光も読むので、その次のフレーム (行 0 = 偶数フィールド) を見る
            for f in 20..23 {
                cam.process(&off, &p, 1, f);
            }
            cam.output().unwrap().data[16 * 3] as f32 / lit.max(1.0)
        };
        let hi = residual(255);
        let lo = residual(90);
        assert!(hi > 0.0 && lo > hi, "hi {hi} lo {lo}");
    }

    /// ビーム電流を超える光は読み残されて尾を引き、ACT で短くなる
    #[test]
    fn comet_tail_and_act() {
        let tail = |act: bool| {
            let mut cam = CameraSim::default();
            let mut p = quiet();
            p.exposure_ev = 3.0;
            p.beam_current = 1.5;
            p.act = act;
            p.camera_gamma = 1.0;
            p.knee = 10.0;
            p.white_clip = 10.0;
            let on = flat(16, 8, 255);
            let off = flat(16, 8, 0);
            for f in 0..10 {
                cam.process(&on, &p, 1, f);
            }
            let mut n = 0;
            for f in 10..40 {
                cam.process(&off, &p, 1, f);
                if f & 1 == 0 && cam.output().unwrap().data[0] > 20 {
                    n += 1;
                }
            }
            n
        };
        let plain = tail(false);
        let with_act = tail(true);
        assert!(plain >= 2 && with_act < plain, "plain {plain} act {with_act}");
    }

    #[test]
    fn amp_keeps_black_and_white() {
        let a = Amp::new(1.5, 0.6);
        assert!(a.apply(0.0).abs() < 1e-5);
        assert!((a.apply(1.0) - 1.0).abs() < 1e-5);
        // 非対称: 中間調が直線からずれる
        assert!((a.apply(0.5) - 0.5).abs() > 0.01);
    }
}
