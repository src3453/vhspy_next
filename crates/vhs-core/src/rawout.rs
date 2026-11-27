//! コンポジット信号を DAC へそのまま流せる raw サンプル列として書き出す。
//!
//! FL2000 系の USB-VGA アダプタ (osmo-fl2k の `fl2k_file`) などで実機のテレビへ入力するためのもの。
//!
//! - フィールドバッファのうちストリームに足すぶん ([`Standard::stream_lines`]) だけをつなげるので、
//!   帰線区間・インターレースの半ラインも含めて途切れない 1 本の信号になる。
//! - 振幅は IRE を線形にコードへ割り当てる (`ire_lo` → 0、`ire_hi` → 最大コード)。範囲外は切る。
//! - 4fsc のままだと副搬送波のイメージが fs − fsc (NTSC で約 10.7 MHz) に出るので、
//!   整数倍のアップサンプル (窓付き sinc の多相 FIR) で DAC 後のフィルタを簡単にできる。
//! - ヘッダはない。サンプルレートは [`RawWriter::sample_rate`] (= 4fsc × アップサンプル倍率)。

use crate::frame::Field;
use crate::timing::Standard;
use std::io::{self, Write};

/// 1 サンプルの形式
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// 符号なし 8 bit (0..=255)
    U8,
    /// 符号付き 8 bit (-128..=127)。osmo-fl2k の `fl2k_file` が既定で読む形式
    S8,
    /// 符号付き 16 bit リトルエンディアン
    S16Le,
}

impl SampleFormat {
    pub fn bytes(self) -> usize {
        match self {
            SampleFormat::U8 | SampleFormat::S8 => 1,
            SampleFormat::S16Le => 2,
        }
    }

    /// 拡張子・名前 ("u8" / "s8" / "s16") から
    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "u8" => Some(Self::U8),
            "s8" | "i8" => Some(Self::S8),
            "s16" | "s16le" | "i16" => Some(Self::S16Le),
            _ => None,
        }
    }
}

/// どの信号を書き出すか
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawChannel {
    /// コンポジット (S 映像のフィールドなら Y + C を足す)
    Composite,
    /// S 映像の Y (コンポジットのフィールドならコンポジットそのもの)
    Luma,
    /// S 映像の C。0 IRE が `ire_lo`〜`ire_hi` の中央のコードになる (コンポジット・コンポーネントのフィールドなら無信号)
    Chroma,
    /// コンポーネントの Pb。0 が中央のコードになる (コンポーネント以外のフィールドなら無信号)
    Pb,
    /// コンポーネントの Pr
    Pr,
}

#[derive(Clone, Copy, Debug)]
pub struct RawOptions {
    pub format: SampleFormat,
    /// アップサンプル倍率 (1 = 4fsc のまま)
    pub upsample: usize,
    /// コード 0 にする IRE
    pub ire_lo: f32,
    /// 最大コードにする IRE
    pub ire_hi: f32,
}

impl RawOptions {
    /// 既定: sync 先端の 5 IRE 下 〜 白の 35 IRE 上 (100% カラーバーの黄・シアンのピークが収まる)
    pub fn new(std: &Standard, format: SampleFormat, upsample: usize) -> Self {
        Self { format, upsample: upsample.max(1), ire_lo: std.ire.sync - 5.0, ire_hi: std.ire.white + 35.0 }
    }

    /// DAC の最大コードが負荷 (75 Ω) の両端で `volts` になるとき、sync 先端〜白が 1.0 V (規格の 1 Vpp) に
    /// なるよう `ire_hi` を決める (`ire_lo` はそのまま)。`volts` が小さいと白の側が切れるので注意
    pub fn with_full_scale_volts(mut self, std: &Standard, volts: f32) -> Self {
        let ire_per_volt = std.ire.white - std.ire.sync;
        self.ire_hi = self.ire_lo + ire_per_volt * volts.max(1e-3);
        self
    }

    /// 白 (100 IRE) が最大コードを超えないか
    pub fn clips_white(&self, std: &Standard) -> bool {
        self.ire_hi < std.ire.white
    }
}

