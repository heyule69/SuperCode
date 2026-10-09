//! Native official logins, isolated per account. Tokens never cross Tauri IPC.
use crate::{accounts, ccswitch::Profile, process, AppState};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::{process::Child, sync::Mutex, task::JoinHandle};

#[derive(Default)]
pub struct Logins(Mutex<BTreeMap<String, Login>>);
impl Logins {
    pub(crate) async fn shutdown(&self, app: &AppHandle) {
        let pending = std::mem::take(&mut *self.0.lock().await);
        for (id, mut login) in pending {
            let _ = login.child.kill().await;
            let dir = directory(app, &login.agent, &id);
            drop(login);
            if let Ok(dir) = dir {
                let _ = tokio::fs::remove_dir_all(dir).await;
            }
        }
    }
}
struct Login {
    agent: String,
    name: String,
    child: Child,
    _input: Option<tokio::process::ChildStdin>,
    _job: process::JobGuard,
    reader: Option<JoinHandle<()>>,
}
struct PendingDirectory {
    path: PathBuf,
    keep: bool,
}
impl Drop for PendingDirectory {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
impl Drop for Login {
    fn drop(&mut self) {
        if let Some(reader) = &self.reader {
            reader.abort();
        }
        let _ = self.child.start_kill();
    }
}

pub(crate) fn account_directory(base: &Path, agent: &str, id: &str) -> Result<PathBuf, String> {
    if !matches!(agent, "claude" | "codex") {
        return Err("此 Agent 不支持官方账号管理".into());
    }
    let uuid = uuid::Uuid::parse_str(id).map_err(|_| "账号标识无效")?;
    if uuid.to_string() != id {
        return Err("账号标识无效".into());
    }
    Ok(crate::agents::ordinary(
        base.join("official-accounts").join(agent).join(id),
    ))
}
pub(crate) fn directory(app: &AppHandle, agent: &str, id: &str) -> Result<PathBuf, String> {
    account_directory(&app.state::<AppState>().data_dir, agent, id)
}
pub(crate) fn configure(
    command: &mut tokio::process::Command,
    app: &AppHandle,
    agent: &str,
    config: &Value,
) -> Result<(), String> {
    configure_in(command, &app.state::<AppState>().data_dir, agent, config)
}
fn configure_in(
    command: &mut tokio::process::Command,
    base: &Path,
    agent: &str,
    config: &Value,
) -> Result<(), String> {
    if let Some(id) = config["accountId"].as_str() {
        if config["official"] != true {
            return Err("账号只能绑定官方登录连接".into());
        }
        let dir = account_directory(base, agent, id)?;
        command.env(
            if agent == "codex" {
                "CODEX_HOME"
            } else {
                "CLAUDE_CONFIG_DIR"
            },
            dir,
        );
        if agent == "codex" {
            command.args(["-c", "cli_auth_credentials_store=\"file\""]);
        }
    }
    Ok(())
}
fn profile(agent: &str, id: &str, name: &str) -> Profile {
    Profile {
        id: format!("account:{id}"),
        agent: agent.into(),
        name: name.into(),
        config: json!({
            "official":true,"accountId":id,"source":"official","providerId":if agent == "claude" {"anthropic"} else {"openai"},
            "protocol":if agent == "claude" {"anthropic"} else {"responses"},"env":{},"config":"model_provider = 'openai'\n"
        }),
    }
}
fn login_url(url: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(url).map_err(|_| "官方登录地址无效")?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("auth.openai.com" | "auth0.openai.com" | "chatgpt.com")
        )
    {
        return Err("Codex 返回了未知登录域名，未打开浏览器".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn list_official_accounts(
    agent: String,
    force: Option<bool>,
    app: AppHandle,
) -> Result<Vec<Value>, String> {
    account_directory(Path::new("."), &agent, &uuid::Uuid::nil().to_string())?;
    let state = app.state::<AppState>();
    let profiles = state.store.profiles()?;
    let default = state.store.default_connection(&agent)?;
    let mut rows = Vec::new();
    let candidates = std::iter::once((
        crate::session_config::OFFICIAL.to_owned(),
        None,
        "本机账号".to_owned(),
    ))
    .chain(
        profiles
            .into_iter()
            .filter(|p| p.agent == agent && p.is_official() && p.account_id().is_some())
            .map(|p| {
                (
                    p.id,
                    p.config["accountId"].as_str().map(str::to_owned),
                    p.name,
                )
            }),
    );
    for (connection, id, name) in candidates {
        let mut status = match app
            .state::<accounts::Accounts>()
            .get_for(&agent, id.as_deref(), force.unwrap_or(false), &app)
            .await
        {
            Ok(value) => value,
            Err(error) => json!({"loggedIn":false,"error":error}),
        };
        status["id"] = connection.clone().into();
        status["agent"] = agent.clone().into();
        status["accountId"] = json!(id);
        status["name"] = name.into();
        status["current"] = (connection == default).into();
        rows.push(status);
    }
    Ok(rows)
}

#[tauri::command]
pub async fn start_official_account_login(
    agent: String,
    name: String,
    app: AppHandle,
) -> Result<Value, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 100 || name.chars().any(char::is_control) {
        return Err("请填写账号名称，长度不超过 100 个 UTF-8 字节".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let dir = directory(&app, &agent, &id)?;
    let login_state = app.state::<Logins>();
    let mut logins = login_state.0.lock().await;
    if logins.values().any(|login| login.agent == agent) {
        return Err("请先完成或取消当前账号的登录".into());
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|_| "无法创建独立账号目录")?;
    let mut pending_directory = PendingDirectory {
        path: dir,
        keep: false,
    };
    let config = json!({"official":true,"accountId":id});
    let (mut child, auth_url) = if agent == "codex" {
        let mut command = accounts::codex_command(&app, Some(&id))?;
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "无法启动 Codex 官方登录")?;
        (child, true)
    } else {
        let mut command = accounts::claude_command(&app, &["auth", "login", "--claudeai"])?;
        configure(&mut command, &app, &agent, &config)?;
        (
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| "无法启动 Claude 官方登录")?,
            false,
        )
    };
    let job = process::JobGuard::attach(&child)?;
    let mut reader = None;
    let mut login_input = None;
    if auth_url {
        let mut input = child.stdin.take().ok_or("无法连接登录进程")?;
        let mut output = tokio::io::BufReader::new(child.stdout.take().ok_or("无法读取登录进程")?);
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            accounts::read_account_method(
                &mut input,
                &mut output,
                "account/login/start",
                json!({"type":"chatgpt"}),
            ),
        )
        .await
        .map_err(|_| "启动官方登录超时")??;
        let url = result["authUrl"].as_str().ok_or("Codex 未返回登录地址")?;
        login_url(url)?;
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|_| "无法打开登录浏览器")?;
        // Drain native notifications without forwarding any credential data.
        reader = Some(tokio::spawn(async move {
            while matches!(crate::runtime::read_frame(&mut output).await, Ok(Some(_))) {}
        }));
        login_input = Some(input);
    }
    logins.insert(
        id.clone(),
        Login {
            agent,
            name: name.into(),
            child,
            _input: login_input,
            _job: job,
            reader,
        },
    );
    pending_directory.keep = true;
    Ok(json!({"accountId":id,"message":"请在浏览器登录所需账号，完成后点击检查登录。"}))
}

