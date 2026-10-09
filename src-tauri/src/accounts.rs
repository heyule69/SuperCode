//! Official sign-in remains owned by the installed agent. SuperCode never receives OAuth tokens.
use crate::{process, AppState};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
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
        self.get_for(agent, None, force, app).await
    }
    pub(crate) async fn get_for(
        &self,
        agent: &str,
        account_id: Option<&str>,
        force: bool,
        app: &AppHandle,
    ) -> Result<Value, String> {
        let index = match agent {
            "codex" => 0,
            "claude" => 1,
            _ => return Err("不支持此 Agent 的官方账号".into()),
        };
        let _check = self.checks[index].lock().await;
        let cache_key = format!("{agent}:{}", account_id.unwrap_or("native"));
        if !force {
            if let Some((at, value)) = self.cache.lock().await.get(&cache_key) {
                if at.elapsed() < Duration::from_secs(60) {
                    return Ok(value.clone());
                }
            }
        }
        let value = read_official_status(agent, account_id, app).await?;
        self.cache
            .lock()
            .await
            .insert(cache_key, (Instant::now(), value.clone()));
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
    let mut rows = vec![codex, claude];
    for profile in app
        .state::<AppState>()
        .store
        .profiles()?
        .into_iter()
        .filter(|p| p.is_official() && p.account_id().is_some())
    {
        let mut status = app
            .state::<Accounts>()
            .get_for(
                &profile.agent,
                profile.account_id(),
                force.unwrap_or(false),
                &app,
            )
            .await
            .unwrap_or_else(|error| json!({"loggedIn":false,"error":error}));
        status["agent"] = profile.agent.into();
        status["connectionId"] = profile.id.into();
        rows.push(status);
    }
    Ok(rows)
}

/// A short-lived account-only connection. It never starts/resumes a thread, loads
/// SuperCode tools, or changes the main runtime's route, epoch or session state.
pub(crate) async fn read_codex_limits(app: &AppHandle) -> Result<Value, String> {
    read_codex_limits_for(app, None).await
}
pub(crate) async fn read_codex_limits_for(
    app: &AppHandle,
    account_id: Option<&str>,
) -> Result<Value, String> {
    read_codex_value(app, account_id, "account/rateLimits/read", json!({})).await
}
pub(crate) fn codex_command(
    app: &AppHandle,
    account_id: Option<&str>,
) -> Result<tokio::process::Command, String> {
    let launch = crate::agents::resolve(app, "codex")?;
    let mut command = launch.command(&["app-server", "--listen", "stdio://"]);
    for setting in process::codex_config_overrides(false)? {
        command.arg("-c").arg(setting);
    }
    command.args(["-c", "model_provider=\"openai\""]);
    crate::official_accounts::configure(
        &mut command,
        app,
        "codex",
        &json!({"official":true,"accountId":account_id}),
    )?;
    // Official allowance belongs to the CLI's saved ChatGPT login, not an API key.
    for variable in [
        "OPENAI_API_KEY",
        "OPENAI_BASE_URL",
        "SUPERCODE_PROVIDER_API_KEY",
    ] {
        command.env_remove(variable);
    }
    Ok(command)
}
pub(crate) async fn read_codex_value(
    app: &AppHandle,
    account_id: Option<&str>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let mut command = codex_command(app, account_id)?;
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
pub(crate) async fn read_account_method<W, R>(
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
pub(crate) fn claude_command(
    app: &AppHandle,
    args: &[&str],
) -> Result<tokio::process::Command, String> {
    let launch = crate::agents::resolve(app, "claude")?;
    let mut c = launch.command(&["--setting-sources", ""]);
    c.args(args);
    for name in [
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
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
async fn read_official_status(
    agent: &str,
    account_id: Option<&str>,
    app: &AppHandle,
) -> Result<Value, String> {
    if agent == "codex" {
        let value = read_codex_value(
            app,
            account_id,
            "account/read",
            json!({"refreshToken":false}),
        )
        .await?;
        return Ok(codex_status(&value));
    }
    if agent != "claude" {
        return Err("不支持此 Agent 的账号".into());
    }
    use tokio::io::AsyncReadExt;
    let mut command = claude_command(app, &["auth", "status", "--json"])?;
    crate::official_accounts::configure(
        &mut command,
        app,
        agent,
        &json!({"official":true,"accountId":account_id}),
    )?;
    let mut child = command
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
pub async fn use_official_account(
    agent: String,
    account_id: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let status = app
        .state::<Accounts>()
        .get_for(&agent, account_id.as_deref(), true, &app)
        .await?;
    if status["loggedIn"] != true {
        return Err("请先完成此账号的官方登录".into());
    }
    state.runtime.release(&app).await?;
    if let Some(id) = account_id {
        let connection = format!("account:{id}");
        state
            .store
            .profile(&agent, &connection)?
            .ok_or("账号不存在")?;
        state.store.select_profile(&agent, Some(&connection))?;
    } else {
        state.store.use_official(&agent)?;
    }
    let _ = app.emit("workspace-updated", ());
    Ok(())
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
