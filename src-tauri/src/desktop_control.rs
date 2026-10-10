//! Desktop handoff is driven by actual managed MCP requests, never model text.
use crate::{client_features::ToolServer, desktop_glow, AppState};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[derive(Clone)]
struct Endpoint {
    address: String,
    token: String,
    lease: String,
}
#[derive(Default)]
struct Connection {
    agent: String,
    session: Option<String>,
    revoked: bool,
    calls: HashSet<String>,
}
#[derive(Default)]
struct Activity {
    connections: HashMap<String, Connection>,
    engaged: HashSet<String>,
    blocked: HashSet<String>,
    halted: HashSet<String>,
    suspended: bool,
    epoch: u64,
    session: Option<String>,
    agent: Option<String>,
    turn: Option<String>,
}
impl Activity {
    fn start(&mut self, connection: &str, id: &str) -> bool {
        let Some(row) = self.connections.get_mut(connection) else {
            return false;
        };
        if row.revoked
            || row.session != self.session
            || self.agent.as_ref().is_some_and(|agent| agent != &row.agent)
            || self.blocked.contains(&row.agent)
            || row.calls.len() >= 128
            || self.engaged.iter().any(|owner| owner != &row.agent)
        {
            return false;
        }
        row.calls.insert(id.into());
        if self.engaged.is_empty() {
            self.epoch = desktop_glow::input_epoch();
        }
        self.engaged.insert(row.agent.clone());
        self.suspended = false;
        true
    }
    fn end(&mut self, connection: &str, id: &str) {
        if let Some(row) = self.connections.get_mut(connection) {
            row.calls.remove(id);
        }
    }
    fn visible(&self) -> bool {
        // Keep the cue during the model's observation/action loop, including thinking.
        !self.suspended && !self.engaged.is_empty()
    }
    fn finish(&mut self, agent: &str) {
        self.engaged.remove(agent);
        self.blocked.insert(agent.into());
        for row in self
            .connections
            .values_mut()
            .filter(|row| row.agent == agent)
        {
            row.calls.clear();
        }
    }
    fn authorize_turn(&mut self, agent: &str) -> bool {
        self.blocked.remove(agent);
        self.halted.remove(agent)
    }
    fn revoke(&mut self, agent: &str) {
        self.finish(agent);
        self.halted.insert(agent.into());
        for row in self
            .connections
            .values_mut()
            .filter(|row| row.agent == agent)
        {
            row.revoked = true;
        }
    }
    fn prepare_session(&mut self, agent: &str, session: &str) -> bool {
        let changed = self.session.as_deref().is_some_and(|id| id != session);
        self.session = Some(session.into());
        self.agent = Some(agent.into());
        self.turn = None;
        self.engaged.clear();
        self.suspended = false;
        self.authorize_turn(agent) || changed
    }
    fn owns_event(&self, session: &str, turn: Option<&str>) -> bool {
        self.session.as_deref().is_none_or(|id| id == session)
            && turn.is_none_or(|turn| self.turn.as_deref() == Some(turn))
    }
    fn disconnect(&mut self, connection: &str) {
        if let Some(row) = self.connections.remove(connection) {
            if !row.revoked
                && row.session == self.session
                && !self
                    .connections
                    .values()
                    .any(|c| c.agent == row.agent && c.session == self.session && !c.revoked)
            {
                self.engaged.remove(&row.agent);
            }
        }
    }
}
#[derive(Default)]
pub struct DesktopControl {
    endpoint: Mutex<Option<Endpoint>>,
    activity: Mutex<Activity>,
    yielded: Mutex<Vec<String>>,
    glow: Mutex<Option<desktop_glow::Glow>>,
    listener: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    cancellations: Mutex<HashMap<String, (String, bool, tokio::sync::watch::Sender<bool>)>>,
    focused: Mutex<Option<String>>,
    gate: Mutex<Option<Arc<crate::computer_gate::Gate>>>,
    cue_status: Mutex<desktop_glow::Status>,
    diagnostics: Mutex<()>,
    manual_takeover: AtomicBool,
}

