//! キャプチャできるトップレベルウィンドウの一覧 (Windows のみ)

#[derive(serde::Serialize)]
pub struct WindowInfo {
    pub title: String,
    /// ウィンドウハンドル (キャプチャを開くときに渡す)
    pub hwnd: u64,
    /// クライアント領域の大きさ (キャプチャする範囲)
    pub width: i32,
    pub height: i32,
}

#[cfg(not(windows))]
pub fn list_windows() -> Vec<WindowInfo> {
    Vec::new()
}

#[cfg(windows)]
pub fn list_windows() -> Vec<WindowInfo> {
    use std::ffi::c_void;
    type Hwnd = *mut c_void;

    #[repr(C)]
    #[derive(Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(cb: unsafe extern "system" fn(Hwnd, isize) -> i32, lp: isize) -> i32;
        fn IsWindowVisible(h: Hwnd) -> i32;
        fn GetWindowTextLengthW(h: Hwnd) -> i32;
        fn GetWindowTextW(h: Hwnd, buf: *mut u16, n: i32) -> i32;
        fn GetClientRect(h: Hwnd, r: *mut Rect) -> i32;
        fn GetWindowLongW(h: Hwnd, idx: i32) -> i32;
        fn GetWindowThreadProcessId(h: Hwnd, pid: *mut u32) -> u32;
    }
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmGetWindowAttribute(h: Hwnd, attr: u32, out: *mut c_void, size: u32) -> i32;
    }

    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TOOLWINDOW: i32 = 0x80;
    const DWMWA_CLOAKED: u32 = 14;

    unsafe extern "system" fn cb(h: Hwnd, lp: isize) -> i32 {
        let out = unsafe { &mut *(lp as *mut Vec<WindowInfo>) };
        unsafe {
            if IsWindowVisible(h) == 0 {
                return 1;
            }
            if GetWindowLongW(h, GWL_EXSTYLE) & WS_EX_TOOLWINDOW != 0 {
                return 1;
            }
            // 別の仮想デスクトップや UWP の隠れたウィンドウ
            let mut cloaked = 0u32;
            if DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut c_void, 4)
                == 0
                && cloaked != 0
            {
                return 1;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, &mut pid);
            if pid == std::process::id() {
                return 1;
            }
            let len = GetWindowTextLengthW(h);
            if len <= 0 {
                return 1;
            }
            let mut buf = vec![0u16; len as usize + 1];
            let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
            let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
            if title.trim().is_empty() {
                return 1;
            }
            let mut r = Rect::default();
            GetClientRect(h, &mut r);
            let (width, height) = (r.right - r.left, r.bottom - r.top);
            if width <= 0 || height <= 0 {
                return 1;
            }
            out.push(WindowInfo {
                title,
                hwnd: h as usize as u64,
                width,
                height,
            });
        }
        1
    }

    let mut out = Vec::new();
    unsafe { EnumWindows(cb, &mut out as *mut Vec<WindowInfo> as isize) };
    out
}
