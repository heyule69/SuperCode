//! Native notifications carry no Agent output, commands, credentials or approval actions.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Complete,
    Error,
    Approval,
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Unseen,
    Background,
    Always,
}
#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Preferences {
    enabled: bool,
    mode: Mode,
    complete: bool,
    error: bool,
    approval: bool,
    sound: bool,
    details: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: Mode::Unseen,
            complete: true,
            error: true,
            approval: true,
            sound: false,
            details: false,
        }
    }
}
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    session_id: String,
    settings_open: bool,
}
#[derive(Default)]
pub struct Notifications {
    preferences: Mutex<Preferences>,
    contexts: Mutex<HashMap<String, Context>>,
    sent: Mutex<HashMap<String, Instant>>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    state: &'static str,
    message: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenTask {
    session_id: Option<String>,
}

fn permitted(prefs: &Preferences, kind: Kind, foreground: bool, viewing: bool) -> bool {
    prefs.enabled
        && match kind {
            Kind::Complete => prefs.complete,
            Kind::Error => prefs.error,
            Kind::Approval => prefs.approval,
        }
        && match prefs.mode {
            Mode::Always => true,
            Mode::Background => !foreground,
            Mode::Unseen => !viewing,
        }
}
fn claim(sent: &mut HashMap<String, Instant>, key: String, now: Instant) -> bool {
    sent.retain(|_, at| now.saturating_duration_since(*at) < Duration::from_secs(86400));
    if sent.contains_key(&key) {
        return false;
    }
    if sent.len() >= 256 {
        if let Some(oldest) = sent
            .iter()
            .min_by_key(|(_, at)| *at)
            .map(|(key, _)| key.clone())
        {
            sent.remove(&oldest);
        }
    }
    sent.insert(key, now);
    true
}
fn text(kind: Kind, details: bool, session: &str, project: &str) -> (String, String) {
    let (title, description) = match kind {
        Kind::Complete => ("任务已完成", "打开 SuperCode 查看结果。"),
        Kind::Error => ("任务运行失败", "打开 SuperCode 查看错误并继续处理。"),
        Kind::Approval => ("需要你的确认", "任务正在等待审批或回答，请打开对话处理。"),
    };
    let clean = |value: &str| {
        value
            .chars()
            .filter(|c| !c.is_control())
            .take(72)
            .collect::<String>()
    };
    (
        title.into(),
        if details {
            format!("{} · {}\n{description}", clean(project), clean(session))
        } else {
            description.into()
        },
    )
}

#[tauri::command]
pub fn update_notification_context(window: WebviewWindow, app: AppHandle, context: Context) {
    crate::desktop_lifecycle::update_context(&window, &context.session_id, context.settings_open);
    if let Ok(mut contexts) = app.state::<Notifications>().contexts.lock() {
        contexts.retain(|label, _| app.get_webview_window(label).is_some());
        contexts.insert(window.label().into(), context);
    }
}

#[tauri::command]
pub fn update_notification_preferences(app: AppHandle, preferences: Preferences) {
    if let Ok(mut saved) = app.state::<Notifications>().preferences.lock() {
        *saved = preferences;
    }
}

fn task_event(
    session: &crate::storage::Session,
    event: &serde_json::Value,
) -> Option<(Kind, String)> {
    let params = &event["params"];
    if event["method"] == "turn/completed" {
        let turn = params["turn"]["id"].as_str()?;
        if turn.is_empty() || session.turn_id.as_deref() != Some(turn) {
            return None;
        }
        let kind = match params["turn"]["status"].as_str()? {
            "completed" => Kind::Complete,
            "failed" => Kind::Error,
            _ => return None,
        };
        return Some((kind, turn.into()));
    }
    let id = event.get("id")?;
    if !(id.is_string() || id.is_number())
        || !matches!(session.status.as_str(), "starting" | "running" | "waiting")
    {
        return None;
    }
    Some((
        Kind::Approval,
        serde_json::json!([session.turn_id, id]).to_string(),
    ))
}

// Called once from the native runtime, after the event was saved successfully.
// The pre-event session identifies the exact turn even though completion clears turn_id.
pub(crate) fn on_agent_event(
    app: &AppHandle,
    session: &crate::storage::Session,
    event: &serde_json::Value,
) {
    let Some((kind, event_key)) = task_event(session, event) else {
        return;
    };
    let handle = app.clone();
    let native_id = session.native_id.clone().unwrap_or_default();
    tauri::async_runtime::spawn(async move {
        let report = handle.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            notify_task(&handle, &native_id, kind, event_key)
        })
        .await;
        if let Ok(Err(error)) = result {
            let _ = report.emit("runtime-log", format!("系统通知：{error}"));
        }
    });
}

