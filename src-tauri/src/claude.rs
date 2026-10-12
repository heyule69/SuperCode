//! Native Claude CLI, bidirectional JSONL. No shell or resident Node bridge.
use crate::{process, protocol, runtime, storage::Session, AppState};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    process::{Child, ChildStdin},
    sync::{oneshot, Mutex, Notify},
};

#[derive(Default)]
pub struct Runtime {
    pub client: Mutex<Option<Arc<Turn>>>,
    sessions: crate::session_runtimes::Sessions<Runtime>,
}
pub struct Turn {
    input: Mutex<ChildStdin>,
    child: Mutex<Child>,
    _job: process::JobGuard,
    pub requests: Mutex<HashMap<String, Value>>,
    pub pid: u32,
    session: String,
    native: String,
    turn: std::sync::Mutex<String>,
    signature: std::sync::Mutex<String>,
    pub active: AtomicBool,
    alive: AtomicBool,
    last_used: AtomicU64,
    controls: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    inputs: std::sync::Mutex<InputState>,
    stopped: AtomicBool,
    finished: Notify,
    _bridge: Option<crate::bridge::Bridge>,
}

#[derive(Default)]
struct InputState {
    pending: HashSet<String>,
    cancel_queued: bool,
    usage: [u64; 5],
    cost_baseline: Option<f64>,
    turn_cost: Option<f64>,
}
impl InputState {
    fn consume(&mut self, message: &Value) -> Vec<String> {
        let ids = message["user_message_uuids"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        let single = if message["type"] == "user" {
            message["uuid"].as_str()
        } else {
            message["user_message_uuid"].as_str()
        };
        ids.into_iter()
            .chain(single)
            .filter(|id| self.pending.remove(*id))
            .map(str::to_owned)
            .collect()
    }
    fn usage(&mut self, message: &Value) -> Value {
        let u = &message["usage"];
        let current = [
            crate::usage_tokens::count(&u["input_tokens"]).unwrap_or(0),
            crate::usage_tokens::count(&u["cache_read_input_tokens"]).unwrap_or(0),
            crate::usage_tokens::count(&u["cache_creation_input_tokens"]).unwrap_or(0),
            crate::usage_tokens::count(&u["output_tokens"]).unwrap_or(0),
            crate::usage_tokens::count(&u["output_tokens_details"]["thinking_tokens"]).unwrap_or(0),
        ];
        for (total, next) in self.usage.iter_mut().zip(current) {
            *total = total.saturating_add(next);
        }
        if let Some(cost) = message["total_cost_usd"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0)
        {
            if let Some(base) = self.cost_baseline {
                self.turn_cost = Some(self.turn_cost.unwrap_or(0.0) + (cost - base).max(0.0));
            }
            self.cost_baseline = Some(cost);
        }
        let [input, cached, write, output, reasoning] = self.usage;
        let input = input.saturating_add(cached).saturating_add(write);
        json!({"total":{"inputTokens":input,"cachedInputTokens":cached,"cacheWriteInputTokens":write,"outputTokens":output,"reasoningOutputTokens":reasoning,"totalTokens":input.saturating_add(output)},"costUsd":self.turn_cost})
    }
    fn next_turn(&mut self) {
        self.usage = [0; 5];
        self.turn_cost = None;
    }
}

fn publish(app: &AppHandle, event: Value) {
    runtime::publish_event(app, event);
}

fn is_main_result(message: &Value) -> bool {
    message["type"] == "result" && message["parent_tool_use_id"].is_null()
}

/// A native model result can precede the parent's background-task follow-up.
/// New CLIs report run idleness separately; older CLIs still use task lifecycles.
#[derive(Default)]
struct RunCompletion {
    state: Option<String>,
    result_received: bool,
}
impl RunCompletion {
    fn observe(&mut self, message: &Value) -> bool {
        if message["type"] == "system" && message["subtype"] == "session_state_changed" {
            if let Some(state @ ("running" | "requires_action" | "idle")) = message["state"].as_str() {
                self.state = Some(state.to_owned());
                return true;
            }
            return false;
        }
        if matches!(message["type"].as_str(), Some("assistant" | "stream_event"))
            && message["parent_tool_use_id"].is_null()
        {
            // An old result must not finish a newly started parent response.
            self.result_received = false;
        }
        false
    }
    fn result(&mut self) { self.result_received = true; }
    fn ready(&self, children_pending: bool) -> bool {
        self.result_received && !children_pending
            && matches!(self.state.as_deref(), None | Some("idle"))
    }
    fn finished(&mut self) { self.result_received = false; }
}

fn permission_args(read_only: bool, permission: &str) -> Result<Vec<String>, String> {
    let mode = crate::client_features::claude_native_permission(if read_only { "read" } else { permission })?;
    // Native plan mode protects the workspace while allowing its designated
    // plan file, questions, subagents and ExitPlanMode. A Read/Glob/Grep tool
    // whitelist removes the planning workflow itself.
    Ok(vec!["--permission-mode".into(), mode.into(), "--tools".into(), "default".into()])
}

fn result_failure(message: &Value) -> Option<String> {
    let subtype = message["subtype"].as_str().unwrap_or("");
    if message["is_error"] != true && !subtype.starts_with("error_") { return None; }
    // SDK error results carry an `errors` array, often without `result` text.
    let errors = message["errors"].as_array().map(|errors| errors.iter()
        .filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).take(8)
        .map(|s| protocol::bounded(s, 512)).collect::<Vec<_>>()).unwrap_or_default();
    if !errors.is_empty() { return Some(protocol::bounded(&errors.join("\n"), 4096)); }
    let fallback = match subtype {
        "error_max_turns" => "Claude 达到原生任务轮次上限，任务未完成",
        "error_max_budget_usd" => "Claude 达到原生费用上限，任务未完成",
        _ => "Claude 任务执行失败，原生进程未返回错误详情",
    };
    Some(protocol::bounded(message["result"].as_str().map(str::trim)
        .filter(|s| !s.is_empty()).unwrap_or(fallback), 4096))
}
fn event(method: &str, native: &str, turn: &str, mut params: Value) -> Value {
    params["threadId"] = json!(native);
    params["turnId"] = json!(turn);
    json!({"method":method,"params":params})
}
impl Turn {
    fn id(&self) -> String {
        self.turn.lock().unwrap().clone()
    }
    async fn write(&self, value: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        self.input
            .lock()
            .await
            .write_all(&bytes)
            .await
            .map_err(|e| e.to_string())
    }
}
fn tool_response(tool_name: &Value, input: &Value, result: &Value) -> Result<Value, String> {
    if result["decision"] == "accept" {
        let mut response = json!({"behavior":"allow","updatedInput":input});
        if let Some(permission) = plan_execution_permission(tool_name, result)? {
            // Apply the user's execution choice in the same native reply as
            // ExitPlanMode. Allowing only that call leaves subsequent Write
            // requests under the old/manual policy.
            response["updatedPermissions"] = json!([{"type":"setMode","mode":crate::client_features::claude_native_permission(permission)?,"destination":"session"}]);
        }
        return Ok(response);
    }
    if let Some(answers) = result.get("answers") {
        if tool_name != "AskUserQuestion" {
            return Err("此请求不是用户提问".into());
        }
        let mut updated = input.clone();
        let mut values = serde_json::Map::new();
        for (index, question) in input["questions"]
            .as_array()
            .ok_or("问题格式无效")?
            .iter()
            .enumerate()
        {
            let answer = answers[index.to_string()]["answers"]
                .as_array()
                .ok_or("问题回答格式无效")?
                .iter()
                .filter_map(Value::as_str)
                .filter(|a| !a.trim().is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            if answer.is_empty() {
                return Err("请回答所有问题".into());
            }
            values.insert(
                question["question"].as_str().unwrap_or("").into(),
                json!(answer),
            );
        }
        updated["answers"] = Value::Object(values);
        return Ok(json!({"behavior":"allow","updatedInput":updated}));
    }
    let message = if tool_name == "ExitPlanMode" {
        let feedback = result["feedback"].as_str().unwrap_or("").trim();
        let instruction =
            "用户选择继续修改计划，暂不执行。请保持计划模式，完善计划后再次请求用户确认。";
        if feedback.is_empty() {
            instruction.to_owned()
        } else {
            format!(
                "{instruction}\n用户的修改意见：{}",
                protocol::bounded(feedback, 16 * 1024)
            )
        }
    } else {
        "用户拒绝了本次操作".to_owned()
    };
    Ok(json!({"behavior":"deny","message":message}))
}

fn plan_execution_permission<'a>(tool_name: &Value, result: &'a Value) -> Result<Option<&'a str>, String> {
    if tool_name != "ExitPlanMode" || result["decision"] != "accept" { return Ok(None); }
    let permission = match result.get("executionPermission") {
        None => "edit",
        Some(value) => value.as_str().ok_or("计划执行权限格式无效")?,
    };
    crate::client_features::validate_permission("claude", permission)?;
    if permission == "read" { return Err("实施计划需要执行权限，不能继续使用计划模式".into()); }
    Ok(Some(permission))
}

