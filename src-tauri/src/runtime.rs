use crate::{process, protocol, storage, AppState};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin},
    sync::{oneshot, Mutex, Notify},
    time::{timeout, Duration},
};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

#[derive(Clone, Copy, PartialEq, Debug)]
enum TurnPhase {
    Pending,
    Started,
    Finished,
}

#[derive(Default)]
struct TurnLifecycle {
    turns: Mutex<HashMap<String, (String, TurnPhase)>>,
    changed: Notify,
}

impl TurnLifecycle {
    async fn expect(&self, thread: &str, turn: &str) {
        let mut turns = self.turns.lock().await;
        // A notification can precede its RPC response. Never regress that state.
        if turns
            .get(thread)
            .is_none_or(|(id, phase)| id != turn && *phase == TurnPhase::Finished)
        {
            if turns.len() >= 128 {
                if let Some(id) = turns
                    .iter()
                    .find(|(_, (_, p))| *p == TurnPhase::Finished)
                    .map(|(id, _)| id.clone())
                {
                    turns.remove(&id);
                }
            }
            turns.insert(thread.into(), (turn.into(), TurnPhase::Pending));
        }
        self.changed.notify_waiters();
    }

    async fn observe(&self, message: &Value) {
        let phase = match message["method"].as_str() {
            Some("turn/started") => TurnPhase::Started,
            Some("turn/completed") => TurnPhase::Finished,
            _ => return,
        };
        if let (Some(thread), Some(turn)) = (
            message["params"]["threadId"].as_str(),
            message["params"]["turn"]["id"].as_str(),
        ) {
            let mut turns = self.turns.lock().await;
            if turns.len() >= 128 && !turns.contains_key(thread) {
                if let Some(id) = turns
                    .iter()
                    .find(|(_, (_, p))| *p == TurnPhase::Finished)
                    .map(|(id, _)| id.clone())
                {
                    turns.remove(&id);
                }
            }
            turns.insert(thread.into(), (turn.into(), phase));
            self.changed.notify_waiters();
        }
    }

    async fn phase(&self, thread: &str, turn: &str) -> Result<TurnPhase, String> {
        match self.turns.lock().await.get(thread) {
            Some((id, phase)) if id == turn => Ok(*phase),
            Some(_) => Err("任务已切换，不能停止之前的任务".into()),
            None => Ok(TurnPhase::Pending),
        }
    }

    async fn ready(&self, thread: &str, turn: &str) -> Result<TurnPhase, String> {
        loop {
            let changed = self.changed.notified();
            // Register before inspecting state, so a start cannot be missed.
            tokio::pin!(changed);
            changed.as_mut().enable();
            let phase = self.phase(thread, turn).await?;
            if phase != TurnPhase::Pending {
                return Ok(phase);
            }
            changed.await;
        }
    }
}

#[derive(Default)]
pub struct Runtime {
    pub client: Mutex<Option<Arc<Client>>>,
    sessions: crate::session_runtimes::Sessions<Runtime>,
    startup: Mutex<()>,
    account_query: Mutex<()>,
    epoch: Arc<AtomicU64>,
}

pub struct Client {
    input: Mutex<ChildStdin>,
    _child: Arc<Mutex<Child>>,
    _job: process::JobGuard,
    pending: Pending,
    pub requests: Arc<Mutex<HashMap<String, Value>>>,
    next: AtomicU64,
    pub alive: Arc<AtomicBool>,
    pub last_used: Arc<AtomicU64>,
    pub pid: u32,
    routing: u64,
    tool_configuration: u64,
    project: Option<std::path::PathBuf>,
    pub loaded_threads: Mutex<Vec<String>>,
    pub turn_settings: Mutex<Option<(String, String, String)>>,
    request_namespace: String,
    pub planning_supported: Mutex<Option<bool>>,
    turns: Arc<TurnLifecycle>,
}

