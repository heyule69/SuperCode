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
                    browser::stop_with_reason(
                        app,
                        &tab.session,
                        true,
                        if key == 27 {
                            "escape_key"
                        } else {
                            "browser_keyboard_input"
                        },
                    );
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
                            browser::stop_with_reason(
                                app,
                                &tab.session,
                                true,
                                "browser_mouse_input",
                            );
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
    #[derive(Debug, PartialEq)]
    enum Recovery {
        Automatic,
        Reload,
        PageError,
    }
    fn recovery(
        kind: webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_KIND,
    ) -> Recovery {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        match kind {
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => Recovery::Reload,
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED
            | COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED
            | COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => Recovery::PageError,
            // GPU and utility exits recover inside WebView2. They are not user stop requests.
            _ => Recovery::Automatic,
        }
    }
    fn admit_reload(attempts: &mut Vec<std::time::Instant>, now: std::time::Instant) -> bool {
        attempts.retain(|at| now.duration_since(*at) < std::time::Duration::from_secs(60));
        if attempts.len() >= 2 {
            return false;
        }
        attempts.push(now);
        true
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
                        browser::stop_with_reason(&escape_app, &escape_tab.session, true, "browser_escape_key");
                    }
                }
                Ok(())
            }));
            let _ = controller.add_AcceleratorKeyPressed(&escape, &mut token);
            if let Ok(core) = controller.CoreWebView2() {
                let failed =
                    webview2_com::ProcessFailedEventHandler::create(Box::new(move |sender, args| {
                        use webview2_com::Microsoft::Web::WebView2::Win32::*;
                        use windows::core::Interface;
                        let Some(args) = args else { return Ok(()); };
                        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                        args.ProcessFailedKind(&mut kind)?;
                        let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
                        let mut exit = 0;
                        if let Ok(details) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
                            let _ = details.Reason(&mut reason); let _ = details.ExitCode(&mut exit);
                        }
                        let policy = recovery(kind);
                        let restored = if policy == Recovery::Reload {
                            browser::invalidate_observation(&failed_tab);
                            let allowed = failed_tab.recovery_attempts.lock()
                                .is_ok_and(|mut attempts| admit_reload(&mut attempts, std::time::Instant::now()));
                            allowed && sender.as_ref().is_some_and(|core| core.Reload().is_ok())
                        } else { false };
                        if policy != Recovery::Automatic {
                            let message = if restored { "网页进程已恢复，请重新获取页面" }
                                else if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE { "网页暂时无响应，当前浏览器操作可能超时" }
                                else { "网页进程异常，当前浏览器操作可能失败" };
                            browser::failure(&app, &failed_tab, message);
                        }
                        // A browser failure only affects browser tools. Do not revoke the lease or
                        // interrupt the entire agent, and do not label it as user takeover.
                        let log_app = app.clone(); let session = failed_tab.session.clone();
                        tauri::async_runtime::spawn(async move {
                            crate::runtime_diagnostics::record(&log_app, Some(&session), "browser_process_failed",
                                serde_json::json!({"kind":kind.0,"reasonCode":reason.0,"exitCode":exit,
                                    "recovery":if restored { "reloaded" } else if policy == Recovery::Automatic { "automatic" } else { "browser_tool_error" }}));
                        });
                        Ok(())
                    }));
                let _ = core.add_ProcessFailed(&failed, &mut token);
            }
        });
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        #[test]
        fn gpu_and_utility_recovery_are_not_agent_interrupts() {
            assert_eq!(
                recovery(COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED),
                Recovery::Automatic
            );
            assert_eq!(
                recovery(COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED),
                Recovery::Automatic
            );
            assert_eq!(
                recovery(COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED),
                Recovery::Reload
            );
            for kind in [
                COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
                COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
                COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
            ] {
                assert_eq!(recovery(kind), Recovery::PageError);
            }
        }
        #[test]
        fn repeated_renderer_failures_cannot_cause_an_infinite_reload_loop() {
            let now = std::time::Instant::now();
            let mut attempts = Vec::new();
            assert!(admit_reload(&mut attempts, now));
            assert!(admit_reload(&mut attempts, now));
            assert!(!admit_reload(&mut attempts, now));
            assert!(admit_reload(
                &mut attempts,
                now + std::time::Duration::from_secs(61)
            ));
            assert_eq!(attempts.len(), 1);
        }
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
