//! 処理スレッド。入力の取得 → パイプライン → プレビュー/録画への出力を回す。

use crate::ffmpeg::{AUDIO_RATE, AudioTrack, FrameStream, Recorder, StreamSpec, probe_duration, process_audio};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use vhs_core::audio::AudioProcessor;
use vhs_core::{INPUT_HEIGHT, INPUT_WIDTH, Params, Pipeline, RgbFrame};

const FIELD_PERIOD: Duration = Duration::from_nanos(16_683_350); // 1001/60000 s

/// フロントエンドへ渡すフレーム。先頭 16 バイトがヘッダ (id: u64, w: u32, h: u32)、以降 RGBA。
pub struct FrameSlot {
    pub id: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub source: String,
    pub fields_per_sec: f32,
    pub camera_ms: f32,
    pub encode_ms: f32,
    pub tape_ms: f32,
    pub rf_ms: f32,
    pub decode_ms: f32,
    pub capture_ms: f32,
    /// キャプチャ圧縮の量子化スケールと直前フレームの符号量 [kbit]
    pub codec_q: Option<f32>,
    pub codec_kbit: Option<f32>,
    pub recording: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExportProgress {
    pub done: u64,
    pub total: Option<u64>,
    pub finished: bool,
    pub error: Option<String>,
}

pub struct Shared {
    pub params: Mutex<Params>,
    params_version: AtomicU64,
    pub frame: Mutex<FrameSlot>,
    pub stats: Mutex<Stats>,
    export_cancel: Mutex<Option<Arc<AtomicBool>>>,
}

#[derive(Debug)]
pub enum SourceSpec {
    Pattern,
    Image(PathBuf),
    Stream(StreamSpec),
}

enum Cmd {
    Open(SourceSpec, std::sync::mpsc::Sender<Result<(), String>>),
    StartRecording(PathBuf, std::sync::mpsc::Sender<Result<(), String>>),
    StopRecording(std::sync::mpsc::Sender<Result<(), String>>),
}

pub struct Engine {
    tx: Sender<Cmd>,
    pub shared: Arc<Shared>,
}

enum Source {
    Still(RgbFrame),
    Stream(FrameStream),
}

fn load_image(path: &Path) -> Result<RgbFrame, String> {
    let img = image::open(path).map_err(|e| format!("画像を開けません: {e}"))?.to_rgb8();
    let img = image::imageops::resize(&img, INPUT_WIDTH as u32, INPUT_HEIGHT as u32, image::imageops::FilterType::CatmullRom);
    Ok(RgbFrame::from_raw(INPUT_WIDTH, INPUT_HEIGHT, img.into_raw()).unwrap())
}

fn source_name(spec: &SourceSpec) -> String {
    match spec {
        SourceSpec::Pattern => "テストパターン".into(),
        SourceSpec::Image(p) => format!("画像: {}", p.file_name().unwrap_or_default().to_string_lossy()),
        SourceSpec::Stream(StreamSpec::Camera { device, .. }) => format!("カメラ: {device}"),
        SourceSpec::Stream(StreamSpec::File { path, .. }) => {
            format!("動画: {}", path.file_name().unwrap_or_default().to_string_lossy())
        }
    }
}

fn open_source(spec: &SourceSpec) -> Result<Source, String> {
    Ok(match spec {
        SourceSpec::Pattern => Source::Still(vhs_core::pattern::color_bars()),
        SourceSpec::Image(p) => Source::Still(load_image(p)?),
        SourceSpec::Stream(s) => Source::Stream(FrameStream::open(s, true)?),
    })
}

impl Engine {
    pub fn start() -> Self {
        let shared = Arc::new(Shared {
            params: Mutex::new(Params::default()),
            params_version: AtomicU64::new(1),
            frame: Mutex::new(FrameSlot { id: 0, bytes: Vec::new() }),
            stats: Mutex::new(Stats::default()),
            export_cancel: Mutex::new(None),
        });
        let (tx, rx) = channel();
        let sh = shared.clone();
        std::thread::Builder::new().name("vhs-engine".into()).spawn(move || run(sh, rx)).unwrap();
        Self { tx, shared }
    }

