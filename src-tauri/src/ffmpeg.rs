//! ffmpeg サブプロセスによる入出力。
//!
//! 入力 (カメラ / 動画) は ffmpeg に 59.94 fps・パイプライン入力解像度・RGB24 まで変換させ、
//! 標準出力から生フレームを読む。出力 (録画) は RGBA の生フレームを標準入力へ流し込む。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use vhs_core::{INPUT_HEIGHT, INPUT_WIDTH, OUTPUT_HEIGHT, OUTPUT_WIDTH, RgbFrame};

pub const FIELD_RATE: &str = "60000/1001";

pub fn ffmpeg_path() -> PathBuf {
    std::env::var_os("VHS_FFMPEG").map(PathBuf::from).unwrap_or_else(|| "ffmpeg".into())
}

fn ffprobe_path() -> PathBuf {
    let ff = ffmpeg_path();
    match ff.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(dir) => dir.join(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" }),
        None => "ffprobe".into(),
    }
}

fn command(program: &Path) -> Command {
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
        .args(["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"])
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
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"])
        .arg(path)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum StreamSpec {
    Camera { device: String, width: Option<u32>, height: Option<u32>, fps: Option<u32> },
    File { path: PathBuf, #[serde(default)] looped: bool, #[serde(default = "yes")] realtime: bool },
}

fn yes() -> bool {
    true
}

/// ffmpeg からフレームを読み出すストリーム
pub struct FrameStream {
    child: Child,
    pub rx: Receiver<RgbFrame>,
    _reader: JoinHandle<()>,
}

impl FrameStream {
    /// `lossy` = true なら処理が追いつかないとき古いフレームを捨てる (ライブ向け)
    pub fn open(spec: &StreamSpec, lossy: bool) -> Result<Self, String> {
        let mut cmd = command(&ffmpeg_path());
        cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
        match spec {
            StreamSpec::Camera { device, width, height, fps } => {
                cmd.args(["-f", "dshow", "-rtbufsize", "64M", "-fflags", "nobuffer", "-flags", "low_delay"]);
                if let (Some(w), Some(h)) = (width, height) {
                    cmd.args(["-video_size", &format!("{w}x{h}")]);
                }
                if let Some(f) = fps {
                    cmd.args(["-framerate", &f.to_string()]);
                }
                cmd.arg("-i").arg(format!("video={device}"));
            }
            StreamSpec::File { path, looped, realtime } => {
                if *realtime {
                    cmd.arg("-re");
                }
                if *looped {
                    cmd.args(["-stream_loop", "-1"]);
                }
                cmd.arg("-i").arg(path);
            }
        }
        let vf = format!("fps={FIELD_RATE},scale={INPUT_WIDTH}:{INPUT_HEIGHT}:flags=area");
        cmd.args(["-an", "-vf", &vf, "-pix_fmt", "rgb24", "-f", "rawvideo", "-"]);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| format!("ffmpeg を起動できません: {e}"))?;
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
            let size = INPUT_WIDTH * INPUT_HEIGHT * 3;
            loop {
                let mut buf = vec![0u8; size];
                if stdout.read_exact(&mut buf).is_err() {
                    break;
                }
                let frame = RgbFrame::from_raw(INPUT_WIDTH, INPUT_HEIGHT, buf).unwrap();
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
        Ok(Self { child, rx, _reader: reader })
    }
}

impl Drop for FrameStream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 処理結果を動画ファイルへ書き出す
pub struct Recorder {
    child: Child,
    tx: Option<SyncSender<Vec<u8>>>,
    writer: Option<JoinHandle<()>>,
    pub path: PathBuf,
}

impl Recorder {
    /// `audio_from` を指定するとその音声トラックを多重化する
    pub fn start(path: &Path, audio_from: Option<&Path>) -> Result<Self, String> {
        let mut cmd = command(&ffmpeg_path());
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
        cmd.args(["-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{OUTPUT_WIDTH}x{OUTPUT_HEIGHT}")]);
        cmd.args(["-framerate", FIELD_RATE, "-i", "-"]);
        if let Some(a) = audio_from {
            cmd.arg("-i").arg(a);
            cmd.args(["-map", "0:v", "-map", "1:a?", "-c:a", "aac", "-b:a", "192k", "-shortest"]);
        }
        // 4:3 の正方画素 640x480 にして書き出す
        cmd.args(["-vf", "scale=640:480:flags=bicubic,setsar=1"]);
        cmd.args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "16", "-pix_fmt", "yuv420p"]);
        cmd.arg(path);
        cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| format!("ffmpeg を起動できません: {e}"))?;
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
        Ok(Self { child, tx: Some(tx), writer: Some(writer), path: path.to_path_buf() })
    }

    pub fn push(&self, rgba: &[u8]) -> bool {
        self.tx.as_ref().is_some_and(|tx| tx.send(rgba.to_vec()).is_ok())
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.tx.take();
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        let status = self.child.wait().map_err(|e| e.to_string())?;
        if status.success() { Ok(()) } else { Err(format!("ffmpeg が異常終了しました ({status})")) }
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
