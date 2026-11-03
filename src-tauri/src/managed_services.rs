//! Explicit, application-owned services. The approved native shell invokes the
//! controller, but the GUI starts the service outside the agent's Windows job.
use crate::process;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{Emitter, Manager as _};
use tokio::{
    io::AsyncReadExt,
    process::Child,
    sync::{oneshot, Mutex as AsyncMutex},
};

const BODY_LIMIT: usize = 48 * 1024;
const LOG_LIMIT: usize = 256 * 1024;
const MAX_RUNNING: usize = 8;
const MAX_RECORDS: usize = 16;
const FORMAT: &str = "supercode-services-v1";

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Request {
    Start {
        name: String,
        program: PathBuf,
        cwd: PathBuf,
        args: Vec<String>,
    },
    List {},
    Stop {
        id: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Endpoint {
    format: String,
    port: u16,
    token: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    session_id: Option<String>,
    request: Request,
}

#[derive(Default)]
struct Approvals {
    pending: Mutex<std::collections::HashMap<String, PendingApproval>>,
}
struct PendingApproval {
    event: Value,
    reply: oneshot::Sender<bool>,
}
struct ApprovalScope {
    queue: Arc<Approvals>,
    id: String,
    app: Option<tauri::AppHandle>,
}
impl Drop for ApprovalScope {
    fn drop(&mut self) {
        self.queue.pending.lock().unwrap().remove(&self.id);
        if let Some(app) = &self.app {
            let _ = app.emit(
                "agent-event",
                json!({"method":"serverRequest/resolved","params":{"requestId":self.id}}),
            );
        }
    }
}
impl Approvals {
    fn events(&self) -> Vec<Value> {
        self.pending
            .lock()
            .unwrap()
            .values()
            .map(|pending| pending.event.clone())
            .collect()
    }
    fn respond(&self, id: &str, result: &Value) -> Result<(), String> {
        let accepted = match result["decision"].as_str() {
            Some("accept") => true,
            Some("decline") => false,
            _ => return Err("服务托管只支持允许本次或拒绝".into()),
        };
        let pending = self
            .pending
            .lock()
            .unwrap()
            .remove(id)
            .ok_or("该服务请求已结束")?;
        pending
            .reply
            .send(accepted)
            .map_err(|_| "该服务请求已取消".into())
    }
    fn cancel_turn(&self, session: &str, turn: Option<&str>) {
        let mut pending = self.pending.lock().unwrap();
        let ids = pending
            .iter()
            .filter(|(_, approval)| {
                approval.event["params"]["appSessionId"] == session
                    && (turn.is_none() || approval.event["params"]["turnId"].as_str() == turn)
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            pending.remove(&id);
        }
    }
    async fn confirm(
        self: &Arc<Self>,
        app: Option<tauri::AppHandle>,
        session: &str,
        turn: Option<&str>,
        request: &Request,
    ) -> Result<(), String> {
        let id = format!("service-approval:{}", uuid::Uuid::new_v4());
        let (title, description, cwd) = match request {
            Request::Start {
                program, cwd, args, ..
            } => (
                "启动独立服务",
                json!({"program":program,"args":args}),
                Some(cwd),
            ),
            Request::Stop { id } => ("停止独立服务", json!({"serviceId":id}), None),
            Request::List {} => return Ok(()),
        };
        let event = json!({"id":id,"method":"item/commandExecution/requestApproval","params":{
            "appSessionId":session,"turnId":turn,"toolName":title,"cwd":cwd,
            "reason":"这是在 Agent 沙箱之外独立托管的服务。请核对程序与参数；允许后，服务会运行到显式停止或关闭 SuperCode。此授权仅限当前操作，不更改 Agent 权限。",
            "command":serde_json::to_string_pretty(&description).unwrap(),"availableDecisions":["accept","decline"]}});
        let (reply, receiver) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            if pending.len() >= MAX_RUNNING {
                return Err("服务审批请求已满，请先处理已有请求".into());
            }
            pending.insert(
                id.clone(),
                PendingApproval {
                    event: event.clone(),
                    reply,
                },
            );
        }
        let _scope = ApprovalScope {
            queue: self.clone(),
            id,
            app: app.clone(),
        };
        if let Some(app) = app {
            app.emit("agent-event", event)
                .map_err(|_| "无法展示服务审批，未启动服务")?;
        }
        match tokio::time::timeout(Duration::from_secs(300), receiver).await {
            Ok(Ok(true)) => Ok(()),
            Ok(Ok(false)) => Err("用户已拒绝服务操作，未改变服务".into()),
            _ => Err("服务审批已超时或取消，未执行操作".into()),
        }
    }
}

struct Entry {
    id: String,
    name: String,
    cwd: PathBuf,
    program: String,
    signature: String,
    child: Child,
    job: Option<process::JobGuard>,
    state: &'static str,
    log: PathBuf,
    pumps: Vec<tokio::task::JoinHandle<()>>,
}
impl Entry {
    fn refresh(&mut self) -> Result<(), String> {
        if self.state == "running"
            && self
                .child
                .try_wait()
                .map_err(|_| "无法检查服务进程")?
                .is_some()
        {
            self.state = "exited";
            self.job.take();
        }
        Ok(())
    }
    fn value(&self) -> Value {
        json!({"id":self.id,"name":self.name,"program":self.program,"cwd":self.cwd,
            "pid":self.child.id(),"state":self.state,"logPath":self.log,
            "lifetime":"until explicitly stopped or SuperCode exits",
            "note":"进程运行不代表网址已可访问，请另行检查实际网址。日志为有界原始字节文件。"})
    }
}

struct ServiceManager {
    logs: PathBuf,
    entries: AsyncMutex<Vec<Entry>>,
}
impl ServiceManager {
    async fn execute(&self, request: Request) -> Result<Value, String> {
        let mut entries = self.entries.lock().await;
        for entry in entries.iter_mut() {
            entry.refresh()?;
        }
        match request {
            Request::List {} => {
                Ok(json!({"services":entries.iter().map(Entry::value).collect::<Vec<_>>()}))
            }
            Request::Stop { id } => {
                let entry = entries
                    .iter_mut()
                    .find(|entry| entry.id == id)
                    .ok_or("未找到此软件托管的服务，不会按 PID 停止其他进程")?;
                if entry.state == "running" {
                    // Only this entry's owned job is closed. Never kill by name or PID.
                    entry.job.take();
                    let _ = entry.child.start_kill();
                    tokio::time::timeout(Duration::from_secs(3), entry.child.wait())
                        .await
                        .map_err(|_| "服务停止尚未确认，请查询状态")?
                        .map_err(|_| "无法确认服务退出")?;
                    entry.state = "stopped";
                    for pump in entry.pumps.drain(..) {
                        let _ = tokio::time::timeout(Duration::from_secs(1), pump).await;
                    }
                }
                Ok(entry.value())
            }
            Request::Start {
                name,
                program,
                cwd,
                args,
            } => {
                let (program, cwd, signature) = validate_launch(&name, &program, &cwd, &args)?;
                if !cfg!(windows) {
                    return Err("持久服务托管当前仅支持 Windows".into());
                }
                if let Some(old) = entries.iter().find(|entry| {
                    entry.state == "running" && entry.name == name && entry.cwd == cwd
                }) {
                    return if old.signature == signature {
                        Ok(old.value())
                    } else {
                        Err("同名服务仍在运行，请确认后显式停止旧服务，再更换启动参数".into())
                    };
                }
                if entries
                    .iter()
                    .filter(|entry| entry.state == "running")
                    .count()
                    >= MAX_RUNNING
                {
                    return Err("最多同时托管 8 个服务，请先停止不再需要的服务".into());
                }
                while entries.len() >= MAX_RECORDS {
                    let index = entries
                        .iter()
                        .position(|entry| entry.state != "running")
                        .ok_or("服务记录已满")?;
                    entries.remove(index);
                }
                let id = uuid::Uuid::new_v4().to_string();
                let log_path = self.logs.join(format!("{id}.bin"));
                let log = Arc::new(Mutex::new(BoundedLog::new(log_path.clone())?));
                let mut command = process::command(&program, &[]);
                // Separate literal arguments, never a shell command assembled from input.
                let mut child = command
                    .args(&args)
                    .current_dir(&cwd)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .map_err(|_| "无法启动托管服务，请检查程序路径、参数和工作目录")?;
                let job = match process::JobGuard::attach(&child) {
                    Ok(job) => job,
                    Err(_) => {
                        let _ = child.kill().await;
                        return Err("无法建立服务进程组，已停止本次启动的进程".into());
                    }
                };
                let mut pumps = Vec::new();
                if let Some(stdout) = child.stdout.take() {
                    pumps.push(drain(stdout, log.clone()));
                }
                if let Some(stderr) = child.stderr.take() {
                    pumps.push(drain(stderr, log));
                }
                let entry = Entry {
                    id,
                    name,
                    program: program
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into(),
                    cwd,
                    signature,
                    child,
                    job: Some(job),
                    state: "running",
                    log: log_path,
                    pumps,
                };
                let result = entry.value();
                entries.push(entry);
                prune_logs(
                    &self.logs,
                    &entries.iter().map(|entry| entry.id.clone()).collect(),
                );
                Ok(result)
            }
        }
    }
}

fn validate_launch(
    name: &str,
    program: &Path,
    cwd: &Path,
    args: &[String],
) -> Result<(PathBuf, PathBuf, String), String> {
    if name.trim().is_empty()
        || name.len() > 128
        || name.chars().any(char::is_control)
        || args.len() > 64
        || args
            .iter()
            .any(|arg| arg.contains('\0') || arg.len() > 8192)
        || args.iter().map(String::len).sum::<usize>() > 32768
    {
        return Err("服务名称或参数超出限制".into());
    }
    if !program.is_absolute() || !cwd.is_absolute() {
        return Err("程序和工作目录必须使用绝对路径".into());
    }
    let program = program.canonicalize().map_err(|_| "服务程序不存在")?;
    let cwd = cwd.canonicalize().map_err(|_| "服务工作目录不存在")?;
    if !program.is_file() || !cwd.is_dir() {
        return Err("服务程序或工作目录类型无效".into());
    }
    if cfg!(windows)
        && !program
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return Err("请直接使用 node.exe、python.exe、ssh.exe 等程序；不拼接执行 cmd/bat/PowerShell 命令字符串。脚本可通过解释器的独立参数运行。".into());
    }
    let signature = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&json!([&program, &cwd, args])).unwrap())
    );
    Ok((program, cwd, signature))
}

