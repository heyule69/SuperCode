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
type Reply = oneshot::Sender<Result<Value, String>>;
struct Client {
    pid: u32,
    input: Mutex<ChildStdin>,
    child: Mutex<Child>,
    _job: process::JobGuard,
    pending: Mutex<HashMap<String, Reply>>,
    sequence: AtomicU64,
    pi: bool,
    alive: AtomicBool,
    concurrent_prompt: AtomicBool,
}
impl Client {
    async fn start(
        launch: &Launch,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        pi: bool,
    ) -> Result<(Arc<Self>, mpsc::Receiver<Value>), String> {
        let mut command = launch.command(&[]);
        command
            .args(args)
            .current_dir(cwd)
            .envs(env.iter().cloned())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| format!("无法启动 Agent：{e}"))?;
        let job = process::JobGuard::attach(&child)?;
        let input = child.stdin.take().ok_or("Agent 无输入管道")?;
        let output = child.stdout.take().ok_or("Agent 无输出管道")?;
        let stderr = child.stderr.take().ok_or("Agent 无错误管道")?;
        let client = Arc::new(Self {
            pid: child.id().unwrap_or(0),
            input: Mutex::new(input),
            child: Mutex::new(child),
            _job: job,
            pending: Mutex::new(HashMap::new()),
            sequence: AtomicU64::new(1),
            alive: AtomicBool::new(true),
            pi,
            concurrent_prompt: AtomicBool::new(false),
        });
        let reader = client.clone();
        let (tx, rx) = mpsc::channel(16);
        // Always drain diagnostics, but never log raw provider credentials or unbounded terminal output.
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr);
            while runtime::read_frame(&mut lines)
                .await
                .is_ok_and(|v| v.is_some())
            {}
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(output);
            loop {
                let value = match runtime::read_frame(&mut lines).await {
                    Ok(Some(line)) => match serde_json::from_slice::<Value>(&line) {
                        Ok(v) => v,
                        Err(_) => continue,
                    },
                    _ => break,
                };
                if is_response(&value, pi) {
                    let key = value["id"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value["id"].to_string());
                    if let Some(reply) = reader.pending.lock().await.remove(&key) {
                        let _ = reply.send(response(&value, pi));
                    }
                    continue;
                }
                if tx.send(value).await.is_err() {
                    break;
                }
            }
            reader.alive.store(false, Ordering::Release);
            for (_, reply) in reader.pending.lock().await.drain() {
                let _ = reply.send(Err("Agent 连接已关闭".into()));
            }
            let _ = tx.send(json!({"type":"supercode_disconnected"})).await;
        });
        Ok((client, rx))
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
    async fn request(&self, method: &str, params: Value, seconds: u64) -> Result<Value, String> {
        let (id, rx) = self.begin_request(method, params).await?;
        let result = tokio::time::timeout(Duration::from_secs(seconds), rx).await;
        self.pending.lock().await.remove(&id);
        result
            .map_err(|_| format!("Agent 请求 {method} 超时"))?
            .map_err(|_| "Agent 已退出".to_owned())?
    }
    async fn begin_request(
        &self,
        method: &str,
        mut params: Value,
    ) -> Result<(String, oneshot::Receiver<Result<Value, String>>), String> {
        let id = self.sequence.fetch_add(1, Ordering::Relaxed).to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        let value = if self.pi {
            params["type"] = method.into();
            params["id"] = id.clone().into();
            params
        } else {
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        };
        if let Err(e) = self.write(value).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }
        Ok((id, rx))
    }
    async fn close(&self) {
        self.alive.store(false, Ordering::Release);
        let _ = self.child.lock().await.kill().await;
    }
}
fn is_response(v: &Value, pi: bool) -> bool {
    if pi {
        v["type"] == "response"
    } else {
        v.get("id").is_some() && v.get("method").is_none()
    }
}
fn response(v: &Value, pi: bool) -> Result<Value, String> {
    if pi {
        if v["success"] == false {
            Err(protocol::bounded(
                v["error"].as_str().unwrap_or("Pi 请求失败"),
                4096,
            ))
        } else {
            Ok(v["data"].clone())
        }
    } else if v.get("error").is_some() {
        Err(protocol::bounded(
            v["error"]["message"]
                .as_str()
                .unwrap_or("OpenCode 请求失败"),
            4096,
        ))
    } else {
        Ok(v["result"].clone())
    }
}
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
            let (client, mut events) = Client::start(launch, &args, cwd, &[], pi).await?;
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
    pub requests: Mutex<HashMap<String, Value>>,
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
}
fn publish(app: &AppHandle, value: Value) {
    runtime::publish_event(app, value);
}
fn event(t: &Turn, method: &str, mut p: Value) -> Value {
    p["threadId"] = t.native.clone().into();
    p["turnId"] = t.turn.clone().into();
    json!({"method":method,"params":p})
}
impl Runtime {
    pub async fn steering_mode(&self, session: &str) -> &'static str {
        let slot = self.client.lock().await;
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
        let slot = self.client.lock().await;
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
        if let Some(c) = self.warm.lock().await.take() {
            c.client.close().await;
        }
        if let Some(turn) = self.client.lock().await.take() {
            turn.stopped.store(true, Ordering::Relaxed);
            turn.client.close().await;
        }
    }
    pub async fn interrupt(&self, session: &str) -> Result<(), String> {
        let slot = self.client.lock().await;
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
        let t = self.client.lock().await.clone().ok_or("Agent 连接已关闭")?;
        let id = id.as_str().ok_or("请求 ID 无效")?;
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
        publish(
            app,
            event(&t, "serverRequest/resolved", json!({"requestId":id})),
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
        "{}:{}",
        t.agent,
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
    let mut value = json!({"autoupdate":false,"permission":{"*":rule}});
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
        let models = crate::providers::model_ids(config)
            .into_iter()
            .map(|m| (wire_model(config, &m), json!({"name":m})))
            .collect::<serde_json::Map<String, Value>>();
        value["provider"] = json!({"supercode":{"npm":npm,"name":"SuperCode","options":{"baseURL":sdk_base(config),"apiKey":crate::providers::key(config).unwrap_or("")},"models":models}});
    }
    value
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
    if config["protocol"] == "anthropic" {
        for suffix in ["[1m]", "[256k]"] {
            if id.to_ascii_lowercase().ends_with(suffix) {
                return id[..id.len() - suffix.len()].to_owned();
            }
        }
    }
    id.to_owned()
}
const PI_EXTENSION: &str = r#"export default function(pi) {
  const config = JSON.parse(process.env.SUPERCODE_PI_CONFIG || '{}');
  for (const [name, server] of Object.entries(config.mcp || {})) pi.registerMcpServer(name, server);
  if (config.provider) pi.registerProvider('supercode', config.provider);
  pi.on('tool_call', async (event, ctx) => {
    if (config.permission === 'summary') return {block:true,reason:'Summary does not allow tools'};
    const read = ['read', 'grep', 'find', 'ls', 'glob'].includes(event.toolName);
    if (config.permission === 'read' && !read) return {block:true,reason:'Read-only mode'};
    if (config.permission !== 'full' && (!read || config.permission === 'strict')) {
      if (!await ctx.ui.confirm('允许本次工具操作？', event.toolName + '\n' + JSON.stringify(event.input))) return {block:true,reason:'User declined'};
    }
  });
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
    let mut env = vec![];
    let mut args = vec![];
    if agent == "opencode" {
        args.push("acp".into());
        env.push((
            "OPENCODE_CONFIG_CONTENT".into(),
            opencode_config(&config, permission).to_string(),
        ));
        env.push(("OPENCODE_DISABLE_AUTOUPDATE".into(), "true".into()));
    } else {
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
        if let Some(base) = config["baseUrl"].as_str() {
            let mut seen = std::collections::HashSet::new();
            settings["provider"] = json!({"baseUrl":base,"apiKey":crate::providers::key(&config).unwrap_or(""),"api":match config["protocol"].as_str(){Some("anthropic")=>"anthropic-messages",Some("responses")=>"openai-responses",_=>"openai-completions"},"models":crate::providers::model_ids(&config).into_iter().filter_map(|m|{let id=wire_model(&config,&m);if !seen.insert(id.clone()){return None;}Some(json!({"id":id,"name":m,"reasoning":true,"input":["text","image"],"contextWindow":if m.to_ascii_lowercase().ends_with("[1m]"){1000000}else{128000},"maxTokens":8192,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}}))}).collect::<Vec<_>>()});
        }
        env.push(("SUPERCODE_PI_CONFIG".into(), settings.to_string()));
    }
    let (client, events) = Client::start(&launch, &args, cwd, &env, agent == "pi").await?;
    Ok((client, events, config))
}
pub async fn models(
    app: &AppHandle,
    agent: &str,
    session_id: Option<&str>,
) -> Result<Value, String> {
    let s = session_id
        .map(|s| app.state::<AppState>().store.session(s))
        .transpose()?;
    let route = app.state::<AppState>().store.route(agent, session_id)?;
    if route.profile.is_some() {
        let mut catalog = crate::providers::catalog_models(&route.config);
        catalog["source"] = route.source(agent);
        return Ok(catalog);
    }
    if app.state::<AppState>().store.running()? > 0 {
        return Err("请在当前任务结束后刷新本机模型".into());
    }
    let cwd = s
        .as_ref()
        .map(|s| {
            app.state::<AppState>()
                .store
                .session_workspace(s)
                .map(|p| p.path)
        })
        .transpose()?
        .unwrap_or_else(|| app.state::<AppState>().data_dir.to_string_lossy().into());
    let (client, mut rx, _) = start_for(app, agent, Path::new(&cwd), s.as_ref(), "ask").await?;
    let drain = tokio::spawn(async move {
        while let Some(v) = rx.recv().await {
            if v["type"] == "supercode_disconnected" {
                break;
            }
        }
    });
    let result=async{if agent=="pi"{
        let state=client.request("get_state",json!({}),60).await?;
        let levels=client.request("get_available_thinking_levels",json!({}),30).await?;
        let v=client.request("get_available_models",json!({}),60).await?;
        let data=v["models"].as_array().ok_or("Pi 模型列表格式无效")?.iter().map(|m| {
            let selected=m["provider"]==state["model"]["provider"]&&m["id"]==state["model"]["id"];
            let mut row=json!({"id":format!("{}/{}",m["provider"].as_str().unwrap_or(""),m["id"].as_str().unwrap_or("")),"model":format!("{}/{}",m["provider"].as_str().unwrap_or(""),m["id"].as_str().unwrap_or("")),"displayName":m["name"],"contextWindow":m["contextWindow"],"isDefault":selected});
            if selected { row["defaultReasoningEffort"]=state["thinkingLevel"].clone(); row["supportedReasoningEfforts"]=json!(levels["levels"].as_array().into_iter().flatten().filter_map(|v|v.as_str()).map(|level|json!({"reasoningEffort":level})).collect::<Vec<_>>()); }
            row
        }).collect::<Vec<_>>();Ok(json!({"data":data,"source":route.source(agent)}))}else{
        client.request("initialize",init(),60).await?;let v=client.request("session/new",json!({"cwd":cwd,"mcpServers":[]}),90).await?;let data=v["models"]["availableModels"].as_array().ok_or("OpenCode 未返回模型列表，请先配置本机供应商")?.iter().map(|m|json!({"id":m["modelId"],"model":m["modelId"],"displayName":m["name"],"isDefault":m["modelId"]==v["models"]["currentModelId"]})).collect::<Vec<_>>();Ok(json!({"data":data,"source":route.source(agent)}))}}.await;
    client.close().await;
    drain.abort();
    result
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
    let mut slot = state.native.client.lock().await;
    if slot.is_some() {
        return Err("上一任务尚未结束".into());
    }
    let route = state.store.route(&s.agent, Some(&s.id))?;
    let signature = json!([
        route.fingerprint(),
        s.id,
        cwd,
        model.as_ref().or(s.model.as_ref()),
        permission,
        effort
    ])
    .to_string();
    let cached = state.native.warm.lock().await.take();
    let cached = if let Some(c) = cached {
        if c.session == s.id && c.signature == signature && c.client.alive.load(Ordering::Acquire) {
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
        requests: Mutex::new(HashMap::new()),
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
            state.native.client.lock().await.take();
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
        state.native.client.lock().await.take();
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
    let turn = Arc::new(Turn {
        client: client.clone(),
        requests: Mutex::new(HashMap::new()),
        session: s.id.clone(),
        native,
        turn: boot.turn.clone(),
        agent: s.agent.clone(),
        stopped: AtomicBool::new(false),
        accepting: AtomicBool::new(true),
        inputs_pending: AtomicU64::new(0),
        input_done: Notify::new(),
        settled: AtomicBool::new(false),
        prompt_accepted: AtomicBool::new(false),
        prompt_ready: Notify::new(),
        started: AtomicBool::new(false),
        stop_done: AtomicBool::new(true),
        signature: signature.clone(),
    });
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
    let mut slot = state.native.client.lock().await;
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
    let app = app.clone();
    tokio::spawn(async move {
        let mut mapped = Mapper {
            priced: !config["baseUrl"].is_string(),
            ..Mapper::default()
        };
        let mut completed = false;
        let mut protocol_finished = false;
        let mut drained = false;
        let mut failure = None;
        let prompt_timeout = tokio::time::sleep(Duration::from_secs(3600));
        tokio::pin!(prompt_timeout);
        loop {
            tokio::select! {
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
                let state = app.state::<AppState>();
                let slot = state.native.client.lock().await;
                turn.accepting.store(false, Ordering::Release);
                if turn.agent == "pi" {
                    let state_request = client.request("get_state", json!({}), 30);
                    tokio::pin!(state_request);
                    let native_state = loop {
                        tokio::select! {
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
        let state = app.state::<AppState>();
        let mut slot = state.native.client.lock().await;
        if slot.as_ref().is_some_and(|v| Arc::ptr_eq(v, &turn)) {
            slot.take();
            if drained
                && matches!(
                    completion["status"].as_str(),
                    Some("completed" | "interrupted")
                )
                && client.alive.load(Ordering::Acquire)
            {
                *state.native.warm.lock().await = Some(Warm {
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
        let t=Turn { client:client.clone(),requests:Mutex::new(HashMap::new()),session:session.id.clone(),native,turn:format!("handoff-{}",uuid::Uuid::new_v4()),agent:session.agent.clone(),stopped:AtomicBool::new(false),accepting:AtomicBool::new(false),inputs_pending:AtomicU64::new(0),input_done:Notify::new(),settled:AtomicBool::new(false),prompt_accepted:AtomicBool::new(false),prompt_ready:Notify::new(),started:AtomicBool::new(false),stop_done:AtomicBool::new(true),signature:String::new() };
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
#[derive(Default)]
struct Mapper {
    items: HashMap<String, Value>,
    message: u64,
    failed: bool,
    failure: Option<String>,
    usage: Value,
    priced: bool,
    acp_phase: String,
    acp_segment: u64,
    media_segment: u64,
}
impl Mapper {
    fn map(&mut self, t: &Turn, v: &Value) -> Vec<Value> {
        let mut out = vec![];
        let mut item = None;
        let mut delta = None;
        if t.agent == "pi" {
            match v["type"].as_str().unwrap_or("") {
                "message_start" if v["message"]["role"] == "assistant" => self.message += 1,
                "message_update" => {
                    let e = &v["assistantMessageEvent"];
                    let index = e["contentIndex"].as_u64().unwrap_or(0);
                    let id = format!("{}-msg-{}-{index}", t.turn, self.message);
                    match e["type"].as_str().unwrap_or("") {
                        "text_start" | "text_delta" | "thinking_start" | "thinking_delta" => {
                            let thought = e["type"].as_str().unwrap().starts_with("thinking");
                            let row=self.items.entry(id.clone()).or_insert_with(||if thought{json!({"id":id,"type":"reasoning","summary":[""],"status":"inProgress"})}else{json!({"id":id,"type":"agentMessage","text":"","status":"inProgress"})});
                            let old = if thought {
                                row["summary"][0].as_str().unwrap_or("")
                            } else {
                                row["text"].as_str().unwrap_or("")
                            };
                            let text = protocol::bounded(
                                &format!("{old}{}", e["delta"].as_str().unwrap_or("")),
                                protocol::TEXT_LIMIT,
                            );
                            if thought {
                                row["summary"][0] = text.into();
                            } else {
                                row["text"] = text.into();
                            }
                            item = Some(row.clone());
                        }
                        _ => {}
                    }
                }
                "message_end" if v["message"]["role"] == "assistant" => {
                    let m = &v["message"];
                    self.failed |= m["stopReason"] == "error";
                    if let Some(blocks) = m["content"].as_array() {
                        for (i, b) in blocks.iter().enumerate() {
                            let id = format!("{}-msg-{}-{i}", t.turn, self.message);
                            let row = match b["type"].as_str() {
                                Some("text") => {
                                    json!({"id":id,"type":"agentMessage","text":b["text"],"status":"completed"})
                                }
                                Some("thinking") => {
                                    json!({"id":id,"type":"reasoning","summary":[b["thinking"]],"status":"completed"})
                                }
                                Some("image" | "audio" | "video") => {
                                    json!({"id":id,"type":"agentMessage","text":"","contentItems":[b],"status":"completed"})
                                }
                                _ => continue,
                            };
                            self.items.remove(&id);
                            out.push(event(t, "item/completed", json!({"item":row})));
                        }
                    }
                    if let Some(error) = m["errorMessage"].as_str() {
                        self.failure = Some(protocol::bounded(error, 4096));
                        out.push(event(t, "error", json!({"error":{"message":error}})));
                    }
                    if m["usage"].is_object() {
                        out.push(event(
                            t,
                            "thread/tokenUsage/updated",
                            json!({"tokenUsage":self.add_usage(&m["usage"])}),
                        ));
                    }
                }
                "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
                    let id = format!(
                        "{}-tool-{}",
                        t.turn,
                        v["toolCallId"].as_str().unwrap_or("unknown")
                    );
                    let end = v["type"] == "tool_execution_end";
                    let row=self.items.entry(id.clone()).or_insert(json!({"id":id,"type":"claudeToolCall","tool":pi_tool(v["toolName"].as_str().unwrap_or("Tool")),"arguments":tool_args(&v["args"]),"status":"inProgress"}));
                    if end {
                        row["output"] = blocks_text(&v["result"]["content"]).into();
                        row["contentItems"] = v["result"]["content"].clone();
                        row["status"] = if v["isError"] == true {
                            "failed"
                        } else {
                            "completed"
                        }
                        .into();
                    }
                    item = Some(row.clone());
                }
                "compaction_start" => {
                    item = Some(
                        json!({"id":format!("{}-compact",t.turn),"type":"contextCompaction","status":"inProgress"}),
                    )
                }
                "compaction_end" => {
                    item = Some(
                        json!({"id":format!("{}-compact",t.turn),"type":"contextCompaction","status":"completed"}),
                    )
                }
                _ => {}
            }
        } else if v["method"] == "session/update" && v["params"]["sessionId"] == t.native {
            let u = &v["params"]["update"];
            match u["sessionUpdate"].as_str().unwrap_or("") {
                "agent_message_chunk" | "agent_thought_chunk" => {
                    let thought = u["sessionUpdate"] == "agent_thought_chunk";
                    if !thought
                        && matches!(
                            u["content"]["type"].as_str(),
                            Some("image" | "audio" | "video" | "resource" | "resource_link")
                        )
                    {
                        self.media_segment += 1;
                        out.push(event(t, "item/completed", json!({"item":{"id":format!("{}-media-{}",t.turn,self.media_segment),"type":"agentMessage","text":"","contentItems":[u["content"]],"status":"completed"}})));
                        return out;
                    }
                    let phase = if thought { "thought" } else { "msg" };
                    if u["messageId"].as_str().is_none() && self.acp_phase != phase {
                        let old = self
                            .items
                            .iter()
                            .filter(|(_, row)| {
                                matches!(row["type"].as_str(), Some("agentMessage" | "reasoning"))
                            })
                            .map(|(id, _)| id.clone())
                            .collect::<Vec<_>>();
                        for id in old {
                            if let Some(mut row) = self.items.remove(&id) {
                                row["status"] = "completed".into();
                                out.push(event(t, "item/completed", json!({"item":row})));
                            }
                        }
                        self.acp_segment += 1;
                        self.acp_phase = phase.into();
                    }
                    let id = format!(
                        "{}-{}-{}",
                        t.turn,
                        if thought { "thought" } else { "msg" },
                        u["messageId"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| self.acp_segment.to_string())
                    );
                    let row = self.items.entry(id.clone()).or_insert_with(|| {
                        if thought {
                            json!({"id":id,"type":"reasoning","summary":[""],"status":"inProgress"})
                        } else {
                            json!({"id":id,"type":"agentMessage","text":"","status":"inProgress"})
                        }
                    });
                    let d = u["content"]["text"].as_str().unwrap_or("");
                    let old = if thought {
                        row["summary"][0].as_str().unwrap_or("")
                    } else {
                        row["text"].as_str().unwrap_or("")
                    };
                    let text = protocol::bounded(&format!("{old}{d}"), protocol::TEXT_LIMIT);
                    if thought {
                        row["summary"][0] = text.into();
                    } else {
                        row["text"] = text.into();
                    }
                    delta = Some(event(
                        t,
                        if thought {
                            "item/reasoning/summaryTextDelta"
                        } else {
                            "item/agentMessage/delta"
                        },
                        json!({"itemId":id,"delta":d,"summaryIndex":0}),
                    ));
                    item = Some(row.clone());
                }
                "tool_call" | "tool_call_update" => {
                    self.acp_phase = "tool".into();
                    let old = self
                        .items
                        .iter()
                        .filter(|(_, row)| {
                            matches!(row["type"].as_str(), Some("agentMessage" | "reasoning"))
                        })
                        .map(|(id, _)| id.clone())
                        .collect::<Vec<_>>();
                    for id in old {
                        if let Some(mut row) = self.items.remove(&id) {
                            row["status"] = "completed".into();
                            out.push(event(t, "item/completed", json!({"item":row})));
                        }
                    }
                    let id = format!(
                        "{}-tool-{}",
                        t.turn,
                        u["toolCallId"].as_str().unwrap_or("unknown")
                    );
                    let row=self.items.entry(id.clone()).or_insert(json!({"id":id,"type":"claudeToolCall","tool":acp_tool(u),"arguments":tool_args(&u["rawInput"]),"status":"inProgress"}));
                    if u["kind"].is_string() {
                        row["tool"] = acp_tool(u).into();
                    }
                    if let Some(c) = u.get("rawInput") {
                        row["arguments"] = tool_args(c);
                    }
                    if let Some(c) = u.get("content") {
                        row["output"] = blocks_text(c).into();
                        row["contentItems"] = c.clone();
                    }
                    if matches!(u["status"].as_str(), Some("completed" | "failed")) {
                        row["status"] = u["status"].clone();
                    }
                    item = Some(row.clone());
                }
                "plan" => {
                    item = Some(
                        json!({"id":format!("{}-plan",t.turn),"type":"executionPlan","text":u["entries"].to_string(),"status":"inProgress"}),
                    )
                }
                "usage_update" => {
                    out.push(event(t,"thread/tokenUsage/updated",json!({"tokenUsage":{"total":{},"last":{},"contextTokens":u["used"],"modelContextWindow":u["size"]}})));
                }
                _ => {}
            }
        }
        if let Some(mut row) = item {
            row["turnId"] = t.turn.clone().into();
            let method = if matches!(row["status"].as_str(), Some("completed" | "failed")) {
                self.items.remove(row["id"].as_str().unwrap_or(""));
                "item/completed"
            } else {
                "item/started"
            };
            out.push(event(t, method, json!({"item":row})));
        }
        if let Some(delta) = delta {
            /* complete snapshots above are authoritative; avoid appending duplicate text */
            let _ = delta;
        }
        out
    }
    fn finish(&mut self, interrupted: bool) -> Vec<Value> {
        self.items
            .drain()
            .map(|(_, mut v)| {
                v["status"] = if interrupted {
                    "interrupted"
                } else {
                    "completed"
                }
                .into();
                v
            })
            .collect()
    }
    fn add_usage(&mut self, native: &Value) -> Value {
        let last = pi_usage(native);
        if !self.usage.is_object() {
            self.usage = json!({"total":{},"last":{},"contextTokens":0});
        }
        for key in [
            "inputTokens",
            "outputTokens",
            "cachedInputTokens",
            "cacheWriteInputTokens",
            "totalTokens",
        ] {
            self.usage["total"][key] = json!(self.usage["total"][key]
                .as_u64()
                .unwrap_or(0)
                .saturating_add(last[key].as_u64().unwrap_or(0)));
        }
        self.usage["last"] = last;
        self.usage["contextTokens"] = json!(
            native["input"].as_u64().unwrap_or(0)
                + native["cacheRead"].as_u64().unwrap_or(0)
                + native["cacheWrite"].as_u64().unwrap_or(0)
        );
        if self.priced {
            if let Some(cost) = native["cost"]["total"].as_f64() {
                self.usage["costUsd"] = json!(self.usage["costUsd"].as_f64().unwrap_or(0.0) + cost);
            }
        }
        self.usage.clone()
    }
}
fn acp_tool(v: &Value) -> &str {
    match v["kind"].as_str() {
        Some("read") => "Read",
        Some("edit") => "Edit",
        Some("execute") => "Bash",
        Some("search") => "Grep",
        _ => v["title"].as_str().unwrap_or("Tool"),
    }
}
fn blocks_text(v: &Value) -> String {
    let text = v
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|b| b["text"].as_str().or_else(|| b["content"]["text"].as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    protocol::bounded(&text, protocol::TEXT_LIMIT)
}
fn pi_usage(v: &Value) -> Value {
    json!({"inputTokens":v["input"],"outputTokens":v["output"],"cachedInputTokens":v["cacheRead"],"cacheWriteInputTokens":v["cacheWrite"],"totalTokens":v["totalTokens"]})
}
fn pi_tool(name: &str) -> &str {
    match name {
        "read" => "Read",
        "bash" => "Bash",
        "write" => "Write",
        "edit" => "Edit",
        "grep" => "Grep",
        "find" | "ls" => "Glob",
        _ => name,
    }
}
fn tool_args(v: &Value) -> Value {
    let mut args = v.clone();
    if let Some(path) = v["file_path"]
        .as_str()
        .or(v["filePath"].as_str())
        .or(v["path"].as_str())
    {
        args["file_path"] = path.into();
    }
    args
}
#[cfg(test)]
mod tests {
    use super::*;
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
    fn usage_accumulates_all_native_messages_but_never_claims_custom_models_are_free() {
        let mut m = Mapper::default();
        let usage = json!({"input":10,"output":4,"cacheRead":5,"cacheWrite":2,"totalTokens":21,"cost":{"total":0.01}});
        m.add_usage(&usage);
        let total = m.add_usage(&usage);
        assert_eq!(total["total"]["totalTokens"], 42);
        assert_eq!(total["total"]["cacheWriteInputTokens"], 4);
        assert_eq!(total["last"]["inputTokens"], 10);
        assert_eq!(total["contextTokens"], 17);
        assert!(total.get("costUsd").is_none());
        let mut native = Mapper {
            priced: true,
            ..Mapper::default()
        };
        native.add_usage(&usage);
        assert_eq!(native.add_usage(&usage)["costUsd"], 0.02);
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