impl Runtime {
    pub fn for_session(&self, id: &str) -> Arc<Runtime> { self.sessions.get(id) }
    pub async fn session_client(&self, id: &str) -> Option<Arc<Client>> {
        self.sessions.existing(id)?.client.lock().await.clone()
    }
    pub async fn clients(&self) -> Vec<Arc<Client>> {
        let mut clients = self.client.lock().await.iter().cloned().collect::<Vec<_>>();
        for (_, runtime) in self.sessions.entries() {
            clients.extend(runtime.client.lock().await.iter().cloned());
        }
        clients
    }
    pub async fn shutdown(&self) {
        for (_, runtime) in self.sessions.entries() { runtime.shutdown_local().await; }
        self.shutdown_local().await;
    }
    async fn shutdown_local(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        let client = self.client.lock().await.take();
        if let Some(client) = client {
            client.alive.store(false, Ordering::Relaxed);
            let _ = client._child.lock().await.start_kill();
        }
    }
    /// A quota read must also work before the first turn, without replacing the chat client.
    pub async fn account_limits(&self, app: &AppHandle, routing: u64) -> Option<Value> {
        let _query = self.account_query.lock().await;
        if let Some(limits) = self.existing_account_limits(routing).await {
            return Some(limits);
        }
        crate::accounts::read_codex_limits(app).await.ok()
    }
    /// Allowance queries reuse the matching live client without starting/switching an agent.
    pub async fn existing_account_limits(&self, routing: u64) -> Option<Value> {
        let client = self.clients().await.into_iter()
            .filter(|c| c.routing == routing && c.alive.load(Ordering::Relaxed))
            .next()?;
        timeout(
            Duration::from_secs(10),
            client.request("account/rateLimits/read", json!({})),
        )
        .await
        .ok()?
        .ok()
    }
    pub async fn clear_for_agent_switch(&self) {
        self.shutdown().await;
    }
    pub async fn get(&self, app: &AppHandle) -> Result<Arc<Client>, String> {
        self.get_for_session(app, None).await
    }
    pub async fn get_for_session(
        &self,
        app: &AppHandle,
        session_id: Option<&str>,
    ) -> Result<Arc<Client>, String> {
        self.get_for_connection(app, session_id, None).await
    }
    pub async fn get_for_connection(
        &self,
        app: &AppHandle,
        session_id: Option<&str>,
        connection_id: Option<&str>,
    ) -> Result<Arc<Client>, String> {
        if let Some(id) = session_id {
            return self.for_session(id).get_context(app, session_id, connection_id).await;
        }
        self.get_context(app, None, connection_id).await
    }
    async fn get_context(
        &self, app: &AppHandle, session_id: Option<&str>, connection_id: Option<&str>,
    ) -> Result<Arc<Client>, String> {
        let _startup = self.startup.lock().await;
        crate::desktop_lifecycle::ensure_running(app)?;
        let route = app.state::<AppState>().store.route_with_connection(
            "codex",
            session_id,
            connection_id,
        )?;
        let routing = route.fingerprint();
        let tool_configuration = crate::client_features::tool_configuration(app)?;
        let project = session_id
            .map(|id| {
                let state = app.state::<AppState>();
                let session = state.store.session(id)?;
                state
                    .store
                    .session_workspace(&session)
                    .map(|project| std::path::PathBuf::from(project.path))
            })
            .transpose()?;
        let mut slot = self.client.lock().await;
        if let Some(client) = slot.as_ref().filter(|c| {
            c.alive.load(Ordering::Relaxed)
                && context_matches(
                    (c.routing, c.tool_configuration),
                    c.project.as_deref(),
                    (routing, tool_configuration),
                    project.as_deref(),
                )
        }) {
            return Ok(client.clone());
        }
        let selected_is_active = session_id.is_some_and(|id| {
            app.state::<AppState>().store.session(id).is_ok_and(|s| matches!(s.status.as_str(), "running" | "waiting"))
        });
        if selected_is_active
            && slot
                .as_ref()
                .is_some_and(|c| c.alive.load(Ordering::Relaxed))
        {
            return Err("运行期间无法更换 Codex 连接，请等待当前任务完成".into());
        }
        slot.take();
        let generation = self.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let client = Arc::new(
            Client::start(
                app.clone(),
                self.epoch.clone(),
                generation,
                &route,
                tool_configuration,
                project.as_deref(),
                session_id.map(str::to_owned),
            )
            .await?,
        );
        *slot = Some(client.clone());
        drop(slot);
        let initialized = async {
            client.request("initialize",json!({"clientInfo":{"name":"supercode","title":"SuperCode","version":"0.1.0"},"capabilities":{"experimentalApi":true}})).await?;
            client.write(json!({"method":"initialized","params":{}})).await
        }.await;
        if let Err(error) = initialized { self.shutdown_local().await; return Err(error); }
        Ok(client)
    }

