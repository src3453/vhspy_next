//! 処理スレッド。入力の取得 → パイプライン → プレビュー/録画への出力を回す。

use crate::crt::{Crt, CrtInfo};
use crate::scope::{ScopeField, ScopeRing};
use crate::still::{Still, StillFrame};
use crate::ffmpeg::{AUDIO_RATE, AudioTrack, FrameStream, Recorder, StreamSpec, probe_duration, process_audio};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use vhs_core::audio::AudioProcessor;
use vhs_core::{Params, Pipeline, RgbFrame, Standard};

/// 1 フィールドの時間 (NTSC なら 1001/60000 s)
fn field_period(tv: &Standard) -> Duration {
    let (n, d) = tv.field_rate_ratio;
    Duration::from_nanos(1_000_000_000 * d as u64 / n as u64)
}

/// フロントエンドへ渡すフレーム。先頭 16 バイトがヘッダ (id: u64, w: u32, h: u32)、以降 RGBA。
/// CRT シェーダーを掛けたときは、その後ろに元の画像 (w: u32, h: u32, RGBA) が続く (スコープ用)。
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
    pub crt_ms: f32,
    /// CRT シェーダーの状態 (`get_crt_info`) が変わるたびに増える
    pub crt_version: u64,
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
    pub crt_info: Mutex<CrtInfo>,
    /// プレビュー画面の高さ [物理 px] (0 なら未設定)。CRT シェーダーはこの大きさで描くとマスクがきれいに出る
    pub viewport_height: AtomicU32,
    /// CRT シェーダーを読み直す
    pub crt_reload: AtomicBool,
    /// デッキのメニュー表示 (イマーシブモードの OSD)。再生出力の信号に重畳する
    menu: Mutex<Option<String>>,
    menu_version: AtomicU64,
    /// オシロスコープが開いている (このときだけコンポジット信号を写す)
    pub scope_active: AtomicBool,
    /// 最近のフィールドのコンポジット信号 (オシロスコープ用)
    pub scope: Mutex<ScopeRing>,
}

#[derive(Clone, Debug)]
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
    /// 画像ファイル (配置・パンはフィールドごとに `params.image` で決める)
    Image(Box<Still>),
    Stream(FrameStream),
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

