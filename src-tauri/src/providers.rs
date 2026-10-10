//! Provider presets and editable local API connections. Model IDs are passed through verbatim.
use crate::{ccswitch::Profile, AppState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub id: Option<String>,
    pub name: String,
    pub agent: String,
    pub provider_id: String,
    pub plan: String,
    pub protocol: String,
    pub base_url: String,
    pub model: String,
    pub models: Vec<String>,
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub has_credential: bool,
    #[serde(default)]
    pub official: bool,
}

#[tauri::command]
pub fn provider_catalog() -> Value {
    bundled_catalog().clone()
}

fn bundled_catalog() -> &'static Value {
    static CATALOG: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../resources/providers.json"))
            .expect("bundled provider catalog")
    })
}

/// Display metadata only: never send a URL, credential or CLI configuration to the picker.
pub fn model_source(config: &Value, agent: &str, connection_name: Option<&str>) -> Value {
    if matches!(agent, "opencode" | "pi") && configured_base_url(config).is_none() {
        return json!({"providerId":"custom","providerName":"本机模型配置","connectionName":connection_name.unwrap_or("本机配置"),"mark":if agent=="pi"{"P"}else{"O"}});
    }
    let base = configured_base_url(config);
    let url = base.as_deref().and_then(|s| reqwest::Url::parse(s).ok());
    let catalog = bundled_catalog();
    let native_provider = config["config"]
        .as_str()
        .and_then(|s| toml::from_str::<toml::Table>(s).ok())
        .and_then(|t| {
            t.get("model_provider")
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        });
    let explicit = config["providerId"].as_str();
    let mut matched_plan = None;
    let provider = explicit
        .and_then(|id| catalog.as_array()?.iter().find(|p| p["id"] == id))
        .or_else(|| {
            if explicit.is_some() {
                return None;
            }
            let url = url.as_ref()?;
            let mut best = None;
            let mut longest = 0;
            for p in catalog.as_array()? {
                for preset in p["presets"].as_array()? {
                    let Some(endpoint) = preset["baseUrl"]
                        .as_str()
                        .and_then(|s| reqwest::Url::parse(s).ok())
                    else {
                        continue;
                    };
                    let path = endpoint.path().trim_end_matches('/');
                    if url.host_str() == endpoint.host_str()
                        && url.port_or_known_default() == endpoint.port_or_known_default()
                        && (url.path().trim_end_matches('/') == path
                            || url.path().starts_with(&format!("{path}/")))
                        && path.len() >= longest
                    {
                        // Some API and subscription plans share an endpoint. Do not guess a plan.
                        let ambiguous = best
                            .is_some_and(|previous: &Value| previous["id"] == p["id"])
                            && path.len() == longest;
                        longest = path.len();
                        best = Some(p);
                        matched_plan = if ambiguous {
                            None
                        } else {
                            preset["id"].as_str()
                        };
                    }
                }
            }
            // Kimi also publishes api.kimi.ai as the Coding Plan endpoint.
            if best.is_none()
                && url.host_str() == Some("api.kimi.ai")
                && url.path().starts_with("/coding")
            {
                matched_plan = Some("coding");
                best = catalog.as_array()?.iter().find(|p| p["id"] == "kimi");
            }
            best
        });
    let plan = config["plan"].as_str().or(matched_plan);
    let id = provider.and_then(|p| p["id"].as_str()).unwrap_or(
        if base.is_none()
            && explicit.is_none()
            && native_provider.as_deref().is_none_or(|id| id == "openai")
        {
            if agent == "claude" {
                "anthropic"
            } else {
                "openai"
            }
        } else {
            "custom"
        },
    );
    let coding = id == "kimi" && plan == Some("coding");
    let name = if coding {
        "Kimi Code"
    } else if id == "custom" {
        connection_name
            .or_else(|| url.as_ref().and_then(|u| u.host_str()))
            .or(native_provider.as_deref())
            .unwrap_or("自定义 API")
    } else {
        provider
            .and_then(|p| p["name"].as_str())
            .unwrap_or(match id {
                "anthropic" => "Anthropic",
                "openai" => "OpenAI",
                _ => connection_name
                    .or_else(|| url.as_ref().and_then(|u| u.host_str()))
                    .or(native_provider.as_deref())
                    .unwrap_or("自定义 API"),
            })
    };
    let plan_name = provider
        .and_then(|p| p["presets"].as_array())
        .and_then(|presets| presets.iter().find(|p| p["id"].as_str() == plan))
        .and_then(|p| p["name"].as_str());
    // A custom connection can point directly at a known model family. Keep its
    // user-defined supplier name, while exposing safe model display metadata.
    let family = url
        .as_ref()
        .filter(|url| {
            matches!(url.host_str(), Some("api.kimi.com" | "api.kimi.ai"))
                && (url.path().trim_end_matches('/') == "/coding"
                    || url.path().starts_with("/coding/"))
        })
        .map(|_| "kimi");
    let initial = connection_name
        .unwrap_or(name)
        .trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or("?".into());
    let mark = if id == "custom" {
        initial.as_str()
    } else {
        provider
            .and_then(|p| p["mark"].as_str())
            .unwrap_or(if id == "anthropic" {
                "✳"
            } else if id == "openai" {
                "◎"
            } else {
                &initial
            })
    };
    json!({"providerId":id,"providerName":name,"mark":mark,
        "planName":if coding {Some("编程计划")} else if id == "custom" && base.is_some() {Some("API")} else {plan_name},"connectionName":connection_name,"modelFamily":family})
}

