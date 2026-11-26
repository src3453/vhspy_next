//! オシロスコープ用に、最近のフィールドのコンポジット信号を持っておき、選んだラインだけを返す。
//!
//! エンジンはオシロスコープが開いている間だけ、毎フィールド [`ScopeRing::push`] する。フロントエンドは
//! 表示の更新ごとに [`ScopeRing::extract`] で前回より新しいフィールドを全部受け取り、残光に積み重ねる。

use std::collections::VecDeque;
use std::sync::Arc;
use vhs_core::frame::Field;

/// 持っておくフィールド数 (表示の更新が遅いときに間のフィールドを取りこぼさないため)
pub const RING_FIELDS: usize = 16;

/// 1 フィールドぶんの信号 (S 映像なら Y + C を足したもの)
pub struct ScopeField {
    pub id: u64,
    pub pal: bool,
    pub parity: u8,
    pub line_samples: usize,
    pub raster_lines: usize,
    /// バッファ先頭のライン番号 (1 始まり)
    pub first_line: usize,
    pub frame_total_lines: usize,
    /// バッファのうちこのフィールドだけのライン数 (次のフィールドのバッファの先頭と重ならない部分)。
    /// 続けて並べると切れ目のない信号になる
    pub own_lines: usize,
    /// 垂直同期の始まるライン番号 [第 1, 第 2 フィールド]
    pub vsync_line: [f32; 2],
    pub fs: f32,
    pub data: Arc<Vec<f32>>,
}

impl ScopeField {
    pub fn from_field(id: u64, field: &Field) -> Self {
        let s = field.std;
        let data = if field.separate {
            field
                .data
                .iter()
                .zip(&field.chroma)
                .map(|(y, c)| y + c)
                .collect()
        } else {
            field.data.clone()
        };
        let p = field.parity as usize & 1;
        let fl = s.raster_first_line;
        let own_lines = (fl[1 - p] + s.frame_total_lines - fl[p]) % s.frame_total_lines;
        Self {
            id,
            pal: s.is_pal(),
            parity: field.parity,
            line_samples: s.line_samples,
            raster_lines: s.raster_lines,
            first_line: s.raster_first_line[field.parity as usize & 1],
            frame_total_lines: s.frame_total_lines,
            own_lines: own_lines.min(s.raster_lines),
            vsync_line: s.vsync_line,
            fs: s.fs,
            data: Arc::new(data),
        }
    }

    /// ライン番号 n の信号 (このフィールドのバッファになければ None)
    fn line(&self, n: usize) -> Option<&[f32]> {
        let t = self.frame_total_lines;
        let i = (n + t - self.first_line % t) % t;
        (i < self.raster_lines)
            .then(|| &self.data[i * self.line_samples..(i + 1) * self.line_samples])
    }
}

#[derive(Default)]
pub struct ScopeRing {
    fields: VecDeque<Arc<ScopeField>>,
}

impl ScopeRing {
    pub fn push(&mut self, f: ScopeField) {
        self.fields.push_back(Arc::new(f));
        while self.fields.len() > RING_FIELDS {
            self.fields.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.fields.clear();
    }

    /// `after` より新しいフィールド (新しい方から最大 `max_fields`、古い順に並べる)
    /// 持っているいちばん新しい番号が `after` より小さければ番号が振り直された (エンジンの作り直し) ので、最初から返す。
    pub fn newer(&self, after: u64, max_fields: usize) -> Vec<Arc<ScopeField>> {
        let newest = self.fields.back().map_or(0, |f| f.id);
        let after = if newest < after { 0 } else { after };
        let mut v: Vec<_> = self
            .fields
            .iter()
            .rev()
            .filter(|f| f.id > after)
            .take(max_fields.max(1))
            .cloned()
            .collect();
        v.reverse();
        v
    }
}

/// フィールドの列から、ライン番号 `line` から `count` ライン (フレームの終わりで折り返す) のうち
/// 各フィールドにあるものを取り出す。リトルエンディアン:
/// ```text
/// 件数: u32
/// フィールドごと: id: u64, pal: u32, parity: u32, line_samples: u32, fs: f32, ライン数 n: u32,
///                 続いて n 回 (ライン番号: u32, f32 × line_samples)
/// ```
pub fn extract(fields: &[Arc<ScopeField>], line: usize, count: usize) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    for f in fields {
        let t = f.frame_total_lines;
        let wanted: Vec<(usize, &[f32])> = (0..count.min(t))
            .filter_map(|k| {
                let n = (line.max(1) - 1 + k) % t + 1;
                f.line(n).map(|d| (n, d))
            })
            .collect();
        out.extend_from_slice(&f.id.to_le_bytes());
        for v in [f.pal as u32, f.parity as u32, f.line_samples as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&f.fs.to_le_bytes());
        out.extend_from_slice(&(wanted.len() as u32).to_le_bytes());
        for (n, d) in wanted {
            out.extend_from_slice(&(n as u32).to_le_bytes());
            out.reserve(d.len() * 4);
            for v in d {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_requested_lines_per_field() {
        let s = &vhs_core::timing::NTSC;
        let mut ring = ScopeRing::default();
        for i in 0..20u64 {
            let mut f = Field::new(s);
            f.set_index(i);
            // バッファ先頭 (第 1 フィールドはライン 10、第 2 はライン 272) に目印
            f.data[0] = i as f32;
            ring.push(ScopeField::from_field(i + 1, &f));
        }
        assert_eq!(ring.newer(0, 100).len(), RING_FIELDS);
        let got = ring.newer(18, 8);
        assert_eq!(got.iter().map(|f| f.id).collect::<Vec<_>>(), vec![19, 20]);

        // ライン 10 から 3 ライン: 第 1 フィールド (id 19, index 18) は 3 ライン、第 2 フィールドにはない
        let out = extract(&got, 10, 3);
        let u32_at = |o: usize| u32::from_le_bytes(out[o..o + 4].try_into().unwrap());
        assert_eq!(u32_at(0), 2);
        assert_eq!(u64::from_le_bytes(out[4..12].try_into().unwrap()), 19);
        assert_eq!(u32_at(20), 910);
        assert_eq!(u32_at(28), 3);
        assert_eq!(u32_at(32), 10);
        assert_eq!(f32::from_le_bytes(out[36..40].try_into().unwrap()), 18.0);
        let second = 4 + 28 + 3 * (4 + 910 * 4);
        assert_eq!(u32_at(second + 24), 0);
        assert_eq!(out.len(), second + 28);
    }

    #[test]
    fn line_lookup_wraps_into_next_frame() {
        let s = &vhs_core::timing::NTSC;
        let mut f = Field::new(s);
        f.set_index(1); // 第 2 フィールド: ライン 272〜525、続いて次のフレームの 1〜9
        let sf = ScopeField::from_field(1, &f);
        assert!(sf.line(272).is_some() && sf.line(525).is_some() && sf.line(9).is_some());
        assert!(sf.line(10).is_none() && sf.line(271).is_none());
    }
}
