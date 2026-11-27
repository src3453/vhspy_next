//! Windows Graphics Capture によるウィンドウキャプチャ。
//!
//! gdigrab (GDI の BitBlt) では GPU で描くウィンドウ (ブラウザ・ゲームなど) が真っ黒になり、
//! タイトルに日本語などを含むウィンドウも見つけられないので、ウィンドウハンドルを直接 WGC に渡す。
//! 取り込んだクライアント領域をフィールド周波数で ffmpeg に流し、切り抜き・縮小・RGB24 変換をさせる。

use crate::ffmpeg::{FrameStream, ScalingMethod, command, ffmpeg_path};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use vhs_core::{RgbFrame, Standard};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible,
};
use windows::core::{BOOL, IInspectable, Interface};

fn hwnd(h: u64) -> HWND {
    HWND(h as usize as *mut _)
}

fn window_title(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// キャプチャするウィンドウを決める。一覧から選んだハンドルがまだ生きていればそれを、
/// なければ (開き直しなど) タイトルが一致する見えているウィンドウを使う
pub fn resolve_window(title: &str, handle: Option<u64>) -> Result<u64, String> {
    if let Some(h) = handle.filter(|&h| h != 0)
        && unsafe { IsWindow(Some(hwnd(h))) }.as_bool()
    {
        return Ok(h);
    }
    struct Find<'a> {
        title: &'a str,
        found: Option<u64>,
    }
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
        let f = unsafe { &mut *(lp.0 as *mut Find) };
        if unsafe { IsWindowVisible(h) }.as_bool() && window_title(h) == f.title {
            f.found = Some(h.0 as usize as u64);
            return false.into();
        }
        true.into()
    }
    let mut f = Find { title, found: None };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut f as *mut Find as isize));
    }
    f.found
        .ok_or_else(|| format!("ウィンドウ「{title}」が見つかりません"))
}

/// ウィンドウを作ったプロセスの ID (音声のループバックに使う)
pub fn window_pid(handle: u64) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd(handle), Some(&mut pid)) };
    pid
}

