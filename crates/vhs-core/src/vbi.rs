//! 垂直帰線区間 (VBI) に重畳するデータ・試験信号の波形と、受像機側の字幕のスライス。
//!
//! 送出側 ([`render_line`]): ライン番号ごとに次の信号をブランキングレベルの上に描く。
//! - クローズドキャプション (EIA-608): NTSC ライン 21 (第 1 フィールド, CC1) / 284 (第 2 フィールド)。
//!   503 kHz (32 fH) のクロックランイン 7 周期 → スタートビット (0, 0, 1) → 7 ビット + 奇数パリティ × 2 バイト。
//!   0 / 50 IRE の NRZ
//! - VITC (SMPTE 12M): NTSC ライン 14/16/277/279、PAL 19/21/332/334。115 fH (PAL 116 fH) の NRZ で
//!   (同期ビット 1,0 + 8 ビット) × 9 群 = 90 ビット。最後の群は CRC (x⁸ + 1)
//! - VITS (挿入試験信号): 白バー・2T パルス・変調パルス・変調階段波 (NTSC 17/280、PAL 17/330) と
//!   マルチバースト (NTSC 18/281、PAL 18/331)
//! - 文字多重: PAL は WST テレテキスト (6.9375 Mbit/s、444 fH)、NTSC は NABTS か JTES (どちらも 5.7272 Mbit/s、364 fH)。
//!   JTES の物理層・データラインは `jtes` クレート
//! - 識別信号: PAL は WSS (ライン 23 前半、5 MHz の 137 エレメント、バイフェーズ)、
//!   NTSC は IEC 61880 / CGMS-A (ライン 20/283、fsc/8 の 20 ビット + CRC)
//!
//! 受像機側 ([`slice_caption`]): 垂直同期から数えたライン 21 / 284 をスライスし、ランインで
//! ビットの位相を合わせて 2 バイトを取り出す。字幕の組み立てと表示は [`crate::caption`]。

use crate::caption;
use crate::params::{TeletextSystem, VbiParams};
use crate::raster::LineInfo;
use crate::timing::{SIN4, Standard, TvStandard};

#[inline]
fn us(s: &Standard, t: f32) -> f32 {
    t * 1e-6 * s.fs
}

/// NRZ のビット列を立ち上がり時間 `rise` [サンプル] の余弦の縁で描いて `out` に足す
fn nrz(out: &mut [f32], bits: &[bool], t0: f32, period: f32, rise: f32, level: f32) {
    let len = bits.len() as isize;
    let bit_at = |k: isize| k >= 0 && k < len && bits[k as usize];
    let rise = rise.max(0.5);
    let lo = (t0 - rise).floor().max(0.0) as usize;
    let hi = ((t0 + bits.len() as f32 * period + rise).ceil() as usize).min(out.len());
    for n in lo..hi {
        let t = n as f32 - t0;
        let k = (t / period).floor() as isize;
        let pos = t - k as f32 * period;
        let (d, a, b) = if pos < 0.5 * period {
            (pos, bit_at(k - 1), bit_at(k))
        } else {
            (pos - period, bit_at(k), bit_at(k + 1))
        };
        let w = if d <= -0.5 * rise {
            0.0
        } else if d >= 0.5 * rise {
            1.0
        } else {
            0.5 - 0.5 * (std::f32::consts::PI * (d + 0.5 * rise) / rise).cos()
        };
        let (a, b) = (a as u8 as f32, b as u8 as f32);
        out[n] += level * (a + (b - a) * w);
    }
}

/// バイト列を LSB から順にビット列にする
fn bytes_lsb(bytes: &[u8]) -> Vec<bool> {
    bytes.iter().flat_map(|&b| (0..8).map(move |i| b >> i & 1 == 1)).collect()
}

/// 7 ビットの文字に奇数パリティを付ける
pub fn odd_parity(c: u8) -> u8 {
    let c = c & 0x7f;
    if c.count_ones() % 2 == 0 { c | 0x80 } else { c }
}