pub fn install(app: &AppHandle) -> Result<(), String> {
    if !cfg!(windows) {
        return Ok(());
    }
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let endpoint = Endpoint {
        address: listener
            .local_addr()
            .map_err(|e| e.to_string())?
            .to_string(),
        token: uuid::Uuid::new_v4().to_string(),
        lease: uuid::Uuid::new_v4().to_string(),
    };
    let gate =
        crate::computer_gate::Gate::create(&crate::computer_gate::event_name(&endpoint.lease))?;
    *app.state::<DesktopControl>().gate.lock().unwrap() = Some(Arc::new(gate));
    *app.state::<DesktopControl>()
        .endpoint
        .lock()
        .map_err(|e| e.to_string())? = Some(endpoint.clone());
    let handle = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
            return;
        };
        while let Ok((socket, _)) = listener.accept().await {
            let app = handle.clone();
            tauri::async_runtime::spawn(async move {
                connection(app, socket).await;
            });
        }
    });
    // Tauri's join handle wraps Tokio's handle; abort it explicitly on app exit.
    *app.state::<DesktopControl>()
        .listener
        .lock()
        .map_err(|e| e.to_string())? = Some(task);
    Ok(())
}

/// Only exports are wrapped. Saved configuration and installation tests stay unchanged.
pub fn wrap(app: &AppHandle, mut server: ToolServer, agent: &str) -> Result<ToolServer, String> {
    server = crate::browser::wrap(app, server, agent)?;
    if server.enabled
        && server.id == "computer"
        && server.kind == "computer"
        && server.url.is_none()
        && cfg!(windows)
    {
        let endpoint = app
            .state::<DesktopControl>()
            .endpoint
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or("电脑控制与停止机制未连接，已阻止操作")?;
        {
            let mut args = vec![
                "--computer-mcp-bridge".into(),
                endpoint.address,
                endpoint.token,
                agent.into(),
                server.command,
            ];
            args.append(&mut server.args);
            server.command = std::env::current_exe()
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .into_owned();
            server.args = args;
        }
    }
    Ok(server)
}

