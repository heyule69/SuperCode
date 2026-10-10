use crate::{process, AppState};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

pub fn permission(mode: &str) -> Result<(&'static str, &'static str, Value), String> {
    match mode {
        "read" => Ok(("read-only", "on-request", json!({"type":"readOnly"}))),
        "ask" | "strict" | "auto" => Ok((
            "workspace-write",
            "on-request",
            json!({"type":"workspaceWrite","networkAccess":false}),
        )),
        "full" => Ok((
            "danger-full-access",
            "never",
            json!({"type":"dangerFullAccess"}),
        )),
        _ => Err("权限模式无效".into()),
    }
}

pub fn approvals_reviewer(mode: &str) -> &'static str {
    if mode == "auto" {
        "auto_review"
    } else {
        "user"
    }
}

pub fn validate_permission(agent: &str, mode: &str) -> Result<(), String> {
    let valid = match agent {
        "codex" => permission(mode).is_ok(),
        "claude" => matches!(
            mode,
            "read" | "ask" | "strict" | "auto" | "edit" | "deny" | "full"
        ),
        "opencode" | "pi" => matches!(mode, "read" | "ask" | "strict" | "full"),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err("此 Agent 不支持所选审批规则".into())
    }
}

pub fn claude_native_permission(mode: &str) -> Result<&'static str, String> {
    validate_permission("claude", mode)?;
    Ok(match mode {
        "read" => "plan",
        "ask" | "strict" => "manual",
        "auto" => "auto",
        "edit" => "acceptEdits",
        "deny" => "dontAsk",
        "full" => "bypassPermissions",
        _ => unreachable!(),
    })
}

pub(crate) fn validate_steering_settings(
    signature: &str,
    permission: &str,
    effort: Option<&str>,
) -> Result<(), String> {
    let settings: Value = serde_json::from_str(signature).map_err(|_| "活动任务配置无效")?;
    let settings = settings.as_array().ok_or("活动任务配置无效")?;
    if settings.len() < 2
        || settings[settings.len() - 2] != permission
        || settings[settings.len() - 1] != json!(effort)
    {
        return Err("引导消息的权限或推理设置与当前任务不同，请保留排队发送".into());
    }
    Ok(())
}

pub fn instructions(app: &AppHandle) -> String {
    let text = app
        .state::<AppState>()
        .store
        .setting("custom_instructions")
        .ok()
        .flatten()
        .unwrap_or_default();
    let tools = automation_instructions(&servers(app).unwrap_or_default());
    format!("Read and write every project text file as UTF-8 without BOM. Tell the user before converting non-UTF-8 files.\nYou are running inside the SuperCode graphical desktop app. The conversation supports inline images, video and audio, not just terminal text. Tool image results are displayed automatically. To show a local media file, write ![descriptive caption](<absolute file path>); use an absolute path with spaces inside angle brackets. The same syntax supports video and audio files. Use ordinary Markdown links for files that should only be referenced. Do not open another application or copy screenshots to the desktop merely to make media visible in the conversation. Never claim a media file was shown unless you actually returned the media result or referenced an existing file.\n{tools}\n{text}")
}