/// ハミング 8/4 (テレテキストのアドレス・制御バイト)
fn hamming84(d: u8) -> u8 {
    let bit = |i: u8| d >> i & 1;
    let (d1, d2, d3, d4) = (bit(0), bit(1), bit(2), bit(3));
    let p1 = 1 ^ d1 ^ d3 ^ d4;
    let p2 = 1 ^ d1 ^ d2 ^ d4;
    let p3 = 1 ^ d1 ^ d2 ^ d3;
    let p4 = 1 ^ p1 ^ d1 ^ p2 ^ d2 ^ p3 ^ d3 ^ d4;
    p1 | d1 << 1 | p2 << 2 | d2 << 3 | p3 << 4 | d3 << 5 | p4 << 6 | d4 << 7
}

/// 決定的な擬似乱数 (データ内容用)
fn hash(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^ (x >> 33)
}

/// このラインに載せる VBI 信号の種類
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VbiService {
    Caption,
    Vitc,
    VitsComposite,
    VitsMultiburst,
    Teletext,
    Wss,
}

/// ライン番号ごとの割り当て (使わない設定のものは None)
pub fn service(s: &Standard, n: usize, p: &VbiParams) -> Option<VbiService> {
    use VbiService::*;
    if !p.enabled {
        return None;
    }
    let ntsc = s.kind == TvStandard::Ntsc;
    let fixed = if ntsc {
        match n {
            21 | 284 if p.closed_caption => Some(Caption),
            14 | 16 | 277 | 279 if p.vitc => Some(Vitc),
            17 | 280 if p.vits => Some(VitsComposite),
            18 | 281 if p.vits => Some(VitsMultiburst),
            20 | 283 if p.wss => Some(Wss),
            _ => None,
        }
    } else {
        match n {
            19 | 21 | 332 | 334 if p.vitc => Some(Vitc),
            17 | 330 if p.vits => Some(VitsComposite),
            18 | 331 if p.vits => Some(VitsMultiburst),
            23 if p.wss => Some(Wss),
            _ => None,
        }
    };
    if fixed.is_some() || !p.teletext {
        return fixed;
    }
    // 文字多重は空いているラインに先頭から割り当てる
    if ntsc && p.teletext_system == TeletextSystem::Jtes {
        // 省令第77号で使えるライン。第 1 フィールドの n H を使うときは第 2 フィールドの (n + 263) H も使うので、
        // 両方が空いている対だけを割り当てる
        let i = JTES_LINES.iter().find_map(|f| f.iter().position(|&m| m == n))?;
        let free = |k: usize| JTES_LINES.iter().all(|f| service_fixed_free(s, f[k], p));
        let used = (0..i).filter(|&k| free(k)).count();
        return (free(i) && used < p.teletext_lines as usize).then_some(Teletext);
    }
    let candidates: [&[usize]; 2] = if ntsc {
        [&NABTS_LINES[0], &NABTS_LINES[1]]
    } else {
        [&WST_LINES[0], &WST_LINES[1]]
    };
    let range = candidates.iter().find(|r| r.contains(&n))?;
    let mut used = 0;
    for &m in range.iter() {
        if service_fixed_free(s, m, p) {
            if m == n {
                return (used < p.teletext_lines as usize).then_some(Teletext);
            }
            used += 1;
        }
    }
    None
}

const fn range_lines<const N: usize>(first: usize) -> [usize; N] {
    let mut a = [0; N];
    let mut i = 0;
    while i < N {
        a[i] = first + i;
        i += 1;
    }
    a
}
const NABTS_LINES: [[usize; 11]; 2] = [range_lines(10), range_lines(273)];
const WST_LINES: [[usize; 16]; 2] = [range_lines(7), range_lines(320)];
/// JTES の重畳ライン (第 1 フィールド, 第 2 フィールド)。同じ添字どうしが n H と (n + 263) H の対
pub const JTES_LINES: [[usize; 4]; 2] = [[14, 15, 16, 21], [277, 278, 279, 284]];

fn service_fixed_free(s: &Standard, n: usize, p: &VbiParams) -> bool {
    let q = VbiParams { teletext: false, ..p.clone() };
    service(s, n, &q).is_none()
}

