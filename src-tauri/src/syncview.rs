//! 生信号ビュー: コンポジット信号をデコードせずに、そのまま白黒の画像にする (MMSSTV の Sync 画面のようなもの)。
//!
//! 最近のフィールドを切れ目なく並べて 1 本の信号にし、指定した幅 (サンプル数、小数可) ごとに折り返して行にする。
//! 縦の同期は信号から見つけた垂直同期 (最初の幅広パルス) で取り、横は同期を取らない。
//! 幅がラインの長さとずれていれば、水平同期のパルスが斜めの帯になる。帰線区間・VBI・同期も全部見える。

use crate::scope::ScopeField;
use std::sync::Arc;

/// 表示の設定
#[derive(Clone, Copy, Debug)]
pub struct SyncViewReq {
    /// 1 行のサンプル数 (ラインの長さ。ずらすと斜めになる)
    pub width: f64,
    /// 横の位置 [サンプル]
    pub phase: f64,
    /// true なら 1 フレーム (第 1 フィールドの垂直同期で揃える)、false なら 1 フィールド
    pub frame: bool,
    /// 垂直同期で揃える (false ならフリーラン: 最新の信号の末尾で揃える)
    pub sync: bool,
    /// 垂直同期の前 (と、次の垂直同期の後) に見せるライン数
    pub pre_lines: f64,
}

/// 並べた信号のうち、1 つのフィールドが受け持つ区間
struct Seg {
    offset: usize,
    first_line: usize,
}

/// 切れ目なく並べた信号
struct Stream {
    data: Vec<f32>,
    segs: Vec<Seg>,
    ls: usize,
    total: usize,
}

impl Stream {
    /// 新しい方から続いている (番号が連続し、方式が同じ) フィールドだけを並べる
    fn build(fields: &[Arc<ScopeField>]) -> Option<Stream> {
        let last = fields.last()?;
        let mut start = fields.len() - 1;
        while start > 0 {
            let (a, b) = (&fields[start - 1], &fields[start]);
            if a.id + 1 != b.id || a.line_samples != b.line_samples || a.pal != b.pal || a.parity == b.parity {
                break;
            }
            start -= 1;
        }
        let ls = last.line_samples;
        let mut data = Vec::new();
        let mut segs = Vec::new();
        for f in &fields[start..] {
            segs.push(Seg { offset: data.len(), first_line: f.first_line });
            data.extend_from_slice(&f.data[..f.own_lines * ls]);
        }
        Some(Stream { data, segs, ls, total: last.frame_total_lines })
    }

    /// 位置 `s` [サンプル] のライン番号 (1 始まり、小数部はライン内の位置)
    fn line_at(&self, s: f64) -> f64 {
        let i = self.segs.partition_point(|g| g.offset as f64 <= s).max(1) - 1;
        let g = &self.segs[i];
        let l = g.first_line as f64 - 1.0 + (s - g.offset as f64) / self.ls as f64;
        l.rem_euclid(self.total as f64) + 1.0
    }
}

/// 垂直同期の始まり (最初の幅広パルスの立ち下がり) の位置を全部探す。
/// 同期レベルと帰線レベルの中間より低い区間のうち、1/4 ラインより長いものが幅広パルス
/// (水平同期 4.7 µs、等化パルス 2.3 µs はずっと短い)。直前の 2 ラインに幅広パルスがなければ垂直同期の始まり
fn find_vsyncs(d: &[f32], ls: usize, pal: bool) -> Vec<usize> {
    let th = if pal { -150.0 / 7.0 } else { -20.0 };
    // 副搬送波 2 周期ぶんの平均で色とノイズを落とす
    const K: usize = 8;
    let min_run = ls / 4;
    let mut out = Vec::new();
    let mut acc = 0.0f32;
    let mut run_start: Option<usize> = None;
    let mut last_broad_end: Option<usize> = None;
    for i in 0..d.len() {
        acc += d[i];
        if i >= K {
            acc -= d[i - K];
        }
        if i + 1 < K {
            continue;
        }
        // 平均の中心
        let c = i + 1 - K / 2;
        let low = acc / (K as f32) < th;
        match (low, run_start) {
            (true, None) => run_start = Some(c),
            (false, Some(st)) => {
                run_start = None;
                // 信号の先頭で切れているパルスは始まりがわからないので使わない
                if c - st >= min_run && st > K {
                    let fresh = last_broad_end.is_none_or(|e| st - e > 2 * ls);
                    // 最初の幅広パルスの前が信号に入っていないと、本当に最初かわからない
                    if fresh && (last_broad_end.is_some() || st > 2 * ls) {
                        out.push(st);
                    }
                    last_broad_end = Some(c);
                }
            }
            _ => {}
        }
    }
    out
}

