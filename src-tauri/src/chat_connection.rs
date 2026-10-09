//! Chat must use a configured connection, never an invisible native CLI fallback.
use crate::{
    session_config::{LOCAL, OFFICIAL},
    storage::Store,
    AppState,
};
use serde_json::Value;
use std::future::Future;
use tauri::{AppHandle, Manager};

pub const PROVIDER_REQUIRED: &str = "请先添加模型供应商，再发送消息。";

enum ConnectionKind {
    Configured,
    Official,
    LegacyOfficial,
}

fn connection_kind(
    store: &Store,
    agent: &str,
    session_id: Option<&str>,
    connection_id: Option<&str>,
) -> Result<ConnectionKind, String> {
    if !matches!(agent, "claude" | "codex" | "opencode" | "pi") {
        return Err("该 Agent 尚未接入执行".into());
    }
    // Existing conversations are pinned. A different draft/default connection
    // must not authorize a conversation whose own supplier was removed.
    let id = if let Some(session_id) = session_id {
        let session = store.session(session_id)?;
        if session.agent != agent {
            return Err("会话与 Agent 不匹配".into());
        }
        session.connection_id.ok_or(PROVIDER_REQUIRED)?
    } else if let Some(id) = connection_id {
        id.into()
    } else {
        store.default_connection(agent)?
    };
    // Old Codex conversations kept @local when the official-account alias was
    // unified. Only those pinned conversations may verify that legacy alias.
    if id == LOCAL && agent == "codex" && session_id.is_some() {
        return Ok(ConnectionKind::LegacyOfficial);
    }
    if id == LOCAL || id.is_empty() {
        return Err(PROVIDER_REQUIRED.into());
    }
    if id == OFFICIAL {
        return if matches!(agent, "claude" | "codex") {
            Ok(ConnectionKind::Official)
        } else {
            Err(PROVIDER_REQUIRED.into())
        };
    }
    let profile = store.profile(agent, &id)?.ok_or(PROVIDER_REQUIRED)?;
    if profile.is_official() {
        Ok(ConnectionKind::Official)
    } else {
        Ok(ConnectionKind::Configured)
    }
}

fn verify_official(
    kind: &ConnectionKind,
    status: &Value,
    native_config: impl FnOnce() -> Result<Value, String>,
) -> Result<(), String> {
    if status["loggedIn"] != true {
        return Err(PROVIDER_REQUIRED.into());
    }
    if matches!(kind, ConnectionKind::LegacyOfficial) {
        let config = native_config()?;
        if crate::providers::model_source(&config, "codex", None)["providerId"] != "openai"
            || crate::providers::key(&config).is_some()
        {
            return Err(PROVIDER_REQUIRED.into());
        }
    }
    Ok(())
}

async fn check_with<F: Future<Output = Result<Value, String>>>(
    store: &Store,
    agent: &str,
    session_id: Option<&str>,
    connection_id: Option<&str>,
    account: impl FnOnce() -> F,
) -> Result<(), String> {
    match connection_kind(store, agent, session_id, connection_id)? {
        ConnectionKind::Configured => Ok(()),
        kind @ (ConnectionKind::Official | ConnectionKind::LegacyOfficial) => {
            let status = account()
                .await
                .map_err(|e| format!("无法确认官方账号登录状态：{e}"))?;
            verify_official(&kind, &status, || {
                crate::process::read_codex_config()
                    .map(|config| serde_json::json!({"config":config}))
            })
        }
    }
}

pub(crate) async fn check(
    app: &AppHandle,
    agent: &str,
    session_id: Option<&str>,
    connection_id: Option<&str>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let route = state.store.route_with_connection(agent, session_id, connection_id);
    let account_id = route.as_ref().ok().and_then(|r| r.config["accountId"].as_str());
    check_with(&state.store, agent, session_id, connection_id, || async {
        // Accounts caches read-only login checks; API connections need no Agent
        // process or network request for this preflight.
        app.state::<crate::accounts::Accounts>()
            .get_for(agent, account_id, false, app)
            .await
    })
    .await
}