fn execution_signature(signature: &str, permission: &str) -> Result<String, String> {
    let mut value: Value = serde_json::from_str(signature).map_err(|_| "活动任务配置无效")?;
    let fields = value.as_array_mut().filter(|fields| fields.len() >= 3).ok_or("活动任务配置无效")?;
    let len = fields.len();
    fields[len - 3] = json!(false);
    fields[len - 2] = json!(permission);
    Ok(value.to_string())
}

impl Runtime {
    pub fn for_session(&self, id: &str) -> Arc<Runtime> { self.sessions.get(id) }
    pub async fn clients(&self) -> Vec<Arc<Turn>> {
        let mut clients = self.client.lock().await.iter().cloned().collect::<Vec<_>>();
        for (_, runtime) in self.sessions.entries() { clients.extend(runtime.client.lock().await.iter().cloned()); }
        clients
    }
    pub async fn steer(
        &self,
        session: &str,
        expected: &str,
        id: &str,
        content: Vec<Value>,
        permission: &str,
        effort: Option<&str>,
    ) -> Result<(), String> {
        // Serialize admission with the result handler: a late click must never land in a different turn.
        let runtime = self.for_session(session);
        let slot = runtime.client.lock().await;
        let turn = slot.as_ref().ok_or("Claude 连接已关闭")?;
        if turn.session != session
            || turn.id() != expected
            || !turn.active.load(Ordering::Acquire)
            || turn.stopped.load(Ordering::Acquire)
        {
            return Err("当前任务已结束或变化，请继续排队发送".into());
        }
        uuid::Uuid::parse_str(id).map_err(|_| "引导消息 ID 无效")?;
        crate::client_features::validate_steering_settings(&turn.signature.lock().unwrap(), permission, effort)?;
        turn.inputs.lock().unwrap().pending.insert(id.into());
        let result = turn.write(json!({"type":"user","uuid":id,"priority":"next","session_id":turn.native,"parent_tool_use_id":null,"message":{"role":"user","content":content}})).await;
        if result.is_err() {
            turn.inputs.lock().unwrap().pending.remove(id);
        }
        result
    }
    pub async fn release_idle(&self) {
        for (id, runtime) in self.sessions.entries() {
            runtime.release_idle_local().await;
            if runtime.client.lock().await.is_none() { self.sessions.remove_idle(&id, &runtime); }
        }
        self.release_idle_local().await;
    }
    async fn release_idle_local(&self) {
        let mut slot = self.client.lock().await;
        if slot.as_ref().is_some_and(|c| {
            !c.active.load(Ordering::Acquire)
                && crate::storage::now().saturating_sub(c.last_used.load(Ordering::Relaxed) as i64)
                    >= 300
        }) {
            if let Some(c) = slot.take() {
                let _ = c.child.lock().await.start_kill();
            }
        }
    }
    pub async fn shutdown(&self) {
        for (_, runtime) in self.sessions.entries() { runtime.shutdown_local().await; }
        self.shutdown_local().await;
    }
    async fn shutdown_local(&self) {
        if let Some(turn) = self.client.lock().await.take() {
            turn.stopped.store(true, Ordering::Relaxed);
            let _ = turn.child.lock().await.start_kill();
        }
    }
    pub async fn interrupt(&self, _app: &AppHandle, session: &str) -> Result<(), String> {
        let runtime = self.for_session(session);
        let turn = runtime
            .client
            .lock()
            .await
            .as_ref()
            .cloned()
            .ok_or("Claude 尚未启动")?;
        if turn.session != session {
            return Err("此会话没有运行中的任务".into());
        }
        if !turn.active.load(Ordering::Acquire) {
            return Err("此会话没有运行中的任务".into());
        }
        turn.stopped.store(true, Ordering::Release);
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        turn.controls.lock().await.insert(id.clone(), tx);
        let cancel_queued = turn.inputs.lock().unwrap().cancel_queued;
        turn.write(
            json!({"type":"control_request","request_id":id,"request":{"subtype":"interrupt","cancel_queued":true}}),
        )
        .await?;
        let ack = tokio::time::timeout(std::time::Duration::from_secs(10), rx).await;
        turn.controls.lock().await.remove(&id);
        // Older CLIs leave queued input alive after an interrupt. Close only when needed,
        // rather than allowing a supposedly stopped task to restart behind the user's back.
        if !cancel_queued && !turn.inputs.lock().unwrap().pending.is_empty() {
            let _ = turn.child.lock().await.start_kill();
        }
        if !matches!(ack, Ok(Ok(ref v)) if v["subtype"] != "error") {
            let _ = turn.child.lock().await.start_kill();
        }
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            while turn.active.load(Ordering::Acquire) {
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        })
        .await
        .map_err(|_| "Claude 停止状态同步超时".to_owned())?;
        Ok(())
    }
    pub async fn respond(&self, app: &AppHandle, id: Value, result: Value) -> Result<(), String> {
        let key = id.as_str().ok_or("请求 ID 无效")?;
        let mut selected = None;
        for turn in self.clients().await { if turn.requests.lock().await.contains_key(key) { selected = Some(turn); break; } }
        let turn = selected.ok_or("该请求已结束")?;
        let mut requests = turn.requests.lock().await;
        let request = requests.get(key).ok_or("该请求已结束")?;
        let raw_id = key.strip_prefix(&format!("claude:{}:", turn.session)).ok_or("请求 ID 无效")?;
        let input = &request["params"]["input"];
        if request["params"]["subtype"] != "can_use_tool" {
            turn.write(json!({"type":"control_response","response":{"subtype":"error","request_id":raw_id,"error":"SuperCode does not support this request yet"}})).await?;
            requests.remove(key);
            publish(
                app,
                event(
                    "serverRequest/resolved",
                    &turn.native,
                    &turn.id(),
                    json!({"requestId":id}),
                ),
            );
            return Ok(());
        }
        let permission = plan_execution_permission(&request["params"]["toolName"], &result)?;
        let signature = permission.map(|permission| execution_signature(&turn.signature.lock().unwrap(), permission)).transpose()?;
        let response = tool_response(&request["params"]["toolName"], input, &result)?;
        turn.write(json!({"type":"control_response","response":{"subtype":"success","request_id":raw_id,"response":response}})).await?;
        if let Some(signature) = signature { *turn.signature.lock().unwrap() = signature; }
        requests.remove(key);
        publish(
            app,
            event(
                "serverRequest/resolved",
                &turn.native,
                &turn.id(),
                json!({"requestId":id}),
            ),
        );
        Ok(())
    }
}