#[tauri::command]
pub fn get_client_preferences(app: AppHandle) -> Result<Value, String> {
    let s = app.state::<AppState>();
    Ok(json!({"instructions":s.store.setting("custom_instructions")?.unwrap_or_default()}))
}
#[tauri::command]
pub fn save_client_preferences(instructions: String, app: AppHandle) -> Result<(), String> {
    if instructions.len() > 16 * 1024 {
        return Err("自定义指令不能超过 16 KB".into());
    }
    app.state::<AppState>()
        .store
        .set_setting("custom_instructions", &instructions)
}
#[tauri::command]
pub fn get_usage(session_id: Option<String>, app: AppHandle) -> Result<Value, String> {
    Ok(json!({"records":app.state::<AppState>().store.usage_records(session_id.as_deref())?}))
}
#[tauri::command]
pub async fn get_account_limits(app: AppHandle) -> Result<Value, String> {
    let state = app.state::<AppState>();
    if state
        .store
        .active_profile("codex")?
        .is_some_and(|p| !p.is_official())
    {
        return Err("API 连接不提供 ChatGPT 账号额度".into());
    }
    state
        .runtime
        .get(&app)
        .await?
        .request("account/rateLimits/read", json!({}))
        .await
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub kind: String,
    pub path: String,
    pub name: String,
}
#[derive(Debug)]
pub struct PreparedInput {
    pub codex: Vec<Value>,
    pub claude: Vec<Value>,
    pub metadata: Value,
}
pub fn prepare_input(text: &str, attachments: &[Attachment]) -> Result<PreparedInput, String> {
    if attachments.len() > 12 {
        return Err("最多添加 12 个附件".into());
    }
    let mut codex = vec![json!({"type":"text","text":text})];
    let mut claude = vec![json!({"type":"text","text":text})];
    let mut expanded = text.to_string();
    let mut metadata = vec![];
    let mut image_bytes = 0u64;
    for a in attachments {
        let path = Path::new(&a.path)
            .canonicalize()
            .map_err(|_| format!("附件不存在：{}", a.name))?;
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let p = path.to_string_lossy().to_string();
        match a.kind.as_str() {
            "image" => {
                let bytes = crate::image_attachments::read_image(
                    &path,
                    crate::image_attachments::IMAGE_LIMIT,
                )?;
                let size = bytes.len() as u64;
                image_bytes += size;
                if image_bytes > 20 * 1024 * 1024 {
                    return Err("图片总量不能超过 20 MB".into());
                }
                let (_, mime, _) = crate::image_attachments::image_format(&bytes)?;
                codex.push(json!({"type":"localImage","path":p}));
                claude.push(json!({"type":"image","source":{"type":"base64","media_type":mime,"data":base64::engine::general_purpose::STANDARD.encode(bytes)}}));
            }
            "skill" => {
                if name != "SKILL.md" {
                    return Err("技能必须选择 SKILL.md".into());
                }
                let content = read_utf8(&path, 128 * 1024)?;
                expanded.push_str(&content);
                codex.push(json!({"type":"skill","name":a.name,"path":p}));
                claude.push(json!({"type":"text","text":format!("用户选择的技能：{}\n路径：{}\n{}",a.name,p,content)}));
            }
            "file" => {
                let content = read_utf8(&path, 256 * 1024)?;
                let extra = format!("\n\n用户附加的文件（路径：{}）：\n{}", p, content);
                expanded.push_str(&extra);
                codex.push(json!({"type":"text","text":extra}));
                claude.push(json!({"type":"text","text":extra}));
            }
            "directory" => {
                if !path.is_dir() {
                    return Err("附件不是目录".into());
                }
                let extra = format!("\n用户选择的目录：{}。按需读取其内容。", p);
                expanded.push_str(&extra);
                codex.push(json!({"type":"text","text":extra}));
                claude.push(json!({"type":"text","text":extra}));
            }
            _ => return Err("附件类型无效".into()),
        }
        metadata.push(
            json!({"kind":a.kind,"path":p,"name":if a.kind == "image" { &a.name } else { &name }}),
        );
    }
    if expanded.len() > 1024 * 1024 {
        return Err("附件文本总量不能超过 1 MB".into());
    }
    let bytes = claude
        .iter()
        .map(|v| v["source"]["data"].as_str().map(str::len).unwrap_or(0))
        .sum::<usize>();
    if bytes > 28 * 1024 * 1024 {
        return Err("图片总量不能超过 20 MB".into());
    }
    Ok(PreparedInput {
        codex,
        claude,
        metadata: json!({"attachments":metadata}),
    })
}
pub fn read_utf8(path: &Path, limit: u64) -> Result<String, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > limit {
        return Err("文件过大".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("文件过大".into());
    }
    if bytes.starts_with(&[239, 187, 191]) {
        return Err("文件带 UTF-8 BOM，未更改编码".into());
    }
    String::from_utf8(bytes).map_err(|_| "文件不是 UTF-8，未更改编码".into())
}
pub fn resolve_codex_skills(prepared: &PreparedInput, catalog: &Value) -> Vec<Value> {
    let known = catalog["data"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|v| v["skills"].as_array().into_iter().flatten())
        .collect::<Vec<_>>();
    prepared
        .codex
        .iter()
        .enumerate()
        .map(|(index, input)| {
            if input["type"] != "skill" {
                return input.clone();
            }
            let selected = input["path"]
                .as_str()
                .and_then(|p| Path::new(p).canonicalize().ok());
            let native = known.iter().find(|s| {
                s["enabled"] != false
                    && selected.is_some()
                    && s["path"]
                        .as_str()
                        .and_then(|p| Path::new(p).canonicalize().ok())
                        == selected
            });
            if let Some(skill) = native {
                json!({"type":"skill","name":skill["name"],"path":skill["path"]})
            } else {
                prepared
                    .claude
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"text","text":""}))
            }
        })
        .collect()
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolServer {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub command: String,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub enabled: bool,
}
pub fn servers(app: &AppHandle) -> Result<Vec<ToolServer>, String> {
    let raw = app
        .state::<AppState>()
        .store
        .setting("tool_servers")?
        .unwrap_or("[]".into());
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

// A warm process only knows the MCP configuration it received at startup.
// Ignore display names and ordering, but replace it when its effective tools change.
pub(crate) fn tool_configuration(app: &AppHandle) -> Result<u64, String> {
    Ok(tool_configuration_key(&servers(app)?))
}
fn tool_configuration_key(servers: &[ToolServer]) -> u64 {
    let mut enabled: Vec<_> = servers.iter().filter(|s| s.enabled).collect();
    enabled.sort_by(|a, b| a.id.cmp(&b.id));
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for server in enabled {
        server.id.hash(&mut hash);
        if let Some(url) = &server.url {
            ("http", url).hash(&mut hash);
        } else {
            ("stdio", &server.command, &server.args).hash(&mut hash);
        }
    }
    hash.finish()
}
fn automation_instructions(servers: &[ToolServer]) -> String {
    let names: Vec<_> = servers.iter().filter(|s| s.enabled && matches!(s.id.as_str(), "browser" | "computer"))
        .map(|s| format!("supercode_{} ({})", s.id, s.name)).collect();
    if names.is_empty() { return String::new(); }
    format!("SuperCode has configured and enabled these managed MCP servers: {}. This describes configuration, not live connection status. Check the tools actually advertised in this session and use those tools for authorized automation. Some MCP tools may be deferred; use the tool discovery/search tool, when advertised, to search for these server names before concluding that their tools are unavailable. If a configured server has no advertised tools, report a connection/loading problem and direct the user to SuperCode's automation settings; do not claim its package is missing or propose installing a second copy without checking installation status. Follow the user's approvals and permissions for every action.", names.join(", "))
}
#[tauri::command]
pub fn list_tool_servers(app: AppHandle) -> Result<Value, String> {
    Ok(
        json!({"servers":servers(&app)?,"node":process::find_program("node"),"uvx":process::find_program("uvx"),"npx":process::find_program("npx")}),
    )
}
#[tauri::command]
pub async fn save_tool_servers(servers: Vec<ToolServer>, app: AppHandle) -> Result<(), String> {
    if app.state::<AppState>().store.running()? > 0 {
        return Err("任务结束后才能更改工具连接".into());
    }
    validate_servers(&servers)?;
    app.state::<AppState>().store.set_setting(
        "tool_servers",
        &serde_json::to_string(&servers).map_err(|e| e.to_string())?,
    )?;
    app.state::<AppState>()
        .runtime
        .clear_for_agent_switch()
        .await;
    Ok(())
}
pub fn validate_servers(servers: &[ToolServer]) -> Result<(), String> {
    if servers.len() > 32 {
        return Err("最多配置 32 个 MCP 连接".into());
    }
    let mut ids = HashSet::new();
    for s in servers {
        if s.id.is_empty()
            || s.id.len() > 64
            || !s
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !ids.insert(s.id.replace('-', "_"))
        {
            return Err("连接 ID 无效或重复".into());
        }
        if s.name.trim().is_empty()
            || s.name.len() > 128
            || s.command.len() > 4096
            || s.args.len() > 64
            || s.args.iter().any(|v| v.len() > 4096)
        {
            return Err("工具名称为空或配置过长".into());
        }
        if let Some(url) = &s.url {
            let parsed = reqwest::Url::parse(url).map_err(|_| "MCP 地址无效")?;
            if !parsed.username().is_empty() || parsed.password().is_some() {
                return Err("MCP 地址不能包含用户名或密码".into());
            }
            let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
            if parsed.scheme() != "https" && !(parsed.scheme() == "http" && local) {
                return Err("HTTP MCP 需要 HTTPS 或本机地址".into());
            }
        } else if s.command.trim().is_empty() {
            return Err("请填写 MCP 程序".into());
        }
        if s.command.contains('\0') || s.args.iter().any(|v| v.contains('\0')) {
            return Err("工具参数无效".into());
        }
    }
    Ok(())
}
pub fn codex_mcp_overrides(app: &AppHandle) -> Result<Vec<String>, String> {
    let mut out = vec![];
    for s in servers(app)?.into_iter().filter(|s| s.enabled) {
        let prefix = format!("mcp_servers.supercode_{}", s.id.replace('-', "_"));
        out.push(format!("{prefix}.enabled=true"));
        out.push(format!("{prefix}.startup_timeout_sec=120"));
        if let Some(url) = s.url {
            out.push(format!("{prefix}.url={}", toml::Value::String(url)));
        } else {
            out.push(format!(
                "{prefix}.command={}",
                toml::Value::String(s.command)
            ));
            out.push(format!(
                "{prefix}.args={}",
                toml::Value::Array(s.args.into_iter().map(toml::Value::String).collect())
            ));
        }
    }
    Ok(out)
}
pub fn claude_mcp(app: &AppHandle, native: Value) -> Result<Value, String> {
    let mut map = native["mcpServers"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for s in servers(app)?.into_iter().filter(|s| s.enabled) {
        map.insert(
            format!("supercode_{}", s.id),
            if let Some(url) = s.url {
                json!({"type":"http","url":url})
            } else {
                json!({"type":"stdio","command":s.command,"args":s.args})
            },
        );
    }
    Ok(json!({"mcpServers":map}))
}
#[tauri::command]
pub async fn tool_server_status(app: AppHandle) -> Result<Value, String> {
    app.state::<AppState>()
        .runtime
        .get(&app)
        .await?
        .request("mcpServerStatus/list", json!({"limit":100}))
        .await
}

fn skill_roots(
    app: &AppHandle,
    project_id: Option<&str>,
) -> Result<Vec<crate::skills::SkillRoot>, String> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from);
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    let project = project_id
        .map(|id| app.state::<AppState>().store.project(id))
        .transpose()?;
    let mut roots = crate::skills::roots(
        &app.path().app_data_dir().map_err(|e| e.to_string())?,
        home.as_deref(),
        codex.as_deref(),
        claude.as_deref(),
        project.as_ref().map(|p| Path::new(&p.path)),
    );
    crate::extensions::filter_skill_roots(app, project_id, &mut roots)?;
    Ok(roots)
}