// Model discovery and allowance queries must use the same eligibility as send.
// Missing suppliers are an empty state; unrelated failures remain visible.
pub(crate) async fn available_route(
    app: &AppHandle,
    agent: &str,
    session_id: Option<&str>,
    connection_id: Option<&str>,
) -> Result<Option<crate::session_config::Route>, String> {
    match check(app, agent, session_id, connection_id).await {
        Ok(()) => {
            let state = app.state::<AppState>();
            let route = state
                .store
                .route_with_connection(agent, session_id, connection_id)?;
            Ok(Some(route))
        }
        Err(error) if error == PROVIDER_REQUIRED => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn model_source(route: Option<&crate::session_config::Route>, agent: &str) -> Value {
    if let Some(route) = route {
        let mut source = route.source(agent);
        source["available"] = true.into();
        source
    } else {
        serde_json::json!({"providerId":"unknown","providerName":"","mark":"","available":false})
    }
}

#[tauri::command]
pub async fn check_chat_connection(
    agent: String,
    session_id: Option<String>,
    connection_id: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    check(
        &app,
        &agent,
        session_id.as_deref(),
        connection_id.as_deref(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccswitch::Profile;
    use serde_json::json;

    fn store() -> Store {
        Store::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap()
    }
    fn profile(agent: &str) -> Profile {
        Profile {
            id: format!("api:{agent}"),
            agent: agent.into(),
            name: "Configured API".into(),
            config: json!({"apiKey":"test-key","model":"test-model"}),
        }
    }

    #[test]
    fn missing_model_source_has_no_supplier_metadata_and_configured_sources_are_available() {
        for agent in ["claude", "codex", "opencode", "pi"] {
            let empty = model_source(None, agent);
            assert_eq!(empty["available"], false);
            assert_eq!(empty["providerName"], "");
            assert_eq!(empty["mark"], "");
            assert!(empty.get("connectionName").is_none());
            let p = profile(agent);
            let route = crate::session_config::Route {
                id: p.id.clone(),
                config: p.config.clone(),
                profile: Some(p),
            };
            let available = model_source(Some(&route), agent);
            assert_eq!(available["available"], true);
            assert_eq!(available["connectionName"], "Configured API");
            assert!(!available.to_string().contains("test-key"));
        }
    }

    #[test]
    fn legacy_codex_login_is_preserved_but_cannot_authorize_hidden_api_connections() {
        let store = store();
        let project = store
            .add_project(std::path::Path::new("D:/legacy-connection-test"))
            .unwrap();
        let session = store
            .create_agent_session(&project.id, None, "codex")
            .unwrap();
        store
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET connection_id=?2 WHERE id=?1",
                rusqlite::params![session.id, LOCAL],
            )
            .unwrap();
        let kind = connection_kind(&store, "codex", Some(&session.id), None).unwrap();
        assert!(matches!(kind, ConnectionKind::LegacyOfficial));
        assert_eq!(
            verify_official(&kind, &json!({"loggedIn":false}), || panic!(
                "must verify login before reading native config"
            ))
            .unwrap_err(),
            PROVIDER_REQUIRED
        );
        for (config, allowed) in [
            (json!({"config":"model_provider = \"openai\""}), true),
            (
                json!({"apiKey":"test-key","config":"model_provider = \"openai\""}),
                false,
            ),
            (
                json!({"config":"model_provider = \"kimi\"\n[model_providers.kimi]\nbase_url = \"https://api.kimi.com/v1\""}),
                false,
            ),
        ] {
            assert_eq!(
                verify_official(&kind, &json!({"loggedIn":true}), || Ok(config)).is_ok(),
                allowed
            );
        }
        assert_eq!(
            store.session(&session.id).unwrap().connection_id.as_deref(),
            Some(LOCAL)
        );
    }

    #[tokio::test]
    async fn missing_or_hidden_connections_never_read_native_configuration_or_login() {
        let store = store();
        for agent in ["claude", "codex", "opencode", "pi"] {
            for id in [LOCAL, "deleted", ""] {
                let result = check_with(&store, agent, None, Some(id), || async {
                    panic!("missing supplier must not start a native login check")
                })
                .await;
                assert_eq!(result.unwrap_err(), PROVIDER_REQUIRED);
            }
        }
        assert!(store.sessions().unwrap().is_empty());
    }

    #[tokio::test]
    async fn configured_connections_work_for_all_four_agents_without_login_checks() {
        let store = store();
        for agent in ["claude", "codex", "opencode", "pi"] {
            let profile = profile(agent);
            store
                .import_profiles(std::slice::from_ref(&profile))
                .unwrap();
            check_with(&store, agent, None, Some(&profile.id), || async {
                panic!("API connection must not query official login")
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn official_connections_require_confirmed_login_not_just_a_selected_flag() {
        let store = store();
        for agent in ["claude", "codex"] {
            let profile = Profile {
                id: format!("official:{agent}"),
                agent: agent.into(),
                name: "Official".into(),
                config: if agent == "codex" {
                    json!({"config":"model_provider = \"openai\""})
                } else {
                    json!({"env":{}})
                },
            };
            store
                .import_profiles(std::slice::from_ref(&profile))
                .unwrap();
            for id in [OFFICIAL, profile.id.as_str()] {
                for logged_in in [false, true] {
                    let result = check_with(&store, agent, None, Some(id), || async {
                        Ok(json!({"loggedIn":logged_in}))
                    })
                    .await;
                    assert_eq!(result.is_ok(), logged_in);
                    if !logged_in {
                        assert_eq!(result.unwrap_err(), PROVIDER_REQUIRED);
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn a_different_default_cannot_authorize_an_unconfigured_existing_chat() {
        let store = store();
        let project = store
            .add_project(std::path::Path::new("D:/connection-test"))
            .unwrap();
        let session = store
            .create_agent_session(&project.id, None, "claude")
            .unwrap();
        let profile = profile("claude");
        store
            .import_profiles(std::slice::from_ref(&profile))
            .unwrap();
        let result = check_with(
            &store,
            "claude",
            Some(&session.id),
            Some(&profile.id),
            || async { panic!("pinned missing connection must not query login") },
        )
        .await;
        assert_eq!(result.unwrap_err(), PROVIDER_REQUIRED);
        assert_eq!(store.session(&session.id).unwrap().status, "idle");
        assert!(store.transcript(&session.id).unwrap().is_empty());
        assert!(store.followups(Some(&session.id)).unwrap().is_empty());
    }

    #[tokio::test]
    async fn deleted_connection_and_agent_mismatch_do_not_fall_back_to_another_supplier() {
        let store = store();
        let project = store
            .add_project(std::path::Path::new("D:/connection-test"))
            .unwrap();
        let profile = profile("claude");
        store
            .import_profiles(std::slice::from_ref(&profile))
            .unwrap();
        let session = store
            .create_configured_session(&project.id, None, "claude", Some(&profile.id))
            .unwrap();
        // Simulate a stale imported/restored binding. Normal deletion already
        // protects bound profiles, but chat authorization must still fail closed.
        store
            .0
            .lock()
            .unwrap()
            .execute("DELETE FROM agent_profiles WHERE id=?1", [&profile.id])
            .unwrap();
        let result = check_with(&store, "claude", Some(&session.id), None, || async {
            Ok(Value::Null)
        })
        .await;
        assert_eq!(result.unwrap_err(), PROVIDER_REQUIRED);
        let result = check_with(
            &store,
            "codex",
            Some(&session.id),
            Some(OFFICIAL),
            || async { Ok(json!({"loggedIn":true})) },
        )
        .await;
        assert_eq!(result.unwrap_err(), "会话与 Agent 不匹配");
    }
}