/// VBI のラインに信号を描く。`y` はブランキングを 0 とした輝度 (送出側の帯域制限を通す)、
/// `c` は色副搬送波 (`base` はラインの副搬送波位相)。何か描いたら true
pub fn render_line(s: &Standard, info: &LineInfo, p: &VbiParams, base: usize, y: &mut [f32], c: &mut [f32]) -> bool {
    let Some(svc) = service(s, info.number, p) else { return false };
    let second_field = info.number > s.frame_total_lines / 2 + 1;
    match svc {
        VbiService::Caption => {
            let pair = caption_pair(p, info.frame, second_field);
            render_caption(s, pair, y);
        }
        VbiService::Vitc => render_vitc(s, info.frame, second_field, y),
        VbiService::VitsComposite => render_vits_composite(s, base, y, c),
        VbiService::VitsMultiburst => render_multiburst(s, y),
        VbiService::Teletext => render_teletext(s, info, p, y),
        VbiService::Wss => render_wss(s, y),
    }
    true
}

// ---------------------------------------------------------------- クローズドキャプション

/// 送る文を行に分ける (改行か、1 行の入力欄で書く `\n` の 2 文字)
pub fn text_lines(text: &str) -> Vec<String> {
    text.replace("\\n", "\n").lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

/// フレーム番号・フィールドに対応する字幕の 2 バイト (パリティ付き)。選んだチャンネルのフィールドでなければヌル
fn caption_pair(p: &VbiParams, frame: u64, second_field: bool) -> [u8; 2] {
    let ch = p.caption_channel.clamp(1, 4);
    if (ch >= 3) != second_field {
        return [0x80, 0x80];
    }
    let seq = caption::sequence(&text_lines(&p.caption_text), p.caption_mode, ch);
    let [a, b] = seq[(frame % seq.len() as u64) as usize];
    [odd_parity(a), odd_parity(b)]
}

/// ライン 21 の波形: ランイン → スタートビット → 2 バイト
fn render_caption(s: &Standard, pair: [u8; 2], y: &mut [f32]) {
    let tb = s.line_samples as f32 / 32.0;
    let t_ri = us(s, 10.5);
    let level = 50.0;
    for (n, v) in y.iter_mut().enumerate() {
        let t = n as f32 - t_ri;
        if (0.0..7.0 * tb).contains(&t) {
            *v += 0.5 * level * (1.0 - (std::f32::consts::TAU * t / tb).cos());
        }
    }
    let mut bits = vec![false, false, true];
    bits.extend(bytes_lsb(&pair));
    nrz(y, &bits, t_ri + 7.0 * tb, tb, us(s, 0.24), level);
}

/// AFC 後の輝度ライン (IRE) からライン 21 の 2 バイト (パリティ付き) を取り出す。ランインがなければ None
pub fn slice_caption(s: &Standard, line: &[f32]) -> Option<[u8; 2]> {
    use std::f32::consts::TAU;
    let tb = s.line_samples as f32 / 32.0;
    let t_ri = us(s, 10.5);
    let lo = t_ri.max(0.0) as usize;
    let hi = ((t_ri + 7.0 * tb) as usize).min(line.len());
    if hi <= lo {
        return None;
    }
    // ランインの平均 (スライスレベル) と位相
    let mean = line[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
    let (mut zr, mut zi) = (0f32, 0f32);
    for n in lo..hi {
        let th = TAU * (n as f32 - t_ri) / tb;
        let v = line[n] - mean;
        zr += v * th.cos();
        zi += v * th.sin();
    }
    let amp = 2.0 * (zr * zr + zi * zi).sqrt() / (hi - lo) as f32;
    // ランインの振幅は 25 IRE。半分もなければランインなし
    if amp < 10.0 {
        return None;
    }
    let phi = (-zi).atan2(-zr);
    let t0 = t_ri + phi / TAU * tb;
    let sample = |t: f32| {
        let i = t.floor();
        let k = i as isize;
        if k < 0 || k as usize + 1 >= line.len() {
            return 0.0;
        }
        let f = t - i;
        line[k as usize] * (1.0 - f) + line[k as usize + 1] * f
    };
    let bit = |k: usize| sample(t0 + (7.0 + k as f32 + 0.5) * tb) > mean;
    if bit(0) || bit(1) || !bit(2) {
        return None;
    }
    let byte = |b: usize| (0..8).fold(0u8, |acc, i| acc | (bit(3 + 8 * b + i) as u8) << i);
    Some([byte(0), byte(1)])
}

// ---------------------------------------------------------------- VITC

/// フレーム番号 → (時, 分, 秒, フレーム)。NTSC はドロップフレーム
fn timecode(s: &Standard, frame: u64) -> (u64, u64, u64, u64, bool) {
    match s.kind {
        TvStandard::Ntsc => {
            let d = frame / 17982;
            let m = frame % 17982;
            let adj = if m < 2 { 18 * d } else { 18 * d + 2 * ((m - 2) / 1798) };
            let n = frame + adj;
            (n / 108000 % 24, n / 1800 % 60, n / 30 % 60, n % 30, true)
        }
        TvStandard::Pal => (frame / 90000 % 24, frame / 1500 % 60, frame / 25 % 60, frame % 25, false),
    }
}

/// VITC の 90 ビット
pub fn vitc_bits(s: &Standard, frame: u64, field_mark: bool) -> Vec<bool> {
    let (h, m, sec, f, drop) = timecode(s, frame);
    let mut w: u64 = 0;
    let mut put = |v: u64, at: u32| w |= v << at;
    put(f % 10, 0);
    put(f / 10, 8);
    put(drop as u64, 10);
    put(sec % 10, 16);
    put(sec / 10, 24);
    put(field_mark as u64, 27);
    put(m % 10, 32);
    put(m / 10, 40);
    put(h % 10, 48);
    put(h / 10, 56);
    let mut bits = Vec::with_capacity(90);
    for g in 0..8 {
        bits.extend([true, false]);
        bits.extend((0..8).map(|i| w >> (8 * g + i) & 1 == 1));
    }
    bits.extend([true, false]);
    // CRC: G(x) = x⁸ + 1 で先頭 82 ビットを割った余り
    let mut crc = 0u8;
    for &b in &bits {
        let fb = (crc >> 7) ^ b as u8;
        crc = (crc << 1) | fb;
    }
    bits.extend((0..8).rev().map(|i| crc >> i & 1 == 1));
    bits
}

fn render_vitc(s: &Standard, frame: u64, second_field: bool, y: &mut [f32]) {
    let per = if s.kind == TvStandard::Ntsc { 115.0 } else { 116.0 };
    let level = if s.kind == TvStandard::Ntsc { 80.0 } else { 550.0 / 7.0 };
    let bits = vitc_bits(s, frame, second_field);
    nrz(y, &bits, us(s, 11.2), s.line_samples as f32 / per, us(s, 0.2), level);
}

// ---------------------------------------------------------------- VITS

/// sin² パルス (半値幅 `had` [サンプル]、中心 `t0`)
fn sin2(t: f32, t0: f32, had: f32) -> f32 {
    let x = (t - t0) / (2.0 * had);
    if x.abs() >= 0.5 { 0.0 } else { (std::f32::consts::PI * x).cos().powi(2) }
}

/// 縁を sin² (立ち上がり `rise`) にした矩形 [a, b)
fn bar(t: f32, a: f32, b: f32, rise: f32) -> f32 {
    let edge = |d: f32| {
        if d <= -0.5 * rise {
            0.0
        } else if d >= 0.5 * rise {
            1.0
        } else {
            0.5 + 0.5 * (std::f32::consts::PI * d / rise).sin()
        }
    };
    edge(t - a).min(1.0 - edge(t - b))
}

/// 合成試験信号: 白バー・2T パルス・変調パルス (NTSC 12.5T / PAL 20T)・変調階段波
fn render_vits_composite(s: &Standard, base: usize, y: &mut [f32], c: &mut [f32]) {
    let ntsc = s.kind == TvStandard::Ntsc;
    // T = 1 / (2 × 公称帯域): NTSC 125 ns、PAL 100 ns
    let t_unit = if ntsc { us(s, 0.125) } else { us(s, 0.1) };
    let (bar_a, bar_b) = if ntsc { (us(s, 12.0), us(s, 30.0)) } else { (us(s, 12.0), us(s, 22.0)) };
    let p2t = if ntsc { us(s, 34.0) } else { us(s, 26.0) };
    let (pmod, mod_t) = if ntsc { (us(s, 37.0), 12.5) } else { (us(s, 30.0), 20.0) };
    let (st_a, st_b) = (us(s, 42.0), us(s, 60.0));
    let rise = 2.0 * t_unit;
    for n in 0..y.len() {
        let t = n as f32;
        let k = (n + base) & 3;
        let mut v = 100.0 * bar(t, bar_a, bar_b, rise);
        v += 100.0 * sin2(t, p2t, 2.0 * t_unit);
        let env = sin2(t, pmod, mod_t * t_unit);
        v += 50.0 * env;
        let mut ch = 50.0 * env;
        // 6 段の階段波 (0〜90 IRE、PAL は 0〜100%) に ±20 IRE の色副搬送波
        let st = bar(t, st_a, st_b, rise);
        if st > 0.0 {
            let step = (((t - st_a) / ((st_b - st_a) / 6.0)).floor()).clamp(0.0, 5.0);
            let top = if ntsc { 90.0 } else { 100.0 };
            v += st * step * top / 5.0;
            ch += 20.0 * st;
        }
        y[n] += v;
        // バーストと同じ位相 (-U 軸)
        c[n] -= ch * SIN4[k];
    }
}

/// マルチバースト: 白フラグと 6 つの周波数の正弦波パケット
fn render_multiburst(s: &Standard, y: &mut [f32]) {
    let freqs: [f32; 6] =
        if s.kind == TvStandard::Ntsc { [0.5, 1.25, 2.0, 3.0, 3.58, 4.2] } else { [0.5, 1.0, 2.0, 4.0, 4.8, 5.8] };
    let rise = us(s, 0.25);
    for n in 0..y.len() {
        let t = n as f32;
        let mut v = 70.0 * bar(t, us(s, 12.0), us(s, 16.0), rise);
        for (i, f) in freqs.iter().enumerate() {
            let a = us(s, 18.0 + 6.5 * i as f32);
            let b = a + us(s, 5.5);
            let w = bar(t, a, b, rise);
            if w > 0.0 {
                let ph = std::f32::consts::TAU * f * 1e6 * (t - a) / s.fs;
                v += w * (40.0 - 30.0 * ph.cos());
            }
        }
        y[n] += v;
    }
}

// ---------------------------------------------------------------- 文字多重

fn render_teletext(s: &Standard, info: &LineInfo, p: &VbiParams, y: &mut [f32]) {
    let lines = p.teletext_lines.max(1) as u64;
    if s.kind == TvStandard::Pal {
        // WST: ランイン 0x55 0x55、フレーミングコード 0x27、マガジン 1 のページ 100 を 1 行ずつ送る
        let slot = info.stream % lines;
        let packet = (info.stream / s.frame_total_lines as u64 * 2 + (info.number > 313) as u64) * lines + slot;
        let row = (packet % 24) as u8;
        let mut bytes = vec![0x55, 0x55, 0x27];
        bytes.push(hamming84(1 | (row & 1) << 3));
        bytes.push(hamming84(row >> 1));
        let text: String = if row == 0 {
            let (h, m, sec, _, _) = timecode(s, info.frame);
            bytes.extend([0u8, 0, 0, 0, 0, 0, 0, 0].iter().map(|&d| hamming84(d)));
            format!("P100 VHS NEXT TEXT  {h:02}:{m:02}:{sec:02}  ")
        } else {
            let src = text_lines(&p.caption_text);
            let l = src.get((row as usize - 1) % src.len().max(1)).map(String::as_str).unwrap_or("");
            format!(" {row:02} {l}")
        };
        let width = if row == 0 { 32 } else { 40 };
        let mut t: Vec<u8> = text.bytes().map(|b| if b.is_ascii() { b } else { b'?' }).take(width).collect();
        t.resize(width, b' ');
        bytes.extend(t.into_iter().map(odd_parity));
        let bits = bytes_lsb(&bytes);
        let period = s.line_samples as f32 / 444.0;
        nrz(y, &bits, us(s, 10.3), period, period, 462.0 / 7.0);
    } else if p.teletext_system == TeletextSystem::Jtes {
        let packet = jtes_packet(s, info);
        jtes::wave::render(y, &packet, &jtes_format(s));
    } else {
        // NABTS: ランイン、フレーミングコード 0xE7、33 バイトのパケット (内容は擬似乱数)
        let mut bytes = vec![0x55, 0x55, 0xE7];
        let h = hash(info.stream ^ 0x4e41_4254_5300);
        bytes.extend((0..33).map(|i| (hash(h.wrapping_add(i)) & 0xff) as u8));
        let bits = bytes_lsb(&bytes);
        let period = s.line_samples as f32 / 364.0;
        nrz(y, &bits, us(s, 10.5), period, period, 70.0);
    }
}

/// JTES のライン形式 (ペデスタル = 0 IRE、白 = 100 IRE。日本の NTSC はセットアップなし)
pub fn jtes_format(s: &Standard) -> jtes::LineFormat {
    jtes::LineFormat::for_line(s.line_samples as f32, 100.0)
}

/// このラインで送る JTES のパケット。
/// TODO: データグループ・番組データの層ができるまでは、送出第1モードの空パケット (全 NUL) を送る。
/// 連続性指標はラインごとに、フィールドが進むたびに 1 ずつ巡回させる
fn jtes_packet(s: &Standard, info: &LineInfo) -> jtes::Packet {
    let second_field = info.number > s.frame_total_lines / 2 + 1;
    let field = info.frame * 2 + second_field as u64;
    let mut p = jtes::Packet::empty(jtes::Service::Mode1, (field % 16) as u8);
    p.transmission_start = field == 0;
    p
}

/// AFC 後の輝度ライン (IRE) から JTES のパケットを取り出して誤り訂正する。(パケット, 訂正したビット数)
pub fn slice_jtes(s: &Standard, line: &[f32]) -> Option<(jtes::Packet, usize)> {
    jtes::wave::receive(line, &jtes_format(s))
}

// ---------------------------------------------------------------- 識別信号

/// CRC-6 (x⁶ + x + 1、初期値 1 を 6 つ) — IEC 61880
fn crc6(bits: &[bool]) -> [bool; 6] {
    let mut r = [true; 6];
    for &b in bits {
        let fb = b ^ r[5];
        for i in (1..6).rev() {
            r[i] = r[i - 1];
        }
        r[0] = fb;
        r[1] ^= fb;
    }
    let mut out = [false; 6];
    for i in 0..6 {
        out[i] = r[5 - i];
    }
    out
}

fn render_wss(s: &Standard, y: &mut [f32]) {
    if s.kind == TvStandard::Pal {
        // ランイン 29 エレメント、スタートコード 24 エレメント、14 ビット × 6 エレメント (バイフェーズ)
        let mut el: Vec<bool> = (0..29).rev().map(|i| 0x1F1C_71C7u32 >> i & 1 == 1).collect();
        el.extend((0..24).rev().map(|i| 0x1E_3C1Fu32 >> i & 1 == 1));
        // 4:3 フル (アスペクト群 0001)、それ以外は 0
        let data: u16 = 0b1000;
        for i in 0..14 {
            let b = data >> i & 1 == 1;
            el.extend(if b { [true, true, true, false, false, false] } else { [false, false, false, true, true, true] });
        }
        nrz(y, &el, us(s, 11.0), s.fs / 5e6, 1.5, 500.0 / 7.0);
    } else {
        // 基準ビット 1,0 → 20 ビット (アスペクト 2、ワード 1 4、CGMS 等 8、CRC 6)
        let data = [false; 14];
        let mut bits = vec![true, false];
        bits.extend(data);
        bits.extend(crc6(&data));
        nrz(y, &bits, us(s, 11.2), 8.0 * s.fs / (4.0 * s.fsc), 4.0, 70.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::line_info;
    use crate::timing::{NTSC, NTSC_228, PAL};

    #[test]
    fn caption_slices_back() {
        let s = &NTSC;
        for pair in [[0x94, 0x25], [odd_parity(b'H'), odd_parity(b'i')], [0x80, 0x80]] {
            let mut y = vec![0f32; s.line_samples];
            render_caption(s, pair, &mut y);
            assert_eq!(slice_caption(s, &y), Some(pair));
        }
        // ランインのないライン
        assert_eq!(slice_caption(s, &vec![0f32; s.line_samples]), None);
    }

    #[test]
    fn line_assignment_does_not_collide() {
        let p = VbiParams { vitc: true, teletext: true, wss: true, teletext_lines: 20, ..VbiParams::default() };
        for s in [&NTSC, &NTSC_228, &PAL] {
            let mut tele = 0;
            for n in 1..=s.frame_total_lines {
                if service(s, n, &p) == Some(VbiService::Teletext) {
                    tele += 1;
                    let info = line_info(s, (n - 1) as u64);
                    assert_eq!(info.video, crate::raster::Video::None, "{:?} line {n}", s.kind);
                }
            }
            assert!(tele > 0);
        }
    }

    #[test]
    fn jtes_uses_allowed_free_lines() {
        let jt = |vitc: bool, cc: bool| VbiParams {
            teletext: true,
            teletext_system: TeletextSystem::Jtes,
            teletext_lines: 8,
            vitc,
            closed_caption: cc,
            ..VbiParams::default()
        };
        let lines = |p: &VbiParams| -> Vec<usize> {
            (1..=NTSC.frame_total_lines).filter(|&n| service(&NTSC, n, p) == Some(VbiService::Teletext)).collect()
        };
        assert_eq!(lines(&jt(false, false)), [14, 15, 16, 21, 277, 278, 279, 284]);
        // 字幕・VITC があるラインは避ける
        assert_eq!(lines(&jt(false, true)), [14, 15, 16, 277, 278, 279]);
        assert_eq!(lines(&jt(true, true)), [15, 278]);
        // 対は n と n + 263
        for k in 0..JTES_LINES[0].len() {
            assert_eq!(JTES_LINES[1][k], JTES_LINES[0][k] + 263);
        }
        // ライン数の上限は対の数で数える
        assert_eq!(lines(&VbiParams { teletext_lines: 2, ..jt(false, false) }), [14, 15, 277, 278]);
        // PAL は WST のまま
        assert!(service(&PAL, 7, &jt(false, false)) == Some(VbiService::Teletext));
    }

    #[test]
    fn jtes_line_slices_back_through_transmit_filter() {
        let p = VbiParams {
            teletext: true,
            teletext_system: TeletextSystem::Jtes,
            closed_caption: false,
            vits: false,
            ..VbiParams::default()
        };
        for s in [&NTSC, &NTSC_228] {
            for (frame, n) in [(0u64, 14usize), (3, 21), (3, 284)] {
                let sl = frame * s.frame_total_lines as u64 + (n - 1) as u64;
                let info = line_info(s, sl);
                assert_eq!(info.number, n);
                let mut y = vec![0f32; s.line_samples];
                let mut c = vec![0f32; s.line_samples];
                assert!(render_line(s, &info, &p, 0, &mut y, &mut c));
                // 送出側の輝度帯域 (4.2 MHz の 4 次バターワース) を通す
                let mut lp = crate::filter::Cascade::butter_lowpass_at(4, 4.2e6, s.fs);
                lp.process_slice(&mut y);
                let (packet, corrected) = slice_jtes(s, &y).unwrap_or_else(|| panic!("{:?} line {n}", s.kind));
                assert_eq!(packet, jtes_packet(s, &info));
                assert!(corrected <= 2, "{corrected}");
            }
        }
    }

    #[test]
    fn vitc_has_90_bits() {
        let b = vitc_bits(&NTSC, 12345, false);
        assert_eq!(b.len(), 90);
        for g in 0..9 {
            assert!(b[10 * g] && !b[10 * g + 1]);
        }
    }
}
