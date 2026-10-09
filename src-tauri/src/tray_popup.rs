//! On-demand tray panel. No workspace, agent, or transcript is loaded here.
use crate::{desktop_lifecycle, window_theme, AppState};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex,
};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, Window, WindowEvent};

pub const LABEL: &str = "tray-menu";
const WIDTH: f64 = 300.0;

#[derive(Clone, Copy)]
struct Request {
    token: u64,
    point: PhysicalPosition<f64>,
}
#[derive(Default)]
pub struct TrayPopup {
    serial: AtomicU64,
    creating: AtomicBool,
    request: Mutex<Option<Request>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentChat {
    id: String,
    title: String,
    project: String,
    status: String,
    unread: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenWindow {
    label: String,
    title: String,
    project: String,
    status: String,
    main: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub(crate) token: u64,
    recent: Vec<RecentChat>,
    has_more: bool,
    pub(crate) windows: Vec<OpenWindow>,
}

fn recent(store: &crate::storage::Store) -> Result<(Vec<RecentChat>, bool), String> {
    let conn = store.0.lock().map_err(|e| e.to_string())?;
    let mut query = conn.prepare("SELECT s.id,s.title,p.name,s.status,coalesce(i.unread,0) FROM sessions s JOIN projects p ON p.id=s.project_id LEFT JOIN sidebar_items i ON i.kind='session' AND i.id=s.id WHERE s.archived=0 AND coalesce(i.removed,0)=0 AND NOT EXISTS (SELECT 1 FROM sidebar_items x WHERE x.kind='project' AND x.id=p.id AND x.removed=1) ORDER BY s.updated_at DESC,s.rowid DESC LIMIT 31").map_err(|e| e.to_string())?;
    let mut rows = query
        .query_map([], |row| {
            Ok(RecentChat {
                id: row.get(0)?,
                title: row.get::<_, String>(1)?.chars().take(160).collect(),
                project: row.get::<_, String>(2)?.chars().take(80).collect(),
                status: row.get(3)?,
                unread: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let has_more = rows.len() > 30;
    rows.truncate(30);
    Ok((rows, has_more))
}

pub fn snapshot(app: &AppHandle) -> Result<Snapshot, String> {
    let state = app.state::<AppState>();
    let (recent, has_more) = recent(&state.store)?;
    let projects = state.store.projects()?;
    let mut windows = Vec::new();
    for (label, _, session, settings) in desktop_lifecycle::window_contexts(app) {
        let Some(window) = app.get_webview_window(&label) else {
            continue;
        };
        let chat = state.store.session(&session).ok();
        windows.push(OpenWindow {
            main: label == "main",
            label,
            title: if settings {
                "设置".into()
            } else {
                chat.as_ref().map(|s| s.title.clone()).unwrap_or_else(|| {
                    window
                        .title()
                        .unwrap_or_default()
                        .trim_end_matches(" — SuperCode")
                        .to_owned()
                })
            },
            project: chat
                .as_ref()
                .and_then(|s| projects.iter().find(|p| p.id == s.project_id))
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            status: chat.map(|s| s.status).unwrap_or_else(|| "idle".into()),
        });
    }
    let token = app
        .state::<TrayPopup>()
        .request
        .lock()
        .map_err(|e| e.to_string())?
        .map(|r| r.token)
        .unwrap_or(0);
    Ok(Snapshot {
        token,
        recent,
        has_more,
        windows,
    })
}

#[tauri::command]
pub fn tray_menu_snapshot(app: AppHandle) -> Result<Snapshot, String> {
    snapshot(&app)
}

pub async fn open(app: AppHandle, point: PhysicalPosition<f64>) -> Result<(), String> {
    desktop_lifecycle::ensure_running(&app)?;
    let state = app.state::<TrayPopup>();
    let token = state.serial.fetch_add(1, Ordering::AcqRel) + 1;
    *state.request.lock().map_err(|e| e.to_string())? = Some(Request { token, point });
    if let Some(window) = app.get_webview_window(LABEL) {
        return window
            .emit("tray-menu-open", token)
            .map_err(|e| e.to_string());
    }
    if state.creating.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let theme = app
        .get_webview_window("main")
        .and_then(|w| w.theme().ok())
        .unwrap_or(tauri::Theme::Light);
    let canvas = window_theme::canvas_for_window(&app, "main").unwrap_or_else(|| {
        if theme == tauri::Theme::Dark {
            tauri::webview::Color(24, 24, 24, 255)
        } else {
            tauri::webview::Color(243, 244, 245, 255)
        }
    });
    let mut builder =
        tauri::WebviewWindowBuilder::new(&app, LABEL, tauri::WebviewUrl::App("tray.html".into()))
            .title("SuperCode 托盘菜单")
            .decorations(false)
            // Tao adds hidden caption offsets when resizing a not-yet-shown
            // window with native shadows. A popup needs exact content bounds.
            .shadow(false)
            .resizable(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .visible(false)
            .focused(false)
            .inner_size(WIDTH, 600.0)
            .theme(Some(theme))
            .background_color(canvas);
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
    let result = builder.build().map_err(|e| e.to_string());
    state.creating.store(false, Ordering::Release);
    result?;
    Ok(())
}

// Clamp in physical coordinates, including monitors with negative origins.
fn placement(
    point: (f64, f64),
    area: (i32, i32, u32, u32),
    size: (u32, u32),
    gap: i32,
) -> (i32, i32) {
    let (left, top, width, height) = area;
    let x = (point.0.round() as i32 - size.0 as i32 + 20).clamp(
        left + gap,
        (left + width as i32 - size.0 as i32 - gap).max(left + gap),
    );
    let above = point.1.round() as i32 - size.1 as i32 - gap;
    let y = above.clamp(
        top + gap,
        (top + height as i32 - size.1 as i32 - gap).max(top + gap),
    );
    (x, y)
}

#[tauri::command]
pub fn present_tray_menu(
    window: tauri::WebviewWindow,
    token: u64,
    height: f64,
    app: AppHandle,
) -> Result<(), String> {
    if window.label() != LABEL || !height.is_finite() {
        return Err("无效的托盘菜单请求".into());
    }
    let request = *app
        .state::<TrayPopup>()
        .request
        .lock()
        .map_err(|e| e.to_string())?;
    let Some(request) = request.filter(|r| r.token == token) else {
        return Ok(());
    };
    let monitor = app
        .monitor_from_point(request.point.x, request.point.y)
        .map_err(|e| e.to_string())?
        .or(app.primary_monitor().map_err(|e| e.to_string())?)
        .ok_or("无法获取显示器")?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let gap = (8.0 * scale).round() as i32;
    let width = (WIDTH * scale)
        .round()
        .min((area.size.width as i32 - gap * 2).max(1) as f64) as u32;
    let height = (height.clamp(120.0, 720.0) * scale)
        .ceil()
        .min((area.size.height as i32 - gap * 2).max(1) as f64) as u32;
    let (x, y) = placement(
        (request.point.x, request.point.y),
        (
            area.position.x,
            area.position.y,
            area.size.width,
            area.size.height,
        ),
        (width, height),
        gap,
    );
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    window
        .set_size(tauri::PhysicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    if !window.is_visible().map_err(|e| e.to_string())? {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn dismiss(app: &AppHandle) {
    let state = app.state::<TrayPopup>();
    if let Ok(mut request) = state.request.lock() {
        *request = None;
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
    let serial = state.serial.load(Ordering::Acquire);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let state = app.state::<TrayPopup>();
        if state.serial.load(Ordering::Acquire) == serial
            && state.request.lock().is_ok_and(|r| r.is_none())
        {
            if let Some(window) = app.get_webview_window(LABEL) {
                let _ = window.destroy();
            }
        }
    });
}

#[tauri::command]
pub async fn tray_menu_action(
    action: String,
    value: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    desktop_lifecycle::ensure_running(&app)?;
    // Validate a destination before dismissing, so errors remain visible.
    let target = if action == "session" {
        let id = value.as_deref().ok_or("请选择对话")?;
        let valid: bool = app.state::<AppState>().store.0.lock().map_err(|e| e.to_string())?.query_row("SELECT EXISTS (SELECT 1 FROM sessions s WHERE s.id=?1 AND s.archived=0 AND NOT EXISTS (SELECT 1 FROM sidebar_items i WHERE (i.kind='session' AND i.id=s.id OR i.kind='project' AND i.id=s.project_id) AND i.removed=1))", [id], |row| row.get(0)).map_err(|e| e.to_string())?;
        if !valid {
            return Err("对话已归档或移除，请重新打开菜单".into());
        }
        let label = desktop_lifecycle::window_contexts(&app)
            .into_iter()
            .find(|(_, _, session, settings)| session == id && !settings)
            .map(|(label, _, _, _)| label)
            .unwrap_or_else(|| "main".into());
        Some(label)
    } else if action == "window" {
        let label = value.as_deref().ok_or("请选择窗口")?;
        if !desktop_lifecycle::window_contexts(&app)
            .iter()
            .any(|(id, _, _, _)| id == label)
        {
            return Err("窗口已关闭".into());
        }
        Some(label.to_owned())
    } else if !matches!(
        action.as_str(),
        "dismiss" | "show" | "new-chat" | "new-window" | "search" | "quit"
    ) {
        return Err("未知的托盘菜单操作".into());
    } else {
        None
    };
    dismiss(&app);
    match action.as_str() {
        "session" => {
            let label = target.unwrap();
            desktop_lifecycle::show_window(&app, &label)?;
            app.emit_to(
                &label,
                "desktop-navigate",
                serde_json::json!({"sessionId":value}),
            )
            .map_err(|e| e.to_string())
        }
        "window" => desktop_lifecycle::show_window(&app, &target.unwrap()),
        "show" => desktop_lifecycle::show_window(&app, "main"),
        "new-window" => desktop_lifecycle::new_workspace_window(app)
            .await
            .map(|_| ()),
        "new-chat" if app.state::<AppState>().store.running()? > 0 => {
            desktop_lifecycle::new_workspace_window(app)
                .await
                .map(|_| ())
        }
        "new-chat" | "search" => {
            desktop_lifecycle::show_window(&app, "main")?;
            app.emit_to(
                "main",
                "desktop-navigate",
                serde_json::json!({"newChat":action=="new-chat","search":action=="search"}),
            )
            .map_err(|e| e.to_string())
        }
        "quit" => desktop_lifecycle::request_app_exit(app),
        _ => Ok(()),
    }
}

pub fn refresh(app: &AppHandle) {
    if let Some(window) = app
        .get_webview_window(LABEL)
        .filter(|w| w.is_visible().unwrap_or(false))
    {
        let _ = window.emit("tray-menu-updated", ());
    }
}
pub fn on_window_event(window: &Window, event: &WindowEvent) -> bool {
    if window.label() != LABEL {
        return false;
    }
    if matches!(event, WindowEvent::Focused(false)) {
        // Explorer can take focus while the pointer crosses from its tray flyout
        // into our panel. Keep the panel alive while the pointer is inside it.
        let app = window.app_handle().clone();
        let token = app.state::<TrayPopup>().serial.load(Ordering::Acquire);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            loop {
                let state = app.state::<TrayPopup>();
                if state.serial.load(Ordering::Acquire) != token
                    || state.request.lock().is_ok_and(|r| r.is_none())
                {
                    break;
                }
                let Some(panel) = app.get_webview_window(LABEL) else {
                    break;
                };
                if panel.is_focused().unwrap_or(false) {
                    break;
                }
                if !pointer_inside(&panel) {
                    dismiss(&app);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            }
        });
    }
    true
}

fn inside(point: (i32, i32), origin: (i32, i32), size: (u32, u32)) -> bool {
    point.0 >= origin.0
        && point.1 >= origin.1
        && i64::from(point.0) < i64::from(origin.0) + i64::from(size.0)
        && i64::from(point.1) < i64::from(origin.1) + i64::from(size.1)
}
#[cfg(windows)]
fn pointer_inside(window: &tauri::WebviewWindow) -> bool {
    let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) } == 0 {
        return false;
    }
    match (window.outer_position(), window.outer_size()) {
        (Ok(p), Ok(s)) => inside((point.x, point.y), (p.x, p.y), (s.width, s.height)),
        _ => false,
    }
}
#[cfg(not(windows))]
fn pointer_inside(_window: &tauri::WebviewWindow) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hover_bounds_include_negative_monitor_coordinates() {
        assert!(inside((-400, 50), (-500, 10), (300, 400)));
        assert!(!inside((-201, 410), (-500, 10), (300, 400)));
        assert!(!inside((-501, 50), (-500, 10), (300, 400)));
    }
    #[test]
    fn menu_stays_inside_work_area_with_mixed_monitor_origins() {
        assert_eq!(
            placement((1390., 870.), (0, 0, 1440, 860), (420, 440), 8),
            (990, 412)
        );
        assert_eq!(
            placement((-1900., 20.), (-1920, 0, 1920, 1040), (630, 600), 12),
            (-1908, 12)
        );
        assert_eq!(
            placement((5., 5.), (0, 0, 1440, 860), (420, 440), 8),
            (8, 8)
        );
    }
    #[test]
    fn recent_history_excludes_archived_removed_and_pinned_order() {
        let store =
            crate::storage::Store::from_connection(rusqlite::Connection::open_in_memory().unwrap())
                .unwrap();
        let project = store.add_project(std::path::Path::new(".")).unwrap();
        let older = store.create_session(&project.id, None).unwrap();
        let newer = store.create_session(&project.id, None).unwrap();
        let archived = store.create_session(&project.id, None).unwrap();
        store.archive(&archived.id).unwrap();
        {
            let conn = store.0.lock().unwrap();
            conn.execute("UPDATE sessions SET updated_at=1 WHERE id=?1", [&older.id])
                .unwrap();
            conn.execute("UPDATE sessions SET updated_at=2 WHERE id=?1", [&newer.id])
                .unwrap();
            conn.execute(
                "INSERT INTO sidebar_items(kind,id,pinned,unread) VALUES('session',?1,1,1)",
                [&older.id],
            )
            .unwrap();
        }
        let (rows, more) = recent(&store).unwrap();
        assert!(!more);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, newer.id);
        assert!(rows[1].unread);
        store
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE sidebar_items SET removed=1 WHERE id=?1",
                [&older.id],
            )
            .unwrap();
        assert_eq!(recent(&store).unwrap().0.len(), 1);
        for _ in 0..35 {
            store.create_session(&project.id, None).unwrap();
        }
        let (bounded, more) = recent(&store).unwrap();
        assert_eq!(bounded.len(), 30);
        assert!(more);
        store
            .0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO sidebar_items(kind,id,removed) VALUES('project',?1,1)",
                [&project.id],
            )
            .unwrap();
        assert!(recent(&store).unwrap().0.is_empty());
    }
}
