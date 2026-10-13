//! Application-owned WebView2 sessions, independent of their panel visibility.
use crate::{browser_bridge, client_features::ToolServer, AppState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, Webview};

#[derive(Clone)]
struct Lease {
    session: String,
    agent: String,
    turn: Option<String>,
    gate: Arc<AtomicBool>,
    presented: Arc<AtomicBool>,
}
impl Lease {
    fn check(&self) -> Result<(), String> {
        if self.gate.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err("user_takeover".into())
        }
    }
}
fn select_lease(leases: &HashMap<String, Lease>, agent: &str, session: Option<&str>) -> Option<Lease> {
    if let Some(id) = session { return leases.get(id).filter(|l| l.agent == agent).cloned(); }
    let mut matches = leases.values().filter(|l| l.agent == agent && l.gate.load(Ordering::Acquire));
    let only = matches.next()?.clone();
    matches.next().is_none().then_some(only)
}
struct Observation {
    id: String,
    context: i64,
    revision: u64,
    at: Instant,
}
pub(crate) struct Tab {
    pub session: String,
    pub label: String,
    pub window: String,
    pub visible: Arc<AtomicBool>,
    mount: Mutex<String>,
    pub focused: AtomicBool,
    pub recovery_attempts: Mutex<Vec<Instant>>,
    pub hwnd: isize,
    pub scale: AtomicU64,
    pub rect: Mutex<(f64, f64, f64, f64)>,
    used: Mutex<Instant>,
    layout_revision: AtomicU64,
    revision: AtomicU64,
    observation: Mutex<Option<Observation>>,
    operation: tokio::sync::Mutex<()>,
    status: Mutex<String>,
    action: Mutex<String>,
    location: Mutex<(String, String)>,
}
#[derive(Default)]
pub struct Browser {
    tabs: Mutex<HashMap<String, Arc<Tab>>>,
    creating: tokio::sync::Mutex<()>,
    leases: Mutex<HashMap<String, Lease>>,
    endpoint: Mutex<Option<(String, String)>>,
    listener: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    calls: Mutex<HashMap<(String, String), Arc<AtomicBool>>>,
    cancelled_calls: Mutex<VecDeque<((String, String), Instant)>>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub session_id: String,
    pub url: String,
    pub title: String,
    pub status: String,
    pub action: String,
}
fn state(tab: &Tab) -> BrowserState {
    let (url, title) = tab.location.lock().unwrap().clone();
    BrowserState {
        session_id: tab.session.clone(),
        url,
        title,
        status: tab.status.lock().unwrap().clone(),
        action: tab.action.lock().unwrap().clone(),
    }
}
fn publish(app: &AppHandle, tab: &Tab) {
    let _ = app.emit_to(tab.window.clone(), "browser-state", state(tab));
}
fn status(app: &AppHandle, tab: &Tab, phase: &str, action: &str) {
    *tab.status.lock().unwrap() = phase.into();
    *tab.action.lock().unwrap() = action.into();
    publish(app, tab);
}
pub(crate) fn failure(app: &AppHandle, tab: &Tab, message: &str) {
    status(app, tab, "error", message);
}
fn tab(app: &AppHandle, session: &str) -> Result<Arc<Tab>, String> {
    app.state::<Browser>()
        .tabs
        .lock()
        .unwrap()
        .get(session)
        .cloned()
        .ok_or("当前聊天没有打开浏览器".into())
}
pub fn server() -> Result<ToolServer, String> {
    Ok(ToolServer {
        id: "browser".into(),
        name: "浏览器 · SuperCode 内置".into(),
        kind: "browser".into(),
        command: std::env::current_exe()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned(),
        args: vec!["--browser-mcp".into()],
        url: None,
        enabled: cfg!(windows),
    })
}
pub fn with_builtin(mut saved: Vec<ToolServer>) -> Result<Vec<ToolServer>, String> {
    let mut builtin = server()?;
    if saved
        .iter()
        .any(|s| s.id == "browser" && s.kind != "browser")
    {
        return Ok(saved);
    }
    builtin.enabled = saved
        .iter()
        .find(|s| s.id == "browser")
        .map(|s| s.enabled)
        .unwrap_or(builtin.enabled);
    saved.retain(|s| s.id != "browser");
    saved.push(builtin);
    Ok(saved)
}
pub fn wrap(app: &AppHandle, mut server: ToolServer, agent: &str) -> Result<ToolServer, String> {
    if server.enabled
        && server.id == "browser"
        && server.kind == "browser"
        && server.args.first().is_some_and(|a| a == "--browser-mcp")
    {
        let (address, token) = app
            .state::<Browser>()
            .endpoint
            .lock()
            .unwrap()
            .clone()
            .ok_or("内置浏览器未连接")?;
        server.args = vec!["--browser-mcp".into(), address, token, agent.into()];
    }
    Ok(server)
}
pub fn authorize_session(app: &AppHandle, agent: &str, session: &str) {
    let state = app.state::<Browser>();
    let mut slots = state.leases.lock().unwrap();
    if let Some(old) = slots.get(session) {
        old.gate.store(false, Ordering::Release);
    }
    slots.insert(session.into(), Lease {
        session: session.into(),
        agent: agent.into(),
        turn: None,
        gate: Arc::new(AtomicBool::new(true)),
        presented: Arc::new(AtomicBool::new(false)),
    });
    if let Ok(tab) = tab(app, session) {
        tab.observation.lock().unwrap().take();
        status(app, &tab, "idle", "");
    }
}
pub fn observe(app: &AppHandle, event: &Value) {
    let method = event["method"].as_str().unwrap_or("");
    if !matches!(method, "turn/started" | "turn/completed") {
        return;
    }
    let Some(native) = event["params"]["threadId"].as_str() else {
        return;
    };
    let Ok(session) = app.state::<AppState>().store.for_native(native) else {
        return;
    };
    let state = app.state::<Browser>();
    let mut owners = state.leases.lock().unwrap();
    let Some(lease) = owners.get_mut(&session.id) else {
        return;
    };
    let turn = event["params"]["turn"]["id"]
        .as_str()
        .or(event["params"]["turnId"].as_str());
    if method == "turn/started" {
        if lease.turn.is_none() {
            lease.turn = turn.map(str::to_owned);
        }
        return;
    }
    if lease.turn.as_deref() != turn {
        return;
    }
    lease.gate.store(false, Ordering::Release);
    if let Ok(tab) = tab(app, &session.id) {
        if *tab.status.lock().unwrap() != "stopped" {
            status(app, &tab, "idle", "");
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = cue(&app, &tab, None, false, None).await;
        });
    }
}
pub(crate) fn active_tab(app: &AppHandle) -> Option<Arc<Tab>> {
    let state = app.state::<Browser>();
    let leases = state.leases.lock().unwrap();
    let tabs = state.tabs.lock().unwrap();
    tabs.values().filter(|tab| leases.get(&tab.session).is_some_and(|l| l.gate.load(Ordering::Acquire))
        && matches!(tab.status.lock().unwrap().as_str(), "working" | "error"))
        .max_by_key(|tab| *tab.used.lock().unwrap()).cloned()
}
pub fn stop(app: &AppHandle, session: &str) {
    stop_with_reason(app, session, "browser_control_revoked");
}
fn revoke_control(browser: &Browser, session: &str) -> bool {
    browser.leases.lock().unwrap().get(session)
        .is_some_and(|lease| lease.gate.swap(false, Ordering::AcqRel))
}
pub(crate) fn stop_with_reason(app: &AppHandle, session: &str, reason: &'static str) {
    let state = app.state::<Browser>();
    let had_owner = revoke_control(&state, session);
    if let Ok(tab) = tab(app, session) {
        status(app, &tab, "stopped", "已由你接管");
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = cue(&app, &tab, None, false, None).await;
        });
    }
    // Taking over browser input cancels its tools, while the agent can continue
    // other work. Only the chat's Stop action may interrupt the whole turn.
    if had_owner {
        let app = app.clone();
        let session = session.to_owned();
        tauri::async_runtime::spawn(async move {
            crate::runtime_diagnostics::record(&app, Some(&session), "browser_takeover", json!({"reason":reason}));
        });
    }
}
pub(crate) fn invalidate_observation(tab: &Tab) {
    tab.revision.fetch_add(1, Ordering::AcqRel);
    tab.observation.lock().unwrap().take();
}
pub fn shutdown(app: &AppHandle) {
    let state = app.state::<Browser>();
    for lease in state.leases.lock().unwrap().values() {
        lease.gate.store(false, Ordering::Release);
    }
    if let Some(task) = state.listener.lock().unwrap().take() {
        task.abort();
    };
}
pub fn url(value: &str) -> Result<tauri::Url, String> {
    let url = tauri::Url::parse(value).map_err(|_| "请输入完整的 http 或 https 地址")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("浏览器只支持不含账号密码的 HTTP(S) 地址".into());
    }
    Ok(url)
}

