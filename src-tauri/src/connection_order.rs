//! One persisted order per agent. The first route is the default for new chats.
use crate::{
    session_config::{LOCAL, OFFICIAL},
    storage::Store,
    AppState,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeMap, HashSet};
use tauri::{AppHandle, Emitter, Manager};

type Result<T> = std::result::Result<T, String>;

fn official_aliases(conn: &Connection, agent: &str) -> Result<HashSet<String>> {
    if agent != "codex" {
        return Ok(HashSet::new());
    }
    let mut query = conn
        .prepare("SELECT id,name,config FROM agent_profiles WHERE agent=?1")
        .map_err(|e| e.to_string())?;
    let rows = query
        .query_map([agent], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut aliases = HashSet::from([LOCAL.to_owned()]);
    for row in rows {
        let (id, name, raw) = row.map_err(|e| e.to_string())?;
        let profile = crate::ccswitch::Profile {
            id: id.clone(),
            agent: agent.into(),
            name,
            config: serde_json::from_str(&crate::credentials::open(&raw)?)
                .map_err(|_| "本地配置 JSON 无效")?,
        };
        if profile.is_official() {
            aliases.insert(id);
        }
    }
    Ok(aliases)
}

fn setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
        r.get(0)
    })
    .optional()
    .map_err(|e| e.to_string())
}