fn configured_base_url(config: &Value) -> Option<String> {
    config["baseUrl"]
        .as_str()
        .or(config["env"]["ANTHROPIC_BASE_URL"].as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            let table = toml::from_str::<toml::Table>(config["config"].as_str()?).ok()?;
            let provider = table.get("model_provider")?.as_str()?;
            table
                .get("model_providers")?
                .get(provider)?
                .get("base_url")?
                .as_str()
                .map(str::to_owned)
        })
}

#[tauri::command]
pub async fn get_model_source(
    agent: String,
    session_id: Option<String>,
    connection_id: Option<String>,
    app: AppHandle,
) -> Result<Value, String> {
    let route = crate::chat_connection::available_route(
        &app, &agent, session_id.as_deref(), connection_id.as_deref(),
    ).await?;
    Ok(crate::chat_connection::model_source(route.as_ref(), &agent))
}

pub fn real_claude_model(config: &Value, requested: Option<&str>) -> Option<String> {
    let env = &config["env"];
    let value = requested
        .filter(|s| !s.trim().is_empty())
        .or(config["model"].as_str())
        .or(env["ANTHROPIC_MODEL"].as_str());
    if let Some(value) = value {
        let suffix = if value.eq_ignore_ascii_case("opus") {
            Some("OPUS")
        } else if value.eq_ignore_ascii_case("sonnet") {
            Some("SONNET")
        } else if value.eq_ignore_ascii_case("haiku") {
            Some("HAIKU")
        } else {
            None
        };
        if let Some(suffix) = suffix {
            if let Some(actual) = env[format!("ANTHROPIC_DEFAULT_{suffix}_MODEL")]
                .as_str()
                .filter(|s| !s.is_empty())
            {
                return Some(actual.to_owned());
            }
        }
        return Some(value.to_owned());
    }
    [
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    ]
    .iter()
    .find_map(|key| {
        env[*key]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    })
}

pub fn model_ids(config: &Value) -> Vec<String> {
    let mut models = Vec::new();
    let mut add = |s: &str| {
        if !s.is_empty() && !models.iter().any(|m| m == s) {
            models.push(s.to_owned());
        }
    };
    if let Some(default) = real_claude_model(config, None) {
        add(&default);
    }
    if let Some(values) = config["models"].as_array() {
        for s in values.iter().filter_map(Value::as_str) {
            add(s);
        }
    }
    for key in [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    ] {
        if let Some(s) = config["env"][key].as_str() {
            add(s);
        }
    }
    models
}

pub fn catalog_models(config: &Value) -> Value {
    let default = real_claude_model(config, None);
    json!({"data":model_ids(config).into_iter().map(|id| json!({"id":id,"model":id,"displayName":id,"isDefault":Some(id.as_str())==default.as_deref()})).collect::<Vec<_>>()})
}