/// 整数倍アップサンプラ (Blackman 窓付き sinc の多相 FIR)。フィールドをまたいで状態を持つ
struct Upsampler {
    factor: usize,
    /// 1 相あたりのタップ数
    taps: usize,
    /// 相ごとの係数 [phase][tap]
    coef: Vec<Vec<f32>>,
    /// 直前の入力の末尾 (taps − 1 サンプル)
    tail: Vec<f32>,
    ext: Vec<f32>,
}

impl Upsampler {
    /// 通過域は入力のナイキストの 0.9 倍 (NTSC 4fsc なら約 6.4 MHz)
    const CUTOFF: f64 = 0.9;
    const TAPS: usize = 32;

    fn new(factor: usize, fill: f32) -> Self {
        let taps = Self::TAPS;
        let half = taps as f64 / 2.0;
        let coef = (0..factor)
            .map(|p| {
                let mut h: Vec<f64> = (0..taps)
                    .map(|j| {
                        // 出力の時刻は窓の中央 (half − 1) から p / factor だけ進んだところ
                        let d = j as f64 - (half - 1.0) - p as f64 / factor as f64;
                        let x = Self::CUTOFF * d;
                        let sinc = if x.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
                        let t = (d / half).clamp(-1.0, 1.0);
                        let w = 0.42 + 0.5 * (std::f64::consts::PI * t).cos() + 0.08 * (2.0 * std::f64::consts::PI * t).cos();
                        sinc * w
                    })
                    .collect();
                // 直流利得を相ごとに 1 にする (同期・ブランキングのレベルが相で揺れないように)
                let s: f64 = h.iter().sum();
                h.iter_mut().for_each(|v| *v /= s);
                h.into_iter().map(|v| v as f32).collect()
            })
            .collect();
        Self { factor, taps, coef, tail: vec![fill; taps - 1], ext: Vec::new() }
    }

    /// 入力の遅れ [入力サンプル]
    fn delay(&self) -> usize {
        self.taps / 2 - 1
    }

    fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        out.clear();
        if self.factor == 1 {
            out.extend_from_slice(input);
            return;
        }
        self.ext.clear();
        self.ext.extend_from_slice(&self.tail);
        self.ext.extend_from_slice(input);
        out.reserve(input.len() * self.factor);
        for win in self.ext.windows(self.taps) {
            for h in &self.coef {
                out.push(win.iter().zip(h).map(|(a, b)| a * b).sum());
            }
        }
        let keep = self.taps - 1;
        self.tail.clear();
        self.tail.extend_from_slice(&self.ext[self.ext.len() - keep..]);
    }
}

/// フィールドを順に受け取り、連続した raw サンプル列として書き出す
pub struct RawWriter<W: Write> {
    out: W,
    std: &'static Standard,
    opts: RawOptions,
    channel: RawChannel,
    up: Upsampler,
    line: Vec<f32>,
    upsampled: Vec<f32>,
    bytes: Vec<u8>,
    samples: u64,
    fields: u64,
}

impl<W: Write> RawWriter<W> {
    pub fn new(out: W, std: &'static Standard, opts: RawOptions, channel: RawChannel) -> Self {
        // 書き出し前の状態はブランキング (C は 0) とみなす
        let fill = std.ire.blank;
        Self {
            out,
            std,
            opts,
            channel,
            up: Upsampler::new(opts.upsample.max(1), fill),
            line: Vec::new(),
            upsampled: Vec::new(),
            bytes: Vec::new(),
            samples: 0,
            fields: 0,
        }
    }

    /// 出力のサンプルレート [Hz]
    pub fn sample_rate(&self) -> f64 {
        Self::rate_for(self.std, self.opts.upsample)
    }

    pub fn rate_for(std: &Standard, upsample: usize) -> f64 {
        // fs は f32 で丸まっているので fsc から倍精度で計算し直す
        let fsc = match std.kind {
            crate::timing::TvStandard::Ntsc => 315.0e6 / 88.0,
            crate::timing::TvStandard::Pal => 4_433_618.75,
        };
        4.0 * fsc * upsample.max(1) as f64
    }

    /// 書き出したサンプル数
    pub fn samples(&self) -> u64 {
        self.samples
    }

    pub fn fields(&self) -> u64 {
        self.fields
    }

