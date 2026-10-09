use crate::{
    process, protocol,
    storage::{Message, Project, Session},
    AppState,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    id: String,
    name: String,
    installed: bool,
    path: Option<String>,
    connected: bool,
}

#[derive(Serialize)]
pub struct Bootstrap {
    projects: Vec<Project>,
    sessions: Vec<Session>,
    agents: Vec<AgentInfo>,
    #[serde(rename = "codexPath")]
    codex_path: Option<String>,
    #[serde(rename = "loadMcp")]
    load_mcp: bool,
    profiles: Vec<crate::ccswitch::Summary>,
    #[serde(rename = "officialAgents")]
    official_agents: Vec<String>,
    #[serde(rename = "connectionOrder")]
    connection_order: std::collections::BTreeMap<String, Vec<String>>,
    sidebar: crate::sidebar::SidebarState,
}

#[tauri::command]
pub fn bootstrap(state: State<'_, AppState>, app: AppHandle) -> Result<Bootstrap, String> {
    let codex_path = state.store.codex_path()?;
    let connection_order = state.store.connection_orders()?;
    let profiles = state
        .store
        .profiles()?
        .iter()
        .map(|p| {
            p.summary(connection_order.get(&p.agent).and_then(|ids| ids.first()) == Some(&p.id))
        })
        .collect();
    let official_agents = connection_order
        .iter()
        .filter(|(_, ids)| {
            ids.first()
                .is_some_and(|id| id == crate::session_config::OFFICIAL)
        })
        .map(|(agent, _)| agent.clone())
        .collect();
    let agents = crate::agents::IDS
        .into_iter()
        .map(|id| {
            let s = crate::agents::status(&app, id);
            AgentInfo {
                id: s.id,
                name: s.name,
                installed: s.installed,
                path: s.path,
                connected: s.connected,
            }
        })
        .collect();
    Ok(Bootstrap {
        projects: state.store.projects()?,
        sessions: state.store.sessions()?,
        sidebar: state.store.sidebar()?,
        agents,
        codex_path,
        load_mcp: state.store.load_mcp()?,
        connection_order,
        official_agents,
        profiles,
    })
}

#[tauri::command]
pub async fn configure_codex(
    path: Option<String>,
    load_mcp: bool,
    app: AppHandle,
) -> Result<(), String> {
    let path = path
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let path =
                std::fs::canonicalize(p.trim()).map_err(|e| format!("无法找到 CLI 文件：{e}"))?;
            if !path.is_file() {
                return Err("请选择 Codex CLI 的程序文件".to_owned());
            }
            if cfg!(windows)
                && !path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("cmd"))
            {
                return Err("Windows 下请选择 codex.exe 或 codex.cmd".to_owned());
            }
            Ok(path.to_string_lossy().into_owned())
        })
        .transpose()?;
    let state = app.state::<AppState>();
    state.runtime.release(&app).await?;
    state.store.configure_codex(path.as_deref(), load_mcp)
}

