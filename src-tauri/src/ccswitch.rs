//! Read-only CC Switch SQLite import. Credentials stay on the Rust side.
use crate::AppState;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

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
    pub current: bool,
    pub provider_id: Option<String>,
    pub protocol: String,
    pub plan: Option<String>,
    pub source: String,
    pub models: Vec<String>,
    pub model_source: Value,
}
impl Profile {
    pub fn is_official(&self) -> bool {
        if crate::providers::key(&self.config).is_some() {
            return false;
        }
        if self.agent == "claude" {
            return self.config["env"]["ANTHROPIC_BASE_URL"]
                .as_str()
                .is_none_or(str::is_empty);
        }
        self.config["config"]
            .as_str()
            .and_then(|s| toml::from_str::<toml::Table>(s).ok())
            .is_some_and(|t| {
                t.get("model_provider").and_then(toml::Value::as_str) == Some("openai")
            })
    }
    pub fn summary(&self, current: bool) -> Summary {
        Summary {
            official_account: self.is_official(),
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
        .map(PathBuf::from)
        .ok_or("无法定位用户目录".into())
}
fn database(path: Option<String>) -> Result<PathBuf, String> {
    let mut path = match path.filter(|s| !s.trim().is_empty()) {
        Some(p) => PathBuf::from(p.trim()),
        None => std::env::var_os("CC_SWITCH_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or(home()?.join(".cc-switch")),
    };
    if path.is_dir() {
        path = path.join("cc-switch.db");
    }
    if !path.is_file() {
        return Err("未找到 CC Switch 的 cc-switch.db，可输入数据库路径或所在目录".into());
    }
    Ok(path)
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
fn sanitize(agent: &str, raw: Value) -> Result<Value, String> {
    if agent == "claude" {
        let mut safe = json!({"env":claude_env(&raw),"model":raw["model"],"source":"ccswitch"});
        safe["model"] = json!(crate::providers::real_claude_model(&safe, None));
        return Ok(safe);
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
    let key = raw["auth"]["OPENAI_API_KEY"]
        .as_str()
        .filter(|s| !s.is_empty());
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
                .and_then(toml::Value::as_table)
                .ok_or("Codex 缺少供应商连接配置")?;
            let mut connection = toml::Table::new();
            for key in ["name", "base_url", "wire_api"] {
                if let Some(v) = source.get(key).filter(|v| v.is_str()) {
                    connection.insert(key.into(), v.clone());
                }
            }
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
    } else if key.is_some() {
        return Err("此 Codex API Key 配置未指定供应商，暂不能导入".into());
    }
    if !safe.contains_key("model_provider") {
        safe.insert(
            "model_provider".into(),
            toml::Value::String("openai".into()),
        );
    }
    Ok(
        json!({"config":toml::to_string(&safe).map_err(|_|"配置序列化失败")?,"model":safe.get("model").and_then(toml::Value::as_str),"apiKey":key}),
    )
}
fn read(path: Option<String>) -> Result<Vec<(Profile, bool)>, String> {
    let connection = Connection::open_with_flags(database(path)?, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "无法以只读方式打开 CC Switch 数据库")?;
    connection
        .busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|e| e.to_string())?;
    let mut stmt=connection.prepare("SELECT id,app_type,name,settings_config,is_current FROM providers WHERE app_type IN ('claude','codex') ORDER BY app_type,name LIMIT 500").map_err(|_|"数据库不是受支持的 CC Switch providers 格式")?;
    let rows = stmt
        .query_map([], |r| {
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
    for row in rows {
        let (id, agent, name, raw, current) =
            row.map_err(|_| "CC Switch 数据含无法读取的 UTF-8 文本，未转换或修改原文件")?;
        if raw.len() > 1024 * 1024 {
            return Err(format!("{name} 配置超过 1 MB"));
        }
        let raw: Value =
            serde_json::from_str(&raw).map_err(|_| format!("{name} 配置 JSON 无效"))?;
        profiles.push((
            Profile {
                id: format!("{agent}:{id}"),
                config: sanitize(&agent, raw).map_err(|e| format!("{name}：{e}"))?,
                agent,
                name,
            },
            current,
        ));
    }
    Ok(profiles)
}
#[tauri::command]
pub fn scan_ccswitch(path: Option<String>) -> Result<Vec<Summary>, String> {
    Ok(read(path)?.into_iter().map(|(p, c)| p.summary(c)).collect())
}
#[tauri::command]
pub fn import_ccswitch(
    path: Option<String>,
    ids: Vec<String>,
    app: AppHandle,
) -> Result<usize, String> {
    if app.state::<AppState>().store.running()? != 0 {
        return Err("请先完成或停止当前任务后导入配置".into());
    }
    if ids.is_empty() {
        return Err("请选择要导入的配置".into());
    }
    let all = read(path)?;
    let selected = all
        .into_iter()
        .filter(|(p, _)| ids.contains(&p.id))
        .map(|(p, _)| p)
        .collect::<Vec<_>>();
    if selected.len() != ids.len() {
        return Err("部分配置已变更，请重新扫描".into());
    }
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
    state.runtime.release(&app).await?;
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
    fn sqlite_scan_and_import_are_read_only_and_filter_other_agents() {
        let dir = std::env::temp_dir().join(format!("supercode-cc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cc-switch.db");
        let c = Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE providers(id TEXT,app_type TEXT,name TEXT,settings_config TEXT,is_current INTEGER); INSERT INTO providers VALUES('a','claude','中文','{\"env\":{\"ANTHROPIC_MODEL\":\"m\"}}',1); INSERT INTO providers VALUES('b','pi','Pi','{}',1);").unwrap();
        drop(c);
        let before = std::fs::read(&path).unwrap();
        let rows = read(Some(path.to_string_lossy().into())).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.name, "中文");
        assert!(rows[0].1);
        assert_eq!(before, std::fs::read(&path).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
