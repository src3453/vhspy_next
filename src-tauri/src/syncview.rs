//! 生信号ビュー: コンポジット信号をデコードせずに、そのまま画像にする (MMSSTV の Sync 画面のようなもの)。
//!
//! 最近のフィールドを切れ目なく並べて 1 本の信号にし、指定した幅 (サンプル数、小数可) ごとに折り返して行にする。
//! 縦の同期は信号から見つけた垂直同期 (最初の幅広パルス) で取り、横は同期を取らない。
//! 幅がラインの長さとずれていれば、水平同期のパルスが斜めの帯になる。帰線区間・VBI・同期も全部見える。
//! 画素は信号のレベルそのもの ([`RANGE_LO`]..[`RANGE_HI`] IRE を 0..255) で、色付けとレベル調整は表示側でする。

use crate::scope::ScopeField;
use std::sync::Arc;

/// 画像に入れる信号の範囲 [IRE] (この範囲を 0..255 にする。同期の下・スーパーホワイトの上も見えるように広めにとる)
pub const RANGE_LO: f32 = -60.0;
pub const RANGE_HI: f32 = 140.0;

/// 揃える垂直同期
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Field1,
    Field2,
    /// どちらでも (いちばん新しいもの)
    Either,
}

/// 表示の設定
#[derive(Clone, Copy, Debug)]
pub struct SyncViewReq {
    /// 1 行のサンプル数 (ラインの長さ。ずらすと斜めになる)
    pub width: f64,
    /// 横の位置 [サンプル]
    pub phase: f64,
    /// true なら 1 フレーム、false なら 1 フィールドを見せる
    pub frame: bool,
    /// どのフィールドの垂直同期で揃えるか
    pub anchor: Anchor,
    /// 垂直同期で揃える (false ならフリーラン: 最新の信号の末尾で揃える)
    pub sync: bool,
    /// 垂直同期の前 (と、次の垂直同期の後) に見せるライン数
    pub pre_lines: f64,
    /// 垂直同期を見失ったときに保つ位置 (前の画像の 1 行目のライン番号)。None なら保たずにフリーラン
    pub hold: Option<f64>,
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

    /// ライン番号 `line` (小数可) になる位置のうち、`need` サンプルが後ろに収まるいちばん新しいもの
    fn latest_at_line(&self, line: f64, need: f64) -> Option<f64> {
        let len = self.data.len() as f64;
        let total = self.total as f64;
        for (i, g) in self.segs.iter().enumerate().rev() {
            let end = self.segs.get(i + 1).map_or(len, |n| n.offset as f64);
            let s = g.offset as f64 + (line - g.first_line as f64).rem_euclid(total) * self.ls as f64;
            if s < end && s + need <= len {
                return Some(s);
            }
        }
        None
    }
}