#[tauri::command]
pub fn add_project(
    path: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Project, String> {
    let path = std::fs::canonicalize(path).map_err(|e| format!("无法访问项目文件夹：{e}"))?;
    if !path.is_dir() {
        return Err("请选择文件夹".into());
    }
    let project = state.store.add_project(&path)?;
    let _ = app.emit("workspace-updated", ());
    Ok(project)
}

#[tauri::command]
pub fn create_session(
    project_id: String,
    model: Option<String>,
    agent: Option<String>,
    connection_id: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Session, String> {
    let session = if project_id.is_empty() {
        let root = if std::env::args().any(|a| a == "--smoke-test") {
            state.data_dir.join("workspaces")
        } else {
            app.path()
                .document_dir()
                .map_err(|e| e.to_string())?
                .join("SuperCode")
        };
        state.store.create_projectless_session(
            &root,
            model,
            agent.as_deref().unwrap_or("codex"),
            connection_id.as_deref(),
        )?
    } else {
        state.store.create_configured_session(
            &project_id,
            model,
            agent.as_deref().unwrap_or("codex"),
            connection_id.as_deref(),
        )?
    };
    let _ = app.emit("workspace-updated", ());
    Ok(session)
}

#[tauri::command]
pub fn list_messages(
    session_id: String,
    before: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<Message>, String> {
    let activity = state.activity.lock().map_err(|e| e.to_string())?;
    let mut messages = state.store.messages(&session_id, before)?;
    if before.is_none() {
        if let Some(native) = state.store.session(&session_id)?.native_id {
            for item in activity.snapshot(&native) {
                if let Some(message) = messages.iter_mut().find(|m| item["id"] == m.id) {
                    if let Some((role, text, kind, data)) = protocol::item_message(&item) {
                        message.role = role;
                        message.text = text;
                        message.kind = kind;
                        message.data = data;
                    }
                }
            }
        }
    }
    Ok(messages)
}

#[tauri::command]
pub async fn list_models(
    agent: Option<String>,
    session_id: Option<String>,
    connection_id: Option<String>,
    app: AppHandle,
) -> Result<Value, String> {
    let agent = agent.as_deref().unwrap_or("codex");
    let route = crate::chat_connection::available_route(
        &app, agent, session_id.as_deref(), connection_id.as_deref(),
    ).await?;
    let source = crate::chat_connection::model_source(route.as_ref(), agent);
    let Some(route) = route else {
        return Ok(json!({"data":[],"source":source}));
    };
    if matches!(agent, "opencode" | "pi") {
        let mut catalog = crate::providers::catalog_models(&route.config);
        catalog["source"] = source;
        return Ok(catalog);
    }
    if agent == "claude" {
        let official = route.config["official"] == true
            || route.profile.as_ref().is_some_and(crate::ccswitch::Profile::is_official);
        let config = route.config;
        let mut catalog = crate::providers::catalog_models(&config);
        catalog["source"] = source.clone();
        if catalog["data"].as_array().is_some_and(|a| !a.is_empty()) {
            return Ok(catalog);
        }
        if !official {
            return Ok(json!({"data":[],"needsModel":true,"source":source}));
        }
        return Ok(
            json!({"data":[{"id":"sonnet","model":"sonnet","displayName":"Claude Sonnet","isDefault":true},{"id":"opus","model":"opus","displayName":"Claude Opus","isDefault":false},{"id":"haiku","model":"haiku","displayName":"Claude Haiku","isDefault":false}],"source":source}),
        );
    }
    let profile = route.profile;
    if let Some(profile) = &profile {
        if !profile.is_official() {
            let mut catalog = crate::providers::catalog_models(&profile.config);
            catalog["source"] = source;
            return Ok(catalog);
        }
    }
    if app
        .state::<AppState>()
        .claude
        .client
        .lock()
        .await
        .as_ref()
        .is_some_and(|c| c.active.load(std::sync::atomic::Ordering::Acquire))
    {
        return Err("Claude 正在运行，请先完成任务后加载 Codex 模型".into());
    }
    let mut catalog = app
        .state::<AppState>()
        .runtime
        .get_for_connection(&app, session_id.as_deref(), connection_id.as_deref())
        .await?
        .request("model/list", json!({"limit":100}))
        .await?;
    catalog["source"] = source;
    Ok(catalog)
}

#[tauri::command]
pub async fn send_message(
    session_id: String,
    text: String,
    model: Option<String>,
    read_only: bool,
    permission_mode: Option<String>,
    app: AppHandle,
) -> Result<Session, String> {
    send_chat_message(
        session_id,
        text,
        model,
        read_only,
        permission_mode,
        None,
        None,
        app,
    )
    .await
}

#[tauri::command]
pub async fn send_chat_message(
    session_id: String,
    text: String,
    model: Option<String>,
    read_only: bool,
    permission_mode: Option<String>,
    attachments: Option<Vec<crate::client_features::Attachment>>,
    effort: Option<String>,
    app: AppHandle,
) -> Result<Session, String> {
    send_chat_message_inner(
        session_id,
        text,
        model,
        read_only,
        permission_mode,
        attachments,
        effort,
        None,
        app,
    )
    .await
}
pub(crate) async fn send_chat_message_inner(
    session_id: String,
    text: String,
    model: Option<String>,
    read_only: bool,
    permission_mode: Option<String>,
    attachments: Option<Vec<crate::client_features::Attachment>>,
    effort: Option<String>,
    followup_id: Option<String>,
    app: AppHandle,
) -> Result<Session, String> {
    crate::desktop_lifecycle::ensure_running(&app)?;
    let agents = app.state::<crate::agents::Agents>();
    let _agent_operation = agents
        .operation
        .try_lock()
        .map_err(|_| "Agent 正在安装、更新或测试，请稍后发送")?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("请输入消息".into());
    }
    if text.len() > 128 * 1024 {
        return Err("消息过长，请将内容保存为项目文件后引用".into());
    }
    let state = app.state::<AppState>();
    let session = state.store.session(&session_id)?;
    crate::chat_connection::check(&app, &session.agent, Some(&session_id), None).await?;
    if session.native_id.is_some()
        && model
            .as_deref()
            .is_some_and(|m| session.model.as_deref() != Some(m))
    {
        return Err("请先在模型菜单中切换，切换完成后再发送消息".into());
    }
    if state.store.running()? > 0 {
        return Err("当前已有任务运行。第一版默认同时运行一个任务，请先完成或停止它。".into());
    }
    state.store.claim(&session_id)?;
    // The persisted starting state now blocks installation. Release before native
    // startup so an immediately completed turn can dispatch its queued successor.
    drop(_agent_operation);
    let permission = permission_mode
        .as_deref()
        .unwrap_or(if read_only { "read" } else { "ask" });
    if let Err(error) = crate::client_features::validate_permission(&session.agent, permission) {
        let _ = state.store.set_status(&session_id, "failed", None);
        return Err(error);
    }
    let mut prepared =
        match crate::client_features::prepare_input(&text, &attachments.unwrap_or_default()) {
            Ok(v) => v,
            Err(e) => {
                let _ = state.store.set_status(&session_id, "failed", None);
                return Err(e);
            }
        };
    if let Some(id) = followup_id {
        prepared.metadata["followupId"] = json!(id);
    }
    if effort.as_deref().is_some_and(|v| {
        !matches!(
            v,
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
        )
    }) {
        let _ = state.store.set_status(&session_id, "failed", None);
        return Err("推理档位无效".into());
    }
    let result = send_inner(
        &session_id,
        &text,
        model,
        read_only || permission == "read",
        permission,
        prepared,
        effort,
        &app,
    )
    .await;
    if result.is_err() {
        if state
            .store
            .session(&session_id)
            .is_ok_and(|s| s.status == "starting")
        {
            let _ = state.store.set_status(&session_id, "failed", None);
        }
    }
    result
}

async fn send_inner(
    session_id: &str,
    text: &str,
    model: Option<String>,
    read_only: bool,
    permission: &str,
    mut prepared: crate::client_features::PreparedInput,
    effort: Option<String>,
    app: &AppHandle,
) -> Result<Session, String> {
    let state = app.state::<AppState>();
    let session = state.store.session(session_id)?;
    if session.native_id.is_none() {
        if let Some(context) = state.store.history_context(session_id)? {
            let context = json!({"type":"text","text":context});
            prepared.codex.insert(0, context.clone());
            prepared.claude.insert(0, context);
        }
    }
    let project = state.store.session_workspace(&session)?;
    if !Path::new(&project.path).is_dir() {
        return Err("项目文件夹已不存在，请重新添加项目".into());
    }
    if matches!(session.agent.as_str(), "opencode" | "pi") {
        state.claude.shutdown().await;
        state.runtime.clear_for_agent_switch().await;
        return crate::native_agents::send(
            app,
            &session,
            &project.path,
            text,
            model.or(session.model.clone()),
            permission,
            prepared.claude,
            prepared.metadata,
            effort,
        )
        .await;
    }
    if session.agent == "claude" {
        state.native.shutdown().await;
        state.runtime.clear_for_agent_switch().await;
        return crate::claude::send(
            app,
            &session,
            &project.path,
            text,
            model.or(session.model.clone()),
            read_only,
            permission,
            prepared.claude,
            prepared.metadata,
            effort,
        )
        .await;
    }
    state.claude.shutdown().await;
    state.native.shutdown().await;
    let route = state.store.route("codex", Some(session_id))?;
    let client = state.runtime.get_for_session(app, Some(session_id)).await?;
    let profile_model = route.config["model"].as_str().map(str::to_owned);
    let model = match model.or(session.model.clone()).or(profile_model) {
        Some(model) => Some(model),
        None => {
            let catalog = client.request("model/list", json!({"limit":100})).await?;
            catalog["data"]
                .as_array()
                .and_then(|models| models.iter().find(|m| m["isDefault"] == true))
                .and_then(|m| m["model"].as_str())
                .map(str::to_owned)
        }
    };
    let mode = if read_only { "read" } else { permission };
    let (sandbox, approval, mut policy) = crate::client_features::permission(mode)?;
    let reviewer = crate::client_features::approvals_reviewer(mode);
    let already_loaded = session.native_id.as_ref().is_some_and(|_| false);
    let already_loaded = if let Some(id) = &session.native_id {
        client.loaded_threads.lock().await.contains(id)
    } else {
        already_loaded
    };
    let result = if already_loaded {
        json!({"thread":{"id":session.native_id}})
    } else if let Some(native) = &session.native_id {
        client.request("thread/resume",json!({"threadId":native,"cwd":project.path,"approvalPolicy":approval,"approvalsReviewer":reviewer,"sandbox":sandbox,"model":model,"modelProvider":route.provider()})).await?
    } else {
        client.request("thread/start",json!({"cwd":project.path,"approvalPolicy":approval,"approvalsReviewer":reviewer,"sandbox":sandbox,"model":model,"modelProvider":route.provider(),"developerInstructions":crate::client_features::instructions(app),"serviceName":"supercode"})).await?
    };
    let native = result["thread"]["id"]
        .as_str()
        .ok_or("Codex 未返回会话 ID")?;
    {
        let mut loaded = client.loaded_threads.lock().await;
        if !loaded.iter().any(|id| id == native) {
            if loaded.len() >= 64 {
                loaded.remove(0);
            }
            loaded.push(native.to_owned());
        }
    }
    state.store.bind_native(session_id, native)?;
    state.store.set_model(session_id, model.as_deref())?;
    let user_id = crate::client_features::user_message_id(&prepared.metadata);
    state.store.save_message(
        &user_id,
        session_id,
        "user",
        text,
        "userMessage",
        &prepared.metadata,
    )?;
    if session.title == "新会话" {
        state
            .store
            .rename(session_id, &text.chars().take(24).collect::<String>())?;
    }
    if policy["type"] == "workspaceWrite" {
        policy["writableRoots"] = json!([project.path]);
    }
    let inputs = if prepared.codex.iter().any(|v| v["type"] == "skill") {
        let catalog = client
            .request(
                "skills/list",
                json!({"cwds":[project.path],"forceReload":false}),
            )
            .await
            .unwrap_or(Value::Null);
        crate::client_features::resolve_codex_skills(&prepared, &catalog)
    } else {
        prepared.codex
    };
    let reply=client.request("turn/start",json!({"threadId":native,"input":inputs,"cwd":project.path,"model":model,"effort":effort,"summary":"auto","approvalPolicy":approval,"approvalsReviewer":reviewer,"sandboxPolicy":policy})).await?;
    let turn_id = reply["turn"]["id"].as_str().ok_or("Codex 未返回任务 ID")?;
    client.expect_turn(native, turn_id).await;
    *client.turn_settings.lock().await = Some((
        native.to_owned(),
        turn_id.to_owned(),
        json!([mode, effort]).to_string(),
    ));
    state.store.acknowledge_turn_start(session_id, turn_id)?;
    state.store.session(session_id)
}

#[tauri::command]
pub async fn interrupt_turn(session_id: String, app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let session = state.store.session(&session_id)?;
    if matches!(session.agent.as_str(), "opencode" | "pi") {
        return state.native.interrupt(&session_id).await;
    }
    if session.agent == "claude" {
        return state.claude.interrupt(&app, &session_id).await;
    }
    let slot = state.runtime.client.lock().await;
    let client = slot.as_ref().cloned().ok_or("当前没有运行中的 agent")?;
    drop(slot);
    let native = session.native_id.ok_or("会话尚未启动，请稍后再试")?;
    let turn = session.turn_id.ok_or("任务正在启动，请稍后再试")?;
    if client
        .request("thread/goal/get", json!({"threadId":native}))
        .await
        .is_ok_and(|v| v["goal"]["status"] == "active")
    {
        client
            .request(
                "thread/goal/set",
                json!({"threadId":native,"status":"paused"}),
            )
            .await?;
    }
    client.interrupt_turn(&native, &turn).await
}

#[tauri::command]
pub async fn respond_request(id: Value, result: Value, app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if id
        .as_str()
        .is_some_and(|id| id.starts_with("opencode:") || id.starts_with("pi:"))
    {
        return state.native.respond(&app, id, result).await;
    }
    if id.as_str().is_some_and(|id| id.starts_with("claude:")) {
        return state.claude.respond(&app, id, result).await;
    }
    let client = state
        .runtime
        .client
        .lock()
        .await
        .as_ref()
        .cloned()
        .ok_or("Agent 连接已关闭")?;
    client.respond(id, result).await
}

#[tauri::command]
pub fn rename_session(
    session_id: String,
    title: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    state.store.rename(&session_id, &title)?;
    let _ = app.emit("workspace-updated", ());
    Ok(())
}

#[tauri::command]
pub fn archive_session(
    session_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    state.store.archive(&session_id)?;
    let _ = app.emit("workspace-updated", ());
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    is_git: bool,
    branch: String,
    files: Vec<ChangedFile>,
    diff: String,
}

#[derive(Serialize)]
pub struct ChangedFile {
    path: String,
    status: String,
}

fn chat_files(root: &Path) -> Vec<ChangedFile> {
    fn collect(
        root: &Path,
        directory: &Path,
        depth: usize,
        files: &mut Vec<ChangedFile>,
        visited: &mut usize,
    ) {
        if depth > 4 || files.len() >= 200 || *visited >= 1000 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            if files.len() >= 200 || *visited >= 1000 {
                break;
            }
            *visited += 1;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with('.') || matches!(name, "node_modules" | "target" | "__pycache__") {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                collect(root, &entry.path(), depth + 1, files, visited);
            } else if kind.is_file() {
                if let Ok(path) = entry.path().strip_prefix(root) {
                    files.push(ChangedFile {
                        path: path.to_string_lossy().replace('\\', "/"),
                        status: "??".into(),
                    });
                }
            }
        }
    }
    let mut files = Vec::new();
    collect(root, root, 0, &mut files, &mut 0);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

async fn git(root: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let executable = process::find_program("git").ok_or("未安装 Git")?;
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let mut child = process::command(&executable, args)
            .current_dir(root)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let stdout = child.stdout.take().ok_or("无法读取 Git stdout")?;
        let stderr = child.stderr.take().ok_or("无法读取 Git stderr")?;
        let (out, err, status) = tokio::join!(
            read_limited(stdout, 2 * 1024 * 1024),
            read_limited(stderr, 64 * 1024),
            child.wait()
        );
        let status = status.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(
                String::from_utf8(err?).unwrap_or_else(|_| "Git 返回非 UTF-8 错误信息".into())
            );
        }
        out
    })
    .await
    .map_err(|_| "Git 请求超时")?
}