/// 円周 (ライン番号) 上の距離
fn circ_dist(a: f64, b: f64, total: f64) -> f64 {
    let d = (a - b).rem_euclid(total);
    d.min(total - d)
}

/// 描いた画像
pub struct SyncImage {
    pub id: u64,
    pub pal: bool,
    pub width: usize,
    pub height: usize,
    /// 垂直同期で揃えられた
    pub locked: bool,
    /// 1 行目の先頭のライン番号 (小数部はライン内の位置)
    pub first_line: f64,
    /// 1 行あたりのライン数 (幅 / ラインの長さ)
    pub lines_per_row: f64,
    pub total_lines: usize,
    pub fs: f32,
    /// 輝度 (同期レベル = 0、白 100 IRE = 255)
    pub gray: Vec<u8>,
}

impl SyncImage {
    /// リトルエンディアン: id: u64, width, height, locked, pal, total_lines: u32,
    /// first_line: f64, lines_per_row: f64, fs: f32, 続いて輝度 width × height バイト
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(48 + self.gray.len());
        o.extend_from_slice(&self.id.to_le_bytes());
        for v in [self.width, self.height, self.locked as usize, self.pal as usize, self.total_lines] {
            o.extend_from_slice(&(v as u32).to_le_bytes());
        }
        o.extend_from_slice(&self.first_line.to_le_bytes());
        o.extend_from_slice(&self.lines_per_row.to_le_bytes());
        o.extend_from_slice(&self.fs.to_le_bytes());
        o.extend_from_slice(&self.gray);
        o
    }
}

