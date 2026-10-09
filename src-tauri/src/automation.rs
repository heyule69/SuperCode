//! Private, pinned automation dependencies. Installation never changes PATH or agent config files.
use crate::{
    client_features::{self, ToolServer},
    process, AppState,
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    sync::Mutex as AsyncMutex,
};

const KEY: &str = "automation_installations";
const MCP_VERSION: &str = "0.0.83";
const WINDOWS_VERSION: &str = "0.7.5";
const NODE_VERSION: &str = "22.23.3";
const UV_VERSION: &str = "0.12.23";
const MAX_LINE: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Browser,
    Computer,
}
impl Kind {
    fn id(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Computer => "computer",
        }
    }
    fn version(self) -> &'static str {
        match self {
            Self::Browser => MCP_VERSION,
            Self::Computer => WINDOWS_VERSION,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    kind: Kind,
    version: String,
    root: PathBuf,
    server: ToolServer,
    tools: usize,
    tested_at: u64,
    summary: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    kind: Kind,
    phase: String,
    message: String,
    received: Option<u64>,
    total: Option<u64>,
    installation: Option<Installation>,
    enabled: bool,
}
#[derive(Default)]
pub struct Automation {
    operation: AsyncMutex<()>,
    tasks: Mutex<BTreeMap<Kind, (Status, Arc<AtomicBool>)>>,
}
struct Task {
    app: AppHandle,
    kind: Kind,
    cancel: Arc<AtomicBool>,
    agent: Option<String>,
}
impl Task {
    fn progress(&self, phase: &str, message: &str, received: Option<u64>, total: Option<u64>) {
        if let Some(agent) = &self.agent {
            crate::agents::progress(
                &self.app,
                agent,
                phase,
                message,
                received,
                total,
                self.cancel.clone(),
            );
            return;
        }
        let status = Status {
            kind: self.kind,
            phase: phase.into(),
            message: message.into(),
            received,
            total,
            installation: None,
            enabled: false,
        };
        self.app
            .state::<Automation>()
            .tasks
            .lock()
            .unwrap()
            .insert(self.kind, (status.clone(), self.cancel.clone()));
        let event = if matches!(phase, "ready" | "failed" | "cancelled") {
            list_automation(self.app.clone())
                .ok()
                .and_then(|rows| rows.into_iter().find(|s| s.kind == self.kind))
                .unwrap_or(status)
        } else {
            status
        };
        let _ = self.app.emit("automation-progress", &event);
    }
    fn check(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Relaxed) {
            Err("已取消，原有配置已保留".into())
        } else {
            Ok(())
        }
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn installed(app: &AppHandle) -> Result<Vec<Installation>, String> {
    let raw = app
        .state::<AppState>()
        .store
        .setting(KEY)?
        .unwrap_or_else(|| "[]".into());
    serde_json::from_str(&raw).map_err(|_| "自动化安装记录损坏，请重新安装".into())
}
fn valid_installation(base: &Path, install: &Installation) -> bool {
    // Reject stale/edited paths. A successful test alone does not make missing executables usable.
    let base = ordinary_path(base.canonicalize().unwrap_or_else(|_| base.to_path_buf()));
    let root = ordinary_path(install.root.clone());
    install.root.is_absolute()
        && root.starts_with(base.join("automation"))
        && !install
            .root
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        && Path::new(&install.server.command).is_file()
        && match install.kind {
            Kind::Browser => {
                install
                    .root
                    .join("node_modules/@playwright/mcp/cli.js")
                    .is_file()
                    && install
                        .server
                        .args
                        .windows(2)
                        .any(|a| a[0] == "--executable-path" && Path::new(&a[1]).is_file())
            }
            Kind::Computer => install.root.join(".venv/Scripts/python.exe").is_file(),
        }
}
// Node's CLI resolver cannot accept Windows verbatim paths (EISDIR on C:).
// Canonicalize first to resolve MSIX redirection, then use normal absolute paths.
fn ordinary_path(path: PathBuf) -> PathBuf {
    if cfg!(windows) {
        let value = path.to_string_lossy();
        if let Some(rest) = value.strip_prefix("\\\\?\\UNC\\") {
            return PathBuf::from(format!("\\\\{rest}"));
        }
        if let Some(rest) = value.strip_prefix("\\\\?\\") {
            return PathBuf::from(rest);
        }
    }
    path
}
#[tauri::command]
pub fn list_automation(app: AppHandle) -> Result<Vec<Status>, String> {
    let ready = installed(&app)?;
    let servers = client_features::servers(&app)?;
    let state = app.state::<Automation>();
    let tasks = state.tasks.lock().unwrap();
    Ok([Kind::Browser, Kind::Computer]
        .into_iter()
        .map(|kind| {
            let installation = ready.iter().find(|i| i.kind == kind).cloned();
            let valid = installation
                .as_ref()
                .is_some_and(|i| valid_installation(&app.state::<AppState>().data_dir, i));
            let enabled = valid
                && installation.as_ref().is_some_and(|i| {
                    servers.iter().any(|s| {
                        s.id == kind.id()
                            && s.enabled
                            && s.command == i.server.command
                            && s.args == i.server.args
                    })
                });
            if let Some((status, _)) = tasks.get(&kind) {
                let mut status = status.clone();
                if status.phase == "ready" && !valid {
                    status.phase = "missing".into();
                    status.message = "安装文件缺失，请修复安装".into();
                }
                status.installation = installation;
                status.enabled = enabled;
                return status;
            }
            Status {
                kind,
                phase: if valid {
                    "ready"
                } else if installation.is_some() {
                    "missing"
                } else {
                    "notInstalled"
                }
                .into(),
                message: if valid {
                    "测试通过"
                } else if installation.is_some() {
                    "安装文件缺失，请修复安装"
                } else {
                    "尚未安装"
                }
                .into(),
                received: None,
                total: None,
                installation,
                enabled,
            }
        })
        .collect())
}
fn ensure_idle(app: &AppHandle) -> Result<(), String> {
    if app.state::<AppState>().store.running()? > 0 {
        return Err("请等待当前任务结束，再安装或更改自动化工具".into());
    }
    Ok(())
}
#[tauri::command]
pub fn cancel_automation(kind: Kind, app: AppHandle) {
    if let Some((_, cancel)) = app.state::<Automation>().tasks.lock().unwrap().get(&kind) {
        cancel.store(true, Ordering::Relaxed);
    }
}
#[tauri::command]
pub async fn install_automation(
    kind: Kind,
    repair: Option<bool>,
    app: AppHandle,
) -> Result<Vec<Status>, String> {
    operate(kind, repair.unwrap_or(false), false, app).await
}
#[tauri::command]
pub async fn test_automation(kind: Kind, app: AppHandle) -> Result<Vec<Status>, String> {
    operate(kind, false, true, app).await
}
async fn operate(
    kind: Kind,
    repair: bool,
    test_only: bool,
    app: AppHandle,
) -> Result<Vec<Status>, String> {
    if !cfg!(windows) {
        return Err("此安装器目前支持 Windows".into());
    }
    ensure_idle(&app)?;
    let state = app.state::<Automation>();
    let _lock = state
        .operation
        .try_lock()
        .map_err(|_| "另一项安装或测试正在进行，请稍后再试")?;
    let task = Task {
        agent: None,
        app: app.clone(),
        kind,
        cancel: Arc::new(AtomicBool::new(false)),
    };
    task.progress("preparing", "正在检查安装环境", None, None);
    let result = async {
        let previous = installed(&app)?.into_iter().find(|i| i.kind == kind);
        let pending_key = format!("automation_pending_{}", kind.id());
        let pending: Option<Installation> = app
            .state::<AppState>()
            .store
            .setting(&pending_key)?
            .and_then(|raw| serde_json::from_str::<Option<Installation>>(&raw).ok())
            .flatten();
        let mut installation = if !repair
            && previous.as_ref().is_some_and(|i| {
                valid_installation(&app.state::<AppState>().data_dir, i)
                    && i.version == kind.version()
            }) {
            previous.unwrap()
        } else if !repair
            && !test_only
            && pending.as_ref().is_some_and(|i| {
                i.kind == kind
                    && i.version == kind.version()
                    && valid_installation(&app.state::<AppState>().data_dir, i)
            })
        {
            pending.unwrap()
        } else {
            if test_only {
                return Err("尚未安装完整依赖，请先安装或修复".into());
            }
            setup(&task).await?
        };
        if installation.tested_at == 0 {
            app.state::<AppState>().store.set_setting(
                &pending_key,
                &serde_json::to_string(&installation).map_err(|e| e.to_string())?,
            )?;
        }
        task.check()?;
        task.progress("testing", "正在连接 MCP 并测试实际功能", None, None);
        let result = probe(&installation, &task.cancel).await?;
        installation.tools = result.0;
        installation.summary = result.1;
        installation.tested_at = now();
        if test_only {
            installation.server.enabled = client_features::servers(&app)?
                .iter()
                .any(|s| s.id == kind.id() && s.enabled);
        }
        task.check()?;
        ensure_idle(&app)?;
        let mut ready = installed(&app)?;
        ready.retain(|i| i.kind != kind);
        ready.push(installation.clone());
        // Both agents read this same SuperCode configuration on their next task.
        let mut servers = client_features::servers(&app)?;
        servers.retain(|s| s.id != kind.id());
        servers.push(installation.server.clone());
        client_features::validate_servers(&servers)?;
        // Commit both records in one SQLite transaction; no partially enabled installation.
        app.state::<AppState>().store.set_automation_configuration(
            &serde_json::to_string(&ready).map_err(|e| e.to_string())?,
            &serde_json::to_string(&servers).map_err(|e| e.to_string())?,
        )?;
        app.state::<AppState>()
            .runtime
            .clear_for_agent_switch()
            .await;
        Ok::<(), String>(()).inspect(|_| {
            let _ = app
                .state::<AppState>()
                .store
                .set_setting(&pending_key, "null");
        })
    }
    .await;
    match result {
        Ok(()) => task.progress("ready", "测试通过", None, None),
        Err(ref error) => task.progress(
            if task.cancel.load(Ordering::Relaxed) {
                "cancelled"
            } else {
                "failed"
            },
            error,
            None,
            None,
        ),
    }
    result?;
    list_automation(app.clone())
}
#[tauri::command]
pub async fn set_automation_enabled(
    kind: Kind,
    enabled: bool,
    app: AppHandle,
) -> Result<Vec<Status>, String> {
    ensure_idle(&app)?;
    let state = app.state::<Automation>();
    let _lock = state
        .operation
        .try_lock()
        .map_err(|_| "正在安装或测试，请稍后更改")?;
    let installation = installed(&app)?
        .into_iter()
        .find(|i| i.kind == kind)
        .ok_or("请先安装并测试")?;
    if enabled && !valid_installation(&app.state::<AppState>().data_dir, &installation) {
        return Err("安装文件缺失，请先修复".into());
    }
    let mut servers = client_features::servers(&app)?;
    servers.retain(|s| s.id != kind.id());
    let mut server = installation.server;
    server.enabled = enabled;
    servers.push(server);
    client_features::save_tool_servers(servers, app.clone()).await?;
    state.tasks.lock().unwrap().remove(&kind);
    list_automation(app.clone())
}

fn artifact(tool: &str) -> Result<(String, &'static str), String> {
    let arm = cfg!(target_arch = "aarch64");
    if !arm && !cfg!(target_arch = "x86_64") {
        return Err("暂不支持此 CPU 架构".into());
    }
    Ok(match (tool,arm) {
        ("node",false)=>(format!("https://nodejs.org/dist/v{NODE_VERSION}/node-v{NODE_VERSION}-win-x64.zip"),"2b0ff57b049cda1bbcea2240eec20467018713c1efe1f7360c2681859b90ed71"),
        ("node",true)=>(format!("https://nodejs.org/dist/v{NODE_VERSION}/node-v{NODE_VERSION}-win-arm64.zip"),"33dad22e4cef5ee8f9fbb1b0d037fdacd0e56d12a4580f0d63f68b894deab535"),
        ("uv",false)=>(format!("https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/uv-x86_64-pc-windows-msvc.zip"),"75d05de6762778c31ee183398de7dd15093fad0ed90b1f236d8205ea5ec00c90"),
        ("uv",true)=>(format!("https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/uv-aarch64-pc-windows-msvc.zip"),"13294e232ececbe709c06b74e6ced06f2a225ea5591476685362f22be56a50d5"),
        _=>return Err("未知运行环境".into()),
    })
}
async fn download(url: &str, hash: &str, target: &Path, task: &Task) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;
    let request = tokio::select! {result=client.get(url).send()=>result,_=cancelled(&task.cancel)=>return Err("已取消下载".into())};
    let response = request
        .map_err(|e| format!("下载失败，请检查网络后重试：{e}"))?
        .error_for_status()
        .map_err(|e| format!("下载失败：{e}"))?;
    let total = response.content_length();
    if total.is_some_and(|v| v > 150 * 1024 * 1024) {
        return Err("下载文件超过大小限制".into());
    }
    let mut file = tokio::fs::File::create(target)
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = response.bytes_stream();
    let mut received = 0u64;
    let mut hasher = Sha256::new();
    let mut last = std::time::Instant::now();
    loop {
        let chunk = tokio::select! { chunk=stream.next()=>chunk, _=cancelled(&task.cancel)=>return Err("已取消下载".into()) };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(|e| format!("下载中断：{e}"))?;
        received += chunk.len() as u64;
        if received > 150 * 1024 * 1024 {
            return Err("下载文件超过大小限制".into());
        }
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        hasher.update(&chunk);
        if last.elapsed() > Duration::from_millis(250) {
            task.progress("downloading", "正在下载运行环境", Some(received), total);
            last = std::time::Instant::now();
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    if format!("{:x}", hasher.finalize()) != hash {
        return Err("下载校验失败，未执行该文件。请重试".into());
    }
    Ok(())
}
fn extract_zip(archive: &Path, target: &Path) -> Result<(), String> {
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(archive).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if archive.len() > 20000 {
        return Err("压缩包文件过多".into());
    }
    let mut size = 0u64;
    for n in 0..archive.len() {
        let mut entry = archive.by_index(n).map_err(|e| e.to_string())?;
        let name = entry.enclosed_name().ok_or("压缩包路径无效")?;
        if entry.name().contains('\\')
            || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err("不支持的压缩包路径或链接".into());
        }
        size = size.checked_add(entry.size()).ok_or("压缩包过大")?;
        if size > 700 * 1024 * 1024 {
            return Err("压缩包过大".into());
        }
        let path = target.join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        } else {
            std::fs::create_dir_all(path.parent().ok_or("压缩包路径无效")?)
                .map_err(|e| e.to_string())?;
            let mut out = std::fs::File::create(path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
struct StagingDirectory {
    base: PathBuf,
    root: PathBuf,
    retain: bool,
}
impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if self.retain {
            return;
        }
        let root = self.root.clone();
        let base = self.base.clone();
        // Only remove this newly created staging directory, after checking its resolved boundary.
        tauri::async_runtime::spawn_blocking(move || {
            if let (Ok(base), Ok(target)) =
                (base.join("automation").canonicalize(), root.canonicalize())
            {
                if target.starts_with(&base)
                    && target != base
                    && target
                        .file_name()
                        .is_some_and(|s| uuid::Uuid::parse_str(&s.to_string_lossy()).is_ok())
                {
                    let _ = std::fs::remove_dir_all(target);
                }
            }
        });
    }
}
async fn portable(tool: &str, root: &Path, task: &Task) -> Result<PathBuf, String> {
    task.progress(
        "downloading",
        if tool == "node" {
            "正在下载 Node.js"
        } else {
            "正在下载 Python 安装器"
        },
        None,
        None,
    );
    let (url, hash) = artifact(tool)?;
    let archive = root.join(format!("{tool}.zip"));
    download(&url, hash, &archive, task).await?;
    task.check()?;
    task.progress("installing", "正在解压已校验的运行环境", None, None);
    let dest = root.join(tool);
    let archive2 = archive.clone();
    let dest2 = dest.clone();
    tokio::task::spawn_blocking(move || extract_zip(&archive2, &dest2))
        .await
        .map_err(|e| e.to_string())??;
    let executable = if tool == "node" {
        dest.join(format!(
            "node-v{NODE_VERSION}-win-{}",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x64"
            }
        ))
        .join("node.exe")
    } else {
        dest.join("uv.exe")
    };
    if !executable.is_file() {
        return Err("压缩包缺少运行程序".into());
    }
    let _ = tokio::fs::remove_file(archive).await;
    Ok(executable)
}
async fn cancelled(flag: &Arc<AtomicBool>) {
    loop {
        if flag.load(Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
async fn tail<R: tokio::io::AsyncRead + Unpin>(mut reader: R) -> Result<String, String> {
    let mut retained = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = reader.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        retained.extend_from_slice(&buf[..n]);
        if retained.len() > 8192 {
            retained.drain(..retained.len() - 8192);
        }
    }
    Ok(String::from_utf8_lossy(&retained)
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect())
}
async fn execute(
    program: &Path,
    args: &[String],
    root: &Path,
    env: &[(&str, PathBuf)],
    task: &Task,
) -> Result<String, String> {
    task.check()?;
    let literal = args.iter().map(String::as_str).collect::<Vec<_>>();
    let mut command = process::command(program, &literal);
    command
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动安装器：{e}"))?;
    let _job = process::JobGuard::attach(&child)?;
    let stdout = tokio::spawn(tail(child.stdout.take().ok_or("安装器无输出管道")?));
    let stderr = tokio::spawn(tail(child.stderr.take().ok_or("安装器无错误管道")?));
    let exit = tokio::select! { result=child.wait()=>result.map_err(|e| e.to_string())?, _=cancelled(&task.cancel)=>return Err("已取消安装".into()), _=tokio::time::sleep(Duration::from_secs(900))=>return Err("安装超时，请检查网络后重试".into()) };
    let out = stdout.await.map_err(|e| e.to_string())??;
    let err = stderr.await.map_err(|e| e.to_string())??;
    if !exit.success() {
        return Err(format!(
            "安装失败：{}",
            err.lines()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    Ok(out)
}
async fn setup(task: &Task) -> Result<Installation, String> {
    let base = task.app.state::<AppState>().data_dir.clone();
    let root = base
        .join("automation")
        .join(task.kind.id())
        .join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;
    let root = ordinary_path(root.canonicalize().map_err(|e| e.to_string())?);
    let mut staging = StagingDirectory {
        base: base.clone(),
        root: root.clone(),
        retain: false,
    };
    let (command, args) = if task.kind == Kind::Browser {
        let node = portable("node", &root, task).await?;
        let npm = node
            .parent()
            .ok_or("Node.js 目录无效")?
            .join("node_modules/npm/bin/npm-cli.js");
        let empty = root.join("user.npmrc");
        let global = root.join("global.npmrc");
        tokio::fs::write(&global, b"")
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::write(&empty, b"")
            .await
            .map_err(|e| e.to_string())?;
        let env = [
            ("NPM_CONFIG_USERCONFIG", empty.clone()),
            ("NPM_CONFIG_GLOBALCONFIG", global),
            ("NPM_CONFIG_CACHE", base.join("automation/cache/npm")),
            ("PLAYWRIGHT_BROWSERS_PATH", root.join("browsers")),
        ];
        task.progress("installing", "正在安装 Playwright MCP", None, None);
        execute(
            &node,
            &[
                npm.to_string_lossy().into_owned(),
                "install".into(),
                "--prefix".into(),
                root.to_string_lossy().into_owned(),
                "--registry=https://registry.npmjs.org".into(),
                "--ignore-scripts".into(),
                "--no-audit".into(),
                "--no-fund".into(),
                format!("@playwright/mcp@{MCP_VERSION}"),
            ],
            &root,
            &env,
            task,
        )
        .await?;
        task.progress("installing", "正在下载独立 Chromium 浏览器", None, None);
        execute(
            &node,
            &[
                root.join("node_modules/playwright/cli.js")
                    .to_string_lossy()
                    .into_owned(),
                "install".into(),
                "chromium".into(),
                "--no-shell".into(),
            ],
            &root,
            &env,
            task,
        )
        .await?;
        let chromium = execute(
            &node,
            &[
                "-e".into(),
                "process.stdout.write(require('playwright').chromium.executablePath())".into(),
            ],
            &root,
            &env,
            task,
        )
        .await?;
        if !Path::new(chromium.trim()).is_file() {
            return Err("Chromium 下载不完整".into());
        }
        (
            node,
            vec![
                root.join("node_modules/@playwright/mcp/cli.js")
                    .to_string_lossy()
                    .into_owned(),
                "--browser".into(),
                "chromium".into(),
                "--isolated".into(),
                "--executable-path".into(),
                chromium.trim().into(),
                "--output-dir".into(),
                root.join("output").to_string_lossy().into_owned(),
            ],
        )
    } else {
        let uv = portable("uv", &root, task).await?;
        let env = [
            ("UV_PYTHON_INSTALL_DIR", root.join("python")),
            ("UV_CACHE_DIR", base.join("automation/cache/uv")),
        ];
        task.progress("installing", "正在下载 Python 并创建独立环境", None, None);
        execute(
            &uv,
            &[
                "--no-config".into(),
                "venv".into(),
                "--python".into(),
                "3.13".into(),
                "--managed-python".into(),
                root.join(".venv").to_string_lossy().into_owned(),
            ],
            &root,
            &env,
            task,
        )
        .await?;
        task.progress("installing", "正在安装 Windows MCP", None, None);
        execute(
            &uv,
            &[
                "--no-config".into(),
                "pip".into(),
                "install".into(),
                "--python".into(),
                root.join(".venv/Scripts/python.exe")
                    .to_string_lossy()
                    .into_owned(),
                "--index-url".into(),
                "https://pypi.org/simple".into(),
                format!("windows-mcp=={WINDOWS_VERSION}"),
            ],
            &root,
            &env,
            task,
        )
        .await?;
        (
            root.join(".venv/Scripts/windows-mcp.exe"),
            vec!["serve".into()],
        )
    };
    let server = ToolServer {
        id: task.kind.id().into(),
        name: if task.kind == Kind::Browser {
            "浏览器 · Playwright"
        } else {
            "电脑 · Windows MCP"
        }
        .into(),
        kind: task.kind.id().into(),
        command: command.to_string_lossy().into_owned(),
        args,
        url: None,
        enabled: true,
    };
    staging.retain = true;
    Ok(Installation {
        kind: task.kind,
        version: task.kind.version().into(),
        root,
        server,
        tools: 0,
        tested_at: 0,
        summary: String::new(),
    })
}

/// Shared verified portable Node/npm installer. Only callers' fixed package IDs are accepted.
pub(crate) async fn install_agent_package(
    app: &AppHandle,
    agent: &str,
    root: &Path,
    package: &str,
    cancel: Arc<AtomicBool>,
) -> Result<PathBuf, String> {
    let task = Task {
        app: app.clone(),
        kind: Kind::Browser,
        cancel,
        agent: Some(agent.into()),
    };
    let node = portable("node", root, &task).await?;
    let npm = node
        .parent()
        .ok_or("Node.js 目录无效")?
        .join("node_modules/npm/bin/npm-cli.js");
    let user = root.join("user.npmrc");
    let global = root.join("global.npmrc");
    tokio::fs::write(&user, b"")
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(&global, b"")
        .await
        .map_err(|e| e.to_string())?;
    task.progress("installing", "正在安装并校验官方 Agent 包", None, None);
    execute(
        &node,
        &[
            npm.to_string_lossy().into_owned(),
            "install".into(),
            "--prefix".into(),
            root.to_string_lossy().into_owned(),
            "--registry=https://registry.npmjs.org".into(),
            "--ignore-scripts".into(),
            "--no-audit".into(),
            "--no-fund".into(),
            package.into(),
        ],
        root,
        &[
            ("NPM_CONFIG_USERCONFIG", user),
            ("NPM_CONFIG_GLOBALCONFIG", global),
            (
                "NPM_CONFIG_CACHE",
                app.state::<AppState>().data_dir.join("agents/cache/npm"),
            ),
        ],
        &task,
    )
    .await?;
    task.check()?;
    Ok(node)
}

struct Mcp {
    child: Child,
    _job: process::JobGuard,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    errors: tokio::task::JoinHandle<Result<String, String>>,
    id: u64,
}
impl Mcp {
    fn start(server: &ToolServer, root: &Path, headless: bool) -> Result<Self, String> {
        let mut args = server.args.clone();
        if headless {
            args.push("--headless".into());
        }
        let mut command = process::command(
            Path::new(&server.command),
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        command
            .current_dir(root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if server.kind == "codexProbe" {
            command.env("CODEX_HOME", root);
        }
        let mut child = command.spawn().map_err(|e| format!("MCP 无法启动：{e}"))?;
        let job = process::JobGuard::attach(&child)?;
        let input = child.stdin.take().ok_or("MCP 缺少输入管道")?;
        let output = BufReader::new(child.stdout.take().ok_or("MCP 缺少输出管道")?);
        let errors = tokio::spawn(tail(child.stderr.take().ok_or("MCP 缺少错误管道")?));
        Ok(Self {
            child,
            _job: job,
            input,
            output,
            errors,
            id: 0,
        })
    }
    async fn write(&mut self, value: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|e| format!("MCP 连接中断：{e}"))
    }
    async fn request(
        &mut self,
        method: &str,
        params: Value,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Value, String> {
        self.id += 1;
        let id = self.id;
        self.write(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        let work = async {
            loop {
                let mut line = Vec::new();
                let n = (&mut self.output)
                    .take((MAX_LINE + 1) as u64)
                    .read_until(b'\n', &mut line)
                    .await
                    .map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err("MCP 提前退出，请尝试修复安装".into());
                }
                if n > MAX_LINE {
                    return Err("MCP 返回内容超过限制".into());
                }
                let value: Value =
                    serde_json::from_slice(&line).map_err(|_| "MCP 返回了无效协议数据")?;
                if value.get("id") == Some(&json!(id)) && value.get("method").is_none() {
                    if let Some(error) = value.get("error") {
                        return Err(format!("MCP 请求失败：{error}"));
                    }
                    return value
                        .get("result")
                        .cloned()
                        .ok_or_else(|| "MCP 未返回结果".into());
                }
                if let (Some(id), Some(method)) = (value.get("id"), value["method"].as_str()) {
                    // No automatic approvals or interactive prompts. This probe supports roots only.
                    self.write(if method=="roots/list" {json!({"jsonrpc":"2.0","id":id,"result":{"roots":[]}})} else {json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Not supported by installation test"}})}).await?;
                }
            }
        };
        tokio::select! {r=work=>r,_=cancelled(cancel)=>Err("已取消测试".into()),_=tokio::time::sleep(Duration::from_secs(90))=>Err(format!("MCP {method} 测试超时，请重试"))}
    }
    async fn tool(
        &mut self,
        name: &str,
        args: Value,
        cancel: &Arc<AtomicBool>,
    ) -> Result<String, String> {
        let result = self
            .request("tools/call", json!({"name":name,"arguments":args}), cancel)
            .await?;
        let text = result["content"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if result["isError"] == true {
            return Err(format!(
                "工具 {name} 测试失败：{}",
                text.chars().take(1200).collect::<String>()
            ));
        }
        if text.trim().is_empty() {
            return Err(format!("工具 {name} 没有返回有效结果"));
        }
        Ok(text)
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        self.errors.abort();
    }
}
struct ProbePage(tokio::task::JoinHandle<()>);
impl Drop for ProbePage {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn claude_control(mcp: &mut Mcp, id: &str, request: Value) -> Result<Value, String> {
    mcp.write(json!({"type":"control_request","request_id":id,"request":request}))
        .await?;
    loop {
        let mut line = Vec::new();
        let n = (&mut mcp.output)
            .take((MAX_LINE + 1) as u64)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|e| e.to_string())?;
        if n == 0 || n > MAX_LINE {
            return Err("Claude 控制连接退出或内容超过限制".into());
        }
        let value: Value = serde_json::from_slice(&line).map_err(|_| "Claude 控制协议无效")?;
        if value["type"] == "control_response" && value["response"]["request_id"] == id {
            if value["response"]["subtype"] == "error" {
                return Err(value["response"]["error"]
                    .as_str()
                    .unwrap_or("Claude 初始化失败")
                    .into());
            }
            return Ok(value["response"]["response"].clone());
        }
        if value["type"] == "control_request" {
            return Err("Agent 验证遇到交互请求，未自动批准".into());
        }
    }
}
async fn verify_agent(app: &AppHandle, agent: &str) -> Result<Value, String> {
    let root = app
        .state::<AppState>()
        .data_dir
        .join("automation/agent-probe")
        .join(agent);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let cancel = Arc::new(AtomicBool::new(false));
    let launch = crate::agents::resolve(app, agent)?;
    let (executable, args) = if agent == "codex" {
        let mut args = launch.prefix.clone();
        args.extend(["app-server".into(), "--listen".into(), "stdio://".into()]);
        for value in client_features::codex_mcp_overrides(app)?
            .into_iter()
            .filter(|v| {
                v.starts_with("mcp_servers.supercode_browser.")
                    || v.starts_with("mcp_servers.supercode_computer.")
            })
        {
            args.extend(["-c".into(), value]);
        }
        (launch.program, args)
    } else {
        let mut config = client_features::claude_mcp(app, json!({}))?;
        config["mcpServers"]
            .as_object_mut()
            .ok_or("没有 MCP 配置")?
            .retain(|k, _| matches!(k.as_str(), "supercode_browser" | "supercode_computer"));
        (
            launch.program,
            launch
                .prefix
                .into_iter()
                .chain([
                    "--print".into(),
                    "--input-format".into(),
                    "stream-json".into(),
                    "--output-format".into(),
                    "stream-json".into(),
                    "--verbose".into(),
                    "--permission-prompt-tool".into(),
                    "stdio".into(),
                    "--setting-sources".into(),
                    "".into(),
                    "--settings".into(),
                    "{\"disableAllHooks\":true}".into(),
                    "--strict-mcp-config".into(),
                    "--mcp-config".into(),
                    config.to_string(),
                ])
                .collect(),
        )
    };
    let server = ToolServer {
        id: "agent-probe".into(),
        name: "Agent integration verification".into(),
        kind: if agent == "codex" {
            "codexProbe"
        } else {
            "claudeProbe"
        }
        .into(),
        command: executable.to_string_lossy().into_owned(),
        args,
        url: None,
        enabled: false,
    };
    let mut mcp = Mcp::start(&server, &root, false)?;
    let work = async {
        if agent == "codex" {
            mcp.request("initialize",json!({"clientInfo":{"name":"supercode_automation_probe","version":"0.1.0"},"capabilities":{"experimentalApi":true}}),&cancel).await?;
            mcp.write(json!({"method":"initialized","params":{}}))
                .await?;
        } else {
            claude_control(
                &mut mcp,
                "init",
                json!({"subtype":"initialize","hooks":{},"skills":[]}),
            )
            .await?;
        }
        for step in 0..30 {
            let value = if agent == "codex" {
                mcp.request("mcpServerStatus/list", json!({"limit":100}), &cancel)
                    .await?
            } else {
                claude_control(
                    &mut mcp,
                    &format!("status{step}"),
                    json!({"subtype":"mcp_status"}),
                )
                .await?
            };
            let rows = if agent == "codex" {
                value["data"].as_array()
            } else {
                value["mcpServers"].as_array()
            }
            .ok_or_else(|| {
                format!(
                    "Agent 未返回 MCP 状态：{}",
                    value.to_string().chars().take(500).collect::<String>()
                )
            })?;
            let mut connected = Vec::new();
            for name in ["supercode_browser", "supercode_computer"] {
                if let Some(row) = rows.iter().find(|r| r["name"] == name) {
                    let tools = row["tools"]
                        .as_object()
                        .map(|v| v.len())
                        .or_else(|| row["tools"].as_array().map(|v| v.len()))
                        .unwrap_or(0);
                    if tools > 0
                        && row["error"].is_null()
                        && (agent == "codex" || row["status"] == "connected")
                    {
                        connected.push(json!({"name":name,"tools":tools}));
                    }
                }
            }
            if connected.len() == 2 {
                return Ok::<Value, String>(
                    json!({"ok":true,"agent":agent,"servers":connected,"modelTurn":false}),
                );
            }
            if step == 29 {
                return Err(format!(
                    "Agent 尚未连接两项自动化工具：{}",
                    serde_json::to_string(rows)
                        .unwrap_or_default()
                        .chars()
                        .take(1200)
                        .collect::<String>()
                ));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Err("Agent 连接验证超时".into())
    };
    tokio::time::timeout(Duration::from_secs(90), work)
        .await
        .map_err(|_| "Agent 连接验证超时".to_string())?
}
fn button_ref(snapshot: &str) -> Result<String, String> {
    let line = snapshot
        .lines()
        .find(|l| l.contains("SC Test Button"))
        .ok_or("测试页面未出现按钮")?;
    let start = line.find("[ref=").ok_or("测试按钮没有可访问引用")? + 5;
    let end = line[start..].find(']').ok_or("测试按钮引用格式无效")? + start;
    Ok(line[start..end].into())
}
fn click_arguments(tool: &Value, reference: &str) -> Result<Value, String> {
    let properties = tool["inputSchema"]["properties"]
        .as_object()
        .ok_or("浏览器点击工具没有参数定义")?;
    if properties.contains_key("target") {
        Ok(json!({"target":reference}))
    } else if properties.contains_key("ref") {
        Ok(json!({"element":"SC Test Button","ref":reference}))
    } else {
        Err("浏览器点击工具参数不兼容".into())
    }
}
async fn probe(
    installation: &Installation,
    cancel: &Arc<AtomicBool>,
) -> Result<(usize, String), String> {
    let mut mcp = Mcp::start(
        &installation.server,
        &installation.root,
        installation.kind == Kind::Browser,
    )?;
    let init=mcp.request("initialize",json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"SuperCode installation test","version":"0.1.0"}}),cancel).await?;
    if init["capabilities"]["tools"].is_null() {
        return Err("MCP 未声明工具能力".into());
    }
    mcp.write(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await?;
    let mut tools = Vec::new();
    let mut cursor = None;
    for _ in 0..20 {
        let value = mcp
            .request(
                "tools/list",
                cursor
                    .as_ref()
                    .map(|c| json!({"cursor":c}))
                    .unwrap_or(json!({})),
                cancel,
            )
            .await?;
        tools.extend(
            value["tools"]
                .as_array()
                .ok_or("MCP 未返回工具列表")?
                .iter()
                .cloned(),
        );
        cursor = value["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    if cursor.is_some() || tools.len() > 512 {
        return Err("MCP 工具列表超过限制".into());
    }
    let names = tools
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect::<Vec<_>>();
    let summary = if installation.kind == Kind::Browser {
        if !["browser_navigate", "browser_click", "browser_snapshot"]
            .iter()
            .all(|n| names.contains(n))
        {
            return Err("浏览器 MCP 缺少所需工具".into());
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string())?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let router=axum::Router::new().route("/",axum::routing::get(||async {axum::response::Html("<!doctype html><meta charset=utf-8><title>SuperCode Test</title><button onclick=\"document.querySelector('p').textContent='SC_AUTOMATION_OK'\">SC Test Button</button><p>Waiting for test</p>")}));
        let _page = ProbePage(tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        }));
        let snapshot = mcp
            .tool(
                "browser_navigate",
                json!({"url":format!("http://{address}/")}),
                cancel,
            )
            .await?;
        let snapshot = if button_ref(&snapshot).is_ok() {
            snapshot
        } else {
            mcp.tool("browser_snapshot", json!({}), cancel).await?
        };
        let reference = button_ref(&snapshot)
            .map_err(|e| format!("{e}：{}", snapshot.chars().take(1200).collect::<String>()))?;
        mcp.tool(
            "browser_click",
            click_arguments(
                tools
                    .iter()
                    .find(|t| t["name"] == "browser_click")
                    .ok_or("没有浏览器点击工具")?,
                &reference,
            )?,
            cancel,
        )
        .await?;
        let snapshot = mcp.tool("browser_snapshot", json!({}), cancel).await?;
        if !snapshot.contains("SC_AUTOMATION_OK") {
            return Err("浏览器未完成点击验证".into());
        }
        "已验证打开网页、点击按钮和读取页面".into()
    } else {
        let snapshot = tools
            .iter()
            .find(|t| {
                t["name"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case("Snapshot"))
            })
            .ok_or("电脑 MCP 缺少 Snapshot 工具")?;
        for tool in ["Click", "Type", "Scroll"] {
            if !names.iter().any(|n| n.eq_ignore_ascii_case(tool)) {
                return Err(format!("电脑 MCP 缺少 {tool} 工具"));
            }
        }
        let props = snapshot["inputSchema"]["properties"].as_object();
        let mut args = serde_json::Map::new();
        for key in ["use_vision", "use_dom"] {
            if props.is_some_and(|p| p.contains_key(key)) {
                args.insert(key.into(), json!(false));
            }
        }
        let result = mcp
            .tool(
                snapshot["name"].as_str().unwrap(),
                Value::Object(args),
                cancel,
            )
            .await?;
        if !result.contains("Focused Window:")
            || !result.contains("Opened Windows:")
            || result.contains("Error capturing desktop state:")
        {
            return Err("电脑 MCP 未能读取桌面状态，请重新测试".into());
        }
        "已验证读取桌面状态；点击、输入、滚动工具可用".into()
    };
    Ok((tools.len(), summary))
}

/// Explicit developer integration check, no model call or billing. Uses the same commands as the UI.
pub async fn smoke(app: AppHandle) {
    let mut results = Vec::new();
    for kind in [Kind::Browser, Kind::Computer] {
        let result = install_automation(kind, Some(false), app.clone()).await;
        results.push(match result {
            Ok(status) => {
                json!({"kind":kind,"ok":true,"status":status.into_iter().find(|s|s.kind==kind)})
            }
            Err(error) => json!({"kind":kind,"ok":false,"error":error}),
        });
    }
    let mut agents = Vec::new();
    if results.iter().all(|v| v["ok"] == true) {
        for agent in ["codex", "claude"] {
            agents.push(match verify_agent(&app, agent).await {
                Ok(value) => value,
                Err(error) => json!({"ok":false,"agent":agent,"error":error,"modelTurn":false}),
            });
        }
    }
    let ok = results.iter().all(|v| v["ok"] == true)
        && agents.len() == 2
        && agents.iter().all(|v| v["ok"] == true);
    let report = json!({"ok":ok,"realMcp":true,"modelTurn":false,"results":results,"agents":agents,"codexOverrides":client_features::codex_mcp_overrides(&app).unwrap_or_default(),"claudeMcp":client_features::claude_mcp(&app,json!({})).unwrap_or(json!({}))});
    let root = std::env::current_dir()
        .unwrap_or_default()
        .join(".supercode");
    let _ = std::fs::create_dir_all(&root);
    let _ = std::fs::write(
        root.join("automation-smoke-report.json"),
        serde_json::to_vec_pretty(&report).unwrap_or_default(),
    );
    app.exit(if ok { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_click_uses_actual_accessibility_reference() {
        assert_eq!(
            button_ref("- button \"SC Test Button\" [ref=e17]").unwrap(),
            "e17"
        );
        assert!(button_ref("- button missing").is_err());
        assert!(button_ref("SC Test Button [ref=oops").is_err());
        assert_eq!(
            click_arguments(
                &json!({"inputSchema":{"properties":{"target":{"type":"string"}}}}),
                "e17"
            )
            .unwrap(),
            json!({"target":"e17"})
        );
        assert_eq!(
            click_arguments(
                &json!({"inputSchema":{"properties":{"ref":{"type":"string"}}}}),
                "e17"
            )
            .unwrap()["ref"],
            "e17"
        );
        assert!(click_arguments(&json!({}), "e17").is_err());
    }
    #[test]
    fn only_known_kinds_deserialize_and_artifacts_are_pinned() {
        assert!(serde_json::from_str::<Kind>("\"custom command\"").is_err());
        assert!(artifact("untrusted").is_err());
        for tool in ["node", "uv"] {
            let (url, hash) = artifact(tool).unwrap();
            assert!(url.starts_with("https://"));
            assert_eq!(hash.len(), 64);
        }
    }
    #[test]
    fn normal_windows_paths_keep_unicode_and_resolve_verbatim_prefix() {
        if cfg!(windows) {
            assert_eq!(
                ordinary_path(PathBuf::from(r"\\?\C:\用户\app\node.exe")),
                PathBuf::from(r"C:\用户\app\node.exe")
            );
            assert_eq!(
                ordinary_path(PathBuf::from(r"\\?\UNC\host\share\node.exe")),
                PathBuf::from(r"\\host\share\node.exe")
            );
        }
    }
    #[test]
    fn zip_rejects_paths_outside_installation() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("sc-zip-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let zip = root.join("bad.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
        writer
            .start_file("../escaped.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"test").unwrap();
        writer.finish().unwrap();
        assert!(extract_zip(&zip, &root.join("install")).is_err());
        assert!(!root.join("escaped.txt").exists());
        std::fs::remove_file(zip).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[tokio::test]
    async fn output_capture_is_bounded_and_cancellation_wakes() {
        let text = tail(&vec![b'x'; 100000][..]).await.unwrap();
        assert_eq!(text.len(), 8192);
        let flag = Arc::new(AtomicBool::new(true));
        tokio::time::timeout(Duration::from_millis(20), cancelled(&flag))
            .await
            .unwrap();
    }
}
