mod crt;
mod engine;
mod ffmpeg;
mod still;

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

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp", "exr"];

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

/// デッキのメニュー表示 (OSD マークアップ)。None で消す
#[tauri::command]
fn set_menu_osd(engine: State<'_, Engine>, markup: Option<String>) {
    engine.set_menu(markup);
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

/// オシロスコープを開いた・閉じた (開いている間だけエンジンがコンポジット信号を写す)
#[tauri::command]
fn set_scope_active(engine: State<'_, Engine>, active: bool) {
    engine.shared.scope_active.store(active, std::sync::atomic::Ordering::Relaxed);
}

/// `after` より新しいフィールドのコンポジット信号があれば返す (なければ空)。形式は `engine::composite_bytes`
#[tauri::command]
async fn get_composite(engine: State<'_, Engine>, after: u64) -> Result<Response, String> {
    let slot = engine.shared.composite.lock().unwrap();
    if slot.id > after { Ok(Response::new(slot.bytes.clone())) } else { Ok(Response::new(Vec::new())) }
}

#[tauri::command]
fn get_stats(engine: State<'_, Engine>) -> Stats {
    engine.shared.stats.lock().unwrap().clone()
}

/// CRT シェーダーの状態 (読み込んだプリセットとそのパラメータ)
#[tauri::command]
fn get_crt_info(engine: State<'_, Engine>) -> crt::CrtInfo {
    engine.shared.crt_info.lock().unwrap().clone()
}

/// CRT シェーダーを読み直す
#[tauri::command]
fn reload_crt(engine: State<'_, Engine>) {
    engine.shared.crt_reload.store(true, std::sync::atomic::Ordering::Release);
}

/// プレビュー画面の高さ [物理 px]。CRT シェーダーはこの大きさで描く
#[tauri::command]
fn set_viewport(engine: State<'_, Engine>, height: u32) {
    engine.shared.viewport_height.store(height, std::sync::atomic::Ordering::Relaxed);
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

/// フォントファイルを読めるか確かめ、フォント名を返す
#[tauri::command]
async fn check_font(path: String, index: u32) -> Result<String, String> {
    vhs_core::osd::face::describe(&path, index)
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
            set_menu_osd,
            list_cameras,
            open_source,
            get_frame,
            get_composite,
            set_scope_active,
            get_stats,
            start_recording,
            stop_recording,
            start_export,
            cancel_export,
            save_preset,
            load_preset,
            check_font,
            get_crt_info,
            reload_crt,
            set_viewport,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