/// フィールドの列 (古い順) から画像を作る。信号が足りなければ None
pub fn render(fields: &[Arc<ScopeField>], req: &SyncViewReq) -> Option<SyncImage> {
    let st = Stream::build(fields)?;
    let last = fields.last()?;
    let ls = st.ls;
    let w = req.width.clamp(ls as f64 * 0.25, ls as f64 * 4.0);
    let pre = req.pre_lines.clamp(0.0, 50.0);
    let span_lines = if req.frame { st.total as f64 } else { st.total as f64 / 2.0 };
    let view = (span_lines + 2.0 * pre) * ls as f64;
    let rows = (view / w).ceil() as usize;
    // 1 行の画素数はラインのサンプル数のまま (幅を変えても画像の大きさは変えない)
    let cols = ls;
    let need = rows as f64 * w + w;
    let len = st.data.len() as f64;
    let phase = req.phase.rem_euclid(w);

    let mut start = None;
    if req.sync {
        let vs = find_vsyncs(&st.data, ls, last.pal);
        let total = st.total as f64;
        for &v in vs.iter().rev() {
            // 第 1・第 2 フィールドのどちらの垂直同期か (決まった位置のどちらに近いか)
            let n = st.line_at(v as f64);
            let field1 = circ_dist(n, last.vsync_line[0] as f64, total)
                <= circ_dist(n, last.vsync_line[1] as f64, total);
            if req.frame && !field1 {
                continue;
            }
            let s = v as f64 - pre * ls as f64 + phase;
            if s >= 0.0 && s + need <= len {
                start = Some(s);
                break;
            }
        }
    }
    let locked = start.is_some();
    let start = match start {
        Some(s) => s,
        None => {
            // フリーラン: 最新の信号の末尾に合わせる
            let s = len - need;
            if s < 0.0 {
                return None;
            }
            // 横の位置だけは設定どおりにする
            s - (s - phase).rem_euclid(w)
        }
    };

    let (sync, white) = if last.pal { (-300.0 / 7.0, 100.0) } else { (-40.0, 100.0) };
    let scale = 255.0 / (white - sync);
    let step = w / cols as f64;
    let d = &st.data;
    let mut gray = vec![0u8; rows * cols];
    for (r, row) in gray.chunks_mut(cols).enumerate() {
        let base = start + r as f64 * w;
        for (x, px) in row.iter_mut().enumerate() {
            let t = base + x as f64 * step;
            let i = t.floor() as usize;
            let v = match (d.get(i), d.get(i + 1)) {
                (Some(&a), Some(&b)) => a + (b - a) * (t - i as f64) as f32,
                (Some(&a), None) => a,
                _ => sync,
            };
            *px = ((v - sync) * scale).clamp(0.0, 255.0) as u8;
        }
    }
    Some(SyncImage {
        id: last.id,
        pal: last.pal,
        width: cols,
        height: rows,
        locked,
        first_line: st.line_at(start),
        lines_per_row: w / ls as f64,
        total_lines: st.total,
        fs: last.fs,
        gray,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::ScopeField;
    use vhs_core::frame::Field;
    use vhs_core::timing::{NTSC, PAL, Standard};

    /// 同期パルスだけの信号を、ライン番号から作る (垂直同期のパルスの位置を見る)
    fn fields_with_sync(s: &'static Standard, n: usize) -> Vec<Arc<ScopeField>> {
        let ls = s.line_samples;
        let total = s.frame_total_lines;
        let sync = s.ire.sync;
        (0..n as u64)
            .map(|i| {
                let mut f = Field::new(s);
                f.set_index(i);
                let first = s.raster_first_line[f.parity as usize & 1];
                f.data.iter_mut().for_each(|v| *v = 50.0);
                for row in 0..s.raster_lines {
                    let line = (first - 1 + row) % total + 1;
                    let d = &mut f.data[row * ls..(row + 1) * ls];
                    // 垂直同期: vsync_line から 3 ライン (半ライン単位で幅広パルス)
                    let in_v = |pos: f32| {
                        s.vsync_line.iter().any(|&v| {
                            let rel = (pos - (v - 1.0)).rem_euclid(total as f32);
                            rel < 3.0
                        })
                    };
                    for k in 0..ls {
                        let pos = (line - 1) as f32 + k as f32 / ls as f32;
                        let half = (k as f32 / ls as f32 * 2.0).fract() * ls as f32 / 2.0;
                        let low = if in_v(pos) { half < ls as f32 / 2.0 - s.serration_width } else { k < s.sync_end };
                        if low {
                            d[k] = sync;
                        }
                    }
                }
                Arc::new(ScopeField::from_field(i + 1, &f))
            })
            .collect()
    }

    fn check(s: &'static Standard) {
        let fields = fields_with_sync(s, 6);
        let ls = s.line_samples as f64;
        let req = SyncViewReq { width: ls, phase: 0.0, frame: true, sync: true, pre_lines: 3.0 };
        let img = render(&fields, &req).expect("画像");
        assert!(img.locked);
        // 第 1 フィールドの垂直同期の 3 ライン前から始まる
        let want = (s.vsync_line[0] as f64 - 3.0 - 1.0).rem_euclid(s.frame_total_lines as f64) + 1.0;
        assert!(circ_dist(img.first_line, want, s.frame_total_lines as f64) < 0.01, "{} {}", img.first_line, want);
        assert_eq!(img.width, s.line_samples);
        assert_eq!(img.height, s.frame_total_lines + 6);
        // 揃えた位置 (3 行目の先頭) は同期レベル
        assert_eq!(img.gray[3 * img.width + 2], 0);

        // フィールド表示: どちらのフィールドの垂直同期でもよい
        let req = SyncViewReq { frame: false, ..req };
        let img = render(&fields, &req).unwrap();
        assert!(img.locked);
        let d = s.vsync_line.iter().map(|&v| circ_dist(img.first_line + 3.0, v as f64, s.frame_total_lines as f64));
        assert!(d.fold(f64::MAX, f64::min) < 0.01);
    }

    #[test]
    fn locks_to_vsync_ntsc() {
        check(&NTSC);
    }

    #[test]
    fn locks_to_vsync_pal() {
        check(&PAL);
    }

    #[test]
    fn free_runs_without_sync() {
        let s = &NTSC;
        let fields: Vec<_> = (0..4u64)
            .map(|i| {
                let mut f = Field::new(s);
                f.set_index(i);
                Arc::new(ScopeField::from_field(i + 1, &f))
            })
            .collect();
        let req = SyncViewReq { width: 910.5, phase: 0.0, frame: false, sync: true, pre_lines: 3.0 };
        let img = render(&fields, &req).unwrap();
        assert!(!img.locked);
        assert!((img.lines_per_row - 910.5 / 910.0).abs() < 1e-9);
    }

    /// 実際のパイプライン (エンコーダ・テープ) の信号でも垂直同期で揃う
    #[test]
    fn locks_on_pipeline_output() {
        for pal in [false, true] {
            let mut params = vhs_core::Params::default();
            if pal {
                params.standard = vhs_core::timing::TvStandard::Pal;
            }
            let mut p = vhs_core::pipeline::Pipeline::new(params);
            let (w, h) = p.input_size();
            let input = vhs_core::frame::RgbFrame::new(w, h);
            let s = p.standard();
            let mut fields = Vec::new();
            for i in 0..6u64 {
                p.process_field(&input);
                fields.push(Arc::new(ScopeField::from_field(i + 1, p.composite())));
            }
            let req = SyncViewReq { width: s.line_samples as f64, phase: 0.0, frame: true, sync: true, pre_lines: 3.0 };
            let img = render(&fields, &req).unwrap();
            assert!(img.locked, "pal={pal}");
            let want = (s.vsync_line[0] as f64 - 4.0).rem_euclid(s.frame_total_lines as f64) + 1.0;
            assert!(circ_dist(img.first_line, want, s.frame_total_lines as f64) < 0.5, "pal={pal} {} {}", img.first_line, want);
        }
    }
}