    pub async fn release(&self, app: &AppHandle) -> Result<(), String> {
        if app.state::<AppState>().store.running()? != 0 {
            return Err("仍有任务运行或等待审批，请先停止任务".into());
        }
        self.shutdown().await;
        Ok(())
    }

    pub async fn release_idle(&self, app: &AppHandle) {
        for (id, runtime) in self.sessions.entries() {
            if app.state::<AppState>().store.session(&id).is_ok_and(|s| matches!(s.status.as_str(), "starting" | "running" | "waiting")) { continue; }
            runtime.release_idle_local().await;
            if runtime.client.lock().await.is_none() { self.sessions.remove_idle(&id, &runtime); }
        }
        self.release_idle_local().await;
    }
    async fn release_idle_local(&self) {
        let mut slot = self.client.lock().await;
        if slot.as_ref().is_some_and(|c| (storage::now() as u64).saturating_sub(c.last_used.load(Ordering::Relaxed)) >= 300) {
            self.epoch.fetch_add(1, Ordering::Relaxed);
            slot.take();
        }
    }
}

impl Client {
    async fn start(
        app: AppHandle,
        epoch: Arc<AtomicU64>,
        generation: u64,
        route: &crate::session_config::Route,
        tool_configuration: u64,
        project: Option<&std::path::Path>,
        session_id: Option<String>,
    ) -> Result<Self, String> {
        crate::runtime_diagnostics::register_secrets(&app, &route.config);
        if let Some(profile)=&route.profile { crate::runtime_diagnostics::register_secrets(&app, &profile.config); }
        let executable = crate::agents::resolve(&app, "codex")?.program;
        let mut overrides =
            process::codex_config_overrides(app.state::<AppState>().store.load_mcp()?)?;
        overrides.extend(crate::client_features::codex_mcp_overrides(&app)?);
        overrides.extend(crate::extensions::codex_overrides(&app, project)?);
        if route.config["official"] == true {
            overrides.push("model_provider=\"openai\"".into());
        }
        let profile = &route.profile;
        if let Some(profile) = profile {
            overrides.extend(crate::ccswitch::codex_overrides(&profile.config)?);
        }
        let mut args = vec!["app-server", "--listen", "stdio://"];
        for override_value in &overrides {
            args.extend(["-c", override_value.as_str()]);
        }
        let mut command = process::command(&executable, &args);
        crate::official_accounts::configure(&mut command, &app, "codex", &route.config)?;
        if route.config["official"] == true
            || route.profile.as_ref().is_some_and(|p| p.is_official())
        {
            for name in [
                "OPENAI_API_KEY",
                "OPENAI_BASE_URL",
                "SUPERCODE_PROVIDER_API_KEY",
            ] {
                command.env_remove(name);
            }
        }
        if let Some(project) = project {
            command.current_dir(project);
        }
        if let Some(key) = profile.as_ref().and_then(|p| p.config["apiKey"].as_str()) {
            command.env("SUPERCODE_PROVIDER_API_KEY", key);
        }
        if let Some(id) = &session_id { command.env("SUPERCODE_SESSION_ID", id); }
        else { command.env_remove("SUPERCODE_SESSION_ID"); }
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Codex 启动失败：{e}"))?;
        let job = process::JobGuard::attach(&child)?;
        let pid = child.id().ok_or("Codex 进程已退出")?;
        crate::runtime_diagnostics::record(&app, session_id.as_deref(), "agent_process_started", json!({"source":"codex","pid":pid}));
        let input = child.stdin.take().ok_or("无法连接 Codex stdin")?;
        let output = child.stdout.take().ok_or("无法连接 Codex stdout")?;
        let stderr = child.stderr.take().ok_or("无法连接 Codex stderr")?;
        let child = Arc::new(Mutex::new(child));
        let reader_child = child.clone();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let requests = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let (reader_pending, reader_requests, reader_alive) =
            (pending.clone(), requests.clone(), alive.clone());
        let last_used = Arc::new(AtomicU64::new(storage::now() as u64));
        let reader_used = last_used.clone();
        let reader_app = app.clone();
        let turns = Arc::new(TurnLifecycle::default());
        let reader_turns = turns.clone();
        let request_namespace = format!("codex:{}:", uuid::Uuid::new_v4());
        let reader_namespace = request_namespace.clone();
        let log_session = session_id.clone();
        tauri::async_runtime::spawn(async move {
            let mut reader = BufReader::new(output);
            loop {
                let frame = match read_frame(&mut reader).await {
                    Ok(Some(frame)) => frame,
                    Ok(None) => break,
                    Err(error) => {
                        crate::runtime_diagnostics::message(&reader_app, session_id.as_deref(), "codex", "error", "protocol_read_error", &error);
                        break;
                    }
                };
                let mut message: Value = match serde_json::from_slice(&frame) {
                    Ok(value) => value,
                    Err(error) => {
                        crate::runtime_diagnostics::message(&reader_app, session_id.as_deref(), "codex", "error", "protocol_parse_error", &error.to_string());
                        continue;
                    }
                };
                reader_used.store(storage::now() as u64, Ordering::Relaxed);
                if message.get("method").is_none() {
                    if let Some(id) = message["id"].as_u64() {
                        if let Some(sender) = reader_pending.lock().await.remove(&id) {
                            let result = if message.get("error").is_some() {
                                crate::runtime_diagnostics::record(&reader_app,session_id.as_deref(),"agent_request_error",
                                    json!({"source":"codex","code":message["error"]["code"],"message":message["error"]["message"]}));
                                Err(message["error"]["message"]
                                    .as_str()
                                    .unwrap_or("Codex 请求失败")
                                    .to_string())
                            } else {
                                Ok(message["result"].clone())
                            };
                            let _ = sender.send(result);
                        }
                    }
                } else {
                    reader_turns.observe(&message).await;
                    if let Some(id) = &session_id { message["params"]["appSessionId"] = id.clone().into(); }
                    namespace_request(&mut message, &reader_namespace);
                    if let Some(id) = message.get("id") {
                        reader_requests
                            .lock()
                            .await
                            .insert(id.to_string(), message.clone());
                    }
                    if message["method"] == "serverRequest/resolved" {
                        reader_requests
                            .lock()
                            .await
                            .remove(&message["params"]["requestId"].to_string());
                    }
                    publish_event(&reader_app, message);
                }
            }
            reader_alive.store(false, Ordering::Relaxed);
            let exit_code = reader_child.lock().await.try_wait().ok().flatten().and_then(|s|s.code());
            crate::runtime_diagnostics::record(&reader_app, session_id.as_deref(), "agent_process_output_closed", json!({"source":"codex","pid":pid,"exitCode":exit_code}));
            let _ = reader_child.lock().await.kill().await;
            for (_, sender) in reader_pending.lock().await.drain() {
                let _ = sender.send(Err("Codex 连接已关闭，请重新发送消息".into()));
            }
            reader_requests.lock().await.clear();
            if epoch.load(Ordering::Relaxed) == generation {
                if let Some(id) = session_id {
                    let state = reader_app.state::<AppState>();
                    let active = state.store.session(&id).ok().is_some_and(|s| matches!(s.status.as_str(), "running" | "starting" | "waiting"));
                    crate::runtime_diagnostics::record(&reader_app, Some(&id), "agent_connection_closed",
                        json!({"active":active,"pid":pid,"exitCode":exit_code,"errorType":active.then_some("connection")}));
                    if let Some(buffer) = state.activity.existing(&id) {
                        if let Ok(mut activity) = buffer.lock() {
                            for item in activity.finish("interrupted") {
                                if let (Some(item_id), Some((role, text, kind, data))) = (item["id"].as_str(), protocol::item_message(&item)) {
                                    let _ = state.store.save_message(item_id, &id, &role, &text, &kind, &data);
                                }
                            }
                        }
                        state.activity.remove_idle(&id, &buffer);
                    }
                    state.store.fail_session(&id);
                    crate::outbox::kick(&reader_app);
                    crate::desktop_lifecycle::refresh(&reader_app);
                }
                let _ = reader_app.emit("workspace-updated", ());
                let _ = reader_app.emit("runtime-stopped", ());
            }
        });
        tauri::async_runtime::spawn(async move {
            crate::runtime_diagnostics::stderr(Some(app), log_session, "codex", stderr).await;
        });
        Ok(Self {
            input: Mutex::new(input),
            _child: child,
            _job: job,
            pending,
            requests,
            request_namespace,
            next: AtomicU64::new(1),
            alive,
            last_used,
            loaded_threads: Mutex::new(Vec::new()),
            turn_settings: Mutex::new(None),
            planning_supported: Mutex::new(None),
            turns,
            pid,
            routing: route.fingerprint(),
            tool_configuration,
            project: project.map(std::path::Path::to_owned),
        })
    }