#[tauri::command]
pub async fn finish_official_account_login(
    account_id: String,
    app: AppHandle,
) -> Result<Value, String> {
    let login_state = app.state::<Logins>();
    let mut logins = login_state.0.lock().await;
    let login = logins
        .get(&account_id)
        .ok_or("登录已结束，请重新添加账号")?;
    let mut status = app
        .state::<accounts::Accounts>()
        .get_for(&login.agent, Some(&account_id), true, &app)
        .await?;
    if status["loggedIn"] != true {
        return Ok(status);
    }
    let mut profile = profile(&login.agent, &account_id, &login.name);
    if login.agent == "claude" {
        profile.config["model"] = "sonnet".into();
        profile.config["models"] = json!(["sonnet", "opus", "haiku"]);
    } else if let Ok(catalog) =
        accounts::read_codex_value(&app, Some(&account_id), "model/list", json!({"limit":100}))
            .await
    {
        let models: Vec<_> = catalog["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|model| model["hidden"] != true)
            .filter_map(|model| model["model"].as_str())
            .take(100)
            .collect();
        let default = catalog["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|model| model["isDefault"] == true)
            .and_then(|model| model["model"].as_str())
            .or_else(|| models.first().copied());
        profile.config["model"] = json!(default);
        profile.config["models"] = json!(models);
    }
    let store = &app.state::<AppState>().store;
    store.import_profiles(std::slice::from_ref(&profile))?;
    // A first successful login replaces an unavailable fallback. Additional
    // accounts leave an already usable default, and all pinned chats, intact.
    if matches!(crate::chat_connection::check(&app, &login.agent, None, None).await, Err(error) if error == crate::chat_connection::PROVIDER_REQUIRED)
    {
        store.select_profile(&login.agent, Some(&profile.id))?;
    }
    status["isDefault"] = (store.default_connection(&login.agent)? == profile.id).into();
    logins.remove(&account_id);
    let _ = app.emit("workspace-updated", ());
    Ok(status)
}

#[tauri::command]
pub async fn cancel_official_account_login(
    account_id: String,
    app: AppHandle,
) -> Result<(), String> {
    let login = app.state::<Logins>().0.lock().await.remove(&account_id);
    if let Some(mut login) = login {
        let _ = login.child.kill().await;
        let dir = directory(&app, &login.agent, &account_id)?;
        drop(login);
        tokio::fs::remove_dir_all(dir)
            .await
            .map_err(|_| "无法移除未完成的账号登录")?;
    }
    Ok(())
}

#[tauri::command]
pub async fn rename_official_account(
    agent: String,
    account_id: String,
    name: String,
    app: AppHandle,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 100 || name.chars().any(char::is_control) {
        return Err("账号名称无效".into());
    }
    let store = &app.state::<AppState>().store;
    let mut profile = store
        .profile(&agent, &format!("account:{account_id}"))?
        .ok_or("账号不存在")?;
    profile.name = name.into();
    store.import_profiles(&[profile])?;
    let _ = app.emit("workspace-updated", ());
    Ok(())
}

#[tauri::command]
pub async fn remove_official_account(
    agent: String,
    account_id: String,
    app: AppHandle,
) -> Result<(), String> {
    let dir = directory(&app, &agent, &account_id)?;
    let state = app.state::<AppState>();
    let id = format!("account:{account_id}");
    state.store.profile(&agent, &id)?.ok_or("账号不存在")?;
    state.runtime.release(&app).await?;
    state.store.delete_profile(&id)?; // Refuse removal while any chat still uses it.
    if dir.exists() {
        tokio::fs::remove_dir_all(dir)
            .await
            .map_err(|_| "连接已移除，但无法清理账号目录")?;
    }
    let _ = app.emit("workspace-updated", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_paths_are_separate_and_cannot_escape_the_app_directory() {
        let root = Path::new("D:/SuperCode");
        let a = uuid::Uuid::new_v4().to_string();
        let b = uuid::Uuid::new_v4().to_string();
        assert_ne!(
            account_directory(root, "codex", &a).unwrap(),
            account_directory(root, "codex", &b).unwrap()
        );
        assert_ne!(
            account_directory(root, "codex", &a).unwrap(),
            account_directory(root, "claude", &a).unwrap()
        );
        for id in [
            "../auth",
            "D:/elsewhere",
            "",
            "00000000-0000-0000-0000-000000000000/../",
        ] {
            assert!(account_directory(root, "codex", id).is_err());
        }
        assert!(account_directory(root, "pi", &a).is_err());
    }
    #[test]
    fn managed_logins_are_distinct_profiles_with_no_key_or_token_in_summaries() {
        let a = profile("codex", &uuid::Uuid::new_v4().to_string(), "个人账号");
        let b = profile("codex", &uuid::Uuid::new_v4().to_string(), "工作账号");
        assert!(a.is_official());
        assert_ne!(a.id, b.id);
        assert_ne!(
            crate::session_config::routing_identity(&a),
            crate::session_config::routing_identity(&b)
        );
        assert!(a.summary(false).account_id.is_some());
        assert!(!a.summary(false).has_credential);
        assert!(login_url("https://chatgpt.com/auth").is_ok());
        assert!(login_url("https://chatgpt.com.example.test/auth").is_err());
    }
    #[test]
    fn native_commands_use_account_specific_credential_stores() {
        let id = uuid::Uuid::new_v4().to_string();
        let root = Path::new("D:/SuperCode");
        for (agent, variable) in [("codex", "CODEX_HOME"), ("claude", "CLAUDE_CONFIG_DIR")] {
            let mut command = tokio::process::Command::new("unused-test-program");
            configure_in(
                &mut command,
                root,
                agent,
                &json!({"official":true,"accountId":id}),
            )
            .unwrap();
            let expected = account_directory(root, agent, &id).unwrap();
            assert!(command
                .as_std()
                .get_envs()
                .any(|(key, value)| key == variable && value == Some(expected.as_os_str())));
            if agent == "codex" {
                assert!(command
                    .as_std()
                    .get_args()
                    .any(|arg| arg == "cli_auth_credentials_store=\"file\""));
            }
            assert!(configure_in(&mut command, root, agent, &json!({"accountId":id})).is_err());
        }
    }
    #[test]
    fn two_official_accounts_keep_distinct_defaults_chat_bindings_and_reopen_correctly() {
        for agent in ["codex", "claude"] {
            let path =
                std::env::temp_dir().join(format!("sc-account-test-{}.db", uuid::Uuid::new_v4()));
            let a = profile(agent, &uuid::Uuid::new_v4().to_string(), "个人账号");
            let b = profile(agent, &uuid::Uuid::new_v4().to_string(), "工作账号");
            let (chat_a, chat_b) = {
                let store = crate::storage::Store::open(&path).unwrap();
                store.import_profiles(&[a.clone(), b.clone()]).unwrap();
                assert!(store.connection_orders().unwrap()[agent].contains(&a.id));
                assert!(store.connection_orders().unwrap()[agent].contains(&b.id));
                let project = store
                    .add_project(Path::new("D:/account-binding-test"))
                    .unwrap();
                store.select_profile(agent, Some(&a.id)).unwrap();
                let chat_a = store
                    .create_agent_session(&project.id, None, agent)
                    .unwrap();
                store.select_profile(agent, Some(&b.id)).unwrap();
                let chat_b = store
                    .create_agent_session(&project.id, None, agent)
                    .unwrap();
                assert!(store.delete_profile(&a.id).is_err());
                (chat_a.id, chat_b.id)
            };
            let store = crate::storage::Store::open(&path).unwrap();
            assert_eq!(
                store.route(agent, Some(&chat_a)).unwrap().config["accountId"],
                a.config["accountId"]
            );
            assert_eq!(
                store.route(agent, Some(&chat_b)).unwrap().config["accountId"],
                b.config["accountId"]
            );
            assert_eq!(store.default_connection(agent).unwrap(), b.id);
            drop(store);
            let _ = std::fs::remove_file(path);
        }
    }
}