async fn read_limited<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut overflow = false;
    loop {
        let count = reader.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if bytes.len() + count <= limit {
            bytes.extend_from_slice(&buffer[..count]);
        } else {
            overflow = true;
        }
    }
    if overflow {
        Err("Git 输出过大，请缩小变更范围后再查看".into())
    } else {
        Ok(bytes)
    }
}

#[tauri::command]
pub async fn workspace_changes(
    project_id: String,
    session_id: Option<String>,
    app: AppHandle,
) -> Result<Changes, String> {
    let root = app
        .state::<AppState>()
        .store
        .workspace(&project_id, session_id.as_deref())?
        .path;
    if git(&root, &["rev-parse", "--is-inside-work-tree"])
        .await
        .is_err()
    {
        return Ok(Changes {
            is_git: false,
            branch: String::new(),
            files: if project_id.is_empty() {
                chat_files(Path::new(&root))
            } else {
                vec![]
            },
            diff: String::new(),
        });
    }
    let status = git(
        &root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
    )
    .await?;
    let status =
        String::from_utf8(status).map_err(|_| "Git 文件名不是 UTF-8，无法展示；未修改文件编码")?;
    let mut files = vec![];
    let mut entries = status.split('\0');
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let code = &entry[..2];
        let path = &entry[3..];
        files.push(ChangedFile {
            path: path.into(),
            status: code.trim().into(),
        });
        if code.contains('R') || code.contains('C') {
            entries.next();
        }
        if files.len() >= 200 {
            break;
        }
    }
    let branch = git(&root, &["branch", "--show-current"])
        .await
        .unwrap_or_default();
    let diff = git(&root, &["diff", "--no-ext-diff", "--no-color", "--", "."]).await?;
    let staged = git(
        &root,
        &["diff", "--cached", "--no-ext-diff", "--no-color", "--", "."],
    )
    .await?;
    let diff = String::from_utf8([diff, staged].concat())
        .map_err(|_| "Diff 包含非 UTF-8 内容，无法展示；未修改文件编码")?;
    Ok(Changes {
        is_git: true,
        branch: String::from_utf8(branch).unwrap_or_default().trim().into(),
        files,
        diff: protocol::bounded(&diff, 256 * 1024),
    })
}

