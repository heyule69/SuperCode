//! Read-only CC Switch import (SQLite and legacy JSON). Credentials stay in Rust.
use crate::AppState;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

#[path = "ccswitch_native.rs"]
mod native;

#[derive(Clone)]
pub struct Profile {
    pub id: String,
    pub agent: String,
    pub name: String,
    pub config: Value,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub agent: String,
    pub name: String,
    pub model: Option<String>,
    pub has_credential: bool,
    pub official_account: bool,
    pub account_id: Option<String>,
    pub current: bool,
    pub provider_id: Option<String>,
    pub protocol: String,
    pub plan: Option<String>,
    pub source: String,
    pub models: Vec<String>,
    pub model_source: Value,
}
impl Profile {
    pub fn account_id(&self) -> Option<&str> {
        self.config["accountId"].as_str()
    }
    pub fn is_official(&self) -> bool {
        if crate::providers::key(&self.config).is_some() {
            return false;
        }
        if self.config["official"] == true {
            return matches!(self.agent.as_str(), "claude" | "codex");
        }
        if self.agent == "claude" {
            return self.config["env"]["ANTHROPIC_BASE_URL"]
                .as_str()
                .is_none_or(str::is_empty);
        }
        self.agent == "codex"
            && self.config["config"]
                .as_str()
                .and_then(|s| toml::from_str::<toml::Table>(s).ok())
                .is_some_and(|t| {
                    t.get("model_provider").and_then(toml::Value::as_str) == Some("openai")
                })
    }
    pub fn summary(&self, current: bool) -> Summary {
        Summary {
            official_account: self.is_official(),
            account_id: self.account_id().map(str::to_owned),
            id: self.id.clone(),
            agent: self.agent.clone(),
            name: self.name.clone(),
            model: crate::providers::real_claude_model(&self.config, None),
            has_credential: self.config["apiKey"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
                || self.config["env"].as_object().is_some_and(|o| {
                    ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"]
                        .iter()
                        .any(|key| {
                            o.get(*key)
                                .and_then(Value::as_str)
                                .is_some_and(|s| !s.is_empty())
                        })
                }),
            current,
            provider_id: self.config["providerId"].as_str().map(str::to_owned),
            protocol: self.config["protocol"]
                .as_str()
                .unwrap_or(if self.agent == "claude" {
                    "anthropic"
                } else {
                    "responses"
                })
                .into(),
            plan: self.config["plan"].as_str().map(str::to_owned),
            source: self.config["source"].as_str().unwrap_or("ccswitch").into(),
            models: crate::providers::model_ids(&self.config),
            model_source: crate::providers::model_source(
                &self.config,
                &self.agent,
                Some(&self.name),
            ),
        }
    }
}
fn home() -> Result<PathBuf, String> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var_os("HOME").filter(|s| !s.is_empty()))
        .map(PathBuf::from)
        .ok_or("无法定位用户目录".into())
}
fn expand_path(path: &str, home: &std::path::Path) -> PathBuf {
    let path = path.trim();
    if path == "~" {
        home.into()
    } else if let Some(relative) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        home.join(relative)
    } else {
        PathBuf::from(path)
    }
}
fn path_override(
    store: &std::path::Path,
    home: &std::path::Path,
) -> Result<Option<PathBuf>, String> {
    use std::io::Read;
    let file = match std::fs::File::open(store) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取 CC Switch 自定义目录设置".into()),
    };
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取 CC Switch 自定义目录设置")?;
    if bytes.len() > 64 * 1024 {
        return Err("CC Switch 路径设置超过大小限制".into());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "CC Switch 路径设置不是 UTF-8，未转换或修改原文件")?;
    if text.starts_with('\u{feff}') {
        return Err("CC Switch 路径设置含 BOM，未转换或修改原文件".into());
    }
    let value: Value = serde_json::from_str(text).map_err(|_| "CC Switch 路径设置 JSON 无效")?;
    Ok(value["app_config_dir_override"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| expand_path(s, home))
        .filter(|p| p.is_dir()))
}
fn locate_database(
    manual: Option<&str>,
    configured: Option<PathBuf>,
    store: Option<&std::path::Path>,
    user_home: &std::path::Path,
    legacy_home: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let mut path = if let Some(path) = manual.filter(|s| !s.trim().is_empty()) {
        expand_path(path, user_home)
    } else if let Some(path) =
        configured.filter(|p| !p.as_os_str().to_string_lossy().trim().is_empty())
    {
        expand_path(&path.to_string_lossy(), user_home)
    } else if let Some(path) = store
        .map(|p| path_override(p, user_home))
        .transpose()?
        .flatten()
    {
        path
    } else {
        let default = user_home.join(".cc-switch");
        if source_in_directory(&default).is_some() {
            default
        } else {
            legacy_home
                .map(|p| p.join(".cc-switch"))
                .filter(|p| source_in_directory(p).is_some())
                .unwrap_or(default)
        }
    };
    if path.is_dir() {
        path = source_in_directory(&path).unwrap_or_else(|| path.join("cc-switch.db"));
    }
    if !path.is_file() {
        return Err(
            "未找到 CC Switch 配置，可输入 cc-switch.db、旧版 config.json 或所在目录".into(),
        );
    }
    Ok(path)
}
fn source_in_directory(dir: &std::path::Path) -> Option<PathBuf> {
    ["cc-switch.db", "config.json"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
}
fn database(path: Option<String>) -> Result<PathBuf, String> {
    // CC Switch stores its custom directory in Tauri's app_paths.json.
    let store = if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(|p| PathBuf::from(p).join("com.ccswitch.desktop/app_paths.json"))
    } else {
        None
    };
    locate_database(
        path.as_deref(),
        std::env::var_os("CC_SWITCH_CONFIG_DIR")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from),
        store.as_deref(),
        &home()?,
        std::env::var_os("HOME").map(PathBuf::from),
    )
}
fn claude_env(raw: &Value) -> Value {
    let env = raw["env"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter(|(k, v)| {
                    v.is_string()
                        && (k.starts_with("ANTHROPIC_")
                            || [
                                "DISABLE_PROMPT_CACHING",
                                "MAX_THINKING_TOKENS",
                                "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
                                "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS",
                            ]
                            .contains(&k.as_str()))
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    Value::Object(env)
}
fn imported_base_url(value: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(value.trim()).map_err(|_| "CC Switch API 地址无效")?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return Err("CC Switch API 地址须使用 HTTP 或 HTTPS".into());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("CC Switch API 地址不能包含账号、密码、查询参数或片段".into());
    }
    // Existing CC Switch connections may use a LAN or remote HTTP gateway.
    // Import preserves its transport rather than applying new-connection rules.
    Ok(value.trim().trim_end_matches('/').into())
}
fn sanitize(agent: &str, raw: Value) -> Result<Value, String> {
    sanitize_provider(agent, "", raw)
}
pub(crate) fn sanitize_provider(agent: &str, id: &str, raw: Value) -> Result<Value, String> {
    if !raw.is_object() {
        return Err("配置 JSON 必须是对象".into());
    }
    if agent == "claude" {
        let mut safe = json!({"env":claude_env(&raw),"model":raw["model"],"source":"ccswitch"});
        safe["model"] = json!(crate::providers::real_claude_model(&safe, None));
        // MAX_CONTEXT declares this selected model's window. It is not a safe
        // global override for differently sized child models on this connection.
        // AUTO_COMPACT_WINDOW is only a trigger window, never model capacity.
        if let (Some(model),Some(window))=(safe["model"].as_str().map(str::to_owned),crate::model_context::positive(&raw["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"])) {
            safe["modelMetadata"]=json!({model:{"contextWindow":window}});
        }
        return Ok(safe);
    }
    if matches!(agent, "opencode" | "pi") {
        return native::sanitize(agent, id, &raw);
    }
    let cfg: toml::Table =
        toml::from_str(raw["config"].as_str().ok_or("Codex 配置缺少 config TOML")?)
            .map_err(|_| "CC Switch 的 Codex 配置 TOML 无效，未转换或修改原文件")?;
    let mut safe = toml::Table::new();
    for key in [
        "model",
        "model_provider",
        "model_reasoning_effort",
        "service_tier",
    ] {
        if let Some(v) = cfg.get(key).filter(|v| v.is_str()) {
            safe.insert(key.into(), v.clone());
        }
    }
    if safe.get("service_tier").and_then(toml::Value::as_str) == Some("priority") {
        safe.insert("service_tier".into(), toml::Value::String("fast".into()));
    }
    let active_provider = cfg
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    safe.remove("model_provider");
    if let Some(provider) = active_provider {
        safe.insert(
            "model_provider".into(),
            toml::Value::String(provider.into()),
        );
    }
    // CC Switch also supports inline bearer tokens. Only the active custom
    // provider may supply one; dormant provider entries cannot change login.
    let inline_key = active_provider
        .filter(|provider| *provider != "openai")
        .and_then(|provider| cfg.get("model_providers")?.get(provider))
        .and_then(|provider| provider.get("experimental_bearer_token"))
        .and_then(toml::Value::as_str)
        .or_else(|| {
            cfg.get("experimental_bearer_token")
                .and_then(toml::Value::as_str)
        });
    let key = raw["auth"]["OPENAI_API_KEY"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| inline_key.map(str::trim).filter(|s| !s.is_empty()));
    if let Some(provider) = safe
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
    {
        if !provider
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err("Codex 供应商 ID 含不支持的字符".into());
        }
        if provider != "openai" {
            let source = cfg
                .get("model_providers")
                .and_then(|v| v.get(&provider))
                .and_then(toml::Value::as_table);
            let mut connection = toml::Table::new();
            for key in ["name", "wire_api"] {
                if let Some(v) = source
                    .and_then(|source| source.get(key))
                    .filter(|v| v.is_str())
                {
                    connection.insert(key.into(), v.clone());
                }
            }
            let base = source
                .and_then(|source| source.get("base_url"))
                .or_else(|| cfg.get("base_url"))
                .and_then(toml::Value::as_str)
                .ok_or("Codex 缺少供应商连接地址")?;
            connection.insert(
                "base_url".into(),
                toml::Value::String(imported_base_url(base)?),
            );
            if connection
                .get("wire_api")
                .and_then(toml::Value::as_str)
                .is_some_and(|v| v != "responses")
            {
                return Err("目前仅支持 Codex Responses 供应商配置".into());
            }
            connection.insert("wire_api".into(), toml::Value::String("responses".into()));
            if key.is_some() {
                connection.insert(
                    "env_key".into(),
                    toml::Value::String("SUPERCODE_PROVIDER_API_KEY".into()),
                );
                connection.insert("requires_openai_auth".into(), toml::Value::Boolean(false));
            }
            safe.insert(
                "model_providers".into(),
                toml::Value::Table(toml::Table::from_iter([(
                    provider,
                    toml::Value::Table(connection),
                )])),
            );
        }
    }
    if !safe.contains_key("model_provider") {
        safe.insert(
            "model_provider".into(),
            toml::Value::String("openai".into()),
        );
    }
    // An omitted model_provider means built-in OpenAI, not a broken config.
    // API credentials always get an explicit API route, separate from login.
    // Never select leftover model_providers entries without an active provider.
    if key.is_some() && safe.get("model_provider").and_then(toml::Value::as_str) == Some("openai") {
        let base = active_provider
            .and_then(|provider| cfg.get("model_providers")?.get(provider))
            .and_then(|v| v.get("base_url"))
            .and_then(toml::Value::as_str)
            .or_else(|| cfg.get("base_url").and_then(toml::Value::as_str))
            .or_else(|| cfg.get("openai_base_url").and_then(toml::Value::as_str))
            .unwrap_or("https://api.openai.com/v1");
        let connection = toml::Table::from_iter([
            ("name".into(), toml::Value::String("OpenAI API".into())),
            (
                "base_url".into(),
                toml::Value::String(imported_base_url(base)?),
            ),
            ("wire_api".into(), toml::Value::String("responses".into())),
            (
                "env_key".into(),
                toml::Value::String("SUPERCODE_PROVIDER_API_KEY".into()),
            ),
            ("requires_openai_auth".into(), toml::Value::Boolean(false)),
        ]);
        safe.insert(
            "model_provider".into(),
            toml::Value::String("supercode_api".into()),
        );
        safe.insert(
            "model_providers".into(),
            toml::Value::Table(toml::Table::from_iter([(
                "supercode_api".into(),
                toml::Value::Table(connection),
            )])),
        );
    }
    Ok(
        json!({"config":toml::to_string(&safe).map_err(|_|"配置序列化失败")?,"model":safe.get("model").and_then(toml::Value::as_str),"apiKey":key}),
    )
}
struct ReadResult {
    path: PathBuf,
    profiles: Vec<(Profile, bool)>,
    warnings: Vec<String>,
}
#[derive(Serialize)]
pub struct ScanResult {
    pub path: String,
    pub profiles: Vec<Summary>,
    pub warnings: Vec<String>,
}
fn read(path: Option<String>, agent: &str) -> Result<ReadResult, String> {
    if !matches!(agent, "claude" | "codex" | "opencode" | "pi") {
        return Err("不支持的 Agent，请重新选择供应商标签".into());
    }
    let path = database(path)?;
    if path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        return read_legacy(path, agent);
    }
    let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "无法以只读方式打开 CC Switch 数据库")?;
    connection
        .busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|e| e.to_string())?;
    let mut stmt=connection.prepare("SELECT id,app_type,name,settings_config,COALESCE(is_current,0) FROM providers WHERE app_type = ?1 ORDER BY name LIMIT 500").map_err(|_|"数据库不是受支持的 CC Switch providers 格式")?;
    let rows = stmt
        .query_map([agent], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, bool>(4)?,
            ))
        })
        .map_err(|_| "无法读取 CC Switch 配置")?;
    let mut profiles = vec![];
    let mut warnings = vec![];
    for row in rows {
        let (id, agent, name, raw, current) = match row {
            Ok(row) => row,
            Err(rusqlite::Error::FromSqlConversionFailure(_, _, error))
                if error.is::<std::str::Utf8Error>() =>
            {
                warnings.push("已跳过非 UTF-8 配置：未转换或修改原文件".into());
                continue;
            }
            Err(_) => {
                warnings.push("已跳过无法读取的配置：字段格式无效，未转换或修改原文件".into());
                continue;
            }
        };
        if raw.len() > 1024 * 1024 {
            warnings.push(format!("{name}：配置超过 1 MB，已跳过"));
            continue;
        }
        let config = serde_json::from_str::<Value>(&raw)
            .map_err(|_| "配置 JSON 无效".into())
            .and_then(|raw| sanitize_provider(&agent, &id, raw));
        let config = match config {
            Ok(config) => config,
            Err(error) => {
                warnings.push(format!("{name}：{error}，已跳过"));
                continue;
            }
        };
        profiles.push((
            Profile {
                id: format!("{agent}:{id}"),
                config,
                agent,
                name,
            },
            current,
        ));
    }
    Ok(ReadResult {
        path,
        profiles,
        warnings,
    })
}
fn read_legacy(path: PathBuf, selected_agent: &str) -> Result<ReadResult, String> {
    use std::io::Read;
    let file = std::fs::File::open(&path).map_err(|_| "无法只读打开 CC Switch 旧版配置")?;
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取 CC Switch 旧版配置")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("CC Switch 旧版配置超过 8 MB".into());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "CC Switch 旧版配置不是 UTF-8，未转换或修改原文件")?;
    if text.starts_with('\u{feff}') {
        return Err("CC Switch 旧版配置含 BOM，未转换或修改原文件".into());
    }
    let value: Value = serde_json::from_str(text).map_err(|_| "CC Switch 旧版配置 JSON 无效")?;
    if !crate::agents::IDS
        .iter()
        .any(|agent| value[*agent]["providers"].is_object())
    {
        return Err("文件不是受支持的 CC Switch 旧版供应商配置".into());
    }
    let mut profiles = Vec::new();
    let mut warnings = Vec::new();
    let total: usize = crate::agents::IDS
        .into_iter()
        .filter(|agent| *agent == selected_agent)
        .filter_map(|agent| value[agent]["providers"].as_object().map(|rows| rows.len()))
        .sum();
    if total > 500 {
        return Err("CC Switch 配置超过 500 个连接".into());
    }
    for agent in crate::agents::IDS
        .into_iter()
        .filter(|agent| *agent == selected_agent)
    {
        if let Some(rows) = value[agent]["providers"].as_object() {
            for (id, row) in rows {
                let name = row["name"].as_str().unwrap_or(id);
                if row["settingsConfig"].to_string().len() > 1024 * 1024 {
                    warnings.push(format!("{name}：配置超过 1 MB，已跳过"));
                    continue;
                }
                match sanitize_provider(agent, id, row["settingsConfig"].clone()) {
                    Ok(config) => profiles.push((
                        Profile {
                            id: format!("{agent}:{id}"),
                            agent: agent.into(),
                            name: name.into(),
                            config,
                        },
                        value[agent]["current"].as_str() == Some(id),
                    )),
                    Err(error) => warnings.push(format!("{name}：{error}，已跳过")),
                }
            }
        }
    }
    Ok(ReadResult {
        path,
        profiles,
        warnings,
    })
}
#[tauri::command]
pub fn scan_ccswitch(path: Option<String>, agent: String) -> Result<ScanResult, String> {
    let result = read(path, &agent)?;
    Ok(ScanResult {
        path: result.path.to_string_lossy().into(),
        profiles: result
            .profiles
            .into_iter()
            .map(|(p, c)| p.summary(c))
            .collect(),
        warnings: result.warnings,
    })
}
fn selected_profiles(
    result: ReadResult,
    agent: &str,
    ids: &[String],
) -> Result<Vec<Profile>, String> {
    if ids.is_empty() {
        return Err("请选择要导入的配置".into());
    }
    let prefix = format!("{agent}:");
    if ids.iter().any(|id| !id.starts_with(&prefix)) {
        return Err("只能导入当前 Agent 的配置，请重新选择".into());
    }
    let selected: Vec<_> = result
        .profiles
        .into_iter()
        .filter(|(p, _)| ids.contains(&p.id))
        .map(|(p, _)| p)
        .collect();
    if selected.iter().any(|p| p.agent != agent) {
        return Err("只能导入当前 Agent 的配置，请重新选择".into());
    }
    if selected.len() != ids.len() {
        return Err("部分配置已变更，请重新扫描".into());
    }
    Ok(selected)
}
#[tauri::command]
pub fn import_ccswitch(
    path: Option<String>,
    agent: String,
    ids: Vec<String>,
    app: AppHandle,
) -> Result<usize, String> {
    if ids.is_empty() {
        return Err("请选择要导入的配置".into());
    }
    let selected = selected_profiles(read(path, &agent)?, &agent, &ids)?;
    app.state::<AppState>().store.import_profiles(&selected)?;
    Ok(selected.len())
}
#[tauri::command]
pub async fn select_agent_profile(
    agent: String,
    id: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.store.select_profile(&agent, id.as_deref())?;
    let _ = app.emit("workspace-updated", ());
    Ok(())
}
pub fn local_claude_configuration() -> Result<Value, String> {
    let path = home()?.join(".claude/settings.json");
    let bytes = match std::fs::read(path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({"env":{}})),
        Err(_) => return Err("无法读取本机 Claude 配置".into()),
    };
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err("Claude 配置含 UTF-8 BOM，未转换或修改原文件".into());
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|_| "Claude 配置不是 UTF-8，未转换或修改原文件")?;
    sanitize(
        "claude",
        serde_json::from_str(text).map_err(|_| "本机 Claude 配置 JSON 无效")?,
    )
}
pub fn codex_overrides(config: &Value) -> Result<Vec<String>, String> {
    let cfg: toml::Table = toml::from_str(config["config"].as_str().unwrap_or(""))
        .map_err(|_| "本地导入配置 TOML 无效")?;
    let mut out = vec![];
    for (key, value) in &cfg {
        if key == "model_providers" {
            if let Some(providers) = value.as_table() {
                for (name, provider) in providers {
                    if let Some(fields) = provider.as_table() {
                        for (field, value) in fields {
                            out.push(format!("model_providers.{name}.{field}={value}"));
                        }
                    }
                }
            }
        } else {
            out.push(format!("{key}={value}"));
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imported_claude_max_context_is_metadata_for_selected_model_not_a_global_child_override() {
        let config=sanitize("claude",json!({"env":{"ANTHROPIC_MODEL":"sonnet","ANTHROPIC_DEFAULT_SONNET_MODEL":"private-provider-model","ANTHROPIC_DEFAULT_HAIKU_MODEL":"glm-5-turbo","CLAUDE_CODE_MAX_CONTEXT_TOKENS":"1000000","CLAUDE_CODE_AUTO_COMPACT_WINDOW":"500000"}})).unwrap();
        assert_eq!(config["model"],"private-provider-model");
        assert_eq!(config["modelMetadata"]["private-provider-model"]["contextWindow"],1000000);
        assert!(config["modelMetadata"]["glm-5-turbo"].is_null());
        assert!(config["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"].is_null());
        assert!(config["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"].is_null());
        let trigger_only=sanitize("claude",json!({"model":"private","env":{"CLAUDE_CODE_AUTO_COMPACT_WINDOW":"1000000"}})).unwrap();
        assert!(trigger_only["modelMetadata"].is_null());
        for invalid in [json!(0),json!(-1),json!(100000001),json!(true),json!("broken")] {
            let invalid=sanitize("claude",json!({"model":"private","env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":invalid}})).unwrap();
            assert!(invalid["modelMetadata"].is_null());
        }
    }
    #[test]
    fn existing_http_gateway_routes_survive_import_without_relaxing_address_structure() {
        for config in [
            "model_provider='gateway'\n[model_providers.gateway]\nbase_url='http://gateway.local:8317/v1/'",
            "base_url='http://192.0.2.1:8317/v1/'",
        ] {
            let safe = sanitize("codex", json!({"config":config,"auth":{"OPENAI_API_KEY":"private-token"}})).unwrap();
            let table: toml::Table = toml::from_str(safe["config"].as_str().unwrap()).unwrap();
            let provider = table["model_provider"].as_str().unwrap();
            assert!(table["model_providers"][provider]["base_url"].as_str().unwrap().starts_with("http://"));
            assert_eq!(table["model_providers"][provider]["env_key"].as_str(), Some("SUPERCODE_PROVIDER_API_KEY"));
            assert!(!safe["config"].as_str().unwrap().contains("private-token"));
        }
        for invalid in [
            "file:///tmp/config",
            "http://secret@gateway.local/v1",
            "http://gateway.local/v1?key=secret",
        ] {
            assert!(imported_base_url(invalid).is_err());
        }
    }
    #[test]
    fn codex_config_variants_keep_only_active_credentials_and_preserve_gateway_routes() {
        for config in [
            "model_provider='  gateway  '\n[model_providers.gateway]\nbase_url='https://gateway.example/v1'\nexperimental_bearer_token='inline-secret'",
            "model_provider='gateway'\nbase_url='https://gateway.example/v1'\nexperimental_bearer_token='inline-secret'",
            "model_provider=' '\nbase_url='https://gateway.example/v1'\nexperimental_bearer_token='inline-secret'",
        ] {
            let safe = sanitize("codex", json!({"config":config})).unwrap();
            assert_eq!(safe["apiKey"], "inline-secret");
            let table: toml::Table = toml::from_str(safe["config"].as_str().unwrap()).unwrap();
            let provider = table["model_provider"].as_str().unwrap();
            assert_eq!(table["model_providers"][provider]["base_url"].as_str(), Some("https://gateway.example/v1"));
            assert!(codex_overrides(&safe).unwrap().iter().all(|arg| !arg.contains("inline-secret")));
        }
        let config = "model='m'\n[model_providers.leftover]\nexperimental_bearer_token='dormant-secret'\n[model_providers.openai]\nbase_url='https://wrong.example/v1'\nexperimental_bearer_token='dormant-secret'";
        let safe = sanitize("codex", json!({"config":config})).unwrap();
        assert!(safe["apiKey"].is_null());
        assert!(Profile {
            id: "a".into(),
            agent: "codex".into(),
            name: "Login".into(),
            config: safe
        }
        .is_official());
        let safe = sanitize(
            "codex",
            json!({"config":config,"auth":{"OPENAI_API_KEY":"auth-secret"}}),
        )
        .unwrap();
        let table: toml::Table = toml::from_str(safe["config"].as_str().unwrap()).unwrap();
        assert_eq!(
            table["model_providers"]["supercode_api"]["base_url"].as_str(),
            Some("https://api.openai.com/v1")
        );
        let safe = sanitize("codex", json!({"config":"experimental_bearer_token='inline-secret'","auth":{"OPENAI_API_KEY":"auth-secret"}})).unwrap();
        assert_eq!(safe["apiKey"], "auth-secret");
    }
    #[test]
    fn legacy_json_and_new_sqlite_layouts_are_detected_without_user_data_or_cli_installations() {
        let dir = std::env::temp_dir().join(format!("supercode-cc-兼容-{}", uuid::Uuid::new_v4()));
        let home = dir.join("全新用户 空格");
        let source = home.join(".cc-switch");
        std::fs::create_dir_all(&source).unwrap();
        let legacy = source.join("config.json");
        std::fs::write(&legacy,serde_json::to_vec(&json!({"version":2,"claude":{"current":"a","providers":{"a":{"name":"旧版 Claude","settingsConfig":{"env":{"ANTHROPIC_MODEL":"m"}}},"bad":{"name":"错误行","settingsConfig":null}}},"codex":{"current":"b","providers":{"b":{"name":"旧版 API","settingsConfig":{"auth":{"OPENAI_API_KEY":"test-secret"},"config":"model='m'"}}}}})).unwrap()).unwrap();
        let before = std::fs::read(&legacy).unwrap();
        let detected = locate_database(None, None, None, &home, None).unwrap();
        assert_eq!(detected, legacy);
        let scan = scan_ccswitch(Some(detected.to_string_lossy().into()), "claude".into()).unwrap();
        assert_eq!(scan.profiles.len(), 1);
        assert_eq!(scan.warnings.len(), 1);
        assert!(scan.profiles.iter().all(|p| p.current));
        assert!(!serde_json::to_string(&scan)
            .unwrap()
            .contains("test-secret"));
        assert_eq!(before, std::fs::read(&legacy).unwrap());
        let codex = scan_ccswitch(Some(legacy.to_string_lossy().into()), "codex".into()).unwrap();
        assert_eq!(codex.profiles.len(), 1);
        assert!(codex.warnings.is_empty());
        assert_eq!(codex.profiles[0].agent, "codex");
        assert!(!serde_json::to_string(&codex)
            .unwrap()
            .contains("test-secret"));
        // New releases add columns; only stable provider fields are read.
        let db = source.join("cc-switch.db");
        let c = Connection::open(&db).unwrap();
        c.execute_batch("CREATE TABLE providers(id TEXT,app_type TEXT,name TEXT,settings_config TEXT,is_current INTEGER,meta TEXT,provider_type TEXT,sort_index INTEGER); INSERT INTO providers VALUES('a','claude','新版 Claude','{\"env\":{\"ANTHROPIC_MODEL\":\"m\"}}',NULL,'{}',NULL,10)").unwrap();
        drop(c);
        assert_eq!(locate_database(None, None, None, &home, None).unwrap(), db);
        let scan = scan_ccswitch(Some(source.to_string_lossy().into()), "claude".into()).unwrap();
        assert_eq!(scan.profiles.len(), 1);
        assert!(scan.warnings.is_empty());
        assert!(!scan.profiles[0].current);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn default_codex_api_keys_import_as_api_connections_without_using_inactive_providers() {
        for config in [
            "model='m'",
            "model='m'\nmodel_provider='openai'",
            "model='m'\n[model_providers.leftover]\nbase_url='https://wrong.example/v1'",
        ] {
            let safe = sanitize(
                "codex",
                json!({"auth":{"OPENAI_API_KEY":"test-secret"},"config":config}),
            )
            .unwrap();
            let table: toml::Table = toml::from_str(safe["config"].as_str().unwrap()).unwrap();
            assert_eq!(table["model_provider"].as_str(), Some("supercode_api"));
            let connection = &table["model_providers"]["supercode_api"];
            assert_eq!(
                connection["base_url"].as_str(),
                Some("https://api.openai.com/v1")
            );
            assert_eq!(
                connection["env_key"].as_str(),
                Some("SUPERCODE_PROVIDER_API_KEY")
            );
            assert_eq!(connection["requires_openai_auth"].as_bool(), Some(false));
            let summary = Profile {
                id: "api".into(),
                agent: "codex".into(),
                name: "API".into(),
                config: safe,
            }
            .summary(false);
            assert!(!summary.official_account);
            assert!(summary.has_credential);
            assert!(!serde_json::to_string(&summary)
                .unwrap()
                .contains("test-secret"));
        }
        let safe = sanitize("codex", json!({"auth":{"OPENAI_API_KEY":"test-secret"},"config":"base_url='https://gateway.example/v1'\nmodel='m'"})).unwrap();
        let table: toml::Table = toml::from_str(safe["config"].as_str().unwrap()).unwrap();
        assert_eq!(
            table["model_providers"]["supercode_api"]["base_url"].as_str(),
            Some("https://gateway.example/v1")
        );
        let official = Profile {
            id: "login".into(),
            agent: "codex".into(),
            name: "登录".into(),
            config: sanitize("codex", json!({"auth":{},"config":"model='m'"})).unwrap(),
        };
        assert!(official.is_official());
    }
    #[test]
    fn partial_scan_preserves_valid_connections_and_never_exposes_failed_config_contents() {
        let dir =
            std::env::temp_dir().join(format!("supercode-cc-partial-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cc-switch.db");
        let c = Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE providers(id TEXT,app_type TEXT,name TEXT,settings_config TEXT,is_current INTEGER)").unwrap();
        for (id, agent, name, config) in [
            (
                "claude",
                "claude",
                "正常 Claude",
                json!({"env":{"ANTHROPIC_AUTH_TOKEN":"test-secret","ANTHROPIC_MODEL":"m"}})
                    .to_string(),
            ),
            (
                "api",
                "codex",
                "tokeni",
                json!({"auth":{"OPENAI_API_KEY":"test-secret"},"config":"model='m'"}).to_string(),
            ),
            (
                "broken",
                "codex",
                "错误配置",
                json!({"auth":{"OPENAI_API_KEY":"test-secret"},"config":"invalid='test-secret"})
                    .to_string(),
            ),
            ("json", "codex", "错误 JSON", "invalid test-secret".into()),
        ] {
            c.execute(
                "INSERT INTO providers VALUES(?1,?2,?3,?4,1)",
                rusqlite::params![id, agent, name, config],
            )
            .unwrap();
        }
        c.execute_batch(
            "INSERT INTO providers VALUES('encoding','codex','错误编码',CAST(x'fffe' AS TEXT),1)",
        )
        .unwrap();
        drop(c);
        let before = std::fs::read(&path).unwrap();
        let scan = scan_ccswitch(Some(path.to_string_lossy().into()), "codex".into()).unwrap();
        assert_eq!(scan.profiles.len(), 1);
        assert_eq!(scan.warnings.len(), 3);
        assert!(scan
            .warnings
            .iter()
            .any(|warning| warning.contains("非 UTF-8")));
        assert!(scan
            .profiles
            .iter()
            .any(|p| p.name == "tokeni" && !p.official_account));
        assert!(!serde_json::to_string(&scan)
            .unwrap()
            .contains("test-secret"));
        assert_eq!(before, std::fs::read(&path).unwrap());
        let claude = scan_ccswitch(Some(path.to_string_lossy().into()), "claude".into()).unwrap();
        assert_eq!(claude.profiles.len(), 1);
        assert!(claude.warnings.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn database_detection_follows_manual_environment_custom_default_and_legacy_locations() {
        let dir = std::env::temp_dir().join(format!("supercode-cc-path-{}", uuid::Uuid::new_v4()));
        let user_home = dir.join("user");
        let legacy = dir.join("legacy");
        let custom = dir.join("custom");
        let env_dir = dir.join("env");
        for path in [legacy.join(".cc-switch"), custom.clone(), env_dir.clone()] {
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("cc-switch.db"), b"").unwrap();
        }
        let store = dir.join("app_paths.json");
        std::fs::write(
            &store,
            serde_json::to_vec(&json!({"app_config_dir_override":custom})).unwrap(),
        )
        .unwrap();
        assert_eq!(
            locate_database(None, None, None, &user_home, Some(legacy.clone())).unwrap(),
            legacy.join(".cc-switch/cc-switch.db")
        );
        let default = user_home.join(".cc-switch");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(default.join("cc-switch.db"), b"").unwrap();
        assert_eq!(
            locate_database(None, None, None, &user_home, Some(legacy)).unwrap(),
            default.join("cc-switch.db")
        );
        assert_eq!(
            locate_database(None, None, Some(&store), &user_home, None).unwrap(),
            custom.join("cc-switch.db")
        );
        assert_eq!(
            locate_database(None, Some(env_dir.clone()), Some(&store), &user_home, None).unwrap(),
            env_dir.join("cc-switch.db")
        );
        assert_eq!(
            locate_database(
                None,
                Some(PathBuf::from(" ")),
                Some(&store),
                &user_home,
                None
            )
            .unwrap(),
            custom.join("cc-switch.db")
        );
        assert_eq!(
            locate_database(
                None,
                Some(PathBuf::from("~/.cc-switch")),
                None,
                &user_home,
                None
            )
            .unwrap(),
            default.join("cc-switch.db")
        );
        assert_eq!(
            locate_database(
                Some("~/.cc-switch"),
                Some(env_dir),
                Some(&store),
                &user_home,
                None
            )
            .unwrap(),
            default.join("cc-switch.db")
        );
        std::fs::write(&store, b"\xff\xfe").unwrap();
        assert!(locate_database(None, None, Some(&store), &user_home, None)
            .unwrap_err()
            .contains("UTF-8"));
        assert!(
            locate_database(Some("~/.cc-switch"), None, Some(&store), &user_home, None).is_ok()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore = "Read-only inspection of the current user's CC Switch database; run explicitly"]
    fn installed_database_is_automatically_detected_without_importing() {
        let scan = scan_ccswitch(None, "claude".into()).unwrap();
        assert!(!scan.profiles.is_empty(), "no supported connections found");
        println!(
            "Detected database: {}\nImportable connections: {}\nSkipped connections: {}",
            scan.path,
            scan.profiles.len(),
            scan.warnings.len()
        );
    }
    #[test]
    fn import_excludes_hooks_permissions_and_never_returns_credentials_in_summary() {
        let config=sanitize("claude",json!({"env":{"ANTHROPIC_AUTH_TOKEN":"test-secret","ANTHROPIC_MODEL":"test-model","NODE_OPTIONS":"unsafe"},"hooks":{"command":"unsafe"},"permissions":{"defaultMode":"bypassPermissions"}})).unwrap();
        assert!(config.get("hooks").is_none());
        assert!(config.get("permissions").is_none());
        assert!(config["env"].get("NODE_OPTIONS").is_none());
        let profile = Profile {
            id: "claude:test".into(),
            agent: "claude".into(),
            name: "test".into(),
            config,
        };
        assert!(!serde_json::to_string(&profile.summary(false))
            .unwrap()
            .contains("test-secret"));
        let codex=sanitize("codex",json!({"auth":{"OPENAI_API_KEY":"test-secret"},"config":"model_provider='custom'\nmodel='test'\napproval_policy='never'\n[model_providers.custom]\nbase_url='https://example.com/v1'\nwire_api='responses'\n"})).unwrap();
        let args = codex_overrides(&codex).unwrap().join(" ");
        assert!(!args.contains("test-secret"));
        assert!(!args.contains("approval_policy"));
        assert!(args.contains("env_key"));
    }
    #[test]
    fn import_selection_rejects_other_agents_duplicates_and_missing_rows_before_writing() {
        let rows = || ReadResult {
            path: PathBuf::from("cc-switch.db"),
            warnings: vec![],
            profiles: vec![
                (
                    Profile {
                        id: "claude:a".into(),
                        agent: "claude".into(),
                        name: "Claude".into(),
                        config: json!({}),
                    },
                    true,
                ),
                (
                    Profile {
                        id: "codex:a".into(),
                        agent: "codex".into(),
                        name: "Codex".into(),
                        config: json!({}),
                    },
                    true,
                ),
            ],
        };
        let own = selected_profiles(rows(), "claude", &["claude:a".into()]).unwrap();
        assert_eq!(own.len(), 1);
        assert_eq!(own[0].agent, "claude");
        for ids in [
            vec!["codex:a".into()],
            vec!["claude:a".into(), "codex:a".into()],
        ] {
            assert!(selected_profiles(rows(), "claude", &ids)
                .err()
                .unwrap()
                .contains("当前 Agent"));
        }
        for ids in [
            vec![],
            vec!["claude:missing".into()],
            vec!["claude:a".into(), "claude:a".into()],
        ] {
            assert!(selected_profiles(rows(), "claude", &ids).is_err());
        }
        let mut invalid = rows();
        invalid.profiles[0].0.agent = "codex".into();
        assert!(selected_profiles(invalid, "claude", &["claude:a".into()]).is_err());
        assert!(read(Some("missing.db".into()), "unknown")
            .err()
            .unwrap()
            .contains("不支持的 Agent"));
    }
    #[test]
    fn sqlite_scan_and_import_are_read_only_and_filter_other_agents() {
        let dir = std::env::temp_dir().join(format!("supercode-cc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cc-switch.db");
        let c = Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE providers(id TEXT,app_type TEXT,name TEXT,settings_config TEXT,is_current INTEGER); INSERT INTO providers VALUES('a','claude','中文','{\"env\":{\"ANTHROPIC_MODEL\":\"m\"}}',1);").unwrap();
        c.execute(
            "INSERT INTO providers VALUES('a','codex','Codex',?1,1)",
            [json!({"config": "model='m'"}).to_string()],
        )
        .unwrap();
        for (agent, config) in [
            (
                "opencode",
                json!({"npm":"@ai-sdk/anthropic","options":{"baseURL":"https://gateway.example/v1","apiKey":"test-secret"},"models":{"m":{"name":"模型"}}}),
            ),
            (
                "pi",
                json!({"api":"openai-completions","baseUrl":"http://localhost:11434/v1","apiKey":"test-secret","models":[{"id":"m","contextWindow":32768}]}),
            ),
        ] {
            c.execute(
                "INSERT INTO providers VALUES('a',?1,?1,?2,1)",
                rusqlite::params![agent, config.to_string()],
            )
            .unwrap();
        }
        drop(c);
        let before = std::fs::read(&path).unwrap();
        let rows = read(Some(path.to_string_lossy().into()), "claude").unwrap();
        assert_eq!(rows.profiles.len(), 1);
        assert_eq!(rows.profiles[0].0.name, "中文");
        assert!(rows.profiles[0].1);
        assert!(rows.warnings.is_empty());
        let codex = scan_ccswitch(Some(path.to_string_lossy().into()), "codex".into()).unwrap();
        assert_eq!(codex.profiles.len(), 1);
        assert_eq!(codex.profiles[0].id, "codex:a");
        assert_eq!(codex.profiles[0].agent, "codex");
        for agent in ["opencode", "pi"] {
            let scan = scan_ccswitch(Some(path.to_string_lossy().into()), agent.into()).unwrap();
            assert_eq!(scan.profiles.len(), 1);
            assert_eq!(scan.profiles[0].id, format!("{agent}:a"));
            assert_eq!(scan.profiles[0].model.as_deref(), Some("m"));
            assert_eq!(scan.profiles[0].models, ["m"]);
            assert!(scan.profiles[0].has_credential);
            assert!(!scan.profiles[0].official_account);
            assert!(!serde_json::to_string(&scan)
                .unwrap()
                .contains("test-secret"));
            assert!(scan.warnings.is_empty());
        }
        assert_eq!(before, std::fs::read(&path).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn native_import_on_a_fresh_device_survives_reopen_without_changing_ccswitch() {
        let dir =
            std::env::temp_dir().join(format!("supercode-原生导入 空格-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let config = json!({
            "opencode":{"providers":{"same":{"name":"OpenCode 连接","settingsConfig":{"provider":{"same":{"npm":"@ai-sdk/openai","options":{"baseURL":"https://open.example/v1","apiKey":"open-secret","headers":{"X-Project":"project-secret"}},"models":{"m":{"name":"测试模型","limit":{"context":200000,"output":8192}}}}},"model":"same/m"}}}},
            "pi":{"providers":{"same":{"name":"Pi 连接","settingsConfig":{"baseUrl":"https://pi.example/v1","api":"openai-completions","apiKey":"pi-secret","models":[{"id":"m","reasoning":false,"input":["text"],"contextWindow":32768,"maxTokens":4096}]}}}}
        });
        std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        let before = std::fs::read(&path).unwrap();
        let db = dir.join("supercode.db");
        let store = crate::storage::Store::open(&db).unwrap();
        for agent in ["opencode", "pi"] {
            let rows = read(Some(path.to_string_lossy().into()), agent).unwrap();
            assert!(rows.warnings.is_empty());
            assert_eq!(rows.profiles.len(), 1);
            let profiles = selected_profiles(rows, agent, &[format!("{agent}:same")]).unwrap();
            store.import_profiles(&profiles).unwrap();
            store.select_profile(agent, Some(&profiles[0].id)).unwrap();
        }
        let project = store.add_project(&dir).unwrap();
        store.create_configured_session(&project.id, None, "pi", Some("pi:same")).unwrap();
        let mut updated = store.active_profile("pi").unwrap().unwrap();
        updated.config["piProvider"]["models"][0]["baseUrl"] = json!("https://changed.example/v1");
        assert!(store.import_profiles(&[updated]).unwrap_err().contains("已绑定聊天"));
        drop(store);
        let store = crate::storage::Store::open(&db).unwrap();
        let open = store.route("opencode", None).unwrap().config;
        let pi = store.route("pi", None).unwrap().config;
        assert_eq!(open["apiKey"], "open-secret");
        assert_eq!(open["protocol"], "responses");
        assert_eq!(
            open["opencodeProvider"]["models"]["m"]["limit"]["context"],
            200000
        );
        assert_eq!(pi["apiKey"], "pi-secret");
        assert_eq!(pi["piProvider"]["models"][0]["contextWindow"], 32768);
        for agent in ["opencode", "pi"] {
            let profile = store.active_profile(agent).unwrap().unwrap();
            assert!(!profile.is_official());
            assert_eq!(
                crate::providers::catalog_models(&profile.config)["data"][0]["id"],
                "m"
            );
            assert!(!serde_json::to_string(&profile.summary(true))
                .unwrap()
                .contains("secret"));
        }
        drop(store);
        #[cfg(windows)]
        assert!(!std::fs::read(&db)
            .unwrap()
            .windows(b"pi-secret".len())
            .any(|bytes| bytes == b"pi-secret"));
        assert_eq!(before, std::fs::read(&path).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
