//! OSD マークアップの解析とレイアウト。
//!
//! ```text
//! <pos=48,40><edge=black>PLAY ▶</edge>
//! <pos=706,400><align=right><color=#ffd84a><bg=#0000c0>{MON}.{D} {Y}</bg></color>
//! ```
//!
//! # タグ
//! 囲みタグ (`</名前>` で閉じる。入れ子可):
//! - `<color=色>` 文字色
//! - `<bg=色>` 文字の背景 (帯)
//! - `<edge=色>` 縁取り
//! - `<shadow=色>` 右下の影
//! - `<size=N>` ドットの大きさ (1〜8)
//! - `<blink>` 約 1 Hz で点滅
//!
//! 単独タグ:
//! - `<pos=X,Y>` 以降の行の基準位置 (754×480 の画素座標)
//! - `<align=left|center|right>` 基準位置に対する行の揃え
//!
//! 色は `#rgb` `#rrggbb` `#rrggbbaa` か色名 (white, black, red, green, blue, yellow, cyan,
//! magenta, gray, orange, none)。
//!
//! # 置換
//! `{counter}` (H:MM:SS テープカウンタ), `{date}` (NOV.23 2026), `{time}` (14:30:00),
//! `{Y}` `{M}` `{D}` `{h}` `{m}` `{s}` `{MON}` `{h12}` `{ampm}` `{f}` (フレーム 00〜29)。
//! 時刻は `start_datetime` から経過フィールド数だけ進む。
//!
//! `\<` `\{` `\\` で記号そのものを書ける。解釈できないタグや置換はそのまま表示する。

use super::font::{GLYPH_H, GLYPH_W};
use crate::timing::FIELD_RATE;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }
}

/// 色を解釈する。`Some(None)` は "none" (描かない)。
pub fn parse_color(s: &str) -> Option<Option<Rgba>> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        let nib = |c: u8| (c as char).to_digit(16).map(|v| v as u8);
        let b = hex.as_bytes();
        let byte = |i: usize| Some(nib(b[i])? * 16 + nib(b[i + 1])?);
        return match b.len() {
            3 => {
                let (r, g, bl) = (nib(b[0])?, nib(b[1])?, nib(b[2])?);
                Some(Some(Rgba::rgb8(r * 17, g * 17, bl * 17)))
            }
            6 => Some(Some(Rgba::rgb8(byte(0)?, byte(2)?, byte(4)?))),
            8 => {
                let mut c = Rgba::rgb8(byte(0)?, byte(2)?, byte(4)?);
                c.a = byte(6)? as f32 / 255.0;
                Some(Some(c))
            }
            _ => None,
        };
    }
    let (r, g, b) = match s.as_str() {
        "none" | "transparent" | "off" => return Some(None),
        "white" => (255, 255, 255),
        "black" => (0, 0, 0),
        "red" => (255, 0, 0),
        "green" => (0, 255, 0),
        "blue" => (0, 0, 255),
        "yellow" => (255, 255, 0),
        "cyan" => (0, 255, 255),
        "magenta" => (255, 0, 255),
        "gray" | "grey" => (128, 128, 128),
        "orange" => (255, 160, 0),
        _ => return None,
    };
    Some(Some(Rgba::rgb8(r, g, b)))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub color: Rgba,
    pub bg: Option<Rgba>,
    pub edge: Option<Rgba>,
    pub shadow: Option<Rgba>,
    pub blink: bool,
    pub size: u32,
}

/// 配置済みの 1 文字 (x, y はセル左上の画素座標)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub x: i32,
    pub y: i32,
    pub ch: char,
    pub style: Style,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Align {
    Left,
    Center,
    Right,
}

/// 置換に使う時刻情報
pub struct Context {
    /// 経過時間 [s]
    elapsed: f64,
    /// 現在時刻 (Unix 秒相当, タイムゾーンなし)
    now: i64,
}

impl Context {
    pub fn new(start_datetime: &str, field_index: u64) -> Self {
        let elapsed = field_index as f64 / FIELD_RATE as f64;
        let start = parse_datetime(start_datetime).unwrap_or(946_684_800); // 2000-01-01
        Self { elapsed, now: start + elapsed as i64 }
    }

    pub fn blink_on(&self) -> bool {
        (self.elapsed * 2.0) as u64 % 2 == 0
    }

    fn expand(&self, key: &str) -> Option<String> {
        const MON: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];
        let days = self.now.div_euclid(86_400);
        let sod = self.now.rem_euclid(86_400);
        let (y, mo, d) = civil_from_days(days);
        let (h, mi, s) = (sod / 3600, sod / 60 % 60, sod % 60);
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        let el = self.elapsed as u64;
        Some(match key {
            "counter" => format!("{}:{:02}:{:02}", el / 3600, el / 60 % 60, el % 60),
            "date" => format!("{}.{} {}", MON[mo as usize - 1], d, y),
            "time" => format!("{h:02}:{mi:02}:{s:02}"),
            "Y" => y.to_string(),
            "M" => format!("{mo:02}"),
            "D" => format!("{d:02}"),
            "h" => format!("{h:02}"),
            "m" => format!("{mi:02}"),
            "s" => format!("{s:02}"),
            "MON" => MON[mo as usize - 1].to_string(),
            "h12" => h12.to_string(),
            "ampm" => (if h < 12 { "AM" } else { "PM" }).to_string(),
            "f" => format!("{:02}", ((self.elapsed.fract()) * 30.0) as u32),
            _ => return None,
        })
    }
}

