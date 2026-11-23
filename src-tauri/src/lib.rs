mod engine;
mod ffmpeg;

use engine::{Engine, SourceSpec, Stats};
use ffmpeg::StreamSpec;
use serde::Deserialize;
use std::path::PathBuf;
use tauri::State;
use tauri::ipc::Response;
use vhs_core::Params;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum OpenRequest {
    Pattern,
    Camera { device: String, width: Option<u32>, height: Option<u32>, fps: Option<u32> },
    /// 拡張子で画像/動画を判別する
    File { path: PathBuf },
}

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp"];

#[tauri::command]
fn get_params(engine: State<'_, Engine>) -> Params {
    engine.shared.params.lock().unwrap().clone()
}

#[tauri::command]
fn default_params() -> Params {
    Params::default()
}

#[tauri::command]
fn set_params(engine: State<'_, Engine>, params: Params) {
    engine.set_params(params);
}

#[tauri::command]
async fn list_cameras() -> Result<Vec<String>, String> {
    ffmpeg::list_cameras()
}

#[tauri::command]
async fn open_source(engine: State<'_, Engine>, req: OpenRequest) -> Result<(), String> {
    let spec = match req {
        OpenRequest::Pattern => SourceSpec::Pattern,
        OpenRequest::Camera { device, width, height, fps } => {
            SourceSpec::Stream(StreamSpec::Camera { device, width, height, fps })
        }
        OpenRequest::File { path } => {
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if IMAGE_EXTS.contains(&ext.as_str()) {
                SourceSpec::Image(path)
            } else {
                SourceSpec::Stream(StreamSpec::File { path, looped: true, realtime: true })
            }
        }
    };
    engine.open(spec)
}

/// `after` より新しいフレームがあれば返す (なければ空)。
#[tauri::command]
async fn get_frame(engine: State<'_, Engine>, after: u64) -> Result<Response, String> {
    let slot = engine.shared.frame.lock().unwrap();
    if slot.id > after { Ok(Response::new(slot.bytes.clone())) } else { Ok(Response::new(Vec::new())) }
}

#[tauri::command]
fn get_stats(engine: State<'_, Engine>) -> Stats {
    engine.shared.stats.lock().unwrap().clone()
}

#[tauri::command]
async fn start_recording(engine: State<'_, Engine>, path: PathBuf) -> Result<(), String> {
    engine.start_recording(path)
}

#[tauri::command]
async fn stop_recording(engine: State<'_, Engine>) -> Result<(), String> {
    engine.stop_recording()
}

#[tauri::command]
fn start_export(app: tauri::AppHandle, engine: State<'_, Engine>, input: PathBuf, output: PathBuf) -> Result<(), String> {
    engine.start_export(app, input, output)
}

#[tauri::command]
fn cancel_export(engine: State<'_, Engine>) {
    engine.cancel_export();
}

#[tauri::command]
async fn save_preset(path: PathBuf, params: Params) -> Result<(), String> {
    let s = serde_json::to_string_pretty(&params).map_err(|e| e.to_string())?;
    std::fs::write(path, s).map_err(|e| e.to_string())
}

#[tauri::command]
async fn load_preset(path: PathBuf) -> Result<Params, String> {
    let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Engine::start())
        .invoke_handler(tauri::generate_handler![
            get_params,
            default_params,
            set_params,
            list_cameras,
            open_source,
            get_frame,
            get_stats,
            start_recording,
            stop_recording,
            start_export,
            cancel_export,
            save_preset,
            load_preset,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