struct BoundedLog {
    path: PathBuf,
    file: Option<std::fs::File>,
    bytes: usize,
}
impl BoundedLog {
    fn new(path: PathBuf) -> Result<Self, String> {
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|_| "无法创建服务日志")?;
        Ok(Self {
            path,
            file: Some(file),
            bytes: 0,
        })
    }
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if self.bytes + bytes.len() > LOG_LIMIT {
            self.file.take();
            let previous = self.path.with_extension("previous.bin");
            match std::fs::remove_file(&previous) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            std::fs::rename(&self.path, &previous)?;
            self.file = Some(
                std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&self.path)?,
            );
            self.bytes = 0;
        }
        if let Some(file) = &mut self.file {
            file.write_all(bytes)?;
            self.bytes += bytes.len();
        }
        Ok(())
    }
}
fn drain(
    mut stream: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    log: Arc<Mutex<BoundedLog>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = [0; 8192];
        loop {
            match stream.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(bytes) => {
                    if let Ok(mut log) = log.lock() {
                        let _ = log.write(&buffer[..bytes]);
                    }
                }
            }
        }
    })
}
fn prune_logs(root: &Path, protected: &HashSet<String>) {
    let Ok(files) = std::fs::read_dir(root) else {
        return;
    };
    let mut owned = files
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let id = name
                .strip_suffix(".previous.bin")
                .or_else(|| name.strip_suffix(".bin"))?;
            if uuid::Uuid::parse_str(id).is_err() || protected.contains(id) {
                return None;
            }
            let metadata = std::fs::symlink_metadata(entry.path()).ok()?;
            metadata
                .file_type()
                .is_file()
                .then(|| (metadata.modified().ok(), entry.path()))
        })
        .collect::<Vec<_>>();
    owned.sort_by_key(|(modified, _)| *modified);
    let remove = owned.len().saturating_sub(32);
    for (_, path) in owned.into_iter().take(remove) {
        let _ = std::fs::remove_file(path);
    }
}