/// "YYYY-MM-DD HH:MM[:SS]" (区切りは - / . と空白・T を許容) を Unix 秒相当に変換する
pub fn parse_datetime(s: &str) -> Option<i64> {
    let nums: Vec<i64> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse().ok())
        .collect::<Option<_>>()?;
    if nums.len() < 3 {
        return None;
    }
    let (y, mo, d) = (nums[0], nums[1], nums[2]);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let h = nums.get(3).copied().unwrap_or(0).clamp(0, 23);
    let mi = nums.get(4).copied().unwrap_or(0).clamp(0, 59);
    let sec = nums.get(5).copied().unwrap_or(0).clamp(0, 59);
    Some(days_from_civil(y, mo as u32, d as u32) * 86_400 + h * 3600 + mi * 60 + sec)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + (m <= 2) as i64;
    (y, m, d)
}

/// レイアウト中の 1 行
struct Line {
    x: i32,
    y: i32,
    align: Align,
    chars: Vec<(char, Style)>,
}

struct Layout {
    out: Vec<Glyph>,
    line: Line,
}

impl Layout {
    fn advance(size: u32) -> i32 {
        (GLYPH_W as i32 + 1) * 2 * size as i32
    }

    fn line_height(size: u32) -> i32 {
        (GLYPH_H as i32 + 3) * 2 * size as i32
    }

    /// 現在の行を確定して配置する
    fn flush(&mut self) {
        let line = &mut self.line;
        let width: i32 = line.chars.iter().map(|(_, s)| Self::advance(s.size)).sum::<i32>()
            - line.chars.last().map_or(0, |(_, s)| 2 * s.size as i32);
        let mut x = match line.align {
            Align::Left => line.x,
            Align::Center => line.x - width / 2,
            Align::Right => line.x - width,
        };
        // 大きさの違う文字はベースライン (下端) を揃える
        let max_h = line.chars.iter().map(|(_, s)| s.size).max().unwrap_or(0) as i32 * 2 * GLYPH_H as i32;
        for (ch, style) in line.chars.drain(..) {
            let h = style.size as i32 * 2 * GLYPH_H as i32;
            self.out.push(Glyph { x, y: line.y + max_h - h, ch, style });
            x += Self::advance(style.size);
        }
    }

    fn newline(&mut self, cur_size: u32) {
        let size = self.line.chars.iter().map(|(_, s)| s.size).max().unwrap_or(cur_size);
        self.flush();
        self.line.y += Self::line_height(size);
    }
}