fn canonical_target(path: &Path) -> Option<PathBuf> {
    if let Ok(path) = path.canonicalize() {
        return Some(path);
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    let mut ancestor = path;
    let mut missing = vec![];
    while !ancestor.exists() {
        missing.push(ancestor.file_name()?.to_owned());
        ancestor = ancestor.parent()?;
    }
    let mut resolved = ancestor.canonicalize().ok()?;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    Some(resolved)
}
pub fn capture_change(app: &AppHandle, session: &crate::storage::Session, params: &Value) {
    let item = &params["item"];
    let Some(path) = item["arguments"]["file_path"].as_str() else {
        return;
    };
    let Ok(project) = app.state::<AppState>().store.session_workspace(&session) else {
        return;
    };
    let root = Path::new(&project.path);
    let path = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    };
    let resolved = canonical_target(&path);
    let Some(path) = resolved else {
        return;
    };
    if !root.canonicalize().is_ok_and(|r| path.starts_with(r)) {
        return;
    }
    let state = app.state::<AppState>();
    let id = path.to_string_lossy().to_string();
    let Ok(mut snapshots) = state.change_snapshots.lock() else {
        return;
    };
    if item["status"] == "completed" {
        let Some((path, before)) = snapshots.get(&id).cloned() else {
            return;
        };
        let Ok(after) = read_utf8(Path::new(&path), 256 * 1024) else {
            return;
        };
        let relative = Path::new(&path)
            .strip_prefix(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()))
            .unwrap_or(Path::new(&path))
            .to_string_lossy()
            .replace('\\', "/");
        let before = before.unwrap_or_default();
        let diff = if before == after {
            String::new()
        } else {
            format!(
                "diff --git a/{relative} b/{relative}\n{}",
                similar::TextDiff::from_lines(&before, &after)
                    .unified_diff()
                    .context_radius(3)
                    .header(&format!("a/{relative}"), &format!("b/{relative}"))
                    .to_string()
            )
        };
        let turn = params["turnId"].as_str().unwrap_or("unknown");
        let data = json!({"turnId":turn,"status":"completed","path":relative});
        let _ = state.store.save_message(
            &format!("change-{turn}:{relative}"),
            &session.id,
            "activity",
            &crate::protocol::bounded(&diff, 128 * 1024),
            "turnDiff",
            &data,
        );
    } else if snapshots.len() < 32 && !snapshots.contains_key(&id) {
        let before = if path.is_file() {
            let Ok(content) = read_utf8(&path, 256 * 1024) else {
                return;
            };
            Some(content)
        } else {
            None
        };
        snapshots.insert(id, (path.to_string_lossy().into(), before));
    }
}
#[tauri::command]
pub async fn list_local_skills(
    project_id: Option<String>,
    app: AppHandle,
) -> Result<Value, String> {
    let roots = skill_roots(&app, project_id.as_deref())?;
    let catalog = tauri::async_runtime::spawn_blocking(move || crate::skills::discover(&roots))
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_value(catalog).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn read_skill(
    path: String,
    project_id: Option<String>,
    app: AppHandle,
) -> Result<String, String> {
    let roots = skill_roots(&app, project_id.as_deref())?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::skills::read_selected(Path::new(&path), &roots)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn create_local_skill(
    name: String,
    description: String,
    instructions: String,
    app: AppHandle,
) -> Result<String, String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || v == b'-' || v == b'_')
    {
        return Err("技能名称只能包含英文、数字、横线和下划线".into());
    }
    if instructions.len() > 128 * 1024 || description.len() > 4096 {
        return Err("技能内容过长".into());
    }
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("skills")
        .join(&name);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let path = root.join("SKILL.md");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| "同名技能已存在".to_owned())?;
    let content = format!(
        "---\nname: {}\ndescription: {}\n---\n\n{}\n",
        name,
        serde_json::to_string(&description).unwrap(),
        instructions
    );
    file.write_all(content.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into())
}
#[tauri::command]
pub async fn agent_extensions(
    action: String,
    params: Value,
    app: AppHandle,
) -> Result<Value, String> {
    let method = match action.as_str() {
        "skills" => "skills/list",
        "plugins" => "plugin/list",
        "install-plugin" => "plugin/install",
        "uninstall-plugin" => "plugin/uninstall",
        "skill-state" => "skills/config/write",
        _ => return Err("扩展操作无效".into()),
    };
    if ["install-plugin", "uninstall-plugin", "skill-state"].contains(&action.as_str())
        && app.state::<AppState>().store.running()? > 0
    {
        return Err("任务结束后才能更改扩展".into());
    }
    app.state::<AppState>()
        .runtime
        .get(&app)
        .await?
        .request(method, params)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_steering_cannot_silently_change_active_permissions_or_reasoning() {
        for signature in [
            json!(["route", 42, "session", "cwd", "model", "read", null]),
            json!(["route", 42, "session", "cwd", "model", true, "read", null]),
        ] {
            let s = signature.to_string();
            assert!(validate_steering_settings(&s, "read", None).is_ok());
            assert!(validate_steering_settings(&s, "full", None).is_err());
            assert!(validate_steering_settings(&s, "read", Some("high")).is_err());
        }
        assert!(validate_steering_settings("[]", "read", None).is_err());
    }
    #[test]
    fn enabling_installing_or_reconfiguring_tools_invalidates_warm_connections() {
        let mut computer = ToolServer { id: "computer".into(), name: "电脑自动化".into(), kind: "computer".into(), command: "C:/tools/windows-mcp.exe".into(), args: vec!["serve".into()], url: None, enabled: true };
        let empty = tool_configuration_key(&[]);
        let installed = tool_configuration_key(&[computer.clone()]);
        assert_ne!(empty, installed);
        computer.enabled = false;
        assert_eq!(empty, tool_configuration_key(&[computer.clone()]));
        computer.enabled = true;
        computer.command = "C:/tools/new/windows-mcp.exe".into();
        assert_ne!(installed, tool_configuration_key(&[computer.clone()]));
        let before_args = tool_configuration_key(&[computer.clone()]);
        computer.args.push("--flag".into());
        assert_ne!(before_args, tool_configuration_key(&[computer.clone()]));
        computer.url = Some("https://example.com/mcp".into());
        let before_url = tool_configuration_key(&[computer.clone()]);
        computer.url = Some("https://example.com/new-mcp".into());
        assert_ne!(before_url, tool_configuration_key(&[computer]));
    }
    #[test]
    fn tool_display_changes_do_not_restart_connections() {
        let mut computer = ToolServer { id: "computer".into(), name: "电脑".into(), kind: "computer".into(), command: "windows-mcp.exe".into(), args: vec!["serve".into()], url: None, enabled: true };
        let browser = ToolServer { id: "browser".into(), command: "node.exe".into(), ..computer.clone() };
        let original = tool_configuration_key(&[computer.clone(), browser.clone()]);
        computer.name = "Windows MCP".into();
        assert_eq!(original, tool_configuration_key(&[browser, computer]));
    }
    #[test]
    fn managed_tools_context_distinguishes_configuration_from_live_connection() {
        let mut computer = ToolServer { id: "computer".into(), name: "电脑 · Windows MCP".into(), kind: "computer".into(), command: "windows-mcp.exe".into(), args: vec!["serve".into()], url: None, enabled: true };
        let context = automation_instructions(&[computer.clone()]);
        assert!(context.contains("supercode_computer"));
        assert!(context.contains("not live connection status"));
        assert!(context.contains("tool discovery/search"));
        computer.enabled = false;
        assert!(automation_instructions(&[computer]).is_empty());
    }
    #[test]
    fn native_full_access_and_read_modes_are_distinct() {
        assert_eq!(permission("full").unwrap().2["type"], "dangerFullAccess");
        assert_eq!(permission("full").unwrap().1, "never");
        assert_eq!(permission("read").unwrap().2["type"], "readOnly");
        assert!(permission("bypass").is_err());
    }
    #[test]
    fn codex_automatic_review_requires_an_explicit_mode_and_keeps_network_sandboxed() {
        for mode in ["ask", "strict", "auto"] {
            let (sandbox, approval, policy) = permission(mode).unwrap();
            assert_eq!(sandbox, "workspace-write");
            assert_eq!(approval, "on-request");
            assert_eq!(policy["networkAccess"], false);
            assert_eq!(
                approvals_reviewer(mode),
                if mode == "auto" {
                    "auto_review"
                } else {
                    "user"
                }
            );
        }
        assert_eq!(approvals_reviewer("read"), "user");
        assert!(permission("edit").is_err());
    }
    #[test]
    fn agents_reject_unsupported_modes_and_claude_uses_native_permission_names() {
        for (mode, native) in [
            ("read", "plan"),
            ("ask", "manual"),
            ("strict", "manual"),
            ("edit", "acceptEdits"),
            ("auto", "auto"),
            ("deny", "dontAsk"),
            ("full", "bypassPermissions"),
        ] {
            assert_eq!(claude_native_permission(mode).unwrap(), native);
        }
        for agent in ["opencode", "pi"] {
            for mode in ["read", "ask", "strict", "full"] {
                assert!(validate_permission(agent, mode).is_ok());
            }
            for mode in ["auto", "edit", "deny"] {
                assert!(validate_permission(agent, mode).is_err());
            }
        }
        assert!(validate_permission("codex", "auto").is_ok());
        assert!(validate_permission("codex", "deny").is_err());
        assert!(validate_permission("unknown", "full").is_err());
        assert!(claude_native_permission("invalid").is_err());
    }
    #[test]
    fn attachments_reject_bad_types_and_bom() {
        let path = std::env::temp_dir().join(format!("supercode-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, [239, 187, 191, 65]).unwrap();
        assert!(prepare_input(
            "x",
            &[Attachment {
                kind: "file".into(),
                name: "test".into(),
                path: path.to_string_lossy().into()
            }]
        )
        .unwrap_err()
        .contains("BOM"));
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn mcp_config_rejects_duplicate_names_and_remote_http() {
        let s = ToolServer {
            id: "a".into(),
            name: "A".into(),
            kind: "custom".into(),
            command: "node".into(),
            args: vec!["literal $(input)".into()],
            url: None,
            enabled: true,
        };
        assert!(validate_servers(&[s.clone()]).is_ok());
        assert!(validate_servers(&[s.clone(), s.clone()]).is_err());
        assert!(validate_servers(&[ToolServer {
            url: Some("http://evil.test".into()),
            ..s
        }])
        .is_err());
    }
    #[test]
    fn attachment_content_and_images_reach_native_inputs() {
        let root = std::env::temp_dir().join(format!("supercode-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("中文.txt");
        std::fs::write(&file, "中文 $(echo unchanged)\n").unwrap();
        let image = root.join("a.png");
        std::fs::write(&image, b"\x89PNG\r\n\x1a\nfixture").unwrap();
        let skill = root.join("SKILL.md");
        std::fs::write(&skill, "---\nname: test\n---\nReply TEST_SKILL").unwrap();
        let inputs = prepare_input(
            "hello",
            &[
                Attachment {
                    kind: "file".into(),
                    path: file.to_string_lossy().into(),
                    name: "中文.txt".into(),
                },
                Attachment {
                    kind: "image".into(),
                    path: image.to_string_lossy().into(),
                    name: "a.png".into(),
                },
                Attachment {
                    kind: "skill".into(),
                    path: skill.to_string_lossy().into(),
                    name: "test".into(),
                },
            ],
        )
        .unwrap();
        assert!(inputs.codex[1]["text"]
            .as_str()
            .unwrap()
            .contains("$(echo unchanged)"));
        assert_eq!(inputs.codex[2]["type"], "localImage");
        assert_eq!(inputs.codex[3]["type"], "skill");
        assert_eq!(inputs.claude[2]["source"]["media_type"], "image/png");
        assert!(inputs.claude[3]["text"]
            .as_str()
            .unwrap()
            .contains("TEST_SKILL"));
        assert_eq!(inputs.metadata["attachments"].as_array().unwrap().len(), 3);
        let fallback = resolve_codex_skills(&inputs, &Value::Null);
        assert!(fallback[3]["text"].as_str().unwrap().contains("TEST_SKILL"));
        let native = resolve_codex_skills(
            &inputs,
            &json!({"data":[{"skills":[{"path":skill.to_string_lossy(),"name":"test","enabled":true}]}]}),
        );
        assert_eq!(native[3]["type"], "skill");
        assert_eq!(native[3]["path"], skill.to_string_lossy().as_ref());
        assert!(canonical_target(&root.join("nested/new/file.ts"))
            .unwrap()
            .starts_with(root.canonicalize().unwrap()));
        assert!(canonical_target(&root.join("missing/../outside.ts")).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}

// Stable IDs keep retried follow-ups from duplicating the user message history.
pub fn user_message_id(metadata: &Value) -> String {
    format!(
        "user-{}",
        metadata["followupId"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
    )
}