pub async fn send(
    app: &AppHandle,
    session: &Session,
    cwd: &str,
    text: &str,
    model: Option<String>,
    read_only: bool,
    permission: &str,
    content: Vec<Value>,
    metadata: Value,
    effort: Option<String>,
) -> Result<Session, String> {
    let state = app.state::<AppState>();
    let session_runtime = state.claude.for_session(&session.id);
    let route = state.store.route("claude", Some(&session.id))?;
    let fingerprint = route.fingerprint();
    let mut config = route.config;
    crate::runtime_diagnostics::register_secrets(app, &config);
    let model = crate::providers::real_claude_model(&config, model.as_deref());
    let signature = json!([
        fingerprint,
        crate::client_features::tool_configuration(app)?,
        session.id,
        cwd,
        model,
        read_only,
        permission,
        effort
    ])
    .to_string();
    let mut slot = session_runtime.client.lock().await;
    if let Some(c) = slot.as_ref() {
        if c.active.load(Ordering::Acquire) {
            return Err("Claude 的上一任务尚未结束".into());
        }
        if c.alive.load(Ordering::Acquire) && *c.signature.lock().unwrap() == signature {
            state.store.set_model(&session.id, model.as_deref())?;
            state.store.save_message(
                &crate::client_features::user_message_id(&metadata),
                &session.id,
                "user",
                text,
                "userMessage",
                &metadata,
            )?;
            *c.turn.lock().unwrap() = uuid::Uuid::new_v4().to_string();
            c.stopped.store(false, Ordering::Release);
            c.active.store(true, Ordering::Release);
            c.inputs.lock().unwrap().next_turn();
            c.last_used
                .store(crate::storage::now() as u64, Ordering::Relaxed);
            publish(
                app,
                event(
                    "turn/started",
                    &c.native,
                    &c.id(),
                    json!({"turn":{"id":c.id()}}),
                ),
            );
            if let Err(e) = c.write(json!({"type":"user","session_id":c.native,"parent_tool_use_id":null,"message":{"role":"user","content":content}})).await {
                c.active.store(false,Ordering::Release);
                c.alive.store(false,Ordering::Release);
                let _ = c.child.lock().await.start_kill();
                publish(app,event("turn/completed",&c.native,&c.id(),json!({"turn":{"id":c.id(),"status":"failed","error":{"message":e}}})));
                return Err(e);
            }
            return state.store.session(&session.id);
        }
    }
    if let Some(c) = slot.take() {
        let _ = c.child.lock().await.start_kill();
    }
    let bridge = if config["protocol"] == "chat" {
        let bridge = crate::bridge::Bridge::start(config.clone()).await?;
        config["env"] = json!({"ANTHROPIC_BASE_URL":bridge.base_url,"ANTHROPIC_AUTH_TOKEN":bridge.token,"ANTHROPIC_MODEL":model,"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1"});
        Some(bridge)
    } else {
        None
    };

    let executable = crate::agents::resolve(app, "claude")?.program;
    crate::runtime_diagnostics::register_secrets(app, &config);
    let native = session
        .native_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // Validate IDs before passing flags. Prompts and permission replies always travel via stdin.
    uuid::Uuid::parse_str(&native).map_err(|_| "Claude 原生会话 ID 无效")?;
    let permission_args = permission_args(read_only, permission)?;
    let (native_mcp, plugin_settings, plugin_args) =
        crate::extensions::claude_config(app, Path::new(cwd))?;
    let mut args = vec![
        "--print".to_owned(),
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
        "--replay-user-messages".into(),
        "--permission-prompt-tool".into(),
        "stdio".into(),
        "--strict-mcp-config".into(),
        "--settings".into(),
        plugin_settings.to_string(),
        "--append-system-prompt".into(),
        crate::client_features::instructions(app),
    ];
    args.extend(permission_args);
    // This flag permits a later explicit plan confirmation to restore full
    // access; it does not activate bypass while the process is in plan mode.
    if permission == "full" || read_only || permission == "read" {
        args.push("--allow-dangerously-skip-permissions".into());
    }
    if let Some(effort) = effort.filter(|v| matches!(v.as_str(), "low" | "medium" | "high" | "max"))
    {
        args.extend(["--effort".into(), effort]);
    }
    args.extend(plugin_args);
    let mcp = crate::client_features::claude_mcp(app, native_mcp)?;
    crate::runtime_diagnostics::register_secrets(app, &mcp);
    if mcp["mcpServers"].as_object().is_some_and(|v| !v.is_empty()) {
        args.extend(["--mcp-config".into(), mcp.to_string()]);
    }
    args.push(format!(
        "--{}={native}",
        if session.native_id.is_some() {
            "resume"
        } else {
            "session-id"
        }
    ));
    if let Some(model) = &model {
        args.push(format!("--model={model}"));
    }
    let mut command = process::command(
        &executable,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    // Use only connection/model data from settings. Permissions remain under the UI's control.
    command.args(["--setting-sources", ""]);
    if route.id != crate::session_config::LOCAL {
        for key in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_MODEL",
            "ANTHROPIC_REASONING_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
        ] {
            command.env_remove(key);
        }
    }
    if let Some(env) = config["env"].as_object() {
        for (key, value) in env {
            if let Some(value) = value.as_str() {
                command.env(key, value);
            }
        }
    }
    crate::official_accounts::configure(&mut command, app, "claude", &config)?;
    command.env("SUPERCODE_SESSION_ID", &session.id);
    command.env("CLAUDE_CODE_SDK_READS_SESSION_STATE", "1");
    let mut child = command
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Claude 启动失败：{e}"))?;
    let job = process::JobGuard::attach(&child)?;
    let output = child.stdout.take().ok_or("无法读取 Claude 输出")?;
    let stderr = child.stderr.take().ok_or("无法读取 Claude 日志")?;
    let turn = Arc::new(Turn {
        pid: child.id().ok_or("Claude 已退出")?,
        input: Mutex::new(child.stdin.take().ok_or("无法连接 Claude stdin")?),
        child: Mutex::new(child),
        _job: job,
        requests: Mutex::new(HashMap::new()),
        session: session.id.clone(),
        native: native.clone(),
        turn: std::sync::Mutex::new(uuid::Uuid::new_v4().to_string()),
        signature: std::sync::Mutex::new(signature),
        active: AtomicBool::new(true),
        alive: AtomicBool::new(true),
        last_used: AtomicU64::new(crate::storage::now() as u64),
        controls: Mutex::new(HashMap::new()),
        inputs: std::sync::Mutex::new(InputState {
            cost_baseline: if session.native_id.is_none() {
                Some(0.0)
            } else {
                None
            },
            ..InputState::default()
        }),
        stopped: AtomicBool::new(false),
        finished: Notify::new(),
        _bridge: bridge,
    });
    let log_app = app.clone();
    let log_session = session.id.clone();
    crate::runtime_diagnostics::record(app, Some(&session.id), "agent_process_started", json!({"source":"claude","pid":turn.pid,"model":model}));
    tauri::async_runtime::spawn(async move {
        crate::runtime_diagnostics::stderr(Some(log_app), Some(log_session), "claude", stderr).await;
    });
    let mut reader = BufReader::new(output);
    // Native SDK defaults forward only child tool activity. Ask for child text
    // explicitly so independent reports can be rendered in their own panel.
    turn.write(json!({"type":"control_request","request_id":"supercode-init","request":{"subtype":"initialize","hooks":{},"skills":[],"forwardSubagentText":true}})).await?;
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let frame = runtime::read_frame(&mut reader)
                .await?
                .ok_or("Claude 在初始化时退出，请检查登录或配置")?;
            let value: Value =
                serde_json::from_slice(&frame).map_err(|e| format!("Claude 协议异常：{e}"))?;
            if value["type"] == "control_response"
                && value["response"]["request_id"] == "supercode-init"
            {
                if value["response"]["subtype"] == "error" {
                    return Err(value["response"]["error"]
                        .as_str()
                        .unwrap_or("Claude 初始化失败")
                        .to_owned());
                }
                return Ok::<(), String>(());
            }
        }
    })
    .await
    .map_err(|_| "Claude 初始化超时")??;
    state.store.bind_native(&session.id, &native)?;
    state.store.set_model(&session.id, model.as_deref())?;
    state.store.save_message(
        &crate::client_features::user_message_id(&metadata),
        &session.id,
        "user",
        text,
        "userMessage",
        &metadata,
    )?;
    if session.title == "新会话" {
        state
            .store
            .rename(&session.id, &text.chars().take(24).collect::<String>())?;
    }
    turn.write(json!({"type":"user","session_id":native,"parent_tool_use_id":null,"message":{"role":"user","content":content}})).await?;
    publish(
        app,
        event(
            "turn/started",
            &native,
            &turn.id(),
            json!({"turn":{"id":turn.id()}}),
        ),
    );
    *slot = Some(turn.clone());
    drop(slot);
    let reader_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut stream = Stream::default();
        let mut completion = RunCompletion::default();
        let mut failure = Some("Claude 连接在任务完成前关闭".to_owned());
        loop {
            let frame = match runtime::read_frame(&mut reader).await {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            };
            let message: Value = match serde_json::from_slice(&frame) {
                Ok(v) => v,
                Err(e) => {
                    failure = Some(format!("Claude JSONL 格式无效：{e}"));
                    break;
                }
            };
            turn.last_used
                .store(crate::storage::now() as u64, Ordering::Relaxed);
            let session_state_event = completion.observe(&message);
            if session_state_event {
                crate::runtime_diagnostics::record(&reader_app, Some(&turn.session), "agent_session_state", json!({"turnId":turn.id(),"state":message["state"]}));
                if !completion.ready(stream.tasks.pending()) {
                    continue;
                }
            }
            if message["type"] == "system" && matches!(message["subtype"].as_str(), Some("task_started" | "task_progress" | "task_updated" | "task_notification")) {
                crate::runtime_diagnostics::record(&reader_app, Some(&turn.session), "agent_background_task_event", json!({"turnId":turn.id(),"subtype":message["subtype"],"taskId":message["task_id"],"toolUseId":message["tool_use_id"],"status":message["status"].as_str().or_else(||message["patch"]["status"].as_str())}));
            }
            if message["type"] == "assistant" && message["parent_tool_use_id"].is_string() {
                crate::runtime_diagnostics::record(&reader_app, Some(&turn.session), "agent_child_message", json!({"turnId":turn.id(),"toolUseId":message["parent_tool_use_id"],"contentBlocks":message["message"]["content"].as_array().map(Vec::len)}));
            }
            let consumed = turn.inputs.lock().unwrap().consume(&message);
            for id in consumed {
                crate::outbox::native_consumed(&reader_app, &id);
            }
            if message["type"] == "control_response" {
                if let Some(id) = message["response"]["request_id"].as_str() {
                    if let Some(tx) = turn.controls.lock().await.remove(id) {
                        let _ = tx.send(message["response"].clone());
                    }
                }
                continue;
            }
            if message["type"] == "system" && message["subtype"] == "init" {
                turn.inputs.lock().unwrap().cancel_queued = message["capabilities"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v == "interrupt_cancel_queued_v1"));
                let commands = message["slash_commands"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .filter(|s| s.len() <= 200)
                            .take(200)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let data = json!({"commands":commands});
                let _ = reader_app.state::<AppState>().store.save_message(
                    "agent-capabilities",
                    &turn.session,
                    "system",
                    "",
                    "agentCapabilities",
                    &data,
                );
                publish(
                    &reader_app,
                    event("agent/commands", &turn.native, &turn.id(), data),
                );
            } else if message["type"] == "system" && message["subtype"] == "compact_boundary" {
                stream.reset_context();
                publish(
                    &reader_app,
                    event(
                        "thread/tokenUsage/updated",
                        &turn.native,
                        &turn.id(),
                        json!({"tokenUsage":stream.context_usage()}),
                    ),
                );
                let item = json!({"type":"contextCompaction","id":format!("compact-{}",turn.id()),"status":"completed","text":"Claude 已完成原生上下文压缩。","metadata":message["compact_metadata"]});
                publish(
                    &reader_app,
                    event(
                        "item/completed",
                        &turn.native,
                        &turn.id(),
                        json!({"item":item}),
                    ),
                );
            } else if message["type"] == "control_request" {
                let id = format!("claude:{}:{}", turn.session, message["request_id"].as_str().unwrap_or(""));
                let request = &message["request"];
                let mut params = json!({"subtype":request["subtype"],"toolName":request["tool_name"],"input":request["input"],"reason":request["decision_reason"]});
                let method = if request["subtype"] == "can_use_tool"
                    && request["tool_name"] == "AskUserQuestion"
                {
                    params["questions"] = json!(request["input"]["questions"]
                        .as_array()
                        .map(|qs| qs
                            .iter()
                            .enumerate()
                            .map(|(i, q)| {
                                let mut q = q.clone();
                                q["id"] = json!(i.to_string());
                                q
                            })
                            .collect::<Vec<_>>())
                        .unwrap_or_default());
                    "item/tool/requestUserInput"
                } else if request["subtype"] == "can_use_tool" {
                    "claude/tool/requestApproval"
                } else {
                    "claude/unsupportedRequest"
                };
                let mut notification = event(method, &turn.native, &turn.id(), params);
                notification["id"] = json!(id);
                turn.requests.lock().await.insert(id, notification.clone());
                publish(&reader_app, notification);
            } else if message["type"] == "control_cancel_request" {
                let id = format!("claude:{}:{}", turn.session, message["request_id"].as_str().unwrap_or(""));
                turn.requests.lock().await.remove(&id);
                publish(
                    &reader_app,
                    event(
                        "serverRequest/resolved",
                        &turn.native,
                        &turn.id(),
                        json!({"requestId":id}),
                    ),
                );
            } else if message["type"] == "result" || session_state_event {
                // Nested agent results belong to the Task tool, not this turn.
                // Keep the main task, pending approvals and stream intact.
                if !session_state_event && !is_main_result(&message) {
                    continue;
                }
                let result_slot = session_runtime.client.lock().await;
                if !turn.active.load(Ordering::Acquire) {
                    drop(result_slot);
                    continue;
                }
                if !session_state_event {
                    completion.result();
                    let u = &message["usage"];
                    if u.is_object() {
                        let data = stream.result_usage(&message, &mut turn.inputs.lock().unwrap());
                        publish(
                            &reader_app,
                            event(
                                "thread/tokenUsage/updated",
                                &turn.native,
                                &turn.id(),
                                json!({"tokenUsage":data}),
                            ),
                        );
                    }
                    if stream.message_id.is_empty() && message["is_error"] != true {
                        if let Some(text) = message["result"].as_str().filter(|s| !s.is_empty()) {
                            let item = json!({"type":"agentMessage","id":format!("command-result-{}",turn.id()),"text":protocol::bounded(text,128*1024)});
                            publish(
                                &reader_app,
                                event(
                                    "item/completed",
                                    &turn.native,
                                    &turn.id(),
                                    json!({"item":item}),
                                ),
                            );
                        }
                    }
                    failure = result_failure(&message);
                    for notification in stream.partial(&turn.native, &turn.id()) {
                        publish(&reader_app, notification);
                    }
                }
                // A result ends one model turn. Native session state remains
                // running while child notifications owe another parent reply.
                // Preserve the task, stream and approvals until the run is idle.
                if !turn.stopped.load(Ordering::Acquire) && failure.is_none() && !completion.ready(stream.tasks.pending()) {
                    crate::runtime_diagnostics::record(&reader_app, Some(&turn.session), "background_agents_pending", json!({"turnId":turn.id(),"sessionState":completion.state,"childrenPending":stream.tasks.pending()}));
                    drop(result_slot);
                    continue;
                }
                let stopped = turn.stopped.load(Ordering::Acquire);
                if stopped {
                    failure = None;
                }
                if !stopped && failure.is_none() && !turn.inputs.lock().unwrap().pending.is_empty()
                {
                    // A text-only response can finish before the queued supplement is replayed.
                    // It is still the same client task until all submitted input has been consumed.
                    drop(result_slot);
                    continue;
                }
                stream = Stream {
                    model: stream.model.clone(),
                    model_context_window: stream.model_context_window,
                    last_context: stream.context_usage(),
                    ..Stream::default()
                };
                turn.requests.lock().await.clear();
                let unconsumed = turn
                    .inputs
                    .lock()
                    .unwrap()
                    .pending
                    .drain()
                    .collect::<Vec<_>>();
                for id in &unconsumed {
                    crate::outbox::native_failed(&reader_app, id, "任务已停止，补充消息尚未接收");
                }
                if !unconsumed.is_empty() {
                    let _ = turn.child.lock().await.start_kill();
                }
                let status = if stopped {
                    "interrupted"
                } else if failure.is_some() {
                    "failed"
                } else {
                    "completed"
                };
                let completed_id = turn.id();
                turn.last_used
                    .store(crate::storage::now() as u64, Ordering::Relaxed);
                turn.active.store(false, Ordering::Release);
                completion.finished();
                publish(
                    &reader_app,
                    event(
                        "turn/completed",
                        &turn.native,
                        &completed_id,
                        json!({"turn":{"id":completed_id,"status":status,"error":failure.take().map(|e|json!({"message":e}))}}),
                    ),
                );
                turn.finished.notify_one();
                drop(result_slot);
                failure = Some("Claude 连接在任务完成前关闭".to_owned());
                continue;
            } else {
                for notification in stream.translate(&message, &turn.native, &turn.id()) {
                    publish(&reader_app, notification);
                }
            }
        }
        for notification in stream.partial(&turn.native, &turn.id()) {
            publish(&reader_app, notification);
        }
        let exit = turn.child.lock().await.try_wait().ok().flatten();
        let active = turn.active.load(Ordering::Acquire);
        let stopped = turn.stopped.load(Ordering::Acquire);
        if active && !stopped && failure.is_none() {
            // A successful model result deferred for background work is not a
            // successful run if the native connection dies before becoming idle.
            failure = Some("Claude 连接在任务完成前关闭".to_owned());
        }
        crate::runtime_diagnostics::record(&reader_app, Some(&turn.session), "agent_connection_closed",
            json!({"turnId":turn.id(),"active":active,
                "stopRequested":stopped,"pid":turn.pid,"exitCode":exit.and_then(|s| s.code()),"error":failure,
                "errorType":if active && !stopped { failure.as_deref().map(crate::runtime_diagnostics::error_type) } else { None }}));
        let _ = turn.child.lock().await.kill().await;
        let mut slot = session_runtime.client.lock().await;
        if slot.as_ref().is_some_and(|c| c.pid == turn.pid) {
            slot.take();
        }
        drop(slot);
        turn.alive.store(false, Ordering::Release);
        let unconsumed = turn
            .inputs
            .lock()
            .unwrap()
            .pending
            .drain()
            .collect::<Vec<_>>();
        for id in unconsumed {
            crate::outbox::native_failed(&reader_app, &id, "Claude 连接已关闭，消息尚未接收");
        }
        if !turn.active.swap(false, Ordering::AcqRel) {
            return;
        }
        let stopped = turn.stopped.load(Ordering::Relaxed);
        if stopped {
            failure = None;
        }
        let status = if stopped {
            "interrupted"
        } else if failure.is_some() {
            "failed"
        } else {
            "completed"
        };
        publish(
            &reader_app,
            event(
                "turn/completed",
                &turn.native,
                &turn.id(),
                json!({"turn":{"id":turn.id(),"status":status,"error":failure.map(|e|json!({"message":e}))}}),
            ),
        );
        turn.finished.notify_one();
    });
    state.store.session(&session.id)
}

