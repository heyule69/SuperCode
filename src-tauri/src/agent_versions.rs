//! On-demand stable release checks. Four small cache entries, no agent startup.
use crate::agents::IDS;
use futures_util::{future::join_all, StreamExt};
use semver::Version;
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Latest {
    pub id: String,
    pub latest_version: Option<String>,
    pub checked_at: Option<i64>,
    pub error: Option<String>,
}
struct Cached {
    value: Latest,
    attempted_at: i64,
}
pub struct Versions {
    http: reqwest::Client,
    cache: Mutex<BTreeMap<String, Cached>>,
    fetches: [Mutex<()>; 4],
}
impl Default for Versions {
    fn default() -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(12))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("SuperCode/0.1.0")
                .build()
                .expect("版本查询客户端初始化失败"),
            cache: Mutex::new(BTreeMap::new()),
            fetches: std::array::from_fn(|_| Mutex::new(())),
        }
    }
}
pub fn package_name(id: &str) -> Result<&'static str, String> {
    match id {
        "codex" => Ok("@openai/codex"),
        "claude" => Ok("@anthropic-ai/claude-code"),
        "opencode" => Ok("opencode-ai"),
        "pi" => Ok("@earendil-works/pi-coding-agent"),
        _ => Err("Agent 无效".into()),
    }
}
pub fn installed_version(text: &str) -> Option<Version> {
    text.split_whitespace().find_map(|part| {
        Version::parse(
            part.trim_matches(|c: char| c == '(' || c == ')' || c == ',')
                .trim_start_matches('v'),
        )
        .ok()
    })
}
pub fn package_spec(id: &str, version: &str) -> Result<String, String> {
    let parsed = Version::parse(version).map_err(|_| "发布源返回的版本无效")?;
    if version.len() > 80 || !parsed.pre.is_empty() || parsed.to_string() != version {
        return Err("发布源未提供有效的稳定版本".into());
    }
    Ok(format!("{}@{version}", package_name(id)?))
}
fn metadata_version(id: &str, value: &Value) -> Result<String, String> {
    if value["name"].as_str() != Some(package_name(id)?) {
        return Err("发布源返回的 Agent 不匹配".into());
    }
    let version = value["version"].as_str().ok_or("发布源未返回版本")?;
    package_spec(id, version)?;
    Ok(version.into())
}
impl Versions {
    async fn fetch(&self, id: &str) -> Result<String, String> {
        let url = format!("https://registry.npmjs.org/{}/latest", package_name(id)?);
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| "无法检查最新版本，请检查网络后重试")?
            .error_for_status()
            .map_err(|_| "发布源暂不可用，请稍后重试")?;
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "版本查询中断，请重试")?;
            if bytes.len() + chunk.len() > 256 * 1024 {
                return Err("版本数据超过大小限制".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "版本数据格式无效")?;
        metadata_version(id, &value)
    }
    pub async fn get(&self, id: &str, force: bool) -> Result<Latest, String> {
        let index = IDS.iter().position(|v| *v == id).ok_or("Agent 无效")?;
        let _fetch = self.fetches[index].lock().await;
        let previous = {
            let cache = self.cache.lock().await;
            if let Some(cached) = cache.get(id) {
                let ttl = if cached.value.error.is_some() {
                    30
                } else {
                    900
                };
                if !force && crate::storage::now().saturating_sub(cached.attempted_at) < ttl {
                    return Ok(cached.value.clone());
                }
            }
            cache.get(id).map(|c| c.value.clone())
        };
        let at = crate::storage::now();
        let value = match self.fetch(id).await {
            Ok(version) => Latest {
                id: id.into(),
                latest_version: Some(version),
                checked_at: Some(at),
                error: None,
            },
            Err(error) => Latest {
                id: id.into(),
                latest_version: previous.as_ref().and_then(|v| v.latest_version.clone()),
                checked_at: previous.and_then(|v| v.checked_at),
                error: Some(error),
            },
        };
        self.cache.lock().await.insert(
            id.into(),
            Cached {
                value: value.clone(),
                attempted_at: at,
            },
        );
        Ok(value)
    }
    pub async fn target(&self, id: &str, expected: Option<&str>) -> Result<String, String> {
        let latest = self.get(id, false).await?;
        if let Some(error) = latest.error {
            return Err(error);
        }
        let version = latest.latest_version.ok_or("未取得最新版本，请重新检查")?;
        if expected.is_some_and(|v| v != version) {
            return Err("最新版本已变化，请重新检查后再安装".into());
        }
        Ok(version)
    }
}
#[tauri::command]
pub async fn check_agent_updates(
    force: Option<bool>,
    app: AppHandle,
) -> Result<Vec<Latest>, String> {
    let versions = app.state::<Versions>();
    join_all(
        IDS.iter()
            .map(|id| versions.get(id, force.unwrap_or(false))),
    )
    .await
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn release_versions_are_pinned_to_allowlisted_packages() {
        assert_eq!(
            package_spec("codex", "0.162.0").unwrap(),
            "@openai/codex@0.162.0"
        );
        for version in [
            "latest",
            "v1.0.0",
            "1.0.0-beta.1",
            "1.0.0 --global",
            "1.0.0;exit",
            "01.0.0",
        ] {
            assert!(package_spec("pi", version).is_err());
        }
        assert!(package_spec("arbitrary", "1.0.0").is_err());
        assert!(
            metadata_version("codex", &json!({"name":"opencode-ai","version":"1.0.0"})).is_err()
        );
        assert!(metadata_version("pi", &json!({"name":package_name("pi").unwrap()})).is_err());
        assert_eq!(
            metadata_version(
                "pi",
                &json!({"name":package_name("pi").unwrap(),"version":"1.0.4"})
            )
            .unwrap(),
            "1.0.4"
        );
    }
    #[test]
    fn native_banners_compare_semantically_including_prereleases() {
        assert!(
            installed_version("codex-cli 0.160.1").unwrap() < Version::parse("0.162.0").unwrap()
        );
        assert!(
            installed_version("2.1.99 (Claude Code)").unwrap() < Version::parse("2.1.295").unwrap()
        );
        assert_eq!(
            installed_version("v1.0.4\n"),
            Some(Version::parse("1.0.4").unwrap())
        );
        assert!(installed_version("1.0.4-beta.1").unwrap() < Version::parse("1.0.4").unwrap());
        assert!(installed_version("1.0.4+build.1")
            .unwrap()
            .cmp_precedence(&Version::parse("1.0.4").unwrap())
            .is_eq());
        assert!(installed_version("unknown").is_none());
    }
}
