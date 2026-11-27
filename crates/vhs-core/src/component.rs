//! デッキの映像出力のうち、S 映像より上の接続。
//!
//! - コンポーネント (Y, Pb, Pr): 再生した Y/C (C は副搬送波に載った色) から、デッキの中の色復調器で
//!   色差を取り出してベースバンドのまま送る。復調器は再生バーストにラインごとに同期し (PAL はアイデントで
//!   PAL スイッチを決め)、色差のローパスを通す。受像機の Y/C 分離・色副搬送波の PLL は通らない
//! - ダイレクト (デジタル / HDMI): コンポーネントと同じ信号をデッキの中で標本化し、色差を間引いて
//!   ([`ChromaSampling`]) 送る。受像機側は RGB に戻すだけ

use crate::filter::Cascade;
use crate::frame::{Field, RgbaFrame};
use crate::params::ChromaSampling;
use crate::timing::*;
use rayon::prelude::*;

/// Pb = 0.564 (B-Y) = U · 0.564 / 0.493
pub const PB_PER_U: f32 = 0.564 / 0.493;
/// Pr = 0.713 (R-Y) = V · 0.713 / 0.877
pub const PR_PER_V: f32 = 0.713 / 0.877;

/// デッキの色復調後の色差ローパス [MHz] (コンポーネント出力の色差帯域)
const CHROMA_BW_MHZ: f32 = 1.5;
/// このバースト振幅 [IRE] より弱いラインは色を消す (デッキのカラーキラー)
const KILL_BURST_IRE: f32 = 3.0;

/// デッキの色復調器 (Y/C → Y, Pb, Pr)。PLL と PAL アイデントの状態をフィールドをまたいで持つ
#[derive(Default)]
pub struct ComponentOut {
    /// 次に来るはずの通算ライン (途切れたら PLL を引き込み直す)
    next_stream: Option<u64>,
    /// 直前のラインの副搬送波位相
    pll_prev: Option<f32>,
    /// PAL アイデント: 直前のラインのバースト位相 (通算ライン, μ) と投票
    prev_mu: Option<(u64, f32)>,
    pal_vote: f32,
    /// ラインごとの (基準位相, PAL スイッチ, 色を出すか)
    refs: Vec<(f32, f32, bool)>,
    pb: Vec<f32>,
    pr: Vec<f32>,
}

impl ComponentOut {
    /// S 映像 (`field.separate`) のフィールドをコンポーネントに変える。Y はそのまま
    pub fn process(&mut self, field: &mut Field) {
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, TAU};
        assert!(field.separate && !field.component, "Y/C 分離のフィールドを渡すこと");
        let s = field.std;
        let (ls, rl) = (s.line_samples, s.raster_lines);
        let pal = s.is_pal();
        let wrap = |a: f32| a - TAU * (a / TAU).round();
        let step = s.line_phase_step() as f32 * FRAC_PI_2;
        let start = field.stream_line(0);
        if self.next_stream != Some(start) {
            self.pll_prev = None;
            self.prev_mu = None;
        }
        self.next_stream = Some(start + rl as u64);

        // --- バーストの位相・振幅 (ライン並列) ---
        let gate = s.burst_start..s.burst_end + 16;
        let bursts: Vec<(f32, f32, f32)> = field
            .chroma
            .par_chunks(ls)
            .map(|c| {
                let (mut zr, mut zi, mut amp) = (0f32, 0f32, 0f32);
                for cyc in gate.clone().step_by(4) {
                    let (mut cr, mut ci) = (0f32, 0f32);
                    for n in cyc..cyc + 4 {
                        cr += c[n] * COS4[n & 3];
                        ci -= c[n] * SIN4[n & 3];
                    }
                    zr += cr;
                    zi += ci;
                    amp = amp.max(0.5 * (cr * cr + ci * ci).sqrt());
                }
                (zr, zi, amp)
            })
            .collect();

        // --- 副搬送波の再生 (逐次): ラインごとにバーストへ同期し、バーストのないラインは惰性で回す ---
        self.refs.clear();
        for (l, &(zr, zi, amp)) in bursts.iter().enumerate() {
            let sl = start + l as u64;
            let valid = amp >= KILL_BURST_IRE;
            let mu = zi.atan2(zr) - FRAC_PI_2;
            let sv = if pal {
                let alt = if sl & 1 == 0 { 1.0 } else { -1.0 };
                if valid {
                    if let Some((psl, pmu)) = self.prev_mu {
                        if psl + 1 == sl {
                            // 隣のラインとの位相差 ≒ (ライン当たりの進み) - 90°·sV
                            let d = wrap(mu - pmu - step);
                            if d.abs() > FRAC_PI_4 && d.abs() < 3.0 * FRAC_PI_4 {
                                self.pal_vote = (0.95 * self.pal_vote - d.signum() * alt).clamp(-20.0, 20.0);
                            }
                        }
                    }
                }
                self.prev_mu = valid.then_some((sl, mu));
                if self.pal_vote < 0.0 { -alt } else { alt }
            } else {
                1.0
            };
            let phase = match (valid, self.pll_prev) {
                (true, _) => if pal { mu + FRAC_PI_4 * sv } else { mu },
                (false, Some(ph)) => wrap(ph + step),
                (false, None) => 0.0,
            };
            self.pll_prev = Some(phase);
            self.refs.push((phase, sv, valid));
        }