    pub async fn write(&self, message: Value) -> Result<(), String> {
        self.last_used
            .store(storage::now() as u64, Ordering::Relaxed);
        let mut data = serde_json::to_vec(&message).map_err(|e| e.to_string())?;
        data.push(b'\n');
        self.input
            .lock()
            .await
            .write_all(&data)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        if let Err(error) = self
            .write(json!({"id":id,"method":method,"params":params}))
            .await
        {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }
        let result = timeout(Duration::from_secs(45), rx).await;
        self.pending.lock().await.remove(&id);
        match result {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => Err("Codex 连接已关闭".into()),
            Err(_) => Err(format!("Codex 请求超时：{method}")),
        }
    }

    pub async fn expect_turn(&self, thread: &str, turn: &str) {
        self.turns.expect(thread, turn).await;
    }

    pub async fn interrupt_turn(&self, thread: &str, turn: &str) -> Result<(), String> {
        timeout(Duration::from_secs(15), async {
            loop {
                if self.turns.ready(thread, turn).await? == TurnPhase::Finished {
                    return Ok(());
                }
                match self
                    .request("turn/interrupt", json!({"threadId":thread,"turnId":turn}))
                    .await
                {
                    Ok(_) => return Ok(()),
                    // Some app-server versions notify before installing the active task.
                    // Retry only for this exact turn; a completed/replaced turn exits above.
                    Err(error) if error == "no active turn to interrupt" => {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        })
        .await
        .map_err(|_| "Codex 停止超时，请重试".to_string())?
    }

    pub async fn respond(&self, id: Value, result: Value) -> Result<(), String> {
        let key = id.to_string();
        if !self.requests.lock().await.contains_key(&key) {
            return Err("该请求已结束，请刷新会话".into());
        }
        let raw_id = id.as_str().and_then(|id| id.strip_prefix(&self.request_namespace)).ok_or("请求 ID 无效")?;
        let raw_id: Value = serde_json::from_str(raw_id).map_err(|_| "请求 ID 无效")?;
        if result.is_null() {
            self.write(json!({"id":raw_id,"error":{"code":-32601,"message":"SuperCode does not support this request yet"}})).await?;
        } else {
            self.write(json!({"id":raw_id,"result":result})).await?;
        }
        self.requests.lock().await.remove(&key);
        Ok(())
    }
}

fn context_matches(
    active_route: (u64, u64),
    active_project: Option<&std::path::Path>,
    route: (u64, u64),
    project: Option<&std::path::Path>,
) -> bool {
    active_route == route && project.is_none_or(|path| active_project == Some(path))
}
fn namespace_request(message: &mut Value, namespace: &str) {
    if let Some(id) = message.get("id").cloned() { message["id"] = format!("{namespace}{id}").into(); }
    if message["method"] == "serverRequest/resolved" {
        if let Some(id) = message["params"].get("requestId").cloned() {
            message["params"]["requestId"] = format!("{namespace}{id}").into();
        }
    }
}

pub(crate) async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Vec<u8>>, String> {
    let mut frame = Vec::new();
    loop {
        let buffer = reader.fill_buf().await.map_err(|e| e.to_string())?;
        if buffer.is_empty() {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Err("Codex 返回了不完整的 JSONL 数据".into())
            };
        }
        let end = buffer.iter().position(|b| *b == b'\n');
        let length = end.map_or(buffer.len(), |i| i + 1);
        if frame.len() + length > 4 * 1024 * 1024 {
            return Err("Codex 单条事件超过 4 MB，连接已关闭以保护内存".into());
        }
        frame.extend_from_slice(&buffer[..length]);
        reader.consume(length);
        if end.is_some() {
            return Ok(Some(frame));
        }
    }
}

pub(crate) fn publish_event(app: &AppHandle, mut message: Value) {
    if message["params"]["appSessionId"].is_null() {
        if let Some(native) = message["params"]["threadId"].as_str() {
            if let Ok(session) = app.state::<AppState>().store.for_native(native) { message["params"]["appSessionId"] = session.id.into(); }
        }
    }
    crate::media::normalize_event(app, &mut message);
    if matches!(message["method"].as_str(),Some("item/started"|"item/updated"|"item/completed")) {
        crate::file_changes::normalize_native(&mut message["params"]["item"]);
        if let Some(native)=message["params"]["threadId"].as_str() {
            if let Ok(session)=app.state::<AppState>().store.for_native(native) {
                let item=&message["params"]["item"];
                if session.agent!="codex" && item["type"]=="claudeToolCall" && matches!(item["tool"].as_str(),Some("Edit"|"Write"|"MultiEdit"|"NotebookEdit"|"apply_patch"|"ApplyPatch")) {
                    crate::client_features::capture_change(app,&session,&mut message["params"]);
                }
            }
        }
        // Native pre/post-write content is used only by the bounded diff tracker;
        // retain the computed patch, never duplicate both full texts in history/UI.
        if let Some(item)=message["params"]["item"].as_object_mut() { item.remove("fileSnapshot"); }
    }
    persist_event(app, &message);
    crate::runtime_diagnostics::observe(app, &message);
    crate::desktop_control::observe(app, &message);
    crate::browser::observe(app, &message);
    if message["method"] == "turn/completed" {
        if message["params"]["turn"]["status"] != "completed" {
            if let Some(native) = message["params"]["threadId"].as_str() {
                if let Ok(session) = app.state::<AppState>().store.for_native(native) {
                    if matches!(session.status.as_str(), "failed" | "interrupted") {
                        app.state::<AppState>().store.pause_followups(&session.id);
                    }
                }
            }
        }
        crate::outbox::kick(app);
    }
    let _ = app.emit("agent-event", message);
}

pub(crate) fn persist_event(app: &AppHandle, message: &Value) {
    let state = app.state::<AppState>();
    let method = message["method"].as_str().unwrap_or("");
    let params = &message["params"];
    let Some(native) = params["threadId"].as_str() else {
        return;
    };
    let Ok(session) = state.store.for_native(native) else {
        return;
    };
    if method == "turn/completed" {
        if let Ok(mut snapshots) = state.change_snapshots.lock() {
            snapshots.finish_session(&session.id);
        }
    }
    if method == "thread/tokenUsage/updated" {
        let mut usage = params["tokenUsage"].clone();
        if !usage.is_object() {
            return;
        }
        usage["cumulative"] = json!(session.agent == "codex");
        // Keep context-only ACP/stream updates, without counting them as billing.
        if usage["total"].as_object().is_none_or(|v| v.is_empty())
            && usage.get("contextTokens").is_none()
        {
            return;
        }
        let turn = params["turnId"]
            .as_str()
            .or(session.turn_id.as_deref())
            .unwrap_or("unknown");
        let _ = state.store.record_usage(&session, turn, usage);
        return;
    }
    if method == "turn/diff/updated" {
        if let Some(turn) = params["turnId"].as_str() {
            let diff = protocol::bounded(params["diff"].as_str().unwrap_or(""), 256 * 1024);
            let _ = state.store.save_message(
                &format!("diff-{turn}"),
                &session.id,
                "activity",
                &diff,
                "turnDiff",
                &json!({"turnId":turn,"status":"completed"}),
            );
        }
        return;
    }
    if matches!(method, "thread/goal/updated" | "thread/goal/cleared") {
        let text = crate::agent_commands::goal_text(&params["goal"]);
        let _ = state.store.save_message(
            &format!("goal-{native}"),
            &session.id,
            "system",
            &text,
            "goalState",
            &json!({"goal":params["goal"]}),
        );
        return;
    }
    let session_activity = state.activity.get(&session.id);
    let mut activity = match session_activity.lock() {
        Ok(a) => a,
        Err(_) => return,
    };
    let item = activity.update(message);
    let save = |item: &Value| {
        if let (Some(id), Some((role, text, kind, data))) =
            (item["id"].as_str(), protocol::item_message(item))
        {
            state
                .store
                .save_message(id, &session.id, &role, &text, &kind, &data)
        } else {
            Ok(())
        }
    };
    if let Some(item) = &item {
        let _ = save(item);
    }
    let result = match method {
        "turn/started" => {
            state
                .store
                .set_status(&session.id, "running", params["turn"]["id"].as_str())
        }
        "turn/completed" => {
            let status = params["turn"]["status"].as_str().unwrap_or("failed");
            for item in activity.finish(status) {
                let _ = save(&item);
            }
            if let Some(error) = params["turn"]["error"]["message"].as_str() {
                let _ = state.store.save_message(
                    &format!("error-{}", params["turn"]["id"]),
                    &session.id,
                    "system",
                    error,
                    "error",
                    &Value::Null,
                );
            }
            state.store.complete_turn(
                &session.id,
                params["turn"]["id"].as_str().unwrap_or(""),
                status,
            )
        }
        _ if message.get("id").is_some() => {
            state
                .store
                .set_status(&session.id, "waiting", session.turn_id.as_deref())
        }
        "serverRequest/resolved" => {
            state
                .store
                .set_status(&session.id, "running", session.turn_id.as_deref())
        }
        _ => Ok(()),
    };
    if result.is_ok() {
        crate::notifications::on_agent_event(app, &session, message);
        if matches!(
            method,
            "turn/started" | "turn/completed" | "serverRequest/resolved"
        ) || message.get("id").is_some()
        {
            drop(activity);
            if method == "turn/completed" { state.activity.remove_idle(&session.id, &session_activity); }
            crate::desktop_lifecycle::refresh(app);
        }
    }
    if let Err(error) = result {
        crate::runtime_diagnostics::message(app, Some(&session.id), "storage", "error", "message_save_error", &error);
    }
}

pub async fn idle_watch(app: AppHandle) {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        crate::runtime_diagnostics::cleanup(&app);
        let state = app.state::<AppState>();
        state.claude.release_idle().await;
        state.native.release_idle().await;
        state.runtime.release_idle(&app).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn turn_event(method: &str, id: &str) -> Value {
        json!({"method":method,"params":{"threadId":"thread","turn":{"id":id}}})
    }

    #[tokio::test]
    async fn immediate_stop_waits_for_native_start_and_keeps_completion() {
        let turns = Arc::new(TurnLifecycle::default());
        turns.expect("thread", "first").await;
        let waiting = turns.clone();
        let task = tokio::spawn(async move { waiting.ready("thread", "first").await });
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        turns.observe(&turn_event("turn/started", "first")).await;
        assert_eq!(task.await.unwrap().unwrap(), TurnPhase::Started);
        turns.observe(&turn_event("turn/completed", "first")).await;
        // A late turn/start reply must not resurrect a completed turn.
        turns.expect("thread", "first").await;
        assert_eq!(
            turns.ready("thread", "first").await.unwrap(),
            TurnPhase::Finished
        );
        turns.expect("thread", "second").await;
        assert!(turns.ready("thread", "first").await.is_err());
    }

    #[tokio::test]
    async fn native_start_before_rpc_reply_stays_ready_and_history_is_bounded() {
        let turns = TurnLifecycle::default();
        turns.observe(&turn_event("turn/started", "first")).await;
        turns.expect("thread", "first").await;
        assert_eq!(
            turns.ready("thread", "first").await.unwrap(),
            TurnPhase::Started
        );
        for i in 0..256 {
            let event = json!({"method":"turn/completed","params":{"threadId":format!("t-{i}"),"turn":{"id":"finished"}}});
            turns.observe(&event).await;
        }
        assert_eq!(turns.turns.lock().await.len(), 128);
        assert_eq!(
            turns.phase("thread", "first").await.unwrap(),
            TurnPhase::Started
        );
    }
    #[test]
    fn project_extension_context_is_not_reused_for_another_project() {
        let a = std::path::Path::new("project-a");
        let b = std::path::Path::new("project-b");
        assert!(context_matches((1, 3), Some(a), (1, 3), Some(a)));
        assert!(!context_matches((1, 3), Some(a), (1, 3), Some(b)));
        assert!(!context_matches((1, 3), None, (1, 3), Some(a)));
        assert!(!context_matches((1, 3), Some(a), (2, 3), Some(a)));
        assert!(!context_matches((1, 3), Some(a), (1, 4), Some(a)));
        // Inventory queries can reuse a matching live client without switching it.
        assert!(context_matches((1, 3), Some(a), (1, 3), None));
    }
    #[test]
    fn request_ids_are_isolated_and_resolved_with_the_same_namespace() {
        for raw in [json!(1), json!("one:two")] {
            let mut a = json!({"method":"approval","id":raw});
            let mut b = a.clone();
            namespace_request(&mut a, "codex:a:");
            namespace_request(&mut b, "codex:b:");
            assert_ne!(a["id"], b["id"]);
            assert_eq!(serde_json::from_str::<Value>(a["id"].as_str().unwrap().strip_prefix("codex:a:").unwrap()).unwrap(), raw);
            let mut resolved = json!({"method":"serverRequest/resolved","params":{"requestId":raw}});
            namespace_request(&mut resolved, "codex:a:");
            assert_eq!(resolved["params"]["requestId"], a["id"]);
        }
    }
    #[tokio::test]
    async fn jsonl_only_splits_at_lf_and_rejects_partial_frames() {
        let bytes = "{\"text\":\"你好\u{2028}🧋\"}\n{\"id\":2}\n".as_bytes();
        let mut reader = BufReader::with_capacity(4, bytes);
        let first = read_frame(&mut reader).await.unwrap().unwrap();
        assert!(serde_json::from_slice::<Value>(&first).is_ok());
        assert_eq!(
            serde_json::from_slice::<Value>(&read_frame(&mut reader).await.unwrap().unwrap())
                .unwrap()["id"],
            2
        );
        assert!(read_frame(&mut reader).await.unwrap().is_none());
        let mut partial = BufReader::new("{\"id\":3}".as_bytes());
        assert!(read_frame(&mut partial).await.is_err());
    }
}