pub(crate) fn order(conn: &Connection, agent: &str) -> Result<Vec<String>> {
    let aliases = official_aliases(conn, agent)?;
    let mut query = conn
        .prepare("SELECT id FROM agent_profiles WHERE agent=?1 ORDER BY name,id")
        .map_err(|e| e.to_string())?;
    let ids = query
        .query_map([agent], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let previous = setting(conn, &format!("connection_order_{agent}"))?
        .map(|s| serde_json::from_str::<Vec<String>>(&s).map_err(|_| "连接排序数据无效".to_owned()))
        .transpose()?;
    let official = setting(conn, &format!("official_{agent}"))?.as_deref() == Some("1");
    let default = setting(conn, &format!("profile_{agent}"))?
        .filter(|id| ids.contains(id))
        .unwrap_or_else(|| {
            if official {
                OFFICIAL.into()
            } else {
                LOCAL.into()
            }
        });
    let mut available: Vec<_> = ids.into_iter().filter(|id| !aliases.contains(id)).collect();
    if matches!(agent, "codex" | "claude") {
        available.push(OFFICIAL.into());
    }
    // Keep the pre-existing CLI route available, without adding it to accounts
    // that have always used explicit connections.
    if matches!(agent, "opencode" | "pi")
        || agent != "codex"
            && (previous.is_none() && default == LOCAL
                || previous
                    .as_ref()
                    .is_some_and(|v| v.iter().any(|id| id == LOCAL)))
    {
        available.push(LOCAL.into());
    }
    let available_ids: HashSet<_> = available.iter().cloned().collect();
    let mut seen = HashSet::new();
    let mut result: Vec<_> = previous
        .unwrap_or_else(|| vec![default])
        .into_iter()
        .map(|id| {
            if aliases.contains(&id) {
                OFFICIAL.into()
            } else {
                id
            }
        })
        .filter(|id| available_ids.contains(id) && seen.insert(id.clone()))
        .collect();
    for id in available {
        if seen.insert(id.clone()) {
            result.push(id);
        }
    }
    Ok(result)
}

fn write_order(conn: &Connection, agent: &str, ids: &[String]) -> Result<()> {
    let first = ids.first().ok_or("至少需要保留一个连接")?;
    conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![format!("connection_order_{agent}"), serde_json::to_string(ids).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM settings WHERE key IN (?1,?2)",
        params![format!("profile_{agent}"), format!("official_{agent}")],
    )
    .map_err(|e| e.to_string())?;
    if first != LOCAL {
        let (key, value) = if first == OFFICIAL {
            (format!("official_{agent}"), "1")
        } else {
            (format!("profile_{agent}"), first.as_str())
        };
        conn.execute(
            "INSERT INTO settings(key,value) VALUES(?1,?2)",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn reconcile(conn: &Connection) -> Result<()> {
    for agent in ["claude", "codex", "opencode", "pi"] {
        write_order(conn, agent, &order(conn, agent)?)?;
    }
    Ok(())
}

impl Store {
    pub fn connection_orders(&self) -> Result<BTreeMap<String, Vec<String>>> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        ["claude", "codex", "opencode", "pi"]
            .into_iter()
            .map(|agent| Ok((agent.into(), order(&conn, agent)?)))
            .collect()
    }

    pub fn initialize_connection_orders(&self) -> Result<()> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        reconcile(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn reorder_connections(&self, agent: &str, ids: &[String]) -> Result<()> {
        if !matches!(agent, "claude" | "codex" | "opencode" | "pi") {
            return Err("Agent 无效".into());
        }
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let available = order(&tx, agent)?;
        let set: HashSet<_> = ids.iter().collect();
        if ids.len() != available.len()
            || set.len() != ids.len()
            || available.iter().any(|id| !set.contains(id))
        {
            return Err("连接列表已经变化，请刷新后重新排序".into());
        }
        write_order(&tx, agent, ids)?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub(crate) fn make_connection_default(&self, agent: &str, id: &str) -> Result<()> {
        if !matches!(agent, "claude" | "codex" | "opencode" | "pi") {
            return Err("Agent 无效".into());
        }
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut ids = order(&tx, agent)?;
        let aliases = official_aliases(&tx, agent)?;
        let id = if aliases.contains(id) { OFFICIAL } else { id };
        if !ids.iter().any(|v| v == id) && id != LOCAL {
            return Err("连接不存在或不属于此 Agent".into());
        }
        ids.retain(|v| v != id);
        ids.insert(0, id.into());
        write_order(&tx, agent, &ids)?;
        tx.commit().map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn reorder_provider_connections(agent: String, ids: Vec<String>, app: AppHandle) -> Result<()> {
    // Existing sessions are pinned to their routes; sorting needs no process restart.
    app.state::<AppState>()
        .store
        .reorder_connections(&agent, &ids)?;
    let _ = app.emit("workspace-updated", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccswitch::Profile;
    use serde_json::json;
    use std::path::Path;

    fn profile(id: &str, agent: &str) -> Profile {
        Profile {
            id: id.into(),
            agent: agent.into(),
            name: id.into(),
            config: json!({"model":"test-model","apiKey":"fixture"}),
        }
    }

    fn official(id: &str) -> Profile {
        Profile {
            id: id.into(),
            agent: "codex".into(),
            name: "OpenAI Official".into(),
            config: json!({"config":"model_provider='openai'\nmodel='configured-model'","model":"configured-model"}),
        }
    }

    #[test]
    fn codex_migration_unifies_login_aliases_without_rebinding_history() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store
            .import_profiles(&[official("imported"), official("other-import")])
            .unwrap();
        let project = store
            .add_project(Path::new("D:/official-alias-test"))
            .unwrap();
        let local = store
            .create_agent_session(&project.id, Some("old-model".into()), "codex")
            .unwrap();
        let imported = store
            .create_agent_session(&project.id, Some("configured-model".into()), "codex")
            .unwrap();
        {
            let conn = store.0.lock().unwrap();
            conn.execute(
                "UPDATE sessions SET connection_id=?2 WHERE id=?1",
                params![local.id, LOCAL],
            )
            .unwrap();
            conn.execute(
                "UPDATE sessions SET connection_id='imported' WHERE id=?1",
                [&imported.id],
            )
            .unwrap();
            conn.execute("INSERT INTO settings(key,value) VALUES('connection_order_codex',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(&[LOCAL,"imported",OFFICIAL,"other-import"]).unwrap()]).unwrap();
            conn.execute(
                "DELETE FROM settings WHERE key IN ('profile_codex','official_codex')",
                [],
            )
            .unwrap();
        }
        let store = Store::from_connection(store.0.into_inner().unwrap()).unwrap();
        assert_eq!(store.connection_orders().unwrap()["codex"], [OFFICIAL]);
        assert_eq!(store.default_connection("codex").unwrap(), OFFICIAL);
        assert_eq!(
            store.session(&local.id).unwrap().connection_id.as_deref(),
            Some(LOCAL)
        );
        assert_eq!(
            store.session(&local.id).unwrap().model.as_deref(),
            Some("old-model")
        );
        assert_eq!(
            store
                .session(&imported.id)
                .unwrap()
                .connection_id
                .as_deref(),
            Some("imported")
        );
        assert_eq!(
            store
                .route_for("codex", "imported")
                .unwrap()
                .profile
                .unwrap()
                .id,
            "imported"
        );
        assert_eq!(store.profiles().unwrap().len(), 2);
    }

    #[test]
    fn official_profile_activation_uses_current_login_and_api_routes_keep_order() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store
            .import_profiles(&[official("imported"), profile("api", "codex")])
            .unwrap();
        store
            .reorder_connections("codex", &["api".into(), OFFICIAL.into()])
            .unwrap();
        assert_eq!(store.default_connection("codex").unwrap(), "api");
        store.select_profile("codex", Some("imported")).unwrap();
        assert_eq!(
            store.connection_orders().unwrap()["codex"],
            [OFFICIAL, "api"]
        );
        assert_eq!(store.default_connection("codex").unwrap(), OFFICIAL);
        store
            .reorder_connections("codex", &["api".into(), OFFICIAL.into()])
            .unwrap();
        let store = Store::from_connection(store.0.into_inner().unwrap()).unwrap();
        assert_eq!(
            store.connection_orders().unwrap()["codex"],
            ["api", OFFICIAL]
        );
        assert_eq!(store.default_connection("codex").unwrap(), "api");
    }

    #[test]
    fn migration_keeps_old_default_first_and_restart_keeps_order() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);CREATE TABLE agent_profiles(id TEXT PRIMARY KEY,agent TEXT NOT NULL,name TEXT NOT NULL,config TEXT NOT NULL)").unwrap();
        for id in ["a", "z"] {
            conn.execute(
                "INSERT INTO agent_profiles VALUES(?1,'claude',?1,?2)",
                params![
                    id,
                    crate::credentials::seal(&json!({"model":"test"}).to_string()).unwrap()
                ],
            )
            .unwrap();
        }
        conn.execute("INSERT INTO settings VALUES('profile_claude','z')", [])
            .unwrap();
        let store = Store::from_connection(conn).unwrap();
        assert_eq!(
            store.connection_orders().unwrap()["claude"],
            ["z", "a", OFFICIAL]
        );
        store
            .reorder_connections("claude", &["a".into(), OFFICIAL.into(), "z".into()])
            .unwrap();
        let store = Store::from_connection(store.0.into_inner().unwrap()).unwrap();
        assert_eq!(store.default_connection("claude").unwrap(), "a");
        assert_eq!(
            store.connection_orders().unwrap()["claude"],
            ["a", OFFICIAL, "z"]
        );
        assert_eq!(
            store
                .profiles()
                .unwrap()
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "z"]
        );
    }

    #[test]
    fn sorting_changes_only_new_chat_defaults_and_is_atomic() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store
            .import_profiles(&[
                profile("a", "claude"),
                profile("b", "claude"),
                profile("c", "codex"),
            ])
            .unwrap();
        store.select_profile("claude", Some("a")).unwrap();
        store.select_profile("codex", Some("c")).unwrap();
        let project = store.add_project(Path::new("D:/ordering-test")).unwrap();
        let chat = store
            .create_agent_session(&project.id, Some("test-model".into()), "claude")
            .unwrap();
        let native = "native-order-test";
        store.bind_native(&chat.id, native).unwrap();
        store.claim(&chat.id).unwrap();
        let mut ids = store.connection_orders().unwrap()["claude"].clone();
        ids.retain(|id| id != "b");
        ids.insert(0, "b".into());
        store.reorder_connections("claude", &ids).unwrap();
        let pinned = store.session(&chat.id).unwrap();
        assert_eq!(pinned.connection_id.as_deref(), Some("a"));
        assert_eq!(pinned.native_id.as_deref(), Some(native));
        assert_eq!(pinned.status, "starting");
        assert_eq!(store.default_connection("codex").unwrap(), "c");
        assert_eq!(
            store
                .create_agent_session(&project.id, None, "claude")
                .unwrap()
                .connection_id
                .as_deref(),
            Some("b")
        );
        for invalid in [
            vec!["a".into(), "a".into()],
            vec!["c".into()],
            vec!["b".into()],
        ] {
            assert!(store.reorder_connections("claude", &invalid).is_err());
            assert_eq!(store.connection_orders().unwrap()["claude"], ids);
            assert_eq!(store.default_connection("claude").unwrap(), "b");
        }
    }

    #[test]
    fn imports_append_renames_keep_position_deletion_promotes_next_and_official_is_sortable() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store
            .import_profiles(&[profile("z", "claude"), profile("y", "claude")])
            .unwrap();
        store.select_profile("claude", Some("z")).unwrap();
        let before = store.connection_orders().unwrap()["claude"].clone();
        let mut renamed = profile("z", "claude");
        renamed.name = "AAA renamed".into();
        store
            .import_profiles(&[renamed, profile("a", "claude")])
            .unwrap();
        let mut after = before;
        after.push("a".into());
        assert_eq!(store.connection_orders().unwrap()["claude"], after);
        store.delete_profile("z").unwrap();
        assert_eq!(store.default_connection("claude").unwrap(), after[1]);
        store.use_official("claude").unwrap();
        assert_eq!(store.connection_orders().unwrap()["claude"][0], OFFICIAL);
        assert!(store.official("claude").unwrap());
        store.select_profile("claude", Some("a")).unwrap();
        assert_eq!(store.connection_orders().unwrap()["claude"][0], "a");
        assert!(!store.official("claude").unwrap());
    }
}