        // --- 同期検波と色差ローパス (ライン並列) ---
        let lp = Cascade::butter_lowpass_at(2, CHROMA_BW_MHZ * 1e6, s.fs);
        let lead = lp.group_delay(1e3, s.fs).round().max(0.0) as usize;
        // バーストの区間は検波しない (色差の出力は帰線区間で 0)
        let from = s.burst_end + 8;
        self.pb.resize(field.data.len(), 0.0);
        self.pr.resize(field.data.len(), 0.0);
        let refs = &self.refs;
        let c_all = &field.chroma;
        self.pb.par_chunks_mut(ls).zip(self.pr.par_chunks_mut(ls)).enumerate().for_each(|(l, (pb, pr))| {
            pb.fill(0.0);
            pr.fill(0.0);
            let (psi, sv, valid) = refs[l];
            if !valid {
                return;
            }
            let c = &c_all[l * ls..(l + 1) * ls];
            let mut cos_t = [0f32; 4];
            let mut sin_t = [0f32; 4];
            for q in 0..4 {
                let a = q as f32 * FRAC_PI_2 + psi;
                cos_t[q] = 2.0 * a.cos();
                sin_t[q] = 2.0 * a.sin();
            }
            let (mut lu, mut lv) = (lp, lp);
            let (ku, kv) = (PB_PER_U, PR_PER_V * sv);
            for n in from..ls + lead {
                let x = if n < ls { c[n] } else { 0.0 };
                // C = U sin + V cos
                let u = lu.process(x * sin_t[n & 3]);
                let v = lv.process(x * cos_t[n & 3]);
                if n >= from + lead {
                    pb[n - lead] = ku * u;
                    pr[n - lead] = kv * v;
                }
            }
        });
        std::mem::swap(&mut field.chroma, &mut self.pb);
        if field.chroma_r.len() != field.data.len() {
            field.chroma_r = vec![0.0; field.data.len()];
        }
        std::mem::swap(&mut field.chroma_r, &mut self.pr);
        field.component = true;
    }
}

/// ダイレクト接続の色差の間引き。出力画像の `parity` の行 (今のフィールド) を、
/// Y, Cb, Cr にして色差を水平に間引き (4:2:2 はコサイトの [1 2 1]/4 で偶数画素に取り、奇数画素は補間)、RGB に戻す
pub fn subsample_rows(out: &mut RgbaFrame, parity: u8, sampling: ChromaSampling) {
    if sampling == ChromaSampling::Yuv444 {
        return;
    }
    let w = out.width;
    if w < 3 {
        return;
    }
    out.data.par_chunks_mut(w * 4).enumerate().for_each(|(row, px)| {
        if row % 2 != parity as usize {
            return;
        }
        let mut y = vec![0f32; w];
        let mut cb = vec![0f32; w];
        let mut cr = vec![0f32; w];
        for (x, p) in px.chunks_exact(4).enumerate() {
            let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
            let yy = 0.299 * r + 0.587 * g + 0.114 * b;
            y[x] = yy;
            cb[x] = (b - yy) * (0.5 / 0.886);
            cr[x] = (r - yy) * (0.5 / 0.701);
        }
        let at = |c: &[f32], x: isize| c[x.clamp(0, w as isize - 1) as usize];
        // 偶数画素の色差 (コサイト)
        let half: Vec<(f32, f32)> = (0..w.div_ceil(2))
            .map(|k| {
                let x = 2 * k as isize;
                let f = |c: &[f32]| 0.25 * at(c, x - 1) + 0.5 * at(c, x) + 0.25 * at(c, x + 1);
                (f(&cb), f(&cr))
            })
            .collect();
        for (x, p) in px.chunks_exact_mut(4).enumerate() {
            let k = x / 2;
            let (b, r) = if x % 2 == 0 {
                half[k]
            } else {
                let (a, c) = (half[k], half[(k + 1).min(half.len() - 1)]);
                (0.5 * (a.0 + c.0), 0.5 * (a.1 + c.1))
            };
            let yy = y[x];
            let rr = yy + r * (0.701 / 0.5);
            let bb = yy + b * (0.886 / 0.5);
            let gg = (yy - 0.299 * rr - 0.114 * bb) / 0.587;
            p[0] = (rr + 0.5).clamp(0.0, 255.0) as u8;
            p[1] = (gg + 0.5).clamp(0.0, 255.0) as u8;
            p[2] = (bb + 0.5).clamp(0.0, 255.0) as u8;
        }
    });
}
