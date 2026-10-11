//! App-owned model catalogs. No agent process or RPC is used for discovery.
use crate::AppState;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io::Read, path::PathBuf, time::Duration};
use tauri::{AppHandle, Manager};

// Tokens stay in Rust and are sent only to this documented official endpoint.
const ENDPOINT: &str = "https://api.openai.com/v1/models";
struct Login {
    access_token: String,
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
        access_token: access_token.into(),
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
    })).collect();
    json!({"data":models,"origin":"preset","notice":"官方模型目录 · 实际可用范围由账号权限决定"})
}
fn parse_catalog(value: &Value) -> Result<Value, String> {
    let rows = value["models"].as_array().ok_or("官方接口未返回模型目录")?;
    if rows.len() > 200 {
        return Err("官方模型目录超过大小限制".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut models = Vec::new();
    for row in rows.iter().filter(|row| row["visibility"] == "list") {
        let id = row["slug"]
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
        let efforts: Vec<_> = row["supported_reasoning_levels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|level| {
                level["effort"].as_str().map(
                    |effort| json!({"reasoningEffort":effort,"description":level["description"]}),
                )
            })
            .collect();
        models.push(json!({"id":id,"model":id,"displayName":row["display_name"].as_str().unwrap_or(id),"isDefault":models.is_empty(),"defaultReasoningEffort":row["default_reasoning_level"],"supportedReasoningEfforts":efforts}));
    }
    Ok(json!({"data":models,"origin":"officialApi"}))
}
async fn fetch(client: &reqwest::Client, login: &Login, endpoint: &str) -> Result<Value, String> {
    let response = client
        .get(endpoint)
        .bearer_auth(&login.access_token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| "无法连接官方模型接口".to_owned())?;
    if !response.status().is_success() {
        return Err(format!(
            "官方模型接口返回 HTTP {}",
            response.status().as_u16()
        ));
    }
    parse_catalog(&crate::providers::read_json(response).await?)
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
    let mut local = saved_or_preset(saved.as_deref());
    if !refresh {
        return Ok(local);
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("SuperCode/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "无法初始化官方模型接口")?;
    match fetch(&client, &login, ENDPOINT).await {
        Ok(catalog) => {
            store.set_setting(&key, &catalog.to_string())?;
            Ok(catalog)
        }
        Err(error) => {
            local["warning"] = format!("{error}，当前显示本地模型目录").into();
            Ok(local)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn http_catalog_uses_oauth_and_reports_failures_without_leaking_the_response_or_redirecting(
    ) {
        use axum::{
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
            routing::get,
            Json, Router,
        };
        let router = Router::new().route("/models", get(|headers: HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer private-token");
            Json(json!({"models":[{"slug":"server-model","display_name":"Server Model","visibility":"list"}]}))
        })).route("/denied", get(|| async { (StatusCode::FORBIDDEN, "private response with private-token").into_response() }))
        .route("/redirect", get(|| async { axum::response::Redirect::temporary("/models") }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let login = Login {
            access_token: "private-token".into(),
            account: "account".into(),
            plan: None,
        };
        let catalog = fetch(&client, &login, &format!("http://{address}/models"))
            .await
            .unwrap();
        assert_eq!(catalog["data"][0]["model"], "server-model");
        assert_eq!(catalog["origin"], "officialApi");
        let error = fetch(&client, &login, &format!("http://{address}/denied"))
            .await
            .unwrap_err();
        assert!(error.contains("403"));
        assert!(!error.contains("private"));
        assert!(
            fetch(&client, &login, &format!("http://{address}/redirect"))
                .await
                .unwrap_err()
                .contains("307")
        );
        server.abort();
    }
    #[test]
    fn full_official_preset_is_app_owned_and_has_distinct_effort_capabilities() {
        let data = preset();
        let rows = data["data"].as_array().unwrap();
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0]["model"], "gpt-6.1-sol");
        assert_eq!(
            rows.iter().filter(|row| row["isDefault"] == true).count(),
            1
        );
        assert!(rows[0]["supportedReasoningEfforts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["reasoningEffort"] == "ultra"));
        assert!(!rows[3]["supportedReasoningEfforts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["reasoningEffort"] == "ultra"));
    }
    #[test]
    fn direct_catalog_uses_visible_server_order_and_an_empty_catalog_does_not_restore_presets() {
        let catalog=parse_catalog(&json!({"models":[{"slug":"hidden","visibility":"hide"},{"slug":"new","display_name":"New model","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high"}]},{"slug":"old","visibility":"list"}]})).unwrap();
        assert_eq!(catalog["data"][0]["model"], "new");
        assert_eq!(catalog["data"][1]["model"], "old");
        assert_eq!(
            catalog["data"][0]["supportedReasoningEfforts"][0]["reasoningEffort"],
            "high"
        );
        let empty = parse_catalog(&json!({"models":[]})).unwrap();
        assert_eq!(saved_or_preset(Some(&empty.to_string()))["data"], json!([]));
        assert!(parse_catalog(&json!({"error":"secret"}))
            .unwrap_err()
            .find("secret")
            .is_none());
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
            access_token: "different-token".into(),
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