fn notify_task(
    app: &AppHandle,
    native_id: &str,
    kind: Kind,
    event_key: String,
) -> Result<bool, String> {
    let state = app.state::<crate::AppState>();
    let session = state.store.for_native(native_id)?;
    // Reject stale events after another turn started, and stopped tasks.
    if kind == Kind::Complete && session.status != "idle"
        || kind == Kind::Error && session.status != "failed"
        || kind == Kind::Approval && session.status != "waiting"
    {
        return Ok(false);
    }
    let notifications = app.state::<Notifications>();
    let preferences = notifications
        .preferences
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    let contexts = notifications.contexts.lock().map_err(|e| e.to_string())?;
    let mut foreground = false;
    let mut viewing = false;
    for (label, win) in app.webview_windows() {
        if win.is_focused().unwrap_or(false) && !win.is_minimized().unwrap_or(false) {
            foreground = true;
            viewing |= contexts
                .get(&label)
                .is_some_and(|c| !c.settings_open && c.session_id == session.id);
        }
    }
    drop(contexts);
    if !permitted(&preferences, kind, foreground, viewing) {
        return Ok(false);
    }
    let key = format!(
        "{}:{}:{event_key}",
        session.id,
        match kind {
            Kind::Complete => "complete",
            Kind::Error => "error",
            Kind::Approval => "approval",
        }
    );
    {
        let mut sent = notifications.sent.lock().map_err(|e| e.to_string())?;
        if !claim(&mut sent, key.clone(), Instant::now()) {
            return Ok(false);
        }
    }
    let project = state
        .store
        .projects()?
        .into_iter()
        .find(|p| p.id == session.project_id)
        .map(|p| p.name)
        .unwrap_or_default();
    let (title, body) = text(kind, preferences.details, &session.title, &project);
    let result = show(app, &title, &body, preferences.sound, Some(session.id));
    if result.is_err() {
        notifications
            .sent
            .lock()
            .map_err(|e| e.to_string())?
            .remove(&key);
    }
    result.map(|_| true)
}

