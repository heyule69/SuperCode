//! Close keeps each workspace and its agents alive; only explicit Quit exits.
use crate::{window_state, window_theme, AppState};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
#[cfg(not(windows))]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Window, WindowEvent,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

const TRAY: &str = "supercode-tray";
const WINDOW_ITEM: &str = "supercode-window:";

#[derive(Default)]
struct Workspace {
    number: usize,
    session: String,
    settings: bool,
}

#[derive(Default)]
pub struct DesktopLifecycle {
    available: AtomicBool,
    quitting: AtomicBool,
    confirming: AtomicBool,
    windows: Mutex<HashMap<String, Workspace>>,
}

fn hide_on_close(available: bool, quitting: bool) -> bool {
    available && !quitting
}
fn prevent_implicit_exit(available: bool, quitting: bool, code: Option<i32>) -> bool {
    hide_on_close(available, quitting) && code.is_none()
}

#[cfg(any(test, not(windows)))]
fn menu_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect::<String>()
        .replace('&', "&&")
}

pub fn window_contexts(app: &AppHandle) -> Vec<(String, usize, String, bool)> {
    let mut entries = app
        .state::<DesktopLifecycle>()
        .windows
        .lock()
        .map(|saved| {
            saved
                .iter()
                .map(|(label, workspace)| {
                    (
                        label.clone(),
                        workspace.number,
                        workspace.session.clone(),
                        workspace.settings,
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    entries.sort_by_key(|(_, number, _, _)| *number);
    entries
}

#[cfg(not(windows))]
fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let show = MenuItem::with_id(app, "supercode-show", "显示主窗口", true, None::<&str>)?;
    let windows = Submenu::new(app, "正在运行的窗口", true)?;
    // Snapshot first: no lifecycle/storage lock is held across native menu calls.
    let mut entries = app
        .state::<DesktopLifecycle>()
        .windows
        .lock()
        .map(|saved| {
            saved
                .iter()
                .map(|(label, workspace)| {
                    (
                        label.clone(),
                        workspace.number,
                        workspace.session.clone(),
                        workspace.settings,
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    entries.sort_by_key(|(_, number, _, _)| *number);
    for (label, number, session_id, settings) in entries {
        let Some(window) = app.get_webview_window(&label) else {
            continue;
        };
        let session = app
            .try_state::<AppState>()
            .and_then(|s| s.store.session(&session_id).ok());
        let title = if settings {
            "设置".into()
        } else {
            session
                .as_ref()
                .map(|s| s.title.clone())
                .unwrap_or_else(|| {
                    window
                        .title()
                        .unwrap_or_else(|_| "新窗口".into())
                        .trim_end_matches(" — SuperCode")
                        .to_owned()
                })
        };
        let prefix = if label == "main" {
            "主窗口".into()
        } else {
            format!("窗口 {number}")
        };
        let status = session
            .filter(|s| matches!(s.status.as_str(), "starting" | "running" | "waiting"))
            .map(|s| {
                if s.status == "waiting" {
                    " · 等待确认"
                } else {
                    " · 运行中"
                }
            })
            .unwrap_or("");
        let item = MenuItem::with_id(
            app,
            format!("{WINDOW_ITEM}{label}"),
            menu_text(&format!("{prefix} · {title}{status}")),
            true,
            None::<&str>,
        )?;
        windows.append(&item)?;
    }
    let new = MenuItem::with_id(app, "supercode-new", "新窗口", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "supercode-quit", "退出", true, None::<&str>)?;
    Menu::with_items(app, &[&show, &windows, &new, &separator, &quit])
}

pub fn refresh(app: &AppHandle) {
    let Some(state) = app.try_state::<DesktopLifecycle>() else {
        return;
    };
    if !state.available.load(Ordering::Acquire) || state.quitting.load(Ordering::Acquire) {
        return;
    }
    crate::tray_popup::refresh(app);
    #[cfg(not(windows))]
    if let Some(tray) = app.tray_by_id(TRAY) {
        if let Err(error) = menu(app).and_then(|menu| tray.set_menu(Some(menu))) {
            eprintln!("更新系统托盘菜单失败：{error}");
        }
    }
}

pub fn update_context(window: &tauri::WebviewWindow, session: &str, settings: bool) {
    let app = window.app_handle();
    let Some(state) = app.try_state::<DesktopLifecycle>() else {
        return;
    };
    let changed = if let Ok(mut windows) = state.windows.lock() {
        let number = windows.values().map(|w| w.number).max().unwrap_or(0) + 1;
        let entry = windows
            .entry(window.label().to_owned())
            .or_insert_with(|| Workspace {
                number,
                ..Workspace::default()
            });
        let changed = entry.session != session || entry.settings != settings;
        entry.session = session.into();
        entry.settings = settings;
        changed
    } else {
        false
    };
    if changed {
        refresh(app);
    }
}

pub fn show_window(app: &AppHandle, label: &str) -> Result<(), String> {
    let window = app.get_webview_window(label).ok_or("窗口已关闭")?;
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

pub fn ensure_running(app: &AppHandle) -> Result<(), String> {
    if app
        .try_state::<DesktopLifecycle>()
        .is_some_and(|state| state.quitting.load(Ordering::Acquire))
    {
        return Err("SuperCode 正在退出，请稍后重新打开".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn new_workspace_window(app: AppHandle) -> Result<String, String> {
    ensure_running(&app)?;
    let label = format!("chat-workspace-{}", uuid::Uuid::new_v4());
    let theme = app
        .get_webview_window("main")
        .and_then(|window| window.theme().ok())
        .unwrap_or(tauri::Theme::Dark);
    let canvas = window_theme::canvas_for_window(&app, "main").unwrap_or_else(|| {
        if theme == tauri::Theme::Dark {
            tauri::webview::Color(24, 24, 23, 255)
        } else {
            tauri::webview::Color(252, 250, 245, 255)
        }
    });
    let mut builder = tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App("index.html?window=new".into()),
    )
    .title("新窗口 — SuperCode")
    .decorations(false)
    .center()
    .inner_size(1100.0, 800.0)
    .min_inner_size(760.0, 580.0)
    .theme(Some(theme))
    .background_color(canvas);
    // Reuse a configured main profile, including the isolated smoke-test one.
    // Ordinary installations retain Tauri's shared default WebView profile.
    if let Some(directory) = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .and_then(|w| w.data_directory.clone())
    {
        builder = builder.data_directory(directory);
    }
    let window = builder.build().map_err(|e| e.to_string())?;
    update_context(&window, "", false);
    refresh(&app);
    Ok(label)
}

fn dispatch(app: &AppHandle, id: &str) {
    if id == "supercode-show" {
        let _ = show_window(app, "main");
    } else if let Some(label) = id.strip_prefix(WINDOW_ITEM) {
        let _ = show_window(app, label);
    } else if id == "supercode-new" {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = new_workspace_window(app).await {
                eprintln!("创建窗口失败：{error}");
            }
        });
    } else if id == "supercode-quit" {
        let _ = request_app_exit(app.clone());
    }
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let icon = app
        .default_window_icon()
        .expect("SuperCode application icon")
        .clone();
    if let Some(window) = app.get_webview_window("main") {
        update_context(&window, "", false);
    }
    let builder = TrayIconBuilder::with_id(TRAY)
        .tooltip("SuperCode")
        .icon(icon)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| dispatch(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            #[cfg(windows)]
            if let TrayIconEvent::Click {
                position,
                button: MouseButton::Right,
                button_state: MouseButtonState::Up,
                ..
            } = &event
            {
                let app = tray.app_handle().clone();
                let point = *position;
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = crate::tray_popup::open(app, point).await {
                        eprintln!("打开托盘菜单失败：{error}");
                    }
                });
            }
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let _ = show_window(tray.app_handle(), "main");
            }
        });
    #[cfg(not(windows))]
    let builder = builder.menu(&menu(app)?);
    builder.build(app)?;
    app.state::<DesktopLifecycle>()
        .available
        .store(true, Ordering::Release);
    Ok(())
}

/// Returns true when CloseRequested was handled, so persistence does not freeze.
pub fn on_window_event(window: &Window, event: &WindowEvent) -> bool {
    if crate::tray_popup::on_window_event(window, event) {
        return true;
    }
    let app = window.app_handle();
    let Some(state) = app.try_state::<DesktopLifecycle>() else {
        return false;
    };
    if let WindowEvent::CloseRequested { api, .. } = event {
        if hide_on_close(
            state.available.load(Ordering::Acquire),
            state.quitting.load(Ordering::Acquire),
        ) {
            api.prevent_close();
            window_state::prepare_hide(window);
            if let Err(error) = window.hide() {
                eprintln!("隐藏窗口失败：{error}");
            }
            refresh(app);
            return true;
        }
    }
    if matches!(event, WindowEvent::Destroyed) {
        if let Ok(mut windows) = state.windows.lock() {
            windows.remove(window.label());
        }
        refresh(app);
    } else if matches!(event, WindowEvent::Focused(true)) {
        refresh(app);
    }
    false
}

pub fn on_exit_requested(app: &AppHandle, code: Option<i32>, api: &tauri::ExitRequestApi) {
    if app.try_state::<DesktopLifecycle>().is_some_and(|s| {
        prevent_implicit_exit(
            s.available.load(Ordering::Acquire),
            s.quitting.load(Ordering::Acquire),
            code,
        )
    }) {
        api.prevent_exit();
    }
}

#[tauri::command]
pub fn request_app_exit(app: AppHandle) -> Result<(), String> {
    let state = app.state::<DesktopLifecycle>();
    if state.quitting.load(Ordering::Acquire) || state.confirming.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let running = match app.state::<AppState>().store.running() {
        Ok(count) => count,
        Err(error) => {
            state.confirming.store(false, Ordering::Release);
            return Err(error);
        }
    };
    if running == 0 {
        begin_exit(app);
        return Ok(());
    }
    let _ = show_window(&app, "main");
    let mut dialog = app
        .dialog()
        .message(format!(
            "还有 {running} 个任务正在运行或等待确认。退出会停止任务；继续运行会保留所有窗口。"
        ))
        .title("退出 SuperCode？")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "退出并停止任务".into(),
            "继续运行".into(),
        ));
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.parent(&window);
    }
    dialog.show(move |accepted| {
        if accepted {
            begin_exit(app);
        } else {
            app.state::<DesktopLifecycle>()
                .confirming
                .store(false, Ordering::Release);
        }
    });
    Ok(())
}

pub fn is_quitting(app: &AppHandle) -> bool {
    app.state::<DesktopLifecycle>()
        .quitting
        .load(Ordering::Acquire)
}

pub fn reserve_update_exit(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<DesktopLifecycle>();
    if state.quitting.swap(true, Ordering::AcqRel) {
        return Err("SuperCode 正在退出。".into());
    }
    match app.state::<AppState>().store.running() {
        Ok(0) => Ok(()),
        result => {
            state.quitting.store(false, Ordering::Release);
            match result {
                Err(error) => Err(error),
                _ => Err("还有任务正在运行或等待确认，请完成任务后再更新。".into()),
            }
        }
    }
}
pub fn cancel_update_exit(app: &AppHandle) {
    app.state::<DesktopLifecycle>()
        .quitting
        .store(false, Ordering::Release);
}
pub fn finish_update_exit(app: AppHandle) {
    exit_reserved(app);
}

fn begin_exit(app: AppHandle) {
    if app
        .state::<DesktopLifecycle>()
        .quitting
        .swap(true, Ordering::AcqRel)
    {
        return;
    }
    exit_reserved(app);
}
fn exit_reserved(app: AppHandle) {
    window_state::flush(&app);
    let _ = app.emit("desktop-before-exit", ());
    tauri::async_runtime::spawn(async move {
        // Give each live WebView a chance to flush its current composer draft.
        tokio::time::sleep(Duration::from_millis(350)).await;
        let state = app.state::<AppState>();
        let shutdown = async {
            tokio::join!(
                state.runtime.shutdown(),
                state.claude.shutdown(),
                state.native.shutdown(),
                async {
                    if let Some(mut child) = state.claude_login.lock().await.take() {
                        let _ = child.start_kill();
                    }
                }
            );
        };
        let _ = tokio::time::timeout(Duration::from_secs(3), shutdown).await;
        state.store.fail_active();
        app.exit(0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_requires_working_tray_and_never_blocks_explicit_quit() {
        assert!(hide_on_close(true, false));
        assert!(!hide_on_close(false, false));
        assert!(!hide_on_close(true, true));
        assert!(prevent_implicit_exit(true, false, None));
        assert!(!prevent_implicit_exit(true, false, Some(0)));
        assert!(!prevent_implicit_exit(true, true, None));
        assert!(!prevent_implicit_exit(false, false, None));
    }
    #[test]
    fn native_menu_titles_preserve_unicode_without_mnemonics_or_controls() {
        assert_eq!(menu_text("图像 & 视频\n测试"), "图像 && 视频测试");
        assert_eq!(menu_text(&"中".repeat(100)).chars().count(), 80);
    }
}
