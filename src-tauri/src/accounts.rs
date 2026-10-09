//! Official sign-in remains owned by the installed agent. SuperCode never receives OAuth tokens.
use crate::{process, AppState};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct Accounts {
    cache: Mutex<BTreeMap<String, (Instant, Value)>>,
    checks: [Mutex<()>; 2],
}
impl Accounts {
    pub(crate) async fn get(
        &self,
        agent: &str,
        force: bool,
        app: &AppHandle,
    ) -> Result<Value, String> {
        let index = match agent {
            "codex" => 0,
            "claude" => 1,
            _ => return Err("不支持此 Agent 的官方账号".into()),
        };
        let _check = self.checks[index].lock().await;
        if !force {
            if let Some((at, value)) = self.cache.lock().await.get(agent) {
                if at.elapsed() < Duration::from_secs(60) {
                    return Ok(value.clone());
                }
            }
        }
        let value = read_official_status(agent, app).await?;
        self.cache
            .lock()
            .await
            .insert(agent.into(), (Instant::now(), value.clone()));
        Ok(value)
    }
}

#[tauri::command]
pub async fn list_provider_accounts(
    force: Option<bool>,
    app: AppHandle,
) -> Result<Vec<Value>, String> {
    async fn one(agent: &str, force: bool, app: &AppHandle) -> Value {
        if crate::agents::resolve(app, agent).is_err() {
            return json!({"agent":agent,"loggedIn":false});
        }
        match app.state::<Accounts>().get(agent, force, app).await {
            Ok(mut value) => {
                value["agent"] = agent.into();
                value
            }
            Err(error) => json!({"agent":agent,"loggedIn":false,"error":error}),
        }
    }
    let (codex, claude) = tokio::join!(
        one("codex", force.unwrap_or(false), &app),
        one("claude", force.unwrap_or(false), &app)
    );
    Ok(vec![codex, claude])
}

/// A short-lived account-only connection. It never starts/resumes a thread, loads
/// SuperCode tools, or changes the main runtime's route, epoch or session state.
pub(crate) async fn read_codex_limits(app: &AppHandle) -> Result<Value, String> {
    read_codex_value(app, "account/rateLimits/read", json!({})).await
}
async fn read_codex_value(app: &AppHandle, method: &str, params: Value) -> Result<Value, String> {
    let launch = crate::agents::resolve(app, "codex")?;
    let mut command = launch.command(&["app-server", "--listen", "stdio://"]);
    for setting in process::codex_config_overrides(false)? {
        command.arg("-c").arg(setting);
    }
    command.args(["-c", "model_provider=\"openai\""]);
    // Official allowance belongs to the CLI's saved ChatGPT login, not an API key.
    for variable in [
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "SUPERCODE_PROVIDER_API_KEY",
    ] {
        command.env_remove(variable);
    }
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "无法启动 Codex 额度查询")?;
    let _job = process::JobGuard::attach(&child)?;
    let input = child.stdin.take().ok_or("无法连接 Codex stdin")?;
    let output = child.stdout.take().ok_or("无法连接 Codex stdout")?;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        read_account_method(input, tokio::io::BufReader::new(output), method, params),
    )
    .await
    .map_err(|_| "查询 Codex 官方额度超时".to_owned())
    .and_then(|value| value);
    // kill_on_drop + JobGuard also clean up if the IPC future is cancelled.
    let _ = child.kill().await;
    result
}

#[cfg(test)]
async fn read_account_limits<W, R>(mut input: W, mut output: R) -> Result<Value, String>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncBufRead + Unpin,
{
    read_account_method(
        &mut input,
        &mut output,
        "account/rateLimits/read",
        json!({}),
    )
    .await
}
async fn read_account_method<W, R>(
    mut input: W,
    mut output: R,
    method: &str,
    params: Value,
) -> Result<Value, String>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncBufRead + Unpin,
{
    account_request(
        &mut input,
        &mut output,
        1,
        "initialize",
        json!({"clientInfo":{"name":"supercode_quota","title":"SuperCode","version":"0.1.0"},"capabilities":{"experimentalApi":true}}),
    )
    .await?;
    write_account_frame(&mut input, json!({"method":"initialized","params":{}})).await?;
    account_request(&mut input, &mut output, 2, method, params).await
}

async fn write_account_frame<W: tokio::io::AsyncWrite + Unpin>(
    input: &mut W,
    message: Value,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut data = serde_json::to_vec(&message).map_err(|_| "Codex 查询参数无效")?;
    data.push(b'\n');
    input
        .write_all(&data)
        .await
        .map_err(|_| "Codex 额度连接已关闭")?;
    input
        .flush()
        .await
        .map_err(|_| "Codex 额度连接已关闭".into())
}

