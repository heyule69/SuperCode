use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};
use tauri::{Manager, Webview};

#[derive(Default)]
pub struct UiRecovery {
    attached: Mutex<HashSet<String>>,
    attempts: Mutex<HashMap<String, Vec<u64>>>,
    recovered: Mutex<HashSet<String>>,
}

fn admit(attempts: &mut Vec<u64>, now: u64) -> bool {
    attempts.retain(|at| now.saturating_sub(*at) < 60);
    if attempts.len() >= 2 {
        return false;
    }
    attempts.push(now);
    true
}

fn record(app: &tauri::AppHandle, value: serde_json::Value) {
    if let Some(state) = app.try_state::<crate::AppState>() {
        let dir = &state.data_dir;
        let path = dir.join("ui-crashes.jsonl");
        // Keep diagnostics bounded and credential-free. Preserve at most the last 64 KiB.
        let prior = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                eprintln!("无法读取 UTF-8 崩溃日志，未修改原文件：{e}");
                return;
            }
        };
        let tail: String = prior
            .lines()
            .rev()
            .take(100)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|s| format!("{s}\n"))
            .collect();
        let _ = std::fs::write(
            path,
            format!("{}{}\n", crate::protocol::bounded(&tail, 64 * 1024), value),
        );
    }
}

#[cfg(windows)]
pub fn attach(webview: &Webview) {
    use webview2_com::{Microsoft::Web::WebView2::Win32::*, ProcessFailedEventHandler};
    use windows::core::Interface;
    let app = webview.app_handle().clone();
    let label = webview.label().to_string();
    let Some(state) = app.try_state::<UiRecovery>() else {
        return;
    };
    let Ok(mut attached) = state.attached.lock() else {
        return;
    };
    if !attached.insert(label.clone()) {
        return;
    }
    drop(attached);
    let callback_app = app.clone();
    let callback_label = label.clone();
    let result = webview.with_webview(move |platform| unsafe {
        let register = || -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?;
            let handler = ProcessFailedEventHandler::create(Box::new(move |sender,args| {
                let Some(args) = args else { return Ok(()); };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                args.ProcessFailedKind(&mut kind)?;
                let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
                let mut exit = 0;
                if let Ok(details) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() { let _=details.Reason(&mut reason); let _=details.ExitCode(&mut exit); }
                let state = callback_app.state::<UiRecovery>();
                let recoverable = kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED;
                let retry = recoverable && state.attempts.lock().is_ok_and(|mut attempts|admit(attempts.entry(callback_label.clone()).or_default(),crate::storage::now() as u64));
                let restored = retry && sender.as_ref().is_some_and(|core| core.Reload().is_ok());
                if restored { if let Ok(mut pending) = state.recovered.lock() { pending.insert(callback_label.clone()); } }
                record(&callback_app,serde_json::json!({"at":crate::storage::now(),"window":callback_label,"kind":kind.0,"reason":reason.0,"exitCode":exit,"restored":restored}));
                Ok(())
            }));
            let mut token = 0;
            core.add_ProcessFailed(&handler,&mut token)?;
            Ok(())
        };
        if let Err(error) = register() { eprintln!("WebView 崩溃恢复监听初始化失败：{error}"); }
    });
    if result.is_err() {
        if let Ok(mut attached) = app.state::<UiRecovery>().attached.lock() {
            attached.remove(&label);
        }
    }
}

#[cfg(not(windows))]
pub fn attach(_webview: &Webview) {}

#[tauri::command]
pub fn get_ui_recovery(webview: Webview) -> bool {
    webview
        .state::<UiRecovery>()
        .recovered
        .lock()
        .is_ok_and(|mut labels| labels.remove(webview.label()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn recovery_is_bounded_and_resumes_after_cooldown() {
        let mut attempts = vec![];
        assert!(super::admit(&mut attempts, 100));
        assert!(super::admit(&mut attempts, 101));
        assert!(!super::admit(&mut attempts, 102));
        assert!(super::admit(&mut attempts, 160));
        assert_eq!(attempts, vec![101, 160]);
    }
}

pub fn forget(window: &tauri::Window) {
    if let Some(state) = window.try_state::<UiRecovery>() {
        if let Ok(mut labels) = state.attached.lock() {
            labels.remove(window.label());
        }
        if let Ok(mut attempts) = state.attempts.lock() {
            attempts.remove(window.label());
        }
        if let Ok(mut pending) = state.recovered.lock() {
            pending.remove(window.label());
        }
    }
}

/// Regression probe for the isolated smoke-test WebView only. Never ordinary startup.
#[cfg(windows)]
pub async fn verify_renderer_recovery(app: &tauri::AppHandle) -> Result<(), String> {
    use std::time::Duration;
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    if !std::env::args().any(|a| a == "--smoke-test") {
        return Err("崩溃探针只允许在隔离验证中运行".into());
    }
    for _ in 0..30 {
        if app
            .state::<UiRecovery>()
            .attached
            .lock()
            .is_ok_and(|labels| labels.contains("main"))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let window = app.get_webview_window("main").ok_or("没有验证窗口")?;
    window
        .with_webview(|platform| unsafe {
            if let Ok(core) = platform.controller().CoreWebView2() {
                let callback =
                    CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
                let _ = core.CallDevToolsProtocolMethod(
                    windows::core::w!("Page.crash"),
                    windows::core::w!("{}"),
                    &callback,
                );
            }
        })
        .map_err(|e| e.to_string())?;
    for _ in 0..80 {
        let path = app
            .state::<crate::AppState>()
            .data_dir
            .join("ui-crashes.jsonl");
        if let Ok(text) = std::fs::read_to_string(path) {
            if text
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .any(|row| row["restored"] == true)
            {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("WebView 渲染进程崩溃后没有恢复".into())
}
