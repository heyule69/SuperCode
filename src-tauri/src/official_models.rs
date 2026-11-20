//! Fast local discovery; explicit refresh uses the same native catalog as execution.
use crate::AppState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io::Read, path::PathBuf};
use tauri::{AppHandle, Manager};

struct Login {
    account: String,
    plan: Option<String>,
}
fn home(app: &AppHandle, account_id: Option<&str>) -> Result<PathBuf, String> {
    if let Some(id) = account_id {
        return crate::official_accounts::directory(app, "codex", id);
    }
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|path| PathBuf::from(path).join(".codex"))
        })
        .ok_or_else(|| "无法找到官方登录目录".into())
}
fn read_login(path: &std::path::Path) -> Result<Option<Login>, String> {
    let file = match std::fs::File::open(path.join("auth.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取官方登录文件".into()),
    };
    let mut bytes = Vec::new();
    file.take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取官方登录文件")?;
    if bytes.len() > 256 * 1024 {
        return Err("官方登录文件超过大小限制".into());
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|_| "官方登录文件不是 UTF-8，未转换或修改编码")?;
    if text.starts_with('\u{feff}') {
        return Err("官方登录文件含 BOM，未转换或修改编码".into());
    }
    let value: Value = serde_json::from_str(text).map_err(|_| "官方登录文件格式无效")?;
    parse_login(&value)
}
fn parse_login(value: &Value) -> Result<Option<Login>, String> {
    if value["auth_mode"]
        .as_str()
        .is_some_and(|mode| !matches!(mode, "chatgpt" | "chatgptAuthTokens"))
    {
        return Ok(None);
    }
    let Some(access_token) = value["tokens"]["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    if access_token.len() > 64 * 1024 || access_token.chars().any(char::is_whitespace) {
        return Err("官方登录令牌格式无效".into());
    }
    let Some(account) = value["tokens"]["account_id"]
        .as_str()
        .filter(|s| !s.is_empty())
    else {
        return Ok(None);
    };
    // JWT claims are display metadata, never an authorization or entitlement check.
    let plan = value["tokens"]["id_token"]
        .as_str()
        .and_then(|token| token.split('.').nth(1))
        .and_then(|claims| {
            use base64::Engine;
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(claims)
                .ok()
        })
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|claims| {
            claims["https://api.openai.com/auth"]["chatgpt_plan_type"]
                .as_str()
                .map(str::to_owned)
        });
    Ok(Some(Login {
        account: account.into(),
        plan,
    }))
}
pub(crate) fn status(app: &AppHandle, account_id: Option<&str>) -> Result<Value, String> {
    Ok(match read_login(&home(app, account_id)?)? {
        Some(login) => json!({"loggedIn":true,"method":"chatgpt","plan":login.plan}),
        None => json!({"loggedIn":false,"method":null,"plan":null}),
    })
}
fn cache_key(scope: &str, login: &Login) -> String {
    // A switch of the native ChatGPT login cannot reuse another account's catalog.
    let digest = Sha256::digest(format!("{scope}\0{}", login.account).as_bytes());
    format!("official_models:{digest:x}")
}
pub(crate) fn preset() -> Value {
    let catalog: Value = serde_json::from_str(include_str!("../../resources/official-models.json"))
        .expect("bundled official model catalog");
    let models: Vec<_> = catalog["models"].as_array().unwrap().iter().map(|model| json!({
        "id":model["model"],"model":model["model"],"displayName":model["displayName"],"isDefault":model["isDefault"]==true,
        "defaultReasoningEffort":model["defaultReasoningEffort"],"supportedReasoningEfforts":model["efforts"].as_array().unwrap().iter().map(|effort| json!({"reasoningEffort":effort})).collect::<Vec<_>>()
        ,"contextWindow":model["contextWindow"],"effectiveContextWindowPercent":model["effectiveContextWindowPercent"],"serviceTiers":model["serviceTiers"]
    })).collect();
    json!({"data":models,"origin":"preset","notice":"官方模型目录 · 实际可用范围由账号权限决定"})
}
fn parse_catalog(value: &Value) -> Result<Value, String> {
    let rows = value["data"].as_array().ok_or("Codex 未返回模型目录")?;
    if rows.len() > 200 {
        return Err("官方模型目录超过大小限制".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut models = Vec::new();
    for row in rows {
        let id = row["model"]
            .as_str()
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= 160
                    && id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            })
            .ok_or("官方模型标识无效")?;
        if !seen.insert(id) {
            continue;
        }
        let efforts: Vec<_> = row["supportedReasoningEfforts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|level| {
                level["reasoningEffort"].as_str().map(|effort| json!({"reasoningEffort":effort}))
            })
            .collect();
        let mut parsed = crate::model_context::metadata(row);
        let tiers: Vec<_> = ["fast", "ultrafast"].into_iter().filter(|tier| crate::codex_speed::supports(row, tier))
            .map(|tier| json!({"id":tier,"name":if tier == "fast" {"Fast"} else {"Ultrafast"}})).collect();
        parsed.as_object_mut().unwrap().extend(json!({"id":id,"model":id,"displayName":row["displayName"].as_str().unwrap_or(id),"isDefault":row["isDefault"]==true,"defaultReasoningEffort":row["defaultReasoningEffort"],"supportedReasoningEfforts":efforts,"serviceTiers":tiers}).as_object().unwrap().clone());
        models.push(parsed);
    }
    Ok(json!({"data":models,"origin":"native"}))
}
fn saved_or_preset(saved: Option<&str>) -> Value {
    saved
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .filter(|catalog| {
            catalog["data"]
                .as_array()
                .is_some_and(|rows| rows.len() <= 200)
        })
        .map(|mut catalog| {
            catalog["origin"] = "saved".into();
            catalog
        })
        .unwrap_or_else(preset)
}
pub(crate) async fn catalog(
    app: &AppHandle,
    account_id: Option<&str>,
    refresh: bool,
) -> Result<Value, String> {
    let login = read_login(&home(app, account_id)?)?.ok_or("请先添加并登录官方账号")?;
    let key = cache_key(account_id.unwrap_or("native"), &login);
    let state = app.state::<AppState>();
    let store = &state.store;
    let saved = store.setting(&key)?;
    if !refresh {
        return Ok(saved_or_preset(saved.as_deref()));
    }
    let catalog = parse_catalog(&crate::codex_speed::read_catalog(app, account_id).await?)?;
    store.set_setting(&key, &catalog.to_string())?;
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_official_preset_is_app_owned_and_has_distinct_effort_capabilities() {
        let data = preset();
        let rows = data["data"].as_array().unwrap();
        assert_eq!(rows.len(), 9);
        assert_eq!(rows[0]["model"], "gpt-6.2-sol");
        assert_eq!(rows[0]["contextWindow"], 500000);
        assert_eq!(rows[0]["effectiveContextWindowPercent"], 95);
        assert_eq!(
            rows.iter().filter(|row| row["isDefault"] == true).count(),
            1
        );
        assert!(rows[0]["supportedReasoningEfforts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["reasoningEffort"] == "ultra"));
        assert!(!rows.iter().find(|row| row["model"] == "gpt-6-luna").unwrap()["supportedReasoningEfforts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["reasoningEffort"] == "ultra"));
        assert_eq!(rows.iter().find(|row| row["isDefault"] == true).unwrap()["model"],"gpt-6.1-sol");
        let astra = rows.iter().find(|row| row["model"] == "gpt-6.2-astra").unwrap();
        assert_eq!(astra["contextWindow"],272000);
        assert!(crate::codex_speed::supports(&rows[0],"ultrafast"));
        assert!(crate::codex_speed::supports(astra,"ultrafast"));
    }
    #[test]
    fn native_catalog_preserves_server_order_defaults_and_empty_results() {
        let catalog=parse_catalog(&json!({"data":[{"model":"new","displayName":"New model","isDefault":false,"defaultReasoningEffort":"high","supportedReasoningEfforts":[{"reasoningEffort":"high"}]},{"model":"old","isDefault":true}]})).unwrap();
        assert_eq!(catalog["data"][0]["model"], "new");
        assert_eq!(catalog["data"][1]["model"], "old");
        assert_eq!(catalog["data"][0]["isDefault"], false);
        assert_eq!(catalog["data"][1]["isDefault"], true);
        assert_eq!(catalog["origin"], "native");
        assert_eq!(
            catalog["data"][0]["supportedReasoningEfforts"][0]["reasoningEffort"],
            "high"
        );
        let empty = parse_catalog(&json!({"data":[]})).unwrap();
        assert_eq!(saved_or_preset(Some(&empty.to_string()))["data"], json!([]));
        assert!(parse_catalog(&json!({"error":"secret"}))
            .unwrap_err()
            .find("secret")
            .is_none());
    }
    #[test]
    fn native_capacity_is_preserved_without_exposing_extra_fields() {
        let catalog=parse_catalog(&json!({"data":[{"model":"gpt-6.1-sol","contextWindow":600000,"effectiveContextWindowPercent":95,"private":"secret"}]})).unwrap();
        assert_eq!(catalog["data"][0]["contextWindow"],600000);
        assert_eq!(catalog["data"][0]["effectiveContextWindowPercent"],95);
        assert!(!catalog.to_string().contains("secret"));
    }
    #[test]
    fn speed_capabilities_are_preserved_without_inventing_unsupported_tiers() {
        let catalog = parse_catalog(&json!({"data":[{"model":"model","serviceTiers":[{"id":"priority","name":"Fast","private":"secret"},{"id":"ultrafast","name":"Ultrafast"}]}]})).unwrap();
        assert_eq!(catalog["data"][0]["serviceTiers"], json!([{"id":"fast","name":"Fast"},{"id":"ultrafast","name":"Ultrafast"}]));
        assert!(!catalog.to_string().contains("secret"));
        assert_eq!(parse_catalog(&json!({"data":[{"model":"model"}]})).unwrap()["data"][0]["serviceTiers"],json!([]));
        assert!(preset()["data"].as_array().unwrap().iter().all(crate::codex_speed::supports_fast));
    }
    #[test]
    fn local_official_status_never_treats_api_keys_as_official_login_or_leaks_tokens() {
        assert!(
            parse_login(&json!({"auth_mode":"apikey","OPENAI_API_KEY":"private-key"}))
                .unwrap()
                .is_none()
        );
        assert!(parse_login(&json!({"tokens":{"access_token":"secret"}}))
            .unwrap()
            .is_none());
        let a=parse_login(&json!({"auth_mode":"chatgpt","tokens":{"access_token":"private-token","account_id":"a"}})).unwrap().unwrap();
        let b = Login {
            account: "b".into(),
            plan: None,
        };
        assert_ne!(cache_key("native", &a), cache_key("native", &b));
        assert_ne!(cache_key("native", &a), cache_key("managed", &a));
        assert!(!cache_key("native", &a).contains("private-token"));
    }
    #[test]
    fn login_file_is_bounded_utf8_and_no_cli_is_needed() {
        let dir = std::env::temp_dir().join(format!(
            "supercode-official-models-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_login(&dir).unwrap().is_none());
        std::fs::write(dir.join("auth.json"), b"\xff\xfe").unwrap();
        match read_login(&dir) {
            Err(error) => assert!(error.contains("UTF-8")),
            Ok(_) => panic!("invalid UTF-8 accepted"),
        }
        std::fs::write(dir.join("auth.json"),br#"{"auth_mode":"chatgpt","tokens":{"access_token":"private-token","account_id":"account"}}"#).unwrap();
        assert!(read_login(&dir).unwrap().is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