#[derive(Clone)]
struct Bridge {
    token: String,
    manager: Arc<ServiceManager>,
    approvals: Arc<Approvals>,
    app: Option<tauri::AppHandle>,
}
impl Bridge {
    async fn execute(&self, envelope: Envelope) -> Result<Value, String> {
        if !matches!(envelope.request, Request::List {}) {
            let session = envelope
                .session_id
                .as_deref()
                .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                .ok_or("服务操作必须从对应聊天发起，并由用户确认")?;
            let original_turn = if let Some(app) = &self.app {
                let current = app.state::<crate::AppState>().store.session(session)?;
                if !matches!(current.status.as_str(), "starting" | "running" | "waiting") {
                    return Err("原任务已结束，未执行服务操作".into());
                }
                current.turn_id
            } else {
                None
            };
            // Validate before presenting an approval, but launch only after the
            // explicit GUI response. A shell invocation alone is not proof of
            // permission to run outside that agent's sandbox.
            if let Request::Start {
                name,
                program,
                cwd,
                args,
            } = &envelope.request
            {
                validate_launch(name, program, cwd, args)?;
            }
            self.approvals
                .confirm(
                    self.app.clone(),
                    session,
                    original_turn.as_deref(),
                    &envelope.request,
                )
                .await?;
            if let Some(app) = &self.app {
                let current = app.state::<crate::AppState>().store.session(session)?;
                if current.turn_id != original_turn
                    || !matches!(current.status.as_str(), "starting" | "running" | "waiting")
                {
                    return Err("任务已停止或切换，未执行旧的服务请求".into());
                }
            }
        }
        self.manager.execute(envelope.request).await
    }
}
async fn handle(
    State(bridge): State<Bridge>,
    headers: HeaderMap,
    Json(request): Json<Envelope>,
) -> Result<Json<Value>, StatusCode> {
    if headers.contains_key("origin")
        || headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            != Some(format!("Bearer {}", bridge.token).as_str())
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(Json(match bridge.execute(request).await {
        Ok(result) => json!({"ok":true,"result":result}),
        Err(error) => json!({"ok":false,"error":error}),
    }))
}
fn router(bridge: Bridge) -> axum::Router {
    axum::Router::new()
        .route("/service", axum::routing::post(handle))
        .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(bridge)
}