    /// アップサンプルによる遅れ [出力サンプル] (ストリームの先頭がこれだけずれる)
    pub fn latency(&self) -> usize {
        if self.opts.upsample > 1 { self.up.delay() * self.opts.upsample } else { 0 }
    }

    /// 1 フィールドぶん書き出す。フィールドは通算番号の順に渡すこと
    pub fn write_field(&mut self, field: &Field) -> io::Result<()> {
        assert!(std::ptr::eq(field.std, self.std), "テレビ方式が途中で変わった");
        let n_lines = self.std.stream_lines(field.parity).min(field.lines());
        let ls = self.std.line_samples;
        let center = 0.5 * (self.opts.ire_lo + self.opts.ire_hi);
        self.line.clear();
        for l in 0..n_lines {
            let y = field.line(l);
            let comp = field.component_lines(l);
            let diff = match self.channel {
                RawChannel::Chroma => field.chroma_line(l),
                RawChannel::Pb => comp.map(|c| c.0),
                RawChannel::Pr => comp.map(|c| c.1),
                _ => None,
            };
            match (self.channel, field.chroma_line(l)) {
                (RawChannel::Composite, Some(c)) => self.line.extend(y.iter().zip(c).map(|(y, c)| y + c)),
                (RawChannel::Composite | RawChannel::Luma, _) => self.line.extend_from_slice(y),
                _ => match diff {
                    Some(c) => self.line.extend(c.iter().map(|c| c + center)),
                    None => self.line.extend(std::iter::repeat_n(center, ls)),
                },
            }
        }
        self.up.process(&self.line, &mut self.upsampled);
        self.quantize();
        self.out.write_all(&self.bytes)?;
        self.samples += self.upsampled.len() as u64;
        self.fields += 1;
        Ok(())
    }

