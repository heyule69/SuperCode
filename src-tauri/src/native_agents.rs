//! Native OpenCode ACP and Pi RPC adapters. Prompts/answers are JSONL, never shell commands.
use crate::{
    agents::Launch, client_features, process, protocol, runtime, storage::Session, AppState,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    process::{Child, ChildStdin},
    sync::{mpsc, oneshot, Mutex, Notify},
};
mod mapper;
mod requests;
use mapper::*;
use requests::*;

fn init() -> Value {
    json!({"protocolVersion":1,"clientInfo":{"name":"SuperCode","version":env!("CARGO_PKG_VERSION")},"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}})
}
pub async fn probe_cancel(
    agent: &str,
    launch: &Launch,
    cwd: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    if agent == "pi" {
        let version = crate::agents::capture(launch, &["--version"], cwd).await?;
        crate::agents::compatible_version(agent, &version)?;
    }
    match agent {
        "opencode" | "pi" | "codex" => {
            let pi = agent == "pi";
            let args = if pi {
                vec![
                    "--mode".into(),
                    "rpc".into(),
                    "--no-session".into(),
                    "--no-extensions".into(),
                    "--no-skills".into(),
                    "--no-mcp".into(),
                ]
            } else if agent == "codex" {
                vec!["app-server".into()]
            } else {
                vec!["acp".into()]
            };
            let (client, mut events) = Client::start(launch, &args, cwd, &[], pi, None).await?;
            let drain = tokio::spawn(async move {
                while let Some(v) = events.recv().await {
                    if v["type"] == "supercode_disconnected" {
                        break;
                    }
                }
            });
            let request = client
                .request(
                    if pi { "get_state" } else { "initialize" },
                    if pi { json!({}) } else if agent=="codex" {json!({"clientInfo":{"name":"supercode","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})}else { init() },
                    45,
                );
            let result = tokio::select! {result=request=>result,_=wait_cancel(cancel)=>Err("已取消连接测试".into())};
            client.close().await;
            drain.abort();
            let data = result?;
            if agent == "opencode" && data["protocolVersion"] != 1 {
                return Err("OpenCode ACP 版本不兼容，请更新".into());
            }
            Ok(())
        }
        "claude" => {
            let help = tokio::select! {v=crate::agents::capture(launch,&["--help"],cwd)=>v?,_=wait_cancel(cancel)=>return Err("已取消连接测试".into())};
            if !help.contains("--input-format") {
                return Err("Claude CLI 不支持双向协议，请更新".into());
            }
            Ok(())
        }
        _ => Err("Agent 无效".into()),
    }
}
async fn wait_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}
#[derive(Default)]
pub struct Runtime {
    pub client: Mutex<Option<Arc<Turn>>>,
    warm: Mutex<Option<Warm>>,
    sessions: crate::session_runtimes::Sessions<Runtime>,
}
struct Warm {
    client: Arc<Client>,
    rx: mpsc::Receiver<Value>,
    native: String,
    session: String,
    config: Value,
    defaults: Value,
    signature: String,
    used: i64,
}
pub struct Turn {
    client: Arc<Client>,
    pub requests: Arc<Mutex<HashMap<String, Value>>>,
    session: String,
    native: String,
    turn: String,
    agent: String,
    stopped: AtomicBool,
    accepting: AtomicBool,
    inputs_pending: AtomicU64,
    input_done: Notify,
    settled: AtomicBool,
    prompt_accepted: AtomicBool,
    prompt_ready: Notify,
    started: AtomicBool,
    stop_done: AtomicBool,
    signature: String,
}

fn verified_concurrent_prompt(version: Option<&str>) -> bool {
    // ACP has no advertised steering capability. This exact release was source-audited
    // and exercised with concurrent prompts; newer ACP implementations may reject them.
    version == Some("1.18.23")
}
impl Turn {
    pub fn pid(&self) -> u32 {
        self.client.pid
    }
    fn after_setup(&self, native: String) -> Self {
        Self {
            client: self.client.clone(),
            requests: self.requests.clone(),
            session: self.session.clone(),
            native,
            turn: self.turn.clone(),
            agent: self.agent.clone(),
            stopped: AtomicBool::new(false),
            accepting: AtomicBool::new(true),
            inputs_pending: AtomicU64::new(0),
            input_done: Notify::new(),
            settled: AtomicBool::new(false),
            prompt_accepted: AtomicBool::new(false),
            prompt_ready: Notify::new(),
            started: AtomicBool::new(false),
            stop_done: AtomicBool::new(true),
            signature: self.signature.clone(),
        }
    }
}
fn publish(app: &AppHandle, value: Value) {
    runtime::publish_event(app, value);
}
fn event(t: &Turn, method: &str, mut p: Value) -> Value {
    p["threadId"] = t.native.clone().into();
    p["turnId"] = t.turn.clone().into();
    p["appSessionId"] = t.session.clone().into();
    json!({"method":method,"params":p})
}
impl Runtime {
    pub fn for_session(&self, id: &str) -> Arc<Runtime> {
        self.sessions.get(id)
    }
    pub async fn clients(&self) -> Vec<Arc<Turn>> {
        let mut clients = self.client.lock().await.iter().cloned().collect::<Vec<_>>();
        for (_, runtime) in self.sessions.entries() {
            clients.extend(runtime.client.lock().await.iter().cloned());
        }
        clients
    }
    pub async fn warm_pids(&self) -> Vec<u32> {
        let mut pids = self.warm_pid().await.into_iter().collect::<Vec<_>>();
        for (_, runtime) in self.sessions.entries() {
            pids.extend(runtime.warm_pid().await);
        }
        pids
    }
    pub async fn steering_mode(&self, session: &str) -> &'static str {
        let runtime = self.for_session(session);
        let slot = runtime.client.lock().await;
        if let Some(t) = slot.as_ref().filter(|t| t.session == session) {
            if t.agent == "pi" || t.client.concurrent_prompt.load(Ordering::Acquire) {
                return "native";
            }
        }
        "interrupt"
    }
    pub async fn warm_pid(&self) -> Option<u32> {
        self.warm.lock().await.as_ref().map(|c| c.client.pid)
    }
    pub async fn release_idle(&self) {
        for (id, runtime) in self.sessions.entries() {
            runtime.release_idle_local().await;
            if runtime.client.lock().await.is_none() && runtime.warm.lock().await.is_none() {
                self.sessions.remove_idle(&id, &runtime);
            }
        }
        self.release_idle_local().await;
    }
    async fn release_idle_local(&self) {
        let mut warm = self.warm.lock().await;
        if warm
            .as_ref()
            .is_some_and(|c| crate::storage::now().saturating_sub(c.used) >= 300)
        {
            if let Some(c) = warm.take() {
                c.client.close().await;
            }
        }
    }
    pub async fn steer(
        &self,
        app: &AppHandle,
        session: &str,
        expected: &str,
        followup_id: &str,
        content: Vec<Value>,
        permission: &str,
        effort: Option<&str>,
    ) -> Result<(), String> {
        let runtime = self.for_session(session);
        let slot = runtime.client.lock().await;
        let t = slot.as_ref().cloned().ok_or("Agent 尚未启动")?;
        if t.session != session
            || t.turn != expected
            || !t.accepting.load(Ordering::Acquire)
            || t.stopped.load(Ordering::Acquire)
        {
            return Err("当前任务已变化，请重新发送".into());
        }
        client_features::validate_steering_settings(&t.signature, permission, effort)?;
        let (method, params) = if t.agent == "pi" {
            let mut p = prompt_content("pi", &content, "");
            // prompt's streamingBehavior also starts a run if the previous run settled
            // just before this JSONL command was read. steer alone leaves an idle queue stranded.
            p["streamingBehavior"] = json!("steer");
            ("prompt", p)
        } else if t.agent == "opencode" && t.client.concurrent_prompt.load(Ordering::Acquire) {
            (
                "session/prompt",
                json!({"sessionId":t.native,"prompt":prompt_content("opencode",&content,"")}),
            )
        } else {
            return Err("本机 OpenCode ACP 版本未验证并行补充消息，请使用停止并引导".into());
        };
        t.inputs_pending.fetch_add(1, Ordering::AcqRel);
        let (id, rx) = match t.client.begin_request(method, params).await {
            Ok(v) => v,
            Err(e) => {
                t.inputs_pending.fetch_sub(1, Ordering::AcqRel);
                t.input_done.notify_one();
                return Err(e);
            }
        };
        let app = app.clone();
        let followup_id = followup_id.to_owned();
        tokio::spawn(async move {
            let reply = tokio::time::timeout(Duration::from_secs(3600), rx).await;
            t.client.pending.lock().await.remove(&id);
            match reply {
                Ok(Ok(Ok(v))) if v["stopReason"] == "cancelled" => {
                    crate::outbox::native_failed(&app, &followup_id, "任务已停止，补充消息尚未完成")
                }
                Ok(Ok(Ok(_))) => crate::outbox::native_consumed(&app, &followup_id),
                Ok(Ok(Err(e))) => crate::outbox::native_failed(&app, &followup_id, &e),
                _ => crate::outbox::native_failed(
                    &app,
                    &followup_id,
                    "Agent 尚未确认补充消息，消息已保留",
                ),
            }
            t.inputs_pending.fetch_sub(1, Ordering::AcqRel);
            t.input_done.notify_one();
        });
        Ok(())
    }
    pub async fn shutdown(&self) {
        for (_, runtime) in self.sessions.entries() {
            runtime.shutdown_local().await;
        }
        self.shutdown_local().await;
    }
    async fn shutdown_local(&self) {
        if let Some(c) = self.warm.lock().await.take() {
            c.client.close().await;
        }
        if let Some(turn) = self.client.lock().await.take() {
            turn.stopped.store(true, Ordering::Relaxed);
            turn.client.close().await;
        }
    }
    pub async fn interrupt(&self, session: &str) -> Result<(), String> {
        let runtime = self.for_session(session);
        let slot = runtime.client.lock().await;
        let t = slot.clone().ok_or("Agent 尚未启动")?;
        if t.session != session {
            return Err("此会话没有运行中的任务".into());
        }
        if !t.accepting.load(Ordering::Acquire)
            && (t.prompt_accepted.load(Ordering::Acquire)
                || t.started.load(Ordering::Acquire)
                || !t.stop_done.load(Ordering::Acquire))
        {
            // Completion owns the connection now; a late stop must not cancel
            // the next prompt after this client is moved into the warm cache.
            return Err("当前任务已结束或正在停止".into());
        }
        t.stop_done.store(false, Ordering::Release);
        t.stopped.store(true, Ordering::Relaxed);
        let accepting = t.accepting.swap(false, Ordering::AcqRel);
        drop(slot);
        // During setup no prompt exists yet. ACP cancellation before the session
        // loop starts may be ignored, so close early starts rather than let work
        // begin after the user has stopped it.
        if !t.started.load(Ordering::Acquire) && (!accepting || t.agent != "pi") {
            t.client.close().await;
            return Ok(());
        }
        let result = if t.agent == "pi" {
            // Pi input hooks are asynchronous. abort before prompt acceptance
            // can otherwise race with a run that has not started yet.
            let ready = tokio::time::timeout(Duration::from_secs(10), async {
                while !t.prompt_accepted.load(Ordering::Acquire) {
                    t.prompt_ready.notified().await;
                }
            })
            .await;
            if ready.is_err() {
                t.client.close().await;
                return Ok(());
            }
            // Already-submitted steering can still be inside an asynchronous input
            // hook. Let its acceptance settle before clearing, so it cannot refill
            // the native queue after cancellation.
            let admitted = tokio::time::timeout(Duration::from_secs(10), async {
                while t.inputs_pending.load(Ordering::Acquire) > 0 {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await;
            if admitted.is_err() || t.client.request("clear_queue", json!({}), 5).await.is_err() {
                t.client.close().await;
                return Ok(());
            }
            // Pi abort otherwise continues queued messages. Clear first, then
            // abort; a final clear also removes work queued by cancellation hooks.
            match t.client.request("abort", json!({}), 10).await {
                Ok(_) => t
                    .client
                    .request("clear_queue", json!({}), 5)
                    .await
                    .map(|_| ()),
                Err(e) => Err(e),
            }
        } else {
            t.client.write(json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":t.native}})).await
        };
        if result.is_err() {
            t.client.close().await;
        }
        t.stop_done.store(true, Ordering::Release);
        t.input_done.notify_one();
        Ok(())
    }
    pub async fn respond(&self, app: &AppHandle, id: Value, result: Value) -> Result<(), String> {
        let id = id.as_str().ok_or("请求 ID 无效")?;
        let mut selected = None;
        for turn in self.clients().await {
            if turn.requests.lock().await.contains_key(id) {
                selected = Some(turn);
                break;
            }
        }
        let t = selected.ok_or("该请求已结束")?;
        let request = t
            .requests
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or("该请求已结束")?;
        let wire = permission_reply(&request, &result, &t.agent)?;
        t.client.write(wire).await?;
        t.requests.lock().await.remove(id);
        let current = if let Some(runtime) = self.sessions.existing(&t.session) {
            runtime
                .client
                .lock()
                .await
                .as_ref()
                .filter(|active| {
                    active.turn == t.turn && Arc::ptr_eq(&active.requests, &t.requests)
                })
                .cloned()
        } else {
            None
        };
        publish(
            app,
            event(
                current.as_deref().unwrap_or(&t),
                "serverRequest/resolved",
                json!({"requestId":id}),
            ),
        );
        Ok(())
    }
}
fn permission_reply(request: &Value, result: &Value, agent: &str) -> Result<Value, String> {
    let raw = &request["params"]["nativeRequest"];
    if agent == "opencode" {
        let options = raw["params"]["options"]
            .as_array()
            .ok_or("OpenCode 审批选项无效")?;
        let accepted = result["decision"] == "accept";
        let option = options.iter().find(|o| {
            o["kind"]
                == if accepted {
                    "allow_once"
                } else {
                    "reject_once"
                }
        });
        let outcome = option
            .map(|o| json!({"outcome":"selected","optionId":o["optionId"]}))
            .unwrap_or(json!({"outcome":"cancelled"}));
        if accepted && option.is_none() {
            return Err("该请求不支持本次允许".into());
        }
        Ok(json!({"jsonrpc":"2.0","id":raw["id"],"result":{"outcome":outcome}}))
    } else {
        let mut wire = json!({"type":"extension_ui_response","id":raw["id"]});
        if raw["method"] == "confirm" {
            wire["confirmed"] = json!(result["decision"] == "accept");
        } else if let Some(value) = result["answers"]["0"]["answers"]
            .as_array()
            .and_then(|v| v.first())
            .and_then(Value::as_str)
        {
            if raw["method"] == "select"
                && !raw["options"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|o| o == value))
            {
                return Err("请选择提供的选项".into());
            }
            wire["value"] = value.into();
        } else {
            wire["cancelled"] = true.into();
        }
        Ok(wire)
    }
}
fn user_request(t: &Turn, raw: Value) -> Option<Value> {
    let rawid = raw.get("id")?;
    let id = format!(
        "{}:{}:{}",
        t.agent,
        t.session,
        rawid
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| rawid.to_string())
    );
    if t.agent == "opencode" && raw["method"] == "session/request_permission" {
        return Some(
            json!({"id":id,"method":"claude/tool/requestApproval","params":{"threadId":t.native,"turnId":t.turn,"reason":raw["params"]["toolCall"]["title"],"input":raw["params"]["toolCall"],"nativeRequest":raw}}),
        );
    }
    if t.agent == "pi" && raw["type"] == "extension_ui_request" {
        if raw["method"] == "confirm" {
            return Some(
                json!({"id":id,"method":"claude/tool/requestApproval","params":{"threadId":t.native,"turnId":t.turn,"reason":raw["title"],"input":raw["message"],"nativeRequest":raw}}),
            );
        }
        if matches!(raw["method"].as_str(), Some("select" | "input" | "editor")) {
            let options: Vec<Value> = raw["options"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .map(|v| json!({"label":v,"description":""}))
                        .collect()
                })
                .unwrap_or_default();
            return Some(
                json!({"id":id,"method":"item/tool/requestUserInput","params":{"threadId":t.native,"turnId":t.turn,"questions":[{"id":"0","question":raw["title"],"options":options}],"nativeRequest":raw}}),
            );
        }
    }
    None
}
async fn setup_events<T>(
    app: &AppHandle,
    t: &Arc<Turn>,
    rx: &mut mpsc::Receiver<Value>,
    future: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::pin!(future);
    loop {
        tokio::select! {
            result=&mut future=>return result,
            value=rx.recv()=>{
                let value=value.ok_or("Agent 在准备会话时退出")?;
                native_capabilities(app,&t.session,&value);
                if value["type"]=="supercode_disconnected" { return Err("Agent 在准备会话时退出".into()); }
                if let Some(request)=user_request(t,value.clone()) { t.requests.lock().await.insert(request["id"].as_str().unwrap().into(),request.clone());publish(app,request); }
                else if value.get("method").is_some()&&value.get("id").is_some() { t.client.write(json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Client capability not supported"}})).await?; }
                else if value["type"]=="extension_error" { return Err(protocol::bounded(value["error"].as_str().unwrap_or("Pi 扩展加载失败"),4096)); }
                // Replay, model catalogs, and startup notifications are intentionally consumed here.
            },
            _=tokio::time::sleep(Duration::from_millis(200)),if t.stopped.load(Ordering::Relaxed)=>return Err("已停止准备会话".into()),
        }
    }
}
fn mcp(app: &AppHandle, acp: bool) -> Result<Value, String> {
    let servers = client_features::servers(app)?;
    let mut rows = vec![];
    let mut object = serde_json::Map::new();
    for s in servers.into_iter().filter(|s| s.enabled) {
        let s = crate::desktop_control::wrap(app, s, if acp { "opencode" } else { "pi" })?;
        let name = format!(
            "supercode_{}",
            s.id.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        );
        if acp {
            rows.push(mcp_entry(&s, &name, true));
        } else {
            let entry = mcp_entry(&s, &name, false);
            object.insert(name, entry);
        }
    }
    Ok(if acp {
        json!(rows)
    } else {
        Value::Object(object)
    })
}
fn mcp_entry(s: &client_features::ToolServer, name: &str, acp: bool) -> Value {
    match (acp, &s.url) {
        (true, None) => json!({"name":name,"command":s.command,"args":s.args,"env":[]}),
        (true, Some(url)) => json!({"name":name,"type":"http","url":url,"headers":[]}),
        (false, None) => json!({"command":s.command,"args":s.args,"timeout":120}),
        (false, Some(url)) => json!({"url":url,"timeout":120}),
    }
}
fn opencode_config(config: &Value, permission: &str) -> Value {
    let rule = if permission == "full" { "allow" } else { "ask" };
    let mut value = json!({"autoupdate":false,"compaction":{"auto":true},"permission":{"*":rule}});
    if permission == "read" {
        value["permission"] =
            json!({"*":"deny","read":"allow","glob":"allow","grep":"allow","list":"allow"});
    }
    if permission == "summary" {
        value["permission"] = json!({"*":"deny"});
    }
    if config["baseUrl"].is_string() {
        let npm = match config["protocol"].as_str() {
            Some("anthropic") => "@ai-sdk/anthropic",
            Some("responses") => "@ai-sdk/openai",
            _ => "@ai-sdk/openai-compatible",
        };
        let native = &config["opencodeProvider"];
        let models = crate::providers::model_ids(config)
            .into_iter()
            .map(|m| {
                let mut definition = native["models"]
                    .get(&m)
                    .filter(|v| v.is_object())
                    .cloned()
                    .unwrap_or_else(|| json!({"name":m}));
                if !definition["limit"]["context"].is_number() {
                    if let Some(capacity) = crate::model_context::for_model(config, &m) {
                        if !definition["limit"].is_object() {
                            definition["limit"] = json!({});
                        }
                        definition["limit"]["context"] = json!(capacity);
                    }
                }
                (wire_model(config, &m), definition)
            })
            .collect::<serde_json::Map<String, Value>>();
        let mut options = native["options"].as_object().cloned().unwrap_or_default();
        options.insert(
            "baseURL".into(),
            if native.is_object() {
                config["baseUrl"].clone()
            } else {
                json!(sdk_base(config))
            },
        );
        options.insert(
            "apiKey".into(),
            json!(crate::providers::key(config).unwrap_or("")),
        );
        value["provider"] = json!({"supercode":{"npm":npm,"name":native["name"].as_str().unwrap_or("SuperCode"),"options":options,"models":models}});
    }
    value
}
fn opencode_environment(config: &Value, permission: &str) -> [(String, String); 3] {
    [
        (
            "OPENCODE_CONFIG_CONTENT".into(),
            opencode_config(config, permission).to_string(),
        ),
        ("OPENCODE_DISABLE_AUTOUPDATE".into(), "true".into()),
        // A parent terminal may disable compaction. Managed chats keep native
        // automatic compaction enabled without changing global/project files.
        ("OPENCODE_DISABLE_AUTOCOMPACT".into(), "false".into()),
    ]
}
// AI SDK appends /messages; the other Anthropic SDKs append /v1/messages.
fn sdk_base(config: &Value) -> String {
    let base = config["baseUrl"]
        .as_str()
        .unwrap_or("")
        .trim_end_matches('/');
    if config["protocol"] == "anthropic" && !base.ends_with("/v1") {
        format!("{base}/v1")
    } else {
        base.to_owned()
    }
}
fn wire_model(config: &Value, id: &str) -> String {
    // Native declarations use literal model IDs, including any suffix.
    if config["opencodeProvider"].is_object() || config["piProvider"].is_object() {
        return id.to_owned();
    }
    if config["protocol"] == "anthropic" {
        for suffix in ["[1m]", "[256k]"] {
            if id.to_ascii_lowercase().ends_with(suffix) {
                return id[..id.len() - suffix.len()].to_owned();
            }
        }
    }
    id.to_owned()
}
fn pi_provider_config(config: &Value) -> Option<Value> {
    let base = config["baseUrl"].as_str()?;
    let native = &config["piProvider"];
    let mut provider = native.as_object().cloned().unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let models = crate::providers::model_ids(config)
        .into_iter()
        .filter_map(|m| {
            let id = wire_model(config, &m);
            if !seen.insert(id.clone()) {
                return None;
            }
            let input = if native.is_object() {
                json!(["text"])
            } else {
                json!(["text", "image"])
            };
            let context = crate::model_context::for_model(config, &m).unwrap_or(128000);
            let mut definition = json!({
                "id": id, "name": m, "reasoning": !native.is_object(), "input": input,
                "contextWindow": context, "maxTokens": 8192,
                "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}
            });
            if let Some(saved) = native["models"]
                .as_array()
                .and_then(|models| models.iter().find(|model| model["id"] == m))
                .and_then(Value::as_object)
            {
                let cost = definition["cost"].as_object().unwrap().clone();
                definition.as_object_mut().unwrap().extend(saved.clone());
                if let Some(saved_cost) = saved.get("cost").and_then(Value::as_object) {
                    let mut cost = cost;
                    cost.extend(saved_cost.clone());
                    definition["cost"] = Value::Object(cost);
                }
            }
            Some(definition)
        })
        .collect::<Vec<_>>();
    provider.insert("baseUrl".into(), json!(base));
    provider.insert(
        "apiKey".into(),
        json!(crate::providers::key(config).unwrap_or("")),
    );
    provider.insert(
        "api".into(),
        json!(match config["protocol"].as_str() {
            Some("anthropic") => "anthropic-messages",
            Some("responses") => "openai-responses",
            _ => "openai-completions",
        }),
    );
    provider.insert("models".into(), json!(models));
    Some(Value::Object(provider))
}
const PI_EXTENSION: &str = r#"import * as fs from 'node:fs/promises';
import * as path from 'node:path';
export default function(pi) {
  const config = JSON.parse(process.env.SUPERCODE_PI_CONFIG || '{}');
  const snapshots = new Map();
  let snapshotBytes = 0;
  const limit = 256 * 1024;
  const inside = (root, target) => { const relative=path.relative(root,target); return relative!== '..' && !relative.startsWith('..'+path.sep) && !path.isAbsolute(relative); };
  async function targetPath(cwd, input) {
    if (typeof input.path !== 'string') return null;
    const root=await fs.realpath(cwd), target=path.resolve(root,input.path);
    let ancestor=target, missing=[];
    while (true) {
      try { ancestor=await fs.realpath(ancestor); break; }
      catch (error) { if (error.code!=='ENOENT' || missing.length>=32) throw error; missing.unshift(path.basename(ancestor)); ancestor=path.dirname(ancestor); }
    }
    const resolved=path.join(ancestor,...missing);
    return inside(root,resolved) ? {absolute:resolved,path:path.relative(root,resolved).replaceAll('\\','/')} : null;
  }
  async function readSnapshot(target) {
    try {
      if ((await fs.stat(target)).size>limit) throw new Error('snapshot too large');
      const bytes=await fs.readFile(target);
      if (bytes.length>limit) throw new Error('snapshot too large');
      return new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(bytes);
    } catch(error) { if(error.code==='ENOENT') return null; throw error; }
  }
  for (const [name, server] of Object.entries(config.mcp || {})) pi.registerMcpServer(name, server);
  if (config.provider) pi.registerProvider('supercode', config.provider);
  pi.on('tool_call', async (event, ctx) => {
    if (config.permission === 'summary') return {block:true,reason:'Summary does not allow tools'};
    const desktopRead = event.toolName.includes('supercode_computer') && /(?:__|[./:])desktop_(?:list_windows|snapshot)$/.test(event.toolName);
    const read = ['read', 'grep', 'find', 'ls', 'glob'].includes(event.toolName) || desktopRead;
    if (config.permission === 'read' && !read) return {block:true,reason:'Read-only mode'};
    if (config.permission !== 'full' && (!read || config.permission === 'strict')) {
      if (!await ctx.ui.confirm('允许本次工具操作？', event.toolName + '\n' + JSON.stringify(event.input))) return {block:true,reason:'User declined'};
    }
    // Capture inside the native pre-execution hook. A fast write can finish
    // before the desktop consumes its tool_execution_start notification.
    if (['write','edit'].includes(event.toolName) && snapshots.size<64) {
      try {
        const target=await targetPath(ctx.cwd,event.input);
        if (!target) return;
        const before=await readSnapshot(target.absolute), bytes=Buffer.byteLength(before||'','utf8');
        if (snapshotBytes+bytes>2*1024*1024) return;
        snapshots.set(event.toolCallId,{...target,before,bytes}); snapshotBytes+=bytes;
      } catch { /* Keep native execution and the desktop fallback available. */ }
    }
  });
  pi.on('tool_result', async event => {
    const snapshot=snapshots.get(event.toolCallId);
    if (!snapshot) return;
    snapshots.delete(event.toolCallId); snapshotBytes-=snapshot.bytes;
    try {
      const after=await readSnapshot(snapshot.absolute);
      return {details:{...(event.details||{}),supercodeFileChange:{path:snapshot.path,before:snapshot.before,after}}};
    } catch { /* Large/binary files retain their original native result. */ }
  });
  pi.on('agent_end', () => { snapshots.clear(); snapshotBytes=0; });
}"#;
async fn start_for(
    app: &AppHandle,
    agent: &str,
    cwd: &Path,
    session: Option<&Session>,
    permission: &str,
) -> Result<(Arc<Client>, mpsc::Receiver<Value>, Value), String> {
    let launch = crate::agents::resolve(app, agent)?;
    let config = app
        .state::<AppState>()
        .store
        .route(agent, session.map(|s| s.id.as_str()))?
        .config;
    start_configured(app, agent, cwd, session, permission, config, &launch).await
}
async fn start_configured(
    app: &AppHandle,
    agent: &str,
    cwd: &Path,
    session: Option<&Session>,
    permission: &str,
    config: Value,
    launch: &Launch,
) -> Result<(Arc<Client>, mpsc::Receiver<Value>, Value), String> {
    let mut env = vec![];
    if let Some(session) = session {
        env.push(("SUPERCODE_SESSION_ID".into(), session.id.clone()));
    }
    let mut args = vec![];
    if agent == "opencode" {
        args.push("acp".into());
        env.extend(opencode_environment(&config, permission));
    } else if agent == "pi" {
        let dir = app.state::<AppState>().data_dir.join("agents/pi-runtime");
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| e.to_string())?;
        let extension = crate::agents::ordinary(dir.join("supercode.mjs"));
        tokio::fs::write(&extension, PI_EXTENSION.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        args.extend([
            "--mode".into(),
            "rpc".into(),
            "--extension".into(),
            extension.to_string_lossy().into(),
            "--append-system-prompt".into(),
            client_features::instructions(app),
        ]);
        if permission == "summary" {
            args.extend([
                "--no-extensions".into(),
                "--no-skills".into(),
                "--no-mcp".into(),
            ]);
        }
        if let Some(s) = session {
            args.extend([
                "--session-id".into(),
                s.native_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                "--session-dir".into(),
                crate::agents::ordinary(dir.join("sessions"))
                    .to_string_lossy()
                    .into(),
            ]);
        } else {
            args.push("--no-session".into());
        }
        let mut settings = json!({"permission":permission,"mcp":if permission=="summary"{json!({})}else{mcp(app,false)?}});
        if let Some(provider) = pi_provider_config(&config) {
            settings["provider"] = provider;
        }
        env.push(("SUPERCODE_PI_CONFIG".into(), settings.to_string()));
    } else if agent == "codex" {
        args.extend(["app-server".into(), "--listen".into(), "stdio://".into()]);
        let mut overrides = crate::ccswitch::codex_overrides(&config)?;
        overrides.extend([
            "mcp_servers={}".into(),
            "features.shell_tool=false".into(),
            "web_search=\"disabled\"".into(),
        ]);
        for value in overrides {
            args.extend(["-c".into(), value]);
        }
        if let Some(key) = crate::providers::key(&config) {
            env.push(("SUPERCODE_PROVIDER_API_KEY".into(), key.to_owned()));
        }
    } else {
        return Err("连接测试不支持此引擎".into());
    }
    crate::runtime_diagnostics::register_secrets(app, &config);
    let (client, events) = Client::start(
        launch,
        &args,
        cwd,
        &env,
        agent == "pi",
        Some((app.clone(), agent.to_owned(), session.map(|s| s.id.clone()))),
    )
    .await?;
    Ok((client, events, config))
}
// Test an explicitly selected profile in a separate, tool-free process. Never
// replace a chat's runtime or activate the tested profile as the default.
pub async fn test_provider_connection(
    app: &AppHandle,
    profile: &crate::ccswitch::Profile,
    launch: &Launch,
) -> Result<Value, String> {
    let model =
        crate::providers::real_claude_model(&profile.config, None).ok_or("请填写模型 ID")?;
    let cwd = app
        .state::<AppState>()
        .data_dir
        .join("agents/connection-tests");
    tokio::fs::create_dir_all(&cwd)
        .await
        .map_err(|_| "无法创建连接测试目录")?;
    let (client, mut events, config) = start_configured(
        app,
        &profile.agent,
        &cwd,
        None,
        "summary",
        profile.config.clone(),
        launch,
    )
    .await?;
    let result = tokio::time::timeout(
        Duration::from_secs(90),
        probe_completion(&client, &mut events, &profile.agent, &config, &model, &cwd),
    )
    .await;
    client.close().await;
    let text = result
        .map_err(|_| format!("{} 套餐连接测试超时", crate::agents::name(&profile.agent)))??;
    if text.trim().is_empty() {
        return Err("模型没有返回测试回复，请检查模型权限和套餐状态".into());
    }
    Ok(
        json!({"ok":true,"model":model,"protocol":config["protocol"],"verifiedBy":crate::agents::name(&profile.agent)}),
    )
}
async fn probe_request(
    client: &Client,
    events: &mut mpsc::Receiver<Value>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let request = client.request(method, params, 60);
    tokio::pin!(request);
    loop {
        tokio::select! {
            result = &mut request => return result,
            event = events.recv() => probe_notification(&event.ok_or("连接测试进程已退出")?)?,
        }
    }
}
fn probe_notification(event: &Value) -> Result<(), String> {
    if event["type"] == "supercode_disconnected" {
        return Err("连接测试进程已退出，请检查引擎安装和运行日志".into());
    }
    if event["type"] == "extension_error" {
        return Err(protocol::bounded(
            event["error"].as_str().unwrap_or("Pi 扩展加载失败"),
            4096,
        ));
    }
    if event.get("id").is_some()
        && (event.get("method").is_some() || event["type"] == "extension_ui_request")
    {
        return Err("连接测试收到审批请求，测试已停止；请在对话中处理该请求".into());
    }
    Ok(())
}
async fn probe_completion(
    client: &Client,
    events: &mut mpsc::Receiver<Value>,
    agent: &str,
    config: &Value,
    model: &str,
    cwd: &Path,
) -> Result<String, String> {
    let wire = wire_model(config, model);
    let native = if agent == "pi" {
        probe_request(client, events, "get_state", json!({})).await?;
        probe_request(
            client,
            events,
            "set_model",
            json!({"provider":"supercode","modelId":wire}),
        )
        .await?;
        String::new()
    } else if agent == "opencode" {
        probe_request(client, events, "initialize", init()).await?;
        let session = probe_request(
            client,
            events,
            "session/new",
            json!({"cwd":cwd,"mcpServers":[]}),
        )
        .await?;
        let id = session["sessionId"]
            .as_str()
            .ok_or("OpenCode 未返回测试会话 ID")?
            .to_owned();
        probe_request(
            client,
            events,
            "session/set_model",
            json!({"sessionId":id,"modelId":format!("supercode/{wire}")}),
        )
        .await?;
        id
    } else {
        probe_request(client, events, "initialize", json!({"clientInfo":{"name":"supercode","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        client
            .write(json!({"method":"initialized","params":{}}))
            .await?;
        let table = toml::from_str::<toml::Table>(config["config"].as_str().unwrap_or(""))
            .map_err(|_| "Codex 连接配置无效")?;
        let provider = table.get("model_provider").and_then(toml::Value::as_str);
        let thread = probe_request(client, events, "thread/start", json!({"cwd":cwd,"model":model,"modelProvider":provider,"sandbox":"read-only","approvalPolicy":"untrusted","ephemeral":true,"developerInstructions":"Reply only OK. Do not use tools.","config":{"features.shell_tool":false,"web_search":"disabled"}})).await?;
        thread["thread"]["id"]
            .as_str()
            .ok_or("Codex 未返回测试会话 ID")?
            .to_owned()
    };
    let prompt = "Reply only OK. Do not use tools.";
    let (method, params) = match agent {
        "pi" => ("prompt", json!({"message":prompt,"images":[]})),
        "opencode" => (
            "session/prompt",
            json!({"sessionId":native,"prompt":[{"type":"text","text":prompt}]}),
        ),
        _ => (
            "turn/start",
            json!({"threadId":native,"model":model,"input":[{"type":"text","text":prompt}]}),
        ),
    };
    let request = client.request(method, params, 90);
    tokio::pin!(request);
    let mut acknowledged = false;
    let mut text = String::new();
    let mut failure = None;
    loop {
        tokio::select! {
            result = &mut request, if !acknowledged => {
                let result = result?;
                if agent == "opencode" {
                    if result["stopReason"] == "cancelled" { return Err("OpenCode 取消了连接测试".into()); }
                    break;
                }
                acknowledged = true;
            },
            event = events.recv() => {
                let event = event.ok_or("连接测试进程已退出")?;
                probe_notification(&event)?;
                if probe_output(&event, agent, &native, &mut text, &mut failure)? { break; }
            },
        }
    }
    while let Ok(event) = events.try_recv() {
        probe_notification(&event)?;
        probe_output(&event, agent, &native, &mut text, &mut failure)?;
    }
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(text)
}
fn probe_output(
    event: &Value,
    agent: &str,
    native: &str,
    text: &mut String,
    failure: &mut Option<String>,
) -> Result<bool, String> {
    if agent == "pi" {
        if event["type"] == "tool_execution_start" {
            return Err("连接测试尝试调用工具，已停止测试".into());
        }
        if event["type"] == "message_update"
            && event["assistantMessageEvent"]["type"] == "text_delta"
        {
            append_probe_text(
                text,
                event["assistantMessageEvent"]["delta"]
                    .as_str()
                    .unwrap_or(""),
            );
        }
        if event["type"] == "message_end" && event["message"]["role"] == "assistant" {
            let message = &event["message"];
            if matches!(message["stopReason"].as_str(), Some("error" | "aborted")) {
                *failure = Some(protocol::bounded(
                    message["errorMessage"]
                        .as_str()
                        .unwrap_or("Pi 模型请求失败"),
                    4096,
                ));
            }
            if text.is_empty() {
                if let Some(parts) = message["content"].as_array() {
                    for part in parts {
                        if part["type"] == "text" {
                            append_probe_text(text, part["text"].as_str().unwrap_or(""));
                        }
                    }
                }
            }
        }
        return Ok(event["type"] == "agent_settled");
    }
    if agent == "opencode" {
        if event["params"]["sessionId"] != native {
            return Ok(false);
        }
        let update = &event["params"]["update"];
        if matches!(
            update["sessionUpdate"].as_str(),
            Some("tool_call" | "tool_call_update")
        ) {
            return Err("连接测试尝试调用工具，已停止测试".into());
        }
        if update["sessionUpdate"] == "agent_message_chunk" && update["content"]["type"] == "text" {
            append_probe_text(text, update["content"]["text"].as_str().unwrap_or(""));
        }
        return Ok(false);
    }
    if event["params"]["threadId"] != native {
        return Ok(false);
    }
    if event["method"] == "error" {
        return Err(protocol::bounded(
            event["params"]["error"]["message"]
                .as_str()
                .unwrap_or("Codex 模型请求失败"),
            4096,
        ));
    }
    if matches!(
        event["method"].as_str(),
        Some("item/started" | "item/completed")
    ) {
        let item = &event["params"]["item"];
        match item["type"].as_str() {
            Some("agentMessage") if event["method"] == "item/completed" => {
                append_probe_text(text, item["text"].as_str().unwrap_or(""))
            }
            Some("userMessage" | "agentMessage" | "reasoning" | "contextCompaction") => {}
            Some(_) => return Err("连接测试尝试调用工具，已停止测试".into()),
            _ => {}
        }
    }
    if event["method"] == "turn/completed" {
        let turn = &event["params"]["turn"];
        if turn["status"] != "completed" {
            *failure = Some(protocol::bounded(
                turn["error"]["message"]
                    .as_str()
                    .unwrap_or("Codex 模型请求失败"),
                4096,
            ));
        }
        return Ok(true);
    }
    Ok(false)
}
fn append_probe_text(text: &mut String, value: &str) {
    let mut end = value.len().min(8192usize.saturating_sub(text.len()));
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    text.push_str(&value[..end]);
}
fn warm_identity_matches(session: &Session, cached_session: &str, cached_native: &str) -> bool {
    // Model handoff and compaction can clear the persisted native binding while
    // the supplier, model and permission signature remain unchanged.
    session.id == cached_session && session.native_id.as_deref() == Some(cached_native)
}
pub async fn send(
    app: &AppHandle,
    s: &Session,
    cwd: &str,
    text: &str,
    model: Option<String>,
    permission: &str,
    content: Vec<Value>,
    metadata: Value,
    effort: Option<String>,
) -> Result<Session, String> {
    let state = app.state::<AppState>();
    let session_runtime = state.native.for_session(&s.id);
    let mut slot = session_runtime.client.lock().await;
    if slot.is_some() {
        return Err("上一任务尚未结束".into());
    }
    let route = state.store.route(&s.agent, Some(&s.id))?;
    let signature = json!([
        route.fingerprint(),
        client_features::tool_configuration(app)?,
        s.id,
        cwd,
        model.as_ref().or(s.model.as_ref()),
        permission,
        effort
    ])
    .to_string();
    let cached = session_runtime.warm.lock().await.take();
    let cached = if let Some(c) = cached {
        if warm_identity_matches(s, &c.session, &c.native)
            && c.signature == signature
            && c.client.alive.load(Ordering::Acquire)
        {
            Some(c)
        } else {
            c.client.close().await;
            None
        }
    } else {
        None
    };
    let (client, mut rx, config, resumed) = if let Some(c) = cached {
        (c.client, c.rx, c.config, Some((c.native, c.defaults)))
    } else {
        let (c, r, config) = start_for(app, &s.agent, Path::new(cwd), Some(s), permission).await?;
        (c, r, config, None)
    };
    let boot = Arc::new(Turn {
        client: client.clone(),
        requests: Arc::new(Mutex::new(HashMap::new())),
        session: s.id.clone(),
        native: s.native_id.clone().unwrap_or_else(|| s.id.clone()),
        turn: uuid::Uuid::new_v4().to_string(),
        agent: s.agent.clone(),
        stopped: AtomicBool::new(false),
        accepting: AtomicBool::new(false),
        inputs_pending: AtomicU64::new(0),
        input_done: Notify::new(),
        settled: AtomicBool::new(false),
        prompt_accepted: AtomicBool::new(false),
        prompt_ready: Notify::new(),
        started: AtomicBool::new(false),
        stop_done: AtomicBool::new(true),
        signature: signature.clone(),
    });
    let begin = (|| {
        state.store.bind_native(&s.id, &boot.native)?;
        state.store.set_status(&s.id, "starting", Some(&boot.turn))
    })();
    if let Err(error) = begin {
        client.close().await;
        let _ = state
            .store
            .restore_native_binding(&s.id, s.native_id.as_deref());
        return Err(error);
    }
    *slot = Some(boot.clone());
    drop(slot);
    publish(
        app,
        event(
            &boot,
            "thread/started",
            json!({"thread":{"id":boot.native}}),
        ),
    );
    let startup = if let Some(ready) = resumed {
        Ok(ready)
    } else {
        setup_events(app, &boot, &mut rx, async {
            if s.agent == "pi" {
                let v = client.request("get_state", json!({}), 60).await?;
                let native = v["sessionId"]
                    .as_str()
                    .ok_or("Pi 未返回会话 ID")?
                    .to_owned();
                Ok((native, v))
            } else {
                let init = client.request("initialize", init(), 60).await?;
                client.concurrent_prompt.store(
                    verified_concurrent_prompt(init["agentInfo"]["version"].as_str()),
                    Ordering::Release,
                );
                let mut p = json!({"cwd":cwd,"mcpServers":mcp(app,true)?});
                let method = if let Some(native) = &s.native_id {
                    if init["agentCapabilities"]["loadSession"] != true {
                        return Err("本机 OpenCode 不支持恢复会话，请更新".into());
                    }
                    p["sessionId"] = native.clone().into();
                    "session/load"
                } else {
                    "session/new"
                };
                let v = client.request(method, p, 120).await?;
                let native = s
                    .native_id
                    .clone()
                    .or_else(|| v["sessionId"].as_str().map(str::to_owned))
                    .ok_or("OpenCode 未返回会话 ID")?;
                Ok((native, v))
            }
        })
        .await
    };
    let (native, defaults) = match startup {
        Ok(v) => v,
        Err(e) => {
            client.close().await;
            session_runtime.client.lock().await.take();
            publish(
                app,
                event(
                    &boot,
                    "turn/completed",
                    json!({"turn":completion_payload(&boot.turn,boot.stopped.load(Ordering::Acquire),false,Some(e.clone()))}),
                ),
            );
            state
                .store
                .restore_native_binding(&s.id, s.native_id.as_deref())?;
            return Err(e);
        }
    };
    // setup_events consumes load replay without re-persisting it. Leave remaining
    // notifications queued so late startup questions and approvals cannot be lost.
    let selected = model
        .or(s.model.clone())
        .or_else(|| config["model"].as_str().map(str::to_owned))
        .or_else(|| {
            if s.agent == "pi" {
                Some(format!(
                    "{}/{}",
                    defaults["model"]["provider"].as_str()?,
                    defaults["model"]["id"].as_str()?
                ))
            } else {
                defaults["models"]["currentModelId"]
                    .as_str()
                    .map(str::to_owned)
            }
        });
    let selected_wire = selected.as_ref().map(|m| {
        if config["baseUrl"].is_string() {
            format!("supercode/{}", wire_model(&config, m))
        } else {
            m.clone()
        }
    });
    let configure = setup_events(app, &boot, &mut rx, async {
        if let Some(model) = &selected_wire {
            if s.agent == "pi" {
                let (provider, id) = model
                    .split_once('/')
                    .ok_or("Pi 模型需包含供应商 ID，请从模型列表选择")?;
                client
                    .request("set_model", json!({"provider":provider,"modelId":id}), 30)
                    .await?;
            } else {
                client
                    .request(
                        "session/set_model",
                        json!({"sessionId":native,"modelId":model}),
                        30,
                    )
                    .await?;
            }
        }
        if s.agent == "pi" {
            let commands = client.request("get_commands", json!({}), 30).await?;
            native_capabilities(app, &s.id, &json!({"commands":commands["commands"]}));
            if let Some(effort) = effort.filter(|e| {
                matches!(
                    e.as_str(),
                    "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
                )
            }) {
                client
                    .request("set_thinking_level", json!({"level":effort}), 30)
                    .await?;
            }
        }
        Ok::<_, String>(())
    })
    .await;
    if let Err(e) = configure {
        client.close().await;
        session_runtime.client.lock().await.take();
        publish(
            app,
            event(
                &boot,
                "turn/completed",
                json!({"turn":completion_payload(&boot.turn,boot.stopped.load(Ordering::Acquire),false,Some(e.clone()))}),
            ),
        );
        state
            .store
            .restore_native_binding(&s.id, s.native_id.as_deref())?;
        return Err(e);
    }
    let state = app.state::<AppState>();
    state.store.bind_native(&s.id, &native)?;
    state.store.set_model(&s.id, selected.as_deref())?;
    state.store.save_message(
        &crate::client_features::user_message_id(&metadata),
        &s.id,
        "user",
        text,
        "userMessage",
        &metadata,
    )?;
    if s.title == "新会话" {
        state
            .store
            .rename(&s.id, &text.chars().take(24).collect::<String>())?;
    }
    let turn = Arc::new(boot.after_setup(native));
    let instructions = client_features::instructions(app);
    let prompt = prompt_content(&s.agent, &content, &instructions);
    let params = if turn.agent == "pi" {
        prompt
    } else {
        json!({"sessionId":turn.native,"prompt":prompt})
    };
    let method = if turn.agent == "pi" {
        "prompt"
    } else {
        "session/prompt"
    };
    let mut slot = session_runtime.client.lock().await;
    if boot.stopped.load(Ordering::Acquire) {
        client.close().await;
        slot.take();
        publish(
            app,
            event(
                &boot,
                "turn/completed",
                json!({"turn":completion_payload(&boot.turn,true,false,None)}),
            ),
        );
        return state.store.session(&s.id);
    }
    // Write before exposing the active turn. Stop/steer must always follow the
    // original prompt on stdin, including immediately after send() returns.
    let (prompt_id, mut done) = match client.begin_request(method, params).await {
        Ok(v) => v,
        Err(e) => {
            client.close().await;
            slot.take();
            publish(
                app,
                event(
                    &boot,
                    "turn/completed",
                    json!({"turn":completion_payload(&boot.turn,false,false,Some(e.clone()))}),
                ),
            );
            return Err(e);
        }
    };
    let mut pending_requests = turn.requests.lock().await;
    let rebound = rebind_public_requests(
        &mut pending_requests,
        &boot.native,
        &turn.native,
        &turn.session,
    );
    *slot = Some(turn.clone());
    drop(slot);
    publish(
        app,
        event(
            &turn,
            "turn/started",
            json!({"turn":{"id":turn.turn,"status":"running"}}),
        ),
    );
    // Re-emit the same IDs after the start notification so restored cards stay
    // visible and unresolved startup requests keep the chat in waiting state.
    for request in rebound {
        publish(app, request);
    }
    drop(pending_requests);
    let model_context_window = if turn.agent == "pi" {
        selected
            .as_deref()
            .and_then(|model| crate::model_context::for_model(&config, model))
            .or_else(|| {
                defaults["model"]["id"]
                    .as_str()
                    .filter(|id| {
                        selected
                            .as_deref()
                            .is_none_or(|model| model.rsplit('/').next() == Some(*id))
                    })
                    .and_then(|_| {
                        crate::model_context::positive(&defaults["model"]["contextWindow"])
                    })
            })
            .or_else(|| config["baseUrl"].is_string().then_some(128000))
    } else {
        None
    };
    let app = app.clone();
    tokio::spawn(async move {
        let mut mapped = Mapper {
            priced: !config["baseUrl"].is_string(),
            model_context_window,
            ..Mapper::default()
        };
        let mut completed = false;
        let mut protocol_finished = false;
        let mut drained = false;
        let mut failure = None;
        let prompt_timeout = tokio::time::sleep(Duration::from_secs(3600));
        tokio::pin!(prompt_timeout);
        let mut checkpoint = tokio::time::interval(Duration::from_secs(1));
        checkpoint.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _=checkpoint.tick()=>{for e in mapped.checkpoint(&turn){publish(&app,e);}},
                value=rx.recv()=>{let Some(value)=value else{failure=Some("Agent 输出连接已关闭".into());break;};
                    if value["type"]=="supercode_disconnected"{failure=Some("Agent 意外退出".into());break;}
                    if value["type"]=="agent_start" || (value["method"]=="session/update" && value["params"]["sessionId"]==turn.native && matches!(value["params"]["update"]["sessionUpdate"].as_str(),Some("agent_message_chunk"|"agent_thought_chunk"|"tool_call"))) {turn.started.store(true,Ordering::Release);}
                    if let Some(request)=user_request(&turn,value.clone()){turn.requests.lock().await.insert(request["id"].as_str().unwrap().into(),request.clone());publish(&app,request);continue;}
                    if turn.agent=="opencode"&&value.get("method").is_some()&&value.get("id").is_some(){let _=client.write(json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Client capability not supported"}})).await;continue;}
                    for e in mapped.map(&turn,&value){if e["method"]!="error" || !turn.stopped.load(Ordering::Relaxed){publish(&app,e);}}
                    native_capabilities(&app,&turn.session,&value);
                    if turn.agent=="pi" {
                        if value["type"]=="agent_start"{turn.settled.store(false,Ordering::Release);}
                        if value["type"]=="agent_settled"{turn.settled.store(true,Ordering::Release);}
                    }
                },
                result=&mut done,if !completed=>{turn.prompt_accepted.store(true,Ordering::Release);turn.prompt_ready.notify_one();match result{Ok(Ok(v))=>{if turn.agent=="opencode"{if v["stopReason"]=="cancelled"{turn.stopped.store(true,Ordering::Relaxed);}protocol_finished=true;}else if v["disposition"]=="handled"{turn.settled.store(true,Ordering::Release);}}Ok(Err(e))=>{failure=Some(e);break;}Err(_)=>{failure=Some("Agent 请求中断".into());break;}}completed=true;},
                _=&mut prompt_timeout,if !completed=>{failure=Some("Agent 原生 prompt 请求超时".into());break;},
                _=turn.input_done.notified()=>{},
                _=tokio::time::sleep(Duration::from_secs(5)),if turn.stopped.load(Ordering::Relaxed)=>{break;}
            }
            if completed
                && turn.inputs_pending.load(Ordering::Acquire) == 0
                && turn.stop_done.load(Ordering::Acquire)
                && (protocol_finished || turn.settled.load(Ordering::Acquire))
            {
                let slot = session_runtime.client.lock().await;
                turn.accepting.store(false, Ordering::Release);
                if turn.agent == "pi" {
                    let state_request = client.request("get_state", json!({}), 30);
                    tokio::pin!(state_request);
                    let native_state = loop {
                        tokio::select! {
                            _=checkpoint.tick()=>{for e in mapped.checkpoint(&turn){publish(&app,e);}},
                            result=&mut state_request=>break result,
                            value=rx.recv()=>{let Some(value)=value else{break Err("Pi 连接已关闭".into());};
                                if value["type"]=="supercode_disconnected"{break Err("Pi 连接已关闭".into());}
                                if let Some(request)=user_request(&turn,value.clone()){turn.requests.lock().await.insert(request["id"].as_str().unwrap().into(),request.clone());publish(&app,request);}
                                for e in mapped.map(&turn,&value){publish(&app,e);}
                            }
                        }
                    };
                    match native_state {
                        Ok(v)
                            if v["isStreaming"] == true
                                || v["isCompacting"] == true
                                || v["pendingMessageCount"].as_u64().unwrap_or(0) > 0 =>
                        {
                            turn.settled.store(false, Ordering::Release);
                            turn.accepting
                                .store(!turn.stopped.load(Ordering::Acquire), Ordering::Release);
                            drop(slot);
                            continue;
                        }
                        Err(e) => {
                            failure = Some(e);
                            drop(slot);
                            break;
                        }
                        _ => {}
                    }
                }
                drained = true;
                drop(slot);
                break;
            }
        }
        // Ensure queued chunks preceding the prompt response are flushed before final status.
        while let Ok(v) = rx.try_recv() {
            for e in mapped.map(&turn, &v) {
                if e["method"] != "error" || !turn.stopped.load(Ordering::Relaxed) {
                    publish(&app, e);
                }
            }
        }
        for item in mapped.finish(turn.stopped.load(Ordering::Relaxed) || failure.is_some()) {
            publish(&app, event(&turn, "item/completed", json!({"item":item})));
        }
        client.pending.lock().await.remove(&prompt_id);
        turn.requests.lock().await.clear();
        let completion = completion_payload(
            &turn.turn,
            turn.stopped.load(Ordering::Relaxed),
            mapped.failed,
            failure.or(mapped.failure),
        );
        let mut slot = session_runtime.client.lock().await;
        if slot.as_ref().is_some_and(|v| Arc::ptr_eq(v, &turn)) {
            slot.take();
            if drained
                && matches!(
                    completion["status"].as_str(),
                    Some("completed" | "interrupted")
                )
                && client.alive.load(Ordering::Acquire)
            {
                *session_runtime.warm.lock().await = Some(Warm {
                    client: client.clone(),
                    rx,
                    native: turn.native.clone(),
                    session: turn.session.clone(),
                    config,
                    defaults,
                    signature,
                    used: crate::storage::now(),
                });
            } else {
                client.close().await;
            }
        }
        publish(
            &app,
            event(&turn, "turn/completed", json!({"turn":completion})),
        );
        drop(slot);
        let _ = app.emit("workspace-updated", ());
    });
    state.store.session(&s.id)
}
fn completion_payload(id: &str, interrupted: bool, failed: bool, error: Option<String>) -> Value {
    let status = if interrupted {
        "interrupted"
    } else if failed || error.is_some() {
        "failed"
    } else {
        "completed"
    };
    json!({"id":id,"status":status,"error":if interrupted {None} else {error}.map(|message|json!({"message":message}))})
}
// A fresh native session generates a portable handoff; no original history/binding is changed.
pub async fn summary(app: &AppHandle, session: &Session, input: &str) -> Result<String, String> {
    let mut fresh = session.clone();
    fresh.native_id = None;
    let cwd = app
        .state::<AppState>()
        .store
        .session_workspace(&session)?
        .path;
    let (client, mut rx, config) = start_for(
        app,
        &session.agent,
        Path::new(&cwd),
        Some(&fresh),
        "summary",
    )
    .await?;
    let result=tokio::time::timeout(Duration::from_secs(120),async {
        let native=if session.agent=="pi" {
            summary_request(&client,&mut rx,"get_state",json!({})).await?["sessionId"].as_str().ok_or("Pi 未返回会话 ID")?.to_owned()
        } else {
            summary_request(&client,&mut rx,"initialize",init()).await?;
            summary_request(&client,&mut rx,"session/new",json!({"cwd":cwd,"mcpServers":[]})).await?["sessionId"].as_str().ok_or("OpenCode 未返回会话 ID")?.to_owned()
        };
        if let Some(model)=session.model.as_deref() {
            let model=if config["baseUrl"].is_string(){format!("supercode/{}",wire_model(&config,model))}else{model.to_owned()};
            if session.agent=="pi" { let(p,id)=model.split_once('/').ok_or("Pi 模型缺少供应商")?;summary_request(&client,&mut rx,"set_model",json!({"provider":p,"modelId":id})).await?; }
            else { summary_request(&client,&mut rx,"session/set_model",json!({"sessionId":native,"modelId":model})).await?; }
        }
        let t=Turn { client:client.clone(),requests:Arc::new(Mutex::new(HashMap::new())),session:session.id.clone(),native,turn:format!("handoff-{}",uuid::Uuid::new_v4()),agent:session.agent.clone(),stopped:AtomicBool::new(false),accepting:AtomicBool::new(false),inputs_pending:AtomicU64::new(0),input_done:Notify::new(),settled:AtomicBool::new(false),prompt_accepted:AtomicBool::new(false),prompt_ready:Notify::new(),started:AtomicBool::new(false),stop_done:AtomicBool::new(true),signature:String::new() };
        let mut mapper=Mapper{priced:!config["baseUrl"].is_string(),..Mapper::default()};
        let mut texts=std::collections::BTreeMap::new();
        let prompt=prompt_content(&session.agent,&[json!({"type":"text","text":input})],"");
        let method=if session.agent=="pi"{"prompt"}else{"session/prompt"};
        let params=if session.agent=="pi"{prompt}else{json!({"sessionId":t.native,"prompt":prompt})};
        let request=client.request(method,params,120);tokio::pin!(request);let mut acknowledged=false;
        loop { tokio::select! {
            value=rx.recv()=> {let value=value.ok_or("压缩连接已关闭")?;summary_notification(&value)?;
                for e in mapper.map(&t,&value) {let item=&e["params"]["item"];if item["type"]=="claudeToolCall" { return Err("压缩尝试调用工具，切换已取消".into()); }if item["type"]=="agentMessage" {if let (Some(id),Some(text))=(item["id"].as_str(),item["text"].as_str()){texts.insert(id.to_owned(),protocol::bounded(text,24*1024));}} }
                if value["type"]=="agent_settled" {break;}
            },
            value=&mut request,if !acknowledged=>{value?;if session.agent=="opencode"{break;}acknowledged=true;}
        }}
        while let Ok(value)=rx.try_recv(){summary_notification(&value)?;for e in mapper.map(&t,&value){let i=&e["params"]["item"];if i["type"]=="agentMessage"{if let(Some(id),Some(text))=(i["id"].as_str(),i["text"].as_str()){texts.insert(id.to_owned(),protocol::bounded(text,24*1024));}}}}
        if mapper.failed {return Err(mapper.failure.unwrap_or("模型未完成压缩".into()));}
        if mapper.usage.is_object(){let mut usage=mapper.usage;usage["purpose"]="handoff".into();app.state::<AppState>().store.record_usage(session,&t.turn,usage)?;}
        Ok(texts.into_values().collect::<Vec<_>>().join("\n"))
    }).await.map_err(|_|"上下文压缩超时".to_owned());
    client.close().await;
    result?
}
fn summary_notification(value: &Value) -> Result<(), String> {
    if value["type"] == "supercode_disconnected" {
        return Err("压缩进程退出".into());
    }
    if value["type"] == "extension_error" {
        return Err(protocol::bounded(
            value["error"].as_str().unwrap_or("Pi 扩展加载失败"),
            4096,
        ));
    }
    if value.get("id").is_some()
        && (value.get("method").is_some() || value["type"] == "extension_ui_request")
    {
        return Err("压缩请求需要审批，切换已取消".into());
    }
    Ok(())
}
async fn summary_request(
    client: &Client,
    rx: &mut mpsc::Receiver<Value>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let request = client.request(method, params, 60);
    tokio::pin!(request);
    loop {
        tokio::select! {result=&mut request=>return result,value=rx.recv()=>summary_notification(&value.ok_or("压缩连接关闭")?)?}
    }
}
fn native_capabilities(app: &AppHandle, session: &str, value: &Value) {
    let commands = value.get("commands").or_else(|| {
        let u = &value["params"]["update"];
        if u["sessionUpdate"] == "available_commands_update" {
            u.get("availableCommands")
        } else {
            None
        }
    });
    if let Some(commands) = commands.and_then(Value::as_array) {
        let names = commands
            .iter()
            .filter_map(|c| c["name"].as_str())
            .filter(|n| n.len() <= 200 && !n.chars().any(char::is_control))
            .take(200)
            .collect::<Vec<_>>();
        let _ = app.state::<AppState>().store.save_message(
            "agent-capabilities",
            session,
            "system",
            "",
            "agentCapabilities",
            &json!({"commands":names}),
        );
    }
}
fn prompt_content(agent: &str, content: &[Value], instructions: &str) -> Value {
    let text = content
        .iter()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    if agent == "pi" {
        let images=content.iter().filter(|c|c["type"]=="image").map(|c|json!({"type":"image","data":c["source"]["data"],"mimeType":c["source"]["media_type"]})).collect::<Vec<_>>();
        json!({"message":text,"images":images})
    } else {
        let mut blocks = vec![json!({"type":"text","text":format!("{instructions}\n\n{text}")})];
        for c in content.iter().filter(|c| c["type"] == "image") {
            blocks.push(json!({"type":"image","data":c["source"]["data"],"mimeType":c["source"]["media_type"]}));
        }
        json!(blocks)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleared_or_replaced_native_checkpoint_does_not_resume_old_context() {
        let mut session = Session {
            id: "chat".into(),
            project_id: String::new(),
            workspace_path: None,
            title: "Test".into(),
            agent: "pi".into(),
            model: Some("glm-5.3".into()),
            connection_id: Some("glm".into()),
            native_id: Some("native-before".into()),
            status: "idle".into(),
            updated_at: 0,
            turn_id: None,
        };
        assert!(warm_identity_matches(&session, "chat", "native-before"));
        assert!(!warm_identity_matches(
            &session,
            "other-chat",
            "native-before"
        ));
        session.native_id = None;
        assert!(!warm_identity_matches(&session, "chat", "native-before"));
        session.native_id = Some("native-after".into());
        assert!(!warm_identity_matches(&session, "chat", "native-before"));
        assert!(warm_identity_matches(&session, "chat", "native-after"));
    }
    #[test]
    fn acp_task_identity_survives_native_title_updates_and_completion() {
        let start = json!({"kind":"other","title":"task","rawInput":{"subagent_type":"explore","prompt":"Read fixture files","description":"kimi-opencode-a"}});
        let mut row = json!({"tool":acp_tool(&start),"arguments":start["rawInput"]});
        assert_eq!(row["tool"], "Task");
        update_acp_tool(
            &mut row,
            &json!({"kind":"other","title":"kimi-opencode-a","status":"in_progress"}),
        );
        update_acp_tool(
            &mut row,
            &json!({"kind":"other","title":"kimi-opencode-a","status":"completed"}),
        );
        assert_eq!(row["tool"], "Task");
        assert_eq!(row["arguments"]["prompt"], "Read fixture files");
        let mut generic = json!({"tool":"Tool"});
        update_acp_tool(
            &mut generic,
            &json!({"kind":"read","title":"Read README.md"}),
        );
        assert_eq!(generic["tool"], "Read");
        assert_eq!(
            acp_tool_status(
                &json!({"status":"failed","content":[{"type":"content","content":{"type":"text","text":"Task cancelled"}}]})
            ),
            Some("interrupted")
        );
        assert_eq!(
            acp_tool_status(
                &json!({"status":"failed","content":[{"type":"content","content":{"type":"text","text":"File does not exist"}}]})
            ),
            Some("failed")
        );
    }
    #[tokio::test]
    async fn provider_probes_use_the_configured_model_and_wait_for_actual_completion() {
        let Some(program) = process::find_program("node") else {
            return;
        };
        // Controlled protocol peers exercise adapters without real provider calls.
        let script = r#"const rl=require('readline').createInterface({input:process.stdin});
const out=v=>process.stdout.write(JSON.stringify(v)+'\n');
rl.on('line',line=>{const v=JSON.parse(line),m=v.method||v.type;
const reply=data=>out(v.type?{type:'response',id:v.id,success:true,data}:{jsonrpc:'2.0',id:v.id,result:data});
if(m==='initialized')return;
if(m==='get_state')return reply({sessionId:'test'});
if(m==='initialize')return reply({protocolVersion:1});
if(m==='session/new')return reply({sessionId:'test'});
if(m==='thread/start'){if(v.params.model!=='glm-5.3'||v.params.ephemeral!==true)process.exit(3);return reply({thread:{id:'test'}});}
if(m==='set_model'){if(v.provider!=='supercode'||v.modelId!=='glm-5.3')process.exit(4);return reply({});}
if(m==='session/set_model'){if(v.params.modelId!=='supercode/glm-5.3')process.exit(5);return reply({});}
if(m==='prompt'){reply({});out({type:'message_end',message:{role:'assistant',content:[{type:'text',text:'OK'}],stopReason:'stop'}});return out({type:'agent_settled'});}
if(m==='session/prompt'){out({method:'session/update',params:{sessionId:'test',update:{sessionUpdate:'agent_message_chunk',content:{type:'text',text:'OK'}}}});return reply({stopReason:'end_turn'});}
if(m==='turn/start'){reply({turn:{id:'turn'}});out({method:'item/completed',params:{threadId:'test',item:{type:'userMessage'}}});out({method:'item/completed',params:{threadId:'test',item:{type:'agentMessage',text:'OK'}}});return out({method:'turn/completed',params:{threadId:'test',turn:{id:'turn',status:'completed'}}});}
process.exit(6);
});"#;
        let launch = Launch {
            program,
            prefix: vec!["-e".into(), script.into()],
            source: "protocol-test".into(),
        };
        for agent in ["pi", "opencode", "codex"] {
            let (client, mut events) =
                Client::start(&launch, &[], Path::new("."), &[], agent == "pi", None)
                    .await
                    .unwrap();
            let config = json!({"baseUrl":"https://example.invalid/anthropic","protocol":"anthropic","model":"glm-5.3","config":"model_provider='selected'"});
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                probe_completion(
                    &client,
                    &mut events,
                    agent,
                    &config,
                    "glm-5.3",
                    Path::new("."),
                ),
            )
            .await;
            client.close().await;
            assert_eq!(result.unwrap().unwrap(), "OK", "{agent}");
        }
    }
    #[tokio::test]
    async fn pi_automatic_compaction_keeps_the_native_thread_and_continues_the_turn() {
        let Some(program) = process::find_program("node") else {
            return;
        };
        // A controlled RPC peer streams overflow recovery and its continued
        // answer. No provider or installed agent is started by this test.
        let script = r#"const rl=require('readline').createInterface({input:process.stdin});
const out=v=>process.stdout.write(JSON.stringify(v)+'\n');
rl.on('line',line=>{const v=JSON.parse(line);
if(v.type!=='prompt')process.exit(2);
out({type:'response',id:v.id,command:'prompt',success:true,data:{disposition:'started'}});
out({type:'agent_start'});
out({type:'compaction_start',reason:'overflow'});
out({type:'compaction_end',reason:'overflow',aborted:true,willRetry:false});
out({type:'compaction_start',reason:'overflow'});
out({type:'compaction_end',reason:'overflow',aborted:false,errorMessage:'Provider summary failed',willRetry:false});
out({type:'compaction_start',reason:'overflow'});
out({type:'compaction_end',reason:'overflow',aborted:false,willRetry:true,result:{tokensBefore:900000}});
out({type:'message_start',message:{role:'assistant'}});
out({type:'message_end',message:{role:'assistant',content:[{type:'text',text:'压缩后继续完成任务'}],stopReason:'stop',usage:{input:1200,output:40,totalTokens:1240}}});
out({type:'agent_end'});out({type:'agent_settled'});
});"#;
        let launch = Launch {
            program,
            prefix: vec!["-e".into(), script.into()],
            source: "protocol-test".into(),
        };
        let (client, mut events) = Client::start(&launch, &[], Path::new("."), &[], true, None)
            .await
            .unwrap();
        let turn = Turn {
            client: client.clone(),
            requests: Arc::new(Mutex::new(HashMap::new())),
            session: "chat".into(),
            native: "native-existing".into(),
            turn: "same-turn".into(),
            agent: "pi".into(),
            stopped: AtomicBool::new(false),
            accepting: AtomicBool::new(true),
            inputs_pending: AtomicU64::new(0),
            input_done: Notify::new(),
            settled: AtomicBool::new(false),
            prompt_accepted: AtomicBool::new(false),
            prompt_ready: Notify::new(),
            started: AtomicBool::new(false),
            stop_done: AtomicBool::new(true),
            signature: String::new(),
        };
        let boot = turn.after_setup("temporary-chat-id".into());
        let request = user_request(&boot, json!({"type":"extension_ui_request","method":"confirm","id":"startup","title":"Startup approval"})).unwrap();
        let request_id = request["id"].as_str().unwrap().to_owned();
        turn.requests
            .lock()
            .await
            .insert(request_id.clone(), request);
        let active = boot.after_setup("native-existing".into());
        assert!(Arc::ptr_eq(&turn.requests, &active.requests));
        let rebound = rebind_public_requests(
            &mut *active.requests.lock().await,
            &boot.native,
            &active.native,
            &active.session,
        );
        assert_eq!(rebound.len(), 1);
        assert_eq!(rebound[0]["params"]["threadId"], "native-existing");
        let request = active
            .requests
            .lock()
            .await
            .get(&request_id)
            .cloned()
            .unwrap();
        assert_eq!(
            permission_reply(&request, &json!({"decision":"accept"}), "pi").unwrap(),
            json!({"type":"extension_ui_response","id":"startup","confirmed":true})
        );
        active.requests.lock().await.remove(&request_id);
        assert!(turn.requests.lock().await.is_empty());
        for agent in ["pi", "opencode"] {
            let mut t = active.after_setup("native-existing".into());
            t.agent = agent.into();
            let mut mapper = Mapper::default();
            let mut buffer = protocol::ActivityBuffer::default();
            let mut writes = 0;
            for index in 0..50 {
                let value = if agent == "pi" {
                    json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"中🧋","contentIndex":0}})
                } else {
                    json!({"method":"session/update","params":{"sessionId":"native-existing","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"中🧋"}}}})
                };
                for event in mapper.map(&t, &value) {
                    writes += usize::from(buffer.update(&event).is_some());
                }
                if index == 19 || index == 39 {
                    for event in mapper.checkpoint(&t) {
                        writes += usize::from(buffer.update(&event).is_some());
                    }
                    assert!(
                        mapper.checkpoint(&t).is_empty(),
                        "unchanged text cannot write another checkpoint"
                    );
                }
                assert_eq!(
                    writes,
                    1 + usize::from(index >= 19) + usize::from(index >= 39),
                    "chunk {index} must only write at checkpoints"
                );
            }
            for item in mapper.finish(false) {
                let completed = buffer
                    .update(&event(&t, "item/completed", json!({"item":item})))
                    .unwrap();
                assert_eq!(completed["text"], "中🧋".repeat(50));
                writes += 1;
            }
            assert_eq!(writes, 4);
            let mut failure_mapper = Mapper::default();
            let value = if agent == "pi" {
                json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"partial","contentIndex":0}})
            } else {
                json!({"method":"session/update","params":{"sessionId":"native-existing","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"partial"}}}})
            };
            failure_mapper.map(&t, &value);
            let final_rows = failure_mapper.finish(true);
            assert_eq!(final_rows[0]["text"], "partial");
            assert_eq!(final_rows[0]["status"], "interrupted");
        }
        let mut mapper = Mapper {
            model_context_window: Some(1_000_000),
            ..Mapper::default()
        };
        mapper.observe_pi_usage(&json!({"input":900000,"output":20}));
        let accepted = client
            .request("prompt", json!({"message":"继续当前任务"}), 5)
            .await
            .unwrap();
        assert_eq!(accepted["disposition"], "started");
        let mapped = tokio::time::timeout(Duration::from_secs(5), async {
            let mut mapped = Vec::new();
            loop {
                let event = events
                    .recv()
                    .await
                    .expect("RPC peer closed before settling");
                mapped.extend(mapper.map(&turn, &event));
                if event["type"] == "agent_settled" {
                    break;
                }
            }
            mapped
        })
        .await;
        client.close().await;
        let mapped = mapped.unwrap();
        let compact_start = mapped
            .iter()
            .position(|event| {
                event["params"]["item"]["type"] == "contextCompaction"
                    && event["method"] == "item/started"
            })
            .unwrap();
        let compact_end = mapped
            .iter()
            .position(|event| {
                event["params"]["item"]["type"] == "contextCompaction"
                    && event["params"]["item"]["status"] == "completed"
            })
            .unwrap();
        let answer = mapped
            .iter()
            .position(|event| event["params"]["item"]["text"] == "压缩后继续完成任务")
            .unwrap();
        assert!(compact_start < compact_end && compact_end < answer);
        assert!(mapped
            .iter()
            .all(|event| event["params"]["threadId"] == "native-existing"
                && event["params"]["turnId"] == "same-turn"));
        assert!(!mapped.iter().any(|event| matches!(
            event["method"].as_str(),
            Some("thread/started" | "turn/completed")
        )));
        let compactions = mapped
            .iter()
            .filter(|event| event["params"]["item"]["type"] == "contextCompaction")
            .map(|event| event["params"]["item"]["status"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            compactions,
            vec![
                "inProgress",
                "interrupted",
                "inProgress",
                "failed",
                "inProgress",
                "completed"
            ]
        );
        let failed = mapped
            .iter()
            .find(|event| event["params"]["item"]["status"] == "failed")
            .unwrap();
        assert_eq!(failed["params"]["item"]["error"], "Provider summary failed");
        assert_eq!(mapper.usage["contextTokens"], 1240);
        assert_eq!(mapper.usage["modelContextWindow"], 1_000_000);
    }
    #[test]
    fn provider_probes_preserve_failures_reject_tools_and_bound_unicode_output() {
        let mut text = String::new();
        let mut failure = None;
        probe_output(&json!({"type":"message_end","message":{"role":"assistant","stopReason":"error","errorMessage":"401 invalid key"}}), "pi", "", &mut text, &mut failure).unwrap();
        assert_eq!(failure.as_deref(), Some("401 invalid key"));
        assert!(probe_output(
            &json!({"type":"tool_execution_start"}),
            "pi",
            "",
            &mut text,
            &mut failure
        )
        .is_err());
        assert!(probe_notification(
            &json!({"type":"extension_ui_request","id":1,"method":"confirm"})
        )
        .is_err());
        assert!(probe_output(&json!({"method":"item/started","params":{"threadId":"test","item":{"type":"commandExecution"}}}), "codex", "test", &mut text, &mut failure).is_err());
        failure = None;
        assert!(probe_output(&json!({"method":"turn/completed","params":{"threadId":"test","turn":{"status":"failed","error":{"message":"model unavailable"}}}}), "codex", "test", &mut text, &mut failure).unwrap());
        assert_eq!(failure.as_deref(), Some("model unavailable"));
        let long = "中文".repeat(4000);
        probe_output(&json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":long},{"type":"text","text":long}]}}), "pi", "", &mut text, &mut failure).unwrap();
        assert!(text.len() <= 8192);
        assert!(text.ends_with('中') || text.ends_with('文'));
    }
    #[tokio::test]
    async fn independent_protocol_processes_do_not_block_or_cancel_each_other() {
        let Some(program) = process::find_program("node") else {
            return;
        };
        // Controlled protocol peers, not real Agent runs or provider requests.
        let script = r#"const rl=require('readline').createInterface({input:process.stdin});
const write=v=>process.stdout.write(JSON.stringify(v)+'\n');
rl.on('line',line=>{const v=JSON.parse(line);const method=v.method||v.type;
if(method==='approval')write(v.type?{type:'extension_ui_request',method:'confirm',id:1,title:'Confirm'}:{jsonrpc:'2.0',method:'session/request_permission',id:1,params:{options:[],toolCall:{title:'Confirm'}}});
if(method==='hold')return;
if(method==='exit')process.exit(0);
write(v.type?{type:'response',command:method,id:v.id,success:true,data:{session:process.env.SUPERCODE_SESSION_ID}}:{jsonrpc:'2.0',id:v.id,result:{session:process.env.SUPERCODE_SESSION_ID}});
});"#;
        let launch = Launch {
            program,
            prefix: vec!["-e".into(), script.into()],
            source: "protocol-test".into(),
        };
        for pi in [false, true] {
            let (a, mut events_a) = Client::start(
                &launch,
                &[],
                Path::new("."),
                &[("SUPERCODE_SESSION_ID".into(), "a".into())],
                pi,
                None,
            )
            .await
            .unwrap();
            let (b, mut events_b) = Client::start(
                &launch,
                &[],
                Path::new("."),
                &[("SUPERCODE_SESSION_ID".into(), "b".into())],
                pi,
                None,
            )
            .await
            .unwrap();
            assert_ne!(a.pid, b.pid);
            let (_, held) = a.begin_request("hold", json!({})).await.unwrap();
            let reply = b.request("ping", json!({}), 5).await.unwrap();
            assert_eq!(reply["session"], "b");
            a.request("approval", json!({}), 5).await.unwrap();
            b.request("approval", json!({}), 5).await.unwrap();
            let raw_a = tokio::time::timeout(Duration::from_secs(5), events_a.recv())
                .await
                .unwrap()
                .unwrap();
            let raw_b = tokio::time::timeout(Duration::from_secs(5), events_b.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(raw_a["id"], raw_b["id"]);
            a.close().await;
            assert!(held.await.unwrap().is_err());
            assert_eq!(
                b.request("ping", json!({}), 5).await.unwrap()["session"],
                "b"
            );
            b.close().await;
        }
    }
    #[test]
    fn concurrency_requires_the_audited_opencode_release() {
        assert!(verified_concurrent_prompt(Some("1.18.23")));
        assert!(!verified_concurrent_prompt(Some("2.0.0")));
        assert!(!verified_concurrent_prompt(Some("1.18.24")));
        assert!(!verified_concurrent_prompt(None));
    }
    #[test]
    fn intentional_stop_has_no_transport_error_but_unexpected_disconnect_remains_failed() {
        let stopped = completion_payload("t", true, false, Some("Agent 意外退出".into()));
        assert_eq!(stopped["status"], "interrupted");
        assert!(stopped["error"].is_null());
        let failed = completion_payload("t", false, false, Some("Agent 意外退出".into()));
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["error"]["message"], "Agent 意外退出");
    }
    #[test]
    fn automation_mcp_kinds_use_executable_transport_and_remote_connections_use_url() {
        let mut server = client_features::ToolServer {
            id: "browser".into(),
            name: "Browser".into(),
            kind: "browser".into(),
            command: "C:/测试/node.exe".into(),
            args: vec!["C:/测试/mcp/cli.js".into()],
            url: None,
            enabled: true,
        };
        assert_eq!(
            mcp_entry(&server, "supercode_browser", true)["command"],
            server.command
        );
        assert!(mcp_entry(&server, "supercode_browser", true)
            .get("url")
            .is_none());
        assert!(mcp_entry(&server, "supercode_browser", false)["args"].is_array());
        server.url = Some("https://example.com/mcp".into());
        assert_eq!(mcp_entry(&server, "remote", true)["type"], "http");
        assert!(mcp_entry(&server, "remote", false).get("command").is_none());
    }
    #[test]
    fn anthropic_endpoints_and_cli_context_annotations_are_translated_without_changing_other_ids() {
        let c = json!({"protocol":"anthropic","baseUrl":"https://api.example/coding/"});
        assert_eq!(sdk_base(&c), "https://api.example/coding/v1");
        assert_eq!(
            sdk_base(&json!({"protocol":"anthropic","baseUrl":"https://api.example/v1/"})),
            "https://api.example/v1"
        );
        assert_eq!(wire_model(&c, "k3[1M]"), "k3");
        assert_eq!(
            wire_model(&json!({"protocol":"chat"}), "literal[1M]"),
            "literal[1M]"
        );
        assert_eq!(
            opencode_config(
                &json!({"protocol":"anthropic","baseUrl":"https://api.example","model":"k3[1M]"}),
                "ask"
            )["provider"]["supercode"]["models"]["k3"]["name"],
            "k3[1M]"
        );
    }
    #[test]
    fn imported_native_declarations_reach_runtime_without_losing_capabilities_or_permissions() {
        let open = crate::ccswitch::sanitize_provider("opencode", "gateway", json!({
            "npm":"@ai-sdk/anthropic", "name":"导入连接",
            "options":{"baseURL":"https://gateway.example/coding","apiKey":"fixture-secret","headers":{"X-Project":"fixture-header"},"setCacheKey":true},
            "models":{"m[1M]":{"name":"真实模型","reasoning":false,"limit":{"context":32768,"output":4096},"modalities":{"input":["text"],"output":["text"]}}},
            "permission":"allow", "plugins":["unsafe"], "mcp":{"unsafe":{}}
        })).unwrap();
        let runtime = opencode_config(&open, "read");
        let provider = &runtime["provider"]["supercode"];
        assert_eq!(
            provider["options"]["baseURL"],
            "https://gateway.example/coding"
        );
        assert_eq!(provider["options"]["apiKey"], "fixture-secret");
        assert_eq!(
            provider["options"]["headers"]["X-Project"],
            "fixture-header"
        );
        assert_eq!(provider["models"]["m[1M]"]["limit"]["context"], 32768);
        assert_eq!(provider["models"]["m[1M]"]["reasoning"], false);
        assert_eq!(runtime["permission"]["*"], "deny");
        assert!(runtime.get("plugins").is_none());
        assert!(runtime.get("mcp").is_none());
        let pi = crate::ccswitch::sanitize_provider("pi", "gateway", json!({
            "api":"openai-completions","baseUrl":"http://localhost:1234/v1","apiKey":"fixture-secret","headers":{"X-Project":"fixture-header"},
            "models":[{"id":"m","name":"中文模型","reasoning":false,"input":["text"],"contextWindow":32768,"maxTokens":2048,"cost":{"input":1,"output":2},"compat":{"supportsDeveloperRole":false}},
                {"id":"other","baseUrl":"https://other.example/v1","api":"openai-responses"}]
        })).unwrap();
        let provider = pi_provider_config(&pi).unwrap();
        assert_eq!(provider["api"], "openai-completions");
        assert_eq!(provider["apiKey"], "fixture-secret");
        assert_eq!(provider["headers"]["X-Project"], "fixture-header");
        assert_eq!(provider["models"][0]["id"], "m");
        assert_eq!(provider["models"][0]["contextWindow"], 32768);
        assert_eq!(provider["models"][0]["reasoning"], false);
        assert_eq!(provider["models"][0]["input"], json!(["text"]));
        assert_eq!(
            provider["models"][0]["compat"]["supportsDeveloperRole"],
            false
        );
        assert_eq!(provider["models"][0]["cost"]["cacheRead"], 0);
        assert_eq!(provider["models"][1]["api"], "openai-responses");
        assert_eq!(provider["models"][1]["baseUrl"], "https://other.example/v1");
        assert!(pi_provider_config(&json!({})).is_none());
    }
    #[test]
    fn usage_accumulates_all_native_messages_but_never_claims_custom_models_are_free() {
        let mut m = Mapper::default();
        let usage = json!({"input":10,"output":4,"cacheRead":5,"cacheWrite":2,"totalTokens":21,"cost":{"total":0.01}});
        m.add_usage(&usage);
        let total = m.add_usage(&usage);
        assert_eq!(total["total"]["totalTokens"], 42);
        assert_eq!(total["total"]["cacheWriteInputTokens"], 4);
        assert_eq!(total["last"]["inputTokens"], 17);
        assert_eq!(total["contextTokens"], 21);
        assert_eq!(total["total"]["inputTokens"], 34);
        assert!(total.get("costUsd").is_none());
        let mut native = Mapper {
            priced: true,
            ..Mapper::default()
        };
        native.add_usage(&usage);
        assert_eq!(native.add_usage(&usage)["costUsd"], 0.02);
    }
    #[test]
    fn streaming_usage_updates_context_without_accumulating_billing_or_erasing_it_with_zero() {
        let mut mapper = Mapper {
            model_context_window: Some(1_000_000),
            ..Mapper::default()
        };
        assert!(!mapper.observe_pi_usage(&json!({"input":0,"output":0,"totalTokens":0})));
        assert!(mapper
            .observe_pi_usage(&json!({"input":100,"cacheRead":50,"output":5,"totalTokens":0})));
        assert_eq!(mapper.usage["contextTokens"], 155);
        assert_eq!(mapper.usage["modelContextWindow"], 1_000_000);
        assert_eq!(mapper.usage["total"], json!({}));
        assert!(mapper.observe_pi_usage(&json!({"input":100,"cacheRead":50,"output":10})));
        assert_eq!(mapper.usage["contextTokens"], 160);
        assert!(!mapper.observe_pi_usage(&json!({"input":100,"cacheRead":50,"output":10})));
        assert!(!mapper.observe_pi_usage(&json!({"input":0,"output":0,"totalTokens":0})));
        assert_eq!(mapper.usage["contextTokens"], 160);
        let data = mapper.add_usage(&json!({"input":100,"cacheRead":50,"output":10}));
        assert_eq!(data["total"]["totalTokens"], 160);
        assert_eq!(data["total"]["inputTokens"], 150);
        assert!(mapper.observe_pi_usage(&json!({"input":20,"output":2})));
        assert_eq!(mapper.usage["contextTokens"], 22);
        assert_eq!(mapper.usage["total"]["totalTokens"], 160);
    }
    #[test]
    fn acp_partial_updates_keep_capacity_and_compaction_can_reduce_context() {
        let mut mapper = Mapper::default();
        let first = mapper.observe_acp_usage(&json!({"used":82300,"size":262144}));
        assert_eq!(first["modelContextWindow"], 262144);
        let next = mapper.observe_acp_usage(&json!({"used":90000}));
        assert_eq!(next["contextTokens"], 90000);
        assert_eq!(next["modelContextWindow"], 262144);
        let compacted = mapper.observe_acp_usage(&json!({"used":10000,"size":0}));
        assert_eq!(compacted["contextTokens"], 10000);
        assert_eq!(compacted["modelContextWindow"], 262144);
        assert_eq!(
            mapper.observe_acp_usage(&json!({"size":1048576}))["modelContextWindow"],
            1048576
        );
    }
    #[test]
    fn generated_provider_limits_are_model_specific_and_native_limits_win() {
        let config = json!({"baseUrl":"https://api.example","protocol":"anthropic","model":"glm-5.3","models":["glm-5.3","kimi-for-coding","kimi-for-coding-highspeed","MiniMax-M2.7"]});
        let provider = pi_provider_config(&config).unwrap();
        for (id, capacity) in [
            ("glm-5.3", 1000000),
            ("kimi-for-coding", 1048576),
            ("kimi-for-coding-highspeed", 262144),
            ("MiniMax-M2.7", 204800),
        ] {
            let row = provider["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == id)
                .unwrap();
            assert_eq!(row["contextWindow"], capacity);
            assert_eq!(
                opencode_config(&config, "read")["provider"]["supercode"]["models"][id]["limit"]
                    ["context"],
                capacity
            );
        }
        let declared = json!({"baseUrl":"https://api.example","protocol":"anthropic","model":"glm-5.3","piProvider":{"models":[{"id":"glm-5.3","contextWindow":32768}]}});
        assert_eq!(
            pi_provider_config(&declared).unwrap()["models"][0]["contextWindow"],
            32768
        );
    }
    #[test]
    fn native_tool_paths_and_interrupted_items_survive_normalization() {
        assert_eq!(
            tool_args(&json!({"filePath":"中文目录/a.md"}))["file_path"],
            "中文目录/a.md"
        );
        assert_eq!(pi_tool("edit"), "Edit");
        let mut m = Mapper::default();
        m.items.insert(
            "x".into(),
            json!({"id":"x","text":"partial","status":"inProgress"}),
        );
        assert_eq!(m.finish(true)[0]["status"], "interrupted");
        assert!(m.items.is_empty());
    }
    #[test]
    fn failures_and_ids_are_not_mistaken_for_events() {
        assert!(is_response(
            &json!({"id":"1","type":"response","success":false}),
            true
        ));
        assert!(response(
            &json!({"type":"response","success":false,"error":"denied"}),
            true
        )
        .is_err());
        assert!(!is_response(
            &json!({"id":1,"method":"session/request_permission"}),
            false
        ));
    }
    #[test]
    fn approvals_only_allow_selected_once() {
        let request = json!({"params":{"nativeRequest":{"id":7,"params":{"options":[{"optionId":"once","kind":"allow_once"},{"optionId":"always","kind":"allow_always"}]}}}});
        let reply = permission_reply(&request, &json!({"decision":"accept"}), "opencode").unwrap();
        assert_eq!(reply["result"]["outcome"]["optionId"], "once");
        let denied =
            permission_reply(&request, &json!({"decision":"decline"}), "opencode").unwrap();
        assert_eq!(denied["result"]["outcome"]["outcome"], "cancelled");
    }
    #[test]
    fn pi_questions_validate_choices() {
        let request = json!({"params":{"nativeRequest":{"id":"x","method":"select","options":["one","two"]}}});
        assert!(permission_reply(
            &request,
            &json!({"answers":{"0":{"answers":["fake"]}}}),
            "pi"
        )
        .is_err());
        assert_eq!(
            permission_reply(
                &request,
                &json!({"answers":{"0":{"answers":["two"]}}}),
                "pi"
            )
            .unwrap()["value"],
            "two"
        );
    }
    #[test]
    fn read_mode_blocks_mutation_and_credentials_stay_in_config() {
        let c = opencode_config(&json!({}), "read");
        assert_eq!(c["permission"]["*"], "deny");
        assert_eq!(c["permission"]["read"], "allow");
        assert!(PI_EXTENSION.contains("User declined"));
    }
    #[test]
    fn opencode_execution_permissions_do_not_keep_the_readonly_policy() {
        for mode in ["ask", "strict", "full"] {
            let config = opencode_config(&json!({}), mode);
            assert_eq!(
                config["permission"]["*"],
                if mode == "full" { "allow" } else { "ask" }
            );
            assert!(config["permission"].get("read").is_none());
        }
        assert_eq!(
            opencode_config(&json!({}), "read")["permission"]["*"],
            "deny"
        );
    }
    #[test]
    fn managed_opencode_compaction_overrides_disabled_parent_environment() {
        for mode in ["ask", "strict", "full", "read", "summary"] {
            let mut command = std::process::Command::new("controlled-protocol-peer");
            command.env("OPENCODE_DISABLE_AUTOCOMPACT", "true");
            command.envs(opencode_environment(&json!({}), mode));
            let env = command
                .get_envs()
                .collect::<std::collections::HashMap<_, _>>();
            assert_eq!(
                env.get(std::ffi::OsStr::new("OPENCODE_DISABLE_AUTOCOMPACT")),
                Some(&Some(std::ffi::OsStr::new("false")))
            );
            let config: Value = serde_json::from_str(
                env[std::ffi::OsStr::new("OPENCODE_CONFIG_CONTENT")]
                    .unwrap()
                    .to_str()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(config["compaction"]["auto"], true);
            assert!(config["compaction"].get("prune").is_none());
            assert_eq!(
                config["permission"]["*"],
                if mode == "full" {
                    "allow"
                } else if matches!(mode, "read" | "summary") {
                    "deny"
                } else {
                    "ask"
                }
            );
        }
    }
    #[test]
    fn pi_operation_results_keep_native_counts_output_and_command_metadata() {
        let mut edit = json!({"arguments":{"file_path":"a.ts"}});
        apply_pi_result(
            &mut edit,
            &json!({"content":[{"type":"text","text":"Edited a.ts"}],"details":{"patch":"@@ -1 +1 @@\n-old\n+new\n","supercodeFileChange":{"path":"a.ts","before":"old\n","after":"new\n"}}}),
            false,
        );
        assert_eq!(edit["changeSummary"][0]["added"], 1);
        assert_eq!(edit["changeSummary"][0]["removed"], 1);
        assert_eq!(edit["fileSnapshot"]["before"], "old\n");
        assert_eq!(edit["output"], "Edited a.ts");
        let mut command = json!({"arguments":{"command":"node check.js"}});
        apply_pi_result(
            &mut command,
            &json!({"content":[{"type":"text","text":"native output"}],"structuredContent":{"exit_code":1,"wall_time_seconds":1.3}}),
            true,
        );
        assert_eq!(command["exitCode"], 1);
        assert_eq!(command["durationMs"], 1300);
        assert_eq!(command["output"], "native output");
        assert!(command["changes"].is_null());
    }
    #[test]
    fn image_and_history_inputs_are_preserved() {
        let p = prompt_content(
            "pi",
            &[
                json!({"type":"text","text":"history"}),
                json!({"type":"image","source":{"data":"abc","media_type":"image/png"}}),
            ],
            "rules",
        );
        assert_eq!(p["message"], "history");
        assert_eq!(p["images"][0]["data"], "abc");
        let acp = prompt_content(
            "opencode",
            &[
                json!({"type":"text","text":"看图"}),
                json!({"type":"image","source":{"data":"abc","media_type":"image/png"}}),
            ],
            "rules",
        );
        assert_eq!(acp[1]["type"], "image");
        assert_eq!(acp[1]["data"], "abc");
        assert_eq!(acp[1]["mimeType"], "image/png");
    }
}
