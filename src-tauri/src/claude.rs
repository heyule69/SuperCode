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
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    process::{Child, ChildStdin},
    sync::{oneshot, Mutex, Notify},
};

#[derive(Default)]
pub struct Runtime {
    pub client: Mutex<Option<Arc<Turn>>>,
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
    signature: String,
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
            u["input_tokens"].as_u64().unwrap_or(0),
            u["cache_read_input_tokens"].as_u64().unwrap_or(0),
            u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
            u["output_tokens"].as_u64().unwrap_or(0),
            u["output_tokens_details"]["thinking_tokens"]
                .as_u64()
                .unwrap_or(0),
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
impl Runtime {
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
        let slot = self.client.lock().await;
        let turn = slot.as_ref().ok_or("Claude 连接已关闭")?;
        if turn.session != session
            || turn.id() != expected
            || !turn.active.load(Ordering::Acquire)
            || turn.stopped.load(Ordering::Acquire)
        {
            return Err("当前任务已结束或变化，请继续排队发送".into());
        }
        uuid::Uuid::parse_str(id).map_err(|_| "引导消息 ID 无效")?;
        crate::client_features::validate_steering_settings(&turn.signature, permission, effort)?;
        turn.inputs.lock().unwrap().pending.insert(id.into());
        let result = turn.write(json!({"type":"user","uuid":id,"priority":"next","session_id":turn.native,"parent_tool_use_id":null,"message":{"role":"user","content":content}})).await;
        if result.is_err() {
            turn.inputs.lock().unwrap().pending.remove(id);
        }
        result
    }
    pub async fn release_idle(&self) {
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
        if let Some(turn) = self.client.lock().await.take() {
            turn.stopped.store(true, Ordering::Relaxed);
            let _ = turn.child.lock().await.start_kill();
        }
    }
    pub async fn interrupt(&self, _app: &AppHandle, session: &str) -> Result<(), String> {
        let turn = self
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
        let turn = self
            .client
            .lock()
            .await
            .as_ref()
            .cloned()
            .ok_or("Claude 连接已关闭")?;
        let key = id.as_str().ok_or("请求 ID 无效")?;
        let mut requests = turn.requests.lock().await;
        let request = requests.get(key).ok_or("该请求已结束")?;
        let raw_id = key.strip_prefix("claude:").ok_or("请求 ID 无效")?;
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
        let response = if result["decision"] == "accept" {
            json!({"behavior":"allow","updatedInput":input})
        } else if let Some(answers) = result.get("answers") {
            if request["params"]["toolName"] != "AskUserQuestion" {
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
            json!({"behavior":"allow","updatedInput":updated})
        } else {
            json!({"behavior":"deny","message":"用户拒绝了本次操作"})
        };
        turn.write(json!({"type":"control_response","response":{"subtype":"success","request_id":raw_id,"response":response}})).await?;
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
    let route = state.store.route("claude", Some(&session.id))?;
    let fingerprint = route.fingerprint();
    let mut config = route.config;
    let model = crate::providers::real_claude_model(&config, model.as_deref());
    let signature = json!([
        fingerprint,
        session.id,
        cwd,
        model,
        read_only,
        permission,
        effort
    ])
    .to_string();
    let mut slot = state.claude.client.lock().await;
    if let Some(c) = slot.as_ref() {
        if c.active.load(Ordering::Acquire) {
            return Err("Claude 的上一任务尚未结束".into());
        }
        if c.alive.load(Ordering::Acquire) && c.signature == signature {
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
    let native = session
        .native_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // Validate IDs before passing flags. Prompts and permission replies always travel via stdin.
    uuid::Uuid::parse_str(&native).map_err(|_| "Claude 原生会话 ID 无效")?;
    let mode = crate::client_features::claude_native_permission(if read_only {
        "read"
    } else {
        permission
    })?;
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
        "--permission-mode".into(),
        mode.into(),
        "--strict-mcp-config".into(),
        "--settings".into(),
        plugin_settings.to_string(),
        "--append-system-prompt".into(),
        crate::client_features::instructions(app),
    ];
    if permission == "full" {
        args.push("--allow-dangerously-skip-permissions".into());
    }
    if let Some(effort) = effort.filter(|v| matches!(v.as_str(), "low" | "medium" | "high" | "max"))
    {
        args.extend(["--effort".into(), effort]);
    }
    args.extend(plugin_args);
    let mcp = crate::client_features::claude_mcp(app, native_mcp)?;
    if mcp["mcpServers"].as_object().is_some_and(|v| !v.is_empty()) {
        args.extend(["--mcp-config".into(), mcp.to_string()]);
    }
    if read_only {
        args.extend(["--tools".into(), "Read,Glob,Grep".into()]);
    } else {
        args.extend(["--tools".into(), "default".into()]);
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
        signature,
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
    tauri::async_runtime::spawn(async move {
        let mut reader = BufReader::new(stderr);
        while let Ok(Some(frame)) = runtime::read_frame(&mut reader).await {
            if let Ok(text) = String::from_utf8(frame) {
                let _ = log_app.emit("runtime-log", protocol::bounded(text.trim(), 4096));
            }
        }
    });
    let mut reader = BufReader::new(output);
    turn.write(json!({"type":"control_request","request_id":"supercode-init","request":{"subtype":"initialize","hooks":{},"skills":[]}})).await?;
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
                let id = format!("claude:{}", message["request_id"].as_str().unwrap_or(""));
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
                let id = format!("claude:{}", message["request_id"].as_str().unwrap_or(""));
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
            } else if message["type"] == "result" {
                let state = reader_app.state::<AppState>();
                let result_slot = state.claude.client.lock().await;
                let u = &message["usage"];
                if u.is_object() {
                    let window = message["modelUsage"]
                        .as_object()
                        .and_then(|v| v.values().find_map(|m| m["contextWindow"].as_u64()));
                    let usage = turn.inputs.lock().unwrap().usage(&message);
                    publish(
                        &reader_app,
                        event(
                            "thread/tokenUsage/updated",
                            &turn.native,
                            &turn.id(),
                            json!({"tokenUsage":{"total":usage["total"],"last":usage["total"],"contextTokens":if stream.context_tokens>0{Some(stream.context_tokens)}else{None},"cumulative":false,"modelContextWindow":window,"costUsd":usage["costUsd"],"nativeCumulativeCostUsd":message["total_cost_usd"]}}),
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
                failure = if message["is_error"] == true {
                    Some(protocol::bounded(
                        message["result"].as_str().unwrap_or("Claude 任务执行失败"),
                        4096,
                    ))
                } else {
                    None
                };
                for notification in stream.partial(&turn.native, &turn.id()) {
                    publish(&reader_app, notification);
                }
                stream = Stream::default();
                turn.requests.lock().await.clear();
                let stopped = turn.stopped.load(Ordering::Acquire);
                if stopped {
                    failure = None;
                }
                if !stopped && failure.is_none() && !turn.inputs.lock().unwrap().pending.is_empty()
                {
                    // A text-only response can finish before the queued supplement is replayed.
                    // It is still the same client task until all submitted input has been consumed.
                    failure = Some("Claude 连接在任务完成前关闭".to_owned());
                    drop(result_slot);
                    continue;
                }
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
        let _ = turn.child.lock().await.kill().await;
        let state = reader_app.state::<AppState>();
        let mut slot = state.claude.client.lock().await;
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
    context_tokens: u64,
    message_id: String,
    text: String,
    tools: HashMap<String, Value>,
    blocks: HashMap<usize, Value>,
}
impl Stream {
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
        if !message["parent_tool_use_id"].is_null() {
            return events;
        }
        match message["type"].as_str().unwrap_or("") {
            "stream_event" => {
                let e = &message["event"];
                if e["type"] == "message_start" {
                    let u = &e["message"]["usage"];
                    self.context_tokens = u["input_tokens"].as_u64().unwrap_or(0)
                        + u["cache_read_input_tokens"].as_u64().unwrap_or(0)
                        + u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                    events.extend(self.partial(native, turn));
                    self.message_id = e["message"]["id"].as_str().unwrap_or("").into();
                    self.text.clear();
                    self.blocks.clear();
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
                        if self.tools.len() < 200 {
                            self.tools.insert(id.clone(), item.clone());
                        }
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
