//! タイムベースコレクタ (ライン TBC)。
//!
//! 実機と同じく、再生信号を A/D 変換してラインメモリに書き込み、ラインごとに水平同期の位置を検出して
//! 基準のクロックで読み出し直す (= 検出した同期の位置ずれだけ逆にずらす)。ジッタ・ワウフラッタ・
//! スキュー・ヘッド切替の曲がりは補正範囲内なら消える。
//!
//! TBC エラーとして次のものが出る。
//! - 補正範囲 (ラインメモリの余裕) を超えた時間軸誤差はそのまま残る
//! - 同期の検出はゲート (前のラインの同期位置の付近) の中だけで行う。ゲートの外に同期が飛んだ
//!   (ヘッド切替・ドロップアウト・ノイズバー) ラインは同期を見失い、前の補正量を保持する。
//!   [`TbcParams::reacquire_lines`] ライン見失い続けるとゲートを開いて再捕捉する
//! - ノイズによる同期の誤トリガ。誤った位置がゲートの基準にもなるので、何ラインか続けて横にずれる
//! - 補正後に残る書き込みクロックのジッタと、A/D の量子化
//!
//! 同期・ブランキングは作り直すので、TBC が誤ったずれは受像機の AFC ではもう直らない。

use crate::decoder::find_sync_edge;
use crate::filter::fractional_shift;
use crate::frame::Field;
use crate::params::TbcParams;
use crate::rng::Rng;
use crate::timing::*;
use rayon::prelude::*;

const STREAM_TBC: u64 = 11;

pub struct Tbc {
    /// ゲートの基準となる同期の位置 (フィールドをまたいで保持)
    sync_pos: f32,
    /// 同期を見失っているライン数
    lost: u32,
    /// 直前に検出した (または保持している) 補正量
    held: f32,
    corrections: Vec<f32>,
    tmp: Vec<f32>,
}

impl Default for Tbc {
    fn default() -> Self {
        Self::new(&NTSC)
    }
}

impl Tbc {
    pub fn new(s: &Standard) -> Self {
        Self {
            sync_pos: s.sync_end as f32,
            lost: 0,
            held: 0.0,
            corrections: vec![0.0; s.field_lines],
            tmp: vec![0.0; s.field_lines * s.line_samples],
        }
    }

    /// 直前のフィールドでラインごとに掛けた補正量 [サンプル] (デバッグ用)
    pub fn corrections(&self) -> &[f32] {
        &self.corrections
    }

