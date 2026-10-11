//! Physical input stops browser automation before the next CDP command is dispatched.
#[cfg(windows)]
mod platform {
    use crate::browser;
    use std::sync::{atomic::Ordering, Arc, OnceLock};
    use tauri::{AppHandle, Manager, Webview};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::{
        Foundation::{LPARAM, LRESULT, POINT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
        UI::WindowsAndMessaging::*,
    };
    static APP: OnceLock<AppHandle> = OnceLock::new();
    unsafe fn own_foreground() -> bool {
        let mut process = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), &mut process);
        process == GetCurrentProcessId()
    }
    unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
            keyboard_input(
                (*(lparam as *const KBDLLHOOKSTRUCT)).vkCode,
                own_foreground(),
            );
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
    fn keyboard_input(key: u32, foreground: bool) {
        if let Some(app) = APP.get() {
            if let Some(tab) = browser::active_tab(app) {
                if key == 27
                    || (foreground
                        && tab.visible.load(Ordering::Acquire)
                        && tab.focused.load(Ordering::Acquire))
                {
                    browser::stop(app, &tab.session, true);
                }
            }
        }
    }
    pub fn verify_escape_callback() -> Result<(), String> {
        if !std::env::args().any(|arg| arg == "--browser-smoke-test") {
            return Err("Browser stop probe is limited to isolated tests".into());
        }
        // Same callback as the installed hook, without injecting keys into another application.
        keyboard_input(27, false);
        Ok(())
    }
    unsafe extern "system" fn mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0
            && matches!(
                wparam as u32,
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_MOUSEWHEEL
            )
        {
            if let Some(app) = APP.get() {
                if let Some(tab) = browser::active_tab(app) {
                    if !tab.visible.load(Ordering::Acquire) {
                        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
                    }
                    // Hooks never call Tauri's synchronous UI dispatcher: a stalled renderer
                    // must not block this input thread and prevent the subsequent Esc.
                    if tab.hwnd != 0 {
                        let scale = f64::from_bits(tab.scale.load(Ordering::Acquire));
                        let mut origin = POINT { x: 0, y: 0 };
                        ClientToScreen(tab.hwnd as _, &mut origin);
                        let point = (*(lparam as *const MSLLHOOKSTRUCT)).pt;
                        let (x, y, width, height) = *tab.rect.lock().unwrap();
                        if point.x as f64 >= origin.x as f64 + x * scale
                            && point.y as f64 >= origin.y as f64 + y * scale
                            && (point.x as f64) < origin.x as f64 + (x + width) * scale
                            && (point.y as f64) < origin.y as f64 + (y + height) * scale
                        {
                            browser::stop(app, &tab.session, true);
                        }
                    }
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
    pub fn install(app: &AppHandle) -> Result<(), String> {
        if APP.set(app.clone()).is_err() {
            return Ok(());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("supercode-browser-input".into())
            .spawn(move || unsafe {
                let module = GetModuleHandleW(std::ptr::null());
                let keys = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), module, 0);
                let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), module, 0);
                if keys.is_null() || mouse.is_null() {
                    if !keys.is_null() {
                        UnhookWindowsHookEx(keys);
                    }
                    if !mouse.is_null() {
                        UnhookWindowsHookEx(mouse);
                    }
                    let _ = tx.send(Err("浏览器 Esc 与接管监听未能启动".to_string()));
                    return;
                }
                let _ = tx.send(Ok(()));
                let mut message = std::mem::zeroed();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                UnhookWindowsHookEx(keys);
                UnhookWindowsHookEx(mouse);
            })
            .map_err(|e| e.to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| "浏览器接管监听启动超时".to_string())?
    }
    pub fn attach(view: &Webview, tab: &Arc<browser::Tab>) {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
        };
        use webview2_com::{AcceleratorKeyPressedEventHandler, FocusChangedEventHandler};
        let focused = tab.clone();
        let blurred = tab.clone();
        let failed_tab = tab.clone();
        let app = view.app_handle().clone();
        let escape_app = app.clone();
        let escape_tab = tab.clone();
        let _ = view.with_webview(move |platform| unsafe {
            let focus = FocusChangedEventHandler::create(Box::new(move |_, _| {
                focused.focused.store(true, Ordering::Release);
                Ok(())
            }));
            let blur = FocusChangedEventHandler::create(Box::new(move |_, _| {
                blurred.focused.store(false, Ordering::Release);
                Ok(())
            }));
            let controller = platform.controller();
            let mut token = 0;
            let _ = controller.add_GotFocus(&focus, &mut token);
            let _ = controller.add_LostFocus(&blur, &mut token);
            // Native page focus does not bubble keys into the React document.
            // Handle Esc at the controller too, before the page can consume it.
            let escape = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut key = 0;
                    let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
                    args.VirtualKey(&mut key)?;
                    args.KeyEventKind(&mut kind)?;
                    if key == 27
                        && matches!(
                            kind,
                            COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                                | COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                        )
                        && browser::active_tab(&escape_app)
                            .is_some_and(|active| Arc::ptr_eq(&active, &escape_tab))
                    {
                        args.SetHandled(true)?;
                        browser::stop(&escape_app, &escape_tab.session, true);
                    }
                }
                Ok(())
            }));
            let _ = controller.add_AcceleratorKeyPressed(&escape, &mut token);
            if let Ok(core) = controller.CoreWebView2() {
                let failed =
                    webview2_com::ProcessFailedEventHandler::create(Box::new(move |_, _| {
                        browser::stop(&app, &failed_tab.session, true);
                        browser::failure(
                            &app,
                            &failed_tab,
                            "网页进程已退出，请刷新网页后再发起任务",
                        );
                        Ok(())
                    }));
                let _ = core.add_ProcessFailed(&failed, &mut token);
            }
        });
    }
}
#[cfg(windows)]
pub use platform::{attach, install, verify_escape_callback};
#[cfg(not(windows))]
pub fn install(_: &tauri::AppHandle) -> Result<(), String> {
    Ok(())
}
#[cfg(not(windows))]
pub fn attach(_: &tauri::Webview, _: &std::sync::Arc<crate::browser::Tab>) {}
