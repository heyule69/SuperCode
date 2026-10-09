//! Bounded local discovery and private, versioned installs. Never overwrite users' CLI configs.
use crate::{process, AppState};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{Read, Seek},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{io::AsyncReadExt, sync::Mutex as AsyncMutex};

pub const IDS: [&str; 4] = ["codex", "claude", "opencode", "pi"];
pub fn name(id: &str) -> &str {
    match id {
        "codex" => "Codex",
        "claude" => "Claude Code",
        "opencode" => "OpenCode",
        "pi" => "Pi",
        _ => id,
    }
}
fn validate(id: &str) -> Result<(), String> {
    if IDS.contains(&id) {
        Ok(())
    } else {
        Err("Agent 无效".into())
    }
}
pub(crate) fn ordinary(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(s) = s.strip_prefix("\\\\?\\UNC\\") {
        PathBuf::from(format!("\\\\{s}"))
    } else if let Some(s) = s.strip_prefix("\\\\?\\") {
        PathBuf::from(s)
    } else {
        path
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Launch {
    pub program: PathBuf,
    #[serde(default)]
    pub prefix: Vec<String>,
    pub source: String,
}
impl Launch {
    pub fn command(&self, args: &[&str]) -> tokio::process::Command {
        let mut c = process::command(&self.program, &[]);
        c.args(&self.prefix).args(args);
        c
    }
    fn valid(&self) -> bool {
        native_binary(&self.program) && self.prefix.first().is_none_or(|p| Path::new(p).is_file())
    }
}
// npm may ship a text placeholder named *.exe before its postinstall step.
// Use the platform binary from its optional package directly without running scripts.
fn native_binary(path: &Path) -> bool {
    if !path.is_file() || !path.is_absolute() {
        return false;
    }
    if !cfg!(windows) {
        return true;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0; 2];
    if file.read_exact(&mut magic).is_err() || magic != *b"MZ" {
        return false;
    }
    let mut offset = [0; 4];
    if file.seek(std::io::SeekFrom::Start(0x3c)).is_err() || file.read_exact(&mut offset).is_err() {
        return false;
    }
    let offset = u32::from_le_bytes(offset);
    if offset > 16 * 1024 * 1024 {
        return false;
    }
    let mut header = [0; 6];
    if file.seek(std::io::SeekFrom::Start(offset as u64)).is_err()
        || file.read_exact(&mut header).is_err()
        || header[..4] != *b"PE\0\0"
    {
        return false;
    }
    let machine = u16::from_le_bytes([header[4], header[5]]);
    if cfg!(target_arch = "aarch64") {
        matches!(machine, 0xaa64 | 0x8664 | 0x14c)
    } else {
        matches!(machine, 0x8664 | 0x14c)
    }
}
fn native_paths(id: &str) -> Vec<String> {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let triple = if arch == "arm64" {
        "aarch64-pc-windows-msvc"
    } else {
        "x86_64-pc-windows-msvc"
    };
    match id {
        "codex" => [format!("@openai/codex-win32-{arch}"), format!("@openai/codex/node_modules/@openai/codex-win32-{arch}"), "@openai/codex".into()].into_iter().flat_map(|p| [format!("node_modules/{p}/vendor/{triple}/bin/codex.exe"), format!("node_modules/{p}/vendor/{triple}/codex/codex.exe")]).collect(),
        "claude" => vec!["node_modules/@anthropic-ai/claude-code/bin/claude.exe".into(), format!("node_modules/@anthropic-ai/claude-code-win32-{arch}/claude.exe"), format!("node_modules/@anthropic-ai/claude-code/node_modules/@anthropic-ai/claude-code-win32-{arch}/claude.exe")],
        "opencode" => vec!["node_modules/opencode-ai/bin/opencode.exe".into(), format!("node_modules/opencode-windows-{arch}/bin/opencode.exe"), format!("node_modules/opencode-ai/node_modules/opencode-windows-{arch}/bin/opencode.exe")],
        _ => vec![],
    }
}
fn at_root(id: &str, root: &Path, node: Option<&Path>, source: &str) -> Option<Launch> {
    if id == "pi" {
        for relative in [
            "node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js",
            "node_modules/@mariozechner/pi-coding-agent/dist/cli.js",
        ] {
            let script = root.join(relative);
            if script.is_file() {
                if let Some(node) = node {
                    return Some(Launch {
                        program: node.into(),
                        prefix: vec![ordinary(script).to_string_lossy().into_owned()],
                        source: source.into(),
                    });
                }
            }
        }
    }
    let candidates = native_paths(id)
        .into_iter()
        .map(|p| root.join(p))
        .chain([root.join(format!("{id}.exe")), root.join(id)]);
    candidates
        .filter(|p| native_binary(p))
        .next()
        .map(|p| Launch {
            program: ordinary(p),
            prefix: vec![],
            source: source.into(),
        })
}
pub(crate) fn local(id: &str) -> Option<Launch> {
    let node = node_program();
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|v| {
            std::env::split_paths(&v)
                .filter(|p| p.is_absolute())
                .take(150)
                .collect()
        })
        .unwrap_or_default();
    if let Some(v) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(v).join("npm"));
    }
    if let Some(v) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(v).join("pnpm");
        dirs.extend([base.clone(), base.join("global/5")]);
    }
    if let Some(v) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(v);
        for p in [
            ".local/bin",
            ".opencode/bin",
            ".bun/bin",
            ".bun/install/global",
            ".local/share/pnpm",
            "scoop/shims",
        ] {
            dirs.push(home.join(p));
        }
    }
    if let Some(path) = std::env::var_os(format!("SUPERCODE_{}_PATH", id.to_ascii_uppercase())) {
        if let Some(v) = from_path(id, &PathBuf::from(path), node.as_deref(), "local") {
            return Some(v);
        }
    }
    dirs.into_iter()
        .find_map(|p| at_root(id, &p, node.as_deref(), "local"))
}
fn node_program() -> Option<PathBuf> {
    process::find_program("node").filter(|p| native_binary(p))
}
fn from_path(id: &str, path: &Path, node: Option<&Path>, source: &str) -> Option<Launch> {
    if !path.is_file() || !path.is_absolute() {
        return None;
    }
    let suffix = path.extension().and_then(|v| v.to_str()).unwrap_or("");
    if matches!(suffix.to_ascii_lowercase().as_str(), "cmd" | "ps1" | "bat") {
        return at_root(id, path.parent()?, node, source);
    }
    if id == "pi" && matches!(suffix, "js" | "mjs") {
        return Some(Launch {
            program: node?.into(),
            prefix: vec![ordinary(path.into()).to_string_lossy().into()],
            source: source.into(),
        });
    }
    if cfg!(windows) && !suffix.eq_ignore_ascii_case("exe") {
        return None;
    }
    if !native_binary(path) {
        return None;
    }
    Some(Launch {
        program: ordinary(path.into()),
        prefix: vec![],
        source: source.into(),
    })
}
pub fn resolve(app: &AppHandle, id: &str) -> Result<Launch, String> {
    validate(id)?;
    let state = app.state::<AppState>();
    let configured = state.store.setting(&format!("agent_path_{id}"))?;
    if let Some(s) = configured.as_deref().filter(|s| *s != "null") {
        let v: Launch = serde_json::from_str(s).map_err(|_| "Agent 配置无效")?;
        if v.valid() {
            return Ok(v);
        }
    }
    // Preserve manual selection from old versions. Explicit null restores automatic detection.
    if id == "codex" && configured.is_none() {
        if let Some(p) = state.store.codex_path()? {
            if let Some(v) = from_path(id, Path::new(&p), node_program().as_deref(), "configured") {
                return Ok(v);
            }
        }
    }
    if let Some(v) = local(id) {
        return Ok(v);
    }
    if let Some(s) = state.store.setting(&format!("agent_install_{id}"))? {
        let v: Launch = serde_json::from_str(&s).map_err(|_| "Agent 安装记录无效")?;
        if v.valid() {
            return Ok(v);
        }
    }
    Err(format!("未找到 {}，请到设置 → Agent 一键安装", name(id)))
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub path: Option<String>,
    pub connected: bool,
    pub source: Option<String>,
    pub version: Option<String>,
    pub phase: String,
    pub message: String,
    pub received: Option<u64>,
    pub total: Option<u64>,
}
pub fn status(app: &AppHandle, id: &str) -> AgentStatus {
    let launch = resolve(app, id).ok();
    AgentStatus {
        id: id.into(),
        name: name(id).into(),
        installed: launch.is_some(),
        path: launch.as_ref().map(|v| {
            v.prefix
                .first()
                .cloned()
                .unwrap_or_else(|| v.program.to_string_lossy().into())
        }),
        connected: true,
        source: launch.as_ref().map(|v| v.source.clone()),
        version: None,
        phase: if launch.is_some() { "ready" } else { "missing" }.into(),
        message: String::new(),
        received: None,
        total: None,
    }
}
#[derive(Default)]
pub struct Agents {
    pub(crate) operation: AsyncMutex<()>,
    tasks: Mutex<BTreeMap<String, (AgentStatus, Arc<AtomicBool>)>>,
}
pub(crate) fn progress(
    app: &AppHandle,
    id: &str,
    phase: &str,
    message: &str,
    received: Option<u64>,
    total: Option<u64>,
    cancel: Arc<AtomicBool>,
) {
    let mut s = status(app, id);
    s.phase = phase.into();
    s.message = message.into();
    s.received = received;
    s.total = total;
    app.state::<Agents>()
        .tasks
        .lock()
        .unwrap()
        .insert(id.into(), (s.clone(), cancel));
    let _ = app.emit("agent-install-progress", s);
}
pub async fn capture(launch: &Launch, args: &[&str], cwd: &Path) -> Result<String, String> {
    let mut child = launch
        .command(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let _job = process::JobGuard::attach(&child)?;
    let mut out = child.stdout.take().ok_or("Agent 无输出")?.take(16384);
    let mut err = child.stderr.take().ok_or("Agent 无错误管道")?.take(16384);
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        let mut a = String::new();
        let mut b = String::new();
        let (a_read, b_read) = tokio::join!(out.read_to_string(&mut a), err.read_to_string(&mut b));
        a_read.map_err(|e| e.to_string())?;
        b_read.map_err(|e| e.to_string())?;
        let exit = child.wait().await.map_err(|e| e.to_string())?;
        if !exit.success() {
            return Err("Agent 启动检查失败，请修复或更新安装".into());
        }
        Ok(a)
    })
    .await
    .map_err(|_| "Agent 检测超时".to_owned())?;
    result
}
#[tauri::command]
pub async fn list_agents(app: AppHandle) -> Result<Vec<AgentStatus>, String> {
    let mut result = vec![];
    for id in IDS {
        let mut s = status(&app, id);
        if let Some((v, _)) = app.state::<Agents>().tasks.lock().unwrap().get(id).cloned() {
            s.phase = v.phase;
            s.message = v.message;
            s.received = v.received;
            s.total = v.total;
        }
        if !matches!(s.phase.as_str(), "downloading" | "installing" | "testing") {
            if let Ok(v) = resolve(&app, id) {
                match capture(&v, &["--version"], &app.state::<AppState>().data_dir).await {
                    Ok(v) => {
                        s.version = Some(crate::protocol::bounded(v.trim(), 200));
                        if let Err(error) = compatible_version(id, &v) {
                            s.phase = "failed".into();
                            s.message = error;
                        }
                    }
                    Err(e) => {
                        s.phase = "failed".into();
                        s.message = e;
                    }
                }
            }
        }
        result.push(s);
    }
    Ok(result)
}
pub(crate) fn compatible_version(id: &str, version: &str) -> Result<(), String> {
    if id == "pi"
        && version
            .trim()
            .split('.')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .is_some_and(|n| n < 1)
    {
        return Err("此 Pi 版本不支持当前 MCP 接入，请修复安装后使用".into());
    }
    Ok(())
}
#[tauri::command]
pub fn cancel_agent_install(id: String, app: AppHandle) -> Result<(), String> {
    validate(&id)?;
    if let Some((_, flag)) = app.state::<Agents>().tasks.lock().unwrap().get(&id) {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}
#[tauri::command]
pub async fn test_agent(id: String, app: AppHandle) -> Result<Vec<AgentStatus>, String> {
    validate(&id)?;
    if app.state::<AppState>().store.running()? > 0 {
        return Err("请先结束当前任务".into());
    }
    let manager = app.state::<Agents>();
    let _lock = manager
        .operation
        .try_lock()
        .map_err(|_| "正在安装或测试 Agent")?;
    if app.state::<AppState>().store.running()? > 0 {
        return Err("请先结束当前任务".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    progress(
        &app,
        &id,
        "testing",
        "正在测试原生连接",
        None,
        None,
        cancel.clone(),
    );
    let result = async {
        let launch = resolve(&app, &id)?;
        crate::native_agents::probe_cancel(&id, &launch, &app.state::<AppState>().data_dir, &cancel)
            .await
    }
    .await;
    progress(
        &app,
        &id,
        if result.is_ok() {
            "ready"
        } else if cancel.load(Ordering::Relaxed) {
            "cancelled"
        } else {
            "failed"
        },
        result
            .as_ref()
            .err()
            .map(String::as_str)
            .unwrap_or("原生连接测试通过"),
        None,
        None,
        cancel,
    );
    result?;
    list_agents(app.clone()).await
}
#[tauri::command]
pub async fn install_agent(
    id: String,
    repair: Option<bool>,
    version: Option<String>,
    app: AppHandle,
) -> Result<Vec<AgentStatus>, String> {
    validate(&id)?;
    let manager = app.state::<Agents>();
    let _operation = manager
        .operation
        .try_lock()
        .map_err(|_| "正在安装另一个 Agent")?;
    if app.state::<AppState>().store.running()? > 0 {
        return Err("请先完成或停止当前任务".into());
    }
    if !repair.unwrap_or(false) && resolve(&app, &id).is_ok() {
        return list_agents(app.clone()).await;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    progress(
        &app,
        &id,
        "downloading",
        "正在准备独立安装",
        None,
        None,
        cancel.clone(),
    );
    let result = async {
        let target = app.state::<crate::agent_versions::Versions>().target(&id, version.as_deref()).await?;
        if cancel.load(Ordering::Relaxed) { return Err("已取消，原有配置已保留".into()); }
        let package = crate::agent_versions::package_spec(&id, &target)?;
        let previous_resolved = resolve(&app, &id).ok();
        let root = app
            .state::<AppState>()
            .data_dir
            .join("agents")
            .join(&id)
            .join(uuid::Uuid::new_v4().to_string());
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|e| e.to_string())?;
        let root = ordinary(root.canonicalize().map_err(|e| e.to_string())?);
        let node = crate::automation::install_agent_package(
            &app,
            &id,
            &root,
            &package,
            cancel.clone(),
        )
        .await?;
        let launch = at_root(&id, &root, Some(&node), "managed").ok_or("安装包缺少 Agent 程序")?;
        progress(
            &app,
            &id,
            "testing",
            "正在检查版本和原生连接",
            None,
            None,
            cancel.clone(),
        );
        let version = capture(&launch, &["--version"], &root).await?;
        if crate::agent_versions::installed_version(&version).map(|v| v.to_string()).as_deref() != Some(target.as_str()) {
            return Err("安装的版本与选定版本不一致，原版本继续使用".into());
        }
        compatible_version(&id, &version)?;
        crate::native_agents::probe_cancel(&id, &launch, &root,&cancel).await?;
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消，原有配置已保留".into());
        }
        if app.state::<AppState>().store.running()? > 0 {
            return Err("任务已开始，安装已保留；空闲时重试启用".into());
        }
        let key = format!("agent_install_{id}");
        let old = app.state::<AppState>().store.setting(&key)?;
        let override_path=app.state::<AppState>().store.setting(&format!("agent_path_{id}"))?;
        // The old executable/version remains intact; snapshots contain paths only, never account tokens.
        tokio::fs::write(
            root.join("previous-installation.json"),
            serde_json::to_vec_pretty(&json!({"agent":id,"previous":old,"previousOverride":override_path,"previousResolved":previous_resolved,"version":version.trim()}))
                .map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| e.to_string())?;
        app.state::<AppState>().store.commit_agent_install(&id,&serde_json::to_string(&launch).map_err(|e|e.to_string())?,repair.unwrap_or(false))?;
        // The routing configuration can remain identical after an executable update.
        // Drop warm clients while the operation gate blocks new sends.
        app.state::<AppState>().runtime.shutdown().await;
        app.state::<AppState>().claude.shutdown().await;
        app.state::<AppState>().native.shutdown().await;
        Ok::<_, String>(())
    }
    .await;
    let preserved = result.is_err() && resolve(&app, &id).is_ok();
    let message = match &result {
        Err(error) if preserved => format!("更新未完成，仍使用原版本：{error}"),
        Err(error) => error.clone(),
        Ok(_) => "安装与连接测试通过".into(),
    };
    progress(
        &app,
        &id,
        if result.is_ok() {
            "ready"
        } else if cancel.load(Ordering::Relaxed) {
            "cancelled"
        } else if preserved {
            "updateFailed"
        } else {
            "failed"
        },
        &message,
        None,
        None,
        cancel,
    );
    let _ = app.emit("workspace-updated", ());
    result?;
    list_agents(app.clone()).await
}
#[tauri::command]
pub async fn update_agent(
    id: String,
    version: Option<String>,
    app: AppHandle,
) -> Result<Vec<AgentStatus>, String> {
    validate(&id)?;
    install_agent(id, Some(true), version, app).await
}
#[tauri::command]
pub async fn configure_agent(
    id: String,
    path: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    validate(&id)?;
    if app.state::<AppState>().store.running()? > 0 {
        return Err("请先停止当前任务".into());
    }
    let manager = app.state::<Agents>();
    let _operation = manager
        .operation
        .try_lock()
        .map_err(|_| "正在安装或测试 Agent")?;
    if app.state::<AppState>().store.running()? > 0 {
        return Err("请先停止当前任务".into());
    }
    let key = format!("agent_path_{id}");
    let value = if let Some(p) = path.filter(|p| !p.trim().is_empty()) {
        let p = Path::new(&p).canonicalize().map_err(|_| "程序路径不存在")?;
        let launch = from_path(&id, &p, node_program().as_deref(), "configured")
            .ok_or("请选择原生程序或 Pi 的 cli.js")?;
        capture(&launch, &["--version"], &app.state::<AppState>().data_dir).await?;
        Some(serde_json::to_string(&launch).map_err(|e| e.to_string())?)
    } else {
        None
    };
    app.state::<AppState>()
        .store
        .set_setting(&key, value.as_deref().unwrap_or("null"))?; // resolve treats null as no override below.
    app.state::<AppState>().runtime.shutdown().await;
    app.state::<AppState>().claude.shutdown().await;
    app.state::<AppState>().native.shutdown().await;
    let _ = app.emit("workspace-updated", ());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_verbatim_paths() {
        assert_eq!(
            ordinary(PathBuf::from(r"\\?\C:\space folder\pi.js")),
            PathBuf::from(r"C:\space folder\pi.js")
        );
        assert_eq!(
            ordinary(PathBuf::from(r"\\?\UNC\host\share\pi.js")),
            PathBuf::from(r"\\host\share\pi.js")
        );
    }
    #[test]
    fn only_allowlisted_agents_can_install() {
        assert!(validate("pi").is_ok());
        assert!(validate("codex & evil").is_err());
        assert_eq!(
            crate::agent_versions::package_name("pi").unwrap(),
            "@earendil-works/pi-coding-agent"
        );
    }
    #[test]
    fn discovery_ignores_shell_wrappers_without_real_program() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let p = root.join("pi.cmd");
        std::fs::write(&p, "untrusted shell").unwrap();
        assert!(from_path("pi", &p, None, "local").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn discovery_skips_npm_exe_placeholder_and_wrong_architecture() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let placeholder = root.join("node_modules/@anthropic-ai/claude-code/bin/claude.exe");
        std::fs::create_dir_all(placeholder.parent().unwrap()).unwrap();
        std::fs::write(&placeholder, b"platform package placeholder").unwrap();
        assert!(at_root("claude", &root, None, "managed").is_none());
        let binary = root.join(native_paths("claude")[1].clone());
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        let mut bytes = vec![0; 128];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        std::fs::write(&binary, &bytes).unwrap();
        assert_eq!(
            at_root("claude", &root, None, "managed").unwrap().program,
            binary
        );
        bytes[68..70].copy_from_slice(&0x1c4u16.to_le_bytes());
        std::fs::write(&binary, bytes).unwrap();
        assert!(at_root("claude", &root, None, "managed").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