    pub fn process(&mut self, field: &mut Field, p: &TbcParams, seed: u64) {
        let s = field.std;
        let (fl, ls) = (s.field_lines, s.line_samples);
        if self.tmp.len() != fl * ls {
            *self = Self::new(s);
        }
        let nominal = s.sync_end as f32;
        let window = p.window_samples.max(0.0);
        let gate = p.gate_samples.max(0.5);

        // --- 同期の検出と補正量 (逐次: ゲートは前のラインの結果に依存する) ---
        for l in 0..fl {
            let mut rng = Rng::derive(seed, field.index, l as u64, STREAM_TBC);
            let wide_open = self.lost >= p.reacquire_lines;
            let found = find_sync_edge(field.line(l), s).filter(|&pos| wide_open || (pos - self.sync_pos).abs() <= gate);
            let mut pos = match found {
                Some(pos) => {
                    self.lost = 0;
                    Some(pos)
                }
                None => {
                    self.lost += 1;
                    None
                }
            };
            if rng.uniform() < p.error_rate {
                // 誤トリガ: ゲート内のどこかを同期と取り違える (ゲートを開いているときは補正範囲全体)
                let span = if wide_open { window.max(gate) } else { gate };
                pos = Some(self.sync_pos + (rng.uniform() * 2.0 - 1.0) * span);
                self.lost = 0;
            }
            if let Some(pos) = pos {
                self.sync_pos = pos;
                self.held = pos - nominal;
            }
            // 補正範囲を超えた分は直せない
            self.corrections[l] = self.held.clamp(-window, window) + rng.normal() * p.residual_samples;
        }

        // --- ラインメモリから読み出し直す ---
        let lv = s.ire;
        let corr = &self.corrections;
        let quant = (p.bits > 0 && p.bits < 24).then(|| {
            // A/D のレンジ: シンクチップの少し下〜ホワイトピークの少し上
            let (lo, hi) = (lv.sync - 10.0, lv.white + 40.0);
            let steps = (1u32 << p.bits.min(16)) as f32 - 1.0;
            (lo, hi, steps)
        });
        let regen = p.regenerate_sync;
        let separate = field.separate;
        self.tmp.copy_from_slice(&field.data);
        let src = &self.tmp;
        field.data.par_chunks_mut(ls).enumerate().for_each(|(l, out)| {
            let line = &src[l * ls..(l + 1) * ls];
            fractional_shift(line, out, -corr[l], lv.blank);
            if let Some((lo, hi, steps)) = quant {
                let k = steps / (hi - lo);
                for v in out.iter_mut() {
                    *v = ((*v - lo) * k).round().clamp(0.0, steps) / k + lo;
                }
            }
            if regen {
                out[..s.sync_end].fill(lv.sync);
                out[s.sync_end..s.burst_start].fill(lv.blank);
                out[s.burst_end..s.active_start].fill(lv.blank);
                out[s.active_end().min(ls)..].fill(lv.blank);
            }
        });
        if separate {
            self.tmp.copy_from_slice(&field.chroma);
            let src = &self.tmp;
            field.chroma.par_chunks_mut(ls).enumerate().for_each(|(l, out)| {
                fractional_shift(&src[l * ls..(l + 1) * ls], out, -corr[l], 0.0);
                if regen {
                    out[..s.burst_start].fill(0.0);
                    out[s.burst_end..s.active_start].fill(0.0);
                    out[s.active_end().min(ls)..].fill(0.0);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_field(offsets: &[f32]) -> Field {
        let s = &NTSC;
        let mut f = Field::new(s);
        let ls = s.line_samples;
        let mut clean = vec![0f32; ls];
        for (n, v) in clean.iter_mut().enumerate() {
            *v = if n < s.sync_end {
                s.ire.sync
            } else if n >= 400 {
                s.ire.white
            } else {
                s.ire.black
            };
        }
        for l in 0..s.field_lines {
            let off = offsets[l % offsets.len()];
            fractional_shift(&clean, &mut f.data[l * ls..(l + 1) * ls], off, s.ire.blank);
        }
        f
    }

    /// 白の立ち上がりの位置 (50 IRE を横切る位置)
    fn edge(line: &[f32]) -> f32 {
        for n in 200..line.len() {
            if line[n - 1] < 50.0 && line[n] >= 50.0 {
                return n as f32 - 1.0 + (50.0 - line[n - 1]) / (line[n] - line[n - 1]);
            }
        }
        f32::NAN
    }

    #[test]
    fn removes_jitter_within_window() {
        let offsets = [0.0, 3.5, -2.25, 5.0, 1.0, -4.0];
        let mut f = test_field(&offsets);
        let mut tbc = Tbc::default();
        let p = TbcParams { enabled: true, residual_samples: 0.0, bits: 0, gate_samples: 20.0, ..Default::default() };
        tbc.process(&mut f, &p, 1);
        let ls = NTSC.line_samples;
        // 同期のスライスの偏り (全ラインで同じ) を差し引くため、ずれのない 0 ライン目と比べる
        let e0 = edge(&f.data[..ls]);
        assert!((e0 - 400.0).abs() < 1.0, "line 0: edge {e0}");
        for l in 1..NTSC.field_lines {
            let e = edge(&f.data[l * ls..(l + 1) * ls]);
            assert!((e - e0).abs() < 0.2, "line {l}: edge {e} (line 0: {e0})");
        }
    }

    #[test]
    fn residual_beyond_window() {
        let p = TbcParams { enabled: true, window_samples: 5.0, residual_samples: 0.0, bits: 0, reacquire_lines: 0, ..Default::default() };
        let ls = NTSC.line_samples;
        // 補正範囲で頭打ちになるラインは同期のスライスの偏りも直さないので、処理前の信号と比べる
        let e0 = edge(&test_field(&[0.0]).data[10 * ls..11 * ls]);
        let mut f = test_field(&[20.0]);
        Tbc::default().process(&mut f, &p, 1);
        let e = edge(&f.data[10 * ls..11 * ls]);
        // 20 サンプルのずれのうち 5 サンプルだけ直る
        assert!((e - e0 - 15.0).abs() < 0.2, "edge {e} (reference {e0})");
    }
}