async fn line(reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> Option<Value> {
    let mut bytes = Vec::new();
    let n = reader.take(2049).read_until(b'\n', &mut bytes).await.ok()?;
    if n == 0 || n > 2048 || !bytes.ends_with(b"\n") {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}
async fn connection(app: AppHandle, socket: tokio::net::TcpStream) {
    let (read, mut write) = socket.into_split();
    let mut read = BufReader::new(read);
    let hello = tokio::time::timeout(Duration::from_secs(3), line(&mut read))
        .await
        .ok()
        .flatten();
    let Some(hello) = hello else {
        return;
    };
    let Some(agent) = hello["agent"]
        .as_str()
        .filter(|agent| matches!(*agent, "codex" | "claude" | "opencode" | "pi"))
    else {
        return;
    };
    let state = app.state::<DesktopControl>();
    let token = state.endpoint.lock().unwrap().as_ref().map(|e| {
        if hello.get("cursor").is_some() {
            e.lease.clone()
        } else {
            e.token.clone()
        }
    });
    if token.is_none() || hello["token"].as_str() != token.as_deref() {
        return;
    }
    if let Some(cursor) = hello.get("cursor") {
        let owned = state.activity.lock().unwrap().engaged.contains(agent);
        let live = state
            .gate
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|g| !g.stopped());
        if owned && live {
            if let (Some(x), Some(y)) = (cursor["x"].as_i64(), cursor["y"].as_i64()) {
                if x.abs() <= 100_000 && y.abs() <= 100_000 {
                    desktop_glow::pointer(x as i32, y as i32, cursor["click"] == true);
                    let _ = write.write_all(b"ok\n").await;
                }
            }
        }
        return;
    }
    let builtin = hello["builtin"] == true;
    let key = uuid::Uuid::new_v4().to_string();
    let (cancel, mut cancelled) = tokio::sync::watch::channel(false);
    app.state::<DesktopControl>()
        .cancellations
        .lock()
        .unwrap()
        .insert(key.clone(), (agent.into(), builtin, cancel));
    {
        let mut activity = state.activity.lock().unwrap();
        let session = activity.session.clone();
        activity.connections.insert(
            key.clone(),
            Connection {
                agent: agent.into(),
                session,
                ..Default::default()
            },
        );
    }
    if write.write_all(b"ok\n").await.is_ok() {
        loop {
            let message = tokio::select! { v=line(&mut read)=>v, _=cancelled.changed()=>None };
            let Some(message) = message else {
                break;
            };
            let Some(id) = message["id"]
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= 256)
            else {
                break;
            };
            let Some(active) = message["active"].as_bool() else {
                break;
            };
            let current = state
                .endpoint
                .lock()
                .unwrap()
                .as_ref()
                .map(|e| crate::computer_gate::event_name(&e.lease));
            // A result from a revoked/previous turn cannot hide or rearm a newer cue.
            if !active
                && message["gate"]
                    .as_str()
                    .is_some_and(|g| Some(g) != current.as_deref())
            {
                if write.write_all(b"{\"ok\":true}\n").await.is_err() {
                    break;
                }
                continue;
            }
            if !active {
                state.activity.lock().unwrap().end(&key, id);
                // The sticky cue is unchanged by a completed call. A native
                // cancellation result must not wait behind a busy WebView.
                let answer = serde_json::json!({"ok":true,"gate":current}).to_string() + "\n";
                if write.write_all(answer.as_bytes()).await.is_err() {
                    break;
                }
                continue;
            }
            // Acknowledge only after minimizing, before forwarding tools/call.
            let app_ui = app.clone();
            let key_ui = key.clone();
            let id = id.to_owned();
            let expected_gate = current.clone();
            let (tx, rx) = tokio::sync::oneshot::channel();
            if app
                .run_on_main_thread(move || {
                    let current_gate = app_ui
                        .state::<DesktopControl>()
                        .endpoint
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|e| crate::computer_gate::event_name(&e.lease));
                    let result =
                        current_gate == expected_gate && set_call(&app_ui, &key_ui, &id, active);
                    let _ = tx.send(result);
                })
                .is_err()
            {
                break;
            }
            let ok = tokio::time::timeout(Duration::from_secs(5), rx)
                .await
                .is_ok_and(|r| r == Ok(true));
            let answer = serde_json::json!({"ok":ok,"gate":if ok { current } else { None }})
                .to_string()
                + "\n";
            if write.write_all(answer.as_bytes()).await.is_err() {
                break;
            }
        }
    }
    let _ = app.clone().run_on_main_thread(move || {
        app.state::<DesktopControl>()
            .cancellations
            .lock()
            .unwrap()
            .remove(&key);
        app.state::<DesktopControl>()
            .activity
            .lock()
            .unwrap()
            .disconnect(&key);
        let _ = refresh(&app);
        if app
            .state::<DesktopControl>()
            .activity
            .lock()
            .unwrap()
            .engaged
            .is_empty()
        {
            restore(&app);
        }
    });
}

