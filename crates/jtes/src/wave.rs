//! 物理層: 296 ビットのデータラインを 1 ラインの波形にする・波形からパケットを取り出す。
//!
//! BT.653-3 Table 1b System D / 山田 (1986) 表 1:
//! - ビットレート 364 fH (= 8/5 fsc ≈ 5.727272 Mbit/s)、ビット周期 Tc = 1H / 364
//! - 水平同期パルスの前縁から、最初の立上り (クロックランの 1 ビット目) の振幅 50 % 点まで 56 Tc (≈ 9.78 µs)。
//!   296 Tc のデータラインの後、次の同期まで 12 Tc が残る (省令第77号 別図第三号)
//! - "0" = ペデスタル (0 %)、"1" = 白の 70 %
//! - 2 値 NRZ を、遮断 fc = 1/(2 Tc)、ロールオフ率 0.6 の余弦ロールオフ特性 (送受を合わせた全体の特性) で整形する
//!
//! ここでは各ビットを余弦ロールオフのインパルス応答 (ナイキストパルス) で描く。ビットの中心でほかのビットの
//! 影響が 0 になるので、ビット中心でスライスすれば受信できる。

use crate::fec;
use crate::packet::{CLOCK_RUN_IN, DataLine, FRAMING_CODE, LINE_BITS, Packet};

/// ロールオフ率
pub const ROLLOFF: f32 = 0.6;
/// 1H あたりのビット数 (ビットレート = 364 fH)
pub const BITS_PER_LINE: f32 = 364.0;
/// 同期前縁から、データ信号の最初の立上りの 50 % 点までの時間 [Tc]
pub const START_TC: f32 = 56.0;
/// 最初の立上りの 50 % 点から、クロックランの 1 ビット目のビット区間の始まりまで [Tc]。
/// 余弦ロールオフ (0.6) のパルスでは、1010… の最初の立上りはビット区間の境界より少し早く 50 % を横切る
/// (境界での値は 58.6 %)。テストで波形から測り直している
pub const EDGE_TO_CELL_TC: f32 = 0.060_765;
/// "1" のレベル [白 = 100 に対する %]
pub const HIGH_PERCENT: f32 = 70.0;

/// インパルス応答を打ち切る幅 [Tc] (片側)
const SPAN: f32 = 8.0;

/// 1 ラインのサンプル列上の時間とレベルの決め方
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineFormat {
    /// ビット周期 Tc [サンプル]
    pub tc: f32,
    /// データ信号の最初の立上りの 50 % 点 [サンプル、同期前縁 = 0]
    pub start: f32,
    /// "1" のレベル (呼び出し側の単位。"0" はペデスタル = 0)
    pub high: f32,
}

impl LineFormat {
    /// 1H のサンプル数と白レベル (100 %) から規格どおりの形式を作る
    pub fn for_line(line_samples: f32, white: f32) -> Self {
        let tc = line_samples / BITS_PER_LINE;
        LineFormat { tc, start: START_TC * tc, high: white * HIGH_PERCENT / 100.0 }
    }

    /// クロックランの 1 ビット目のビット区間の始まり [サンプル]
    pub fn cell_start(&self) -> f32 {
        self.start + EDGE_TO_CELL_TC * self.tc
    }

    /// ビット k の中心 [サンプル]
    pub fn bit_center(&self, k: usize) -> f32 {
        self.cell_start() + (k as f32 + 0.5) * self.tc
    }
}

/// 余弦ロールオフ (ロールオフ率 `beta`) のインパルス応答。x = t / Tc、p(0) = 1、p(整数 ≠ 0) = 0
pub fn raised_cosine(x: f32, beta: f32) -> f32 {
    use std::f32::consts::PI;
    let sinc = |u: f32| if u.abs() < 1e-6 { 1.0 } else { (PI * u).sin() / (PI * u) };
    let d = 1.0 - (2.0 * beta * x).powi(2);
    if d.abs() < 1e-4 {
        // x = ±1/(2β) の除去可能な特異点
        PI / 4.0 * sinc(1.0 / (2.0 * beta))
    } else {
        sinc(x) * (PI * beta * x).cos() / d
    }
}

/// ビット列 (送出順) を `out` に足す
pub fn render_bits(out: &mut [f32], bits: &[bool], f: &LineFormat) {
    let span = SPAN * f.tc;
    for (k, _) in bits.iter().enumerate().filter(|(_, b)| **b) {
        let c = f.bit_center(k);
        let lo = (c - span).floor().max(0.0) as usize;
        let hi = ((c + span).ceil() as usize + 1).min(out.len());
        for (n, v) in out.iter_mut().enumerate().take(hi).skip(lo) {
            *v += f.high * raised_cosine((n as f32 - c) / f.tc, ROLLOFF);
        }
    }
}

