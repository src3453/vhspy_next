//! ffmpeg サブプロセスによる入出力。
//!
//! 入力 (カメラ / 動画) は ffmpeg に 59.94 fps・パイプライン入力解像度・RGB24 まで変換させ、
//! 標準出力から生フレームを読む。出力 (録画) は RGBA の生フレームを標準入力へ流し込む。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use vhs_core::{RgbFrame, Standard};

/// ffmpeg に渡すフィールド周波数 (例: NTSC は "60000/1001")
fn field_rate_arg(tv: &Standard) -> String {
    let (n, d) = tv.field_rate_ratio;
    format!("{n}/{d}")
}

pub fn ffmpeg_path() -> PathBuf {
    std::env::var_os("VHS_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| "ffmpeg".into())
}

fn ffprobe_path() -> PathBuf {
    let ff = ffmpeg_path();
    match ff.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(dir) => dir.join(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        }),
        None => "ffprobe".into(),
    }
}

pub(crate) fn command(program: &Path) -> Command {
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// DirectShow のビデオデバイス名一覧
pub fn list_cameras() -> Result<Vec<String>, String> {
    let out = command(&ffmpeg_path())
        .args([
            "-hide_banner",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ])
        .output()
        .map_err(|e| format!("ffmpeg を起動できません: {e}"))?;
    let text = String::from_utf8_lossy(&out.stderr);
    Ok(text
        .lines()
        .filter(|l| l.trim_end().ends_with("(video)"))
        .filter_map(|l| {
            let s = l.find('"')?;
            let e = l.rfind('"')?;
            (e > s).then(|| l[s + 1..e].to_string())
        })
        .collect())
}

/// 動画の長さ [秒]
pub fn probe_duration(path: &Path) -> Option<f64> {
    let out = command(&ffprobe_path())
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum StreamSpec {
    Camera {
        device: String,
        width: Option<u32>,
        height: Option<u32>,
        fps: Option<u32>,
        #[serde(default)]
        scaling: ScalingMethod,
    },
    Window {
        title: String,
        /// 一覧で選んだウィンドウのハンドル (なければタイトルで探す)
        #[serde(default)]
        hwnd: Option<u64>,
        /// ウィンドウのアプリの音声も取り込む
        #[serde(default = "yes")]
        capture_audio: bool,
        /// 取り込んだ音声を (処理して) 既定の出力デバイスで鳴らす
        #[serde(default = "yes")]
        monitor_audio: bool,
        crop_x: u32,
        crop_y: u32,
        crop_width: u32,
        crop_height: u32,
        #[serde(default)]
        scaling: ScalingMethod,
    },
    File {
        path: PathBuf,
        #[serde(default)]
        looped: bool,
        #[serde(default = "yes")]
        realtime: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalingMethod {
    Nearest,
    Bilinear,
    Bicubic,
    #[default]
    Area,
    Lanczos,
}

impl ScalingMethod {
    pub(crate) fn ffmpeg_flags(self) -> &'static str {
        match self {
            Self::Nearest => "neighbor",
            Self::Bilinear => "bilinear",
            Self::Bicubic => "bicubic",
            Self::Area => "area",
            Self::Lanczos => "lanczos",
        }
    }
}

fn yes() -> bool {
    true
}

/// ffmpeg からフレームを読み出すストリーム
pub struct FrameStream {
    child: Option<Child>,
    pub rx: Receiver<RgbFrame>,
    _reader: Option<JoinHandle<()>>,
    /// 入力を止めるためのもの (ウィンドウキャプチャ)。捨てると止まる
    _guard: Option<Box<dyn Send>>,
}

impl FrameStream {
    /// `lossy` = true なら処理が追いつかないとき古いフレームを捨てる (ライブ向け)。
    /// フレームはテレビ方式 `tv` のフィールド周波数・画像サイズに変換して読み出す。
    /// ffmpeg 以外で作った入力から
    pub(crate) fn from_parts(rx: Receiver<RgbFrame>, guard: Box<dyn Send>) -> Self {
        Self {
            child: None,
            rx,
            _reader: None,
            _guard: Some(guard),
        }
    }

    pub fn open(spec: &StreamSpec, lossy: bool, tv: &'static Standard) -> Result<Self, String> {
        #[cfg(windows)]
        if let StreamSpec::Window {
            title,
            hwnd,
            crop_x,
            crop_y,
            crop_width,
            crop_height,
            scaling,
            ..
        } = spec
        {
            let h = crate::wgc::resolve_window(title, *hwnd)?;
            return crate::wgc::open_stream(
                h,
                (*crop_x, *crop_y, *crop_width, *crop_height),
                *scaling,
                lossy,
                tv,
            );
        }
        let (iw, ih) = tv.image_size();
        let mut cmd = command(&ffmpeg_path());
        cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
        match spec {
            StreamSpec::Camera {
                device,
                width,
                height,
                fps,
                ..
            } => {
                cmd.args([
                    "-f",
                    "dshow",
                    "-rtbufsize",
                    "64M",
                    "-fflags",
                    "nobuffer",
                    "-flags",
                    "low_delay",
                ]);
                if let (Some(w), Some(h)) = (width, height) {
                    cmd.args(["-video_size", &format!("{w}x{h}")]);
                }
                if let Some(f) = fps {
                    cmd.args(["-framerate", &f.to_string()]);
                }
                cmd.arg("-i").arg(format!("video={device}"));
            }
            // Windows では上で WGC に回している
            StreamSpec::Window { .. } => {
                return Err("ウィンドウキャプチャは Windows でのみ利用できます".into());
            }
            StreamSpec::File {
                path,
                looped,
                realtime,
            } => {
                if *realtime {
                    cmd.arg("-re");
                }
                if *looped {
                    cmd.args(["-stream_loop", "-1"]);
                }
                cmd.arg("-i").arg(path);
            }
        }
        let scaling = match spec {
            StreamSpec::Camera { scaling, .. } | StreamSpec::Window { scaling, .. } => *scaling,
            StreamSpec::File { .. } => ScalingMethod::Area,
        };
        let vf = format!(
            "fps={},scale={iw}:{ih}:flags={}",
            field_rate_arg(tv),
            scaling.ffmpeg_flags()
        );
        cmd.args([
            "-an", "-vf", &vf, "-pix_fmt", "rgb24", "-f", "rawvideo", "-",
        ]);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("ffmpeg を起動できません: {e}"))?;
        let mut stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        // stderr を読み捨てないとパイプが詰まる。最後のエラーだけログに出す。
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::BufReader::new(stderr).read_to_string(&mut s);
            if !s.trim().is_empty() {
                eprintln!("[ffmpeg] {}", s.trim());
            }
        });

        let (tx, rx) = sync_channel::<RgbFrame>(2);
        let reader = std::thread::spawn(move || {
            let size = iw * ih * 3;
            loop {
                let mut buf = vec![0u8; size];
                if stdout.read_exact(&mut buf).is_err() {
                    break;
                }
                let frame = RgbFrame::from_raw(iw, ih, buf).unwrap();
                let r = if lossy {
                    match tx.try_send(frame) {
                        Err(TrySendError::Disconnected(_)) => Err(()),
                        _ => Ok(()),
                    }
                } else {
                    tx.send(frame).map_err(|_| ())
                };
                if r.is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child: Some(child),
            rx,
            _reader: Some(reader),
            _guard: None,
        })
    }
}

impl Drop for FrameStream {
    fn drop(&mut self) {
        if let Some(c) = &mut self.child {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// 音声処理のサンプリング周波数
pub const AUDIO_RATE: u32 = 48000;

/// 入力の音声を 48 kHz ステレオ f32 にデコードし、`process` をかけて WAV (IEEE float) に書き出す。
///
/// `latency` サンプル (フレーム) ぶん頭を捨てて尻に無音を足し、処理の遅れを打ち消す。
/// 入力に音声がなければ `Ok(false)`。
pub fn process_audio(
    input: &Path,
    wav: &Path,
    latency: usize,
    cancel: &std::sync::atomic::AtomicBool,
    mut process: impl FnMut(&mut [f32]),
) -> Result<bool, String> {
    let mut child = command(&ffmpeg_path())
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(input)
        .args([
            "-vn",
            "-ac",
            "2",
            "-ar",
            &AUDIO_RATE.to_string(),
            "-f",
            "f32le",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("ffmpeg を起動できません: {e}"))?;
    let mut stdout = child.stdout.take().unwrap();
    let file = std::fs::File::create(wav).map_err(|e| format!("一時ファイルを作れません: {e}"))?;
    let mut out = std::io::BufWriter::new(file);
    write_wav_header(&mut out, 0).map_err(|e| e.to_string())?;

    const CHUNK: usize = 4096; // フレーム
    let mut bytes = vec![0u8; CHUNK * 8];
    let mut samples = vec![0f32; CHUNK * 2];
    let mut skip = latency * 2;
    let mut written = 0u64;
    let mut write = |out: &mut std::io::BufWriter<std::fs::File>,
                     s: &[f32],
                     skip: &mut usize|
     -> std::io::Result<()> {
        let n = (*skip).min(s.len());
        *skip -= n;
        let s = &s[n..];
        for v in s {
            out.write_all(&v.to_le_bytes())?;
        }
        written += s.len() as u64;
        Ok(())
    };
    let mut total_in = 0u64;
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = child.kill();
            return Err("キャンセルしました".into());
        }
        // 端数が出ないよう、読めた分だけ処理する (フレーム境界に揃える)
        let mut filled = 0;
        while filled < bytes.len() {
            match stdout.read(&mut bytes[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) => return Err(e.to_string()),
            }
        }
        let frames = filled / 8;
        if frames == 0 {
            break;
        }
        total_in += frames as u64;
        let buf = &mut samples[..frames * 2];
        for (i, v) in buf.iter_mut().enumerate() {
            *v = f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        }
        process(buf);
        write(&mut out, buf, &mut skip).map_err(|e| e.to_string())?;
        if filled < bytes.len() {
            break;
        }
    }
    let _ = child.wait();
    if total_in == 0 {
        drop(out);
        let _ = std::fs::remove_file(wav);
        return Ok(false);
    }
    // 遅延線に残っている分を押し出す
    let mut tail = vec![0f32; latency * 2];
    process(&mut tail);
    write(&mut out, &tail, &mut skip).map_err(|e| e.to_string())?;

    let mut file = out.into_inner().map_err(|e| e.to_string())?;
    use std::io::Seek;
    file.seek(std::io::SeekFrom::Start(0))
        .map_err(|e| e.to_string())?;
    write_wav_header(&mut file, written * 4).map_err(|e| e.to_string())?;
    Ok(true)
}

/// 2ch / 32 bit float の WAV ヘッダ
pub(crate) fn write_wav_header(w: &mut impl Write, data_bytes: u64) -> std::io::Result<()> {
    let data = data_bytes.min(u32::MAX as u64 - 64) as u32;
    let (ch, bits) = (2u16, 32u16);
    let block = ch * bits / 8;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&3u16.to_le_bytes())?; // IEEE float
    w.write_all(&ch.to_le_bytes())?;
    w.write_all(&AUDIO_RATE.to_le_bytes())?;
    w.write_all(&(AUDIO_RATE * block as u32).to_le_bytes())?;
    w.write_all(&block.to_le_bytes())?;
    w.write_all(&bits.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data.to_le_bytes())
}

/// 映像だけのファイル `video` に音声 `audio` を多重化して `output` に書く (映像は再エンコードしない)
pub fn mux_audio(
    video: &Path,
    audio: &Path,
    output: &Path,
    kbps: u32,
    mono: bool,
) -> Result<(), String> {
    let mut cmd = command(&ffmpeg_path());
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
        .arg(video)
        .arg("-i")
        .arg(audio)
        .args([
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            &format!("{}k", kbps.clamp(8, 512)),
            "-shortest",
        ]);
    if mono {
        cmd.args(["-ac", "1"]);
    }
    let out = cmd
        .arg(output)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg を起動できません: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "音声の多重化に失敗しました: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// 録画ファイルに多重化する音声
pub struct AudioTrack<'a> {
    pub path: &'a Path,
    /// AAC のビットレート [kbps]
    pub kbps: u32,
    pub mono: bool,
}

/// 処理結果を動画ファイルへ書き出す
pub struct Recorder {
    child: Child,
    tx: Option<SyncSender<Vec<u8>>>,
    writer: Option<JoinHandle<()>>,
    pub path: PathBuf,
    /// 入力フレームの (幅, 高さ)
    pub size: (u32, u32),
}

impl Recorder {
    /// `audio` を指定するとその音声トラックを多重化する。
    /// 入力はテレビ方式 `tv` のフィールド周波数の RGBA。
    /// `square` が None なら方式の画像サイズ (非正方画素) で、4:3 に引き伸ばして書き出す。
    /// Some((幅, 高さ)) なら正方画素の画像 (CRT シェーダーの出力) としてそのまま書き出す
    pub fn start(
        path: &Path,
        audio: Option<AudioTrack<'_>>,
        tv: &Standard,
        square: Option<(u32, u32)>,
    ) -> Result<Self, String> {
        let (ow, oh) = square.map_or_else(
            || {
                let (w, h) = tv.image_size();
                (w as u32, h as u32)
            },
            |s| s,
        );
        let mut cmd = command(&ffmpeg_path());
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
        cmd.args([
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{ow}x{oh}"),
        ]);
        cmd.args(["-framerate", &field_rate_arg(tv), "-i", "-"]);
        if let Some(a) = audio {
            cmd.arg("-i").arg(a.path);
            let kbps = format!("{}k", a.kbps.clamp(8, 512));
            cmd.args([
                "-map",
                "0:v",
                "-map",
                "1:a?",
                "-c:a",
                "aac",
                "-b:a",
                &kbps,
                "-shortest",
            ]);
            if a.mono {
                cmd.args(["-ac", "1"]);
            }
        }
        if square.is_none() {
            // 4:3 の正方画素 (NTSC なら 640x480) にして書き出す
            let dw = (oh * 4 / 3 + 1) & !1;
            cmd.args(["-vf", &format!("scale={dw}:{oh}:flags=bicubic,setsar=1")]);
        }
        cmd.args([
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "16", "-pix_fmt", "yuv420p",
        ]);
        cmd.arg(path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("ffmpeg を起動できません: {e}"))?;
        let mut stdin = child.stdin.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::BufReader::new(stderr).read_to_string(&mut s);
            if !s.trim().is_empty() {
                eprintln!("[ffmpeg rec] {}", s.trim());
            }
        });
        let (tx, rx) = sync_channel::<Vec<u8>>(8);
        let writer = std::thread::spawn(move || {
            for f in rx {
                if stdin.write_all(&f).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            tx: Some(tx),
            writer: Some(writer),
            path: path.to_path_buf(),
            size: (ow, oh),
        })
    }

    pub fn push(&self, rgba: &[u8]) -> bool {
        self.tx
            .as_ref()
            .is_some_and(|tx| tx.send(rgba.to_vec()).is_ok())
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.tx.take();
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        let status = self.child.wait().map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("ffmpeg が異常終了しました ({status})"))
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.tx.is_some() {
            self.tx.take();
            if let Some(w) = self.writer.take() {
                let _ = w.join();
            }
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// ffmpeg と音声付きの動画が必要: `VHS_TEST_VIDEO=in.mp4 cargo test -p vhs-next -- --ignored`
    #[test]
    #[ignore]
    fn export_audio_roundtrip() {
        let input = PathBuf::from(std::env::var("VHS_TEST_VIDEO").expect("VHS_TEST_VIDEO"));
        let dir = std::env::temp_dir();
        let wav = dir.join("vhs_test_audio.wav");
        let out = dir.join("vhs_test_out.mp4");
        let params = vhs_core::Params::default();
        let mut proc = vhs_core::audio::AudioProcessor::new(&params, AUDIO_RATE);
        let latency = proc.latency();
        let has = process_audio(&input, &wav, latency, &AtomicBool::new(false), |b| {
            proc.process(b)
        })
        .unwrap();
        assert!(has);
        let tv = params.tv();
        let rec = Recorder::start(
            &out,
            Some(AudioTrack {
                path: &wav,
                kbps: 64,
                mono: true,
            }),
            tv,
            None,
        )
        .unwrap();
        let (ow, oh) = tv.image_size();
        let frame = vec![128u8; ow * oh * 4];
        for _ in 0..120 {
            assert!(rec.push(&frame));
        }
        rec.finish().unwrap();
        let probe = command(&ffprobe_path())
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,channels,bit_rate",
                "-of",
                "compact",
            ])
            .arg(&out)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&probe.stdout);
        println!("{text}");
        assert!(
            text.contains("codec_type=audio") && text.contains("channels=1"),
            "{text}"
        );
        let _ = std::fs::remove_file(&wav);
    }
}