#[tauri::command]
pub async fn notification_status(app: AppHandle) -> Status {
    tauri::async_runtime::spawn_blocking(move || platform_status(&app))
        .await
        .unwrap_or(Status {
            state: "unavailable",
            message: "暂时无法检查系统通知状态".into(),
        })
}
#[tauri::command]
pub async fn test_system_notification(app: AppHandle, sound: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        show(
            &app,
            "系统通知测试",
            "这是一条测试通知。点击后返回 SuperCode 的通知设置。",
            sound,
            None,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn open_system_notification_settings(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    #[cfg(windows)]
    return app
        .opener()
        .open_url("ms-settings:notifications", None::<String>)
        .map_err(|e| e.to_string());
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("请在系统设置中打开通知选项".into())
    }
}

#[cfg(windows)]
fn register(app: &AppHandle) -> Result<(), String> {
    use windows_registry::CURRENT_USER;
    let icon = app
        .state::<crate::AppState>()
        .data_dir
        .join("notification-icon.png");
    let icon_bytes = include_bytes!("../icons/128x128.png");
    if std::fs::read(&icon).ok().as_deref() != Some(icon_bytes.as_slice()) {
        std::fs::write(&icon, icon_bytes).map_err(|e| e.to_string())?;
    }
    // Register only this application's identity; never change OS notification permissions.
    let key = CURRENT_USER
        .create(format!(
            r"SOFTWARE\Classes\AppUserModelId\{}",
            app.config().identifier
        ))
        .map_err(|e| e.to_string())?;
    key.set_string("DisplayName", "SuperCode")
        .map_err(|e| e.to_string())?;
    key.set_string("IconBackgroundColor", "0")
        .map_err(|e| e.to_string())?;
    key.set_string("IconUri", &icon.to_string_lossy())
        .map_err(|e| e.to_string())
}
#[cfg(windows)]
fn platform_status(app: &AppHandle) -> Status {
    use windows::{
        core::HSTRING,
        UI::Notifications::{NotificationSetting, ToastNotificationManager},
    };
    let result = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
        &app.config().identifier,
    ))
    .and_then(|notifier| notifier.Setting());
    let (state, message) = match result {
        Ok(NotificationSetting::Enabled) => ("enabled", "系统允许发送通知"),
        Ok(NotificationSetting::DisabledForApplication) => {
            ("appBlocked", "SuperCode 的系统通知已关闭")
        }
        Ok(NotificationSetting::DisabledForUser) => ("systemBlocked", "系统通知已关闭"),
        Ok(
            NotificationSetting::DisabledByGroupPolicy | NotificationSetting::DisabledByManifest,
        ) => ("policyBlocked", "系统策略不允许发送通知"),
        _ => ("unavailable", "暂时无法检查系统通知状态"),
    };
    Status {
        state,
        message: message.into(),
    }
}
#[cfg(windows)]
fn show(
    app: &AppHandle,
    title: &str,
    body: &str,
    sound: bool,
    session_id: Option<String>,
) -> Result<(), String> {
    use tauri_winrt_notification::{Sound, Toast};
    register(app)?;
    let status = platform_status(app);
    if status.state != "enabled" {
        return Err(status.message);
    }
    let handle = app.clone();
    Toast::new(&app.config().identifier)
        .title(title)
        .text1(body)
        .sound(sound.then_some(Sound::Default))
        .add_button(
            if session_id.is_some() {
                "查看对话"
            } else {
                "返回设置"
            },
            "open",
        )
        .on_activated(move |action| {
            if action.is_none() || action.as_deref() == Some("open") {
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = handle.emit_to(
                        "main",
                        "notification-open",
                        OpenTask {
                            session_id: session_id.clone(),
                        },
                    );
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            Ok(())
        })
        .show()
        .map_err(|e| format!("系统通知未发送：{e}"))
}
#[cfg(not(windows))]
fn platform_status(_app: &AppHandle) -> Status {
    Status {
        state: "enabled",
        message: "系统通知可用，具体显示由系统控制".into(),
    }
}
#[cfg(not(windows))]
fn show(
    app: &AppHandle,
    title: &str,
    body: &str,
    _sound: bool,
    _session_id: Option<String>,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes_notify_only_the_unseen_task_or_background_as_requested() {
        let mut prefs = Preferences::default();
        assert!(permitted(&prefs, Kind::Complete, true, false));
        assert!(!permitted(&prefs, Kind::Complete, true, true));
        prefs.mode = Mode::Background;
        assert!(!permitted(&prefs, Kind::Error, true, false));
        assert!(permitted(&prefs, Kind::Error, false, false));
        prefs.mode = Mode::Always;
        assert!(permitted(&prefs, Kind::Approval, true, true));
        prefs.approval = false;
        assert!(!permitted(&prefs, Kind::Approval, false, false));
        prefs.enabled = false;
        assert!(!permitted(&prefs, Kind::Complete, false, false));
    }
    #[test]
    fn duplicates_across_windows_are_bounded_but_new_turns_are_not_suppressed() {
        let mut sent = HashMap::new();
        let now = Instant::now();
        assert!(claim(&mut sent, "a:complete:turn1".into(), now));
        assert!(!claim(&mut sent, "a:complete:turn1".into(), now));
        assert!(claim(&mut sent, "a:complete:turn2".into(), now));
        for index in 0..300 {
            claim(
                &mut sent,
                index.to_string(),
                now + Duration::from_secs(index),
            );
        }
        assert_eq!(sent.len(), 256);
        assert!(claim(
            &mut sent,
            "expired".into(),
            now + Duration::from_secs(90000)
        ));
        assert_eq!(sent.len(), 1);
    }
    #[test]
    fn private_notifications_never_include_task_or_project_and_titles_are_bounded() {
        let (title, body) = text(Kind::Approval, false, "private task", "private project");
        assert_eq!(title, "需要你的确认");
        assert!(!body.contains("private"));
        let (_, body) = text(Kind::Complete, true, &"中".repeat(200), "项目\n换行");
        assert!(body.contains("项目换行"));
        assert!(!body.contains(&"中".repeat(73)));
    }
    #[test]
    fn only_the_live_turn_can_complete_and_stops_never_notify() {
        use crate::storage::Store;
        use rusqlite::Connection;
        use serde_json::json;
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store
            .add_project(std::path::Path::new("D:/notification-test"))
            .unwrap();
        let chat = store.create_session(&project.id, None).unwrap();
        store.set_status(&chat.id, "running", Some("t1")).unwrap();
        let live = store.session(&chat.id).unwrap();
        let event = |id: &str, status: &str| json!({"method":"turn/completed","params":{"turn":{"id":id,"status":status}}});
        assert!(matches!(
            task_event(&live, &event("t1", "completed")),
            Some((Kind::Complete, _))
        ));
        assert!(matches!(
            task_event(&live, &event("t1", "failed")),
            Some((Kind::Error, _))
        ));
        assert!(task_event(&live, &event("t0", "completed")).is_none());
        assert!(task_event(&live, &event("t1", "interrupted")).is_none());
        store.complete_turn(&chat.id, "t1", "completed").unwrap();
        assert!(task_event(&store.session(&chat.id).unwrap(), &event("t1", "completed")).is_none());
        store.set_status(&chat.id, "running", Some("t2")).unwrap();
        assert!(task_event(&store.session(&chat.id).unwrap(), &event("t1", "completed")).is_none());
        let approval = json!({"id":0,"method":"item/tool/requestUserInput","params":{}});
        assert!(matches!(
            task_event(&live, &approval),
            Some((Kind::Approval, _))
        ));
        assert_ne!(
            task_event(&live, &approval).unwrap().1,
            task_event(&store.session(&chat.id).unwrap(), &approval)
                .unwrap()
                .1
        );
    }
}