#[derive(Default)]
pub struct Services {
    running: Mutex<Option<Running>>,
    approvals: Arc<Approvals>,
}
struct Running {
    task: tauri::async_runtime::JoinHandle<()>,
    manager: Arc<ServiceManager>,
    descriptor: PathBuf,
    token: String,
}

fn service_directory(data: &Path) -> Result<PathBuf, String> {
    let directory = data.join("services");
    let logs = directory.join("logs");
    for path in [&directory, &logs] {
        std::fs::create_dir_all(path).map_err(|_| "无法创建服务托管目录")?;
        let metadata = std::fs::symlink_metadata(path).map_err(|_| "无法读取服务托管目录")?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("服务托管目录不能使用重解析点".into());
            }
        }
        if !metadata.file_type().is_dir() {
            return Err("服务托管目录不能使用链接".into());
        }
    }
    Ok(directory)
}
pub fn initialize(app: &tauri::AppHandle, data: &Path) -> Result<(), String> {
    let directory = service_directory(data)?;
    let manager = Arc::new(ServiceManager {
        logs: directory.join("logs"),
        entries: AsyncMutex::new(Vec::new()),
    });
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").map_err(|_| "无法连接服务托管入口")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "无法初始化服务托管入口")?;
    let port = listener
        .local_addr()
        .map_err(|_| "无法读取服务托管地址")?
        .port();
    let token = uuid::Uuid::new_v4().to_string();
    let descriptor = directory.join("bridge.json");
    std::fs::write(
        &descriptor,
        serde_json::to_vec(&Endpoint {
            format: FORMAT.into(),
            port,
            token: token.clone(),
        })
        .unwrap(),
    )
    .map_err(|_| "无法保存服务托管入口")?;
    crate::runtime_diagnostics::register_secrets(app, &json!({"token":token}));
    let routes = router(Bridge {
        token: token.clone(),
        manager: manager.clone(),
        approvals: app.state::<Services>().approvals.clone(),
        app: Some(app.clone()),
    });
    let task = tauri::async_runtime::spawn(async move {
        if let Ok(listener) = tokio::net::TcpListener::from_std(listener) {
            let _ = axum::serve(listener, routes).await;
        }
    });
    *app.state::<Services>().running.lock().unwrap() = Some(Running {
        task,
        manager,
        descriptor,
        token,
    });
    Ok(())
}
pub fn shutdown(app: &tauri::AppHandle) {
    app.state::<Services>()
        .approvals
        .pending
        .lock()
        .unwrap()
        .clear();
    if let Some(running) = app.state::<Services>().running.lock().unwrap().take() {
        running.task.abort();
        if let Ok(mut entries) = running.manager.entries.try_lock() {
            for entry in entries.iter_mut() {
                entry.job.take();
                let _ = entry.child.start_kill();
            }
        }
        if let Ok(bytes) = std::fs::read(&running.descriptor) {
            if serde_json::from_slice::<Endpoint>(&bytes)
                .is_ok_and(|endpoint| endpoint.token == running.token)
            {
                let _ = std::fs::remove_file(running.descriptor);
            }
        }
    }
}
pub fn pending(app: &tauri::AppHandle) -> Vec<Value> {
    app.state::<Services>().approvals.events()
}
pub fn respond(app: &tauri::AppHandle, id: &str, result: &Value) -> Result<(), String> {
    app.state::<Services>().approvals.respond(id, result)
}
pub fn observe(app: &tauri::AppHandle, message: &Value) {
    if message["method"] == "turn/completed" {
        if let Some(session) = message["params"]["appSessionId"].as_str() {
            app.state::<Services>().approvals.cancel_turn(
                session,
                message["params"]["turn"]["id"]
                    .as_str()
                    .or(message["params"]["turnId"].as_str()),
            );
        }
    }
}
pub async fn reap(app: &tauri::AppHandle) {
    let manager = app
        .state::<Services>()
        .running
        .lock()
        .unwrap()
        .as_ref()
        .map(|running| running.manager.clone());
    if let Some(manager) = manager {
        for entry in manager.entries.lock().await.iter_mut() {
            let _ = entry.refresh();
        }
    }
}