/// 取り込んだクライアント領域 (BGRA)
pub struct BgraFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// キャプチャ中のフレーム内でのクライアント領域 (x, y, 幅, 高さ)。
/// WGC はウィンドウの枠 (DWM の拡張フレーム) ごと取り込むので、そこからの位置を求める
fn client_area(h: HWND, content: SizeInt32) -> (u32, u32, u32, u32) {
    let whole = (
        0,
        0,
        content.Width.max(0) as u32,
        content.Height.max(0) as u32,
    );
    unsafe {
        let mut frame = RECT::default();
        let mut client = RECT::default();
        let mut origin = POINT::default();
        if DwmGetWindowAttribute(
            h,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut frame as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_err()
            || GetClientRect(h, &mut client).is_err()
            || !ClientToScreen(h, &mut origin).as_bool()
        {
            return whole;
        }
        let x = (origin.x - frame.left).clamp(0, content.Width);
        let y = (origin.y - frame.top).clamp(0, content.Height);
        let w = client.right.min(content.Width - x);
        let h = client.bottom.min(content.Height - y);
        if w <= 0 || h <= 0 {
            return whole;
        }
        (x as u32, y as u32, w as u32, h as u32)
    }
}

struct CopyState {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    d3d: IDirect3DDevice,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    size: SizeInt32,
}

// D3D11 のデバイスコンテキストはスレッドセーフではないが、Mutex 越しにしか触らない
unsafe impl Send for CopyState {}

/// WGC のキャプチャセッション。最新のフレームを `latest` に置き続ける
pub struct WindowCapture {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    pub latest: Arc<Mutex<Option<Arc<BgraFrame>>>>,
    pub closed: Arc<AtomicBool>,
}

impl WindowCapture {
    pub fn start(handle: u64) -> Result<Self, String> {
        Self::start_inner(handle).map_err(|e| format!("ウィンドウをキャプチャできません: {e}"))
    }

    fn start_inner(handle: u64) -> windows::core::Result<Self> {
        let h = hwnd(handle);
        unsafe {
            // 既に初期化済み (別の方式でも) なら失敗するが、そのまま使える
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(h)? };

        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        let device = device.unwrap();
        let context = context.unwrap();
        let dxgi: IDXGIDevice = device.cast()?;
        let d3d: IDirect3DDevice =
            unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;

        let format = DirectXPixelFormat::B8G8R8A8UIntNormalized;
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d, format, 2, size)?;
        let latest: Arc<Mutex<Option<Arc<BgraFrame>>>> = Arc::new(Mutex::new(None));
        let closed = Arc::new(AtomicBool::new(false));

        let state = Mutex::new(CopyState {
            device,
            context,
            d3d,
            staging: None,
            size,
        });
        let out = latest.clone();
        pool.FrameArrived(
            &TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |pool, _| {
                let h = hwnd(handle);
                let Some(pool) = pool.as_ref() else {
                    return Ok(());
                };
                let frame = pool.TryGetNextFrame()?;
                let content = frame.ContentSize()?;
                let mut st = state.lock().unwrap();
                if content != st.size {
                    // ウィンドウの大きさが変わった: 次のフレームから新しい大きさで受け取る
                    st.size = content;
                    pool.Recreate(&st.d3d, format, 2, content)?;
                }
                if unsafe { IsIconic(h) }.as_bool() {
                    return Ok(());
                }
                let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
                let tex: ID3D11Texture2D = unsafe { access.GetInterface()? };
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                unsafe { tex.GetDesc(&mut desc) };
                if !matches!(&st.staging, Some((_, w, h)) if (*w, *h) == (desc.Width, desc.Height))
                {
                    let sd = D3D11_TEXTURE2D_DESC {
                        Usage: D3D11_USAGE_STAGING,
                        BindFlags: 0,
                        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                        MiscFlags: 0,
                        ..desc
                    };
                    let mut t = None;
                    unsafe { st.device.CreateTexture2D(&sd, None, Some(&mut t))? };
                    st.staging = Some((t.unwrap(), desc.Width, desc.Height));
                }
                let staging = &st.staging.as_ref().unwrap().0;
                let context = &st.context;
                let content = SizeInt32 {
                    Width: content.Width.min(desc.Width as i32),
                    Height: content.Height.min(desc.Height as i32),
                };
                let (x, y, w, hh) = client_area(h, content);
                if w == 0 || hh == 0 {
                    return Ok(());
                }
                let mut data = vec![0u8; (w * hh * 4) as usize];
                unsafe {
                    context.CopyResource(staging, &tex);
                    let mut map = D3D11_MAPPED_SUBRESOURCE::default();
                    context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut map))?;
                    let pitch = map.RowPitch as usize;
                    let src = std::slice::from_raw_parts(
                        map.pData as *const u8,
                        pitch * desc.Height as usize,
                    );
                    let row = (w * 4) as usize;
                    for r in 0..hh as usize {
                        let s = (y as usize + r) * pitch + x as usize * 4;
                        data[r * row..(r + 1) * row].copy_from_slice(&src[s..s + row]);
                    }
                    context.Unmap(staging, 0);
                }
                *out.lock().unwrap() = Some(Arc::new(BgraFrame {
                    width: w,
                    height: hh,
                    data,
                }));
                Ok(())
            }),
        )?;
        let c = closed.clone();
        item.Closed(
            &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                c.store(true, Ordering::Relaxed);
                Ok(())
            }),
        )?;

        let session = pool.CreateCaptureSession(&item)?;
        // 黄色い枠を出さない (対応していない Windows では無視される)
        let _ = session.SetIsBorderRequired(false);
        let _ = session.SetIsCursorCaptureEnabled(true);
        session.StartCapture()?;
        Ok(Self {
            session,
            pool,
            latest,
            closed,
        })
    }
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// BGRA の生フレームを受けて縮小・RGB24 に変換する ffmpeg。入力の大きさが変わったら作り直す
struct Scaler {
    child: Child,
    stdin: Option<ChildStdin>,
    size: (u32, u32),
}

impl Scaler {
    fn spawn(
        size: (u32, u32),
        scaling: ScalingMethod,
        tv: &Standard,
        tx: SyncSender<RgbFrame>,
        lossy: bool,
    ) -> Result<Self, String> {
        let (iw, ih) = tv.image_size();
        let (n, d) = tv.field_rate_ratio;
        let mut cmd = command(&ffmpeg_path());
        cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
        cmd.args([
            "-f",
            "rawvideo",
            "-pix_fmt",
            "bgra",
            "-s",
            &format!("{}x{}", size.0, size.1),
            "-framerate",
            &format!("{n}/{d}"),
            "-i",
            "-",
            "-vf",
            &format!("scale={iw}:{ih}:flags={}", scaling.ffmpeg_flags()),
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "-",
        ]);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("ffmpeg を起動できません: {e}"))?;
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::BufReader::new(stderr).read_to_string(&mut s);
            if !s.trim().is_empty() {
                eprintln!("[ffmpeg window] {}", s.trim());
            }
        });
        std::thread::spawn(move || {
            let bytes = iw * ih * 3;
            loop {
                let mut buf = vec![0u8; bytes];
                if stdout.read_exact(&mut buf).is_err() {
                    break;
                }
                let frame = RgbFrame::from_raw(iw, ih, buf).unwrap();
                let r = if lossy {
                    !matches!(tx.try_send(frame), Err(TrySendError::Disconnected(_)))
                } else {
                    tx.send(frame).is_ok()
                };
                if !r {
                    break;
                }
            }
        });
        Ok(Self { child, stdin, size })
    }
}