/// 入力を開く。画像サイズ・フィールド周波数はテレビ方式 `tv` に合わせる
fn open_source(spec: &SourceSpec, tv: &Standard) -> Result<Source, String> {
    Ok(match spec {
        SourceSpec::Pattern => Source::Still(vhs_core::pattern::color_bars(tv)),
        SourceSpec::Image(p) => Source::Image(Box::new(Still::load(p)?)),
        SourceSpec::Stream(s) => Source::Stream(FrameStream::open(s, true, tv)?),
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
            crt_info: Mutex::new(CrtInfo::default()),
            viewport_height: AtomicU32::new(0),
            crt_reload: AtomicBool::new(false),
            menu: Mutex::new(None),
            menu_version: AtomicU64::new(0),
            scope_active: AtomicBool::new(false),
            scope: Mutex::new(ScopeRing::default()),
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

    /// デッキのメニュー表示 (OSD マークアップ) を設定する。None で消す
    pub fn set_menu(&self, markup: Option<String>) {
        *self.shared.menu.lock().unwrap() = markup;
        self.shared.menu_version.fetch_add(1, Ordering::Release);
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
    let tv = params.standard.spec();
    let total = probe_duration(input).map(|d| (d * tv.field_rate_ratio.0 as f64 / tv.field_rate_ratio.1 as f64) as u64);

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
        FrameStream::open(&StreamSpec::File { path: input.to_path_buf(), looped: false, realtime: false }, false, tv)?;
    let mut crt = Crt::new(false);
    let square = if crt.prepare(&params.crt)? { Some(Crt::output_size(params.crt.height)) } else { None };
    let rec = Recorder::start(output, audio, tv, square)?;
    let crt_params = params.crt.clone();
    let mut pipeline = Pipeline::new(params);
    let mut done = 0u64;
    for frame in stream.rx.iter() {
        if cancel.load(Ordering::Relaxed) {
            drop(rec);
            let _ = std::fs::remove_file(output);
            return Err("キャンセルしました".into());
        }
        let out = pipeline.process_field(&frame);
        let data = match crt.process(&crt_params, out, crt_params.height) {
            Some((_, _, px)) => px,
            None if square.is_some() => {
                return Err(crt.info().error.clone().unwrap_or_else(|| "CRT シェーダーが使えません".into()));
            }
            None => &out.data,
        };
        if !rec.push(data) {
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
    let mut crt_params = shared.params.lock().unwrap().crt.clone();
    let mut crt = Crt::new(true);
    let mut crt_seen = 0;
    let mut seen_version = 0;
    let mut seen_menu = 0;
    let mut tv = pipeline.standard();
    // 方式が変わったときに開き直すため、今の入力の指定を覚えておく
    let mut spec = SourceSpec::Pattern;
    let mut source = Source::Still(vhs_core::pattern::color_bars(tv));
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
                Cmd::Open(new_spec, reply) => {
                    // 先に古いソースを閉じる (同じカメラを開き直す場合のため)
                    source = Source::Still(vhs_core::pattern::color_bars(tv));
                    spec = SourceSpec::Pattern;
                    let r = open_source(&new_spec, tv).map(|s| {
                        source = s;
                        let mut st = shared.stats.lock().unwrap();
                        st.source = source_name(&new_spec);
                        st.error = None;
                        spec = new_spec;
                    });
                    let _ = reply.send(r);
                }
                Cmd::StartRecording(path, reply) => {
                    if let Some(r) = recorder.take() {
                        let _ = r.finish();
                    }
                    // CRT シェーダーが効いていればその出力 (正方画素) を録画する
                    let square = crt.active(&crt_params).then(|| Crt::output_size(crt_params.height));
                    let r = Recorder::start(&path, None, tv, square).map(|r| {
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

        // ---- メニュー表示 ----
        let v = shared.menu_version.load(Ordering::Acquire);
        if v != seen_menu {
            seen_menu = v;
            pipeline.set_menu(shared.menu.lock().unwrap().clone());
        }

        // ---- パラメータ反映 ----
        let v = shared.params_version.load(Ordering::Acquire);
        if v != seen_version {
            seen_version = v;
            let p = shared.params.lock().unwrap().clone();
            crt_params = p.crt.clone();
            pipeline.set_params(p);
            if !std::ptr::eq(pipeline.standard(), tv) {
                // テレビ方式が変わった: 画像サイズ・フィールド周波数が変わるので入力を開き直す。
                // 録画中のファイルは途中でサイズを変えられないので止める
                tv = pipeline.standard();
                let mut st = shared.stats.lock().unwrap();
                if let Some(r) = recorder.take() {
                    let _ = r.finish();
                    st.recording = None;
                    st.error = Some("テレビ方式が変わったため録画を停止しました".into());
                }
                drop(st);
                // 画像は方式に合わせて毎フィールド配置し直すので開き直さなくてよい
                if !matches!(source, Source::Image(_)) {
                    source = Source::Still(vhs_core::pattern::color_bars(tv));
                    match open_source(&spec, tv) {
                        Ok(s) => source = s,
                        Err(e) => {
                            let mut st = shared.stats.lock().unwrap();
                            st.error = Some(e);
                            st.source = source_name(&SourceSpec::Pattern);
                            spec = SourceSpec::Pattern;
                        }
                    }
                }
            }
        }

        // ---- 入力 ----
        let streamed;
        let mut pace = || {
            // 静止画はフィールド周波数 (NTSC なら 59.94 Hz) で流す
            let now = Instant::now();
            if next_deadline > now {
                std::thread::sleep(next_deadline - now);
            }
            let period = field_period(tv);
            next_deadline = (next_deadline + period).max(Instant::now() - period);
            period
        };
        let image_params = pipeline.params().image.clone();
        let input = match &mut source {
            Source::Still(f) => {
                pace();
                StillFrame::Rgb(f)
            }
            Source::Image(s) => {
                let period = pace();
                s.render(&image_params, tv, period.as_secs_f32())
            }
            Source::Stream(s) => match s.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(f) => {
                    streamed = f;
                    StillFrame::Rgb(&streamed)
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    let mut st = shared.stats.lock().unwrap();
                    st.error = Some("入力が終了しました (ffmpeg のログを確認してください)".into());
                    st.source = source_name(&SourceSpec::Pattern);
                    drop(st);
                    source = Source::Still(vhs_core::pattern::color_bars(tv));
                    spec = SourceSpec::Pattern;
                    continue;
                }
            },
        };

        // ---- 処理 ----
        let out = match input {
            StillFrame::Rgb(f) => pipeline.process_field(f),
            StillFrame::Linear(f) => pipeline.process_field_linear(f),
        };

        // ---- ブラウン管 ----
        if shared.crt_reload.swap(false, Ordering::AcqRel) {
            crt.reload();
        }
        // 録画中は録画の大きさ、そうでなければプレビュー画面の大きさで描く
        let vh = shared.viewport_height.load(Ordering::Relaxed);
        let height = match &recorder {
            Some(r) => r.size.1,
            None if vh > 0 => vh,
            None => crt_params.height,
        };
        let t = Instant::now();
        let shown = crt.process(&crt_params, out, height);
        let crt_time = t.elapsed();
        let (dw, dh, dpx) = shown.unwrap_or((out.width as u32, out.height as u32, &out.data));

        frame_id += 1;
        {
            let mut slot = shared.frame.lock().unwrap();
            slot.id = frame_id;
            slot.bytes.clear();
            slot.bytes.extend_from_slice(&frame_id.to_le_bytes());
            slot.bytes.extend_from_slice(&dw.to_le_bytes());
            slot.bytes.extend_from_slice(&dh.to_le_bytes());
            slot.bytes.extend_from_slice(dpx);
            if shown.is_some() {
                slot.bytes.extend_from_slice(&(out.width as u32).to_le_bytes());
                slot.bytes.extend_from_slice(&(out.height as u32).to_le_bytes());
                slot.bytes.extend_from_slice(&out.data);
            }
        }
        if let Some(r) = &recorder {
            let err = if r.size != (dw, dh) {
                Some("CRT シェーダーの切り替えで画像の大きさが変わったため録画を停止しました")
            } else if !r.push(dpx) {
                Some("録画が中断されました")
            } else {
                None
            };
            if let Some(e) = err {
                if let Some(r) = recorder.take() {
                    let _ = r.finish();
                }
                let mut st = shared.stats.lock().unwrap();
                st.recording = None;
                st.error = Some(e.into());
            }
        }

        // オシロスコープ: 開いているときだけ最新フィールドの信号を写す
        if shared.scope_active.load(Ordering::Relaxed) {
            let f = ScopeField::from_field(frame_id, pipeline.composite());
            shared.scope.lock().unwrap().push(f);
        }

        if crt.info_version != crt_seen {
            crt_seen = crt.info_version;
            *shared.crt_info.lock().unwrap() = crt.info().clone();
            shared.stats.lock().unwrap().crt_version = crt_seen;
        }

        // ---- 統計 ----
        window_count += 1;
        let el = window_start.elapsed();
        if el >= Duration::from_millis(500) {
            let t = pipeline.timings;
            let mut st = shared.stats.lock().unwrap();
            st.fields_per_sec = window_count as f32 / el.as_secs_f32();
            // 入力の画質調整もカメラと同じ前段として数える
            st.camera_ms = (t.input + t.camera).as_secs_f32() * 1e3;
            st.encode_ms = t.encode.as_secs_f32() * 1e3;
            st.tape_ms = t.tape.as_secs_f32() * 1e3;
            st.rf_ms = t.rf.as_secs_f32() * 1e3;
            st.decode_ms = t.decode.as_secs_f32() * 1e3;
            st.capture_ms = t.capture.as_secs_f32() * 1e3;
            st.crt_ms = crt_time.as_secs_f32() * 1e3;
            let cs = pipeline.codec_stats();
            st.codec_q = cs.map(|c| c.q);
            st.codec_kbit = cs.map(|c| c.bits as f32 / 1e3);
            window_start = Instant::now();
            window_count = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最新フレームの (幅, 高さ) を待って返す
    fn wait_size(engine: &Engine, want: (u32, u32)) -> (u32, u32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut got = (0, 0);
        while Instant::now() < deadline {
            {
                let slot = engine.shared.frame.lock().unwrap();
                if slot.bytes.len() >= 16 {
                    let w = u32::from_le_bytes(slot.bytes[8..12].try_into().unwrap());
                    let h = u32::from_le_bytes(slot.bytes[12..16].try_into().unwrap());
                    got = (w, h);
                }
            }
            if got == want {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        got
    }

    /// GUI からテレビ方式を切り替えると、出力の大きさが方式に合わせて変わる (テストパターン入力)
    #[test]
    fn switching_standard_changes_output_size() {
        let engine = Engine::start();
        assert_eq!(wait_size(&engine, (754, 480)), (754, 480));

        let mut p = Params::default();
        p.standard = vhs_core::TvStandard::Pal;
        engine.set_params(p.clone());
        assert_eq!(wait_size(&engine, (922, 576)), (922, 576));
        assert_eq!(engine.shared.stats.lock().unwrap().error, None);

        p.standard = vhs_core::TvStandard::Ntsc;
        engine.set_params(p);
        assert_eq!(wait_size(&engine, (754, 480)), (754, 480));
    }

    /// CRT シェーダーを有効にすると、プレビューが画面の高さ (4:3 の正方画素) で描かれ、元の画像も付く。
    /// GPU と slang プリセットが必要: `VHS_TEST_SLANGP=.../crt-royale.slangp cargo test -p vhs-next -- --ignored`
    #[test]
    #[ignore]
    fn crt_shader_renders_at_viewport_size() {
        let preset = std::env::var("VHS_TEST_SLANGP").expect("VHS_TEST_SLANGP");
        let engine = Engine::start();
        assert_eq!(wait_size(&engine, (754, 480)), (754, 480));
        engine.shared.viewport_height.store(900, Ordering::Relaxed);
        let mut p = Params::default();
        p.crt.enabled = true;
        p.crt.preset = preset.clone();
        engine.set_params(p.clone());
        let deadline = Instant::now() + Duration::from_secs(60);
        while wait_size(&engine, (1200, 900)) != (1200, 900) && Instant::now() < deadline {}
        let info = engine.shared.crt_info.lock().unwrap().clone();
        assert_eq!(info.error, None);
        assert!(!info.params.is_empty());
        assert_eq!(wait_size(&engine, (1200, 900)), (1200, 900));
        {
            let slot = engine.shared.frame.lock().unwrap();
            assert_eq!(slot.bytes.len(), 16 + 1200 * 900 * 4 + 8 + 754 * 480 * 4);
            // 真っ黒ではない
            let mean = slot.bytes[16..16 + 1200 * 900 * 4].iter().map(|&b| b as u64).sum::<u64>() / (1200 * 900 * 4);
            assert!(mean > 40, "mean {mean}");
        }

        // パラメータの上書き (存在しない名前は無視される) と無効化
        let first = info.params.iter().find(|q| q.min != q.max).unwrap();
        p.crt.params.insert(first.id.clone(), first.max);
        p.crt.params.insert("no_such_param".into(), 1.0);
        engine.set_params(p.clone());
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(engine.shared.stats.lock().unwrap().error, None);
        p.crt.enabled = false;
        engine.set_params(p);
        assert_eq!(wait_size(&engine, (754, 480)), (754, 480));
    }

    /// GUI から送られる JSON (snake_case) で PAL を受け取れる
    #[test]
    fn params_json_accepts_pal() {
        let p: Params = serde_json::from_str(r#"{"standard":"pal"}"#).unwrap();
        assert_eq!(p.standard, vhs_core::TvStandard::Pal);
    }
}