async fn account_request<W, R>(
    input: &mut W,
    output: &mut R,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncBufRead + Unpin,
{
    write_account_frame(input, json!({"id":id,"method":method,"params":params})).await?;
    loop {
        let frame = crate::runtime::read_frame(output)
            .await?
            .ok_or("Codex 额度连接已关闭")?;
        let value: Value = serde_json::from_slice(&frame).map_err(|_| "Codex 额度数据无效")?;
        if value.get("method").is_some() {
            if let Some(request_id) = value.get("id") {
                // Account reads do not authorize tool/approval requests.
                write_account_frame(input, json!({"id":request_id,"error":{"code":-32601,"message":"Account-only client"}})).await?;
            }
            continue;
        }
        if value["id"].as_u64() == Some(id) {
            if value.get("error").is_some() {
                return Err("Codex 未能读取官方额度，请检查本机官方登录".into());
            }
            return value
                .get("result")
                .cloned()
                .ok_or("Codex 未返回额度数据".into());
        }
    }
}
fn claude_command(app: &AppHandle, args: &[&str]) -> Result<tokio::process::Command, String> {
    let launch = crate::agents::resolve(app, "claude")?;
    let mut c = launch.command(&["--setting-sources", ""]);
    c.args(args);
    for name in [
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
    ] {
        c.env_remove(name);
    }
    Ok(c)
}
#[tauri::command]
pub async fn official_account_status(agent: String, app: AppHandle) -> Result<Value, String> {
    app.state::<Accounts>().get(&agent, true, &app).await
}
fn codex_status(value: &Value) -> Value {
    let method = value["account"]["type"].as_str();
    json!({"loggedIn":matches!(method, Some("chatgpt" | "chatgptAuthTokens")),"method":method,"plan":value["account"]["planType"]})
}
fn claude_status(value: &Value) -> Value {
    json!({"loggedIn":value["loggedIn"] == true && matches!(value["authMethod"].as_str(), Some("claude.ai" | "oauth_token")),"method":value["authMethod"],"plan":value["subscriptionType"]})
}
async fn read_official_status(agent: &str, app: &AppHandle) -> Result<Value, String> {
    if agent == "codex" {
        let value = read_codex_value(app, "account/read", json!({"refreshToken":false})).await?;
        return Ok(codex_status(&value));
    }
    if agent != "claude" {
        return Err("不支持此 Agent 的账号".into());
    }
    use tokio::io::AsyncReadExt;
    let mut child = claude_command(app, &["auth", "status", "--json"])?
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "无法检查 Claude 登录状态")?;
    let _job = process::JobGuard::attach(&child)?;
    let stdout = child.stdout.take().ok_or("Claude 未返回账号数据")?;
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let mut bytes = Vec::new();
        stdout
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "无法读取 Claude 账号数据")?;
        if bytes.len() > 16 * 1024 {
            return Err("Claude 账号数据超过大小限制");
        }
        child.wait().await.map_err(|_| "无法检查 Claude 登录状态")?;
        Ok(bytes)
    })
    .await
    .map_err(|_| "检查 Claude 登录状态超时")?
    .map_err(str::to_owned)?;
    let value: Value = serde_json::from_slice(&output)
        .map_err(|_| "当前 Claude CLI 未提供 JSON 登录状态，请升级 CLI")?;
    Ok(claude_status(&value))
}
#[tauri::command]
pub async fn use_official_account(agent: String, app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.runtime.release(&app).await?;
    state.store.use_official(&agent)?;
    let _ = app.emit("workspace-updated", ());
    Ok(())
}
#[tauri::command]
pub async fn start_official_login(agent: String, app: AppHandle) -> Result<Value, String> {
    use_official_account(agent.clone(), app.clone()).await?;
    if agent == "codex" {
        let value = app
            .state::<AppState>()
            .runtime
            .get(&app)
            .await?
            .request("account/login/start", json!({"type":"chatgpt"}))
            .await?;
        let url = value["authUrl"]
            .as_str()
            .ok_or("Codex 未返回官方登录地址")?;
        let parsed = reqwest::Url::parse(url).map_err(|_| "官方登录地址无效")?;
        if parsed.scheme() != "https"
            || !matches!(
                parsed.host_str(),
                Some("auth.openai.com" | "auth0.openai.com" | "chatgpt.com")
            )
        {
            return Err("Codex 返回了未知登录域名，未打开浏览器".into());
        }
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|_| "无法打开浏览器")?;
        return Ok(
            json!({"started":true,"loginId":value["loginId"],"message":"请在浏览器完成 ChatGPT 官方登录，完成后点击检查状态。"}),
        );
    }
    let state = app.state::<AppState>();
    let mut slot = state.claude_login.lock().await;
    if slot
        .as_mut()
        .is_some_and(|c| c.try_wait().ok().flatten().is_none())
    {
        return Err("Claude 登录已经开始，请完成浏览器登录或取消后重试".into());
    }
    let child = claude_command(&app, &["auth", "login", "--claudeai"])?
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "无法启动 Claude 官方登录")?;
    *slot = Some(child);
    Ok(
        json!({"started":true,"message":"已启动本机 Claude 官方登录。请在浏览器完成登录，再点击检查状态。"}),
    )
}
#[tauri::command]
pub async fn cancel_official_login(
    agent: String,
    login_id: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    if agent == "claude" {
        if let Some(mut c) = app.state::<AppState>().claude_login.lock().await.take() {
            let _ = c.kill().await;
        }
        return Ok(());
    }
    if agent == "codex" {
        if let Some(id) = login_id {
            app.state::<AppState>()
                .runtime
                .get(&app)
                .await?
                .request("account/login/cancel", json!({"loginId":id}))
                .await?;
        }
        return Ok(());
    }
    Err("Agent 无效".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, BufReader};

    #[test]
    fn only_subscription_accounts_are_listed_and_credentials_are_not_returned() {
        assert_eq!(codex_status(&json!({"account":null}))["loggedIn"], false);
        assert_eq!(
            codex_status(&json!({"account":{"type":"apiKey"}}))["loggedIn"],
            false
        );
        let account = codex_status(
            &json!({"account":{"type":"chatgpt","planType":"plus","email":"private","accessToken":"secret"}}),
        );
        assert_eq!(account["loggedIn"], true);
        assert!(!account.to_string().contains("private"));
        assert!(!account.to_string().contains("secret"));
        assert_eq!(
            claude_status(&json!({"loggedIn":true,"authMethod":"api_key"}))["loggedIn"],
            false
        );
        assert_eq!(
            claude_status(&json!({"loggedIn":true,"authMethod":"claude.ai"}))["loggedIn"],
            true
        );
        assert_eq!(
            claude_status(&json!({"loggedIn":true,"authMethod":"oauth_token"}))["loggedIn"],
            true
        );
        assert_eq!(
            claude_status(&json!({"loggedIn":false,"authMethod":"claude.ai"}))["loggedIn"],
            false
        );
    }

    #[tokio::test]
    async fn account_presence_uses_read_only_rpc_without_a_thread_or_refresh() {
        let (client, server) = tokio::io::duplex(8192);
        let (output, input) = tokio::io::split(client);
        let task = tokio::spawn(read_account_method(
            input,
            BufReader::new(output),
            "account/read",
            json!({"refreshToken":false}),
        ));
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = BufReader::new(reader);
        let init: Value = serde_json::from_slice(
            &crate::runtime::read_frame(&mut reader)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(init["method"], "initialize");
        write_account_frame(&mut writer, json!({"id":1,"result":{}}))
            .await
            .unwrap();
        let initialized: Value = serde_json::from_slice(
            &crate::runtime::read_frame(&mut reader)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(initialized["method"], "initialized");
        let request: Value = serde_json::from_slice(
            &crate::runtime::read_frame(&mut reader)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(request["method"], "account/read");
        assert_eq!(request["params"]["refreshToken"], false);
        write_account_frame(&mut writer, json!({"id":2,"result":{"account":null}}))
            .await
            .unwrap();
        assert_eq!(task.await.unwrap().unwrap()["account"], Value::Null);
        assert!(crate::runtime::read_frame(&mut reader)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn quota_protocol_reads_account_without_a_turn_or_approving_requests() {
        let (client, server) = tokio::io::duplex(8192);
        let (output, input) = tokio::io::split(client);
        let task = tokio::spawn(read_account_limits(input, BufReader::new(output)));
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = BufReader::new(reader);
        let init: Value = serde_json::from_slice(
            &crate::runtime::read_frame(&mut reader)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(init["method"], "initialize");
        assert!(init["params"].get("auth").is_none());
        writer
            .write_all(
                b"{\"method\":\"account/updated\",\"params\":{}}\n{\"id\":1,\"result\":{}}\n",
            )
            .await
            .unwrap();
        let initialized: Value = serde_json::from_slice(
            &crate::runtime::read_frame(&mut reader)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(initialized["method"], "initialized");
        let request: Value = serde_json::from_slice(
            &crate::runtime::read_frame(&mut reader)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(request["method"], "account/rateLimits/read");
        writer.write_all(b"{\"method\":\"item/commandExecution/requestApproval\",\"id\":\"approval\",\"params\":{}}\n").await.unwrap();
        let rejection: Value = serde_json::from_slice(
            &crate::runtime::read_frame(&mut reader)
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(rejection["id"], "approval");
        assert_eq!(rejection["error"]["code"], -32601);
        assert!(rejection.get("result").is_none());
        let limits = json!({"rateLimits":{"planType":"plus","primary":{"usedPercent":0,"windowDurationMins":300}}});
        write_account_frame(&mut writer, json!({"id":2,"result":limits}))
            .await
            .unwrap();
        assert_eq!(task.await.unwrap().unwrap(), limits);
        assert!(crate::runtime::read_frame(&mut reader)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn quota_protocol_fails_on_disconnect_or_rpc_error() {
        let mut input = Vec::new();
        let mut output = BufReader::new(&b""[..]);
        assert!(account_request(
            &mut input,
            &mut output,
            2,
            "account/rateLimits/read",
            json!({})
        )
        .await
        .is_err());
        let mut output =
            BufReader::new(&b"{\"id\":2,\"error\":{\"message\":\"private response\"}}\n"[..]);
        let error = account_request(
            &mut input,
            &mut output,
            2,
            "account/rateLimits/read",
            json!({}),
        )
        .await
        .unwrap_err();
        assert!(!error.contains("private response"));
    }
}