fn set_call(app: &AppHandle, connection: &str, id: &str, active: bool) -> bool {
    let state = app.state::<DesktopControl>();
    let ok = {
        let mut activity = state.activity.lock().unwrap();
        if active {
            activity.start(connection, id)
        } else {
            activity.end(connection, id);
            true
        }
    };
    if !ok {
        return false;
    }
    if active {
        yield_desktop(app);
    }
    if let Err(error) = refresh(app) {
        diagnostic(app, "cue_failed", Some(&error));
        let _ = app.emit("runtime-log", format!("电脑操作已阻止：{error}"));
        stop(app);
        return false;
    }
    true
}
fn yield_desktop(app: &AppHandle) {
    let state = app.state::<DesktopControl>();
    state.manual_takeover.store(false, Ordering::Release);
    let mut yielded = state.yielded.lock().unwrap();
    if !yielded.is_empty() {
        return;
    }
    // Only move SuperCode if it is in front. Never focus an arbitrary other app.
    let focused = app
        .windows()
        .values()
        .find(|w| w.is_focused().unwrap_or(false))
        .map(|w| w.label().to_owned());
    if focused.is_none() {
        return;
    }
    *state.focused.lock().unwrap() = focused;
    for (label, window) in app.windows() {
        if label == crate::tray_popup::LABEL {
            continue;
        }
        if window.is_visible().unwrap_or(false)
            && !window.is_minimized().unwrap_or(false)
            && window.minimize().is_ok()
        {
            yielded.push(label);
        }
    }
}
fn restore(app: &AppHandle) {
    let labels = std::mem::take(&mut *app.state::<DesktopControl>().yielded.lock().unwrap());
    if app
        .state::<DesktopControl>()
        .manual_takeover
        .load(Ordering::Acquire)
    {
        app.state::<DesktopControl>().focused.lock().unwrap().take();
        return;
    }
    for label in &labels {
        if let Some(window) = app.get_window(label) {
            let _ = window.unminimize();
        }
    }
    let focused = app.state::<DesktopControl>().focused.lock().unwrap().take();
    if let Some(label) = focused
        .as_ref()
        .filter(|label| labels.contains(label))
        .or(labels.first())
    {
        if let Some(window) = app.get_window(label) {
            let _ = window.set_focus();
        }
    }
}
fn refresh(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<DesktopControl>();
    let (active, escape, epoch) = {
        let activity = state.activity.lock().unwrap();
        (
            activity.visible(),
            !activity.suspended && !activity.engaged.is_empty(),
            activity.epoch,
        )
    };
    let mut glow = state.glow.lock().unwrap();
    if active && glow.is_none() {
        let gate = state
            .gate
            .lock()
            .unwrap()
            .clone()
            .ok_or("电脑停止机制未连接")?;
        *glow = Some(desktop_glow::Glow::new(app.clone(), gate));
    }
    if let Some(glow) = glow.as_ref() {
        let status = glow.set(active, escape, epoch)?;
        let mut saved = state.cue_status.lock().unwrap();
        if active && saved.edges == 0 {
            diagnostic(app, "cue_ready", None);
        }
        *saved = status;
    }
    Ok(())
}

fn diagnostic(app: &AppHandle, event: &str, error: Option<&str>) {
    use std::io::Write;
    let state = app.state::<DesktopControl>();
    let _lock = state.diagnostics.lock().unwrap();
    let path = app
        .state::<AppState>()
        .data_dir
        .join("desktop-control.jsonl");
    let truncate = std::fs::metadata(&path).is_ok_and(|m| m.len() > 32 * 1024);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!truncate)
        .truncate(truncate)
        .open(path)
    {
        let (session, agent) = {
            let activity = state.activity.lock().unwrap();
            (activity.session.clone(), activity.agent.clone())
        };
        let lease = state
            .endpoint
            .lock()
            .unwrap()
            .as_ref()
            .map(|e| e.lease.clone());
        let entry = serde_json::json!({"time":chrono::Utc::now().to_rfc3339(),"event":event,"sessionId":session,"agent":agent,"leaseId":lease,"reason":error.map(|e|e.chars().take(512).collect::<String>())});
        let _ = writeln!(file, "{entry}");
    }
}
pub(crate) fn cue_ready(app: &AppHandle) -> bool {
    let state = app.state::<DesktopControl>();
    let ready = *state.cue_status.lock().unwrap();
    ready.edges >= 4
        && ready.pointer
        && ready.keyboard
        && ready.mouse
        && state
            .gate
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|g| !g.stopped())
}

/// Only an accepted new user message can resume control. A native turn/started
/// notification (including an automatic retry) is not authorization.
pub(crate) fn authorize_session(
    app: &AppHandle,
    agent: &str,
    session: &str,
) -> Result<bool, String> {
    let state = app.state::<DesktopControl>();
    // A fresh kernel gate is never shared with workers from an earlier user turn.
    let token = uuid::Uuid::new_v4().to_string();
    let gate = Arc::new(crate::computer_gate::Gate::create(
        &crate::computer_gate::event_name(&token),
    )?);
    if let Some(old) = state.gate.lock().unwrap().as_ref() {
        old.stop();
    }
    let old_glow = state.glow.lock().unwrap().take();
    drop(old_glow);
    *state.gate.lock().unwrap() = Some(gate);
    *state.cue_status.lock().unwrap() = desktop_glow::Status::default();
    let reconnect = state
        .activity
        .lock()
        .unwrap()
        .prepare_session(agent, session);
    if let Some(endpoint) = state.endpoint.lock().unwrap().as_mut() {
        endpoint.lease = token;
        if reconnect {
            endpoint.token = uuid::Uuid::new_v4().to_string();
        }
    }
    Ok(reconnect)
}

