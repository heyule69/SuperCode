//! Remote allowance is distinct from local tokens. Unsupported/unavailable data is absent.
use crate::{providers, session_config::Route, AppState};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    label: String,
    remaining_percent: f64,
    reset_at: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct Balance {
    label: String,
    amount: f64,
    currency: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    connection_id: String,
    agent: String,
    provider_id: String,
    provider_name: String,
    connection_name: String,
    plan_name: Option<String>,
    windows: Vec<Window>,
    balances: Vec<Balance>,
    queried_at: u64,
}
#[derive(Default)]
pub struct UsageCache(Mutex<HashMap<(String, u64), (Instant, Option<Usage>)>>);
impl UsageCache {
    fn get(&self, key: &(String, u64)) -> Option<Option<Usage>> {
        self.0
            .lock()
            .ok()?
            .get(key)
            .filter(|(at, value)| {
                at.elapsed() < Duration::from_secs(if value.is_some() { 60 } else { 20 })
            })
            .map(|(_, value)| value.clone())
    }
    fn put(&self, key: (String, u64), value: Option<Usage>) {
        if let Ok(mut cache) = self.0.lock() {
            cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(300));
            if cache.len() >= 64 {
                cache.clear();
            }
            cache.insert(key, (Instant::now(), value));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Adapter {
    KimiCoding,
    KimiBalance,
    Glm,
    MiniMax,
    DeepSeek,
    OpenRouter,
}
fn endpoint(base: &str) -> Option<(Adapter, String)> {
    let url = reqwest::Url::parse(base).ok()?;
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let host = url.host_str()?;
    let path = url.path();
    let (kind, origin, route) = match host {
        "api.kimi.com" | "api.kimi.ai" if path.starts_with("/coding") => (
            Adapter::KimiCoding,
            "https://api.kimi.com",
            "/coding/v1/usages",
        ),
        "api.moonshot.cn" => (
            Adapter::KimiBalance,
            "https://api.moonshot.cn",
            "/v1/users/me/balance",
        ),
        "api.moonshot.ai" => (
            Adapter::KimiBalance,
            "https://api.moonshot.ai",
            "/v1/users/me/balance",
        ),
        "open.bigmodel.cn" => (
            Adapter::Glm,
            "https://open.bigmodel.cn",
            "/api/monitor/usage/quota/limit",
        ),
        "api.z.ai" => (
            Adapter::Glm,
            "https://api.z.ai",
            "/api/monitor/usage/quota/limit",
        ),
        "api.minimax.io" => (
            Adapter::MiniMax,
            "https://api.minimax.io",
            "/v1/api/openplatform/coding_plan/remains",
        ),
        "api.minimax.cn" | "api.minimaxi.com" => (
            Adapter::MiniMax,
            "https://api.minimaxi.com",
            "/v1/api/openplatform/coding_plan/remains",
        ),
        "api.deepseek.com" => (
            Adapter::DeepSeek,
            "https://api.deepseek.com",
            "/user/balance",
        ),
        "openrouter.ai" => (Adapter::OpenRouter, "https://openrouter.ai", "/api/v1/key"),
        _ => return None,
    };
    Some((kind, format!("{origin}{route}")))
}
fn number(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str()?.parse().ok())
        .filter(|n| n.is_finite())
}
fn percent(v: &Value) -> Option<f64> {
    number(v).filter(|n| (0.0..=100.0).contains(n))
}
fn reset(v: &Value) -> Option<String> {
    if let Some(n) = number(v).filter(|n| *n > 0.0) {
        return Some((if n >= 1e12 { n / 1000.0 } else { n }).trunc().to_string());
    }
    v.as_str()
        .filter(|s| s.len() <= 64 && s.contains('T'))
        .map(str::to_owned)
}
fn ratio(value: &Value, label: &str) -> Option<Window> {
    let limit = number(&value["limit"])?;
    let remaining = number(&value["remaining"])?;
    if limit <= 0.0 || remaining < 0.0 || remaining > limit {
        return None;
    }
    Some(Window {
        label: label.into(),
        remaining_percent: remaining / limit * 100.0,
        reset_at: reset(&value["resetTime"]),
    })
}
fn parse(kind: Adapter, v: &Value) -> (Vec<Window>, Vec<Balance>) {
    let mut windows = Vec::new();
    let mut balances = Vec::new();
    if v.get("error").is_some() || v["success"] == false {
        return (windows, balances);
    }
    match kind {
        Adapter::KimiCoding => {
            if let Some(items) = v["limits"].as_array() {
                for item in items.iter().take(8) {
                    if let Some(window) = ratio(&item["detail"], "短周期额度") {
                        windows.push(window);
                    }
                }
            }
            if let Some(window) = ratio(&v["usage"], "周额度") {
                windows.push(window);
            }
        }
        Adapter::Glm => {
            if let Some(items) = v["data"]["limits"].as_array() {
                for item in items.iter().take(12) {
                    let kind = item["type"].as_str().unwrap_or("").to_ascii_uppercase();
                    if !matches!(
                        kind.as_str(),
                        "TOKENS_LIMIT" | "CREDIT_LIMIT" | "TIME_LIMIT"
                    ) {
                        continue;
                    }
                    let Some(used) = percent(&item["percentage"]) else {
                        continue;
                    };
                    let unit = number(&item["unit"]);
                    let label = if kind == "TIME_LIMIT" {
                        "MCP 额度"
                    } else if unit == Some(3.0) {
                        "5 小时额度"
                    } else if unit == Some(6.0) {
                        "周额度"
                    } else {
                        "套餐额度"
                    };
                    windows.push(Window {
                        label: label.into(),
                        remaining_percent: 100.0 - used,
                        reset_at: reset(&item["nextResetTime"]),
                    });
                }
            }
        }
        Adapter::MiniMax => {
            if v["base_resp"]["status_code"]
                .as_i64()
                .is_some_and(|code| code != 0)
            {
                return (windows, balances);
            }
            if let Some(item) = v["model_remains"]
                .as_array()
                .and_then(|items| items.iter().find(|item| item["model_name"] == "general"))
            {
                if let Some(remaining) = percent(&item["current_interval_remaining_percent"]) {
                    windows.push(Window {
                        label: "5 小时额度".into(),
                        remaining_percent: remaining,
                        reset_at: reset(&item["end_time"]),
                    });
                }
                if number(&item["current_weekly_status"]) == Some(1.0) {
                    if let Some(remaining) = percent(&item["current_weekly_remaining_percent"]) {
                        windows.push(Window {
                            label: "周额度".into(),
                            remaining_percent: remaining,
                            reset_at: reset(&item["weekly_end_time"]),
                        });
                    }
                }
            }
        }
        Adapter::KimiBalance => {
            if let Some(amount) = number(&v["data"]["available_balance"]).filter(|n| *n >= 0.0) {
                balances.push(Balance {
                    label: "API 余额".into(),
                    amount,
                    currency: String::new(),
                });
            }
        }
        Adapter::DeepSeek => {
            if let Some(items) = v["balance_infos"].as_array() {
                for item in items.iter().take(4) {
                    if let (Some(amount), Some(currency)) = (
                        number(&item["total_balance"]).filter(|n| *n >= 0.0),
                        item["currency"]
                            .as_str()
                            .filter(|s| matches!(*s, "CNY" | "USD")),
                    ) {
                        balances.push(Balance {
                            label: "API 余额".into(),
                            amount,
                            currency: currency.into(),
                        });
                    }
                }
            }
        }
        Adapter::OpenRouter => {
            // The normal key exposes its own spending cap, not the account wallet.
            if let Some(amount) = number(&v["data"]["limit_remaining"]).filter(|n| *n >= 0.0) {
                balances.push(Balance {
                    label: "Key 预算余额".into(),
                    amount,
                    currency: "USD".into(),
                });
            }
        }
    }
    if kind == Adapter::Glm {
        windows.sort_by_key(|window| match window.label.as_str() {
            "5 小时额度" => 0,
            "周额度" => 1,
            "MCP 额度" => 2,
            _ => 3,
        });
    }
    (windows, balances)
}
fn codex_windows(v: &Value) -> Vec<Window> {
    let groups: Vec<&Value> = v["rateLimitsByLimitId"]
        .as_object()
        .filter(|map| !map.is_empty())
        .map(|map| map.values().collect())
        .unwrap_or_else(|| vec![&v["rateLimits"]]);
    let mut windows = Vec::new();
    for group in groups.into_iter().take(8) {
        for name in ["primary", "secondary"] {
            let value = &group[name];
            let Some(used) = percent(&value["usedPercent"]) else {
                continue;
            };
            let duration = number(&value["windowDurationMins"]).filter(|n| *n > 0.0);
            let label = duration
                .map(|m| {
                    if m == 10080.0 {
                        "周额度".into()
                    } else if m >= 1440.0 {
                        format!("{} 天额度", m / 1440.0)
                    } else if m < 60.0 {
                        format!("{m} 分钟额度")
                    } else {
                        format!("{} 小时额度", m / 60.0)
                    }
                })
                .unwrap_or_else(|| "账号额度".into());
            windows.push(Window {
                label,
                remaining_percent: 100.0 - used,
                reset_at: reset(&value["resetsAt"]),
            });
        }
    }
    windows
}
async fn query(agent: &str, route: &Route, app: &AppHandle) -> Option<Usage> {
    let source = route.source(agent);
    let mut plan_name = source["planName"].as_str().map(str::to_owned);
    let (windows, balances) = if agent == "codex"
        && (route.config["official"] == true
            || route.profile.as_ref().is_some_and(|p| p.is_official()))
    {
        let limits = if let Some(id) = route.config["accountId"].as_str() {
            if let Some(value) = app.state::<AppState>().runtime.existing_account_limits(route.fingerprint()).await { value }
            else { crate::accounts::read_codex_limits_for(app, Some(id)).await.ok()? }
        } else { app
            .state::<AppState>()
            .runtime
            .account_limits(app, route.fingerprint())
            .await? };
        plan_name = limits["rateLimits"]["planType"]
            .as_str()
            .or_else(|| limits["rateLimitsByLimitId"]["codex"]["planType"].as_str())
            .map(|plan| format!("ChatGPT {plan}"))
            .or(plan_name);
        (codex_windows(&limits), Vec::new())
    } else {
        let base = route.config["baseUrl"]
            .as_str()
            .or(route.config["env"]["ANTHROPIC_BASE_URL"].as_str())
            .map(str::to_owned)
            .or_else(|| {
                route
                    .profile
                    .as_ref()
                    .map(|p| providers::settings(p).base_url)
            })?;
        let (adapter, endpoint) = endpoint(&base)?;
        if adapter == Adapter::KimiCoding {
            plan_name = Some("Kimi Coding Plan".into());
        }
        if adapter == Adapter::MiniMax {
            plan_name = Some("MiniMax Coding Plan".into());
        }
        let key = providers::key(&route.config)?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("SuperCode/0.1.0")
            .build()
            .ok()?;
        let request = client.get(endpoint).header("Accept", "application/json");
        let request = if adapter == Adapter::Glm {
            request.header("Authorization", key)
        } else {
            request.bearer_auth(key)
        };
        let json = providers::read_json(request.send().await.ok()?)
            .await
            .ok()?;
        parse(adapter, &json)
    };
    if windows.is_empty() && balances.is_empty() {
        return None;
    }
    Some(Usage {
        connection_id: route.id.clone(),
        agent: agent.into(),
        provider_id: source["providerId"].as_str().unwrap_or("custom").into(),
        provider_name: source["providerName"]
            .as_str()
            .unwrap_or("模型供应商")
            .into(),
        connection_name: source["connectionName"]
            .as_str()
            .unwrap_or("本机连接")
            .into(),
        plan_name,
        windows,
        balances,
        queried_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs(),
    })
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaConnection {
    connection_id: String,
    agent: String,
    connection_name: String,
    source: Value,
}
fn connection(route: &Route, agent: &str) -> QuotaConnection {
    let source = route.source(agent);
    let name = source["connectionName"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(if route.id == crate::session_config::OFFICIAL {
            if agent == "claude" {
                "Claude 官方账号"
            } else {
                "OpenAI 官方账号"
            }
        } else {
            "本机 CLI 连接"
        });
    QuotaConnection {
        connection_id: route.id.clone(),
        agent: agent.into(),
        connection_name: name.into(),
        source,
    }
}
// Return safe display metadata; only official candidates need a cached login check.
#[tauri::command]
pub async fn get_quota_connections(app: AppHandle) -> Result<Vec<QuotaConnection>, String> {
    let handle = app.clone();
    let candidates = tauri::async_runtime::spawn_blocking(move || quota_connections(&handle.state::<AppState>().store))
        .await
        .map_err(|e| e.to_string())??;
    let mut visible = Vec::new();
    for item in candidates {
        let official = item.connection_id == crate::session_config::OFFICIAL
            || app.state::<AppState>().store.profile(&item.agent, &item.connection_id)?
                .is_some_and(|p| p.is_official());
        if !official || crate::chat_connection::check(&app, &item.agent, None, Some(&item.connection_id)).await.is_ok() {
            visible.push(item);
        }
    }
    Ok(visible)
}
fn quota_connections(store: &crate::storage::Store) -> Result<Vec<QuotaConnection>, String> {
    let profiles = store.profiles()?;
    let mut connections = Vec::new();
    for profile in &profiles {
        if profile.agent == "codex" && profile.is_official() && profile.account_id().is_none() {
            continue;
        }
        connections.push(connection(
            &Route {
                id: profile.id.clone(),
                config: profile.config.clone(),
                profile: Some(profile.clone()),
            },
            &profile.agent,
        ));
    }
    for agent in ["codex", "claude"] {
        if agent == "codex"
            && (store.official(agent)?
                || profiles.iter().any(|p| p.agent == agent && p.is_official() && p.account_id().is_none()))
            || agent != "codex"
                && store.official(agent)?
                && !profiles.iter().any(|p| p.agent == agent && p.is_official())
        {
            connections.push(connection(
                &store.route_for(agent, crate::session_config::OFFICIAL)?,
                agent,
            ));
        }
    }
    Ok(connections)
}
#[tauri::command]
pub async fn get_platform_usage(
    agent: String,
    connection_id: String,
    refresh: bool,
    app: AppHandle,
) -> Result<Option<Usage>, String> {
    let Some(route) = crate::chat_connection::available_route(&app, &agent, None, Some(&connection_id)).await? else {
        return Ok(None);
    };
    let cache_key = (agent.clone(), route.fingerprint());
    if !refresh {
        if let Some(value) = app.state::<UsageCache>().get(&cache_key) {
            return Ok(value);
        }
    }
    let value = query(&agent, &route, &app).await;
    app.state::<UsageCache>().put(cache_key, value.clone());
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn quota_candidates_exclude_hidden_native_routes_even_with_old_chat_history() {
        let store = crate::storage::Store::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(std::path::Path::new("D:/quota-empty-test")).unwrap();
        for agent in ["claude", "codex", "opencode", "pi"] {
            store.create_agent_session(&project.id, None, agent).unwrap();
        }
        assert!(store.profiles().unwrap().is_empty());
        assert!(quota_connections(&store).unwrap().iter().all(|item| {
            item.connection_id == crate::session_config::OFFICIAL && item.agent == "codex"
        }));
    }
    #[test]
    fn connection_list_keeps_unsupported_platforms_without_disclosing_credentials() {
        let route = Route {
            id: "custom-1".into(),
            config: json!({"providerId":"custom","baseUrl":"https://custom.example.test/api","apiKey":"must-not-leak"}),
            profile: Some(crate::ccswitch::Profile {
                id: "custom-1".into(),
                name: "我的连接".into(),
                agent: "claude".into(),
                config: Value::Null,
            }),
        };
        let item = connection(&route, "claude");
        assert_eq!(item.connection_name, "我的连接");
        assert_eq!(item.source["providerId"], "custom");
        assert_eq!(item.source["mark"], "我");
        let safe = serde_json::to_string(&item).unwrap();
        assert!(!safe.contains("must-not-leak") && !safe.contains("custom.example.test"));
        assert!(endpoint("https://custom.example.test/api").is_none());
    }
    #[test]
    fn endpoints_never_send_keys_to_unrelated_or_insecure_hosts() {
        for url in [
            "http://open.bigmodel.cn/api/anthropic",
            "https://open.bigmodel.cn.evil.test/api",
            "https://open.bigmodel.cn:444/api",
            "https://key@api.deepseek.com/v1",
            "https://api.siliconflow.cn/v1",
            "https://api.anthropic.com",
        ] {
            assert!(endpoint(url).is_none(), "{url}");
        }
        assert_eq!(
            endpoint("https://api.kimi.ai/coding/v1").unwrap().1,
            "https://api.kimi.com/coding/v1/usages"
        );
        assert_eq!(
            endpoint("https://api.deepseek.com/anthropic").unwrap().0,
            Adapter::DeepSeek
        );
    }
    #[test]
    fn unavailable_and_malformed_data_are_not_zero_allowances() {
        for adapter in [
            Adapter::KimiCoding,
            Adapter::Glm,
            Adapter::MiniMax,
            Adapter::KimiBalance,
            Adapter::DeepSeek,
            Adapter::OpenRouter,
        ] {
            let (w, b) = parse(adapter, &json!({}));
            assert!(w.is_empty() && b.is_empty());
        }
        let (w, _) = parse(
            Adapter::Glm,
            &json!({"success":false,"data":{"limits":[{"type":"TOKENS_LIMIT","percentage":0}]}}),
        );
        assert!(w.is_empty());
        let (w, _) = parse(
            Adapter::KimiCoding,
            &json!({"usage":{"limit":0,"remaining":10}}),
        );
        assert!(w.is_empty());
        let (_, b) = parse(
            Adapter::DeepSeek,
            &json!({"balance_infos":[{"currency":"CNY","total_balance":"NaN"}]}),
        );
        assert!(b.is_empty());
    }
    #[test]
    fn normalizes_real_zeros_used_percentages_and_resets() {
        let (w, _) = parse(
            Adapter::Glm,
            &json!({"data":{"limits":[{"type":"TOKENS_LIMIT","percentage":20,"unit":3,"nextResetTime":1800000000000_i64},{"type":"CREDIT_LIMIT","percentage":"100","unit":6}]}}),
        );
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].remaining_percent, 80.0);
        assert_eq!(w[1].remaining_percent, 0.0);
        assert_eq!(w[0].reset_at.as_deref(), Some("1800000000"));
        let (w, _) = parse(
            Adapter::KimiCoding,
            &json!({"usage":{"limit":"100","remaining":"35","resetTime":"2026-10-08T00:00:00Z"}}),
        );
        assert_eq!(w[0].remaining_percent, 35.0);
        let (_, b) = parse(
            Adapter::DeepSeek,
            &json!({"is_available":false,"balance_infos":[{"currency":"CNY","total_balance":"0.00"}]}),
        );
        assert_eq!(b[0].amount, 0.0);
    }
    #[test]
    fn weekly_minimax_and_openrouter_budget_are_not_account_balance() {
        let (w, _) = parse(
            Adapter::MiniMax,
            &json!({"base_resp":{"status_code":0},"model_remains":[{"model_name":"general","current_interval_remaining_percent":66,"current_weekly_status":1,"current_weekly_remaining_percent":42}]}),
        );
        assert_eq!(w.len(), 2);
        assert_eq!(w[1].remaining_percent, 42.0);
        let (_, b) = parse(
            Adapter::OpenRouter,
            &json!({"data":{"limit_remaining":10,"label":"secret-key-label"}}),
        );
        assert_eq!(b[0].label, "Key 预算余额");
        assert!(!serde_json::to_string(&b).unwrap().contains("secret"));
        assert!(parse(
            Adapter::OpenRouter,
            &json!({"data":{"limit_remaining":null,"usage":10}})
        )
        .1
        .is_empty());
    }
    #[test]
    fn codex_windows_are_validated_and_all_groups_are_read() {
        let w = codex_windows(
            &json!({"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":10,"windowDurationMins":300,"resetsAt":1800000000},"secondary":{"usedPercent":null}},"other":{"primary":{"usedPercent":101}}}}),
        );
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].remaining_percent, 90.0);
    }
    #[test]
    fn official_quota_falls_back_to_legacy_bucket_and_labels_weekly_limits() {
        let limits = json!({"rateLimitsByLimitId":{},"rateLimits":{"primary":{"usedPercent":0,"windowDurationMins":300,"resetsAt":1800000000},"secondary":{"usedPercent":100,"windowDurationMins":10080,"resetsAt":1800100000}}});
        let windows = codex_windows(&limits);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].label, "5 小时额度");
        assert_eq!(windows[0].remaining_percent, 100.0);
        assert_eq!(windows[1].label, "周额度");
        assert_eq!(windows[1].remaining_percent, 0.0);
        assert_eq!(windows[1].reset_at.as_deref(), Some("1800100000"));
        assert!(codex_windows(&json!({})).is_empty());
    }
    #[test]
    fn cache_identity_includes_credentials_and_agent() {
        let cache = UsageCache::default();
        cache.put(("claude".into(), 1), None);
        assert!(cache.get(&("claude".into(), 1)).is_some());
        assert!(cache.get(&("claude".into(), 2)).is_none());
        assert!(cache.get(&("codex".into(), 1)).is_none());
    }

    #[test]
    fn large_connection_catalog_preserves_saved_order_and_safe_metadata() {
        use crate::{ccswitch::Profile, storage::Store};
        let store =
            Store::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let profiles: Vec<_> = (0..500).map(|index| Profile {
            id: format!("bulk-{index:04}"),
            agent: "claude".into(),
            name: format!("连接 {index:04}"),
            config: json!({"providerId":"custom","model":"fixture-model","apiKey":"quota-fixture-secret","baseUrl":"https://fixture.example.test/v1"}),
        }).collect();
        store.import_profiles(&profiles).unwrap();
        let mut ids = store.connection_orders().unwrap().remove("claude").unwrap();
        ids.reverse();
        store.reorder_connections("claude", &ids).unwrap();
        let connections = quota_connections(&store).unwrap();
        let actual: Vec<_> = connections
            .iter()
            .filter(|c| c.connection_id.starts_with("bulk-"))
            .map(|c| c.connection_id.as_str())
            .collect();
        let expected: Vec<_> = ids
            .iter()
            .filter(|id| id.starts_with("bulk-"))
            .map(String::as_str)
            .collect();
        assert_eq!(actual.len(), 500);
        assert_eq!(actual, expected);
        let safe = serde_json::to_string(&connections).unwrap();
        assert!(!safe.contains("quota-fixture-secret"));
        assert!(!safe.contains("fixture.example.test"));
    }
    #[test]
    fn quota_lists_current_codex_login_once_and_preserves_api_connections() {
        use crate::{ccswitch::Profile, storage::Store};
        let store =
            Store::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        store.import_profiles(&[
            Profile { id:"official-a".into(),agent:"codex".into(),name:"OpenAI Official".into(),config:json!({"config":"model_provider='openai'\nmodel='model-a'"}) },
            Profile { id:"official-b".into(),agent:"codex".into(),name:"Other official alias".into(),config:json!({"config":"model_provider='openai'\nmodel='model-b'"}) },
            Profile { id:"api".into(),agent:"codex".into(),name:"My API".into(),config:json!({"model":"api-model","apiKey":"must-not-leak","providerId":"custom","baseUrl":"https://custom.example.test/v1"}) },
        ]).unwrap();
        let connections = quota_connections(&store).unwrap();
        let codex: Vec<_> = connections.iter().filter(|c| c.agent == "codex").collect();
        assert_eq!(codex.len(), 2);
        assert!(codex.iter().any(|c| c.connection_id == "api"));
        let official = codex
            .iter()
            .find(|c| c.connection_id == crate::session_config::OFFICIAL)
            .unwrap();
        assert_eq!(official.connection_name, "ChatGPT 官方账号");
        assert!(!serde_json::to_string(&connections)
            .unwrap()
            .contains("must-not-leak"));
    }
}