pub fn validate_url(value: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(value.trim()).map_err(|_| "API 地址无效，请填写完整 Base URL")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Base URL 不能包含账号、密码、查询参数或片段；API Key 请填在密钥栏".into());
    }
    if url.scheme() != "https"
        && !(url.scheme() == "http"
            && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
    {
        return Err(
            "远程 API 请使用 HTTPS；本地服务可使用 localhost / 127.0.0.1 的 HTTP 地址".into(),
        );
    }
    Ok(value.trim().trim_end_matches('/').to_owned())
}
pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("SuperCode/0.1.0")
        .build()
        .map_err(|_| "无法初始化 API 连接".into())
}
pub fn key(config: &Value) -> Option<&str> {
    config["apiKey"]
        .as_str()
        .or(config["env"]["ANTHROPIC_AUTH_TOKEN"].as_str())
        .or(config["env"]["ANTHROPIC_API_KEY"].as_str())
        .filter(|s| !s.is_empty())
}
pub async fn read_json(response: reqwest::Response) -> Result<Value, String> {
    use futures_util::StreamExt;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "API 返回 HTTP {}，请检查地址、Key、模型和套餐权限",
            status.as_u16()
        ));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "读取 API 响应失败")?;
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("API 响应超过 2 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "API 返回的内容不是受支持的 JSON".to_owned())
}
pub fn settings(profile: &Profile) -> Settings {
    let c = &profile.config;
    let codex_base = c["config"]
        .as_str()
        .and_then(|text| toml::from_str::<toml::Table>(text).ok())
        .and_then(|table| {
            let provider = table.get("model_provider")?.as_str()?;
            if provider == "openai" {
                return Some("https://api.openai.com/v1".to_owned());
            }
            table
                .get("model_providers")?
                .get(provider)?
                .get("base_url")?
                .as_str()
                .map(str::to_owned)
        });
    Settings {
        id: Some(profile.id.clone()),
        name: profile.name.clone(),
        agent: profile.agent.clone(),
        provider_id: c["providerId"].as_str().unwrap_or("custom").into(),
        plan: c["plan"].as_str().unwrap_or("standard").into(),
        protocol: c["protocol"]
            .as_str()
            .unwrap_or(if profile.agent == "claude" {
                "anthropic"
            } else {
                "responses"
            })
            .into(),
        base_url: c["baseUrl"]
            .as_str()
            .or(c["env"]["ANTHROPIC_BASE_URL"].as_str())
            .map(str::to_owned)
            .or(codex_base)
            .unwrap_or_default(),
        model: real_claude_model(c, None).unwrap_or_default(),
        models: model_ids(c),
        api_key: None,
        has_credential: key(c).is_some(),
        official: profile.is_official(),
    }
}
fn find(app: &AppHandle, id: &str) -> Result<Profile, String> {
    app.state::<AppState>()
        .store
        .profiles()?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or("连接配置已不存在".into())
}
#[tauri::command]
pub fn get_provider_profile(id: String, app: AppHandle) -> Result<Settings, String> {
    Ok(settings(&find(&app, &id)?))
}

