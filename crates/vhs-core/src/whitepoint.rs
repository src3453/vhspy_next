//! 受像機の白色点 (色温度)。
//!
//! SMPTE 170M (北米) の基準白は D65、日本の NTSC は 9300 K + 27 MPCD (いわゆる D93) を基準白として
//! スタジオモニタ・受像機を合わせてきた。信号の上では R = G = B が白なのはどちらも同じで、
//! 違うのは「その白を画面がどの色で光らせるか」だけ。
//!
//! ここでは受像機の画面の色を XYZ で求め、それを番組を作ったモニタの白 (視聴者が順応している白) から
//! 出力先 (sRGB、D65) の白へ Bradford 変換で順応させる。送り側と受像機の白が同じなら何も変わらず、
//! D65 の番組を D93 の受像機で見ると青白く、D93 の番組を D65 の受像機で見ると黄ばんで見える。
//! 原色は両方とも Rec.709 (≒ SMPTE C) とみなす。

use crate::frame::RgbaFrame;
use crate::params::WhitePoint;
use rayon::prelude::*;

type M3 = [[f64; 3]; 3];

impl WhitePoint {
    /// xy 色度
    pub fn xy(self) -> (f64, f64) {
        match self {
            WhitePoint::D65 => (0.3127, 0.3290),
            // 9300 K + 27 MPCD
            WhitePoint::D93 => (0.2831, 0.2971),
        }
    }

    fn xyz(self) -> [f64; 3] {
        let (x, y) = self.xy();
        [x / y, 1.0, (1.0 - x - y) / y]
    }
}

fn mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn mul_v(a: &M3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| (0..3).map(|k| a[i][k] * v[k]).sum())
}

fn inv(m: &M3) -> M3 {
    let c = |i: usize, j: usize| {
        let (r0, r1) = ((i + 1) % 3, (i + 2) % 3);
        let (c0, c1) = ((j + 1) % 3, (j + 2) % 3);
        m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
    };
    let det: f64 = (0..3).map(|j| m[0][j] * c(0, j)).sum();
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[j][i] = c(i, j) / det;
        }
    }
    r
}

/// Rec.709 原色で、RGB = (1, 1, 1) が `white` になる RGB→XYZ
fn rgb_to_xyz(white: WhitePoint) -> M3 {
    let prim = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)];
    let mut p = [[0.0; 3]; 3];
    for (j, &(x, y)) in prim.iter().enumerate() {
        p[0][j] = x / y;
        p[1][j] = 1.0;
        p[2][j] = (1.0 - x - y) / y;
    }
    let s = mul_v(&inv(&p), white.xyz());
    for row in p.iter_mut() {
        for j in 0..3 {
            row[j] *= s[j];
        }
    }
    p
}

/// Bradford 色順応 (src の白 → dst の白)
fn bradford(src: WhitePoint, dst: WhitePoint) -> M3 {
    const B: M3 = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
    let (s, d) = (mul_v(&B, src.xyz()), mul_v(&B, dst.xyz()));
    let diag = [[d[0] / s[0], 0.0, 0.0], [0.0, d[1] / s[1], 0.0], [0.0, 0.0, d[2] / s[2]]];
    mul(&inv(&B), &mul(&diag, &B))
}

/// 受像機の (線形) RGB から出力 (sRGB の線形 RGB) への行列。送り側と受像機の白が同じなら `None`
pub fn display_matrix(source: WhitePoint, receiver: WhitePoint) -> Option<[[f32; 3]; 3]> {
    if source == receiver {
        return None;
    }
    let m = mul(&inv(&rgb_to_xyz(WhitePoint::D65)), &mul(&bradford(source, WhitePoint::D65), &rgb_to_xyz(receiver)));
    // 受像機の白 (RGB = 1) が出力の範囲に収まるよう全体を暗くする (画面の最大輝度は受像機の白)
    let peak = m.iter().map(|r| r.iter().sum::<f64>()).fold(0.0, f64::max).max(1.0);
    Some(m.map(|r| r.map(|v| (v / peak) as f32)))
}

/// `parity` の行 (今回のフィールドが描いた行) に行列を掛ける。ガンマは 2.2 とみなす
pub fn apply_rows(frame: &mut RgbaFrame, parity: u8, m: &[[f32; 3]; 3]) {
    const N: usize = 4096;
    let to_lin: Vec<f32> = (0..256).map(|v| (v as f32 / 255.0).powf(2.2)).collect();
    let to_gam: Vec<u8> = (0..N).map(|i| ((i as f32 / (N - 1) as f32).powf(1.0 / 2.2) * 255.0).round() as u8).collect();
    let enc = |v: f32| to_gam[(v.clamp(0.0, 1.0) * (N - 1) as f32).round() as usize];
    let row_bytes = frame.width * 4;
    frame.data.par_chunks_mut(row_bytes).enumerate().for_each(|(row, dst)| {
        if row % 2 != parity as usize {
            return;
        }
        for px in dst.chunks_exact_mut(4) {
            let c = [to_lin[px[0] as usize], to_lin[px[1] as usize], to_lin[px[2] as usize]];
            for i in 0..3 {
                px[i] = enc(m[i][0] * c[0] + m[i][1] * c[1] + m[i][2] * c[2]);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_white_is_identity() {
        assert!(display_matrix(WhitePoint::D93, WhitePoint::D93).is_none());
    }

    #[test]
    fn d93_receiver_is_bluish() {
        let m = display_matrix(WhitePoint::D65, WhitePoint::D93).unwrap();
        let w: Vec<f32> = m.iter().map(|r| r.iter().sum()).collect();
        assert!(w[2] > w[0], "{w:?}");
        let m = display_matrix(WhitePoint::D93, WhitePoint::D65).unwrap();
        let w: Vec<f32> = m.iter().map(|r| r.iter().sum()).collect();
        assert!(w[0] > w[2], "{w:?}");
    }
}