    fn quantize(&mut self) {
        let (lo, hi) = (self.opts.ire_lo, self.opts.ire_hi);
        let scale = 1.0 / (hi - lo).max(1e-6);
        let norm = |v: f32| ((v - lo) * scale).clamp(0.0, 1.0);
        self.bytes.clear();
        match self.opts.format {
            SampleFormat::U8 => self.bytes.extend(self.upsampled.iter().map(|&v| (norm(v) * 255.0).round() as u8)),
            SampleFormat::S8 => {
                self.bytes.extend(self.upsampled.iter().map(|&v| ((norm(v) * 255.0).round() as i16 - 128) as i8 as u8))
            }
            SampleFormat::S16Le => {
                for &v in &self.upsampled {
                    let s = ((norm(v) * 65535.0).round() as i32 - 32768) as i16;
                    self.bytes.extend_from_slice(&s.to_le_bytes());
                }
            }
        }
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    pub fn into_inner(self) -> W {
        self.out
    }
}

/// 書き出しの設定を人が読めるように並べる (サイドカーの .txt やログ用)
pub fn describe(std: &Standard, opts: &RawOptions, rate: f64) -> String {
    let fmt = match opts.format {
        SampleFormat::U8 => "unsigned 8 bit",
        SampleFormat::S8 => "signed 8 bit",
        SampleFormat::S16Le => "signed 16 bit little endian",
    };
    let max = match opts.format {
        SampleFormat::U8 | SampleFormat::S8 => 255.0,
        SampleFormat::S16Le => 65535.0,
    };
    let code = |ire: f32| ((ire - opts.ire_lo) / (opts.ire_hi - opts.ire_lo)).clamp(0.0, 1.0) * max;
    let offset = if opts.format == SampleFormat::U8 { 0.0 } else { -(max + 1.0) / 2.0 };
    let fsv = |ire: f32| (ire - opts.ire_lo) / (opts.ire_hi - opts.ire_lo);
    format!(
        "{std:?} composite raw\n\
         sample rate: {rate:.3} Hz (4fsc x {up})\n\
         format: {fmt}, no header\n\
         mapping: {lo:.1} IRE -> min code, {hi:.1} IRE -> max code\n\
         sync {sync:.0} IRE = code {cs:.0}, blank = {cb:.0}, white = {cw:.0}\n\
         sync..white = {span:.3} of DAC full scale (1.0 Vpp needs full scale = {need:.2} V into 75 ohm)\n",
        std = std.kind,
        up = opts.upsample,
        lo = opts.ire_lo,
        hi = opts.ire_hi,
        sync = std.ire.sync,
        cs = code(std.ire.sync) + offset,
        cb = code(std.ire.blank) + offset,
        cw = code(std.ire.white) + offset,
        span = fsv(std.ire.white) - fsv(std.ire.sync),
        need = 1.0 / (fsv(std.ire.white) - fsv(std.ire.sync)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Params, Pipeline, pattern};

    fn clean_params() -> Params {
        let mut p = Params::default();
        p.tape.enabled = false;
        p.rf.enabled = false;
        p.capture.enabled = false;
        p.camera.enabled = false;
        p.encoder.enabled = true;
        p
    }

    fn render(fields: usize, opts: impl Fn(&'static Standard) -> RawOptions) -> (Vec<u8>, &'static Standard, RawOptions) {
        let params = clean_params();
        let std = params.tv();
        let input = pattern::color_bars(std);
        let mut pipe = Pipeline::new(params);
        let o = opts(std);
        let mut w = RawWriter::new(Vec::new(), std, o, RawChannel::Composite);
        for _ in 0..fields {
            pipe.process_field(&input);
            w.write_field(pipe.composite()).unwrap();
        }
        (w.into_inner(), std, o)
    }

    fn decode_s16(b: &[u8], o: &RawOptions) -> Vec<f32> {
        b.chunks_exact(2)
            .map(|c| {
                let s = i16::from_le_bytes([c[0], c[1]]) as f32 + 32768.0;
                o.ire_lo + s / 65535.0 * (o.ire_hi - o.ire_lo)
            })
            .collect()
    }

    /// 4 フィールド (2 フレーム) をつなげると 1050 ライン ちょうどになり、全ラインの先頭に同期パルスがある
    #[test]
    fn stream_is_continuous_with_sync_every_line() {
        let (b, std, o) = render(4, |s| RawOptions::new(s, SampleFormat::S16Le, 1));
        let v = decode_s16(&b, &o);
        let ls = std.line_samples;
        assert_eq!(v.len(), 2 * std.frame_total_lines * ls);
        for (k, line) in v.chunks_exact(ls).enumerate() {
            // ライン先頭 (水平同期・等化パルス・幅広パルスはどれもここで下がっている)
            assert!((line[8] - std.ire.sync).abs() < 2.0, "line {k}: {}", line[8]);
        }
    }

    /// アップサンプルしても直流のレベル (sync 先端) と長さが保たれる
    #[test]
    fn upsample_keeps_levels() {
        let (b, std, o) = render(2, |s| RawOptions::new(s, SampleFormat::S16Le, 4));
        let v = decode_s16(&b, &o);
        let ls = std.line_samples * 4;
        assert_eq!(v.len(), std.frame_total_lines * ls);
        let lat = 4 * (Upsampler::TAPS / 2 - 1);
        for k in 1..std.frame_total_lines - 1 {
            let x = v[k * ls + lat + 8 * 4];
            assert!((x - std.ire.sync).abs() < 2.0, "line {k}: {x}");
        }
    }

    #[test]
    fn u8_and_s8_mapping() {
        let (u, std, o) = render(1, |s| RawOptions::new(s, SampleFormat::U8, 1));
        let (s, _, _) = render(1, |s| RawOptions::new(s, SampleFormat::S8, 1));
        assert_eq!(u.len(), s.len());
        assert!(u.iter().zip(&s).all(|(&u, &s)| (s as i8) as i16 == u as i16 - 128));
        let expect = ((std.ire.sync - o.ire_lo) / (o.ire_hi - o.ire_lo) * 255.0).round() as i32;
        assert!((u[8] as i32 - expect).abs() <= 2, "{} vs {expect}", u[8]);
    }

    #[test]
    fn full_scale_volts_gives_1vpp() {
        let std = Params::default().tv();
        let o = RawOptions::new(std, SampleFormat::U8, 1).with_full_scale_volts(std, 1.4);
        let span = (std.ire.white - std.ire.sync) / (o.ire_hi - o.ire_lo) * 1.4;
        assert!((span - 1.0).abs() < 1e-4);
        assert!(!o.clips_white(std));
        assert!(RawOptions::new(std, SampleFormat::U8, 1).with_full_scale_volts(std, 0.7).clips_white(std));
    }
}
