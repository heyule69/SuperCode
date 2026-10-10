//! Native, non-activating desktop cue and pointer halo: no WebView/full-screen bitmap.
use std::sync::{mpsc, Arc};
use tauri::AppHandle;
static INPUT_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) fn input_epoch() -> u64 {
    INPUT_EPOCH.load(std::sync::atomic::Ordering::Acquire)
}

pub struct Glow {
    sender: mpsc::Sender<Cue>,
    worker: Option<std::thread::JoinHandle<()>>,
}
struct Cue {
    visible: bool,
    escape: bool,
    epoch: u64,
    reply: mpsc::SyncSender<Result<Status, String>>,
}
#[derive(Clone, Copy, Default, serde::Serialize)]
pub struct Status {
    pub edges: usize,
    pub pointer: bool,
    pub keyboard: bool,
    pub mouse: bool,
}
impl Glow {
    pub fn new(app: AppHandle, gate: Arc<crate::computer_gate::Gate>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || native::run(app, receiver, gate));
        Self {
            sender,
            worker: Some(worker),
        }
    }
    pub fn set(&self, visible: bool, escape: bool, epoch: u64) -> Result<Status, String> {
        let (reply, ready) = mpsc::sync_channel(1);
        self.sender
            .send(Cue {
                visible,
                escape,
                epoch,
                reply,
            })
            .map_err(|_| "电脑光效与停止机制已关闭")?;
        native::wake();
        ready
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| "电脑光效与停止机制未及时就绪，已阻止操作")?
    }
}
impl Drop for Glow {
    fn drop(&mut self) {
        // Closing and joining before a new lease starts prevents the previous
        // cue thread from clearing the next thread's input hook/gate handles.
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        drop(std::mem::replace(&mut self.sender, sender));
        native::wake();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
pub(crate) fn pointer(x: i32, y: i32, click: bool) {
    native::pointer(x, y, click);
}

fn alpha(distance: i32, thickness: i32) -> u8 {
    let t = (1.0 - distance as f32 / thickness as f32).clamp(0.0, 1.0);
    (190.0 * t * t) as u8
}
fn stop_escape(key: u32, tag: usize) -> bool {
    key == 0x1b && tag != crate::computer_gate::INPUT_TAG
}
#[cfg(not(windows))]
mod native {
    pub(super) fn wake() {}
    pub(super) fn pointer(_: i32, _: i32, _: bool) {}
    pub(super) fn run(
        _: tauri::AppHandle,
        receiver: std::sync::mpsc::Receiver<super::Cue>,
        _: std::sync::Arc<crate::computer_gate::Gate>,
    ) {
        while let Ok(cue) = receiver.recv() {
            let _ = cue.reply.send(Err("电脑光效仅支持 Windows".into()));
        }
    }
}
#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ptr::{null, null_mut},
        time::Instant,
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM},
        Graphics::Gdi::*,
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::{GetCurrentThreadId, SetEvent, WaitForSingleObject},
        },
        UI::{
            HiDpi::*,
            Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT, VK_ESCAPE},
            Input::*,
            WindowsAndMessaging::*,
        },
    };
    const CLASS: &[u16] = &[
        83, 117, 112, 101, 114, 67, 111, 100, 101, 68, 101, 115, 107, 116, 111, 112, 71, 108, 111,
        119, 0,
    ];
    const STOP: u32 = WM_APP + 0x43;
    const CLICK: u32 = WM_APP + 0x44;
    const WAKE: u32 = WM_APP + 0x45;
    const POINTER_SIZE: usize = 160;
    const POINTER_HOTSPOT: i32 = (POINTER_SIZE / 2) as i32;
    static HOOK_THREAD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    static HOOK_GATE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static KEY_EVENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static MOUSE_EVENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static AI_X: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    static AI_Y: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    pub(super) fn pointer(x: i32, y: i32, click: bool) {
        AI_X.store(x, std::sync::atomic::Ordering::Release);
        AI_Y.store(y, std::sync::atomic::Ordering::Release);
        unsafe {
            PostThreadMessageW(
                HOOK_THREAD.load(std::sync::atomic::Ordering::Acquire),
                if click { CLICK } else { WAKE },
                0,
                0,
            );
        }
    }
    pub(super) fn input_counts() -> (u64, u64) {
        (
            KEY_EVENTS.load(std::sync::atomic::Ordering::Relaxed),
            MOUSE_EVENTS.load(std::sync::atomic::Ordering::Relaxed),
        )
    }
    /// The input loop never renders, reads storage, waits on the UI, or runs tools.
    /// Keeping hooks on this dedicated thread prevents LowLevelHooksTimeout removal.
    struct Hooks {
        thread: u32,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    impl Hooks {
        fn start(gate: Arc<crate::computer_gate::Gate>) -> Result<Self, String> {
            let (tx, rx) = mpsc::sync_channel(1);
            let worker = std::thread::spawn(move || unsafe {
                let _gate = gate;
                let mut msg: MSG = std::mem::zeroed();
                PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
                let input_window = CreateWindowExW(
                    0,
                    CLASS.as_ptr(),
                    [0u16].as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    null_mut(),
                    GetModuleHandleW(null()),
                    null(),
                );
                if input_window.is_null() {
                    let _ = tx.send(Err("无法创建后台键鼠停止监听".to_string()));
                    return;
                }
                let mut original = [std::mem::zeroed::<RAWINPUTDEVICE>(); 16];
                let mut count = original.len() as u32;
                let registered = GetRegisteredRawInputDevices(
                    original.as_mut_ptr(),
                    &mut count,
                    std::mem::size_of::<RAWINPUTDEVICE>() as u32,
                );
                let original = if registered == u32::MAX {
                    vec![]
                } else {
                    original[..count.min(16) as usize]
                        .iter()
                        .filter(|d| d.usUsagePage == 1 && matches!(d.usUsage, 2 | 6))
                        .copied()
                        .collect::<Vec<_>>()
                };
                let devices = [2, 6].map(|usage| RAWINPUTDEVICE {
                    usUsagePage: 1,
                    usUsage: usage,
                    dwFlags: RIDEV_INPUTSINK,
                    hwndTarget: input_window,
                });
                let raw_ready = RegisterRawInputDevices(
                    devices.as_ptr(),
                    2,
                    std::mem::size_of::<RAWINPUTDEVICE>() as u32,
                ) != 0;
                let hotkey_ready =
                    RegisterHotKey(input_window, 1, MOD_NOREPEAT, VK_ESCAPE as u32) != 0;
                let key =
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), GetModuleHandleW(null()), 0);
                let pointer =
                    SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), GetModuleHandleW(null()), 0);
                if key.is_null() || pointer.is_null() || !raw_ready || !hotkey_ready {
                    if !key.is_null() {
                        UnhookWindowsHookEx(key);
                    }
                    if !pointer.is_null() {
                        UnhookWindowsHookEx(pointer);
                    }
                    cleanup_input(input_window, &original);
                    let _ = tx.send(Err(
                        "无法启用键鼠接管或全局 Esc 停止，已阻止电脑操作".to_string()
                    ));
                    return;
                }
                if tx.send(Ok(GetCurrentThreadId())).is_err() {
                    UnhookWindowsHookEx(key);
                    UnhookWindowsHookEx(pointer);
                    cleanup_input(input_window, &original);
                    return;
                }
                while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                UnhookWindowsHookEx(key);
                UnhookWindowsHookEx(pointer);
                cleanup_input(input_window, &original);
            });
            let thread = rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .map_err(|_| "键鼠停止监听未及时就绪，已阻止操作")??;
            Ok(Self {
                thread,
                worker: Some(worker),
            })
        }
        fn ready(&self) -> bool {
            self.worker
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
        }
    }
    impl Drop for Hooks {
        fn drop(&mut self) {
            unsafe {
                PostThreadMessageW(self.thread, WM_QUIT, 0, 0);
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
    unsafe fn cleanup_input(window: HWND, original: &[RAWINPUTDEVICE]) {
        UnregisterHotKey(window, 1);
        let remove = [2, 6].map(|usage| RAWINPUTDEVICE {
            usUsagePage: 1,
            usUsage: usage,
            dwFlags: RIDEV_REMOVE,
            hwndTarget: null_mut(),
        });
        RegisterRawInputDevices(
            remove.as_ptr(),
            2,
            std::mem::size_of::<RAWINPUTDEVICE>() as u32,
        );
        if !original.is_empty() {
            RegisterRawInputDevices(
                original.as_ptr(),
                original.len() as u32,
                std::mem::size_of::<RAWINPUTDEVICE>() as u32,
            );
        }
        DestroyWindow(window);
    }
    unsafe fn raw_input(l: LPARAM) {
        let mut raw: RAWINPUT = std::mem::zeroed();
        let mut size = std::mem::size_of::<RAWINPUT>() as u32;
        let read = GetRawInputData(
            l as _,
            RID_INPUT,
            (&mut raw as *mut RAWINPUT).cast(),
            &mut size,
            std::mem::size_of::<RAWINPUTHEADER>() as u32,
        );
        if let Some(bring_back) = hardware_takeover(&raw, read) {
            yield_input(
                bring_back,
                if bring_back {
                    1
                } else if raw.header.dwType == RIM_TYPEKEYBOARD {
                    2
                } else {
                    3
                },
            );
        }
    }
    unsafe fn hardware_takeover(raw: &RAWINPUT, read: u32) -> Option<bool> {
        if read == u32::MAX || raw.header.hDevice.is_null() {
            return None;
        }
        if raw.header.dwType == RIM_TYPEKEYBOARD {
            if read
                < (std::mem::size_of::<RAWINPUTHEADER>() + std::mem::size_of::<RAWKEYBOARD>())
                    as u32
            {
                return None;
            }
            let key = raw.data.keyboard;
            if key.Flags & RI_KEY_BREAK as u16 == 0
                && key.ExtraInformation != crate::computer_gate::INPUT_TAG as u32
            {
                return Some(key.VKey == VK_ESCAPE);
            }
        } else if raw.header.dwType == RIM_TYPEMOUSE {
            if read < std::mem::size_of::<RAWINPUT>() as u32 {
                return None;
            }
            let mouse = raw.data.mouse;
            if mouse.ulExtraInformation != crate::computer_gate::INPUT_TAG as u32
                && (mouse.lLastX != 0
                    || mouse.lLastY != 0
                    || mouse.Anonymous.Anonymous.usButtonFlags & 0x0d55 != 0)
            {
                return Some(false);
            }
        }
        None
    }
    #[cfg(test)]
    #[test]
    fn hardware_takeover_ignores_releases_and_our_input_and_checks_packet_size() {
        unsafe {
            let mut raw: RAWINPUT = std::mem::zeroed();
            raw.header.hDevice = 1usize as _;
            raw.header.dwType = RIM_TYPEKEYBOARD;
            raw.data.keyboard.VKey = VK_ESCAPE;
            let size = std::mem::size_of::<RAWINPUT>() as u32;
            assert_eq!(hardware_takeover(&raw, size), Some(true));
            raw.data.keyboard.VKey = 0x7b;
            assert_eq!(hardware_takeover(&raw, size), Some(false));
            raw.data.keyboard.Flags = RI_KEY_BREAK as u16;
            assert_eq!(hardware_takeover(&raw, size), None);
            raw.data.keyboard.Flags = 0;
            raw.data.keyboard.ExtraInformation = crate::computer_gate::INPUT_TAG as u32;
            assert_eq!(hardware_takeover(&raw, size), None);
            raw.data.keyboard.ExtraInformation = 0;
            assert_eq!(hardware_takeover(&raw, 2), None);
            raw.header.hDevice = null_mut();
            assert_eq!(hardware_takeover(&raw, size), None);
            raw.header.hDevice = 1usize as _;
            raw.header.dwType = RIM_TYPEMOUSE;
            raw.data.mouse = std::mem::zeroed();
            raw.data.mouse.lLastX = 1;
            assert_eq!(hardware_takeover(&raw, size), Some(false));
            raw.data.mouse.ulExtraInformation = crate::computer_gate::INPUT_TAG as u32;
            assert_eq!(hardware_takeover(&raw, size), None);
        }
    }
    const WIDTH: i32 = 96;
    pub(super) fn wake() {
        unsafe {
            PostThreadMessageW(
                HOOK_THREAD.load(std::sync::atomic::Ordering::Acquire),
                WAKE,
                0,
                0,
            );
        }
    }
    unsafe fn stopped() -> bool {
        WaitForSingleObject(HOOK_GATE.load(std::sync::atomic::Ordering::Acquire) as _, 0)
            != windows_sys::Win32::Foundation::WAIT_TIMEOUT
    }
    unsafe fn yield_input(bring_back: bool, reason: isize) {
        INPUT_EPOCH.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        SetEvent(HOOK_GATE.load(std::sync::atomic::Ordering::Acquire) as _);
        PostThreadMessageW(
            HOOK_THREAD.load(std::sync::atomic::Ordering::Acquire),
            STOP,
            usize::from(bring_back),
            reason,
        );
    }
    unsafe extern "system" fn keyboard(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code == 0 {
            KEY_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let key = &*(l as *const KBDLLHOOKSTRUCT);
            let down = matches!(w as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if key.dwExtraInfo == crate::computer_gate::INPUT_TAG {
                if down && stopped() {
                    return 1;
                }
            } else if down {
                // A kernel signal stops workers/queued injected input before any UI work.
                yield_input(
                    stop_escape(key.vkCode, key.dwExtraInfo),
                    if key.vkCode == VK_ESCAPE as u32 { 1 } else { 2 },
                );
                if stop_escape(key.vkCode, key.dwExtraInfo) {
                    return 1;
                }
            }
        }
        CallNextHookEx(null_mut(), code, w, l)
    }
    unsafe extern "system" fn mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if code == 0 {
            MOUSE_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let event = &*(l as *const MSLLHOOKSTRUCT);
            let action = matches!(
                w as u32,
                WM_MOUSEMOVE
                    | WM_LBUTTONDOWN
                    | WM_RBUTTONDOWN
                    | WM_MBUTTONDOWN
                    | WM_MOUSEWHEEL
                    | WM_MOUSEHWHEEL
            );
            if event.dwExtraInfo == crate::computer_gate::INPUT_TAG {
                if action && stopped() {
                    return 1;
                }
                AI_X.store(event.pt.x, std::sync::atomic::Ordering::Release);
                AI_Y.store(event.pt.y, std::sync::atomic::Ordering::Release);
                if matches!(w as u32, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
                    PostThreadMessageW(
                        HOOK_THREAD.load(std::sync::atomic::Ordering::Acquire),
                        CLICK,
                        0,
                        0,
                    );
                }
            } else if action {
                yield_input(false, 3);
            }
        }
        CallNextHookEx(null_mut(), code, w, l)
    }
    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
    ) -> LRESULT {
        match message {
            WM_HOTKEY if w == 1 => {
                yield_input(true, 1);
                0
            }
            WM_INPUT => {
                raw_input(l);
                DefWindowProcW(window, message, w, l)
            }
            WM_NCHITTEST => HTTRANSPARENT as isize,
            WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
            _ => DefWindowProcW(window, message, w, l),
        }
    }
    unsafe extern "system" fn monitor(_: HMONITOR, _: HDC, rect: *mut RECT, data: LPARAM) -> i32 {
        (*(data as *mut Vec<RECT>)).push(*rect);
        1
    }
    struct Strip {
        window: HWND,
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
        origin: POINT,
        size: SIZE,
        pixels: *mut u8,
    }
    impl Strip {
        unsafe fn new(rect: RECT, screen: RECT) -> Result<Self, String> {
            Self::create(rect, screen, false)
        }
        unsafe fn halo() -> Result<Self, String> {
            let mut p: POINT = std::mem::zeroed();
            GetCursorPos(&mut p);
            AI_X.store(p.x, std::sync::atomic::Ordering::Release);
            AI_Y.store(p.y, std::sync::atomic::Ordering::Release);
            let r = RECT {
                left: p.x - POINTER_HOTSPOT,
                top: p.y - POINTER_HOTSPOT,
                right: p.x + POINTER_HOTSPOT,
                bottom: p.y + POINTER_HOTSPOT,
            };
            Self::create(r, r, true)
        }
        unsafe fn create(rect: RECT, screen: RECT, halo: bool) -> Result<Self, String> {
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            let window = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                CLASS.as_ptr(),
                [0u16].as_ptr(),
                WS_POPUP,
                rect.left,
                rect.top,
                width,
                height,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            if window.is_null() {
                return Err("无法创建电脑操作光效".into());
            }
            let dc = CreateCompatibleDC(null_mut());
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut pixels = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
            if dc.is_null() || bitmap.is_null() || pixels.is_null() {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                if !dc.is_null() {
                    DeleteDC(dc);
                }
                DestroyWindow(window);
                return Err("无法绘制电脑操作光效".into());
            }
            let old = SelectObject(dc, bitmap);
            let buffer = std::slice::from_raw_parts_mut(
                pixels.cast::<u8>(),
                width as usize * height as usize * 4,
            );
            for y in 0..height {
                for x in 0..width {
                    let gx = rect.left + x;
                    let gy = rect.top + y;
                    let distance = (gx - screen.left)
                        .min(screen.right - 1 - gx)
                        .min(gy - screen.top)
                        .min(screen.bottom - 1 - gy);
                    let radius = ((x as f32 - POINTER_HOTSPOT as f32).powi(2)
                        + (y as f32 - POINTER_HOTSPOT as f32).powi(2))
                    .sqrt();
                    let a = if halo {
                        (110.0 * (1.0 - radius / 70.0).clamp(0.0, 1.0).powi(2)) as u8
                    } else {
                        alpha(distance, WIDTH)
                    };
                    let i = (y as usize * width as usize + x as usize) * 4;
                    // DIB BGRA is premultiplied for AC_SRC_ALPHA.
                    buffer[i] = a;
                    buffer[i + 1] = (196u16 * a as u16 / 255) as u8;
                    buffer[i + 2] = (132u16 * a as u16 / 255) as u8;
                    buffer[i + 3] = a;
                }
            }
            // Keep the visual cue out of supported Windows screenshot APIs.
            SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE);
            let strip = Self {
                window,
                dc,
                bitmap,
                old,
                origin: POINT {
                    x: rect.left,
                    y: rect.top,
                },
                size: SIZE {
                    cx: width,
                    cy: height,
                },
                pixels: pixels.cast(),
            };
            if !strip.paint(255) {
                return Err("无法显示电脑操作光效".into());
            }
            ShowWindow(window, SW_SHOWNOACTIVATE);
            Ok(strip)
        }
        unsafe fn paint(&self, opacity: u8) -> bool {
            let source = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: opacity,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let painted = UpdateLayeredWindow(
                self.window,
                null_mut(),
                &self.origin,
                &self.size,
                self.dc,
                &source,
                0,
                &blend,
                ULW_ALPHA,
            ) != 0;
            // Foreground/focus changes can reorder topmost windows. Reassert
            // the cue above the target without ever activating it.
            painted
                && SetWindowPos(
                    self.window,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                ) != 0
        }
        unsafe fn show(&self, visible: bool) {
            ShowWindow(
                self.window,
                if visible { SW_SHOWNOACTIVATE } else { SW_HIDE },
            );
        }
        unsafe fn follow(&mut self, click: Option<Instant>) -> bool {
            self.origin = POINT {
                x: AI_X.load(std::sync::atomic::Ordering::Acquire) - POINTER_HOTSPOT,
                y: AI_Y.load(std::sync::atomic::Ordering::Acquire) - POINTER_HOTSPOT,
            };
            let progress = click
                .map(|t| t.elapsed().as_secs_f32() / 0.38)
                .filter(|v| *v < 1.0);
            let buffer =
                std::slice::from_raw_parts_mut(self.pixels, POINTER_SIZE * POINTER_SIZE * 4);
            let pointer = pointer_art();
            for y in 0..POINTER_SIZE {
                for x in 0..POINTER_SIZE {
                    let r = ((x as f32 - POINTER_HOTSPOT as f32).powi(2)
                        + (y as f32 - (POINTER_HOTSPOT + 8) as f32).powi(2))
                    .sqrt();
                    let mut a = 110.0 * (1.0 - r / 70.0).clamp(0.0, 1.0).powi(2);
                    if let Some(t) = progress {
                        let radius = 9.0 + (1.0 - (1.0 - t).powi(3)) * 25.0;
                        a += 105.0
                            * (1.0 - t).powi(2)
                            * (1.0 - (r - radius).abs() / 1.8).clamp(0.0, 1.0);
                    }
                    let a = a.min(255.0) as u8;
                    let i = (y * POINTER_SIZE + x) * 4;
                    let art = pointer[y * POINTER_SIZE + x];
                    let inverse = 255 - art[3] as u16;
                    for (channel, color) in [240u16, 160, 68].into_iter().enumerate() {
                        buffer[i + channel] = (art[channel] as u16
                            + color * a as u16 / 255 * inverse / 255)
                            .min(255) as u8;
                    }
                    buffer[i + 3] = (art[3] as u16 + a as u16 * inverse / 255).min(255) as u8;
                }
            }
            self.paint(255)
        }
    }
    fn arrow_distance(x: f32, y: f32) -> (bool, f32) {
        let points = pointer_outline();
        let mut inside = false;
        let mut distance = f32::MAX;
        let mut previous = points.len() - 1;
        for (i, &(px, py)) in points.iter().enumerate() {
            let (qx, qy) = points[previous];
            if (py > y) != (qy > y) && x < (qx - px) * (y - py) / (qy - py) + px {
                inside = !inside;
            }
            let t = (((x - px) * (qx - px) + (y - py) * (qy - py))
                / ((qx - px).powi(2) + (qy - py).powi(2)))
            .clamp(0.0, 1.0);
            distance = distance
                .min(((x - px - t * (qx - px)).powi(2) + (y - py - t * (qy - py)).powi(2)).sqrt());
            previous = i;
        }
        (inside, distance)
    }
    fn pointer_outline() -> &'static [(f32, f32)] {
        static OUTLINE: std::sync::OnceLock<Vec<(f32, f32)>> = std::sync::OnceLock::new();
        OUTLINE.get_or_init(|| {
            fn curve(points: &mut Vec<(f32, f32)>, control: (f32, f32), end: (f32, f32)) {
                let start = *points.last().unwrap();
                for step in 1..=12 {
                    let t = step as f32 / 12.0;
                    let s = 1.0 - t;
                    points.push((
                        s * s * start.0 + 2.0 * s * t * control.0 + t * t * end.0,
                        s * s * start.1 + 2.0 * s * t * control.1 + t * t * end.1,
                    ));
                }
            }
            // A compact, rounded directional pointer. The notch replaces the
            // standard cursor's long stem so it remains distinct at a glance.
            let mut points = vec![(84.0, 80.0)];
            curve(&mut points, (80.0, 79.0), (81.0, 84.0));
            points.push((88.0, 110.0));
            curve(&mut points, (89.0, 115.0), (92.0, 110.0));
            points.push((97.0, 100.0));
            curve(&mut points, (98.0, 98.0), (100.0, 97.0));
            points.push((108.0, 94.0));
            curve(&mut points, (114.0, 91.0), (109.0, 89.0));
            points
        })
    }
    fn pointer_art() -> &'static [[u8; 4]] {
        static ART: std::sync::OnceLock<Vec<[u8; 4]>> = std::sync::OnceLock::new();
        ART.get_or_init(|| {
            (0..POINTER_SIZE * POINTER_SIZE)
                .map(|i| {
                    // The glow is composited separately. Only rasterize the
                    // small arrow bounds; cache the result once per process.
                    if !(77..=116).contains(&(i % POINTER_SIZE))
                        || !(76..=118).contains(&(i / POINTER_SIZE))
                    {
                        return [0; 4];
                    }
                    let mut pixel = [0.0; 4];
                    for sy in [0.25, 0.75] {
                        for sx in [0.25, 0.75] {
                            let (inside, distance) = arrow_distance(
                                (i % POINTER_SIZE) as f32 + sx,
                                (i / POINTER_SIZE) as f32 + sy,
                            );
                            let (color, a) = if inside || distance <= 1.5 {
                                (
                                    if inside && distance > 1.5 {
                                        [23.0, 21.0, 19.0]
                                    } else {
                                        [221.0, 216.0, 211.0]
                                    },
                                    255.0,
                                )
                            } else {
                                (
                                    [30.0, 23.0, 16.0],
                                    30.0 * (1.0 - (distance - 1.5) / 3.0).clamp(0.0, 1.0),
                                )
                            };
                            for c in 0..3 {
                                pixel[c] += color[c] * a / 255.0 / 4.0;
                            }
                            pixel[3] += a / 4.0;
                        }
                    }
                    pixel.map(|v| v.round() as u8)
                })
                .collect()
        })
    }
    impl Drop for Strip {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.window);
                SelectObject(self.dc, self.old);
                DeleteObject(self.bitmap);
                DeleteDC(self.dc);
            }
        }
    }
    unsafe fn strips() -> Result<Vec<Strip>, String> {
        let mut screens = Vec::<RECT>::new();
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(monitor),
            (&mut screens as *mut Vec<RECT>) as isize,
        );
        let mut strips = vec![];
        for screen in screens {
            if screen.right - screen.left < WIDTH * 2 || screen.bottom - screen.top < WIDTH * 2 {
                continue;
            }
            for rect in [
                RECT {
                    bottom: screen.top + WIDTH,
                    ..screen
                },
                RECT {
                    top: screen.bottom - WIDTH,
                    ..screen
                },
                RECT {
                    top: screen.top + WIDTH,
                    bottom: screen.bottom - WIDTH,
                    right: screen.left + WIDTH,
                    ..screen
                },
                RECT {
                    top: screen.top + WIDTH,
                    bottom: screen.bottom - WIDTH,
                    left: screen.right - WIDTH,
                    ..screen
                },
            ] {
                strips.push(Strip::new(rect, screen)?);
            }
        }
        Ok(strips)
    }
    pub(super) fn run(
        app: AppHandle,
        receiver: mpsc::Receiver<Cue>,
        gate: Arc<crate::computer_gate::Gate>,
    ) {
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: GetModuleHandleW(null()),
                lpszClassName: CLASS.as_ptr(),
                ..std::mem::zeroed()
            };
            let registered = RegisterClassW(&class) != 0
                || windows_sys::Win32::Foundation::GetLastError() == 1410;
            let mut active = false;
            let mut escape = false;
            let mut windows = vec![];
            let mut hooks: Option<Hooks> = None;
            let mut halo: Option<Strip> = None;
            let mut clicked: Option<Instant> = None;
            // Create the message queue before installing hooks/posting stop messages.
            let mut message: MSG = std::mem::zeroed();
            PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
            HOOK_THREAD.store(GetCurrentThreadId(), std::sync::atomic::Ordering::Release);
            HOOK_GATE.store(gate.raw(), std::sync::atomic::Ordering::Release);
            let mut started = Instant::now();
            loop {
                let command = if active || escape {
                    match receiver.try_recv() {
                        Ok(cue) => Ok(cue),
                        Err(mpsc::TryRecvError::Disconnected) => {
                            Err(mpsc::RecvTimeoutError::Disconnected)
                        }
                        Err(mpsc::TryRecvError::Empty) => {
                            // Pump low-level hooks immediately; a channel sleep delays every SendInput.
                            MsgWaitForMultipleObjectsEx(
                                0,
                                null(),
                                16,
                                QS_ALLINPUT,
                                MWMO_INPUTAVAILABLE,
                            );
                            receiver.try_recv().map_err(|e| match e {
                                mpsc::TryRecvError::Empty => mpsc::RecvTimeoutError::Timeout,
                                mpsc::TryRecvError::Disconnected => {
                                    mpsc::RecvTimeoutError::Disconnected
                                }
                            })
                        }
                    }
                } else {
                    receiver
                        .recv()
                        .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                };
                match command {
                    Ok(next) => {
                        let result = (|| -> Result<Status, String> {
                            if next.visible {
                                if next.epoch != input_epoch() {
                                    return Err("用户已接管电脑，拒绝恢复旧的控制请求".into());
                                }
                                if !registered || !next.escape {
                                    return Err("电脑光效与停止机制未就绪".into());
                                }
                                if !active {
                                    halo = Some(Strip::halo()?);
                                    windows = strips()?;
                                    if windows.is_empty() {
                                        return Err("无法显示屏幕边缘光效".into());
                                    }
                                    hooks = Some(Hooks::start(gate.clone())?);
                                    for strip in &windows {
                                        strip.show(true);
                                    }
                                    let halo = halo.as_mut().unwrap();
                                    halo.show(true);
                                    if !halo.follow(None)
                                        || windows.iter().any(|w| {
                                            !w.paint(255) || IsWindowVisible(w.window) == 0
                                        })
                                    {
                                        return Err("电脑光效绘制失败，已阻止电脑操作".into());
                                    }
                                    gate.arm()?;
                                    if next.epoch != input_epoch() {
                                        gate.stop();
                                        return Err("用户已接管电脑，拒绝恢复旧的控制请求".into());
                                    }
                                    active = true;
                                    escape = true;
                                    started = Instant::now();
                                    windows_sys::Win32::Graphics::Dwm::DwmFlush();
                                }
                                if gate.stopped() {
                                    return Err("用户已经接管电脑".into());
                                }
                                Ok(Status {
                                    edges: windows.len(),
                                    pointer: halo.is_some(),
                                    keyboard: hooks.as_ref().is_some_and(Hooks::ready),
                                    mouse: hooks.as_ref().is_some_and(Hooks::ready),
                                })
                            } else {
                                gate.stop();
                                active = false;
                                escape = false;
                                Ok(Status::default())
                            }
                        })();
                        if result.is_err() || !next.visible {
                            gate.stop();
                            active = false;
                            escape = false;
                            hooks = None;
                            windows.clear();
                            halo = None;
                            clicked = None;
                        }
                        let _ = next.reply.send(result);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    if message.message == STOP || (message.message == WM_DISPLAYCHANGE && active) {
                        gate.stop();
                        active = false;
                        escape = false;
                        hooks = None;
                        windows.clear();
                        halo = None;
                        clicked = None;
                        let reason = if message.message == WM_DISPLAYCHANGE {
                            "display_changed"
                        } else {
                            match message.lParam {
                                1 => "escape",
                                2 => "keyboard_input",
                                3 => "pointer_input",
                                _ => "manual_input",
                            }
                        };
                        crate::desktop_control::takeover_reason(&app, message.wParam != 0, reason);
                    }
                    if message.message == CLICK {
                        clicked = Some(Instant::now());
                    }
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                if active {
                    let pulse =
                        (started.elapsed().as_secs_f32() * std::f32::consts::TAU / 2.6).cos();
                    let opacity = (222.0 + 27.0 * pulse) as u8;
                    let painted = hooks.as_ref().is_some_and(Hooks::ready)
                        && halo.as_mut().is_some_and(|h| h.follow(clicked))
                        && windows.iter().all(|w| w.paint(opacity));
                    if !painted {
                        gate.stop();
                        crate::desktop_control::takeover_reason(&app, true, "cue_render_failed");
                    }
                }
            }
            gate.stop();
            drop(hooks);
            HOOK_GATE.store(0, std::sync::atomic::Ordering::Release);
            HOOK_THREAD.store(0, std::sync::atomic::Ordering::Release);
            drop(halo);
            windows.clear();
            UnregisterClassW(CLASS.as_ptr(), GetModuleHandleW(null()));
        }
    }
    #[cfg(test)]
    pub fn verify_native_strips() -> Result<usize, String> {
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let before = GetForegroundWindow();
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: GetModuleHandleW(null()),
                lpszClassName: CLASS.as_ptr(),
                ..std::mem::zeroed()
            };
            RegisterClassW(&class);
            // An opaque, test-owned backdrop makes compositor verification
            // independent of whatever applications the user has open.
            let width = GetSystemMetrics(SM_CXSCREEN);
            let height = GetSystemMetrics(SM_CYSCREEN);
            let backdrop = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                CLASS.as_ptr(),
                [0u16].as_ptr(),
                WS_POPUP,
                0,
                0,
                width,
                height,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            if backdrop.is_null() {
                return Err("无法创建光效测试背景".into());
            }
            struct Backdrop(HWND);
            impl Drop for Backdrop {
                fn drop(&mut self) {
                    unsafe {
                        DestroyWindow(self.0);
                    }
                }
            }
            let backdrop = Backdrop(backdrop);
            ShowWindow(backdrop.0, SW_SHOWNOACTIVATE);
            let dc = GetDC(backdrop.0);
            let brush = CreateSolidBrush(20 | (24 << 8) | (32 << 16));
            FillRect(
                dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                },
                brush,
            );
            DeleteObject(brush);
            ReleaseDC(backdrop.0, dc);
            windows_sys::Win32::Graphics::Dwm::DwmFlush();
            let mut windows = strips()?;
            windows.push(Strip::halo()?);
            let mut result = Ok(windows.len());
            for strip in &windows {
                strip.show(false);
                if IsWindow(strip.window) == 0 {
                    result = Err("光效暂停时窗口句柄失效".into());
                }
                strip.show(true);
                let styles = GetWindowLongW(strip.window, GWL_EXSTYLE) as u32;
                let expected =
                    WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_LAYERED;
                let mut affinity = 0;
                if styles & expected != expected
                    || GetWindowDisplayAffinity(strip.window, &mut affinity) == 0
                    || affinity != WDA_EXCLUDEFROMCAPTURE
                {
                    result = Err("光效不满足透明、鼠标穿透或截图排除条件".into());
                }
            }
            // WDA_EXCLUDEFROMCAPTURE deliberately hides production cues from
            // screenshots. Only this fixture temporarily permits sampling.
            for strip in &windows {
                SetWindowDisplayAffinity(strip.window, WDA_NONE);
            }
            pointer(width / 2, height / 2, false);
            if !windows.last_mut().unwrap().follow(None) {
                result = Err("AI 指针绘制失败".into());
            }
            let art = std::slice::from_raw_parts(
                windows.last().unwrap().pixels,
                POINTER_SIZE * POINTER_SIZE * 4,
            );
            let preview = image::RgbaImage::from_fn(
                (POINTER_SIZE * 2) as u32,
                POINTER_SIZE as u32,
                |x, y| {
                    let i = (y as usize * POINTER_SIZE + x as usize % POINTER_SIZE) * 4;
                    let background = if x < POINTER_SIZE as u32 {
                        [24u16, 28, 33]
                    } else {
                        [243, 244, 245]
                    };
                    image::Rgba([
                        (art[i + 2] as u16 + background[0] * (255 - art[i + 3] as u16) / 255) as u8,
                        (art[i + 1] as u16 + background[1] * (255 - art[i + 3] as u16) / 255) as u8,
                        (art[i] as u16 + background[2] * (255 - art[i + 3] as u16) / 255) as u8,
                        255,
                    ])
                },
            );
            let _ = std::fs::create_dir_all(".supercode");
            let _ = preview.save(".supercode/computer-pointer-preview.png");
            windows_sys::Win32::Graphics::Dwm::DwmFlush();
            std::thread::sleep(std::time::Duration::from_millis(40));
            let screen = GetDC(null_mut());
            for (x, y) in [
                (12, height / 2),
                (width - 13, height / 2),
                (width / 2, 12),
                (width / 2, height - 13),
                (width / 2 - 4, height / 2 + 8),
            ] {
                let color = GetPixel(screen, x, y);
                let (red, green, blue) = (color & 255, (color >> 8) & 255, (color >> 16) & 255);
                if color == CLR_INVALID || blue < green + 15 || green < red + 10 || blue < 90 {
                    result = Err(format!("桌面合成后未检测到可见蓝色光效/AI 指针：({x},{y}) RGB {red},{green},{blue}"));
                }
            }
            ReleaseDC(null_mut(), screen);
            if GetForegroundWindow() != before {
                result = Err("光效抢占了前台窗口".into());
            }
            drop(windows);
            drop(backdrop);
            UnregisterClassW(CLASS.as_ptr(), GetModuleHandleW(null()));
            result
        }
    }
}
#[cfg(windows)]
pub(crate) fn input_counts() -> (u64, u64) {
    native::input_counts()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glow_fades_inward_and_has_no_opaque_pixels() {
        assert!(alpha(0, 24) > alpha(8, 24));
        assert!(alpha(8, 24) > alpha(20, 24));
        assert_eq!(alpha(24, 24), 0);
        assert!(alpha(0, 24) < 200);
    }
    #[test]
    fn escape_from_users_or_accessibility_tools_stops_but_agent_escape_does_not() {
        assert!(stop_escape(0x1b, 0));
        assert!(stop_escape(0x1b, 0x1234));
        assert!(!stop_escape(0x1b, crate::computer_gate::INPUT_TAG));
        assert!(!stop_escape(0x0d, 0));
    }
    #[test]
    #[cfg(windows)]
    #[ignore = "Creates native edge strips briefly; run explicitly on an unlocked desktop"]
    fn native_overlay_does_not_activate_or_capture_or_accept_input() {
        assert!(native::verify_native_strips().unwrap() >= 4);
    }
}
