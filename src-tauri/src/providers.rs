//! Provider presets and editable local API connections. Model IDs are passed through verbatim.
use crate::{ccswitch::Profile, AppState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

pub(crate) const MAGPIE_MODEL_LIMIT: usize = 10_000;
const MAGPIE_CATALOG_BYTES: usize = 8 * 1024 * 1024;

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

#[derive(Default)]
struct PreviewMetadata(std::collections::VecDeque<([u8; 32], std::time::Instant, Value)>);
impl PreviewMetadata {
    fn save(&mut self, fingerprint: [u8; 32], metadata: Value, now: std::time::Instant) {
        self.0.retain(|(key, at, _)| {
            key != &fingerprint
                && now.saturating_duration_since(*at) < std::time::Duration::from_secs(300)
        });
        if metadata.as_object().is_none_or(|value| value.is_empty()) {
            return;
        }
        self.0.push_back((fingerprint, now, metadata));
        while self.0.len() > 32 {
            self.0.pop_front();
        }
    }
    fn get(&mut self, fingerprint: &[u8; 32], now: std::time::Instant) -> Option<Value> {
        self.0.retain(|(_, at, _)| {
            now.saturating_duration_since(*at) < std::time::Duration::from_secs(300)
        });
        self.0
            .iter()
            .find(|(key, _, _)| key == fingerprint)
            .map(|(_, _, value)| value.clone())
    }
}
fn preview_metadata() -> &'static std::sync::Mutex<PreviewMetadata> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<PreviewMetadata>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}
fn metadata_fingerprint(settings: &Settings, secret: Option<&str>) -> Result<[u8; 32], String> {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    let base = api_base(&settings.base_url, &settings.protocol)?;
    for value in [
        settings.agent.as_str(),
        settings.protocol.as_str(),
        base.as_str(),
        secret.unwrap_or(""),
    ] {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    Ok(hash.finalize().into())
}
fn apply_preview_metadata(profile: &mut Profile) {
    if profile.is_official() {
        return;
    }
    let Ok(fingerprint) = metadata_fingerprint(&settings(profile), key(&profile.config)) else {
        return;
    };
    let Some(metadata) = preview_metadata()
        .lock()
        .ok()
        .and_then(|mut cache| cache.get(&fingerprint, std::time::Instant::now()))
    else {
        return;
    };
    merge_preview_metadata(profile, &metadata);
}
fn merge_preview_metadata(profile: &mut Profile, metadata: &Value) {
    let mut saved = profile.config["modelMetadata"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for id in model_ids(&profile.config) {
        if let Some(value) = metadata.get(&id) {
            saved.insert(id, value.clone());
        }
    }
    if !saved.is_empty() {
        profile.config["modelMetadata"] = json!(saved);
    }
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
                matched_plan = Some(if url.path().trim_end_matches('/') == "/coding/v1" {
                    "coding-responses"
                } else {
                    "coding"
                });
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
    let coding = id == "kimi" && matches!(plan, Some("coding" | "coding-responses"));
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

pub(crate) fn configured_base_url(config: &Value) -> Option<String> {
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
        &app,
        &agent,
        session_id.as_deref(),
        connection_id.as_deref(),
    )
    .await?;
    Ok(crate::chat_connection::model_source(route.as_ref(), &agent))
}

pub fn real_claude_model(config: &Value, requested: Option<&str>) -> Option<String> {
    let env = &config["env"];
    let value = requested
        .filter(|s| !s.trim().is_empty())
        .or(config["model"].as_str())
        .or(env["ANTHROPIC_MODEL"].as_str());
    if let Some(value) = value {
        let suffix = if value.eq_ignore_ascii_case("fable") {
            Some("FABLE")
        } else if value.eq_ignore_ascii_case("opus") {
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
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
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
    let mut seen = std::collections::HashSet::new();
    let mut add = |s: &str| {
        if !s.is_empty() && seen.insert(s.to_owned()) {
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
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
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
    json!({"data":model_ids(config).into_iter().map(|id| {
        let mut row = crate::model_context::catalog_row(config, &id);
        if row["displayName"].is_null() { row["displayName"] = json!(id); }
        row.as_object_mut().unwrap().extend(json!({"id":id,"model":id,"isDefault":Some(id.as_str())==default.as_deref()}).as_object().unwrap().clone());
        row
    }).collect::<Vec<_>>()})
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
fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("SuperCode/", env!("CARGO_PKG_VERSION")))
}
pub fn client() -> Result<reqwest::Client, String> {
    client_builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|_| "无法初始化 API 连接".into())
}
pub fn generation_client() -> Result<reqwest::Client, String> {
    // Model generations can legitimately exceed the editor's request deadline,
    // including long intervals without data. Keep the bridge's explicit
    // cancellation and bounded connects without a generation time limit.
    client_builder()
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
    read_json_bounded(response, 2 * 1024 * 1024).await
}
async fn read_json_bounded(response: reqwest::Response, limit: usize) -> Result<Value, String> {
    use futures_util::StreamExt;
    let status = response.status();
    if !status.is_success() {
        return Err(http_failure(status.as_u16()));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "读取 API 响应失败")?;
        if bytes.len() + chunk.len() > limit {
            return Err(format!("API 响应超过 {} MiB", limit / (1024 * 1024)));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "API 返回的内容不是受支持的 JSON".to_owned())
}
pub(crate) fn http_failure(status: u16) -> String {
    let reason = match status {
        401 => "认证失败，请检查 API Key",
        403 => "访问被拒绝，请检查模型和接口权限",
        404 => "请求接口或模型不存在，请检查地址和协议",
        429 => "请求受到限流或额度限制",
        500..=599 => "供应商服务或上游网关异常，不能据此判断 Key 或套餐无效",
        300..=399 => "接口返回了重定向，请填写直接的 API 地址",
        _ => "供应商拒绝了请求，请查看返回原因",
    };
    format!("API 返回 HTTP {status}：{reason}")
}
// Editor diagnostics may include the gateway's reason, but only with the
// effective credential available for redaction. Runtime bridge responses keep
// using read_json and never forward arbitrary gateway errors to the model.
async fn provider_json(response: reqwest::Response, secret: Option<&str>) -> Result<Value, String> {
    use futures_util::StreamExt;
    if response.status().is_success() {
        return read_json(response).await;
    }
    let mut message = http_failure(response.status().as_u16());
    let secrets: Vec<String> = secret
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .into_iter()
        .collect();
    let mut endpoint = response.url().clone();
    endpoint.set_query(None);
    endpoint.set_fragment(None);
    let endpoint = crate::log_redaction::text(endpoint.as_str(), &secrets);
    let request_id = ["x-request-id", "request-id", "cf-ray"]
        .iter()
        .find_map(|name| {
            response
                .headers()
                .get(*name)?
                .to_str()
                .ok()
                .filter(|value| value.len() <= 128 && !value.chars().any(char::is_control))
                .map(|value| crate::log_redaction::text(value, &secrets))
        });
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    // Do not expose partial credentials if the response exceeds this bound.
    // An oversized body is omitted in full rather than truncated before masking.
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            bytes.clear();
            break;
        };
        if bytes.len() + chunk.len() > 32 * 1024 {
            bytes.clear();
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    if let Some(reason) = provider_error_reason(&bytes, &secrets) {
        message.push_str(&format!("；供应商原因：{reason}"));
    }
    message.push_str(&format!("；请求地址：{endpoint}"));
    if let Some(request_id) = request_id {
        message.push_str(&format!("；请求 ID：{request_id}"));
    }
    Err(message)
}
fn provider_error_reason(bytes: &[u8], secrets: &[String]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() || text.starts_with('<') {
        return None;
    }
    let reason = if let Ok(value) = serde_json::from_str::<Value>(text) {
        let value = crate::log_redaction::value(value, secrets);
        let extracted = [
            &value["error"]["message"],
            &value["error"],
            &value["message"],
            &value["detail"],
            &value["msg"],
        ]
        .into_iter()
        .filter_map(Value::as_str)
        .find(|text| !text.trim().is_empty())?
        .to_owned();
        extracted
    } else {
        text.to_owned()
    };
    let reason = crate::log_redaction::text(&reason, secrets);
    let reason: String = reason
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(512)
        .collect();
    (!reason.trim().is_empty()).then(|| reason.trim().to_owned())
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
    let profile = find(&app, &id)?;
    ensure_editable(&profile)?;
    Ok(settings(&profile))
}

pub(crate) fn ensure_editable(profile: &Profile) -> Result<(), String> {
    if profile.config["providerId"] == "magpie" || crate::magpie::managed(profile) {
        return Err("Magpie 连接由 Magpie 管理，只支持添加或移除，请在 Magpie 中调整供应商和模型。".into());
    }
    Ok(())
}

pub(crate) fn make_profile(mut s: Settings, previous: Option<&Profile>) -> Result<Profile, String> {
    s.name = s.name.trim().into();
    s.model = s.model.trim().into();
    if s.name.is_empty() || s.name.len() > 100 {
        return Err("连接名称需要 1–100 个 UTF-8 字节".into());
    }
    if s.model.is_empty() || s.model.len() > 200 || s.model.chars().any(char::is_control) {
        return Err("请填写真实模型 ID，长度不超过 200 字节".into());
    }
    validate_protocol(&s, previous)?;
    if s.official && previous.is_none() {
        return Err("官方账号请通过官方登录添加".into());
    }
    if previous.is_some_and(|p| p.agent != s.agent) {
        return Err("编辑连接时不能更换 Agent，请新增连接".into());
    }
    let secret = s
        .api_key
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| previous.and_then(|p| key(&p.config)))
        .or_else(|| local_magpie_key(&s))
        .unwrap_or("")
        .trim();
    if secret.len() > 8192 || secret.chars().any(char::is_control) {
        return Err("API Key 格式无效".into());
    }
    let mut models = vec![s.model.clone()];
    let model_limit = if s.provider_id == "magpie" { MAGPIE_MODEL_LIMIT } else { 200 };
    if s.models.len() > model_limit {
        return Err(format!("单个连接最多保存 {model_limit} 个模型"));
    }
    let mut seen = std::collections::HashSet::from([s.model.clone()]);
    for m in s.models {
        let m = m.trim();
        if m.len() > 200 || m.chars().any(char::is_control) {
            return Err("模型 ID 格式无效".into());
        }
        if !m.is_empty() && seen.insert(m.to_owned()) {
            models.push(m.to_owned());
        }
    }
    if s.provider_id == "magpie" && models.len() > model_limit {
        return Err(format!("单个连接最多保存 {model_limit} 个模型"));
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
    // Keep the user's/imported base visible and persistent. SDK/API version
    // paths belong only to request construction, not to the saved connection.
    let base = validate_url(&s.base_url)?;
    let request_base = api_base(&base, &s.protocol)?;
    let mut config = json!({"providerId":s.provider_id,"plan":s.plan,"protocol":s.protocol,"baseUrl":base,"model":s.model,"models":models,"source":"manual"});
    // Editing a credential or model list must retain native model capabilities
    // and request headers imported from CC Switch on the same API route.
    if let Some(previous) = previous.filter(|p| {
        p.config["protocol"] == s.protocol
            && p.config["baseUrl"]
                .as_str()
                .and_then(|old| api_base(old, &s.protocol).ok())
                .as_deref()
                == Some(request_base.as_str())
    }) {
        for field in ["opencodeProvider", "piProvider"] {
            if let Some(native) = previous.config.get(field) {
                config[field] = native.clone();
            }
        }
        // API capacities can vary by tenant or credential. Preserve imported
        // native declarations, but refresh fetched capacities after a Key edit.
        if key(&previous.config).unwrap_or("").trim() == secret {
            if let Some(metadata) = previous.config.get("modelMetadata") {
                config["modelMetadata"] = metadata.clone();
            }
        }
    }
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
                .as_array()?
                .iter()
                .find(|p| {
                    p["id"] == s.plan
                        && p["protocol"] == "responses"
                        && p["baseUrl"].as_str().is_some_and(|url| {
                            url.trim_end_matches('/') == base.trim_end_matches('/')
                        })
                })
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
fn validate_protocol(s: &Settings, previous: Option<&Profile>) -> Result<(), String> {
    if !matches!(
        s.protocol.as_str(),
        "anthropic" | "chat" | "responses" | "gemini"
    ) {
        return Err("API 协议无效".into());
    }
    let supported = match s.agent.as_str() {
        "claude" => {
            s.protocol == "anthropic"
                || s.protocol == "chat"
                    && previous
                        .is_some_and(|p| p.agent == "claude" && p.config["protocol"] == s.protocol)
        }
        "codex" => s.protocol == "responses",
        "gemini" => s.protocol == "gemini",
        "opencode" | "pi" => true,
        _ => false,
    };
    if !supported {
        return Err("Claude Code 需要 Anthropic Messages；Codex 需要 OpenAI Responses；Gemini CLI 需要 Gemini API".into());
    }
    if previous.is_some_and(|p| p.agent != s.agent) {
        return Err("编辑连接时不能更换 Agent，请新增连接".into());
    }
    Ok(())
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
    if settings.provider_id == "magpie" {
        return Err("请使用顶部 Magpie 菜单直接添加，无需填写 API 配置。".into());
    }
    if let Some(profile) = &previous { ensure_editable(profile)?; }
    if previous.as_ref().is_some_and(|p| p.agent != settings.agent) {
        return Err("更换连接的 Agent 请添加新连接，已有聊天的引擎不能被修改".into());
    }
    let mut profile = make_profile(settings, previous.as_ref())?;
    apply_preview_metadata(&mut profile);
    let state = app.state::<AppState>();
    // Existing chats keep their connection; each runtime rechecks its route
    // before the next turn. Unrelated changes need no process shutdown.
    let was_active = state
        .store
        .active_profile(&profile.agent)?
        .is_some_and(|p| p.id == profile.id);
    state.store.save_profile(&profile, previous.as_ref())?;
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
    let profile = find(&app, &id)?;
    state.store.delete_profiles(&profile.agent, &[id])?;
    let _ = app.emit("workspace-updated", ());
    Ok(())
}
#[tauri::command]
pub async fn delete_provider_profiles(
    agent: String,
    ids: Vec<String>,
    app: AppHandle,
) -> Result<(), String> {
    app.state::<AppState>()
        .store
        .delete_profiles(&agent, &ids)?;
    let _ = app.emit("workspace-updated", ());
    Ok(())
}

// Magpie accepts a public placeholder on loopback. Native CLIs still need a
// nonempty key to choose API authentication instead of their own sign-in.
fn local_magpie_key(s: &Settings) -> Option<&'static str> {
    (s.provider_id == "magpie"
        && reqwest::Url::parse(&s.base_url).ok().is_some_and(|url| {
            matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        }))
    .then_some("magpie")
}

fn listing_secret<'a>(
    s: &'a Settings,
    previous: Option<&'a Profile>,
) -> Result<Option<&'a str>, String> {
    validate_protocol(s, previous)?;
    if previous.is_some_and(Profile::is_official) || s.official {
        return Err("官方登录连接请使用已保存连接的模型列表".into());
    }
    let secret = s
        .api_key
        .as_deref()
        .filter(|key| !key.trim().is_empty())
        .or_else(|| previous.and_then(|profile| key(&profile.config)))
        .or_else(|| local_magpie_key(s))
        .map(str::trim);
    if secret.is_some_and(|secret| secret.len() > 8192 || secret.chars().any(char::is_control)) {
        return Err("API Key 格式无效".into());
    }
    Ok(secret)
}

#[tauri::command]
pub async fn fetch_provider_models_for_settings(
    settings: Settings,
    app: AppHandle,
) -> Result<Vec<String>, String> {
    let previous = settings
        .id
        .as_deref()
        .map(|id| find(&app, id))
        .transpose()?;
    let secret = listing_secret(&settings, previous.as_ref())?;
    // Unsaved settings are a preview. Never mutate a saved route or persist its
    // metadata using the result of a request to a different draft endpoint.
    let fingerprint = metadata_fingerprint(&settings, secret)?;
    let (models, metadata) = fetch_models(&settings, secret).await?;
    if let Ok(mut cache) = preview_metadata().lock() {
        cache.save(fingerprint, metadata, std::time::Instant::now());
    }
    Ok(models)
}

pub(crate) fn gemini_api_base(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if ["v1", "v1beta", "v1alpha"]
        .iter()
        .any(|version| base.ends_with(&format!("/{version}")))
    {
        base.to_owned()
    } else {
        format!("{base}/v1beta")
    }
}
fn api_base(base: &str, protocol: &str) -> Result<String, String> {
    let base = validate_url(base)?;
    Ok(if protocol == "gemini" {
        gemini_api_base(&base)
    } else {
        base
    })
}

fn protocol_headers(
    request: reqwest::RequestBuilder,
    protocol: &str,
    secret: Option<&str>,
) -> reqwest::RequestBuilder {
    let request = if protocol == "anthropic" {
        request.header("anthropic-version", "2023-06-01")
    } else {
        request
    };
    match (protocol, secret) {
        ("anthropic", Some(secret)) => request.header("x-api-key", secret),
        ("gemini", Some(secret)) => request.header("x-goog-api-key", secret),
        (_, Some(secret)) => request.bearer_auth(secret),
        (_, None) => request,
    }
}

fn model_listing_endpoint(s: &Settings) -> Result<(String, &str), String> {
    let base = api_base(&s.base_url, &s.protocol)?;
    if s.provider_id == "magpie" {
        // The OpenAI-shaped catalog also carries names, reasoning levels and
        // context limits; Magpie's Gemini list omits those capabilities.
        let root = ["/v1beta", "/v1alpha", "/v1"]
            .into_iter()
            .find_map(|suffix| base.strip_suffix(suffix))
            .unwrap_or(&base);
        return Ok((format!("{root}/v1/models"), "chat"));
    }
    let mut parsed = reqwest::Url::parse(&base).map_err(|_| "API 地址无效")?;
    // DeepSeek's Anthropic prefix serves Messages, while its model catalog is
    // the same-origin OpenAI-compatible /models endpoint with Bearer auth.
    // Match the official HTTPS origin and prefix, including imported custom profiles.
    if s.protocol == "anthropic"
        && parsed.scheme() == "https"
        && parsed.host_str() == Some("api.deepseek.com")
        && parsed.port_or_known_default() == Some(443)
        && matches!(parsed.path(), "/anthropic" | "/anthropic/v1")
    {
        parsed.set_path("/models");
        return Ok((parsed.to_string(), "chat"));
    }
    let url = if s.protocol == "anthropic" && !base.ends_with("/v1") {
        format!("{base}/v1/models")
    } else {
        format!("{base}/models")
    };
    Ok((url, &s.protocol))
}

pub(crate) async fn fetch_models(s: &Settings, secret: Option<&str>) -> Result<(Vec<String>, Value), String> {
    let (url, listing_protocol) = model_listing_endpoint(s)?;
    let client = if s.provider_id == "magpie" {
        client_builder().timeout(std::time::Duration::from_secs(10)).build()
            .map_err(|_| "无法初始化 Magpie 连接")?
    } else { client()? };
    let mut models = std::collections::BTreeSet::new();
    let mut metadata = json!({});
    let mut page_token: Option<String> = None;
    let mut tokens = std::collections::HashSet::new();
    let model_limit = if s.provider_id == "magpie" { MAGPIE_MODEL_LIMIT } else { 200 };
    // Keep the full gateway directory within explicit byte/count bounds. A
    // larger gateway fails visibly instead of silently losing its later models.
    for _ in 0..4 {
        let mut request = client.get(&url);
        if listing_protocol == "gemini" {
            request = request.query(&[("pageSize", "200")]);
            if let Some(token) = &page_token {
                request = request.query(&[("pageToken", token)]);
            }
        }
        let response = protocol_headers(request, listing_protocol, secret)
                .send()
                .await
                .map_err(|_| if s.provider_id == "magpie" { "无法连接 Magpie 本机网关，请打开 Magpie 检查运行状态" } else { "连接失败或超时，可手动填写模型 ID" })?;
        let value = if s.provider_id == "magpie" && response.status().is_success() {
            read_json_bounded(response, MAGPIE_CATALOG_BYTES).await?
        } else { provider_json(response, secret).await? };
        let rows = if listing_protocol == "gemini" {
            value["models"]
                .as_array()
                .or_else(|| value["data"].as_array())
        } else {
            value["data"].as_array()
        };
        let mut normalized = Vec::new();
        for row in rows.into_iter().flatten() {
            if !row.is_object() {
                continue;
            }
            if listing_protocol == "gemini"
                && row["supportedGenerationMethods"]
                    .as_array()
                    .is_some_and(|methods| {
                        !methods.iter().any(|method| method == "generateContent")
                    })
            {
                continue;
            }
            let raw = row["id"].as_str().or_else(|| row["name"].as_str());
            let Some(id) = raw
                .map(|id| {
                    if listing_protocol == "gemini" {
                        id.strip_prefix("models/").unwrap_or(id)
                    } else {
                        id
                    }
                })
                .filter(|id| {
                    !id.is_empty() && id.len() <= 200 && !id.chars().any(char::is_control)
                })
            else {
                continue;
            };
            if models.len() >= model_limit && !models.contains(id) {
                if s.provider_id == "magpie" {
                    return Err(format!("Magpie 目录超过 {model_limit} 个模型，未截取部分目录，请在 Magpie 中限制可见模型"));
                }
                continue;
            }
            models.insert(id.to_owned());
            let mut normalized_row = row.clone();
            normalized_row["id"] = json!(id);
            normalized.push(normalized_row);
        }
        let normalized = json!({"data": normalized});
        let safe = if s.provider_id == "magpie" {
            crate::model_context::api_catalog_with_limit(&normalized, model_limit)
        } else { crate::model_context::api_catalog(&normalized) };
        // A later page can repeat a model with fewer fields. Keep previously
        // discovered capabilities instead of replacing the whole entry.
        for (id, fields) in safe.as_object().unwrap() {
            let mut saved = metadata[id].as_object().cloned().unwrap_or_default();
            saved.extend(fields.as_object().unwrap().clone());
            metadata[id] = json!(saved);
        }
        if listing_protocol != "gemini" || models.len() >= model_limit {
            break;
        }
        let Some(token) = value["nextPageToken"]
            .as_str()
            .filter(|token| !token.is_empty())
        else {
            break;
        };
        if token.len() > 4096
            || token.chars().any(char::is_control)
            || !tokens.insert(token.to_owned())
        {
            return Err("供应商返回的模型分页标记无效".into());
        }
        page_token = Some(token.to_owned());
    }
    if models.is_empty() {
        return Err(if s.provider_id == "magpie" { "Magpie 尚未提供可用模型，请在 Magpie 中添加或启用供应商后重试。" } else { "供应商没有返回模型列表，请手动添加模型 ID" }.into());
    }
    Ok((models.into_iter().collect(), metadata))
}

/// Read the external gateway only when the model picker requests its catalog.
/// No Magpie process, account configuration or upstream secret is managed here.
pub(crate) async fn magpie_catalog(profile: &Profile) -> (Value, Option<(Vec<String>, Value)>) {
    let mut catalog = catalog_models(&profile.config);
    let settings = settings(profile);
    match fetch_models(&settings, key(&profile.config).or_else(|| local_magpie_key(&settings))).await {
        Ok((models, metadata)) => {
            let mut config = profile.config.clone();
            config["models"] = json!(models);
            config["modelMetadata"] = metadata.clone();
            catalog = catalog_models(&config);
            if let Some(rows) = catalog["data"].as_array_mut() {
                let visible: std::collections::HashSet<_> = models.iter().map(String::as_str).collect();
                rows.retain(|row| row["model"].as_str().is_some_and(|id| visible.contains(id)));
            }
            if real_claude_model(&profile.config, None).is_some_and(|selected| !models.contains(&selected)) {
                catalog["warning"] = json!("上次使用的模型已不在 Magpie 当前目录中，请重新选择模型。");
            }
            return (catalog, Some((models, metadata)));
        }
        Err(error) => {
            catalog["warning"] = json!(format!("无法获取 Magpie 模型，请确认网关已运行且地址与网关 Key 正确。当前保留已保存的模型。{error}"));
        }
    }
    (catalog, None)
}
#[tauri::command]
pub async fn fetch_provider_models(id: String, app: AppHandle) -> Result<Vec<String>, String> {
    let p = find(&app, &id)?;
    if p.is_official() {
        let value =
            crate::commands::list_models(Some(p.agent), None, Some(p.id), Some(true), app).await?;
        if let Some(warning) = value["warning"].as_str() {
            return Err(warning.into());
        }
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
    let (models, metadata) = fetch_models(&s, key(&p.config)).await?;
    if !metadata.as_object().unwrap().is_empty() {
        app.state::<AppState>()
            .store
            .save_model_metadata(&p, &metadata)?;
    }
    Ok(models)
}
#[tauri::command]
pub async fn test_provider_connection(id: String, app: AppHandle) -> Result<Value, String> {
    let p = find(&app, &id)?;
    let s = settings(&p);
    if s.official {
        let status = app
            .state::<crate::accounts::Accounts>()
            .get_for(&p.agent, p.account_id(), true, &app)
            .await?;
        if status["loggedIn"] != true {
            return Err("尚未检测到官方登录，请在供应商页登录官方账号".into());
        }
        return Ok(json!({"ok":true,"model":s.model,"verifiedBy":"officialAccountStatus"}));
    }
    if native_connection_test(&s) {
        return test_engine_connection(&app, &p).await;
    }
    let (url, body) = connection_probe(&s)?;
    let request = protocol_headers(client()?.post(url).json(&body), &s.protocol, key(&p.config));
    let value = provider_json(
        request
            .send()
            .await
            .map_err(|_| "连接失败或超时，请检查网络与 API 地址")?,
        key(&p.config),
    )
    .await?;
    if value.get("error").is_some() {
        return Err("供应商返回错误，请检查模型权限和套餐余额".into());
    }
    if !valid_probe_response(&s.protocol, &value) {
        return Err("API 响应格式与所选协议不匹配，或模型未返回测试回复".into());
    }
    Ok(json!({"ok":true,"model":value["model"].as_str().unwrap_or(&s.model),"protocol":s.protocol}))
}
fn connection_probe(s: &Settings) -> Result<(String, Value), String> {
    let base = api_base(&s.base_url, &s.protocol)?;
    Ok(match s.protocol.as_str() {
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
        "gemini" => {
            let mut url = reqwest::Url::parse(&base).map_err(|_| "API 地址无效")?;
            url.path_segments_mut()
                .map_err(|_| "API 地址无效")?
                .push("models")
                .push(&format!(
                    "{}:generateContent",
                    s.model.strip_prefix("models/").unwrap_or(&s.model)
                ));
            (
                url.to_string(),
                json!({"contents":[{"role":"user","parts":[{"text":"Reply only OK."}]}],"generationConfig":{"maxOutputTokens":32}}),
            )
        }
        _ => (
            format!("{base}/chat/completions"),
            json!({"model":s.model,"max_tokens":32,"messages":[{"role":"user","content":"Reply only OK."}]}),
        ),
    })
}
fn valid_probe_response(protocol: &str, value: &Value) -> bool {
    let text = |value: &Value| value.as_str().is_some_and(|value| !value.trim().is_empty());
    let parts = |value: &Value| {
        value
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| text(&part["text"])))
    };
    match protocol {
        "anthropic" => parts(&value["content"]),
        "responses" => value["output"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| parts(&item["content"]))),
        "gemini" => value["candidates"].as_array().is_some_and(|candidates| {
            candidates
                .iter()
                .any(|candidate| parts(&candidate["content"]["parts"]))
        }),
        "chat" => value["choices"].as_array().is_some_and(|choices| {
            choices.iter().any(|choice| {
                text(&choice["message"]["content"]) || parts(&choice["message"]["content"])
            })
        }),
        _ => false,
    }
}
async fn test_engine_connection(app: &AppHandle, profile: &Profile) -> Result<Value, String> {
    // Resolve the selected engine first, so missing Pi/OpenCode is reported as
    // an installation problem rather than an unrelated Claude protocol error.
    let launch = plan_test_launch(profile, |agent| crate::agents::resolve(app, agent))?;
    crate::runtime_diagnostics::register_secrets(app, &profile.config);
    let result = if profile.agent != "claude" {
        crate::native_agents::test_provider_connection(app, profile, &launch).await
    } else {
        test_claude_plan(app, profile, &launch).await
    };
    result.map_err(|error| {
        crate::runtime_diagnostics::message(
            app,
            None,
            &profile.agent,
            "error",
            "provider_connection_test_failed",
            &error,
        );
        let mut secrets = Vec::new();
        crate::log_redaction::collect(&profile.config, &mut secrets, 0);
        format!(
            "{} 连接测试失败：{}",
            crate::agents::name(&profile.agent),
            crate::log_redaction::text(&error, &secrets)
        )
    })
}
fn plan_test_launch(
    profile: &Profile,
    resolve: impl FnOnce(&str) -> Result<crate::agents::Launch, String>,
) -> Result<crate::agents::Launch, String> {
    let launch = resolve(&profile.agent)?;
    validate_plan_test_engine(&profile.agent, &settings(profile).protocol)?;
    Ok(launch)
}
fn subscription_plan(plan: &str) -> bool {
    matches!(plan.split('-').next(), Some("coding" | "token"))
}
fn native_connection_test(settings: &Settings) -> bool {
    // Gemini CLI gateways may support only the CLI's streaming request shape
    // and SDK headers. Test the same engine used by the chat, for every plan.
    settings.agent == "gemini" || subscription_plan(&settings.plan)
}
fn validate_plan_test_engine(agent: &str, protocol: &str) -> Result<(), String> {
    match (agent, protocol) {
        ("claude", "anthropic")
        | ("codex", "responses")
        | ("opencode" | "pi", "anthropic" | "chat" | "responses" | "gemini")
        | ("gemini", "gemini") => Ok(()),
        ("claude", _) => Err(
            "Claude Code 的套餐测试需要 Anthropic Messages 协议，请检查此连接的 API 协议".into(),
        ),
        ("codex", _) => {
            Err("Codex 的套餐测试需要 OpenAI Responses 协议，请检查此连接的 API 协议".into())
        }
        _ => Err("此连接的引擎或 API 协议无效，请检查连接设置".into()),
    }
}
async fn test_claude_plan(
    app: &AppHandle,
    profile: &Profile,
    launch: &crate::agents::Launch,
) -> Result<Value, String> {
    let model = real_claude_model(&profile.config, None).ok_or("请填写模型 ID")?;
    let mut cmd = launch.command(&[
        "--print",
        "--output-format",
        "json",
        "--tools",
        "",
        "--max-turns",
        "1",
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--settings",
        "{\"disableAllHooks\":true,\"autoCompactEnabled\":true}",
        "Reply only OK.",
    ]);
    for key in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
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
    let context = crate::model_context::configure_claude(&mut cmd, &profile.config, Some(&model));
    if let Some(launch_model) = context.launch_model {
        cmd.args(["--model", &launch_model]);
    }
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| "无法启动 Claude 连接测试")?;
    let _job = crate::process::JobGuard::attach(&child)?;
    if let Some(stderr) = child.stderr.take() {
        let log_app = app.clone();
        tokio::spawn(async move {
            crate::runtime_diagnostics::stderr(Some(log_app), None, "claude", stderr).await;
        });
    }
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
        return Err(value["result"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("模型未返回测试回复，请检查模型 ID、专属 Key 与套餐权限")
            .to_owned());
    }
    Ok(
        json!({"ok":true,"model":value["modelUsage"].as_object().and_then(|o|o.keys().next()).map(String::as_str).unwrap_or(&model),"protocol":"anthropic","verifiedBy":"Claude Code"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn model_peer(responses: Vec<Value>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut block = [0; 1024];
                    let count = stream.read(&mut block).await.unwrap();
                    assert!(count > 0 && bytes.len() < 16 * 1024);
                    bytes.extend_from_slice(&block[..count]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8(bytes).unwrap());
                let body = response.to_string();
                let wire = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(wire.as_bytes()).await.unwrap();
            }
            requests
        });
        (format!("http://{address}"), task)
    }
    fn draft_settings(agent: &str, protocol: &str, base: &str) -> Settings {
        serde_json::from_value(json!({"name":"","agent":agent,"providerId":"custom","plan":"standard","protocol":protocol,"baseUrl":base,"model":"","models":[]})).unwrap()
    }
    #[tokio::test]
    async fn magpie_large_directory_keeps_models_and_metadata_beyond_old_limits() {
        let rows: Vec<_> = (0..5000).map(|i| json!({"id":format!("vendor/model-{i:05}"),"display_name":format!("模型 {i}"),
            "context_length":262144,"supported_reasoning_levels":[{"effort":"high"}],"unused": "x".repeat(400)})).collect();
        let response = json!({"data":rows});
        assert!(response.to_string().len() > 2 * 1024 * 1024);
        let (base, peer) = model_peer(vec![response]).await;
        let mut settings = draft_settings("pi", "responses", &format!("{base}/v1"));
        settings.provider_id = "magpie".into();
        settings.name = "Magpie".into();
        let (models, metadata) = fetch_models(&settings, listing_secret(&settings, None).unwrap()).await.unwrap();
        assert_eq!(models.len(), 5000);
        assert_eq!(metadata.as_object().unwrap().len(), 5000);
        settings.model = "vendor/model-04999".into();
        settings.models = models;
        let mut profile = make_profile(settings, None).unwrap();
        merge_preview_metadata(&mut profile, &metadata);
        let catalog = catalog_models(&profile.config);
        assert_eq!(catalog["data"].as_array().unwrap().len(), 5000);
        let last = catalog["data"].as_array().unwrap().iter().find(|row| row["model"] == "vendor/model-04999").unwrap();
        assert_eq!(last["displayName"], "模型 4999");
        assert_eq!(last["supportedReasoningEfforts"][0]["reasoningEffort"], "high");
        assert!(metadata["vendor/model-04999"].get("unused").is_none());
        peer.await.unwrap();
    }
    #[tokio::test]
    async fn magpie_oversized_directory_fails_instead_of_silently_returning_a_partial_list() {
        let rows: Vec<_> = (0..=MAGPIE_MODEL_LIMIT).map(|i| json!({"id":format!("v/m{i}")})).collect();
        let (base, peer) = model_peer(vec![json!({"data":rows})]).await;
        let mut settings = draft_settings("pi", "responses", &format!("{base}/v1"));
        settings.provider_id = "magpie".into();
        assert!(fetch_models(&settings, Some("magpie")).await.unwrap_err().contains("未截取部分目录"));
        peer.await.unwrap();
    }
    #[tokio::test]
    async fn magpie_models_use_enriched_catalog_and_keep_literal_ids_for_every_engine() {
        for (agent, protocol, suffix) in [("claude","anthropic",""),("codex","responses","/v1"),
            ("opencode","chat","/v1"),("pi","anthropic","/v1"),("gemini","gemini","/v1beta")] {
            let response = json!({"data":[{"id":"fixture/test-reasoner","display_name":"Magpie 测试模型",
                "context_length":262144,"reasoning":true,"supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}]},
                {"id":"models/literal[1m]","reasoning":false}]});
            let (base, peer) = model_peer(vec![response.clone(),response]).await;
            let mut settings = draft_settings(agent, protocol, &format!("{base}{suffix}"));
            settings.provider_id = "magpie".into();
            settings.name = "Magpie".into();
            let secret = listing_secret(&settings, None).unwrap();
            assert_eq!(secret, Some("magpie"));
            let (models, metadata) = fetch_models(&settings, secret).await.unwrap();
            assert_eq!(models, ["fixture/test-reasoner","models/literal[1m]"]);
            settings.model = models[0].clone();
            settings.models = models;
            let mut profile = make_profile(settings, None).unwrap();
            merge_preview_metadata(&mut profile, &metadata);
            assert_eq!(key(&profile.config), Some("magpie"));
            let (catalog, live_metadata) = magpie_catalog(&profile).await;
            assert_eq!(live_metadata.as_ref().map(|(_, metadata)| metadata), Some(&metadata));
            assert_eq!(catalog["data"][0]["model"], "fixture/test-reasoner");
            assert_eq!(catalog["data"][0]["displayName"], "Magpie 测试模型");
            assert_eq!(catalog["data"][0]["contextWindow"], 262144);
            assert_eq!(catalog["data"][0]["supportedReasoningEfforts"], json!([{"reasoningEffort":"low"},{"reasoningEffort":"high"}]));
            let requests = peer.await.unwrap();
            assert!(requests.iter().all(|request| request.starts_with("GET /v1/models HTTP/1.1")
                && request.to_ascii_lowercase().contains("authorization: bearer magpie")));
        }
    }
    #[test]
    fn magpie_placeholder_is_loopback_only_and_never_replaces_a_gateway_key() {
        for base in ["https://gateway.example/v1", "https://127.0.0.1.attacker.example/v1"] {
            let mut settings = draft_settings("codex", "responses", base);
            settings.provider_id = "magpie".into();
            assert_eq!(listing_secret(&settings, None).unwrap(), None);
            settings.api_key = Some("gateway-key".into());
            assert_eq!(listing_secret(&settings, None).unwrap(), Some("gateway-key"));
        }
        let mut settings = draft_settings("codex", "responses", "http://127.0.0.1:3425/v1");
        assert_eq!(listing_secret(&settings, None).unwrap(), None);
        settings.provider_id = "magpie".into();
        settings.api_key = Some("my-gateway-key".into());
        assert_eq!(listing_secret(&settings, None).unwrap(), Some("my-gateway-key"));
    }
    #[tokio::test]
    async fn magpie_refresh_lists_new_models_without_reselecting_a_removed_default() {
        let (base, peer) = model_peer(vec![json!({"data":[{"id":"fixture/new-model","display_name":"新模型"}]})]).await;
        let mut settings = draft_settings("pi", "responses", &format!("{base}/v1"));
        settings.provider_id = "magpie".into();
        settings.name = "Magpie".into();
        settings.model = "fixture/old-model".into();
        let profile = make_profile(settings, None).unwrap();
        let (catalog, refreshed) = magpie_catalog(&profile).await;
        assert_eq!(catalog["data"].as_array().unwrap().len(), 1);
        assert_eq!(catalog["data"][0]["model"], "fixture/new-model");
        assert_eq!(catalog["data"][0]["isDefault"], false);
        assert!(catalog["warning"].as_str().unwrap().contains("重新选择"));
        assert_eq!(profile.config["model"], "fixture/old-model");
        assert_eq!(refreshed.unwrap().0, ["fixture/new-model"]);
        peer.await.unwrap();
    }
    #[tokio::test]
    async fn magpie_offline_retains_saved_models_and_reports_reconnect_hint() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let mut settings = draft_settings("claude", "anthropic", &base);
        settings.provider_id = "magpie".into();
        settings.model = "fixture/saved".into();
        settings.name = "Magpie".into();
        let profile = make_profile(settings, None).unwrap();
        let original = profile.config.clone();
        let (catalog, metadata) = magpie_catalog(&profile).await;
        assert_eq!(catalog["data"][0]["model"], "fixture/saved");
        assert!(catalog["warning"].as_str().unwrap().contains("网关已运行"));
        assert!(metadata.is_none());
        assert_eq!(profile.config, original);
    }
    #[test]
    fn deepseek_anthropic_models_use_official_catalog_and_bearer_auth() {
        for base in [
            "https://api.deepseek.com/anthropic",
            "https://api.deepseek.com/anthropic/",
            "https://api.deepseek.com/anthropic/v1",
            "https://API.DEEPSEEK.COM:443/anthropic",
        ] {
            let settings = draft_settings("claude", "anthropic", base);
            let (url, protocol) = model_listing_endpoint(&settings).unwrap();
            assert_eq!(url, "https://api.deepseek.com/models");
            let request = protocol_headers(client().unwrap().get(url), protocol, Some("fixture-secret"))
                .build().unwrap();
            assert_eq!(request.headers()["authorization"], "Bearer fixture-secret");
            assert!(!request.headers().contains_key("x-api-key"));
            assert!(!request.headers().contains_key("anthropic-version"));
            // Model discovery must not rewrite the saved Messages route.
            assert_eq!(settings.base_url, base);
            assert_eq!(settings.protocol, "anthropic");
        }
    }
    #[test]
    fn deepseek_catalog_override_does_not_rewrite_other_origins_or_protocols() {
        for (base, protocol, expected) in [
            ("https://api.anthropic.com", "anthropic", "https://api.anthropic.com/v1/models"),
            ("https://gateway.example/anthropic", "anthropic", "https://gateway.example/anthropic/v1/models"),
            ("https://api.deepseek.com.attacker.example/anthropic", "anthropic", "https://api.deepseek.com.attacker.example/anthropic/v1/models"),
            ("https://api.deepseek.com:8443/anthropic", "anthropic", "https://api.deepseek.com:8443/anthropic/v1/models"),
            ("https://api.deepseek.com/proxy/anthropic", "anthropic", "https://api.deepseek.com/proxy/anthropic/v1/models"),
            ("https://api.deepseek.com/v1", "chat", "https://api.deepseek.com/v1/models"),
            ("https://api.deepseek.com", "responses", "https://api.deepseek.com/models"),
        ] {
            let settings = draft_settings("pi", protocol, base);
            assert_eq!(model_listing_endpoint(&settings).unwrap(), (expected.to_owned(), protocol));
        }
    }
    #[tokio::test]
    async fn unsaved_gemini_lists_use_native_headers_normalize_names_and_follow_pages() {
        let (base, peer) = model_peer(vec![
            json!({"models":[{"name":"models/gemini-fixture","inputTokenLimit":1048576},null,{"name":""}],"nextPageToken":"a+b/= marker"}),
            json!({"models":[{"name":"models/gemini-fixture"},{"name":"models/gemini-other"}]}),
        ]).await;
        let mut settings = draft_settings("pi", "gemini", &format!("{base}/v1beta"));
        settings.api_key = Some("fixture-secret".into());
        let secret = listing_secret(&settings, None).unwrap();
        let (models, metadata) = fetch_models(&settings, secret).await.unwrap();
        assert_eq!(models, ["gemini-fixture", "gemini-other"]);
        assert_eq!(metadata["gemini-fixture"]["contextWindow"], 1048576);
        let requests = peer.await.unwrap();
        assert!(requests[0].starts_with("GET /v1beta/models?pageSize=200 HTTP/1.1"));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("x-goog-api-key: fixture-secret"));
        assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
        assert!(!requests[0]
            .lines()
            .next()
            .unwrap()
            .contains("fixture-secret"));
        assert!(requests[1]
            .lines()
            .next()
            .unwrap()
            .contains("pageToken=a%2Bb%2F%3D+marker"));
    }
    #[tokio::test]
    async fn model_discovery_does_not_require_key_model_or_connection_name() {
        for (agent, protocol, path) in [
            ("opencode", "chat", "/v1/models"),
            ("pi", "responses", "/v1/models"),
            ("pi", "anthropic", "/v1/models"),
            ("gemini", "gemini", "/v1beta/models?pageSize=200"),
        ] {
            let response = if protocol == "gemini" {
                json!({"models":[{"name":"models/m"}]})
            } else {
                json!({"data":[{"id":"m"},{"id":"m"}]})
            };
            let (base, peer) = model_peer(vec![response]).await;
            let settings = draft_settings(
                agent,
                protocol,
                &if protocol == "gemini" || protocol == "anthropic" {
                    base
                } else {
                    format!("{base}/v1")
                },
            );
            assert!(listing_secret(&settings, None).unwrap().is_none());
            assert_eq!(fetch_models(&settings, None).await.unwrap().0, ["m"]);
            let requests = peer.await.unwrap();
            assert!(
                requests[0].starts_with(&format!("GET {path} HTTP/1.1")),
                "{}",
                requests[0]
            );
            assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
            assert!(!requests[0].to_ascii_lowercase().contains("x-goog-api-key:"));
            if protocol == "anthropic" {
                assert!(requests[0].contains("anthropic-version: 2023-06-01"));
            }
        }
    }
    #[tokio::test]
    async fn repeated_gemini_page_tokens_are_rejected_and_invalid_ids_ignored() {
        let (base, peer) = model_peer(vec![
            json!({"models":[{"name":"models/m"},{"name":"models/bad\n"}],"nextPageToken":"loop"}),
            json!({"models":[],"nextPageToken":"loop"}),
        ])
        .await;
        assert!(
            fetch_models(&draft_settings("opencode", "gemini", &base), None)
                .await
                .unwrap_err()
                .contains("分页标记")
        );
        assert_eq!(peer.await.unwrap().len(), 2);
    }
    #[tokio::test]
    async fn gemini_discovery_excludes_explicit_non_chat_models_and_normalizes_all_base_forms() {
        for (suffix, normalized) in [
            ("", "/v1beta"),
            ("/google", "/google/v1beta"),
            ("/google/v1alpha/", "/google/v1alpha"),
        ] {
            let (base, peer) = model_peer(vec![json!({"models":[{"name":"models/embedding-fixture","supportedGenerationMethods":["embedContent"]},{"name":"models/image-fixture","supportedGenerationMethods":["predict"]},{"name":"models/chat-fixture","supportedGenerationMethods":["generateContent","countTokens"]},{"name":"models/gateway-without-methods"}]})]).await;
            let mut draft = draft_settings("pi", "gemini", &format!("{base}{suffix}"));
            assert_eq!(
                fetch_models(&draft, None).await.unwrap().0,
                ["chat-fixture", "gateway-without-methods"]
            );
            assert!(peer.await.unwrap()[0]
                .starts_with(&format!("GET {normalized}/models?pageSize=200 HTTP/1.1")));
            draft.name = "fixture".into();
            draft.model = "chat-fixture".into();
            let profile = make_profile(draft, None).unwrap();
            let saved_base = format!("{base}{suffix}").trim_end_matches('/').to_owned();
            assert_eq!(profile.config["baseUrl"], saved_base);
            assert_eq!(settings(&profile).base_url, saved_base);
        }
    }
    #[test]
    fn imported_gemini_base_survives_edit_and_save_without_adding_a_version() {
        for base in [
            "https://gateway.example",
            "https://gateway.example/google",
            "https://gateway.example/google/v1alpha",
        ] {
            let config = crate::ccswitch::sanitize_provider("gemini", "fixture", json!({
                "env":{"GOOGLE_GEMINI_BASE_URL":base,"GEMINI_API_KEY":"fixture-secret","GEMINI_MODEL":"gemini-fixture"}
            })).unwrap();
            let previous = Profile {
                id: "gemini:fixture".into(),
                agent: "gemini".into(),
                name: "导入连接".into(),
                config,
            };
            let edit = settings(&previous);
            assert_eq!(edit.base_url, base);
            assert_eq!(edit.api_key, None);
            let saved = make_profile(edit, Some(&previous)).unwrap();
            assert_eq!(saved.config["baseUrl"], base);
            assert_eq!(key(&saved.config), Some("fixture-secret"));
            assert_eq!(settings(&saved).base_url, base);
            let (endpoint, _) = connection_probe(&settings(&saved)).unwrap();
            assert_eq!(
                endpoint,
                format!(
                    "{}/models/gemini-fixture:generateContent",
                    gemini_api_base(base)
                )
            );
        }
        // Changing only the displayed base notation must retain capacities and
        // native SDK declarations attached to the same effective API route.
        let previous = Profile {
            id: "pi:fixture".into(),
            agent: "pi".into(),
            name: "导入连接".into(),
            config: json!({"protocol":"gemini","baseUrl":"https://gateway.example/v1beta","model":"gemini-fixture","apiKey":"fixture-secret",
                "modelMetadata":{"gemini-fixture":{"contextWindow":1048576}},"piProvider":{"name":"fixture"}}),
        };
        let mut edit = settings(&previous);
        edit.base_url = "https://gateway.example".into();
        let saved = make_profile(edit, Some(&previous)).unwrap();
        assert_eq!(saved.config["baseUrl"], "https://gateway.example");
        assert_eq!(
            saved.config["modelMetadata"],
            previous.config["modelMetadata"]
        );
        assert_eq!(saved.config["piProvider"], previous.config["piProvider"]);
    }
    async fn error_peer(status: u16, body: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut block = [0; 1024];
                let count = stream.read(&mut block).await.unwrap();
                assert!(count > 0 && request.len() < 16 * 1024);
                request.extend_from_slice(&block[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let header = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nx-request-id: fixture-request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        });
        (format!("http://{address}"), task)
    }
    #[tokio::test]
    async fn model_listing_keeps_gateway_503_reason_and_request_without_exposing_key() {
        let (base, peer) = error_peer(
            503,
            serde_json::to_vec(
                &json!({"error":{"message":"no available channel; supplied fixture-secret"}}),
            )
            .unwrap(),
        )
        .await;
        let message = fetch_models(
            &draft_settings("gemini", "gemini", &base),
            Some("fixture-secret"),
        )
        .await
        .unwrap_err();
        assert!(message.contains("HTTP 503"));
        assert!(message.contains("供应商服务或上游网关异常"));
        assert!(message.contains("no available channel"));
        assert!(message.contains("[REDACTED]"));
        assert!(message.contains(&format!("{base}/v1beta/models")));
        assert!(!message.contains("pageSize="));
        assert!(message.contains("fixture-request"));
        assert!(!message.contains("fixture-secret"));
        peer.await.unwrap();
    }
    #[tokio::test]
    async fn provider_errors_omit_oversized_bodies_instead_of_leaking_partial_credentials() {
        let secret = "sensitive-value".repeat(400);
        let body = format!("{}{}", "x".repeat(31 * 1024), secret).into_bytes();
        let (base, peer) = error_peer(503, body).await;
        let response = client()
            .unwrap()
            .get(format!("{base}/v1beta/models?pageToken=private-marker"))
            .send()
            .await
            .unwrap();
        let message = provider_json(response, Some(&secret)).await.unwrap_err();
        assert!(message.contains("HTTP 503"));
        assert!(!message.contains("供应商原因"));
        assert!(!message.contains("sensitive-value"));
        assert!(!message.contains("private-marker"));
        peer.await.unwrap();
    }
    #[test]
    fn provider_error_details_are_utf8_bounded_and_redacted_before_shortening() {
        let secret = "long-secret".repeat(500);
        let body = serde_json::to_vec(
            &json!({"error":{"message":format!("{secret}：无可用上游 {}", "中".repeat(1000))}}),
        )
        .unwrap();
        let reason = provider_error_reason(&body, &[secret]).unwrap();
        assert!(reason.starts_with("[REDACTED]：无可用上游"));
        assert!(reason.chars().count() <= 512);
        assert!(!reason.contains("long-secret"));
        assert!(provider_error_reason(b"<html>gateway error</html>", &[]).is_none());
        assert!(provider_error_reason(&[0xff, 0xfe], &[]).is_none());
        assert_eq!(
            provider_error_reason(
                br#"{"error":{"message":""},"message":"upstream unavailable"}"#,
                &[]
            )
            .as_deref(),
            Some("upstream unavailable")
        );
        assert!(http_failure(401).contains("认证失败"));
        assert!(!http_failure(503).contains("请检查 API Key"));
    }
    #[test]
    fn connection_probe_keeps_each_protocol_url_body_and_response_shape_separate() {
        let rows = [
            (
                "claude",
                "anthropic",
                "https://fixture.test",
                "https://fixture.test/v1/messages",
                json!({"content":[{"type":"text","text":"OK"}]}),
            ),
            (
                "codex",
                "responses",
                "https://fixture.test/v1",
                "https://fixture.test/v1/responses",
                json!({"output":[{"type":"message","content":[{"type":"output_text","text":"OK"}]}]}),
            ),
            (
                "pi",
                "chat",
                "https://fixture.test/v1",
                "https://fixture.test/v1/chat/completions",
                json!({"choices":[{"message":{"content":"OK"}}]}),
            ),
            (
                "gemini",
                "gemini",
                "https://fixture.test/google",
                "https://fixture.test/google/v1beta/models/gemini-fixture:generateContent",
                json!({"candidates":[{"content":{"parts":[{"text":"OK"}]}}]}),
            ),
        ];
        for (agent, protocol, base, expected, response) in &rows {
            let mut settings = draft_settings(agent, protocol, base);
            settings.model = "gemini-fixture".into();
            let (url, body) = connection_probe(&settings).unwrap();
            assert_eq!(&url, expected);
            if *protocol == "gemini" {
                assert_eq!(body["contents"][0]["parts"][0]["text"], "Reply only OK.");
                assert!(body.get("messages").is_none());
            }
            assert!(valid_probe_response(protocol, response));
            assert!(!valid_probe_response(
                protocol,
                &json!({"content":[],"output":[],"choices":[],"candidates":[]})
            ));
            for (_, other, _, _, other_response) in &rows {
                if protocol != other {
                    assert!(!valid_probe_response(protocol, other_response));
                }
            }
        }
    }
    #[test]
    fn switching_open_and_pi_protocols_retains_existing_secret_without_exposing_it() {
        for agent in ["opencode", "pi"] {
            for original in ["anthropic", "chat", "responses", "gemini"] {
                let mut initial = draft_settings(agent, original, "https://gateway.example/v1");
                initial.name = "连接".into();
                initial.model = "m".into();
                initial.api_key = Some("retained-secret".into());
                let previous = make_profile(initial, None).unwrap();
                for protocol in ["anthropic", "chat", "responses", "gemini"] {
                    let mut edit = settings(&previous);
                    edit.protocol = protocol.into();
                    assert_eq!(
                        listing_secret(&edit, Some(&previous)).unwrap(),
                        Some("retained-secret")
                    );
                    let saved = make_profile(edit, Some(&previous)).unwrap();
                    assert_eq!(key(&saved.config), Some("retained-secret"));
                    assert_eq!(saved.config["protocol"], protocol);
                    assert!(!serde_json::to_string(&settings(&saved))
                        .unwrap()
                        .contains("retained-secret"));
                }
                let foreign = draft_settings("codex", "responses", "https://gateway.example/v1");
                assert!(listing_secret(&foreign, Some(&previous)).is_err());
            }
        }
    }
    #[test]
    fn preview_capacity_is_scoped_to_endpoint_protocol_agent_key_and_saved_models() {
        let mut draft = draft_settings("gemini", "gemini", "https://gateway.example/v1beta");
        draft.name = "Gemini".into();
        draft.model = "gemini-fixture".into();
        draft.api_key = Some("first-key".into());
        let fingerprint = metadata_fingerprint(&draft, Some("first-key")).unwrap();
        let mut cache = PreviewMetadata::default();
        let now = std::time::Instant::now();
        cache.save(
            fingerprint,
            json!({"gemini-fixture":{"contextWindow":123456},"unselected":{"contextWindow":500}}),
            now,
        );
        for (agent, protocol, url, secret) in [
            (
                "pi",
                "gemini",
                "https://gateway.example/v1beta",
                "first-key",
            ),
            (
                "gemini",
                "chat",
                "https://gateway.example/v1beta",
                "first-key",
            ),
            (
                "gemini",
                "gemini",
                "https://other.example/v1beta",
                "first-key",
            ),
            (
                "gemini",
                "gemini",
                "https://gateway.example/v1beta",
                "second-key",
            ),
        ] {
            assert!(cache
                .get(
                    &metadata_fingerprint(&draft_settings(agent, protocol, url), Some(secret))
                        .unwrap(),
                    now
                )
                .is_none());
        }
        let metadata = cache.get(&fingerprint, now).unwrap();
        let mut profile = make_profile(draft, None).unwrap();
        merge_preview_metadata(&mut profile, &metadata);
        assert_eq!(
            profile.config["modelMetadata"]["gemini-fixture"]["contextWindow"],
            123456
        );
        assert!(profile.config["modelMetadata"].get("unselected").is_none());
        assert_eq!(
            metadata_fingerprint(&settings(&profile), key(&profile.config)).unwrap(),
            fingerprint
        );
        assert!(cache
            .get(&fingerprint, now + std::time::Duration::from_secs(301))
            .is_none());
        for index in 0..100 {
            cache.save([index; 32], metadata.clone(), now);
        }
        assert_eq!(cache.0.len(), 32);
        assert!(!serde_json::to_string(&settings(&profile))
            .unwrap()
            .contains("first-key"));
    }
    #[test]
    fn subscription_tests_accept_the_selected_engine_and_its_supported_protocols() {
        for (agent, protocols) in [
            ("claude", vec!["anthropic"]),
            ("codex", vec!["responses"]),
            ("opencode", vec!["anthropic", "responses", "chat"]),
            ("pi", vec!["anthropic", "responses", "chat"]),
        ] {
            for protocol in ["anthropic", "responses", "chat"] {
                assert_eq!(
                    validate_plan_test_engine(agent, protocol).is_ok(),
                    protocols.contains(&protocol),
                    "{agent}: {protocol}"
                );
            }
        }
        for plan in ["coding", "token", "coding-responses", "token-responses"] {
            assert!(subscription_plan(plan));
        }
        for plan in [
            "standard",
            "standard-responses",
            "tokenhub-responses",
            "anthropic",
        ] {
            assert!(!subscription_plan(plan));
        }
    }
    #[test]
    fn gemini_cli_tests_use_the_native_engine_even_for_standard_imports() {
        for plan in ["standard", "standard-gemini", "coding", "token"] {
            let mut settings = draft_settings("gemini", "gemini", "https://gateway.example");
            settings.plan = plan.into();
            assert!(native_connection_test(&settings));
        }
        for agent in ["opencode", "pi", "codex", "claude"] {
            let mut settings = draft_settings(agent, "gemini", "https://gateway.example");
            assert!(!native_connection_test(&settings));
            settings.plan = "coding".into();
            assert!(native_connection_test(&settings));
        }
    }
    #[test]
    fn missing_engine_is_reported_before_protocol_validation_without_falling_back_to_claude() {
        for agent in ["claude", "codex", "opencode", "pi"] {
            let profile = Profile {
                id: "test".into(),
                agent: agent.into(),
                name: "GLM".into(),
                config: json!({"plan":"coding","protocol":"invalid","model":"glm-5.3"}),
            };
            let missing = format!(
                "未找到 {}，请到设置 → Agent 一键安装",
                crate::agents::name(agent)
            );
            let result = plan_test_launch(&profile, |selected| {
                assert_eq!(selected, agent);
                Err(missing.clone())
            });
            assert_eq!(result.err().as_deref(), Some(missing.as_str()));
        }
    }
    #[test]
    fn editing_imported_native_credentials_retains_metadata_only_on_the_same_route() {
        for (agent, raw, native) in [
            (
                "opencode",
                json!({"npm":"@ai-sdk/openai","options":{"baseURL":"https://api.example/v1","apiKey":"old-key","headers":{"X-Project":"project"}},"models":{"m":{"limit":{"context":200000}}}}),
                "opencodeProvider",
            ),
            (
                "pi",
                json!({"baseUrl":"https://api.example/v1","api":"openai-completions","apiKey":"old-key","models":[{"id":"m","contextWindow":32768}]}),
                "piProvider",
            ),
        ] {
            let mut profile = Profile {
                id: format!("{agent}:a"),
                agent: agent.into(),
                name: "导入连接".into(),
                config: crate::ccswitch::sanitize_provider(agent, "a", raw).unwrap(),
            };
            profile.config["modelMetadata"] = json!({"m":{"contextWindow":65536}});
            let mut edit = settings(&profile);
            edit.api_key = Some("new-key".into());
            let saved = make_profile(edit, Some(&profile)).unwrap();
            assert_eq!(key(&saved.config), Some("new-key"));
            assert_eq!(saved.config[native], profile.config[native]);
            assert!(saved.config.get("modelMetadata").is_none());
            let same_key = make_profile(settings(&profile), Some(&profile)).unwrap();
            assert_eq!(
                same_key.config["modelMetadata"],
                profile.config["modelMetadata"]
            );
            let mut reroute = settings(&saved);
            reroute.base_url = "https://new.example/v1".into();
            assert!(make_profile(reroute, Some(&saved))
                .unwrap()
                .config
                .get(native)
                .is_none());
            let mut reroute = settings(&saved);
            reroute.base_url = "https://new.example/v1".into();
            assert!(make_profile(reroute, Some(&saved))
                .unwrap()
                .config
                .get("modelMetadata")
                .is_none());
        }
    }
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
    fn kimi_coding_responses_uses_subscription_metadata() {
        for base in [
            "https://api.kimi.com/coding/v1",
            "https://api.kimi.ai/coding/v1",
        ] {
            let config = json!({"baseUrl":base,"protocol":"responses"});
            let source = model_source(&config, "codex", Some("Kimi 测试"));
            assert_eq!(source["providerName"], "Kimi Code");
            assert_eq!(source["planName"], "编程计划");
        }
        let source = model_source(
            &json!({"providerId":"kimi","plan":"coding-responses","baseUrl":"https://api.kimi.com/coding/v1"}),
            "codex",
            None,
        );
        assert_eq!(source["providerName"], "Kimi Code");
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
        let fable = json!({"env":{"ANTHROPIC_DEFAULT_FABLE_MODEL":"glm-5.3"}});
        assert_eq!(
            real_claude_model(&fable, Some("fable")).as_deref(),
            Some("glm-5.3")
        );
        assert_eq!(real_claude_model(&fable, None).as_deref(), Some("glm-5.3"));
        assert_eq!(catalog_models(&fable)["data"][0]["contextWindow"], 1000000);
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
        let settings = |agent: &str, protocol: &str| {
            serde_json::from_value::<Settings>(json!({
            "name":"连接","agent":agent,"providerId":"custom","plan":protocol,"protocol":protocol,
            "baseUrl":"https://example.test/v1","model":"test-model","models":["test-model"],"apiKey":"test-key"
        })).unwrap()
        };
        for agent in ["claude", "codex", "opencode", "pi", "gemini"] {
            for protocol in ["anthropic", "responses", "chat", "gemini"] {
                let allowed = match agent {
                    "claude" => protocol == "anthropic",
                    "codex" => protocol == "responses",
                    "gemini" => protocol == "gemini",
                    _ => true,
                };
                assert_eq!(
                    make_profile(settings(agent, protocol), None).is_ok(),
                    allowed,
                    "{agent}/{protocol}"
                );
            }
        }
        let mut fake_official = settings("claude", "anthropic");
        fake_official.official = true;
        assert!(make_profile(fake_official, None).is_err());
        let legacy = Profile {
            id: "legacy-chat".into(),
            agent: "claude".into(),
            name: "旧连接".into(),
            config: json!({"protocol":"chat","baseUrl":"https://example.test/v1","apiKey":"test-key"}),
        };
        assert!(make_profile(settings("claude", "chat"), Some(&legacy)).is_ok());
    }
    #[test]
    fn all_bundled_responses_presets_generate_codex_api_routes_without_official_login() {
        for provider in bundled_catalog().as_array().unwrap() {
            for preset in provider["presets"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| p["protocol"] == "responses" && p["baseUrl"] != "")
            {
                let model = preset["models"][0].as_str().unwrap_or("model-from-account");
                let s = serde_json::from_value::<Settings>(json!({
                    "name":provider["name"], "agent":"codex", "providerId":provider["id"], "plan":preset["id"],
                    "protocol":"responses", "baseUrl":preset["baseUrl"], "model":model, "models":[model], "apiKey":"test-key"
                })).unwrap();
                let profile = make_profile(s, None).unwrap();
                assert!(
                    !profile.is_official(),
                    "{} / {}",
                    provider["id"],
                    preset["id"]
                );
                let table =
                    toml::from_str::<toml::Table>(profile.config["config"].as_str().unwrap())
                        .unwrap();
                let route = &table["model_providers"]["supercode_api"];
                assert_eq!(route["wire_api"].as_str(), Some("responses"));
                assert_eq!(
                    route["base_url"].as_str().unwrap().trim_end_matches('/'),
                    preset["baseUrl"].as_str().unwrap().trim_end_matches('/')
                );
                assert_eq!(
                    route["env_key"].as_str(),
                    Some("SUPERCODE_PROVIDER_API_KEY")
                );
                assert_eq!(route["requires_openai_auth"].as_bool(), Some(false));
                assert!(!profile.config["config"]
                    .as_str()
                    .unwrap()
                    .contains("test-key"));
                let roundtrip = settings(&profile);
                assert_eq!(roundtrip.agent, "codex");
                assert_eq!(roundtrip.provider_id, provider["id"].as_str().unwrap());
                assert_eq!(roundtrip.plan, preset["id"].as_str().unwrap());
            }
        }
    }
    #[test]
    fn hy3_search_requirement_is_endpoint_specific() {
        for (base, disabled) in [
            ("https://tokenhub.tencentmaas.com/v1/", true),
            ("https://gateway.example/v1", false),
        ] {
            let s = serde_json::from_value::<Settings>(json!({"name":"Hy3", "agent":"codex", "providerId":"tencent", "plan":"tokenhub-responses",
                "protocol":"responses", "baseUrl":base, "model":"hy3", "models":["hy3"], "apiKey":"test-key"})).unwrap();
            let profile = make_profile(s, None).unwrap();
            let table =
                toml::from_str::<toml::Table>(profile.config["config"].as_str().unwrap()).unwrap();
            assert_eq!(
                table.get("web_search").and_then(toml::Value::as_str),
                disabled.then_some("disabled")
            );
        }
    }
}