impl Drop for Scaler {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// ウィンドウキャプチャを止めるためのもの (FrameStream が持つ)
struct Pump {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// `crop` = (x, y, 幅, 高さ) はクライアント領域の左上から。幅・高さとも 0 なら全体
pub fn open_stream(
    handle: u64,
    crop: (u32, u32, u32, u32),
    scaling: ScalingMethod,
    lossy: bool,
    tv: &'static Standard,
) -> Result<FrameStream, String> {
    let (cx, cy, cw, ch) = crop;
    if (cw == 0) != (ch == 0) {
        return Err("クロップ幅と高さは 1 以上にしてください".into());
    }
    let capture = WindowCapture::start(handle)?;
    let (tx, rx) = sync_channel::<RgbFrame>(2);
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let thread = std::thread::Builder::new()
        .name("vhs-window".into())
        .spawn(move || {
            let (n, d) = tv.field_rate_ratio;
            let period = Duration::from_nanos(1_000_000_000 * d as u64 / n as u64);
            let mut next = Instant::now();
            let mut scaler: Option<Scaler> = None;
            let mut buf = Vec::new();
            while !stop2.load(Ordering::Relaxed) && !capture.closed.load(Ordering::Relaxed) {
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                }
                next = (next + period).max(Instant::now() - period);
                let Some(f) = capture.latest.lock().unwrap().clone() else {
                    continue;
                };
                // 切り抜き (はみ出す分は詰める)
                let (x, y, w, h) = if cw == 0 {
                    (0, 0, f.width, f.height)
                } else {
                    let x = cx.min(f.width.saturating_sub(1));
                    let y = cy.min(f.height.saturating_sub(1));
                    (x, y, cw.min(f.width - x), ch.min(f.height - y))
                };
                if w == 0 || h == 0 {
                    continue;
                }
                // 奇数幅の rawvideo も扱えるが、縮小フィルタの都合で偶数に揃える
                let (w, h) = ((w & !1).max(2).min(f.width), (h & !1).max(2).min(f.height));
                if scaler.as_ref().is_none_or(|s| s.size != (w, h)) {
                    scaler = None;
                    match Scaler::spawn((w, h), scaling, tv, tx.clone(), lossy) {
                        Ok(s) => scaler = Some(s),
                        Err(e) => {
                            eprintln!("{e}");
                            break;
                        }
                    }
                }
                buf.clear();
                let stride = f.width as usize * 4;
                for r in y..y + h {
                    let s = r as usize * stride + x as usize * 4;
                    buf.extend_from_slice(&f.data[s..s + w as usize * 4]);
                }
                let s = scaler.as_mut().unwrap();
                if s.stdin.as_mut().is_none_or(|i| i.write_all(&buf).is_err()) {
                    break;
                }
            }
            drop(scaler);
            drop(capture);
        })
        .map_err(|e| e.to_string())?;
    Ok(FrameStream::from_parts(
        rx,
        Box::new(Pump {
            stop,
            thread: Some(thread),
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 実在のウィンドウが必要:
    /// `VHS_TEST_WINDOW=タイトル VHS_TEST_OUT=out.png cargo test -p vhs-next wgc -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn captures_window_by_title() {
        unsafe {
            let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
                windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            );
        }
        let title = std::env::var("VHS_TEST_WINDOW").expect("VHS_TEST_WINDOW");
        let h = resolve_window(&title, None).unwrap();
        println!("hwnd {h:#x} pid {}", window_pid(h));
        let tv = vhs_core::Params::default().tv();
        let stream = open_stream(h, (0, 0, 0, 0), ScalingMethod::Area, false, tv).unwrap();
        let t = Instant::now();
        let mut n = 0;
        let mut last = None;
        while t.elapsed() < Duration::from_secs(2) {
            if let Ok(f) = stream.rx.recv_timeout(Duration::from_millis(500)) {
                n += 1;
                last = Some(f);
            }
        }
        println!("{n} frames in 2 s");
        assert!(n > 60, "{n}");
        let f = last.unwrap();
        let mean = f.data.iter().map(|&b| b as u64).sum::<u64>() / f.data.len() as u64;
        println!("mean {mean}");
        if let Ok(out) = std::env::var("VHS_TEST_OUT") {
            image::RgbImage::from_raw(f.width as u32, f.height as u32, f.data.clone())
                .unwrap()
                .save(out)
                .unwrap();
        }
        assert!(mean > 5, "真っ黒");
    }
}
