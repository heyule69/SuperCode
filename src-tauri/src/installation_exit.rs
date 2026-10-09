//! Keep installation shutdown separate from close-to-tray.
#[cfg(windows)]
pub fn attach(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{RegisterWindowMessageW, WM_NCDESTROY},
        },
    };
    struct Hook {
        app: tauri::AppHandle,
        message: u32,
    }
    unsafe extern "system" fn callback(
        hwnd: HWND,
        message: u32,
        wp: WPARAM,
        lp: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        let hook = &*(data as *const Hook);
        if message == hook.message {
            use crate::{desktop_lifecycle, installation_exit_protocol as protocol};
            if desktop_lifecycle::is_quitting(&hook.app) {
                return protocol::ACCEPTED;
            }
            if desktop_lifecycle::reserve_update_exit(&hook.app).is_err() {
                return protocol::BUSY;
            }
            desktop_lifecycle::finish_update_exit(hook.app.clone());
            return protocol::ACCEPTED;
        }
        if message == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(callback), id);
            drop(Box::from_raw(data as *mut Hook));
        }
        DefSubclassProc(hwnd, message, wp, lp)
    }
    let window = app.get_webview_window("main").ok_or("主窗口不存在")?;
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0;
    let name: Vec<u16> = crate::installation_exit_protocol::MESSAGE
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let message = unsafe { RegisterWindowMessageW(name.as_ptr()) };
    if message == 0 {
        return Err("无法注册安装退出消息".into());
    }
    let hook = Box::into_raw(Box::new(Hook {
        app: app.clone(),
        message,
    }));
    // Tauri's setup and this subclass execute on the window's owning UI thread.
    if unsafe { SetWindowSubclass(hwnd, Some(callback), message as usize, hook as usize) } == 0 {
        unsafe {
            drop(Box::from_raw(hook));
        }
        return Err("无法挂接安装退出处理".into());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn attach(_: &tauri::AppHandle) -> Result<(), String> {
    Ok(())
}

/// Hidden, isolated native shutdown fixture. No agent is launched by this mode.
pub fn smoke(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::{Listener, Manager};
    let args: Vec<_> = std::env::args_os().collect();
    let report = args
        .iter()
        .position(|a| a == "--installation-exit-test" || a == "--legacy-installation-exit-test")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from)
        .ok_or("缺少隔离退出测试报告路径")?;
    let root = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join(".supercode");
    let parent =
        std::fs::canonicalize(report.parent().ok_or("报告路径无效")?).map_err(|e| e.to_string())?;
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    if !parent.starts_with(root) {
        return Err("报告必须位于项目隔离测试目录".into());
    }
    let state = app.state::<crate::AppState>();
    let project_path = state.data_dir.join("exit-test-project");
    std::fs::create_dir_all(&project_path).map_err(|e| e.to_string())?;
    let project = state.store.add_project(&project_path)?;
    let session = state.store.create_session(&project.id, None)?;
    state.store.rename(&session.id, "安装退出测试：保留会话")?;
    state
        .store
        .set_setting("installation-exit-sentinel", "保留中文配置")?;
    if args.iter().any(|a| a == "--busy") {
        state
            .store
            .set_status(&session.id, "running", Some("fixture-only"))?;
    }
    let saved = state.data_dir.join("before-exit.json");
    app.listen("desktop-before-exit", move |_| {
        let _ = std::fs::write(&saved, b"{\"savedBeforeExit\":true}");
    });
    let value = serde_json::json!({"pid":std::process::id(), "data":state.data_dir, "session":session.id, "isolated":true});
    let session_id = session.id;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let _ = std::fs::write(
            &report,
            serde_json::to_vec_pretty(&value).unwrap_or_default(),
        );
        // Avoid leaving a fixture process behind after an interrupted test.
        for _ in 0..450 {
            if report.with_extension("resume").exists() {
                let _ = app
                    .state::<crate::AppState>()
                    .store
                    .set_status(&session_id, "idle", None);
                let _ = std::fs::write(report.with_extension("idle"), b"ready");
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        app.exit(0);
    });
    Ok(())
}