/// 垂直同期の始まり (最初の幅広パルスの立ち下がり) の位置 [サンプル、小数] を全部探す。
///
/// 受像機の垂直同期分離 (積分回路) と同じ考え方で、0.4 ライン幅の窓のうち 3/4 以上が同期レベルなら幅広パルスとする
/// (水平同期 4.7 µs、等化パルス 2.3 µs ではそこまで溜まらない)。ノイズでパルスが途切れても、窓で積分するので崩れない。
/// しきい値は信号から測った同期の先端に対して決めるので、同期が減衰していても取れる。
/// 直前の 2 ラインに幅広パルスがなければ垂直同期の始まりとし、立ち下がりの位置を補間で求める。
fn find_vsyncs(d: &[f32], ls: usize) -> Vec<f64> {
    let n = d.len();
    if n < 4 * ls {
        return Vec::new();
    }
    // 副搬送波 2 周期ぶんの平均で色とノイズを落とす (中心を揃える)
    const K: usize = 8;
    let mut pre = Vec::with_capacity(n + 1);
    pre.push(0.0f64);
    let mut acc = 0.0f64;
    for &v in d {
        acc += v as f64;
        pre.push(acc);
    }
    let sm: Vec<f32> = (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(K / 2), (i + K / 2).min(n));
            ((pre[b] - pre[a]) / (b - a) as f64) as f32
        })
        .collect();

    // 同期の先端: 低い方から 0.5 % の値 (水平同期だけでも全体の 7 % あるので、先端の中に入る)
    let mut sorted: Vec<f32> = sm.iter().step_by(7).copied().collect();
    let k = sorted.len() / 200;
    let (_, tip, _) = sorted.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
    let tip = *tip;
    if !(tip < -8.0) {
        // 同期らしいものがない
        return Vec::new();
    }
    // 先端の 1/4 (NTSC で約 -10 IRE)。垂直同期だけが減衰している信号 (幅広パルスが水平同期より浅い) でも取れるように低めにする。
    // 映像は黒でも 0 IRE 以上で、水平同期が窓の 2 割ほどを占めるだけなので、窓の 3/4 には届かない
    let th = (tip * 0.25).clamp(-20.0, -4.0);

    let mut lc = Vec::with_capacity(n + 1);
    lc.push(0u32);
    let mut c = 0u32;
    for &v in &sm {
        c += (v < th) as u32;
        lc.push(c);
    }
    let low_in = |a: usize, len: usize| lc[(a + len).min(n)] - lc[a.min(n)];

    let win = (ls as f64 * 0.4) as usize;
    let need = (win * 3 / 4) as u32;
    // 立ち下がり: ここから 48 サンプルのうち 42 以上が同期レベル (ノイズのとげでは止まらない)
    const EDGE: usize = 48;
    const EDGE_NEED: u32 = 42;
    let mut out = Vec::new();
    let mut last_broad: Option<usize> = None;
    for i in 0..n - win {
        if low_in(i, win) < need {
            continue;
        }
        let fresh = match last_broad {
            Some(e) => i - e > 2 * ls,
            // 信号の先頭の近くでは、本当に最初の幅広パルスかわからない
            None => i > 2 * ls,
        };
        last_broad = Some(i);
        if !fresh {
            continue;
        }
        let Some(j) = (i..(i + win).min(n - EDGE)).find(|&j| sm[j] < th && low_in(j, EDGE) >= EDGE_NEED) else {
            continue;
        };
        // しきい値を横切る位置を前のサンプルとの間で補間する
        let pos = if j > 0 && sm[j - 1] >= th {
            let (a, b) = (sm[j - 1], sm[j]);
            (j - 1) as f64 + ((a - th) / (a - b)) as f64
        } else {
            j as f64
        };
        out.push(pos);
    }
    out
}

/// 円周 (ライン番号) 上の距離
fn circ_dist(a: f64, b: f64, total: f64) -> f64 {
    let d = (a - b).rem_euclid(total);
    d.min(total - d)
}

/// 縦の同期の状態
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lock {
    /// 同期を取らずに流している
    Free = 0,
    /// 垂直同期で揃えた
    Locked = 1,
    /// 垂直同期を見失ったので、前の位置を保っている
    Hold = 2,
}

/// 描いた画像
pub struct SyncImage {
    pub id: u64,
    pub pal: bool,
    pub width: usize,
    pub height: usize,
    pub lock: Lock,
    /// 1 行目の先頭のライン番号 (小数部はライン内の位置)
    pub first_line: f64,
    /// 1 行あたりのライン数 (幅 / ラインの長さ)
    pub lines_per_row: f64,
    pub total_lines: usize,
    pub fs: f32,
    /// 信号のレベル ([`RANGE_LO`] = 0、[`RANGE_HI`] = 255)
    pub gray: Vec<u8>,
}

impl SyncImage {
    /// リトルエンディアン: id: u64, width, height, lock, pal, total_lines: u32,
    /// first_line: f64, lines_per_row: f64, fs: f32, range_lo: f32, range_hi: f32, 続いてレベル width × height バイト
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(56 + self.gray.len());
        o.extend_from_slice(&self.id.to_le_bytes());
        for v in [self.width, self.height, self.lock as usize, self.pal as usize, self.total_lines] {
            o.extend_from_slice(&(v as u32).to_le_bytes());
        }
        o.extend_from_slice(&self.first_line.to_le_bytes());
        o.extend_from_slice(&self.lines_per_row.to_le_bytes());
        for v in [self.fs, RANGE_LO, RANGE_HI] {
            o.extend_from_slice(&v.to_le_bytes());
        }
        o.extend_from_slice(&self.gray);
        o
    }

    /// 画素 (x, y) の信号レベル [IRE]
    #[cfg(test)]
    fn ire(&self, x: usize, y: usize) -> f32 {
        RANGE_LO + self.gray[y * self.width + x] as f32 * (RANGE_HI - RANGE_LO) / 255.0
    }
}