/// Approval/questions retain the native Agent request and bring its chat back.
pub fn observe(app: &AppHandle, event: &Value) {
    let method = event["method"].as_str().unwrap_or("");
    let completed = method == "turn/completed";
    let attention = event.get("id").is_some() && method != "serverRequest/resolved";
    let started = method == "turn/started";
    if !completed && !attention && !started {
        return;
    }
    let Some(native) = event["params"]["threadId"].as_str() else {
        return;
    };
    let Ok(session) = app.state::<AppState>().store.for_native(native) else {
        return;
    };
    let state = app.state::<DesktopControl>();
    {
        let mut activity = state.activity.lock().unwrap();
        if activity
            .session
            .as_deref()
            .is_some_and(|id| id != session.id)
        {
            return;
        }
        if started {
            if activity.turn.is_none() {
                activity.turn = event["params"]["turn"]["id"].as_str().map(str::to_owned);
            }
            return;
        }
        let turn = event["params"]["turn"]["id"]
            .as_str()
            .or(event["params"]["turnId"].as_str());
        if !activity.owns_event(&session.id, turn) {
            return;
        }
    }
    let (bring_back, close_gate) = {
        let mut activity = state.activity.lock().unwrap();
        let owned = activity.engaged.contains(&session.agent);
        if completed {
            activity.finish(&session.agent);
        }
        if attention && owned {
            activity.suspended = true;
        }
        (
            (completed && activity.engaged.is_empty()) || (attention && owned),
            owned && (attention || completed),
        )
    };
    if close_gate {
        if let Some(gate) = state.gate.lock().unwrap().as_ref() {
            gate.stop();
        }
    }
    let app_ui = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(error) = refresh(&app_ui) {
            diagnostic(&app_ui, "cue_failed", Some(&error));
        }
        if bring_back {
            restore(&app_ui);
        }
    });
}

pub fn stop(app: &AppHandle) {
    takeover_reason(app, true, "chat_stop");
}