pub fn configure(command: &mut tokio::process::Command, data: &Path) {
    if let Ok(executable) = std::env::current_exe() {
        command.env("SUPERCODE_SERVICE_EXE", executable);
    }
    command.env("SUPERCODE_SERVICE_DATA_DIR", data);
}
async fn control(data: &Path, request: Request) -> Result<Value, String> {
    let path = data.join("services/bridge.json");
    let file =
        std::fs::File::open(&path).map_err(|_| "服务托管入口不可用，请先打开对应的 SuperCode")?;
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取服务托管入口")?;
    if bytes.len() > 4096 {
        return Err("服务托管入口无效".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "服务托管入口不是 UTF-8，未转换或修改")?;
    let endpoint: Endpoint = serde_json::from_str(text).map_err(|_| "服务托管入口无效")?;
    if endpoint.format != FORMAT
        || endpoint.port == 0
        || uuid::Uuid::parse_str(&endpoint.token).is_err()
    {
        return Err("服务托管入口无效".into());
    }
    let envelope = Envelope {
        session_id: std::env::var("SUPERCODE_SESSION_ID").ok(),
        request,
    };
    control_envelope(endpoint, envelope).await
}
async fn control_envelope(endpoint: Endpoint, envelope: Envelope) -> Result<Value, String> {
    let mut response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(310))
        .build()
        .map_err(|_| "无法初始化服务控制")?
        .post(format!("http://127.0.0.1:{}/service", endpoint.port))
        .bearer_auth(endpoint.token)
        .json(&envelope)
        .send()
        .await
        .map_err(|_| "服务控制未返回确认；启动可能已执行，请先查询 list，勿重复启动")?;
    if !response.status().is_success() {
        return Err("服务控制请求被拒绝，未改变其他进程".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "无法读取服务控制结果")? {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err("服务控制结果超出限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "服务控制结果无效")?;
    if value["ok"] != true {
        return Err(value["error"].as_str().unwrap_or("服务操作失败").into());
    }
    Ok(value["result"].clone())
}
pub fn run_controller(data: Option<&Path>) -> Result<(), String> {
    let data = data.ok_or("服务控制需要 --user-data-dir 指向当前 SuperCode 数据目录")?;
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((BODY_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取服务控制参数")?;
    if bytes.len() > BODY_LIMIT {
        return Err("服务控制参数超出限制".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "服务控制参数必须为 UTF-8，未转换编码")?;
    let request = serde_json::from_str(text).map_err(|_| "服务控制参数不是有效 JSON")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "无法初始化服务控制运行时")?;
    let result = runtime.block_on(control(data, request))?;
    let mut output = serde_json::to_vec(&result).unwrap();
    output.push(b'\n');
    std::io::stdout()
        .write_all(&output)
        .map_err(|_| "无法输出服务控制结果".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "supercode-managed-services-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }
    fn manager(root: &Path) -> Arc<ServiceManager> {
        let directory = service_directory(root).unwrap();
        Arc::new(ServiceManager {
            logs: directory.join("logs"),
            entries: AsyncMutex::new(Vec::new()),
        })
    }
    #[tokio::test]
    async fn private_bridge_rejects_web_origins_missing_tokens_unknown_fields_and_oversized_requests(
    ) {
        let root = root();
        let manager = manager(&root);
        let routes = router(Bridge {
            token: "test-private-token".into(),
            manager: manager.clone(),
            approvals: Arc::new(Approvals::default()),
            app: None,
        });
        for (token, origin, body, expected) in [
            (None, None, r#"{"operation":"list"}"#.to_owned(), 401),
            (
                Some("wrong"),
                None,
                r#"{"operation":"list"}"#.to_owned(),
                401,
            ),
            (
                Some("test-private-token"),
                Some("https://website.example"),
                r#"{"operation":"list"}"#.to_owned(),
                401,
            ),
            (
                Some("test-private-token"),
                None,
                r#"{"operation":"list","program":"injected"}"#.to_owned(),
                422,
            ),
            (
                Some("test-private-token"),
                None,
                format!(
                    r#"{{"operation":"list","padding":"{}"}}"#,
                    "a".repeat(BODY_LIMIT)
                ),
                413,
            ),
            (
                Some("test-private-token"),
                None,
                r#"{"operation":"list"}"#.to_owned(),
                200,
            ),
        ] {
            let mut request =
                axum::http::Request::post("/service").header("content-type", "application/json");
            if let Some(token) = token {
                request = request.header("authorization", format!("Bearer {token}"));
            }
            if let Some(origin) = origin {
                request = request.header("origin", origin);
            }
            let response = routes
                .clone()
                .oneshot(
                    request
                        .body(Body::from(format!(
                            r#"{{"sessionId":null,"request":{body}}}"#
                        )))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), expected);
        }
        assert!(manager.entries.lock().await.is_empty());
        assert!(manager
            .execute(Request::Stop {
                id: uuid::Uuid::new_v4().to_string()
            })
            .await
            .is_err());
    }
    #[tokio::test]
    async fn service_approvals_require_an_explicit_user_response_and_cancel_with_the_owning_turn() {
        let queue = Arc::new(Approvals::default());
        let request = Request::Stop { id: "owned".into() };
        let mut task = Box::pin(queue.confirm(None, "session", Some("turn"), &request));
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut task)
            .await
            .is_err());
        let event = queue.events().pop().unwrap();
        let id = event["id"].as_str().unwrap();
        assert_eq!(
            event["params"]["availableDecisions"],
            json!(["accept", "decline"])
        );
        assert!(queue
            .respond(id, &json!({"decision":"acceptForSession"}))
            .is_err());
        queue.respond(id, &json!({"decision":"decline"})).unwrap();
        assert!(task.await.is_err());
        assert!(queue.events().is_empty());
        let request = Request::Stop { id: "owned".into() };
        let mut task = Box::pin(queue.confirm(None, "session", Some("turn"), &request));
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut task)
            .await
            .is_err());
        queue.cancel_turn("other", Some("turn"));
        assert_eq!(queue.events().len(), 1);
        queue.cancel_turn("session", Some("turn"));
        assert!(task.await.is_err());
        assert!(queue.events().is_empty());
        let mut task = Box::pin(queue.confirm(None, "session", Some("turn"), &request));
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut task)
            .await
            .is_err());
        drop(task);
        assert!(queue.events().is_empty());
    }
    #[tokio::test]
    async fn a_declined_service_request_never_launches_a_process() {
        let root = root();
        let manager = manager(&root);
        let approvals = Arc::new(Approvals::default());
        let bridge = Bridge {
            token: "private".into(),
            manager: manager.clone(),
            approvals: approvals.clone(),
            app: None,
        };
        let mut operation = Box::pin(bridge.execute(Envelope {
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            request: Request::Start {
                name: "denied".into(),
                program: std::env::current_exe().unwrap(),
                cwd: root,
                args: vec!["--help".into()],
            },
        }));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut operation)
                .await
                .is_err()
        );
        assert!(manager.entries.lock().await.is_empty());
        let event = approvals.events().pop().unwrap();
        approvals
            .respond(
                event["id"].as_str().unwrap(),
                &json!({"decision":"decline"}),
            )
            .unwrap();
        assert!(operation.await.is_err());
        assert!(manager.entries.lock().await.is_empty());
    }
    #[test]
    fn output_is_bounded_and_retains_raw_non_utf8_bytes() {
        let root = root();
        let path = root.join("output.bin");
        let mut log = BoundedLog::new(path.clone()).unwrap();
        for _ in 0..100 {
            log.write(&[0xff; 8192]).unwrap();
        }
        drop(log);
        for path in [&path, &path.with_extension("previous.bin")] {
            let bytes = std::fs::read(path).unwrap();
            assert!(bytes.len() <= LOG_LIMIT);
            assert!(!bytes.is_empty());
            assert!(bytes.iter().all(|byte| *byte == 0xff));
        }
    }
    #[test]
    fn pruning_only_removes_old_owned_log_files_and_preserves_active_and_unrelated_files() {
        let root = root();
        let protected = uuid::Uuid::new_v4().to_string();
        for _ in 0..40 {
            std::fs::write(root.join(format!("{}.bin", uuid::Uuid::new_v4())), b"log").unwrap();
        }
        let active = root.join(format!("{protected}.bin"));
        std::fs::write(&active, b"active").unwrap();
        let unrelated = root.join("user-file.bin");
        std::fs::write(&unrelated, b"user").unwrap();
        prune_logs(&root, &HashSet::from([protected]));
        assert!(active.exists());
        assert!(unrelated.exists());
        assert_eq!(std::fs::read_dir(root).unwrap().count(), 34);
    }
    #[test]
    fn validates_literal_launch_arguments_without_converting_them_to_a_shell_command() {
        let cwd = root();
        let program = std::env::current_exe().unwrap();
        let args = vec![
            "空格 中文 参数".into(),
            "$(not-executed) & `literal`".into(),
        ];
        assert!(validate_launch("preview", &program, &cwd, &args).is_ok());
        assert!(validate_launch("preview", Path::new("node.exe"), &cwd, &args).is_err());
        assert!(validate_launch("preview", &program, &cwd, &["nul\0value".into()]).is_err());
        assert!(validate_launch("", &program, &cwd, &args).is_err());
        let script = cwd.join("npm.cmd");
        std::fs::write(&script, b"not executed").unwrap();
        if cfg!(windows) {
            assert!(validate_launch("preview", &script, &cwd, &args).is_err());
        }
    }
    #[tokio::test]
    async fn controller_does_not_use_untrusted_addresses_or_accept_invalid_descriptors() {
        let root = root();
        let directory = service_directory(&root).unwrap();
        for value in [
            json!({"format":FORMAT,"port":0,"token":uuid::Uuid::new_v4().to_string()}),
            json!({"format":FORMAT,"port":1234,"token":"invalid"}),
            json!({"format":FORMAT,"port":1234,"token":uuid::Uuid::new_v4().to_string(),"url":"https://not-local.example/"}),
        ] {
            std::fs::write(
                directory.join("bridge.json"),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
            assert!(control(&root, Request::List {}).await.is_err());
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn ending_agent_job_kills_temporary_children_but_not_hosted_service_and_explicit_stop_is_owned(
    ) {
        let root = root();
        let manager = manager(&root);
        let node = process::find_program("node")
            .expect("Node is part of this project's existing toolchain");
        let fixture = root.join("service fixture 中文.mjs");
        std::fs::write(&fixture, r#"import fs from 'node:fs';import http from 'node:http';import {spawn} from 'node:child_process';
const [mode,...a]=process.argv.slice(2);
if(mode==='server'){const server=http.createServer((_,r)=>r.end('owned-service-alive'));server.listen(0,'127.0.0.1',()=>fs.writeFileSync(a[0],JSON.stringify({port:server.address().port}),'utf8'));}
else {process.stdin.once('data',async()=>{spawn(process.execPath,[process.argv[1],'server',a[3]],{windowsHide:true,stdio:'ignore'});const endpoint=JSON.parse(fs.readFileSync(a[0],'utf8'));const request=JSON.parse(fs.readFileSync(a[1],'utf8'));const response=await fetch('http://127.0.0.1:'+endpoint.port+'/service',{method:'POST',headers:{authorization:'Bearer '+endpoint.token,'content-type':'application/json'},body:JSON.stringify(request)});fs.writeFileSync(a[2],JSON.stringify(await response.json()),'utf8');setInterval(()=>{},1000);});}
"#.as_bytes()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = Endpoint {
            format: FORMAT.into(),
            port: listener.local_addr().unwrap().port(),
            token: uuid::Uuid::new_v4().to_string(),
        };
        let descriptor = root.join("services/bridge.json");
        std::fs::write(&descriptor, serde_json::to_vec(&endpoint).unwrap()).unwrap();
        let approvals = Arc::new(Approvals::default());
        let session = uuid::Uuid::new_v4().to_string();
        let routes = router(Bridge {
            token: endpoint.token,
            manager: manager.clone(),
            approvals: approvals.clone(),
            app: None,
        });
        struct Server(tokio::task::JoinHandle<()>);
        impl Drop for Server {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, routes).await.unwrap();
        }));
        let durable = root.join("durable-port.json");
        let temporary = root.join("temporary-port.json");
        let ready = root.join("controller-ready.json");
        let request_file = root.join("start.json");
        let args = vec![
            fixture.to_string_lossy().into_owned(),
            "server".into(),
            durable.to_string_lossy().into_owned(),
        ];
        std::fs::write(
            &request_file,
            serde_json::to_vec(&Envelope {
                session_id: Some(session.clone()),
                request: Request::Start {
                    name: "preview".into(),
                    program: node.clone(),
                    cwd: root.clone(),
                    args: args.clone(),
                },
            })
            .unwrap(),
        )
        .unwrap();
        let mut controller = process::command(&node, &[])
            .args([
                fixture.as_os_str(),
                std::ffi::OsStr::new("controller"),
                descriptor.as_os_str(),
                request_file.as_os_str(),
                ready.as_os_str(),
                temporary.as_os_str(),
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let job = process::JobGuard::attach(&controller).unwrap();
        use tokio::io::AsyncWriteExt;
        controller
            .stdin
            .take()
            .unwrap()
            .write_all(b"begin\n")
            .await
            .unwrap();
        async fn read_json(path: &Path) -> Value {
            tokio::time::timeout(Duration::from_secs(12), async {
                loop {
                    if let Ok(bytes) = std::fs::read(path) {
                        if let Ok(value) = serde_json::from_slice(&bytes) {
                            break value;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
            })
            .await
            .unwrap()
        }
        async fn accept(queue: &Approvals) {
            let event = tokio::time::timeout(Duration::from_secs(12), async {
                loop {
                    if let Some(event) = queue.events().pop() {
                        break event;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            queue
                .respond(event["id"].as_str().unwrap(), &json!({"decision":"accept"}))
                .unwrap();
        }
        async fn authorized_control(
            root: &Path,
            session: &str,
            queue: &Approvals,
            request: Request,
        ) -> Result<Value, String> {
            let endpoint =
                serde_json::from_slice(&std::fs::read(root.join("services/bridge.json")).unwrap())
                    .unwrap();
            let (result, ()) = tokio::join!(
                control_envelope(
                    endpoint,
                    Envelope {
                        session_id: Some(session.into()),
                        request
                    }
                ),
                accept(queue)
            );
            result
        }
        accept(&approvals).await;
        let confirmation = read_json(&ready).await;
        assert_eq!(confirmation["ok"], true);
        let id = confirmation["result"]["id"].as_str().unwrap().to_owned();
        let durable_port = read_json(&durable).await["port"].as_u64().unwrap() as u16;
        let temporary_port = read_json(&temporary).await["port"].as_u64().unwrap() as u16;
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let durable_url = format!("http://127.0.0.1:{durable_port}/");
        assert_eq!(
            client
                .get(&durable_url)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "owned-service-alive"
        );
        drop(job); // Exactly the same JobGuard drop used by idle agent cleanup.
        tokio::time::timeout(Duration::from_secs(3), controller.wait())
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while client
                .get(format!("http://127.0.0.1:{temporary_port}/"))
                .send()
                .await
                .is_ok()
            {
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            client
                .get(&durable_url)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "owned-service-alive"
        );
        let reused = authorized_control(
            &root,
            &session,
            &approvals,
            Request::Start {
                name: "preview".into(),
                program: node.clone(),
                cwd: root.clone(),
                args,
            },
        )
        .await
        .unwrap();
        assert_eq!(reused["id"], id);
        assert_eq!(
            control(&root, Request::List {}).await.unwrap()["services"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(authorized_control(
            &root,
            &session,
            &approvals,
            Request::Start {
                name: "preview".into(),
                program: node,
                cwd: root.clone(),
                args: vec!["different".into()]
            }
        )
        .await
        .is_err());
        assert!(authorized_control(
            &root,
            &session,
            &approvals,
            Request::Stop {
                id: uuid::Uuid::new_v4().to_string()
            }
        )
        .await
        .is_err());
        assert!(client.get(&durable_url).send().await.is_ok());
        assert_eq!(
            authorized_control(&root, &session, &approvals, Request::Stop { id })
                .await
                .unwrap()["state"],
            "stopped"
        );
        assert!(client.get(&durable_url).send().await.is_err());
    }
}