/// パケットをデータラインとして `out` (同期前縁を 0 とする 1 ラインのサンプル) に足す
pub fn render(out: &mut [f32], packet: &Packet, f: &LineFormat) {
    render_bits(out, &DataLine::bits(packet), f);
}

/// スライスの結果
#[derive(Clone, Debug, PartialEq)]
pub struct Sliced {
    /// フレーミングコードの後の 272 ビット (誤り訂正前)
    pub bits: [bool; fec::N],
    /// クロックランから求めた最初の立上りの 50 % 点 ([`LineFormat::start`] と同じ定義) [サンプル]
    pub start: f32,
    /// スライスレベル (クロックランの平均)
    pub threshold: f32,
    /// フレーミングコードの誤りビット数 (0 か 1)
    pub framing_errors: usize,
}

/// 1 ラインのサンプルからデータラインを探してビットを取り出す。
/// クロックランの位相でビットの位置を合わせ (±1 Tc)、フレーミングコード (1 ビットの誤りまで) でバイト位置を決める
pub fn slice(line: &[f32], f: &LineFormat) -> Option<Sliced> {
    use std::f32::consts::TAU;
    let run = CLOCK_RUN_IN.len() as f32;
    // クロックランの中央 (縁の過渡を避けて 2〜14 ビット目) で平均と位相を測る
    let a = f.cell_start() + 2.0 * f.tc;
    let b = f.cell_start() + (run - 2.0) * f.tc;
    let lo = a.max(0.0) as usize;
    let hi = (b as usize).min(line.len());
    if hi <= lo + 4 {
        return None;
    }
    let mean = line[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
    // 1010… は周期 2 Tc の正弦波。ビット 0 の中心で山になる
    let (mut zr, mut zi) = (0f32, 0f32);
    for (n, &v) in line.iter().enumerate().take(hi).skip(lo) {
        let th = TAU * (n as f32 - f.bit_center(0)) / (2.0 * f.tc);
        zr += (v - mean) * th.cos();
        zi += (v - mean) * th.sin();
    }
    let amp = 2.0 * (zr * zr + zi * zi).sqrt() / (hi - lo) as f32;
    if amp < 0.2 * f.high.abs() {
        return None;
    }
    let shift = zi.atan2(zr) / TAU * 2.0 * f.tc;
    let sample = |t: f32| {
        let i = t.floor();
        let k = i as isize;
        if k < 0 || k as usize + 1 >= line.len() {
            return mean;
        }
        let fr = t - i;
        line[k as usize] * (1.0 - fr) + line[k as usize + 1] * fr
    };
    let polarity = f.high >= 0.0;
    let bit = |t: f32| (sample(t) > mean) == polarity;
    // クロックランの位相は 2 Tc ごとにしか決まらないので、フレーミングコードを ±4 ビットの範囲で探す
    let mut best: Option<(usize, i32)> = None;
    for off in (-4..=4).step_by(2) {
        let base = f.cell_start() + shift + off as f32 * f.tc;
        let at = |k: usize| base + (k as f32 + 0.5) * f.tc;
        let err = FRAMING_CODE.iter().enumerate().filter(|(i, c)| bit(at(16 + i)) != **c).count();
        if err <= 1 && best.is_none_or(|(e, _)| err < e) {
            best = Some((err, off));
        }
    }
    let (framing_errors, off) = best?;
    let cell = f.cell_start() + shift + off as f32 * f.tc;
    let bits = std::array::from_fn(|i| bit(cell + ((LINE_BITS - fec::N + i) as f32 + 0.5) * f.tc));
    Some(Sliced { bits, start: cell - EDGE_TO_CELL_TC * f.tc, threshold: mean, framing_errors })
}

/// スライスから誤り訂正・パケットの解釈まで。(パケット, 訂正したビット数)
pub fn receive(line: &[f32], f: &LineFormat) -> Option<(Packet, usize)> {
    Packet::decode(&slice(line, f)?.bits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::Service;

    /// 4fsc の NTSC (910 サンプル/H)
    fn ntsc() -> LineFormat {
        LineFormat::for_line(910.0, 100.0)
    }

    fn test_packet() -> Packet {
        let mut p = Packet::empty(Service::Mode1, 5);
        p.data = std::array::from_fn(|i| (i as u8).wrapping_mul(73) ^ 0x5A);
        p
    }

    #[test]
    fn timing_constants() {
        let f = ntsc();
        assert!((f.tc - 2.5).abs() < 1e-6);
        // 56 Tc ≈ 9.78 µs (4fsc = 14.31818 MHz)
        let us = f.start / 14.318_18;
        assert!((us - 9.78).abs() < 0.01, "{us}");
        // 56 + 296 + 12 = 364 Tc で 1H
        assert_eq!(START_TC as usize + LINE_BITS + 12, BITS_PER_LINE as usize);
        assert!(f.cell_start() + LINE_BITS as f32 * f.tc < 910.0);
    }

    #[test]
    fn nyquist_pulse() {
        assert!((raised_cosine(0.0, ROLLOFF) - 1.0).abs() < 1e-6);
        for k in 1..8 {
            assert!(raised_cosine(k as f32, ROLLOFF).abs() < 1e-5);
        }
        // 特異点の前後で連続
        let x = 1.0 / (2.0 * ROLLOFF);
        assert!((raised_cosine(x, ROLLOFF) - raised_cosine(x + 1e-3, ROLLOFF)).abs() < 1e-2);
    }

    /// 描いた波形の最初の立上りが 56 Tc で 50 % (35 IRE) を横切る
    #[test]
    fn first_edge_half_amplitude_at_56_tc() {
        // サンプル間隔を細かくして測る (1 Tc = 64 サンプル)
        let f = LineFormat::for_line(364.0 * 64.0, 100.0);
        let mut y = vec![0f32; 364 * 64];
        render(&mut y, &test_packet(), &f);
        let half = f.high / 2.0;
        let from = (f.start - f.tc) as usize;
        let n = (from..).find(|&n| y[n] < half && y[n + 1] >= half).unwrap();
        let cross = n as f32 + (half - y[n]) / (y[n + 1] - y[n]);
        assert!((cross - f.start).abs() < 0.005 * f.tc, "{} Tc", (cross - f.start) / f.tc);
    }

    #[test]
    fn waveform_levels_and_start() {
        let f = ntsc();
        let mut y = vec![0f32; 910];
        let bits = DataLine::bits(&test_packet());
        render_bits(&mut y, &bits, &f);
        // ビット中心で 0 / 70 に戻る (符号間干渉なし)
        for (k, &b) in bits.iter().enumerate() {
            // 中心が整数サンプルにない (Tc = 2.5) ので線形補間の誤差を許す
            let c = f.bit_center(k);
            let n = c.floor() as usize;
            let fr = c - n as f32;
            let iv = y[n] * (1.0 - fr) + y[n + 1] * fr;
            let expect = if b { 70.0 } else { 0.0 };
            assert!((iv - expect).abs() < 9.0, "bit {k}: {iv}");
        }
        // データラインより前 (同期・バースト) にはほとんど何も描かない
        let pre = (f.start - 3.0 * f.tc) as usize;
        assert!(y[..pre].iter().all(|v| v.abs() < 2.0));
        // 帯域: 平均は 35 付近、振幅は 70 を大きく超えない
        let max = y.iter().cloned().fold(f32::MIN, f32::max);
        assert!(max < 85.0, "{max}");
    }

    #[test]
    fn slices_back() {
        let f = ntsc();
        let p = test_packet();
        let mut y = vec![0f32; 910];
        render(&mut y, &p, &f);
        let s = slice(&y, &f).unwrap();
        assert_eq!(s.framing_errors, 0);
        assert!((s.start - f.start).abs() < 0.3, "{}", s.start);
        assert_eq!(s.bits, p.encode());
        assert_eq!(receive(&y, &f), Some((p.clone(), 0)));
        // 何もないライン
        assert_eq!(slice(&vec![0f32; 910], &f), None);
    }

    #[test]
    fn slices_with_offset_gain_and_noise() {
        let f = ntsc();
        let p = test_packet();
        let mut rng = 12345u64;
        let mut noise = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        };
        for (delay, gain, dc) in [(1.3f32, 1.0f32, 0.0f32), (-2.2, 0.7, 5.0), (3.7, 1.2, -3.0)] {
            let shifted = LineFormat { start: f.start + delay, ..f };
            let mut y = vec![0f32; 910];
            render(&mut y, &p, &LineFormat { high: f.high * gain, ..shifted });
            for v in &mut y {
                *v += dc + 20.0 * noise();
            }
            let (q, _) = receive(&y, &f).unwrap_or_else(|| panic!("delay {delay}"));
            assert_eq!(q, p);
        }
    }
}