async fn ensure_tab(app: &AppHandle, session: &str, window: &str) -> Result<Arc<Tab>, String> {
    if let Ok(tab) = tab(app, session) {
        if tab.window != window {
            return Err("此聊天的浏览器已在另一个窗口打开".into());
        }
        *tab.used.lock().unwrap() = Instant::now();
        return Ok(tab);
    }
    let state = app.state::<Browser>();
    let _creating = state.creating.lock().await;
    if let Ok(tab) = tab(app, session) {
        if tab.window != window {
            return Err("此聊天的浏览器已在另一个窗口打开".into());
        }
        *tab.used.lock().unwrap() = Instant::now();
        return Ok(tab);
    }
    if state.tabs.lock().unwrap().len() >= 8 {
        let leases = state.leases.lock().unwrap();
        let mut tabs = state.tabs.lock().unwrap();
        let owners = leases.values().cloned().collect::<Vec<_>>();
        let candidate = inactive_hidden_tab(&tabs, &owners);
        let old = candidate.and_then(|id| tabs.remove(&id));
        drop(tabs);
        drop(leases);
        if let Some(old) = old {
            if let Some(view) = app.get_webview(&old.label) {
                view.close().map_err(|e| e.to_string())?;
            }
        }
    }
    let parent = app.get_window(window).ok_or("浏览器所属窗口已关闭")?;
    #[cfg(windows)]
    let hwnd = parent.hwnd().map_err(|e| e.to_string())?.0 as isize;
    #[cfg(not(windows))]
    let hwnd = 0;
    let scale = parent.scale_factor().map_err(|e| e.to_string())?;
    let label = format!("browser-{}", uuid::Uuid::new_v4().simple());
    let tab = Arc::new(Tab {
        session: session.into(),
        label: label.clone(),
        window: window.into(),
        visible: Arc::new(AtomicBool::new(false)),
        mount: Mutex::new(String::new()),
        focused: AtomicBool::new(false),
        recovery_attempts: Mutex::new(Vec::new()),
        hwnd,
        scale: AtomicU64::new(scale.to_bits()),
        rect: Mutex::new((10000., 0., 960., 640.)),
        used: Mutex::new(Instant::now()),
        layout_revision: AtomicU64::new(0),
        revision: AtomicU64::new(0),
        observation: Mutex::new(None),
        operation: tokio::sync::Mutex::new(()),
        status: Mutex::new("idle".into()),
        action: Mutex::new(String::new()),
        location: Mutex::new(("about:blank".into(), "新标签页".into())),
    });
    let path = app
        .state::<AppState>()
        .data_dir
        .join("browser-profiles")
        .join(session);
    let tab_page = tab.clone();
    let app_page = app.clone();
    let tab_nav = tab.clone();
    let builder=tauri::webview::WebviewBuilder::new(&label,tauri::WebviewUrl::External(tauri::Url::parse("about:blank").unwrap()))
        .data_directory(path)
        .on_navigation(move|address|{tab_nav.revision.fetch_add(1,Ordering::AcqRel);address.as_str()=="about:blank" || url(address.as_str()).is_ok()})
        .on_page_load(move|_view,payload|{
            tab_page.revision.fetch_add(1,Ordering::AcqRel);
            tab_page.location.lock().unwrap().0 = payload.url().as_str().into();
            let _=app_page.emit_to(tab_page.window.clone(),"browser-location",json!({"sessionId":tab_page.session,"url":payload.url().as_str(),"loading":matches!(payload.event(),tauri::webview::PageLoadEvent::Started)}));
        })
        .on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny);
    let view = parent
        .add_child(
            builder,
            tauri::LogicalPosition::new(10000., 0.),
            tauri::LogicalSize::new(960., 640.),
        )
        .map_err(|e| e.to_string())?;
    view.hide().map_err(|e| e.to_string())?;
    crate::browser_native::attach(&view, &tab);
    state
        .tabs
        .lock()
        .unwrap()
        .insert(session.into(), tab.clone());
    Ok(tab)
}
fn inactive_hidden_tab(tabs: &HashMap<String, Arc<Tab>>, leases: &[Lease]) -> Option<String> {
    tabs.iter()
        .filter(|(id, tab)| {
            !tab.visible.load(Ordering::Acquire)
                && !leases.iter().any(|lease| {
                    lease.session == **id && lease.gate.load(Ordering::Acquire)
                })
                && tab.operation.try_lock().is_ok()
        })
        .min_by_key(|(_, tab)| *tab.used.lock().unwrap())
        .map(|(id, _)| id.clone())
}
fn hide_panel(app: &AppHandle, tab: &Tab) {
    tab.visible.store(false, Ordering::Release);
    tab.focused.store(false, Ordering::Release);
    // A user can dismiss the UI while the agent is still preparing its first call.
    // Mark this turn as presented so that call will not reopen the dismissed panel.
    if let Some(lease) = app
        .state::<Browser>()
        .leases
        .lock()
        .unwrap()
        .get(&tab.session)
    {
        lease.presented.store(true, Ordering::Release);
    }
}
fn park_view(view: &Webview) -> Result<(), String> {
    // WebView2 stops producing screenshots when its controller is hidden.
    // Keep the page rendering outside the window without changing its viewport.
    view.set_position(tauri::LogicalPosition::new(10000., 0.))
        .map_err(|e| e.to_string())?;
    view.show().map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn browser_open(
    session_id: String,
    mount_id: String,
    webview: Webview,
    app: AppHandle,
) -> Result<BrowserState, String> {
    app.state::<AppState>().store.session(&session_id)?;
    let tab = ensure_tab(&app, &session_id, webview.window().label()).await?;
    *tab.mount.lock().unwrap() = mount_id;
    Ok(state(&tab))
}
#[tauri::command]
pub async fn browser_layout(
    session_id: String,
    mount_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
    webview: Webview,
    app: AppHandle,
) -> Result<(), String> {
    let tab = tab(&app, &session_id)?;
    if *tab.mount.lock().unwrap() != mount_id {
        return Ok(());
    }
    if tab.window != webview.window().label() {
        return Err("浏览器不属于此窗口".into());
    }
    if ![x, y, width, height].iter().all(|v| v.is_finite())
        || x < 0.
        || y < 0.
        || width < 0.
        || height < 0.
        || width > 12000.
        || height > 12000.
    {
        return Err("浏览器区域无效".into());
    }
    let view = app.get_webview(&tab.label).ok_or("浏览器已关闭")?;
    let layout_revision = tab.layout_revision.fetch_add(1, Ordering::AcqRel) + 1;
    let shown = visible && width >= 180. && height >= 120.;
    if shown {
        // Resizing waits for an in-flight action, so its observed coordinates stay valid.
        let _operation = tab.operation.lock().await;
        // An old mount/layout must not reveal a panel that was closed while waiting.
        if *tab.mount.lock().unwrap() != mount_id
            || tab.layout_revision.load(Ordering::Acquire) != layout_revision
        {
            return Ok(());
        }
        let bounds = (x, y, width, height);
        if *tab.rect.lock().unwrap() != bounds {
            tab.revision.fetch_add(1, Ordering::AcqRel);
            *tab.rect.lock().unwrap() = bounds;
        }
        if let Some(window) = app.get_window(&tab.window) {
            if let Ok(scale) = window.scale_factor() {
                tab.scale.store(scale.to_bits(), Ordering::Release);
            }
        }
        view.set_bounds(tauri::Rect {
            position: tauri::LogicalPosition::new(x, y).into(),
            size: tauri::LogicalSize::new(width, height).into(),
        })
        .map_err(|e| e.to_string())?;
        if tab.layout_revision.load(Ordering::Acquire) != layout_revision {
            return Ok(());
        }
        view.show().map_err(|e| e.to_string())?;
        tab.visible.store(true, Ordering::Release);
    } else {
        hide_panel(&app, &tab);
        // Preserve the viewport and observation: hiding the UI is not a page resize.
        park_view(&view)?;
    }
    Ok(())
}
#[tauri::command]
pub fn browser_takeover(session_id: String, app: AppHandle) {
    stop_with_reason(&app, &session_id, "takeover_button_or_escape");
}
#[tauri::command]
pub async fn browser_close(
    session_id: String,
    mount_id: String,
    webview: Webview,
    app: AppHandle,
) -> Result<(), String> {
    let Ok(tab) = tab(&app, &session_id) else {
        return Ok(());
    };
    if tab.window != webview.window().label() {
        return Err("浏览器不属于此窗口".into());
    }
    if *tab.mount.lock().unwrap() != mount_id {
        return Ok(());
    }
    tab.layout_revision.fetch_add(1, Ordering::AcqRel);
    hide_panel(&app, &tab);
    if let Some(view) = app.get_webview(&tab.label) {
        park_view(&view)?;
    }
    Ok(())
}
#[tauri::command]
pub async fn browser_manual(
    session_id: String,
    action: String,
    address: Option<String>,
    webview: Webview,
    app: AppHandle,
) -> Result<(), String> {
    let tab = tab(&app, &session_id)?;
    if tab.window != webview.window().label() {
        return Err("浏览器不属于此窗口".into());
    }
    stop_with_reason(&app, &session_id, "manual_browser_navigation");
    let _operation = tab.operation.lock().await;
    match action.as_str() {
        "navigate" => {
            let address = url(address.as_deref().unwrap_or(""))?;
            app.get_webview(&tab.label)
                .ok_or("浏览器已关闭")?
                .navigate(address)
                .map_err(|e| e.to_string())?;
        }
        "reload" => {
            cdp(&app, &tab, "Page.reload", json!({}), None).await?;
        }
        "back" | "forward" => {
            let history = cdp(&app, &tab, "Page.getNavigationHistory", json!({}), None).await?;
            let index = history["currentIndex"].as_i64().unwrap_or(0)
                + if action == "back" { -1 } else { 1 };
            if index >= 0 {
                if let Some(id) = history["entries"][index as usize]["id"].as_i64() {
                    cdp(
                        &app,
                        &tab,
                        "Page.navigateToHistoryEntry",
                        json!({"entryId":id}),
                        None,
                    )
                    .await?;
                }
            }
        }
        _ => return Err("未知浏览器操作".into()),
    }
    Ok(())
}

async fn cancelled(lease: &Lease) {
    while lease.gate.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn operation<'a>(
    tab: &'a Tab,
    lease: &Lease,
) -> Result<tokio::sync::MutexGuard<'a, ()>, String> {
    lease.check()?;
    // React can submit a layout update between two tool calls. It holds this lock
    // briefly even when bounds are unchanged; do not report that as a tool failure.
    let guard = tokio::select! {
        result = tokio::time::timeout(Duration::from_millis(250), tab.operation.lock()) =>
            result.map_err(|_| "另一个浏览器操作正在进行，请等待返回结果".to_string())?,
        _ = cancelled(lease) => return Err("user_takeover".into()),
    };
    lease.check()?;
    Ok(guard)
}
#[cfg(windows)]
async fn cdp(
    app: &AppHandle,
    tab: &Tab,
    method: &str,
    args: Value,
    lease: Option<&Lease>,
) -> Result<Value, String> {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::{HSTRING, PCWSTR};
    if let Some(lease) = lease {
        lease.check()?;
    }
    let view = app.get_webview(&tab.label).ok_or("浏览器已关闭")?;
    let method = method.to_owned();
    let parameters = args.to_string();
    let gate = lease.map(|l| l.gate.clone());
    let (tx, rx) = tokio::sync::oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(tx)));
    let completion = sender.clone();
    view.with_webview(move |platform| unsafe {
        if gate.as_ref().is_some_and(|g| !g.load(Ordering::Acquire)) {
            if let Some(tx) = sender.lock().unwrap().take() {
                let _ = tx.send(Err("user_takeover".into()));
            }
            return;
        }
        let result = (|| -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?;
            let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                move |error, result| {
                    let parsed = match error {
                        Err(error) => Err(error.to_string()),
                        Ok(()) => serde_json::from_str::<Value>(&result).map_err(|e| e.to_string()),
                    };
                    if let Some(tx) = completion.lock().unwrap().take() {
                        let _ = tx.send(parsed);
                    }
                    Ok(())
                },
            ));
            let method = HSTRING::from(method);
            let params = HSTRING::from(parameters);
            core.CallDevToolsProtocolMethod(
                PCWSTR(method.as_ptr()),
                PCWSTR(params.as_ptr()),
                &handler,
            )
        })();
        if let Err(error) = result {
            if let Some(tx) = sender.lock().unwrap().take() {
                let _ = tx.send(Err(error.to_string()));
            }
        }
    })
    .map_err(|e| e.to_string())?;
    let response = async {
        tokio::time::timeout(Duration::from_secs(8), rx)
            .await
            .map_err(|_| "浏览器响应超时".to_string())?
            .map_err(|_| "浏览器已关闭".to_string())?
    };
    let value = if let Some(lease) = lease {
        tokio::select! {v=response=>v?,_=cancelled(lease)=>return Err("user_takeover".into())}
    } else {
        response.await?
    };
    if let Some(lease) = lease {
        lease.check()?;
    }
    Ok(value)
}
#[cfg(not(windows))]
async fn cdp(
    _: &AppHandle,
    _: &Tab,
    _: &str,
    _: Value,
    _: Option<&Lease>,
) -> Result<Value, String> {
    Err("内置浏览器自动化目前支持 Windows".into())
}
async fn evaluate(
    app: &AppHandle,
    tab: &Tab,
    expression: String,
    context: Option<i64>,
    lease: Option<&Lease>,
) -> Result<Value, String> {
    let mut args = json!({"expression":expression,"returnByValue":true,"awaitPromise":false});
    if let Some(context) = context {
        args["contextId"] = json!(context);
    }
    let value = cdp(app, tab, "Runtime.evaluate", args, lease).await?;
    if !value["exceptionDetails"].is_null() {
        return Err("网页发生变化，请重新读取浏览器快照".into());
    }
    Ok(value["result"]["value"].clone())
}
async fn cue(
    app: &AppHandle,
    tab: &Tab,
    point: Option<Value>,
    active: bool,
    lease: Option<&Lease>,
) -> Result<(), String> {
    evaluate(
        app,
        tab,
        format!(
            "({})({}, {})",
            include_str!("browser_cue.js"),
            json!(point),
            active
        ),
        None,
        lease,
    )
    .await?;
    Ok(())
}
async fn snapshot(app: &AppHandle, tab: &Tab, lease: &Lease) -> Result<Value, String> {
    lease.check()?;
    cue(app, tab, None, true, Some(lease)).await?;
    let tree = cdp(app, tab, "Page.getFrameTree", json!({}), Some(lease)).await?;
    let world = cdp(
        app,
        tab,
        "Page.createIsolatedWorld",
        json!({"frameId":tree["frameTree"]["frame"]["id"],"worldName":"SuperCodeBrowser"}),
        Some(lease),
    )
    .await?;
    let context = world["executionContextId"]
        .as_i64()
        .ok_or("网页还未准备好，请重新读取快照")?;
    let revision = tab.revision.load(Ordering::Acquire);
    let mut value = evaluate(
        app,
        tab,
        include_str!("browser_page.js").into(),
        Some(context),
        Some(lease),
    )
    .await?;
    if revision != tab.revision.load(Ordering::Acquire) {
        return Err("网页或面板尺寸发生变化，请重新读取快照".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    value["snapshotId"] = json!(id);
    *tab.observation.lock().unwrap() = Some(Observation {
        id,
        context,
        revision,
        at: Instant::now(),
    });
    *tab.location.lock().unwrap() = (
        value["url"].as_str().unwrap_or("").into(),
        value["title"].as_str().unwrap_or("").into(),
    );
    let _ = app.emit_to(
        tab.window.clone(),
        "browser-location",
        json!({"sessionId":tab.session,"url":value["url"],"title":value["title"],"loading":false}),
    );
    let image = cdp(
        app,
        tab,
        "Page.captureScreenshot",
        json!({"format":"jpeg","quality":65,"captureBeyondViewport":false}),
        Some(lease),
    )
    .await;
    let mut content = vec![json!({"type":"text","text":value.to_string()})];
    match image {Ok(image) if image["data"].as_str().is_some_and(|s|s.len()<4*1024*1024)=>content.push(json!({"type":"image","mimeType":"image/jpeg","data":image["data"]})),Err(error)=>content.push(json!({"type":"text","text":format!("Screenshot unavailable: {error}. The DOM snapshot above is real; do not pretend you saw an image.")})),_=>{}}
    lease.check()?;
    Ok(json!({"content":content,"isError":false}))
}
fn observation(tab: &Tab, id: &Value) -> Result<i64, String> {
    let lock = tab.observation.lock().unwrap();
    let observed = lock.as_ref().ok_or("请先读取浏览器快照")?;
    if id.as_str() != Some(observed.id.as_str())
        || observed.at.elapsed() > Duration::from_secs(60)
        || observed.revision != tab.revision.load(Ordering::Acquire)
    {
        return Err("浏览器快照已过期，请重新读取后操作".into());
    }
    Ok(observed.context)
}
async fn point(
    app: &AppHandle,
    tab: &Tab,
    args: &Value,
    context: i64,
    lease: &Lease,
) -> Result<Value, String> {
    if observation(tab, &args["snapshotId"])? != context {
        return Err("浏览器快照已过期，请重新读取后操作".into());
    }
    let reference = json!(args["ref"].as_str().unwrap());
    let expression = format!("({})({reference})", include_str!("browser_target.js"));
    let value = evaluate(app, tab, expression, Some(context), Some(lease)).await?;
    observation(tab, &args["snapshotId"])?;
    match value["status"].as_str() {
        Some("changed") => return Err("控件内容或目标已经变化，请重新读取快照".into()),
        Some("ambiguous") => {
            return Err("页面出现多个相同目标，无法安全定位，请重新读取快照".into())
        }
        Some("ready" | "missing" | "hidden" | "disabled" | "blocked") => {}
        _ => return Err("控件无法定位，请重新读取快照".into()),
    }
    Ok(value)
}
fn same_point(a: &Value, b: &Value) -> bool {
    a["status"] == "ready"
        && b["status"] == "ready"
        && a["generation"] == b["generation"]
        && ["x", "y", "width", "height"].iter().all(|key| {
            a["rect"][key]
                .as_f64()
                .zip(b["rect"][key].as_f64())
                .is_some_and(|(a, b)| (a - b).abs() <= 1.)
        })
        && ["x", "y"].iter().all(|key| {
            a[key]
                .as_f64()
                .zip(b[key].as_f64())
                .is_some_and(|(a, b)| (a - b).abs() <= 1.)
        })
}
// Retry only observations/hover before mouse-down, never a dispatched click.
async fn ready_point(
    app: &AppHandle,
    tab: &Tab,
    args: &Value,
    context: i64,
    lease: &Lease,
    fill: bool,
) -> Result<Value, String> {
    let mut stable: Option<(Value, Instant)> = None;
    loop {
        lease.check()?;
        let current = point(app, tab, args, context, lease).await?;
        if fill && current["status"] == "ready" && current["textField"] != true {
            return Err("此控件不是文本输入框".into());
        }
        if current["status"] == "ready" && (!fill || current["editable"] == true) {
            if let Some((previous, since)) = &stable {
                if same_point(previous, &current) && since.elapsed() >= Duration::from_millis(120) {
                    cdp(
                        app,
                        tab,
                        "Input.dispatchMouseEvent",
                        json!({"type":"mouseMoved","x":current["x"],"y":current["y"]}),
                        Some(lease),
                    )
                    .await?;
                    // Hover handlers can insert overlays or replace/move a control.
                    let hovered = point(app, tab, args, context, lease).await?;
                    if same_point(&current, &hovered) {
                        cue(
                            app,
                            tab,
                            Some(json!({"x":hovered["x"],"y":hovered["y"],"click":true})),
                            true,
                            Some(lease),
                        )
                        .await?;
                        let checked = point(app, tab, args, context, lease).await?;
                        if same_point(&hovered, &checked) && (!fill || checked["editable"] == true)
                        {
                            return Ok(checked);
                        }
                    }
                    stable = None;
                } else if !same_point(previous, &current) {
                    stable = None;
                }
            }
            if stable.is_none() {
                cue(
                    app,
                    tab,
                    Some(json!({"x":current["x"],"y":current["y"]})),
                    true,
                    Some(lease),
                )
                .await?;
                stable = Some((current, Instant::now()));
            }
        } else {
            stable = None;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(60)) => {},
            _ = cancelled(lease) => return Err("user_takeover".into()),
        }
    }
}
async fn focused_field(
    app: &AppHandle,
    tab: &Tab,
    args: &Value,
    context: i64,
    lease: &Lease,
) -> Result<(), String> {
    let expression = format!(
        "({})({})",
        include_str!("browser_target.js"),
        json!(args["ref"].as_str().unwrap())
    );
    let target = evaluate(app, tab, expression, Some(context), Some(lease)).await?;
    if target["status"] != "ready" || target["editable"] != true || target["focused"] != true {
        return Err("点击后输入框或焦点已经变化；未继续输入，请读取新快照确认结果".into());
    }
    Ok(())
}
async fn key(app: &AppHandle, tab: &Tab, value: &str, lease: &Lease) -> Result<(), String> {
    let (key, code, vk, modifiers) = match value.to_uppercase().as_str() {
        "ENTER" => ("Enter", "Enter", 13, 0),
        "TAB" => ("Tab", "Tab", 9, 0),
        "ESC" => ("Escape", "Escape", 27, 0),
        "BACKSPACE" => ("Backspace", "Backspace", 8, 0),
        "DELETE" => ("Delete", "Delete", 46, 0),
        "ARROWUP" => ("ArrowUp", "ArrowUp", 38, 0),
        "ARROWDOWN" => ("ArrowDown", "ArrowDown", 40, 0),
        "ARROWLEFT" => ("ArrowLeft", "ArrowLeft", 37, 0),
        "ARROWRIGHT" => ("ArrowRight", "ArrowRight", 39, 0),
        "HOME" => ("Home", "Home", 36, 0),
        "END" => ("End", "End", 35, 0),
        "CTRL+A" => ("a", "KeyA", 65, 2),
        _ => return Err("不支持此浏览器按键".into()),
    };
    cdp(app,tab,"Input.dispatchKeyEvent",json!({"type":"rawKeyDown","key":key,"code":code,"windowsVirtualKeyCode":vk,"modifiers":modifiers}),Some(lease)).await?;
    cdp(app,tab,"Input.dispatchKeyEvent",json!({"type":"keyUp","key":key,"code":code,"windowsVirtualKeyCode":vk,"modifiers":modifiers}),Some(lease)).await?;
    Ok(())
}
async fn execute(
    app: &AppHandle,
    tab: &Tab,
    lease: &Lease,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    browser_bridge::validate(name, &args)?;
    lease.check()?;
    let _operation = operation(tab, lease).await?;
    lease.check()?;
    if !tab.visible.load(Ordering::Acquire) {
        park_view(&app.get_webview(&tab.label).ok_or("浏览器已关闭")?)?;
    }
    lease.check()?;
    status(
        app,
        tab,
        "working",
        match name {
            "browser_navigate" => "正在打开网页",
            "browser_click" => "正在点击",
            "browser_fill" => "正在填写",
            "browser_scroll" => "正在滚动",
            "browser_press_key" => "正在按键",
            _ => "正在读取网页",
        },
    );
    let mut may_have_run = false;
    let action=async{
        cue(app,tab,None,true,Some(lease)).await?;
        if name=="browser_snapshot" {return snapshot(app,tab,lease).await;}
        if name=="browser_navigate" {
            let address=url(args["url"].as_str().unwrap())?;
            may_have_run=true;tab.observation.lock().unwrap().take();
            let navigation=cdp(app,tab,"Page.navigate",json!({"url":address.as_str()}),Some(lease)).await?;
            if let Some(error)=navigation["errorText"].as_str(){return Err(format!("网页加载失败：{error}"));}
        }else{
            let context=observation(tab,&args["snapshotId"])?;
            let point=if matches!(name,"browser_click"|"browser_fill"){
                let ready=tokio::time::timeout(Duration::from_secs(2),ready_point(app,tab,&args,context,lease,name=="browser_fill")).await;
                lease.check()?;
                Some(ready.map_err(|_|"目标在等待期间持续移动、不可用或被遮挡；尚未点击，请重新读取快照".to_string())??)
            }else{None};
            if name=="browser_press_key" && !["ENTER","TAB","ESC","BACKSPACE","DELETE","ARROWUP","ARROWDOWN","ARROWLEFT","ARROWRIGHT","HOME","END","CTRL+A"].contains(&args["key"].as_str().unwrap().to_uppercase().as_str()){return Err("不支持此浏览器按键".into());}
            if let Some(point)=point {
                lease.check()?;
                observation(tab,&args["snapshotId"])?;
                tab.observation.lock().unwrap().take();
                may_have_run=true;
                for phase in ["mousePressed","mouseReleased"]{cdp(app,tab,"Input.dispatchMouseEvent",json!({"type":phase,"x":point["x"],"y":point["y"],"button":"left","clickCount":1}),Some(lease)).await?;}
                if name=="browser_fill" {focused_field(app,tab,&args,context,lease).await?;key(app,tab,"CTRL+A",lease).await?;focused_field(app,tab,&args,context,lease).await?;cdp(app,tab,"Input.insertText",json!({"text":args["text"]}),Some(lease)).await?;}
            }else if name=="browser_scroll"{
                let center=evaluate(app,tab,"({x:innerWidth/2,y:innerHeight/2})".into(),Some(context),Some(lease)).await?;
                cue(app,tab,Some(center.clone()),true,Some(lease)).await?;
                tab.observation.lock().unwrap().take();may_have_run=true;cdp(app,tab,"Input.dispatchMouseEvent",json!({"type":"mouseWheel","x":center["x"],"y":center["y"],"deltaY":args["deltaY"],"deltaX":args.get("deltaX").cloned().unwrap_or(json!(0))}),Some(lease)).await?;
            }else{tab.observation.lock().unwrap().take();may_have_run=true;key(app,tab,args["key"].as_str().unwrap(),lease).await?;}
        }
        tokio::select!{_=tokio::time::sleep(Duration::from_millis(400))=>{},_=cancelled(lease)=>return Err("user_takeover".into())};
        match snapshot(app,tab,lease).await {
            Ok(value)=>Ok(value),
            Err(error) if error!="user_takeover"=>Ok(browser_bridge::text_result(json!({"completed":true,"observationWarning":error,"nextStep":"The action already ran. Do not repeat it. Read a new browser_snapshot to check the result."}),false)),
            Err(error)=>Err(error),
        }
    }.await;
    match action {
        Ok(value) => Ok(value),
        Err(error) if error == "user_takeover" => Ok(browser_bridge::takeover_result(may_have_run)),
        Err(error) => {
            status(app, tab, "error", &error);
            Ok(browser_bridge::text_result(
                json!({"error":error,"actionMayHaveRun":may_have_run,"nextStep":"Obtain a fresh observation. Never repeat an action that may already have run."}),
                true,
            ))
        }
    }
}

#[derive(Deserialize)]
struct Request {
    agent: String,
    #[serde(default)]
    session: Option<String>,
    id: String,
    name: String,
    arguments: Value,
}
struct CallScope {
    app: AppHandle,
    key: (String, String),
    gate: Arc<AtomicBool>,
}
impl Drop for CallScope {
    fn drop(&mut self) {
        let state = self.app.state::<Browser>();
        let mut calls = state.calls.lock().unwrap();
        if calls
            .get(&self.key)
            .is_some_and(|gate| Arc::ptr_eq(gate, &self.gate))
        {
            calls.remove(&self.key);
        }
    }
}
#[derive(Deserialize)]
struct Cancel {
    agent: String,
    id: String,
}
async fn cancel_request(
    axum::extract::State(app): axum::extract::State<AppHandle>,
    headers: axum::http::HeaderMap,
    axum::Json(request): axum::Json<Cancel>,
) -> axum::http::StatusCode {
    let state = app.state::<Browser>();
    let expected = state
        .endpoint
        .lock()
        .unwrap()
        .as_ref()
        .map(|(_, token)| format!("Bearer {token}"));
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != expected.as_deref() {
        return axum::http::StatusCode::UNAUTHORIZED;
    }
    if request.id.len() > 192
        || !matches!(
            request.agent.as_str(),
            "codex" | "claude" | "opencode" | "pi" | "gemini"
        )
    {
        return axum::http::StatusCode::BAD_REQUEST;
    }
    let calls = state.calls.lock().unwrap();
    let gate = calls
        .get(&(request.agent.clone(), request.id.clone()))
        .cloned();
    if let Some(gate) = gate {
        drop(calls);
        gate.store(false, Ordering::Release);
        let session = state
            .leases
            .lock()
            .unwrap()
            .values()
            .find(|l| Arc::ptr_eq(&l.gate, &gate))
            .map(|l| l.session.clone());
        if let Some(session) = session {
            stop(&app, &session);
        }
    } else {
        let mut pending = state.cancelled_calls.lock().unwrap();
        pending.retain(|(_, at)| at.elapsed() < Duration::from_secs(60));
        if pending.len() >= 64 {
            pending.pop_front();
        }
        pending.push_back(((request.agent, request.id), Instant::now()));
    }
    axum::http::StatusCode::NO_CONTENT
}
async fn request(
    axum::extract::State(app): axum::extract::State<AppHandle>,
    headers: axum::http::HeaderMap,
    axum::Json(request): axum::Json<Request>,
) -> Result<axum::Json<Value>, axum::http::StatusCode> {
    let state = app.state::<Browser>();
    let expected = state
        .endpoint
        .lock()
        .unwrap()
        .as_ref()
        .map(|(_, token)| format!("Bearer {token}"));
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != expected.as_deref() {
        return Err(axum::http::StatusCode::UNAUTHORIZED);
    }
    if request.id.len() > 192
        || !matches!(
            request.agent.as_str(),
            "codex" | "claude" | "opencode" | "pi" | "gemini"
        )
    {
        return Err(axum::http::StatusCode::BAD_REQUEST);
    }
    let lease = select_lease(&state.leases.lock().unwrap(), &request.agent, request.session.as_deref());
    let Some(lease) = lease else {
        return Ok(axum::Json(browser_bridge::text_result(
            json!({"error":"No authorized user turn owns this browser"}),
            true,
        )));
    };
    if lease.check().is_err() {
        return Ok(axum::Json(browser_bridge::takeover_result(false)));
    }
    if let Err(error) = browser_bridge::validate(&request.name, &request.arguments) {
        return Ok(axum::Json(browser_bridge::text_result(
            json!({"error":error}),
            true,
        )));
    }
    let key = (request.agent.clone(), request.id.clone());
    {
        let mut calls = state.calls.lock().unwrap();
        let mut pending = state.cancelled_calls.lock().unwrap();
        pending.retain(|(_, at)| at.elapsed() < Duration::from_secs(60));
        if let Some(index) = pending.iter().position(|(cancelled, _)| cancelled == &key) {
            pending.remove(index);
            return Ok(axum::Json(browser_bridge::takeover_result(false)));
        }
        if calls.values().filter(|gate| Arc::ptr_eq(gate, &lease.gate)).count() >= 16 || calls.contains_key(&key) {
            return Err(axum::http::StatusCode::CONFLICT);
        }
        calls.insert(key.clone(), lease.gate.clone());
    }
    let _scope = CallScope {
        app: app.clone(),
        key,
        gate: lease.gate.clone(),
    };
    let value = async {
        let tab = if let Ok(tab) = tab(&app, &lease.session) {
            tab
        } else {
            ensure_tab(&app, &lease.session, "main").await?
        };
        *tab.used.lock().unwrap() = Instant::now();
        let first_call = !lease.presented.swap(true, Ordering::AcqRel);
        if !tab.visible.load(Ordering::Acquire) && first_call {
            status(&app, &tab, "working", "正在准备网页");
            let _ = app.emit_to(
                tab.window.clone(),
                "browser-show",
                json!({"sessionId":lease.session}),
            );
            let start = Instant::now();
            while !tab.visible.load(Ordering::Acquire) && start.elapsed() < Duration::from_secs(5) {
                lease.check()?;
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        }
        execute(&app, &tab, &lease, &request.name, request.arguments).await
    }
    .await
    .unwrap_or_else(|error: String| {
        if error=="user_takeover"{browser_bridge::takeover_result(false)}
        else{browser_bridge::text_result(json!({"error":error}), true)}
    });
    Ok(axum::Json(value))
}
pub fn install(app: &AppHandle) -> Result<(), String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let address = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .to_string();
    *app.state::<Browser>().endpoint.lock().unwrap() =
        Some((address, uuid::Uuid::new_v4().to_string()));
    let router = axum::Router::new()
        .route("/call", axum::routing::post(request))
        .route("/cancel", axum::routing::post(cancel_request))
        .layer(axum::extract::DefaultBodyLimit::max(256 * 1024))
        .with_state(app.clone());
    *app.state::<Browser>().listener.lock().unwrap() =
        Some(tauri::async_runtime::spawn(async move {
            if let Ok(listener) = tokio::net::TcpListener::from_std(listener) {
                let _ = axum::serve(listener, router).await;
            }
        }));
    crate::browser_native::install(app)?;
    Ok(())
}
pub fn guard<F: Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static>(
    handler: F,
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    move |invoke| {
        if invoke.message.webview_ref().label().starts_with("browser-") {
            invoke.resolver.reject("网站不能访问 SuperCode 应用接口");
            true
        } else {
            handler(invoke)
        }
    }
}
pub async fn probe(app: &AppHandle, cancel: &Arc<AtomicBool>) -> Result<(usize, String), String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let router=axum::Router::new().route("/",axum::routing::get(||async{axum::response::Html("<!doctype html><meta charset=utf-8><title>SuperCode Browser Test</title><style>body{font:16px sans-serif;padding:24px}button,input{display:block;margin:18px;padding:12px}body{min-height:1000px}</style><button onclick=\"document.querySelector('p').textContent='CLICKED';document.querySelector('output').textContent=Number(document.querySelector('output').textContent)+1\">SC Browser Button</button><input aria-label=\"SC Browser Input\"><p>WAITING</p><output>0</output>")}));
    struct Fixture(tokio::task::JoinHandle<()>);
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _fixture = Fixture(tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    }));
    let session = format!("probe-{}", uuid::Uuid::new_v4().simple());
    let tab = ensure_tab(app, &session, "main").await?;
    app.get_webview(&tab.label)
        .ok_or("测试浏览器未创建")?
        .set_bounds(tauri::Rect {
            position: tauri::LogicalPosition::new(10000., 0.).into(),
            size: tauri::LogicalSize::new(640., 480.).into(),
        })
        .map_err(|e| e.to_string())?;
    app.get_webview(&tab.label)
        .ok_or("测试浏览器未创建")?
        .show()
        .map_err(|e| e.to_string())?;
    *tab.rect.lock().unwrap() = (10000., 0., 640., 480.);
    tab.visible.store(true, Ordering::Release);
    let lease = Lease {
        session: session.clone(),
        agent: "probe".into(),
        turn: None,
        gate: Arc::new(AtomicBool::new(true)),
        presented: Arc::new(AtomicBool::new(true)),
    };
    let marker = format!("中文-SC-{}", uuid::Uuid::new_v4().simple());
    let verify = async {
        execute(
            app,
            &tab,
            &lease,
            "browser_navigate",
            json!({"url":format!("http://{address}/")}),
        )
        .await?;
        let value = snapshot(app, &tab, &lease).await?;
        if !value["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["type"] == "image")
        {
            return Err(format!(
                "浏览器截图接口不可用: {}",
                value["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|v| v["type"] == "text")
                    .skip(1)
                    .map(|v| v["text"].as_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        let read = |value: &Value| -> Result<Value, String> {
            serde_json::from_str(value["content"][0]["text"].as_str().ok_or("缺少页面快照")?)
                .map_err(|e| e.to_string())
        };
        let first = read(&value)?;
        let reference = |value: &Value, name: &str| -> Result<String, String> {
            value["elements"]
                .as_array()
                .ok_or("缺少页面控件")?
                .iter()
                .find(|e| e["name"] == name)
                .and_then(|e| e["ref"].as_str())
                .map(str::to_owned)
                .ok_or_else(|| format!("测试网页控件未找到: {name}"))
        };
        let clicked = execute(
            app,
            &tab,
            &lease,
            "browser_click",
            json!({"snapshotId":first["snapshotId"],"ref":reference(&first,"SC Browser Button")?}),
        )
        .await?;
        let clicked = read(&clicked)?;
        if !clicked["text"]
            .as_str()
            .is_some_and(|s| s.contains("CLICKED") && s.contains('1'))
        {
            return Err("真实浏览器点击未生效".into());
        }
        let filled=execute(app,&tab,&lease,"browser_fill",json!({"snapshotId":clicked["snapshotId"],"ref":reference(&clicked,"SC Browser Input")?,"text":marker})).await?;
        let filled = read(&filled)?;
        let actual = evaluate(
            app,
            &tab,
            "document.querySelector('input').value".into(),
            None,
            Some(&lease),
        )
        .await?;
        if actual != marker {
            return Err("真实浏览器输入未生效".into());
        }
        let old = execute(
            app,
            &tab,
            &lease,
            "browser_click",
            json!({"snapshotId":first["snapshotId"],"ref":"e1"}),
        )
        .await?;
        if old["isError"] != true {
            return Err("旧页面快照未被阻止".into());
        }
        execute(
            app,
            &tab,
            &lease,
            "browser_scroll",
            json!({"snapshotId":filled["snapshotId"],"deltaY":220}),
        )
        .await?;
        let scroll = evaluate(app, &tab, "scrollY".into(), None, Some(&lease)).await?;
        if scroll.as_f64().unwrap_or(0.) < 1. {
            return Err("真实浏览器滚动未生效".into());
        }
        lease.gate.store(false, Ordering::Release);
        if execute(
            app,
            &tab,
            &lease,
            "browser_navigate",
            json!({"url":"https://example.com"}),
        )
        .await
        .is_ok()
        {
            return Err("停止后浏览器仍可执行操作".into());
        }
        Ok((
            browser_bridge::tools().len(),
            "已验证真实网页加载、截图、点击、中文输入、滚动与停止门禁".into(),
        ))
    };
    let result = tokio::select! {value=verify=>value,_=async{while !cancel.load(Ordering::Acquire){tokio::time::sleep(Duration::from_millis(20)).await;}lease.gate.store(false,Ordering::Release);}=>Err("已取消浏览器测试".into())};
    tab.visible.store(false, Ordering::Release);
    if let Some(view) = app.get_webview(&tab.label) {
        let _ = view.close();
    }
    app.state::<Browser>().tabs.lock().unwrap().remove(&session);
    result
}
#[path = "browser_smoke.rs"]
mod smoke_checks;
pub async fn smoke(app: AppHandle) {
    let result = async {
        let probe = probe(&app, &Arc::new(AtomicBool::new(false))).await?;
        let transport=smoke_checks::verify(&app).await?;
        let args:Vec<String>=std::env::args().collect();
        #[cfg(windows)]
        let model=if let Some(agent)=args.iter().position(|arg|arg=="--model-browser").and_then(|i|args.get(i+1)){smoke_checks::model(&app,agent).await?}else{json!({"modelTurn":false})};
        #[cfg(not(windows))]
        let model=json!({"modelTurn":false});
        Ok::<Value, String>(
            json!({"nativeWebview":true,"tools":probe.0,"summary":probe.1,"transport":transport,"model":model}),
        )
    }
    .await;
    let report = match result {
        Ok(value) => json!({"ok":true,"result":value}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    let root = std::env::current_dir()
        .unwrap_or_default()
        .join(".supercode");
    let _ = std::fs::create_dir_all(&root);
    let _ = std::fs::write(
        root.join("browser-smoke-report.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    );
    println!("{report}");
    app.exit(if report["ok"] == true { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_lease_is_selected_by_session_even_with_the_same_agent() {
        let make = |id: &str, agent: &str| Lease { session:id.into(), agent:agent.into(), turn:None, gate:Arc::new(AtomicBool::new(true)), presented:Arc::new(AtomicBool::new(false)) };
        let leases = [make("a", "codex"), make("b", "codex"), make("c", "claude")].into_iter().map(|l| (l.session.clone(), l)).collect::<HashMap<_, _>>();
        assert_eq!(select_lease(&leases, "codex", Some("a")).unwrap().session, "a");
        assert_eq!(select_lease(&leases, "codex", Some("b")).unwrap().session, "b");
        assert!(select_lease(&leases, "codex", None).is_none());
        assert!(select_lease(&leases, "codex", Some("c")).is_none());
        leases["a"].gate.store(false, Ordering::Release);
        assert!(select_lease(&leases, "codex", Some("a")).unwrap().check().is_err());
        assert!(select_lease(&leases, "codex", Some("b")).unwrap().check().is_ok());
        assert!(select_lease(&leases, "claude", Some("c")).unwrap().check().is_ok());
    }
    #[test]
    fn browser_takeover_revokes_only_its_owner_until_new_authorization() {
        let browser = Browser::default();
        let make = |session: &str| Lease {
            session: session.into(), agent: "codex".into(), turn: Some("turn".into()),
            gate: Arc::new(AtomicBool::new(true)), presented: Arc::new(AtomicBool::new(true)),
        };
        let owner = make("owned");
        let other = make("other");
        browser.leases.lock().unwrap().extend([
            (owner.session.clone(), owner.clone()), (other.session.clone(), other.clone()),
        ]);
        assert!(revoke_control(&browser, "owned"));
        assert_eq!(owner.check().unwrap_err(), "user_takeover");
        assert!(other.check().is_ok());
        assert!(!revoke_control(&browser, "owned"));
        assert!(!revoke_control(&browser, "unknown"));
        assert!(select_lease(&browser.leases.lock().unwrap(), "codex", Some("owned"))
            .unwrap().check().is_err());
    }
    #[tokio::test]
    async fn cancelling_browser_tool_keeps_parent_session_and_turn_running() {
        let store = crate::storage::Store::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(std::path::Path::new("D:/browser-cancellation-fixture")).unwrap();
        let session = store.create_session(&project.id, None).unwrap();
        store.bind_native(&session.id, "native-fixture").unwrap();
        store.set_status(&session.id, "running", Some("parent-turn")).unwrap();
        let browser = Browser::default();
        let lease = Lease {
            session: session.id.clone(), agent: "codex".into(), turn: Some("parent-turn".into()),
            gate: Arc::new(AtomicBool::new(true)), presented: Arc::new(AtomicBool::new(true)),
        };
        browser.leases.lock().unwrap().insert(session.id.clone(), lease.clone());
        let tab = test_tab(&session.id, true);
        let _layout = tab.operation.lock().await;
        let takeover = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert!(revoke_control(&browser, &session.id));
        };
        let (result, _) = tokio::join!(operation(&tab, &lease), takeover);
        assert_eq!(result.unwrap_err(), "user_takeover");
        let parent = store.session(&session.id).unwrap();
        assert_eq!(parent.status, "running");
        assert_eq!(parent.turn_id.as_deref(), Some("parent-turn"));
        assert_eq!(parent.native_id.as_deref(), Some("native-fixture"));
    }
    fn test_tab(session: &str, visible: bool) -> Arc<Tab> {
        Arc::new(Tab {
            session: session.into(),
            label: session.into(),
            window: "main".into(),
            visible: Arc::new(AtomicBool::new(visible)),
            mount: Mutex::new(String::new()),
            focused: AtomicBool::new(false),
            recovery_attempts: Mutex::new(Vec::new()),
            hwnd: 0,
            scale: AtomicU64::new(1f64.to_bits()),
            rect: Mutex::new((0., 0., 500., 400.)),
            used: Mutex::new(Instant::now()),
            layout_revision: AtomicU64::new(0),
            revision: AtomicU64::new(1),
            observation: Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
            status: Mutex::new("idle".into()),
            action: Mutex::new(String::new()),
            location: Mutex::new(("about:blank".into(), "新标签页".into())),
        })
    }
    #[test]
    fn retained_browser_cache_keeps_visible_active_and_busy_pages() {
        let active = test_tab("active", false);
        let visible = test_tab("visible", true);
        let busy = test_tab("busy", false);
        let older = test_tab("older", false);
        let newer = test_tab("newer", false);
        *older.used.lock().unwrap() = Instant::now() - Duration::from_secs(60);
        let guard = busy.operation.try_lock().unwrap();
        let mut tabs = [&active, &visible, &busy, &older, &newer]
            .into_iter()
            .map(|page| (page.session.clone(), page.clone()))
            .collect::<HashMap<_, _>>();
        let lease = Lease {
            session: "active".into(),
            agent: "codex".into(),
            turn: None,
            gate: Arc::new(AtomicBool::new(true)),
            presented: Arc::new(AtomicBool::new(true)),
        };
        assert_eq!(
            inactive_hidden_tab(&tabs, std::slice::from_ref(&lease)),
            Some("older".into())
        );
        tabs.remove("older");
        assert_eq!(
            inactive_hidden_tab(&tabs, std::slice::from_ref(&lease)),
            Some("newer".into())
        );
        tabs.remove("newer");
        assert_eq!(inactive_hidden_tab(&tabs, std::slice::from_ref(&lease)), None);
        drop(guard);
        assert_eq!(
            inactive_hidden_tab(&tabs, std::slice::from_ref(&lease)),
            Some("busy".into())
        );
        tabs.remove("busy");
        lease.gate.store(false, Ordering::Release);
        assert_eq!(
            inactive_hidden_tab(&tabs, std::slice::from_ref(&lease)),
            Some("active".into())
        );
    }
    #[test]
    fn browser_urls_reject_privileged_schemes_and_embedded_credentials() {
        for value in [
            "file:///C:/secret",
            "javascript:alert(1)",
            "tauri://localhost",
            "https://user:pass@example.com",
        ] {
            assert!(url(value).is_err());
        }
        assert!(url("http://127.0.0.1:1234/test").is_ok());
        assert!(url("https://example.com").is_ok());
    }
    #[test]
    fn snapshots_are_invalidated_on_layout_or_navigation() {
        let tab = Tab {
            session: "s".into(),
            label: "b".into(),
            window: "main".into(),
            visible: Arc::new(AtomicBool::new(true)),
            mount: Mutex::new(String::new()),
            focused: AtomicBool::new(false),
            recovery_attempts: Mutex::new(Vec::new()),
            hwnd: 0,
            scale: AtomicU64::new(1f64.to_bits()),
            rect: Mutex::new((0., 0., 500., 400.)),
            used: Mutex::new(Instant::now()),
            layout_revision: AtomicU64::new(0),
            revision: AtomicU64::new(1),
            observation: Mutex::new(Some(Observation {
                id: "snap".into(),
                context: 1,
                revision: 1,
                at: Instant::now(),
            })),
            operation: tokio::sync::Mutex::new(()),
            status: Mutex::new("idle".into()),
            action: Mutex::new(String::new()),
            location: Mutex::new(("about:blank".into(), "新标签页".into())),
        };
        assert!(observation(&tab, &json!("snap")).is_ok());
        tab.revision.fetch_add(1, Ordering::AcqRel);
        assert!(observation(&tab, &json!("snap")).is_err());
    }
    #[test]
    fn target_stability_compares_current_geometry_and_replacement_generation() {
        let point = json!({"status":"ready","x":40.,"y":90.,"generation":0,"rect":{"x":20.,"y":70.,"width":40.,"height":40.}});
        assert!(same_point(&point, &point));
        let mut moved = point.clone();
        moved["rect"]["x"] = json!(21.5);
        assert!(!same_point(&point, &moved));
        let mut replaced = point.clone();
        replaced["generation"] = json!(1);
        assert!(!same_point(&point, &replaced));
        let mut obscured = point.clone();
        obscured["status"] = json!("blocked");
        assert!(!same_point(&point, &obscured));
        assert!(!same_point(&point, &json!({"status":"ready"})));
    }
    #[tokio::test]
    async fn tool_waits_for_short_layout_updates_but_escape_cancels_waiting() {
        let tab = test_tab("layout", true);
        let lease = Lease {
            session: "layout".into(),
            agent: "codex".into(),
            turn: None,
            gate: Arc::new(AtomicBool::new(true)),
            presented: Arc::new(AtomicBool::new(true)),
        };
        let layout = tab.operation.lock().await;
        let release = async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            drop(layout);
        };
        let (acquired, _) = tokio::join!(operation(&tab, &lease), release);
        drop(acquired.unwrap());
        let _layout = tab.operation.lock().await;
        let escape = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            lease.gate.store(false, Ordering::Release);
        };
        let (cancelled, _) = tokio::join!(operation(&tab, &lease), escape);
        assert_eq!(cancelled.unwrap_err(), "user_takeover");
        lease.gate.store(true, Ordering::Release);
        let start = Instant::now();
        assert!(operation(&tab, &lease)
            .await
            .unwrap_err()
            .contains("另一个浏览器操作"));
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn old_lease_cannot_be_reenabled_by_new_turn() {
        let old = Lease {
            session: "s".into(),
            agent: "claude".into(),
            turn: None,
            gate: Arc::new(AtomicBool::new(true)),
            presented: Arc::new(AtomicBool::new(false)),
        };
        old.gate.store(false, Ordering::Release);
        let new = Lease {
            gate: Arc::new(AtomicBool::new(true)),
            ..old.clone()
        };
        assert!(old.check().is_err());
        assert!(new.check().is_ok());
    }
}