    fn call(&self, make: impl FnOnce(Sender<Result<(), String>>) -> Cmd) -> Result<(), String> {
        let (rtx, rrx) = channel();
        self.tx.send(make(rtx)).map_err(|_| "エンジンが停止しています".to_string())?;
        rrx.recv().map_err(|_| "エンジンが停止しています".to_string())?
    }

    pub fn open(&self, spec: SourceSpec) -> Result<(), String> {
        self.call(|r| Cmd::Open(spec, r))
    }

    pub fn start_recording(&self, path: PathBuf) -> Result<(), String> {
        self.call(|r| Cmd::StartRecording(path, r))
    }

    pub fn stop_recording(&self) -> Result<(), String> {
        self.call(Cmd::StopRecording)
    }

    pub fn set_params(&self, p: Params) {
        *self.shared.params.lock().unwrap() = p;
        self.shared.params_version.fetch_add(1, Ordering::Release);
    }

    /// 動画ファイルを最初から最後まで (実時間より速く) 処理して書き出す
    pub fn start_export(&self, app: AppHandle, input: PathBuf, output: PathBuf) -> Result<(), String> {
        let mut guard = self.shared.export_cancel.lock().unwrap();
        if guard.is_some() {
            return Err("書き出し中です".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *guard = Some(cancel.clone());
        let params = self.shared.params.lock().unwrap().clone();
        let shared = self.shared.clone();
        std::thread::Builder::new()
            .name("vhs-export".into())
            .spawn(move || {
                let r = export(&app, &input, &output, params, &cancel);
                let _ = app.emit(
                    "export-progress",
                    ExportProgress { done: 0, total: None, finished: true, error: r.err() },
                );
                *shared.export_cancel.lock().unwrap() = None;
            })
            .unwrap();
        Ok(())
    }

    pub fn cancel_export(&self) {
        if let Some(c) = self.shared.export_cancel.lock().unwrap().as_ref() {
            c.store(true, Ordering::Relaxed);
        }
    }
}

/// 書き出し後に消す一時ファイル
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn export(app: &AppHandle, input: &Path, output: &Path, params: Params, cancel: &AtomicBool) -> Result<(), String> {
    let total = probe_duration(input).map(|d| (d * 60000.0 / 1001.0) as u64);

    // 音声は先に全部処理して一時 WAV にする (映像より桁違いに軽い)
    let mut audio_tmp = None;
    if params.audio.enabled {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let tmp = TempFile(std::env::temp_dir().join(format!("vhs_audio_{}_{nanos}.wav", std::process::id())));
        let mut proc = AudioProcessor::new(&params, AUDIO_RATE);
        let latency = proc.latency();
        if process_audio(input, &tmp.0, latency, cancel, |buf| proc.process(buf))? {
            audio_tmp = Some(tmp);
        }
    }
    let audio = match &audio_tmp {
        Some(t) => Some(AudioTrack { path: &t.0, kbps: params.audio.codec_kbps, mono: params.audio.mono }),
        None if params.audio.enabled => None,
        // 音声処理を切っているときは元の音声をそのまま使う
        None => Some(AudioTrack { path: input, kbps: params.audio.codec_kbps, mono: false }),
    };

    let stream =
        FrameStream::open(&StreamSpec::File { path: input.to_path_buf(), looped: false, realtime: false }, false)?;
    let rec = Recorder::start(output, audio)?;
    let mut pipeline = Pipeline::new(params);
    let mut done = 0u64;
    for frame in stream.rx.iter() {
        if cancel.load(Ordering::Relaxed) {
            drop(rec);
            let _ = std::fs::remove_file(output);
            return Err("キャンセルしました".into());
        }
        let out = pipeline.process_field(&frame);
        if !rec.push(&out.data) {
            return Err("エンコーダへの書き込みに失敗しました".into());
        }
        done += 1;
        if done % 30 == 0 {
            let _ = app.emit("export-progress", ExportProgress { done, total, finished: false, error: None });
        }
    }
    drop(stream);
    rec.finish()
}

fn run(shared: Arc<Shared>, rx: Receiver<Cmd>) {
    let mut pipeline = Pipeline::new(shared.params.lock().unwrap().clone());
    let mut seen_version = 0;
    let mut source = Source::Still(vhs_core::pattern::color_bars());
    shared.stats.lock().unwrap().source = source_name(&SourceSpec::Pattern);
    let mut recorder: Option<Recorder> = None;
    let mut next_deadline = Instant::now();
    let mut frame_id = 0u64;
    let (mut window_start, mut window_count) = (Instant::now(), 0u32);

    loop {
        // ---- コマンド処理 ----
        loop {
            let cmd = match rx.try_recv() {
                Ok(c) => c,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            };
            match cmd {
                Cmd::Open(spec, reply) => {
                    // 先に古いソースを閉じる (同じカメラを開き直す場合のため)
                    source = Source::Still(vhs_core::pattern::color_bars());
                    let r = open_source(&spec).map(|s| {
                        source = s;
                        let mut st = shared.stats.lock().unwrap();
                        st.source = source_name(&spec);
                        st.error = None;
                    });
                    let _ = reply.send(r);
                }
                Cmd::StartRecording(path, reply) => {
                    if let Some(r) = recorder.take() {
                        let _ = r.finish();
                    }
                    let r = Recorder::start(&path, None).map(|r| {
                        shared.stats.lock().unwrap().recording = Some(r.path.display().to_string());
                        recorder = Some(r);
                    });
                    let _ = reply.send(r);
                }
                Cmd::StopRecording(reply) => {
                    shared.stats.lock().unwrap().recording = None;
                    let r = recorder.take().map(|r| r.finish()).unwrap_or(Ok(()));
                    let _ = reply.send(r);
                }
            }
        }

        // ---- 入力 ----
        let streamed;
        let input: &RgbFrame = match &source {
            Source::Still(f) => {
                // 静止画は 59.94 Hz で同じ画像を流す
                let now = Instant::now();
                if next_deadline > now {
                    std::thread::sleep(next_deadline - now);
                }
                next_deadline = (next_deadline + FIELD_PERIOD).max(Instant::now() - FIELD_PERIOD);
                f
            }
            Source::Stream(s) => match s.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(f) => {
                    streamed = f;
                    &streamed
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    let mut st = shared.stats.lock().unwrap();
                    st.error = Some("入力が終了しました (ffmpeg のログを確認してください)".into());
                    st.source = source_name(&SourceSpec::Pattern);
                    drop(st);
                    source = Source::Still(vhs_core::pattern::color_bars());
                    continue;
                }
            },
        };

        // ---- パラメータ反映 ----
        let v = shared.params_version.load(Ordering::Acquire);
        if v != seen_version {
            seen_version = v;
            pipeline.set_params(shared.params.lock().unwrap().clone());
        }

        // ---- 処理 ----
        let out = pipeline.process_field(input);
        frame_id += 1;
        {
            let mut slot = shared.frame.lock().unwrap();
            slot.id = frame_id;
            slot.bytes.clear();
            slot.bytes.extend_from_slice(&frame_id.to_le_bytes());
            slot.bytes.extend_from_slice(&(out.width as u32).to_le_bytes());
            slot.bytes.extend_from_slice(&(out.height as u32).to_le_bytes());
            slot.bytes.extend_from_slice(&out.data);
        }
        if let Some(r) = &recorder {
            if !r.push(&out.data) {
                recorder = None;
                let mut st = shared.stats.lock().unwrap();
                st.recording = None;
                st.error = Some("録画が中断されました".into());
            }
        }

        // ---- 統計 ----
        window_count += 1;
        let el = window_start.elapsed();
        if el >= Duration::from_millis(500) {
            let t = pipeline.timings;
            let mut st = shared.stats.lock().unwrap();
            st.fields_per_sec = window_count as f32 / el.as_secs_f32();
            st.camera_ms = t.camera.as_secs_f32() * 1e3;
            st.encode_ms = t.encode.as_secs_f32() * 1e3;
            st.tape_ms = t.tape.as_secs_f32() * 1e3;
            st.rf_ms = t.rf.as_secs_f32() * 1e3;
            st.decode_ms = t.decode.as_secs_f32() * 1e3;
            st.capture_ms = t.capture.as_secs_f32() * 1e3;
            let cs = pipeline.codec_stats();
            st.codec_q = cs.map(|c| c.q);
            st.codec_kbit = cs.map(|c| c.bits as f32 / 1e3);
            window_start = Instant::now();
            window_count = 0;
        }
    }
}