fn make_profile(mut s: Settings, previous: Option<&Profile>) -> Result<Profile, String> {
    s.name = s.name.trim().into();
    s.model = s.model.trim().into();
    if s.name.is_empty() || s.name.len() > 100 {
        return Err("连接名称需要 1–100 个 UTF-8 字节".into());
    }
    if s.model.is_empty() || s.model.len() > 200 || s.model.chars().any(char::is_control) {
        return Err("请填写真实模型 ID，长度不超过 200 字节".into());
    }
    if !matches!(s.protocol.as_str(), "anthropic" | "chat" | "responses") {
        return Err("API 协议无效".into());
    }
    if (s.agent == "codex" && s.protocol != "responses")
        || (s.agent == "claude" && s.protocol != "anthropic" && !(s.protocol == "chat" && previous.is_some_and(|p| p.agent == "claude" && p.config["protocol"] == s.protocol)))
        || !matches!(s.agent.as_str(), "claude" | "codex" | "opencode" | "pi")
    {
        return Err(
            "Claude Code 新连接需要 Anthropic Messages；Codex 需要 OpenAI Responses".into(),
        );
    }
    if s.official && previous.is_none() { return Err("官方账号请通过官方登录添加".into()); }
    if previous.is_some_and(|p| p.agent != s.agent) {
        return Err("编辑连接时不能更换 Agent，请新增连接".into());
    }
    let secret = s
        .api_key
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| previous.and_then(|p| key(&p.config)))
        .unwrap_or("")
        .trim();
    if secret.len() > 8192 || secret.chars().any(char::is_control) {
        return Err("API Key 格式无效".into());
    }
    let mut models = vec![s.model.clone()];
    if s.models.len() > 200 {
        return Err("单个连接最多保存 200 个模型".into());
    }
    for m in s.models {
        let m = m.trim();
        if m.len() > 200 || m.chars().any(char::is_control) {
            return Err("模型 ID 格式无效".into());
        }
        if !m.is_empty() && !models.iter().any(|v| v == m) {
            models.push(m.to_owned());
        }
    }
    if let Some(previous) = previous.filter(|p| p.is_official()) {
        if !secret.is_empty() {
            return Err("官方登录连接不接收 API Key，请新增 API 连接".into());
        }
        let mut config = previous.config.clone();
        config["model"] = json!(s.model);
        config["models"] = json!(models);
        if s.agent == "codex" {
            let mut table =
                toml::from_str::<toml::Table>(config["config"].as_str().ok_or("官方配置无效")?)
                    .map_err(|_| "官方配置无效")?;
            table.insert("model".into(), toml::Value::String(s.model));
            config["config"] = json!(toml::to_string(&table).map_err(|_| "无法更新官方模型")?);
        }
        return Ok(Profile {
            id: previous.id.clone(),
            agent: previous.agent.clone(),
            name: s.name,
            config,
        });
    }
    let base = validate_url(&s.base_url)?;
    let mut config = json!({"providerId":s.provider_id,"plan":s.plan,"protocol":s.protocol,"baseUrl":base,"model":s.model,"models":models,"source":"manual"});
    if s.protocol == "anthropic" {
        // Explicit IDs; aliases are used only by legacy CLI subagent internals, never by the selector.
        config["env"] = json!({"ANTHROPIC_BASE_URL":base,"ANTHROPIC_AUTH_TOKEN":secret,"ANTHROPIC_MODEL":s.model,"CLAUDE_CODE_SUBAGENT_MODEL":s.model,"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1"});
    } else {
        config["apiKey"] = json!(secret);
    }
    if s.agent == "codex" {
        let mut table = toml::Table::new();
        table.insert("model".into(), toml::Value::String(s.model.clone()));
        table.insert(
            "model_provider".into(),
            toml::Value::String("supercode_api".into()),
        );
        // Apply a vendor requirement only to its exact bundled endpoint. A user
        // changing the address to a gateway must not inherit that restriction.
        let preset = bundled_catalog().as_array().and_then(|providers| {
            providers.iter().find(|p| p["id"] == s.provider_id)?["presets"]
                .as_array()?.iter().find(|p| p["id"] == s.plan && p["protocol"] == "responses"
                    && p["baseUrl"].as_str().is_some_and(|url| url.trim_end_matches('/') == base.trim_end_matches('/')))
        });
        if preset.is_some_and(|p| p["codex"]["webSearch"] == "disabled") {
            table.insert("web_search".into(), toml::Value::String("disabled".into()));
        }
        let mut connection = toml::Table::new();
        for (k, v) in [
            ("name", s.name.as_str()),
            ("base_url", base.as_str()),
            ("wire_api", "responses"),
            ("env_key", "SUPERCODE_PROVIDER_API_KEY"),
        ] {
            connection.insert(k.into(), toml::Value::String(v.into()));
        }
        connection.insert("requires_openai_auth".into(), toml::Value::Boolean(false));
        table.insert(
            "model_providers".into(),
            toml::Value::Table(toml::Table::from_iter([(
                "supercode_api".into(),
                toml::Value::Table(connection),
            )])),
        );
        config["config"] = json!(toml::to_string(&table).map_err(|_| "无法生成 Codex 配置")?);
    }
    Ok(Profile {
        id: s
            .id
            .unwrap_or_else(|| format!("api:{}", uuid::Uuid::new_v4())),
        agent: s.agent,
        name: s.name,
        config,
    })
}
#[tauri::command]
pub async fn save_provider_profile(
    settings: Settings,
    activate: bool,
    app: AppHandle,
) -> Result<crate::ccswitch::Summary, String> {
    let previous = settings
        .id
        .as_deref()
        .map(|id| find(&app, id))
        .transpose()?;
    if previous.as_ref().is_some_and(|p| p.agent != settings.agent) {
        return Err("更换连接的 Agent 请添加新连接，已有聊天的引擎不能被修改".into());
    }
    let profile = make_profile(settings, previous.as_ref())?;
    let state = app.state::<AppState>();
    state.runtime.release(&app).await?;
    let was_active = state
        .store
        .active_profile(&profile.agent)?
        .is_some_and(|p| p.id == profile.id);
    state
        .store
        .import_profiles(std::slice::from_ref(&profile))?;
    if activate {
        state
            .store
            .select_profile(&profile.agent, Some(&profile.id))?;
    }
    let _ = app.emit("workspace-updated", ());
    Ok(profile.summary(activate || was_active))
}
#[tauri::command]
pub async fn delete_provider_profile(id: String, app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.runtime.release(&app).await?;
    find(&app, &id)?;
    state.store.delete_profile(&id)
}
#[tauri::command]
pub async fn fetch_provider_models(id: String, app: AppHandle) -> Result<Vec<String>, String> {
    let p = find(&app, &id)?;
    if p.is_official() {
        let value = crate::commands::list_models(Some(p.agent), None, Some(p.id), app).await?;
        return Ok(value["data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m["model"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default());
    }
    let s = settings(&p);
    if s.base_url.is_empty() {
        return Err("此 CLI 配置未设置 API 地址，可手动添加模型 ID".into());
    }
    let base = validate_url(&s.base_url)?;
    let url = if s.protocol == "anthropic" && !base.ends_with("/v1") {
        format!("{base}/v1/models")
    } else {
        format!("{base}/models")
    };
    let mut request = client()?.get(url);
    if let Some(key) = key(&p.config) {
        request = request.bearer_auth(key);
        if s.protocol == "anthropic" {
            request = request
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01");
        }
    }
    let value = read_json(
        request
            .send()
            .await
            .map_err(|_| "连接失败或超时，可手动填写模型 ID")?,
    )
    .await?;
    let mut models = value["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    m["id"]
                        .as_str()
                        .filter(|s| s.len() <= 200)
                        .map(str::to_owned)
                })
                .take(200)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    models.sort();
    models.dedup();
    if models.is_empty() {
        return Err("供应商没有返回模型列表，请手动添加模型 ID".into());
    }
    Ok(models)
}
#[tauri::command]
pub async fn test_provider_connection(id: String, app: AppHandle) -> Result<Value, String> {
    let p = find(&app, &id)?;
    let s = settings(&p);
    if s.official {
        let status = app.state::<crate::accounts::Accounts>().get_for(&p.agent, p.account_id(), true, &app).await?;
        if status["loggedIn"] != true {
            return Err("尚未检测到官方登录，请在供应商页登录官方账号".into());
        }
        return Ok(json!({"ok":true,"model":s.model,"verifiedBy":"officialAccountStatus"}));
    }
    let base = validate_url(&s.base_url)?;
    if matches!(s.plan.as_str(), "coding" | "token") {
        if app.state::<AppState>().store.running()? > 0 {
            return Err("请等待当前任务结束后测试套餐连接".into());
        }
        return test_coding_plan(&app, &p).await;
    }
    let (url, body) = match s.protocol.as_str() {
        "anthropic" => (
            format!(
                "{base}{}",
                if base.ends_with("/v1") {
                    "/messages"
                } else {
                    "/v1/messages"
                }
            ),
            json!({"model":s.model,"max_tokens":32,"messages":[{"role":"user","content":"Reply only OK."}]}),
        ),
        "responses" => (
            format!("{base}/responses"),
            json!({"model":s.model,"max_output_tokens":32,"input":"Reply only OK.","store":false}),
        ),
        _ => (
            format!("{base}/chat/completions"),
            json!({"model":s.model,"max_tokens":32,"messages":[{"role":"user","content":"Reply only OK."}]}),
        ),
    };
    let mut request = client()?.post(url).json(&body);
    if let Some(key) = key(&p.config) {
        request = request.bearer_auth(key);
        if s.protocol == "anthropic" {
            request = request
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01");
        }
    }
    let value = read_json(
        request
            .send()
            .await
            .map_err(|_| "连接失败或超时，请检查网络与 API 地址")?,
    )
    .await?;
    if value.get("error").is_some() {
        return Err("供应商返回错误，请检查模型权限和套餐余额".into());
    }
    let valid =
        value["content"].is_array() || value["choices"].is_array() || value["output"].is_array();
    if !valid {
        return Err("API 响应格式与所选协议不匹配".into());
    }
    Ok(json!({"ok":true,"model":value["model"].as_str().unwrap_or(&s.model),"protocol":s.protocol}))
}
async fn test_coding_plan(app: &AppHandle, profile: &Profile) -> Result<Value, String> {
    if profile.agent != "claude" || profile.config["protocol"] == "chat" {
        return Err(
            "套餐连接测试需要通过 Claude Code 的原生 Anthropic 接口，请选择对应预设".into(),
        );
    }
    let launch = crate::agents::resolve(app, "claude")?;
    let model = real_claude_model(&profile.config, None).ok_or("请填写模型 ID")?;
    let mut cmd = launch.command(&[
        "--print",
        "--output-format",
        "json",
        "--model",
        &model,
        "--tools",
        "",
        "--max-turns",
        "1",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--settings",
        "{\"disableAllHooks\":true}",
        "Reply only OK.",
    ]);
    for key in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    ] {
        cmd.env_remove(key);
    }
    if let Some(env) = profile.config["env"].as_object() {
        for (k, v) in env {
            if let Some(v) = v.as_str() {
                cmd.env(k, v);
            }
        }
    }
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "无法启动 Claude 连接测试")?;
    let _job = crate::process::JobGuard::attach(&child)?;
    let output = child.stdout.take().ok_or("无法读取测试输出")?;
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        output.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes),
    )
    .await;
    let _ = child.kill().await;
    read.map_err(|_| "套餐连接测试超时")?
        .map_err(|_| "读取套餐测试结果失败")?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("套餐测试响应过大".into());
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Claude 没有返回受支持的连接测试结果")?;
    if value["is_error"] == true || value["result"].as_str().is_none_or(|s| s.trim().is_empty()) {
        return Err(
            "套餐连接失败，请检查模型 ID、专属 Key 与套餐权限。详细原因可在会话中查看。".into(),
        );
    }
    Ok(
        json!({"ok":true,"model":value["modelUsage"].as_object().and_then(|o|o.keys().next()).map(String::as_str).unwrap_or(&model),"protocol":"anthropic","verifiedBy":"Claude Code"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_source_identifies_imported_provider_without_exposing_configuration() {
        for base in [
            "https://api.kimi.com/coding/",
            "https://api.kimi.ai/coding/v1",
        ] {
            let config = json!({"env":{"ANTHROPIC_BASE_URL":base,"ANTHROPIC_AUTH_TOKEN":"private-key","ANTHROPIC_MODEL":"k3[1M]"}});
            let source = model_source(&config, "claude", Some("kimi"));
            assert_eq!(source["providerId"], "kimi");
            assert_eq!(source["providerName"], "Kimi Code");
            assert_eq!(source["planName"], "编程计划");
            assert!(!source.to_string().contains("private-key"));
            assert!(!source.to_string().contains(base));
            assert_eq!(real_claude_model(&config, None).as_deref(), Some("k3[1M]"));
        }
    }
    #[test]
    fn model_source_respects_custom_connections_and_codex_routes() {
        let custom = model_source(
            &json!({"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com.attacker.invalid/coding/"}}),
            "claude",
            Some("我的连接"),
        );
        assert_eq!(custom["providerId"], "custom");
        assert_eq!(custom["providerName"], "我的连接");
        let codex = json!({"config":"model_provider='my-route'\n[model_providers.my-route]\nbase_url='https://api.x.ai/v1'\n"});
        assert_eq!(model_source(&codex, "codex", None)["providerId"], "xai");
        assert_eq!(
            model_source(&json!({}), "codex", None)["providerName"],
            "OpenAI"
        );
        assert_eq!(
            model_source(
                &json!({"config":"model_provider='private-route'"}),
                "codex",
                None
            )["providerName"],
            "private-route"
        );
        assert!(model_source(
            &json!({"env":{"ANTHROPIC_BASE_URL":"https://api.minimax.cn/anthropic"}}),
            "claude",
            None
        )["planName"]
            .is_null());
        assert_eq!(
            model_source(
                &json!({"providerId":"custom","baseUrl":"https://api.kimi.com/coding"}),
                "claude",
                None
            )["providerId"],
            "custom"
        );
    }
    #[test]
    fn editing_imported_official_profile_preserves_native_authentication() {
        let p = Profile {
            id: "cc:official".into(),
            agent: "codex".into(),
            name: "官方账号".into(),
            config: json!({"config":"model_provider='openai'\nmodel='gpt-example'","model":"gpt-example","apiKey":null}),
        };
        let mut s = settings(&p);
        assert!(s.official);
        assert!(p.summary(false).official_account);
        s.model = "gpt-real-id".into();
        s.name = "新的名称".into();
        let updated = make_profile(s, Some(&p)).unwrap();
        assert!(updated.is_official());
        assert_eq!(updated.config["model"], "gpt-real-id");
        assert!(updated.config["config"]
            .as_str()
            .unwrap()
            .contains("openai"));
        assert!(updated.config.get("apiKey").unwrap().is_null());
    }
    #[test]
    fn custom_kimi_endpoint_exposes_model_family_without_overwriting_connection_identity() {
        let config = json!({"providerId":"custom","baseUrl":"https://api.kimi.com/coding/","apiKey":"never-display-this"});
        let source = model_source(&config, "claude", Some("我的 Kimi"));
        assert_eq!(source["providerId"], "custom");
        assert_eq!(source["providerName"], "我的 Kimi");
        assert_eq!(source["modelFamily"], "kimi");
        assert!(!source.to_string().contains("never-display-this"));
        for endpoint in [
            "https://api.kimi.com.attacker.invalid/coding",
            "https://api.kimi.com/unrelated",
            "https://gateway.invalid/coding",
        ] {
            assert!(model_source(
                &json!({"providerId":"custom","baseUrl":endpoint}),
                "claude",
                None
            )["modelFamily"]
                .is_null());
        }
    }
    #[test]
    fn imported_alias_resolves_to_provider_id_and_deduplicates() {
        let c = json!({"model":"opus","env":{"ANTHROPIC_DEFAULT_OPUS_MODEL":"k3[1M]","ANTHROPIC_DEFAULT_SONNET_MODEL":"k3[1M]","ANTHROPIC_DEFAULT_HAIKU_MODEL":"k3"}});
        assert_eq!(
            real_claude_model(&c, Some("opus")).as_deref(),
            Some("k3[1M]")
        );
        assert_eq!(model_ids(&c), vec!["k3[1M]", "k3"]);
        assert!(!catalog_models(&c).to_string().contains("Opus"));
        assert_eq!(
            real_claude_model(&c, Some("kimi-k2.5")).as_deref(),
            Some("kimi-k2.5")
        );
    }
    #[test]
    fn manual_profile_roundtrip_preserves_model_and_secret_without_exposing_it() {
        let s:Settings=serde_json::from_value(json!({"name":"Kimi","agent":"claude","providerId":"kimi","plan":"coding","protocol":"anthropic","baseUrl":"https://api.kimi.com/coding/","model":"k3","models":["k3","kimi-for-coding"],"apiKey":"secret-for-test"})).unwrap();
        let p = make_profile(s, None).unwrap();
        assert_eq!(p.config["env"]["ANTHROPIC_MODEL"], "k3");
        assert!(p.config["env"]
            .get("ANTHROPIC_DEFAULT_OPUS_MODEL")
            .is_none());
        let safe = serde_json::to_string(&settings(&p)).unwrap();
        assert!(!safe.contains("secret-for-test"));
        let sealed = crate::credentials::seal(&p.config.to_string()).unwrap();
        assert_eq!(
            crate::credentials::open(&sealed).unwrap(),
            p.config.to_string()
        );
        #[cfg(windows)]
        assert!(!sealed.contains("secret-for-test"));
    }
    #[test]
    fn url_and_protocol_reject_credential_leaks_and_incompatible_engines() {
        assert!(validate_url("https://key:secret@example.com/v1").is_err());
        assert!(validate_url("http://remote.example/v1").is_err());
        assert!(validate_url("http://127.0.0.1:8888/v1").is_ok());
        assert!(validate_url("https://example.com/v1?key=secret").is_err());
    }
    #[test]
    fn new_provider_protocols_are_agent_specific_and_native_accounts_cannot_be_forged() {
        let settings = |agent: &str, protocol: &str| serde_json::from_value::<Settings>(json!({
            "name":"连接","agent":agent,"providerId":"custom","plan":protocol,"protocol":protocol,
            "baseUrl":"https://example.test/v1","model":"test-model","models":["test-model"],"apiKey":"test-key"
        })).unwrap();
        for agent in ["claude", "codex", "opencode", "pi"] {
            for protocol in ["anthropic", "responses", "chat"] {
                let allowed = match agent { "claude" => protocol == "anthropic", "codex" => protocol == "responses", _ => true };
                assert_eq!(make_profile(settings(agent, protocol), None).is_ok(), allowed, "{agent}/{protocol}");
            }
        }
        let mut fake_official = settings("claude", "anthropic"); fake_official.official = true;
        assert!(make_profile(fake_official, None).is_err());
        let legacy = Profile { id: "legacy-chat".into(), agent: "claude".into(), name: "旧连接".into(), config: json!({"protocol":"chat","baseUrl":"https://example.test/v1","apiKey":"test-key"}) };
        assert!(make_profile(settings("claude", "chat"), Some(&legacy)).is_ok());
    }
    #[test]
    fn all_bundled_responses_presets_generate_codex_api_routes_without_official_login() {
        for provider in bundled_catalog().as_array().unwrap() {
            for preset in provider["presets"].as_array().unwrap().iter()
                .filter(|p| p["protocol"] == "responses" && p["baseUrl"] != "") {
                let model = preset["models"][0].as_str().unwrap_or("model-from-account");
                let s = serde_json::from_value::<Settings>(json!({
                    "name":provider["name"], "agent":"codex", "providerId":provider["id"], "plan":preset["id"],
                    "protocol":"responses", "baseUrl":preset["baseUrl"], "model":model, "models":[model], "apiKey":"test-key"
                })).unwrap();
                let profile = make_profile(s, None).unwrap();
                assert!(!profile.is_official(), "{} / {}", provider["id"], preset["id"]);
                let table = toml::from_str::<toml::Table>(profile.config["config"].as_str().unwrap()).unwrap();
                let route = &table["model_providers"]["supercode_api"];
                assert_eq!(route["wire_api"].as_str(), Some("responses"));
                assert_eq!(route["base_url"].as_str().unwrap().trim_end_matches('/'), preset["baseUrl"].as_str().unwrap().trim_end_matches('/'));
                assert_eq!(route["env_key"].as_str(), Some("SUPERCODE_PROVIDER_API_KEY"));
                assert_eq!(route["requires_openai_auth"].as_bool(), Some(false));
                assert!(!profile.config["config"].as_str().unwrap().contains("test-key"));
                let roundtrip = settings(&profile);
                assert_eq!(roundtrip.agent, "codex");
                assert_eq!(roundtrip.provider_id, provider["id"].as_str().unwrap());
                assert_eq!(roundtrip.plan, preset["id"].as_str().unwrap());
            }
        }
    }
    #[test]
    fn hy3_search_requirement_is_endpoint_specific() {
        for (base, disabled) in [("https://tokenhub.tencentmaas.com/v1/", true), ("https://gateway.example/v1", false)] {
            let s = serde_json::from_value::<Settings>(json!({"name":"Hy3", "agent":"codex", "providerId":"tencent", "plan":"tokenhub-responses",
                "protocol":"responses", "baseUrl":base, "model":"hy3", "models":["hy3"], "apiKey":"test-key"})).unwrap();
            let profile = make_profile(s, None).unwrap();
            let table = toml::from_str::<toml::Table>(profile.config["config"].as_str().unwrap()).unwrap();
            assert_eq!(table.get("web_search").and_then(toml::Value::as_str), disabled.then_some("disabled"));
        }
    }
}