#[derive(Default)]
struct Stream {
    context_tokens: Option<u64>,
    uncached_input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    cache_write_input_tokens: Option<u64>,
    output_tokens: u64,
    last_usage: Value,
    last_context: Value,
    model: String,
    model_context_window: Option<u64>,
    message_id: String,
    text: String,
    tools: HashMap<String, Value>,
    blocks: HashMap<usize, Value>,
    tasks: crate::claude_tasks::Tasks,
}
impl Stream {
    fn reset_context(&mut self) {
        self.last_context = Value::Null;
        self.clear_request_usage();
    }
    fn clear_request_usage(&mut self) {
        self.context_tokens = None;
        self.uncached_input_tokens = None;
        self.cached_input_tokens = None;
        self.cache_write_input_tokens = None;
        self.output_tokens = 0;
        self.last_usage = json!({});
    }
    fn begin_message(&mut self) {
        self.last_context = self.context_usage();
        self.clear_request_usage();
    }
    fn observe_model(&mut self, model: &str) {
        if self.model != model {
            self.model_context_window = None;
            self.last_context = Value::Null;
        }
        self.model = model.into();
    }
    fn observe_usage(&mut self, usage: &Value) -> bool {
        let input = crate::usage_tokens::count(&usage["input_tokens"]);
        let cached = crate::usage_tokens::count(&usage["cache_read_input_tokens"]);
        let write = crate::usage_tokens::count(&usage["cache_creation_input_tokens"]);
        let output = crate::usage_tokens::count(&usage["output_tokens"]);
        let counts = [input, cached, write, output];
        if counts.iter().all(Option::is_none) {
            return false;
        }
        let reported = counts.iter().flatten().any(|n| *n > 0);
        let input_reported = [input, cached, write].iter().flatten().any(|n| *n > 0);
        // Compatible providers and the SDK may emit zero placeholders. Keep
        // fields independent: cache counts can arrive only in message_delta.
        for (old, new) in [
            (&mut self.uncached_input_tokens, input),
            (&mut self.cached_input_tokens, cached),
            (&mut self.cache_write_input_tokens, write),
        ] {
            if new.is_some() && (input_reported || old.is_none()) {
                *old = new;
            }
        }
        if !self.last_usage.is_object() {
            self.last_usage = json!({});
        }
        if let Some(input) = self.uncached_input_tokens {
            let cached = self.cached_input_tokens.unwrap_or(0);
            let write = self.cache_write_input_tokens.unwrap_or(0);
            let input = input.saturating_add(cached).saturating_add(write);
            self.context_tokens = (input > 0).then_some(input);
            self.last_usage["inputTokens"] = json!(input);
            self.last_usage["cachedInputTokens"] = json!(cached);
            self.last_usage["cacheWriteInputTokens"] = json!(write);
        }
        if let Some(output) = output {
            // Per-message output is cumulative. Completed SDK assistant
            // snapshots can contain a smaller placeholder than the stream.
            self.output_tokens = self.output_tokens.max(output);
            self.last_usage["outputTokens"] = json!(self.output_tokens);
        }
        if let Some(context) = self.context_tokens {
            self.last_usage["totalTokens"] = json!(context.saturating_add(self.output_tokens));
        }
        reported
    }
    fn context_window(&self, result: &Value) -> Option<u64> {
        let models = result["modelUsage"].as_object()?;
        let usage = models.get(&self.model).or_else(|| {
            (models.len() == 1)
                .then(|| models.values().next())
                .flatten()
        })?;
        crate::usage_tokens::count(&usage["contextWindow"]).filter(|n| *n > 0)
    }
    fn context_usage(&self) -> Value {
        let known = self
            .context_tokens
            .map(|input| input.saturating_add(self.output_tokens));
        let last = if known.is_some() {
            self.last_usage.clone()
        } else {
            self.last_context["last"]
                .as_object()
                .map(|_| self.last_context["last"].clone())
                .unwrap_or_else(|| json!({}))
        };
        json!({"total":{},"last":last,"contextTokens":known.or_else(||self.last_context["contextTokens"].as_u64()),"modelContextWindow":self.model_context_window,"cumulative":false})
    }
    fn result_usage(&mut self, result: &Value, inputs: &mut InputState) -> Value {
        self.model_context_window = self.context_window(result).or(self.model_context_window);
        // Result usage sums all calls in the turn. It describes current context
        // only when the native runner confirms there was a single model call.
        if self.context_tokens.is_none() && result["num_turns"] == 1 {
            self.observe_usage(&result["usage"]);
        }
        let total = inputs.usage(result);
        let mut data = self.context_usage();
        data["total"] = total["total"].clone();
        data["costUsd"] = total["costUsd"].clone();
        data["nativeCumulativeCostUsd"] = result["total_cost_usd"].clone();
        data
    }
    fn usage_event(&self, native: &str, turn: &str) -> Value {
        event(
            "thread/tokenUsage/updated",
            native,
            turn,
            json!({"tokenUsage":self.context_usage()}),
        )
    }
    fn partial(&mut self, native: &str, turn: &str) -> Vec<Value> {
        if self.text.is_empty() {
            return vec![];
        }
        let value = event(
            "item/completed",
            native,
            turn,
            json!({"item":{"type":"agentMessage","id":self.message_id,"text":self.text}}),
        );
        self.text.clear();
        vec![value]
    }
    fn translate(&mut self, message: &Value, native: &str, turn: &str) -> Vec<Value> {
        let mut events = vec![];
        if let Some(item) = self.tasks.observe(message, &self.tools, chrono::Utc::now().timestamp_millis()) {
            events.push(event("item/updated", native, turn, json!({"item":item})));
        }
        if !message["parent_tool_use_id"].is_null() {
            return events;
        }
        match message["type"].as_str().unwrap_or("") {
            "stream_event" => {
                let e = &message["event"];
                if e["type"] == "message_start" {
                    self.begin_message();
                    if let Some(model) = e["message"]["model"].as_str() {
                        self.observe_model(model);
                    }
                    self.observe_usage(&e["message"]["usage"]);
                    events.push(self.usage_event(native, turn));
                    events.extend(self.partial(native, turn));
                    self.message_id = e["message"]["id"].as_str().unwrap_or("").into();
                    self.text.clear();
                    self.blocks.clear();
                }
                if e["type"] == "message_delta" && self.observe_usage(&e["usage"]) {
                    events.push(self.usage_event(native, turn));
                }
                let index = e["index"].as_u64().unwrap_or(0) as usize;
                if e["type"] == "content_block_start" && self.blocks.len() < 200 {
                    let block = &e["content_block"];
                    let item = match block["type"].as_str().unwrap_or("") {
                        "thinking" | "redacted_thinking" => Some(
                            json!({"id":format!("{}-reasoning-{index}",self.message_id),"type":"reasoning","summary":[block["thinking"].as_str().unwrap_or("")],"redacted":block["type"]=="redacted_thinking","status":"inProgress"}),
                        ),
                        "tool_use" => Some(
                            json!({"id":block["id"],"type":"claudeToolCall","tool":block["name"],"arguments":block["input"],"inputText":"","status":"preparing"}),
                        ),
                        "text" => Some(
                            json!({"id":self.message_id,"type":"agentMessage","text":self.text,"status":"inProgress"}),
                        ),
                        _ => None,
                    };
                    if let Some(item) = item {
                        if item["type"] == "claudeToolCall" {
                            self.tools
                                .insert(item["id"].as_str().unwrap_or("").into(), item.clone());
                        }
                        events.push(event("item/started", native, turn, json!({"item":item})));
                        self.blocks.insert(index, item);
                    }
                }
                if e["type"] == "content_block_delta" && e["delta"]["type"] == "thinking_delta" {
                    if let Some(item) = self.blocks.get_mut(&index) {
                        let delta = e["delta"]["thinking"].as_str().unwrap_or("");
                        item["summary"][0] = json!(protocol::bounded(
                            &format!("{}{}", item["summary"][0].as_str().unwrap_or(""), delta),
                            protocol::TEXT_LIMIT
                        ));
                        events.push(event(
                            "item/reasoning/summaryTextDelta",
                            native,
                            turn,
                            json!({"itemId":item["id"],"summaryIndex":0,"delta":delta}),
                        ));
                    }
                }
                if e["type"] == "content_block_delta" && e["delta"]["type"] == "input_json_delta" {
                    if let Some(item) = self.blocks.get_mut(&index) {
                        let delta = e["delta"]["partial_json"].as_str().unwrap_or("");
                        item["inputText"] = json!(protocol::bounded(
                            &format!("{}{}", item["inputText"].as_str().unwrap_or(""), delta),
                            protocol::TEXT_LIMIT
                        ));
                        events.push(event(
                            "item/claudeToolCall/inputDelta",
                            native,
                            turn,
                            json!({"itemId":item["id"],"delta":delta}),
                        ));
                    }
                }
                if e["type"] == "content_block_stop" {
                    if let Some(mut item) = self.blocks.remove(&index) {
                        if item["type"] == "reasoning" {
                            item["status"] = json!("completed");
                            events.push(event(
                                "item/completed",
                                native,
                                turn,
                                json!({"item":item}),
                            ));
                        } else if item["type"] == "claudeToolCall" {
                            if let Ok(input) = serde_json::from_str::<Value>(
                                item["inputText"].as_str().unwrap_or(""),
                            ) {
                                item["arguments"] = input;
                            }
                            item["status"] = json!("inProgress");
                            self.tools
                                .insert(item["id"].as_str().unwrap_or("").into(), item.clone());
                            events.push(event("item/updated", native, turn, json!({"item":item})));
                        }
                    }
                }
                if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta" {
                    let delta = e["delta"]["text"].as_str().unwrap_or("");
                    let room = protocol::TEXT_LIMIT.saturating_sub(self.text.len());
                    let mut end = delta.len().min(room);
                    while !delta.is_char_boundary(end) {
                        end -= 1;
                    }
                    let delta = &delta[..end];
                    self.text.push_str(delta);
                    events.push(event(
                        "item/agentMessage/delta",
                        native,
                        turn,
                        json!({"itemId":self.message_id,"delta":delta}),
                    ));
                }
            }
            "assistant" => {
                let id = message["message"]["id"].as_str().unwrap_or("");
                if self.message_id != id {
                    self.begin_message();
                    self.message_id = id.into();
                }
                if let Some(model) = message["message"]["model"].as_str() {
                    self.observe_model(model);
                }
                if self.observe_usage(&message["message"]["usage"]) {
                    events.push(self.usage_event(native, turn));
                }
                if let Some(content) = message["message"]["content"].as_array() {
                    let text = content
                        .iter()
                        .filter_map(|b| {
                            if b["type"] == "text" {
                                b["text"].as_str()
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    let media = content
                        .iter()
                        .filter(|b| {
                            matches!(
                                b["type"].as_str(),
                                Some("image" | "image_url" | "audio" | "video" | "resource")
                            )
                        })
                        .take(12)
                        .cloned()
                        .collect::<Vec<_>>();
                    if !text.is_empty() || !media.is_empty() {
                        events.push(event("item/completed",native,turn,json!({"item":{"id":id,"type":"agentMessage","text":protocol::bounded(&text,protocol::TEXT_LIMIT),"contentItems":media}})));
                        self.text.clear();
                    }
                    for (index, block) in content.iter().enumerate().filter(|(_, b)| {
                        b["type"] == "thinking" || b["type"] == "redacted_thinking"
                    }) {
                        events.push(event("item/completed",native,turn,json!({"item":{"id":format!("{id}-reasoning-{index}"),"type":"reasoning","summary":[protocol::bounded(block["thinking"].as_str().unwrap_or(""),protocol::TEXT_LIMIT)],"redacted":block["type"]=="redacted_thinking","status":"completed"}})));
                    }
                    for block in content.iter().filter(|b| b["type"] == "tool_use") {
                        let id = block["id"].as_str().unwrap_or("").to_owned();
                        let method = if self.tools.contains_key(&id) {
                            "item/updated"
                        } else {
                            "item/started"
                        };
                        let item = json!({"id":id,"type":"claudeToolCall","tool":block["name"],"arguments":block["input"],"status":"inProgress"});
                        // These are in-flight tools, removed on tool_result.
                        // A history cache budget must not lose live Agent IDs.
                        self.tools.insert(id.clone(), item.clone());
                        events.push(event(method, native, turn, json!({"item":item})));
                    }
                }
            }
            "user" => {
                if let Some(content) = message["message"]["content"].as_array() {
                    for block in content.iter().filter(|b| b["type"] == "tool_result") {
                        let id = block["tool_use_id"].as_str().unwrap_or("");
                        let mut item = self.tools.remove(id).unwrap_or_else(
                            || json!({"id":id,"type":"claudeToolCall","tool":"Claude 工具"}),
                        );
                        let output =
                            block["content"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| {
                                    block["content"]
                                        .as_array()
                                        .map(|content| {
                                            content
                                                .iter()
                                                .filter_map(|b| b["text"].as_str())
                                                .collect::<Vec<_>>()
                                                .join("\n")
                                        })
                                        .unwrap_or_default()
                                });
                        if block["content"].is_array() {
                            item["contentItems"] = block["content"].clone();
                        }
                        item["output"] = json!(protocol::bounded(&output, protocol::TEXT_LIMIT));
                        item["status"] = json!(if block["is_error"] == true {
                            "failed"
                        } else {
                            "completed"
                        });
                        self.tasks.launch(&mut item, &message["tool_use_result"], &output, chrono::Utc::now().timestamp_millis());
                        events.push(event("item/completed", native, turn, json!({"item":item})));
                    }
                }
            }
            "tool_progress" => {
                events.push(event("item/mcpToolCall/progress",native,turn,json!({"itemId":message["tool_use_id"],"message":format!("工具运行 {} 秒",message["elapsed_time_seconds"])})));
            }
            _ => {}
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_inflight_agent_calls_survive_the_tool_history_budget() {
        let mut stream = Stream::default();
        let calls = (0..205).map(|i| json!({"type":"tool_use","id":format!("agent-{i}"),"name":"Agent","input":{"description":format!("agent-{i}")}})).collect::<Vec<_>>();
        stream.translate(&json!({"type":"assistant","message":{"id":"m","content":calls}}),"n","t");
        assert_eq!(stream.tools.len(),205);
        for i in 0..205 {
            let id = format!("agent-{i}");
            let task = format!("task-{i}");
            let events = stream.translate(&json!({"type":"user","tool_use_result":{"status":"async_launched","isAsync":true,"agentId":task},"message":{"content":[{"type":"tool_result","tool_use_id":id,"content":"Async agent launched successfully."}]}}),"n","t");
            assert_eq!(events[0]["params"]["item"]["tool"],"Agent");
            assert_eq!(events[0]["params"]["item"]["subagentStatus"],"running");
        }
        assert!(stream.tools.is_empty());
        assert!(stream.tasks.pending());
        let last = stream.translate(&json!({"type":"system","subtype":"task_notification","task_id":"task-204","tool_use_id":"agent-204","status":"completed","result":"LAST_CHILD_DONE"}),"n","t");
        assert_eq!(last[0]["params"]["item"]["subagentResult"],"LAST_CHILD_DONE");
        assert!(stream.tasks.pending());
    }
    #[test]
    fn native_run_waits_for_idle_after_all_background_children_finish() {
        let mut run = RunCompletion::default();
        run.observe(&json!({"type":"system","subtype":"session_state_changed","state":"running"}));
        run.result();
        assert!(!run.ready(true));
        // All children can settle while their parent's follow-up is still owed.
        assert!(!run.ready(false));
        run.observe(&json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
        assert!(run.ready(false));
        run.finished();
        assert!(!run.ready(false));
    }
    #[test]
    fn idle_before_result_and_old_cli_fallback_both_finish_once() {
        let mut run = RunCompletion::default();
        run.observe(&json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
        assert!(!run.ready(false));
        run.result();
        assert!(run.ready(false));
        let mut old = RunCompletion::default();
        old.result();
        assert!(!old.ready(true));
        assert!(old.ready(false));
    }
    #[test]
    fn parent_followup_needs_its_own_result_but_child_text_does_not() {
        let mut run = RunCompletion::default();
        run.result();
        run.observe(&json!({"type":"assistant","parent_tool_use_id":"child-tool"}));
        assert!(run.ready(false));
        for kind in ["assistant", "stream_event"] {
            run.result();
            run.observe(&json!({"type":kind,"parent_tool_use_id":null}));
            run.observe(&json!({"type":"system","subtype":"session_state_changed","state":"idle"}));
            assert!(!run.ready(false));
        }
        run.result();
        assert!(run.ready(false));
    }
    #[test]
    fn pending_native_permission_does_not_finish_the_run() {
        let mut run = RunCompletion::default();
        run.observe(&json!({"type":"system","subtype":"session_state_changed","state":"requires_action"}));
        run.result();
        assert!(!run.ready(false));
        run.observe(&json!({"type":"system","subtype":"session_state_changed","state":"unexpected"}));
        assert!(!run.ready(false));
        run.observe(&json!({"type":"system","subtype":"session_state_changed","state":"running"}));
        assert!(!run.ready(false));
    }
    #[test]
    fn native_async_agent_report_updates_the_original_tool_without_main_context_pollution() {
        let mut stream=Stream::default();
        stream.translate(&json!({"type":"assistant","message":{"id":"m","model":"glm-5.3","content":[{"type":"tool_use","id":"a","name":"Agent","input":{"description":"normal-demo"}}]}}),"n","t");
        let launched=stream.translate(&json!({"type":"user","tool_use_result":{"status":"async_launched","isAsync":true,"agentId":"child-a"},"message":{"content":[{"type":"tool_result","tool_use_id":"a","content":"Async agent launched successfully.\nagentId: child-a (internal ID)"}]}}),"n","t");
        assert!(stream.tasks.pending());
        assert_eq!(launched[0]["params"]["item"]["subagentStatus"],"running");
        let child=stream.translate(&json!({"type":"assistant","parent_tool_use_id":"a","message":{"id":"child-message","model":"glm-5.3","content":[{"type":"text","text":"# 子代理报告\nCHILD_TEXT_DONE"}]}}),"n","t");
        assert_eq!(child.len(),1);
        assert_eq!(child[0]["params"]["item"]["subagentResult"],"# 子代理报告\nCHILD_TEXT_DONE");
        assert_ne!(stream.message_id,"child-message");
        // Task progress is lifecycle metadata, not primary-model token usage.
        stream.translate(&json!({"type":"system","subtype":"task_progress","task_id":"child-a","usage":{"total_tokens":99999}}),"n","t");
        assert!(stream.context_tokens.is_none());
        let completed=stream.translate(&json!({"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification><task-id>child-a</task-id><tool-use-id>a</tool-use-id><status>completed</status><result>完整报告\nCHILD_DONE</result></task-notification>"}}),"n","t");
        assert!(!stream.tasks.pending());
        assert_eq!(completed[0]["params"]["item"]["id"],"a");
        assert_eq!(completed[0]["params"]["item"]["subagentResult"],"完整报告\nCHILD_DONE");
        assert_eq!(completed[0]["params"]["turnId"],"t");
    }
    #[test]
    fn planning_keeps_native_workflow_tools_without_widening_permissions() {
        for (read_only, permission, native) in [(true,"full","plan"),(false,"read","plan"),(false,"ask","manual")] {
            assert_eq!(permission_args(read_only,permission).unwrap(), vec!["--permission-mode",native,"--tools","default"]);
        }
        assert!(permission_args(false,"invalid").is_err());
    }
    #[test]
    fn only_main_agent_results_can_end_the_turn() {
        for result in [
            json!({"type":"result","subtype":"success"}),
            json!({"type":"result","parent_tool_use_id":null,"is_error":true}),
        ] { assert!(is_main_result(&result)); }
        for result in [
            json!({"type":"result","parent_tool_use_id":"planning-task","subtype":"success"}),
            json!({"type":"result","parent_tool_use_id":"planning-task","is_error":true}),
            json!({"type":"assistant","parent_tool_use_id":null}),
        ] { assert!(!is_main_result(&result)); }
    }
    #[test]
    fn sdk_error_arrays_and_limit_reasons_are_not_lost() {
        let result = json!({"type":"result","subtype":"error_during_execution","is_error":true,
            "errors":["context window exceeded", "provider request failed"]});
        assert_eq!(result_failure(&result).unwrap(), "context window exceeded\nprovider request failed");
        assert!(result_failure(&json!({"subtype":"error_max_turns","is_error":true})).unwrap().contains("轮次上限"));
        assert_eq!(result_failure(&json!({"is_error":true,"result":"network disconnected"})), Some("network disconnected".into()));
        assert!(result_failure(&json!({"type":"result","subtype":"success","result":"done"})).is_none());
        assert!(result_failure(&json!({"is_error":true,"errors":["中文".repeat(10000)]})).unwrap().len() <= 4096);
    }
    #[test]
    fn plan_confirmation_sets_execution_mode_without_rewriting_the_plan() {
        let input = json!({"plan":"# 完整计划\n\n1. 实现\n2. 验证","allowedPrompts":[{"tool":"Bash","prompt":"运行测试"}]});
        let reply = tool_response(
            &json!("ExitPlanMode"),
            &input,
            &json!({"decision":"accept"}),
        )
        .unwrap();
        assert_eq!(reply, json!({"behavior":"allow","updatedInput":input,"updatedPermissions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}]}));
        let revised = tool_response(
            &json!("ExitPlanMode"),
            &input,
            &json!({"decision":"decline","feedback":"  先实现图片预览，视频稍后  "}),
        )
        .unwrap();
        assert_eq!(revised["behavior"], "deny");
        assert!(revised.get("updatedInput").is_none());
        assert!(revised["message"]
            .as_str()
            .unwrap()
            .contains("先实现图片预览，视频稍后"));
        let empty = tool_response(
            &json!("ExitPlanMode"),
            &input,
            &json!({"decision":"decline"}),
        )
        .unwrap();
        assert_eq!(empty["behavior"], "deny");
        assert!(empty["message"].as_str().unwrap().contains("暂不执行"));
    }

    #[test]
    fn explicit_plan_execution_permissions_are_validated_and_session_scoped() {
        for (permission, native) in [("full", "bypassPermissions"), ("edit", "acceptEdits"), ("auto", "auto"), ("ask", "manual"), ("strict", "manual"), ("deny", "dontAsk")] {
            let response = tool_response(&json!("ExitPlanMode"), &json!({"plan":"实现与验证"}), &json!({"decision":"accept","executionPermission":permission})).unwrap();
            assert_eq!(response["updatedPermissions"], json!([{"type":"setMode","mode":native,"destination":"session"}]));
        }
        for permission in [json!("read"), json!("invalid"), json!(null), json!(123)] {
            assert!(tool_response(&json!("ExitPlanMode"), &json!({}), &json!({"decision":"accept","executionPermission":permission})).is_err());
        }
        let ordinary = tool_response(&json!("Write"), &json!({"file_path":"index.html"}), &json!({"decision":"accept","executionPermission":"full"})).unwrap();
        assert!(ordinary.get("updatedPermissions").is_none());
        let denied = tool_response(&json!("ExitPlanMode"), &json!({}), &json!({"decision":"decline","executionPermission":"full"})).unwrap();
        assert!(denied.get("updatedPermissions").is_none());
    }

    #[test]
    fn confirmed_plan_updates_reuse_and_steering_configuration() {
        let before = json!(["route",{"tools":[]},"session","D:/project","model",true,"read","high"]).to_string();
        let after = execution_signature(&before, "full").unwrap();
        let value: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(value, json!(["route",{"tools":[]},"session","D:/project","model",false,"full","high"]));
        assert!(crate::client_features::validate_steering_settings(&after,"full",Some("high")).is_ok());
        assert!(crate::client_features::validate_steering_settings(&after,"read",Some("high")).is_err());
        assert!(execution_signature("{}", "edit").is_err());
    }

    #[test]
    fn plan_feedback_is_bounded_and_does_not_change_other_tool_responses() {
        let reply = tool_response(
            &json!("ExitPlanMode"),
            &json!({}),
            &json!({"decision":"decline","feedback":"中".repeat(20000)}),
        )
        .unwrap();
        assert!(reply["message"].as_str().unwrap().len() < 17 * 1024);
        let ordinary = tool_response(
            &json!("Bash"),
            &json!({}),
            &json!({"decision":"decline","feedback":"修改意见"}),
        )
        .unwrap();
        assert_eq!(
            ordinary,
            json!({"behavior":"deny","message":"用户拒绝了本次操作"})
        );
        assert!(tool_response(&json!("ExitPlanMode"), &json!({}), &json!({"answers":{}})).is_err());
        let questions = json!({"questions":[{"question":"显示方式？"}]});
        let answered = tool_response(
            &json!("AskUserQuestion"),
            &questions,
            &json!({"answers":{"0":{"answers":["面板"]}}}),
        )
        .unwrap();
        assert_eq!(answered["updatedInput"]["answers"]["显示方式？"], "面板");
        assert!(tool_response(
            &json!("AskUserQuestion"),
            &questions,
            &json!({"answers":{"0":{"answers":[]}}})
        )
        .is_err());
    }

    #[test]
    fn context_updates_include_cache_and_latest_output_not_aggregate_turn_usage() {
        let mut stream = Stream::default();
        let start = |id: &str, usage: Value| json!({"type":"stream_event","event":{"type":"message_start","message":{"id":id,"model":"glm-fixture","usage":usage}}});
        let updates = stream.translate(&start("one", json!({"input_tokens":10,"cache_read_input_tokens":100,"cache_creation_input_tokens":20,"output_tokens":1})), "n", "t");
        assert_eq!(updates[0]["params"]["tokenUsage"]["contextTokens"], 131);
        let updates = stream.translate(&json!({"type":"stream_event","event":{"type":"message_delta","usage":{"input_tokens":10,"output_tokens":15}}}), "n", "t");
        assert_eq!(updates[0]["params"]["tokenUsage"]["contextTokens"], 145);
        assert_eq!(
            updates[0]["params"]["tokenUsage"]["last"]["inputTokens"],
            130
        );
        let updates = stream.translate(
            &start("two", json!({"input_tokens":20,"output_tokens":0})),
            "n",
            "t",
        );
        assert_eq!(updates[0]["params"]["tokenUsage"]["contextTokens"], 20);
        assert_eq!(stream.context_usage()["last"]["cachedInputTokens"], 0);
        assert_eq!(stream.context_usage()["total"], json!({}));
    }
    #[test]
    fn completed_assistant_context_skips_subagents_and_resets_after_compaction() {
        let mut stream = Stream::default();
        let message = json!({"type":"assistant","message":{"id":"one","usage":{"input_tokens":12,"cache_read_input_tokens":8,"output_tokens":5},"content":[]}});
        let events = stream.translate(&message, "n", "t");
        assert_eq!(events[0]["params"]["tokenUsage"]["contextTokens"], 25);
        let mut child = message;
        child["parent_tool_use_id"] = json!("child");
        child["message"]["usage"]["input_tokens"] = json!(999);
        assert!(stream.translate(&child, "n", "t").is_empty());
        assert_eq!(stream.context_usage()["contextTokens"], 25);
        stream.translate(&json!({"type":"assistant","message":{"id":"two","usage":{"input_tokens":12,"output_tokens":1},"content":[]}}), "n", "t");
        assert_eq!(stream.context_usage()["contextTokens"], 13);
        assert_eq!(stream.context_usage()["last"]["cachedInputTokens"], 0);
        stream.reset_context();
        assert!(stream.context_usage()["contextTokens"].is_null());
        stream.observe_usage(&json!({"input_tokens":0}));
        assert!(stream.context_usage()["contextTokens"].is_null());
    }
    #[test]
    fn late_cache_counts_and_sdk_placeholders_do_not_zero_measured_context() {
        let mut stream = Stream::default();
        let start = |id: &str| json!({"type":"stream_event","event":{"type":"message_start","message":{"id":id,"model":"provider-model","usage":{"input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}}}});
        let events = stream.translate(&start("one"), "n", "t");
        assert!(events[0]["params"]["tokenUsage"]["contextTokens"].is_null());
        let delta = json!({"type":"stream_event","event":{"type":"message_delta","usage":{"cache_read_input_tokens":"24000","cache_creation_input_tokens":100,"output_tokens":300}}});
        let events = stream.translate(&delta, "n", "t");
        assert_eq!(events[0]["params"]["tokenUsage"]["contextTokens"], 24400);
        stream.translate(&json!({"type":"assistant","message":{"id":"one","model":"provider-model","usage":{"input_tokens":0,"output_tokens":0},"content":[]}}), "n", "t");
        assert_eq!(stream.context_usage()["contextTokens"], 24400);
        stream.observe_usage(&json!({"input_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":1}));
        assert_eq!(stream.context_usage()["contextTokens"], 24400);
        let events = stream.translate(&start("two"), "n", "t");
        assert_eq!(events[0]["params"]["tokenUsage"]["contextTokens"], 24400);
        stream.observe_usage(&json!({"input_tokens":12000,"output_tokens":1}));
        assert_eq!(stream.context_usage()["contextTokens"], 12001);
        stream.observe_usage(&json!({"output_tokens":300}));
        stream.observe_usage(&json!({"input_tokens":12000,"output_tokens":1}));
        assert_eq!(stream.context_usage()["contextTokens"], 12300);
        stream.begin_message();
        stream.observe_model("different-model");
        assert!(stream.context_usage()["contextTokens"].is_null());
    }
    #[test]
    fn result_only_usage_is_context_only_for_a_single_request() {
        let mut stream = Stream::default();
        let mut inputs = InputState::default();
        let mut result = json!({"num_turns":1,"usage":{"input_tokens":647,"cache_read_input_tokens":31232,"output_tokens":348},"modelUsage":{"provider-model":{"contextWindow":128000}}});
        let data = stream.result_usage(&result, &mut inputs);
        assert_eq!(data["contextTokens"], 32227);
        assert_eq!(data["modelContextWindow"], 128000);
        assert_eq!(data["total"]["inputTokens"], 31879);
        stream.reset_context();
        result["num_turns"] = json!(4);
        let data = stream.result_usage(&result, &mut inputs);
        assert!(data["contextTokens"].is_null());
        assert_eq!(data["total"]["inputTokens"], 63758);
        stream.observe_usage(&json!({"input_tokens":15000,"output_tokens":12}));
        let data = stream.result_usage(&result, &mut inputs);
        assert_eq!(data["contextTokens"], 15012);
        assert_eq!(data["last"]["inputTokens"], 15000);
    }
    #[test]
    fn context_capacity_matches_the_active_model_instead_of_first_subagent() {
        let stream = Stream {
            model: "glm-fixture".into(),
            ..Stream::default()
        };
        let models = json!({"modelUsage":{"child-model":{"contextWindow":1000000},"glm-fixture":{"contextWindow":128000}}});
        assert_eq!(stream.context_window(&models), Some(128000));
        let unknown = Stream::default();
        assert!(unknown.context_window(&models).is_none());
        assert_eq!(
            unknown.context_window(&json!({"modelUsage":{"one":{"contextWindow":128000}}})),
            Some(128000)
        );
    }
    #[test]
    fn a_result_does_not_consume_a_still_queued_prompt() {
        let mut state = InputState::default();
        state.pending.insert("supplement".into());
        assert!(state
            .consume(&json!({"type":"result","user_message_uuid":"first"}))
            .is_empty());
        assert!(!state.pending.is_empty());
        assert_eq!(
            state.consume(&json!({"type":"user","uuid":"supplement","isReplay":true})),
            vec!["supplement"]
        );
        assert!(state.pending.is_empty());
    }
    #[test]
    fn coalesced_result_consumes_all_our_uuids_once() {
        let mut state = InputState::default();
        state.pending.extend(["a".into(), "b".into()]);
        let consumed = state.consume(&json!({"type":"result","user_message_uuid":"b","user_message_uuids":["a","b","unknown"]}));
        assert_eq!(consumed, vec!["a", "b"]);
        assert!(state.pending.is_empty());
    }
    #[test]
    fn continuation_usage_adds_tokens_but_differences_cumulative_cost() {
        let mut state = InputState {
            cost_baseline: Some(0.0),
            ..InputState::default()
        };
        let result = |cost| json!({"usage":{"input_tokens":20,"output_tokens":12,"output_tokens_details":{"thinking_tokens":3}},"total_cost_usd":cost});
        state.usage(&result(0.2));
        let second = state.usage(&result(0.4));
        assert_eq!(second["total"]["totalTokens"], 64);
        assert_eq!(second["total"]["reasoningOutputTokens"], 6);
        assert_eq!(second["costUsd"], 0.4);
        state.next_turn();
        let third = state.usage(&result(0.6));
        assert_eq!(third["total"]["totalTokens"], 32);
        assert!((third["costUsd"].as_f64().unwrap() - 0.2).abs() < 0.0001);
        let mut resumed = InputState::default();
        assert!(resumed.usage(&result(10.0))["costUsd"].is_null());
        assert_eq!(resumed.usage(&result(10.2))["total"]["totalTokens"], 64);
    }
    #[test]
    fn tool_screenshots_reach_media_normalization_without_entering_plain_text() {
        let mut stream = Stream::default();
        let result = stream.translate(&json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"shot","content":[{"type":"text","text":"已截图"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"encoded-image"}}]}]}}), "n", "t");
        assert_eq!(result[0]["params"]["item"]["output"], "已截图");
        assert_eq!(
            result[0]["params"]["item"]["contentItems"][1]["source"]["data"],
            "encoded-image"
        );
        let reply = stream.translate(&json!({"type":"assistant","message":{"id":"picture","content":[{"type":"image","data":"encoded-image","mimeType":"image/png"}]}}), "n", "t");
        assert_eq!(reply[0]["params"]["item"]["type"], "agentMessage");
        assert_eq!(reply[0]["params"]["item"]["text"], "");
        assert_eq!(
            reply[0]["params"]["item"]["contentItems"][0]["type"],
            "image"
        );
    }
    #[test]
    fn streamed_chinese_final_and_tool_result_keep_identity() {
        let mut stream = Stream::default();
        stream.translate(
            &json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}}}),
            "n",
            "t",
        );
        let delta=stream.translate(&json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"你好🧋"}}}),"n","t");
        assert_eq!(delta[0]["params"]["itemId"], "m1");
        let done=stream.translate(&json!({"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"你好🧋"},{"type":"tool_use","id":"tool1","name":"Read","input":{"file_path":"中文.txt"}}]}}),"n","t");
        assert_eq!(done[0]["params"]["item"]["id"], "m1");
        assert!(stream.partial("n", "t").is_empty());
        let result=stream.translate(&json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool1","content":"文件内容"}]}}),"n","t");
        assert_eq!(result[0]["params"]["item"]["output"], "文件内容");
        assert!(stream.tools.is_empty());
    }
    #[test]
    fn reasoning_and_tool_inputs_are_visible_before_assistant_completion() {
        let mut stream = Stream::default();
        let event = |e: Value| json!({"type":"stream_event","event":e});
        stream.translate(
            &event(json!({"type":"message_start","message":{"id":"m"}})),
            "n",
            "t",
        );
        let start = stream.translate(&event(json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}})),"n","t");
        assert_eq!(start[0]["params"]["item"]["type"], "reasoning");
        let delta = stream.translate(&event(json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"先检查中文"}})),"n","t");
        assert_eq!(delta[0]["method"], "item/reasoning/summaryTextDelta");
        let done = stream.translate(
            &event(json!({"type":"content_block_stop","index":0})),
            "n",
            "t",
        );
        assert_eq!(done[0]["params"]["item"]["summary"][0], "先检查中文");
        stream.translate(&event(json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"r","name":"Read","input":{}}})),"n","t");
        let input = stream.translate(&event(json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\":\"中文.md\"}"}})),"n","t");
        assert_eq!(input[0]["method"], "item/claudeToolCall/inputDelta");
        let ready = stream.translate(
            &event(json!({"type":"content_block_stop","index":1})),
            "n",
            "t",
        );
        assert_eq!(
            ready[0]["params"]["item"]["arguments"]["file_path"],
            "中文.md"
        );
        let assistant = stream.translate(&json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"r","name":"Read","input":{"file_path":"中文.md"}}]}}),"n","t");
        assert_eq!(assistant[0]["method"], "item/updated");
        let failure = stream.translate(&json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"r","content":"不存在","is_error":true}]}}),"n","t");
        assert_eq!(failure[0]["params"]["item"]["status"], "failed");
    }
}