fn contained_file(root: &Path, path: &str) -> Result<PathBuf, String> {
    contained_workspace_path(root, path, false)
}

fn contained_workspace_path(root: &Path, path: &str, directory: bool) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let candidate = std::fs::canonicalize(root.join(path)).map_err(|e| e.to_string())?;
    if !candidate.starts_with(&root) || !(candidate.is_file() || directory && candidate.is_dir()) {
        return Err("只能查看当前工作目录内的文件".into());
    }
    Ok(candidate)
}

fn system_document(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "pdf"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "odt"
            | "odp"
            | "ods"
            | "odf"
            | "rtf"
            | "txt"
            | "md"
            | "csv"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "svg"
            | "bmp"
            | "ico"
            | "mp3"
            | "mp4"
            | "wav"
            | "ogg"
            | "flac"
            | "mkv"
            | "mov"
            | "webm"
            | "htm"
            | "html"
    )
}

#[tauri::command]
pub fn open_project_path(
    project_id: String,
    session_id: Option<String>,
    path: String,
    action: String,
    app: AppHandle,
) -> Result<(), String> {
    let root = app
        .state::<AppState>()
        .store
        .workspace(&project_id, session_id.as_deref())?
        .path;
    let file = contained_workspace_path(Path::new(&root), &path, action == "reveal")?;
    match action.as_str() {
        "reveal" => app
            .opener()
            .reveal_item_in_dir(file)
            .map_err(|error| error.to_string()),
        "open" if system_document(&file) => app
            .opener()
            .open_path(file.to_string_lossy().into_owned(), None::<&str>)
            .map_err(|error| error.to_string()),
        "open" => Err("请在应用中查看源代码，不能直接运行程序或脚本".into()),
        _ => Err("不支持的文件操作".into()),
    }
}