/// フィールドの列 (古い順) から画像を作る。信号が足りなければ None
pub fn render(fields: &[Arc<ScopeField>], req: &SyncViewReq) -> Option<SyncImage> {
    let st = Stream::build(fields)?;
    let last = fields.last()?;
    let ls = st.ls;
    let lsf = ls as f64;
    let w = req.width.clamp(lsf * 0.25, lsf * 4.0);
    let pre = req.pre_lines.clamp(0.0, 50.0);
    let span_lines = if req.frame { st.total as f64 } else { st.total as f64 / 2.0 };
    let view = (span_lines + 2.0 * pre) * lsf;
    let rows = (view / w).ceil() as usize;
    // 1 行の画素数はラインのサンプル数のまま (幅を変えても画像の大きさは変えない)
    let cols = ls;
    let need = rows as f64 * w + w;
    let len = st.data.len() as f64;
    let phase = req.phase.rem_euclid(w);

    let mut start = None;
    let mut lock = Lock::Free;
    if req.sync {
        let total = st.total as f64;
        for &v in find_vsyncs(&st.data, ls).iter().rev() {
            // 第 1・第 2 フィールドのどちらの垂直同期か (決まった位置のどちらに近いか)
            let n = st.line_at(v);
            let f = if circ_dist(n, last.vsync_line[0] as f64, total)
                <= circ_dist(n, last.vsync_line[1] as f64, total)
            {
                0
            } else {
                1
            };
            match (req.anchor, f) {
                (Anchor::Field1, 1) | (Anchor::Field2, 0) => continue,
                _ => {}
            }
            // 第 2 フィールドの垂直同期はラインの途中 (半ライン) から始まるので、行の頭をラインの頭に揃える
            // (揃えないと、フィールドごとに画像が半ライン横にずれる)
            let s = v - last.vsync_line[f].fract() as f64 * lsf - pre * lsf + phase;
            if s >= 0.0 && s + need <= len {
                start = Some(s);
                lock = Lock::Locked;
                break;
            }
        }
        if start.is_none() {
            if let Some(h) = req.hold {
                start = st.latest_at_line(h, need);
                if start.is_some() {
                    lock = Lock::Hold;
                }
            }
        }
    }
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

    let scale = 255.0 / (RANGE_HI - RANGE_LO);
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
                _ => RANGE_LO,
            };
            *px = ((v - RANGE_LO) * scale + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
    Some(SyncImage {
        id: last.id,
        pal: last.pal,
        width: cols,
        height: rows,
        lock,
        first_line: st.line_at(start),
        lines_per_row: w / lsf,
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

    fn req(width: f64, frame: bool, anchor: Anchor) -> SyncViewReq {
        SyncViewReq { width, phase: 0.0, frame, anchor, sync: true, pre_lines: 3.0, hold: None }
    }

    fn check(s: &'static Standard) {
        let fields = fields_with_sync(s, 6);
        let ls = s.line_samples as f64;
        let total = s.frame_total_lines as f64;
        let img = render(&fields, &req(ls, true, Anchor::Field1)).expect("画像");
        assert_eq!(img.lock, Lock::Locked);
        // 第 1 フィールドの垂直同期の 3 ライン前から始まる
        let want = (s.vsync_line[0] as f64 - 3.0 - 1.0).rem_euclid(total) + 1.0;
        assert!(circ_dist(img.first_line, want, total) < 0.01, "{} {}", img.first_line, want);
        assert_eq!(img.width, s.line_samples);
        assert_eq!(img.height, s.frame_total_lines + 6);
        // 揃えた位置 (3 行目の先頭) は同期レベル、その前は帰線レベル
        assert!((img.ire(2, 3) - s.ire.sync).abs() < 1.0, "{}", img.ire(2, 3));
        assert!(img.ire(img.width / 2, 1) > 40.0);

        // 第 2 フィールドで揃えても、行の頭はラインの頭 (半ラインずれない)
        let img = render(&fields, &req(ls, false, Anchor::Field2)).unwrap();
        assert_eq!(img.lock, Lock::Locked);
        let want = (s.vsync_line[1].floor() as f64 - 3.0 - 1.0).rem_euclid(total) + 1.0;
        assert!(circ_dist(img.first_line, want, total) < 0.01, "{} {}", img.first_line, want);
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
    fn free_runs_without_sync_and_holds_position() {
        let s = &NTSC;
        let fields: Vec<_> = (0..4u64)
            .map(|i| {
                let mut f = Field::new(s);
                f.set_index(i);
                Arc::new(ScopeField::from_field(i + 1, &f))
            })
            .collect();
        let r = req(910.5, false, Anchor::Field1);
        let img = render(&fields, &r).unwrap();
        assert_eq!(img.lock, Lock::Free);
        assert!((img.lines_per_row - 910.5 / 910.0).abs() < 1e-9);
        // 見失っても、前の位置を渡せばそこを保つ
        let img = render(&fields, &SyncViewReq { hold: Some(1.0), ..r }).unwrap();
        assert_eq!(img.lock, Lock::Hold);
        assert!(circ_dist(img.first_line, 1.0, 525.0) < 1e-6, "{}", img.first_line);
    }

    /// 実際のパイプライン (エンコーダ・テープ) の信号で、同期が劣化していても毎回同じ位置に揃う。
    /// 「どちらでも」で揃えても、第 1・第 2 フィールドで横 (ライン内の位置) がずれない
    #[test]
    fn steady_lock_on_degraded_pipeline_output() {
        for pal in [false, true] {
            let mut params = vhs_core::Params::default();
            if pal {
                params.standard = vhs_core::timing::TvStandard::Pal;
            }
            params.signal.vertical_sync.attenuation_pct = 55.0;
            params.signal.vertical_sync.noise_ire = 8.0;
            params.signal.horizontal_sync.noise_ire = 8.0;
            let mut p = vhs_core::pipeline::Pipeline::new(params);
            let s = p.standard();
            let ls = s.line_samples as f64;
            let total = s.frame_total_lines as f64;
            let (w, h) = p.input_size();
            let mut input = vhs_core::frame::RgbFrame::new(w, h);
            for (i, px) in input.data.chunks_mut(3).enumerate() {
                px.fill(((i % w) * 255 / w) as u8);
            }
            let mut fields: Vec<Arc<ScopeField>> = Vec::new();
            let (mut lines2v, mut fracs1v) = (Vec::new(), Vec::new());
            for i in 0..30u64 {
                p.process_field(&input);
                fields.push(Arc::new(ScopeField::from_field(i + 1, p.composite())));
                if fields.len() < 6 {
                    continue;
                }
                let f = &fields[fields.len() - 6..];
                let img = render(f, &req(ls, true, Anchor::Field1)).unwrap();
                assert_eq!(img.lock, Lock::Locked, "pal={pal} field {i}");
                lines2v.push(img.first_line);
                let img = render(&f[2..], &req(ls, false, Anchor::Either)).unwrap();
                assert_eq!(img.lock, Lock::Locked, "pal={pal} field {i}");
                fracs1v.push(img.first_line.fract());
            }
            let want = (s.vsync_line[0] as f64 - 4.0).rem_euclid(total) + 1.0;
            for &l in &lines2v {
                // テープを通った分の遅れ (数十サンプル) はあってよい
                assert!(circ_dist(l, want, total) < 0.1, "pal={pal} {l} {want}");
                // 毎回同じ位置 (テープの時間軸の揺れ程度まで)
                assert!(circ_dist(l, lines2v[0], total) < 8.0 / ls, "pal={pal} 2V の位置が揺れる: {lines2v:?}");
            }
            let f0 = fracs1v[0];
            for &f in &fracs1v {
                let d = (f - f0).abs().min(1.0 - (f - f0).abs());
                assert!(d < 8.0 / ls, "pal={pal} 1V の横位置がずれる: {fracs1v:?}");
            }
        }
    }
}