/// マークアップを解釈して配置済みの文字列を返す
pub fn layout(src: &str, default_size: u32, ctx: &Context) -> Vec<Glyph> {
    let base = Style { color: Rgba::WHITE, bg: None, edge: None, shadow: None, blink: false, size: default_size };
    let mut colors = vec![base.color];
    let mut bgs = vec![base.bg];
    let mut edges = vec![base.edge];
    let mut shadows = vec![base.shadow];
    let mut sizes = vec![base.size];
    let mut blink = 0u32;
    let mut lay = Layout { out: Vec::new(), line: Line { x: 48, y: 40, align: Align::Left, chars: Vec::new() } };

    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let style = Style {
            color: *colors.last().unwrap(),
            bg: *bgs.last().unwrap(),
            edge: *edges.last().unwrap(),
            shadow: *shadows.last().unwrap(),
            blink: blink > 0,
            size: *sizes.last().unwrap(),
        };
        let c = chars[i];
        match c {
            '\r' => {}
            '\n' => lay.newline(style.size),
            '\\' if i + 1 < chars.len() && matches!(chars[i + 1], '<' | '{' | '\\') => {
                lay.line.chars.push((chars[i + 1], style));
                i += 1;
            }
            '<' | '{' => {
                let close = if c == '<' { '>' } else { '}' };
                let end = chars[i + 1..].iter().position(|&x| x == close || x == '\n').map(|p| i + 1 + p);
                let inner: Option<String> =
                    end.filter(|&e| chars[e] == close).map(|e| chars[i + 1..e].iter().collect());
                let handled = match (&inner, c) {
                    (Some(tag), '<') => {
                        let tag = tag.trim();
                        if let Some(name) = tag.strip_prefix('/') {
                            fn pop<T>(v: &mut Vec<T>) {
                                if v.len() > 1 {
                                    v.pop();
                                }
                            }
                            match name.trim().to_ascii_lowercase().as_str() {
                                "color" => pop(&mut colors),
                                "bg" => pop(&mut bgs),
                                "edge" => pop(&mut edges),
                                "shadow" => pop(&mut shadows),
                                "size" => pop(&mut sizes),
                                "blink" => blink = blink.saturating_sub(1),
                                _ => {}
                            }
                            matches!(
                                name.trim().to_ascii_lowercase().as_str(),
                                "color" | "bg" | "edge" | "shadow" | "size" | "blink"
                            )
                        } else {
                            let (name, val) = match tag.split_once('=') {
                                Some((n, v)) => (n.trim().to_ascii_lowercase(), Some(v.trim())),
                                None => (tag.to_ascii_lowercase(), None),
                            };
                            match (name.as_str(), val) {
                                ("color", Some(v)) => match parse_color(v) {
                                    Some(col) => {
                                        colors.push(col.unwrap_or(Rgba { a: 0.0, ..Rgba::WHITE }));
                                        true
                                    }
                                    None => false,
                                },
                                ("bg", Some(v)) => parse_color(v).map(|c| bgs.push(c)).is_some(),
                                ("edge", Some(v)) => parse_color(v).map(|c| edges.push(c)).is_some(),
                                ("shadow", Some(v)) => parse_color(v).map(|c| shadows.push(c)).is_some(),
                                ("size", Some(v)) => match v.parse::<u32>() {
                                    Ok(n) => {
                                        sizes.push(n.clamp(1, 8));
                                        true
                                    }
                                    Err(_) => false,
                                },
                                ("blink", None) => {
                                    blink += 1;
                                    true
                                }
                                ("pos", Some(v)) => {
                                    let xy: Vec<i32> = v
                                        .split(',')
                                        .filter_map(|t| t.trim().parse::<f32>().ok().map(|f| f.round() as i32))
                                        .collect();
                                    if xy.len() == 2 {
                                        if !lay.line.chars.is_empty() {
                                            lay.flush();
                                        }
                                        lay.line.x = xy[0];
                                        lay.line.y = xy[1];
                                        true
                                    } else {
                                        false
                                    }
                                }
                                ("align", Some(v)) => {
                                    let a = match v.to_ascii_lowercase().as_str() {
                                        "left" => Some(Align::Left),
                                        "center" | "centre" => Some(Align::Center),
                                        "right" => Some(Align::Right),
                                        _ => None,
                                    };
                                    a.map(|a| lay.line.align = a).is_some()
                                }
                                _ => false,
                            }
                        }
                    }
                    (Some(key), '{') => match ctx.expand(key.trim()) {
                        Some(text) => {
                            lay.line.chars.extend(text.chars().map(|ch| (ch, style)));
                            true
                        }
                        None => false,
                    },
                    _ => false,
                };
                if handled {
                    i = end.unwrap();
                } else {
                    lay.line.chars.push((c, style));
                }
            }
            _ => lay.line.chars.push((c, style)),
        }
        i += 1;
    }
    lay.flush();
    lay.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_roundtrip() {
        let t = parse_datetime("1995-08-12 14:30:05").unwrap();
        let ctx = Context { elapsed: 0.0, now: t };
        assert_eq!(ctx.expand("date").unwrap(), "AUG.12 1995");
        assert_eq!(ctx.expand("time").unwrap(), "14:30:05");
        let ctx = Context { elapsed: 3725.5, now: parse_datetime("2000-02-29 23:59:59").unwrap() + 1 };
        assert_eq!(ctx.expand("counter").unwrap(), "1:02:05");
        assert_eq!(ctx.expand("date").unwrap(), "MAR.1 2000");
        assert_eq!(ctx.expand("ampm").unwrap(), "AM");
        assert_eq!(ctx.expand("h12").unwrap(), "12");
    }

    #[test]
    fn tags_and_layout() {
        let ctx = Context::new("2000-01-01", 0);
        let g = layout("<pos=100,50><color=red>AB</color>C\n<size=3>D</size>\\<x<bogus>", 2, &ctx);
        let s: String = g.iter().map(|g| g.ch).collect();
        assert_eq!(s, "ABCD<x<bogus>");
        assert_eq!((g[0].x, g[0].y), (100, 50));
        assert_eq!(g[1].x, 100 + 24);
        assert_eq!(g[0].style.color, Rgba::rgb8(255, 0, 0));
        assert_eq!(g[2].style.color, Rgba::WHITE);
        // 2 行目
        assert_eq!(g[3].x, 100);
        assert_eq!(g[3].style.size, 3);
        assert!(g[3].y > 50);
    }

    #[test]
    fn align_right() {
        let ctx = Context::new("2000-01-01", 0);
        let g = layout("<pos=700,0><align=right>AB", 2, &ctx);
        // 幅 = 2 文字 * 24 - 字間 4 = 44
        assert_eq!(g[0].x, 700 - 44);
    }
}