#[tauri::command]
pub fn read_project_file(
    project_id: String,
    session_id: Option<String>,
    path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let root = state
        .store
        .workspace(&project_id, session_id.as_deref())?
        .path;
    let path = contained_file(Path::new(&root), &path)?;
    if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
        return Err("文件超过 1 MB，请在外部编辑器中查看".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err("该文件使用 UTF-8 BOM，未转换或修改文件编码".into());
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| "该文件不是有效的 UTF-8 文本，未转换或修改文件编码")?;
    if text.contains('\0') {
        return Err("该文件是二进制内容，不能作为文本预览".into());
    }
    Ok(text)
}

#[tauri::command]
pub async fn runtime_info(app: AppHandle) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let slot = state.runtime.client.lock().await;
    let client = slot.as_ref().cloned();
    drop(slot);
    let mut requests = if let Some(c) = &client {
        c.requests
            .lock()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    let claude_client = state.claude.client.lock().await.as_ref().cloned();
    let native_client = state.native.client.lock().await.as_ref().cloned();
    if let Some(c) = &native_client {
        requests.extend(c.requests.lock().await.values().cloned());
    }
    if let Some(c) = &claude_client {
        requests.extend(c.requests.lock().await.values().cloned());
    }
    let pid = std::process::id();
    let warm_pid = state.native.warm_pid().await;
    let agent_pid = claude_client
        .as_ref()
        .map(|c| c.pid)
        .or_else(|| native_client.as_ref().map(|c| c.pid()))
        .or(warm_pid)
        .or_else(|| client.as_ref().map(|c| c.pid));
    let memory = tauri::async_runtime::spawn_blocking(move || {
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing().with_memory(),
        );
        let mut shell = 0u64;
        let mut agent = 0u64;
        for (process_id, process) in system.processes() {
            let mut cursor = Some(*process_id);
            let mut is_agent = false;
            let mut in_app = false;
            for _ in 0..20 {
                let Some(current) = cursor else {
                    break;
                };
                if Some(current.as_u32()) == agent_pid {
                    is_agent = true;
                }
                if current.as_u32() == pid {
                    in_app = true;
                    break;
                }
                cursor = system.process(current).and_then(|p| p.parent());
            }
            if in_app {
                if is_agent {
                    agent += process.memory();
                } else {
                    shell += process.memory();
                }
            }
        }
        (shell, agent)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(
        json!({"running":warm_pid.is_some() || native_client.is_some() || claude_client.is_some() || client.as_ref().is_some_and(|c|c.alive.load(Ordering::Relaxed)),"pid":agent_pid,"shellBytes":memory.0,"agentBytes":memory.1,"requests":requests,"idleReleaseSeconds":300,"memoryMetric":"workingSet"}),
    )
}

#[tauri::command]
pub async fn release_runtime(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.runtime.release(&app).await?;
    state.claude.shutdown().await;
    state.native.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_open_accepts_documents_without_running_code_or_executables() {
        for name in [
            "报告.PDF",
            "diagram.png",
            "notes.txt",
            "report.docx",
            "data.xlsx",
        ] {
            assert!(system_document(Path::new(name)));
        }
        for name in [
            "app.exe",
            "run.cmd",
            "start.bat",
            "script.ps1",
            "script.js",
            "script.py",
            "App.tsx",
            "file",
        ] {
            assert!(!system_document(Path::new(name)));
        }
    }
    #[test]
    fn preview_rejects_paths_outside_the_project() {
        let base = std::env::temp_dir().join(format!("supercode-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(base.join("project")).unwrap();
        std::fs::write(base.join("project/中文.txt"), "中文").unwrap();
        std::fs::write(base.join("outside.txt"), "private").unwrap();
        assert!(contained_file(&base.join("project"), "中文.txt").is_ok());
        assert!(contained_file(&base.join("project"), "../outside.txt").is_err());
        assert!(contained_workspace_path(&base.join("project"), ".", true).is_ok());
        assert!(contained_workspace_path(&base.join("project"), "..", true).is_err());
        assert!(contained_file(&base.join("project"), ".").is_err());
        std::fs::create_dir_all(base.join("project/nested")).unwrap();
        std::fs::create_dir_all(base.join("project/node_modules")).unwrap();
        std::fs::write(base.join("project/nested/result.html"), "<p>result</p>").unwrap();
        std::fs::write(base.join("project/node_modules/hidden.js"), "x").unwrap();
        assert_eq!(
            chat_files(&base.join("project"))
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            ["nested/result.html", "中文.txt"]
        );
        // The exact temporary directory is owned by this test.
        std::fs::remove_dir_all(base).unwrap();
    }
}

#[tauri::command]
pub async fn pending_requests(app: AppHandle) -> Result<Vec<Value>, String> {
    let state = app.state::<AppState>();
    let codex = state.runtime.client.lock().await.clone();
    let claude = state.claude.client.lock().await.clone();
    let mut requests = vec![];
    if let Some(client) = codex {
        requests.extend(client.requests.lock().await.values().cloned());
    }
    if let Some(client) = claude {
        requests.extend(client.requests.lock().await.values().cloned());
    }
    if let Some(client) = state.native.client.lock().await.clone() {
        requests.extend(client.requests.lock().await.values().cloned());
    }
    Ok(requests)
}
