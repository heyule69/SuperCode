//! Google sign-in is performed by Gemini CLI. Credentials remain in its account
//! directory; each conversation retains separate policy and folder-trust files.
use super::*;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::io::{AsyncBufRead, AsyncWrite};

pub(crate) fn home(base: &Path, config: &Value) -> Result<PathBuf, String> {
    if let Some(id) = config["accountId"].as_str() {
        return crate::official_accounts::account_directory(base, "gemini", id);
    }
    let path = std::env::var_os("GEMINI_CLI_HOME")
        .filter(|path| !path.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE"))
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or("无法确定 Gemini 本机账号目录")?;
    if !path.is_absolute() {
        return Err("Gemini 账号目录必须是绝对路径".into());
    }
    Ok(crate::agents::ordinary(path))
}

pub(super) fn apply_home(
    base: &Path,
    config: &Value,
    env: &mut [(String, String)],
) -> Result<(), String> {
    let target = home(base, config)?.to_string_lossy().into_owned();
    let value = env
        .iter_mut()
        .find(|(name, _)| name == "GEMINI_CLI_HOME")
        .ok_or("Gemini 账号配置不完整")?;
    value.1 = target;
    Ok(())
}

fn login_fingerprint(home: &Path) -> Result<Option<String>, String> {
    use std::io::Read;
    let path = home.join(".gemini/oauth_creds.json");
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取 Gemini 登录凭证".into()),
    };
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取 Gemini 登录凭证")?;
    if bytes.len() > 64 * 1024 {
        return Err("Gemini 登录凭证超过大小限制".into());
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Gemini 登录凭证格式无效，请重新登录")?;
    let secret = credential_identity(&value, chrono::Utc::now().timestamp_millis());
    Ok(secret.map(|secret| {
        format!(
            "{:x}",
            Sha256::digest(format!("{}\0{secret}", home.display()).as_bytes())
        )
    }))
}

fn credential_identity(value: &Value, now: i64) -> Option<&str> {
    if value["type"]
        .as_str()
        .is_some_and(|kind| kind != "authorized_user")
    {
        return None;
    }
    let access = value["access_token"]
        .as_str()
        .filter(|token| !token.trim().is_empty())?;
    value["refresh_token"]
        .as_str()
        .filter(|token| !token.trim().is_empty())
        .or_else(|| {
            value["expiry_date"]
                .as_i64()
                .filter(|expiry| *expiry > now)
                .map(|_| access)
        })
}

pub(crate) fn status(base: &Path, account_id: Option<&str>) -> Result<Value, String> {
    let home = home(base, &json!({"accountId":account_id}))?;
    let logged_in = login_fingerprint(&home)?.is_some();
    Ok(json!({"loggedIn":logged_in,"method":if logged_in {Some("oauth-personal")} else {None}}))
}

pub(crate) async fn login_command(
    app: &AppHandle,
    id: &str,
) -> Result<tokio::process::Command, String> {
    let base = app.state::<AppState>().data_dir.clone();
    let config = json!({"official":true,"accountId":id});
    let runtime = home(&base, &config)?.join("login-runtime");
    tokio::fs::create_dir_all(&runtime)
        .await
        .map_err(|_| "无法创建 Google 登录目录")?;
    let (args, mut env) = gemini::configure(&runtime, &config, "summary", &runtime).await?;
    apply_home(&base, &config, &mut env)?;
    env.iter_mut()
        .find(|(name, _)| name == "NO_BROWSER")
        .unwrap()
        .1 = "false".into();
    let launch = crate::agents::resolve(app, "gemini")?;
    let mut command = launch.command(&[]);
    gemini::clear_environment(&mut command);
    command.args(args).envs(env).current_dir(runtime);
    Ok(command)
}

pub(crate) async fn request<W: AsyncWrite + Unpin, R: AsyncBufRead + Unpin>(
    input: &mut W,
    output: &mut R,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let mut bytes =
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .map_err(|_| "Gemini 登录参数无效")?;
    bytes.push(b'\n');
    input
        .write_all(&bytes)
        .await
        .map_err(|_| "Gemini 登录进程已退出")?;
    input.flush().await.map_err(|_| "Gemini 登录进程已退出")?;
    loop {
        let frame = runtime::read_frame(output)
            .await?
            .ok_or("Gemini 登录进程已退出，请重新登录")?;
        let response: Value =
            serde_json::from_slice(&frame).map_err(|_| "Gemini 登录响应不是有效 JSON")?;
        if response.get("id") == Some(&json!(id)) && response.get("method").is_none() {
            if let Some(error) = response.get("error") {
                // A native error can contain HTTP bodies or credential details.
                // Return its code, never the raw response, to the webview.
                return Err(format!(
                    "Gemini 官方登录失败（ACP {}），请检查网络和 Google 账号权限",
                    error["code"].as_i64().unwrap_or(-32603)
                ));
            }
            return response
                .get("result")
                .cloned()
                .ok_or("Gemini 登录响应无效".into());
        }
        if response.get("id").is_some() && response.get("method").is_some() {
            return Err("Gemini 登录过程发起了非登录请求，已停止".into());
        }
    }
}

pub(crate) async fn initialize<W: AsyncWrite + Unpin, R: AsyncBufRead + Unpin>(
    input: &mut W,
    output: &mut R,
) -> Result<(), String> {
    let initialized = request(input, output, 1, "initialize", super::init()).await?;
    super::validate_acp("gemini", &initialized)?;
    if !initialized["authMethods"]
        .as_array()
        .is_some_and(|methods| {
            methods
                .iter()
                .any(|method| method["id"] == "oauth-personal")
        })
    {
        return Err("当前 Gemini CLI 未提供 Google 登录，请更新 Gemini CLI".into());
    }
    Ok(())
}

pub(crate) async fn complete<W: AsyncWrite + Unpin, R: AsyncBufRead + Unpin>(
    input: &mut W,
    output: &mut R,
    cwd: &Path,
) -> Result<Value, String> {
    request(
        input,
        output,
        2,
        "authenticate",
        json!({"methodId":"oauth-personal"}),
    )
    .await?;
    let session = request(
        input,
        output,
        3,
        "session/new",
        json!({"cwd":crate::agents::ordinary(cwd.to_path_buf()),"mcpServers":[]}),
    )
    .await?;
    parse_catalog(&session)
}

pub(super) fn parse_catalog(session: &Value) -> Result<Value, String> {
    let rows = session["models"]["availableModels"]
        .as_array()
        .ok_or("Gemini CLI 未返回可用模型，请更新 CLI")?;
    if rows.is_empty() || rows.len() > 200 {
        return Err("Gemini 官方模型目录无效".into());
    }
    let current = session["models"]["currentModelId"]
        .as_str()
        .ok_or("Gemini CLI 未返回当前模型")?;
    let mut data = Vec::new();
    for row in rows {
        let id = row["modelId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 200 && !id.chars().any(char::is_control))
            .ok_or("Gemini 官方模型 ID 无效")?;
        if data.iter().any(|model: &Value| model["model"] == id) {
            continue;
        }
        data.push(json!({"id":id,"model":id,"displayName":protocol::bounded(row["name"].as_str().unwrap_or(id),200),"isDefault":id==current}));
    }
    if !data.iter().any(|model| model["isDefault"] == true) {
        return Err("Gemini 当前模型不在官方模型目录中".into());
    }
    Ok(json!({"data":data,"origin":"nativeCli"}))
}

pub(crate) fn save_catalog(app: &AppHandle, config: &Value, catalog: &Value) -> Result<(), String> {
    let state = app.state::<AppState>();
    let home = home(&state.data_dir, config)?;
    let fingerprint = login_fingerprint(&home)?.ok_or("请先完成 Google 官方登录")?;
    state.store.set_setting(
        &format!("gemini_official_models:{fingerprint}"),
        &catalog.to_string(),
    )
}

pub(crate) async fn catalog(
    app: &AppHandle,
    config: &Value,
    refresh: bool,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let home = home(&state.data_dir, config)?;
    let fingerprint = login_fingerprint(&home)?.ok_or("请先完成 Google 官方登录")?;
    if !refresh {
        if let Some(saved) = state
            .store
            .setting(&format!("gemini_official_models:{fingerprint}"))?
        {
            return serde_json::from_str(&saved)
                .map_err(|_| "Gemini 官方模型缓存无效，请刷新模型".into());
        }
    }
    let cwd = state.data_dir.join("agents/gemini-catalog");
    tokio::fs::create_dir_all(&cwd)
        .await
        .map_err(|_| "无法创建 Gemini 模型检查目录")?;
    let launch = crate::agents::resolve(app, "gemini")?;
    let (client, mut events, _) = super::start_configured(
        app,
        "gemini",
        &cwd,
        None,
        "summary",
        config.clone(),
        &launch,
        false,
    )
    .await?;
    let result = async {
        let initialized =
            super::probe_request(&client, &mut events, "initialize", super::init()).await?;
        super::validate_acp("gemini", &initialized)?;
        super::probe_request(
            &client,
            &mut events,
            "authenticate",
            json!({"methodId":"oauth-personal"}),
        )
        .await?;
        let session = super::probe_request(
            &client,
            &mut events,
            "session/new",
            json!({"cwd":cwd,"mcpServers":[]}),
        )
        .await?;
        parse_catalog(&session)
    }
    .await;
    client.close().await;
    let catalog = result?;
    save_catalog(app, config, &catalog)?;
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        std::env::temp_dir().join(format!("supercode-google-login-{}", uuid::Uuid::new_v4()))
    }

    fn native_models() -> Value {
        json!({"models":{"currentModelId":"auto","availableModels":[{"modelId":"auto","name":"Auto"},{"modelId":"gemini-fixture","name":"Gemini fixture","privateToken":"must-not-cross-ipc"}]},"privateToken":"must-not-cross-ipc"})
    }

    #[test]
    fn only_usable_google_oauth_credentials_authorize_a_saved_account() {
        assert!(credential_identity(&json!({"apiKey":"fixture"}), 100).is_none());
        assert!(credential_identity(
            &json!({"access_token":"fixture-access","expiry_date":99}),
            100
        )
        .is_none());
        assert!(credential_identity(&json!({"type":"service_account","access_token":"fixture-access","refresh_token":"fixture-refresh"}), 100).is_none());
        assert!(credential_identity(
            &json!({"access_token":"","refresh_token":"fixture-refresh"}),
            100
        )
        .is_none());
        assert!(credential_identity(
            &json!({"access_token":"fixture-access","expiry_date":101}),
            100
        )
        .is_some());
        assert!(credential_identity(&json!({"access_token":"fixture-access","expiry_date":99,"refresh_token":"fixture-refresh"}), 100).is_some());
    }

    #[test]
    fn saved_status_is_local_account_scoped_and_never_returns_credentials() {
        let root = fixture_root();
        let a = uuid::Uuid::new_v4().to_string();
        let b = uuid::Uuid::new_v4().to_string();
        let a_home = home(&root, &json!({"accountId":a})).unwrap();
        std::fs::create_dir_all(a_home.join(".gemini")).unwrap();
        std::fs::write(a_home.join(".gemini/oauth_creds.json"), json!({"access_token":"fixture-private-access","refresh_token":"fixture-private-refresh","expiry_date":0}).to_string()).unwrap();
        let value = status(&root, Some(&a)).unwrap();
        assert_eq!(value, json!({"loggedIn":true,"method":"oauth-personal"}));
        assert!(!value.to_string().contains("fixture-private"));
        assert_eq!(status(&root, Some(&b)).unwrap()["loggedIn"], false);
        assert!(home(&root, &json!({"accountId":"../outside"})).is_err());
        std::fs::write(
            a_home.join(".gemini/oauth_creds.json"),
            b"not json with private credentials",
        )
        .unwrap();
        let error = status(&root, Some(&a)).unwrap_err();
        assert!(!error.contains("private credentials"));
        std::fs::remove_dir_all(root.canonicalize().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn oauth_reuses_only_its_account_home_and_keeps_conversation_trust_separate() {
        let root = fixture_root();
        let project = root.join("project");
        tokio::fs::create_dir_all(&project).await.unwrap();
        let a = json!({"official":true,"accountId":uuid::Uuid::new_v4().to_string()});
        let b = json!({"official":true,"accountId":uuid::Uuid::new_v4().to_string()});
        let runtime_a = root.join("runtime-a");
        let runtime_b = root.join("runtime-b");
        let (_, mut env_a) = gemini::configure(&runtime_a, &a, "full", &project)
            .await
            .unwrap();
        let (_, mut env_b) = gemini::configure(&runtime_b, &a, "read", &project)
            .await
            .unwrap();
        apply_home(&root, &a, &mut env_a).unwrap();
        apply_home(&root, &a, &mut env_b).unwrap();
        let get = |env: &Vec<(String, String)>, key: &str| {
            env.iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            get(&env_a, "GEMINI_CLI_HOME"),
            get(&env_b, "GEMINI_CLI_HOME")
        );
        assert_ne!(home(&root, &a).unwrap(), home(&root, &b).unwrap());
        assert_ne!(
            get(&env_a, "GEMINI_CLI_TRUSTED_FOLDERS_PATH"),
            get(&env_b, "GEMINI_CLI_TRUSTED_FOLDERS_PATH")
        );
        assert_eq!(get(&env_a, "GOOGLE_GENAI_USE_GCA").as_deref(), Some("true"));
        assert_eq!(get(&env_a, "NO_BROWSER").as_deref(), Some("true"));
        assert!(get(&env_a, "GEMINI_API_KEY").is_none());
        assert!(get(&env_a, "GOOGLE_GEMINI_BASE_URL").is_none());
        assert_eq!(
            gemini::authentication(&a),
            json!({"methodId":"oauth-personal"})
        );
        assert_eq!(
            serde_json::from_slice::<Value>(
                &tokio::fs::read(runtime_b.join(".gemini/trustedFolders.json"))
                    .await
                    .unwrap()
            )
            .unwrap(),
            json!({})
        );
        let (_, api_env) = gemini::configure(
            &root.join("api-runtime"),
            &json!({"apiKey":"fixture-api-key"}),
            "summary",
            &project,
        )
        .await
        .unwrap();
        assert_eq!(
            get(&api_env, "GOOGLE_GENAI_USE_GCA").as_deref(),
            Some("false")
        );
        assert_eq!(
            get(&api_env, "GEMINI_API_KEY").as_deref(),
            Some("fixture-api-key")
        );
        assert!(gemini::configure(
            &runtime_b,
            &json!({"official":true,"apiKey":"fixture"}),
            "summary",
            &project
        )
        .await
        .is_err());
        tokio::fs::remove_dir_all(root.canonicalize().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn google_login_uses_native_oauth_then_lists_models_without_sending_a_prompt() {
        let (client, server) = tokio::io::duplex(16 * 1024);
        let (server_read, mut server_write) = tokio::io::split(server);
        let worker = tokio::spawn(async move {
            let mut output = BufReader::new(server_read);
            for (id, method) in [(1, "initialize"), (2, "authenticate"), (3, "session/new")] {
                let frame = runtime::read_frame(&mut output).await.unwrap().unwrap();
                let request: Value = serde_json::from_slice(&frame).unwrap();
                assert_eq!(request["method"], method);
                assert_eq!(request["id"], id);
                if method == "authenticate" {
                    assert_eq!(request["params"], json!({"methodId":"oauth-personal"}));
                }
                if method == "session/new" {
                    assert_eq!(request["params"]["mcpServers"], json!([]));
                }
                let result = match method {
                    "initialize" => {
                        json!({"protocolVersion":1,"agentInfo":{"version":"0.64.0"},"authMethods":[{"id":"oauth-personal"}]})
                    }
                    "authenticate" => json!({}),
                    _ => native_models(),
                };
                server_write
                    .write_all(
                        format!("{}\n", json!({"jsonrpc":"2.0","id":id,"result":result}))
                            .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let (output, mut input) = tokio::io::split(client);
        let mut output = BufReader::new(output);
        initialize(&mut input, &mut output).await.unwrap();
        let result = complete(&mut input, &mut output, Path::new("D:/example"))
            .await
            .unwrap();
        assert_eq!(result["data"].as_array().unwrap().len(), 2);
        assert_eq!(result["data"][0]["isDefault"], true);
        assert!(!result.to_string().contains("must-not-cross-ipc"));
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn failed_authorization_does_not_leak_native_response_bodies() {
        let (client, server) = tokio::io::duplex(4096);
        let (read, mut write) = tokio::io::split(server);
        let worker = tokio::spawn(async move {
            runtime::read_frame(&mut BufReader::new(read))
                .await
                .unwrap()
                .unwrap();
            write.write_all(b"{\"id\":2,\"error\":{\"code\":-32000,\"message\":\"fixture-private-access\",\"data\":{\"refresh_token\":\"fixture-private-refresh\"}}}\n").await.unwrap();
        });
        let (output, mut input) = tokio::io::split(client);
        let error = request(
            &mut input,
            &mut BufReader::new(output),
            2,
            "authenticate",
            json!({"methodId":"oauth-personal"}),
        )
        .await
        .unwrap_err();
        assert!(error.contains("-32000"));
        assert!(!error.contains("fixture-private"));
        worker.await.unwrap();
    }

    #[test]
    fn invalid_native_catalogs_cannot_invent_a_default_model() {
        assert!(parse_catalog(&json!({})).is_err());
        let mut models = native_models();
        models["models"]["currentModelId"] = "not-returned-by-cli".into();
        assert!(parse_catalog(&models).is_err());
    }
}