/// Manual input takes priority. Mouse/ordinary keys keep the user's foreground;
/// Esc and the chat's Stop button may bring the chat back.
pub(crate) fn takeover_reason(app: &AppHandle, bring_back: bool, reason: &str) {
    let state = app.state::<DesktopControl>();
    state.manual_takeover.store(!bring_back, Ordering::Release);
    if let Some(gate) = state.gate.lock().unwrap().as_ref() {
        gate.stop();
    }
    let agents = {
        let mut activity = state.activity.lock().unwrap();
        let agents = activity.engaged.clone();
        for agent in &agents {
            activity.revoke(agent);
        }
        agents
    };
    if agents.is_empty() {
        return;
    }
    // Native workers observe the gate and return a takeover result. Legacy servers
    // still need their process tree killed because they have no cancellation contract.
    for (agent, builtin, cancel) in state.cancellations.lock().unwrap().values() {
        if agents.contains(agent) && !builtin {
            let _ = cancel.send(true);
        }
    }
    diagnostic(app, "control_stopped", Some(reason));
    let owner = state.activity.lock().unwrap().session.clone();
    let sessions = app.state::<AppState>().store.sessions().unwrap_or_default();
    for session in sessions.into_iter().filter(|s| {
        agents.contains(&s.agent)
            && owner.as_ref().is_none_or(|id| id == &s.id)
            && matches!(s.status.as_str(), "starting" | "running" | "waiting")
    }) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = crate::commands::interrupt_turn(session.id, app).await {
                eprintln!("停止电脑操作失败：{error}");
            }
        });
    }
    let app_ui = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = refresh(&app_ui);
        if bring_back {
            restore(&app_ui);
        } else {
            app_ui
                .state::<DesktopControl>()
                .yielded
                .lock()
                .unwrap()
                .clear();
            app_ui
                .state::<DesktopControl>()
                .focused
                .lock()
                .unwrap()
                .take();
        }
    });
}
pub fn shutdown(app: &AppHandle) {
    let state = app.state::<DesktopControl>();
    if let Some(gate) = state.gate.lock().unwrap().as_ref() {
        gate.stop();
    }
    if let Some(task) = state.listener.lock().unwrap().take() {
        task.abort();
    }
    if let Some(glow) = state.glow.lock().unwrap().take() {
        drop(glow);
    };
}
pub(crate) fn active(app: &AppHandle) -> bool {
    app.state::<DesktopControl>()
        .activity
        .lock()
        .unwrap()
        .visible()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn activity() -> Activity {
        let mut state = Activity::default();
        for (id, agent) in [("a", "codex"), ("b", "claude")] {
            state.connections.insert(
                id.into(),
                Connection {
                    agent: agent.into(),
                    ..Default::default()
                },
            );
        }
        state
    }
    #[test]
    fn cue_persists_between_calls_and_only_one_agent_owns_the_desktop() {
        let mut state = activity();
        assert!(!state.visible());
        assert!(state.start("a", "1"));
        assert!(state.start("a", "1"));
        assert!(state.start("a", "2"));
        assert!(!state.start("b", "1"));
        state.end("a", "1");
        state.end("a", "2");
        assert!(state.visible());
        assert_eq!(state.engaged.len(), 1);
        state.finish("codex");
        assert!(!state.visible());
        assert!(state.start("b", "1"));
    }
    #[test]
    fn stop_and_disconnect_clear_only_their_own_calls_and_stop_blocks_late_calls() {
        let mut state = activity();
        state.start("a", "1");
        assert!(!state.start("b", "1"));
        state.finish("codex");
        assert!(!state.start("a", "late"));
        assert!(!state.visible());
        assert!(state.start("b", "1"));
        assert!(state.visible());
        state.disconnect("b");
        assert!(!state.visible());
        assert!(state.engaged.is_empty());
        state.halted.insert("codex".into());
        assert!(state.authorize_turn("codex"));
        assert!(!state.authorize_turn("codex"));
        assert!(state.start("a", "new-turn"));
    }
    #[test]
    fn attention_suspends_glow_until_the_next_real_call() {
        let mut state = activity();
        state.start("a", "1");
        state.suspended = true;
        assert!(!state.visible());
        state.end("a", "1");
        assert!(!state.visible());
        state.start("a", "2");
        assert!(state.visible());
    }
    #[test]
    fn revoked_transport_cannot_reclaim_control_on_a_new_user_turn() {
        let mut state = activity();
        state.connections.get_mut("a").unwrap().session = Some("chat-a".into());
        state.prepare_session("codex", "chat-a");
        assert!(state.start("a", "typing"));
        state.revoke("codex");
        assert!(state.prepare_session("codex", "chat-a"));
        assert!(!state.start("a", "late-refocus"));
        state.connections.insert(
            "fresh".into(),
            Connection {
                agent: "codex".into(),
                session: Some("chat-a".into()),
                ..Default::default()
            },
        );
        assert!(state.start("fresh", "new-request"));
        state.end("a", "typing");
        assert!(state.visible());
        state.disconnect("a");
        assert!(state.visible());
        assert!(!state.start("b", "wrong-agent"));
    }
    #[test]
    fn old_session_and_old_turn_completion_do_not_end_a_new_lease() {
        let mut state = activity();
        state.prepare_session("codex", "chat-b");
        assert!(!state.owns_event("chat-a", Some("old-turn")));
        assert!(!state.owns_event("chat-b", Some("old-turn")));
        state.turn = Some("new-turn".into());
        assert!(!state.owns_event("chat-b", Some("old-turn")));
        assert!(state.owns_event("chat-b", Some("new-turn")));
        state.connections.get_mut("a").unwrap().session = Some("chat-a".into());
        assert!(!state.start("a", "old-session-call"));
    }
}
