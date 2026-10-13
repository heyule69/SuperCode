//! Live Codex turns can outlast their parent chat turn. These leases belong to
//! one native process and live only in SQLite TEMP storage, never chat history.
use crate::storage::{Result, Store};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::BTreeMap;

fn schema(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TEMP TABLE IF NOT EXISTS connection_usage(owner TEXT NOT NULL,session_id TEXT,connection_id TEXT NOT NULL,thread_id TEXT NOT NULL,turn_id TEXT,active INTEGER NOT NULL,definitive INTEGER NOT NULL,PRIMARY KEY(owner,thread_id));")
        .map_err(|e| e.to_string())
}

pub(crate) fn in_use(conn: &Connection, id: &str) -> Result<bool> {
    schema(conn)?;
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE connection_id=?1 AND status IN ('starting','running','waiting')) OR EXISTS(SELECT 1 FROM temp.connection_usage WHERE connection_id=?1 AND active=1)",
        [id], |row| row.get(0),
    ).map_err(|e| e.to_string())
}

pub(crate) fn running(conn: &Connection) -> Result<usize> {
    schema(conn)?;
    conn.query_row("SELECT (SELECT count(*) FROM sessions WHERE status IN ('starting','running','waiting')) + (SELECT count(*) FROM temp.connection_usage u WHERE active=1 AND NOT EXISTS(SELECT 1 FROM sessions s WHERE s.native_id=u.thread_id AND s.status IN ('starting','running','waiting')))",
        [], |row| row.get(0)).map_err(|e| e.to_string())
}

pub(crate) fn session_tasks(conn: &Connection, id: &str) -> Result<bool> {
    schema(conn)?;
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM temp.connection_usage WHERE session_id=?1 AND active=1)",
        [id],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

pub(crate) fn ensure_session_idle(conn: &Connection, id: &str) -> Result<()> {
    let primary: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND status IN ('starting','running','waiting'))", [id], |row| row.get(0)).map_err(|e| e.to_string())?;
    if primary || session_tasks(conn, id)? {
        return Err("此聊天仍有任务或后台子任务运行，请完成或停止后再操作".into());
    }
    Ok(())
}

fn active_status(status: &str) -> Option<bool> {
    match status {
        "active" | "running" | "inProgress" | "preparing" | "pendingInit" | "pending"
        | "waiting" | "waitingForInput" => Some(true),
        "completed" | "success" | "done" | "failed" | "errored" | "error" | "notFound"
        | "interrupted" | "shutdown" | "stopped" | "cancelled" | "idle" | "notLoaded"
        | "systemError" => Some(false),
        _ => None,
    }
}

fn count(conn: &Connection, owner: &str) -> Result<usize> {
    conn.query_row(
        "SELECT count(*) FROM temp.connection_usage WHERE owner=?1 AND active=1",
        [owner],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

fn write(
    conn: &Connection,
    owner: &str,
    session: Option<&str>,
    connection: &str,
    thread: &str,
    turn: Option<&str>,
    active: bool,
    definitive: bool,
) -> Result<()> {
    // Child lifecycle events are authoritative. A later parent tool snapshot
    // must not revive a child whose native turn already completed.
    conn.execute("INSERT INTO temp.connection_usage(owner,session_id,connection_id,thread_id,turn_id,active,definitive) VALUES(?1,?2,?3,?4,?5,?6,?7 OR NOT ?6) ON CONFLICT(owner,thread_id) DO UPDATE SET turn_id=CASE WHEN ?7 AND excluded.turn_id IS NOT NULL THEN excluded.turn_id ELSE connection_usage.turn_id END,active=CASE WHEN ?7 OR connection_usage.definitive=0 THEN excluded.active ELSE connection_usage.active END,definitive=MAX(connection_usage.definitive,excluded.definitive)",
        params![owner,session,connection,thread,turn,active,definitive]).map_err(|e| e.to_string())?;
    Ok(())
}

impl Store {
    pub fn active_session_ids(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        schema(&conn)?;
        let mut query = conn.prepare("SELECT id FROM sessions WHERE status IN ('starting','running','waiting') UNION SELECT session_id FROM temp.connection_usage WHERE active=1 AND session_id IS NOT NULL ORDER BY 1").map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string());
        rows
    }
    pub fn running_connections(&self) -> Result<BTreeMap<String, Vec<String>>> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        schema(&conn)?;
        let mut query = conn.prepare("SELECT agent,connection_id FROM sessions WHERE connection_id IS NOT NULL AND status IN ('starting','running','waiting') UNION SELECT 'codex',connection_id FROM temp.connection_usage WHERE active=1 ORDER BY 1,2")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut result = BTreeMap::<String, Vec<String>>::new();
        for row in rows {
            let (agent, connection) = row.map_err(|e| e.to_string())?;
            result.entry(agent).or_default().push(connection);
        }
        Ok(result)
    }

    /// Returns changed occupancy and current active turn count for this client.
    pub(crate) fn observe_connection_usage(
        &self,
        owner: &str,
        session: Option<&str>,
        connection: &str,
        message: &Value,
    ) -> Result<Option<(bool, usize)>> {
        let method = message["method"].as_str().unwrap_or("");
        let p = &message["params"];
        let collab = matches!(method, "item/started" | "item/updated" | "item/completed")
            && matches!(
                p["item"]["type"].as_str(),
                Some("collabAgentToolCall" | "collabToolCall")
            );
        if !collab
            && !matches!(
                method,
                "turn/started" | "turn/completed" | "thread/status/changed"
            )
        {
            return Ok(None);
        }
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        schema(&conn)?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let before = count(&tx, owner)?;
        if collab {
            let item = &p["item"];
            let mut children = BTreeMap::<&str, Option<bool>>::new();
            if let Some(ids) = item["receiverThreadIds"].as_array() {
                for id in ids.iter().filter_map(Value::as_str) {
                    children.insert(id, None);
                }
            }
            if let Some(states) = item["agentsStates"].as_object() {
                for (id, state) in states {
                    children.insert(id, state["status"].as_str().and_then(active_status));
                }
            }
            let sender: Option<(bool, Option<String>)> = p["threadId"].as_str().map(|sender| {
                tx.query_row("SELECT active,turn_id FROM temp.connection_usage WHERE owner=?1 AND thread_id=?2",params![owner,sender],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(|e|e.to_string())
            }).transpose()?.flatten();
            // A newer parent turn can be active after an old child's tombstone
            // was evicted. Its old tool snapshots still cannot restart that child.
            if let (Some((_, Some(current_turn))), Some(turn)) = (&sender, p["turnId"].as_str()) {
                if current_turn.as_str() != turn {
                    return Ok(None);
                }
            }
            let sender_active = sender.map(|(active, _)| active);
            for (id, explicit) in children {
                if item["status"] == "failed" && explicit.is_none() {
                    continue;
                }
                let closed = method == "item/completed"
                    && item["tool"] == "closeAgent"
                    && item["status"] != "failed";
                let existing: Option<bool> = tx
                    .query_row(
                        "SELECT active FROM temp.connection_usage WHERE owner=?1 AND thread_id=?2",
                        params![owner, id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                // A completed parent cannot launch a new child via an old tool
                // snapshot. This also keeps evicted terminal children retired.
                if existing.is_none() && sender_active == Some(false) {
                    continue;
                }
                let active = if closed {
                    false
                } else {
                    explicit.unwrap_or(existing.unwrap_or(true))
                };
                write(&tx, owner, session, connection, id, None, active, closed)?;
            }
        } else if let Some(thread) = p["threadId"].as_str() {
            match method {
                "turn/started" => write(
                    &tx,
                    owner,
                    session,
                    connection,
                    thread,
                    p["turn"]["id"].as_str(),
                    true,
                    true,
                )?,
                "turn/completed" => {
                    let turn = p["turn"]["id"].as_str();
                    // A late completion cannot clear a newer turn's lease.
                    let current: Option<Option<String>> = tx.query_row("SELECT turn_id FROM temp.connection_usage WHERE owner=?1 AND thread_id=?2", params![owner,thread], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
                    if current
                        .as_ref()
                        .is_none_or(|id| id.is_none() || id.as_deref() == turn)
                    {
                        write(&tx, owner, session, connection, thread, turn, false, true)?;
                    }
                }
                "thread/status/changed" => {
                    let status = p["status"]["type"].as_str().or(p["status"].as_str());
                    if let Some(active) = status.and_then(active_status) {
                        write(&tx, owner, session, connection, thread, None, active, true)?;
                    }
                }
                _ => {}
            }
        }
        // Retain active work and the current parent's lifecycle. Finished child
        // tombstones need only a bounded recent cache per native process.
        tx.execute("DELETE FROM temp.connection_usage WHERE rowid IN (SELECT u.rowid FROM temp.connection_usage u WHERE owner=?1 AND active=0 AND NOT EXISTS(SELECT 1 FROM sessions s WHERE s.id=u.session_id AND s.native_id=u.thread_id) ORDER BY u.rowid DESC LIMIT -1 OFFSET 128)", [owner]).map_err(|e|e.to_string())?;
        let after = count(&tx, owner)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Some((before != after, after)))
    }

    pub(crate) fn clear_connection_usage(&self, owner: &str) -> Result<bool> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        schema(&conn)?;
        let active = count(&conn, owner)? > 0;
        conn.execute("DELETE FROM temp.connection_usage WHERE owner=?1", [owner])
            .map_err(|e| e.to_string())?;
        Ok(active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccswitch::Profile;
    use serde_json::json;
    use std::path::Path;

    fn fixture() -> (Store, Profile) {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let profile = Profile {
            id: "api".into(),
            agent: "codex".into(),
            name: "API".into(),
            config: json!({"model":"fixture","apiKey":"key"}),
        };
        store.import_profiles(&[profile.clone()]).unwrap();
        (store, profile)
    }
    fn observe(store: &Store, owner: &str, value: Value) -> (bool, usize) {
        store
            .observe_connection_usage(owner, Some("parent-session"), "api", &value)
            .unwrap()
            .unwrap()
    }
    fn turn(method: &str, thread: &str, turn: &str) -> Value {
        json!({"method":method,"params":{"threadId":thread,"turn":{"id":turn,"status":"completed"}}})
    }
    fn collab(states: Value) -> Value {
        json!({"method":"item/completed","params":{"threadId":"parent","item":{"type":"collabAgentToolCall","status":"completed","receiverThreadIds":["child"],"agentsStates":states}}})
    }
    #[test]
    fn background_child_keeps_only_its_connection_locked_after_parent_completes() {
        let (store, mut profile) = fixture();
        let project = store
            .add_project(Path::new("D:/child-connection-guard"))
            .unwrap();
        let chat = store
            .create_configured_session(&project.id, None, "codex", Some("api"))
            .unwrap();
        store.claim(&chat.id).unwrap();
        store.bind_native(&chat.id, "parent").unwrap();
        store
            .set_status(&chat.id, "running", Some("parent-turn"))
            .unwrap();
        observe(
            &store,
            "owner",
            turn("turn/started", "parent", "parent-turn"),
        );
        assert_eq!(
            observe(
                &store,
                "owner",
                collab(json!({"child":{"status":"pendingInit"}}))
            )
            .1,
            2
        );
        assert_eq!(store.running().unwrap(), 2); // Count the active primary only once.
        observe(
            &store,
            "owner",
            turn("turn/completed", "parent", "parent-turn"),
        );
        store
            .complete_turn(&chat.id, "parent-turn", "completed")
            .unwrap();
        assert_eq!(store.session(&chat.id).unwrap().status, "idle");
        assert_eq!(store.running().unwrap(), 1);
        assert_eq!(store.running_connections().unwrap()["codex"], vec!["api"]);
        profile.name = "Edited".into();
        assert!(store.import_profiles(&[profile.clone()]).is_err());
        let other = Profile {
            id: "new".into(),
            name: "New".into(),
            ..profile.clone()
        };
        store.import_profiles(&[other]).unwrap();
        store.select_profile("codex", Some("new")).unwrap();
        assert_eq!(
            store.session(&chat.id).unwrap().connection_id.as_deref(),
            Some("api")
        );
        observe(&store, "owner", turn("turn/started", "child", "child-turn"));
        assert_eq!(
            observe(
                &store,
                "owner",
                turn("turn/completed", "child", "child-turn")
            ),
            (true, 0)
        );
        assert!(store.running_connections().unwrap().is_empty());
        assert_eq!(store.running().unwrap(), 0);
        store.import_profiles(&[profile]).unwrap();
    }
    #[test]
    fn idle_parent_with_live_child_cannot_be_deleted_moved_archived_or_rebound() {
        let (store, _) = fixture();
        let project = store
            .add_project(Path::new("D:/child-parent-guard"))
            .unwrap();
        let target = store
            .add_project(Path::new("D:/child-parent-target"))
            .unwrap();
        let chat = store
            .create_configured_session(&project.id, None, "codex", Some("api"))
            .unwrap();
        store.bind_native(&chat.id, "parent").unwrap();
        store
            .observe_connection_usage(
                "child-owner",
                Some(&chat.id),
                "api",
                &turn("turn/started", "child", "child-turn"),
            )
            .unwrap();
        assert_eq!(store.session(&chat.id).unwrap().status, "idle");
        assert_eq!(store.active_session_ids().unwrap(), vec![chat.id.clone()]);
        assert!(store.delete_session(&chat.id).is_err());
        assert!(store.move_session(&chat.id, &target.id).is_err());
        assert!(store.archive(&chat.id).is_err());
        assert!(store.project_action(&project.id, "archive").is_err());
        assert!(store
            .edit_project(&project.id, "Moved", "D:/new-root")
            .is_err());
        assert!(store.claim_configuration_change(&chat.id).is_err());
        assert_eq!(
            store.session(&chat.id).unwrap().native_id.as_deref(),
            Some("parent")
        );
        store
            .observe_connection_usage(
                "child-owner",
                Some(&chat.id),
                "api",
                &turn("turn/completed", "child", "child-turn"),
            )
            .unwrap();
        assert!(store.active_session_ids().unwrap().is_empty());
        store.move_session(&chat.id, &target.id).unwrap();
        store.claim_configuration_change(&chat.id).unwrap();
    }
    #[test]
    fn late_child_start_cancels_a_pending_handoff_without_rebinding_or_checkpointing() {
        let (store, _) = fixture();
        let project = store
            .add_project(Path::new("D:/late-child-handoff"))
            .unwrap();
        let chat = store
            .create_configured_session(&project.id, Some("old".into()), "codex", Some("api"))
            .unwrap();
        store.bind_native(&chat.id, "parent").unwrap();
        let before = store.session(&chat.id).unwrap();
        store.claim_configuration_change(&chat.id).unwrap();
        store
            .observe_connection_usage(
                "owner",
                Some(&chat.id),
                "api",
                &turn("turn/started", "child", "child-turn"),
            )
            .unwrap();
        let route = store.route_for("codex", "api").unwrap();
        assert!(store
            .commit_model_switch(
                &before,
                &route,
                "new",
                Some(("summary", 0)),
                &json!({"contextMode":"summary"})
            )
            .is_err());
        let after = store.session(&chat.id).unwrap();
        assert_eq!(after.native_id, before.native_id);
        assert_eq!(after.connection_id, before.connection_id);
        assert_eq!(after.model, before.model);
        assert!(store.compressed_context(&chat.id).unwrap().is_none());
        assert!(store.messages(&chat.id, None).unwrap().is_empty());
    }
    #[test]
    fn definitive_child_completion_cannot_be_revived_by_stale_collab_or_old_turn_completion() {
        let (store, _) = fixture();
        observe(&store, "owner", turn("turn/started", "child", "one"));
        observe(&store, "owner", turn("turn/completed", "child", "one"));
        assert_eq!(
            observe(
                &store,
                "owner",
                collab(json!({"child":{"status":"running"}}))
            )
            .1,
            0
        );
        observe(&store, "owner", turn("turn/started", "child", "two"));
        assert_eq!(
            observe(&store, "owner", turn("turn/completed", "child", "one")).1,
            1
        );
        assert_eq!(
            observe(&store, "owner", turn("turn/completed", "child", "two")).1,
            0
        );
    }
    #[test]
    fn stale_parent_completion_cannot_clear_a_newer_native_child_turn() {
        let (store, _) = fixture();
        observe(&store, "owner", turn("turn/started", "child", "one"));
        observe(&store, "owner", turn("turn/completed", "child", "one"));
        observe(&store, "owner", turn("turn/started", "child", "two"));
        assert_eq!(
            observe(
                &store,
                "owner",
                collab(json!({"child":{"status":"completed"}}))
            )
            .1,
            1
        );
        assert_eq!(store.running().unwrap(), 1);
        assert_eq!(
            observe(&store, "owner", turn("turn/completed", "child", "two")).1,
            0
        );
    }
    #[test]
    fn terminal_snapshots_clear_pending_children_and_recent_tombstones_are_bounded() {
        let (store, _) = fixture();
        observe(
            &store,
            "owner",
            collab(json!({"child":{"status":"pendingInit"}})),
        );
        assert_eq!(
            observe(
                &store,
                "owner",
                collab(json!({"child":{"status":"completed"}}))
            )
            .1,
            0
        );
        assert_eq!(
            observe(
                &store,
                "owner",
                collab(json!({"child":{"status":"running"}}))
            )
            .1,
            0
        );
        for i in 0..200 {
            observe(
                &store,
                "owner",
                turn("turn/completed", &format!("finished-{i}"), "turn"),
            );
        }
        let retained: usize = store
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM temp.connection_usage WHERE owner='owner' AND active=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, 128);
    }
    #[test]
    fn completed_parent_cannot_revive_an_evicted_child_with_an_old_tool_snapshot() {
        let (store, _) = fixture();
        let project = store
            .add_project(Path::new("D:/bounded-live-usage"))
            .unwrap();
        let chat = store
            .create_configured_session(&project.id, None, "codex", Some("api"))
            .unwrap();
        store.bind_native(&chat.id, "parent").unwrap();
        let record = |value: Value| {
            store
                .observe_connection_usage("owner", Some(&chat.id), "api", &value)
                .unwrap()
                .unwrap()
        };
        record(turn("turn/started", "parent", "main"));
        record(collab(json!({"child":{"status":"pendingInit"}})));
        record(turn("turn/completed", "child", "child-turn"));
        record(turn("turn/completed", "parent", "main"));
        for i in 0..200 {
            record(turn("turn/completed", &format!("finished-{i}"), "turn"));
        }
        assert_eq!(record(collab(json!({"child":{"status":"running"}}))).1, 0);
        assert!(store.running_connections().unwrap().is_empty());
    }
    #[test]
    fn old_parent_turn_cannot_revive_an_evicted_child_during_a_new_parent_turn() {
        let (store, _) = fixture();
        let project = store
            .add_project(Path::new("D:/new-parent-live-usage"))
            .unwrap();
        let chat = store
            .create_configured_session(&project.id, None, "codex", Some("api"))
            .unwrap();
        store.bind_native(&chat.id, "parent").unwrap();
        let record = |value: Value| {
            store
                .observe_connection_usage("owner", Some(&chat.id), "api", &value)
                .unwrap()
        };
        record(turn("turn/started", "parent", "main-1"));
        record(collab(json!({"child":{"status":"pendingInit"}})));
        record(turn("turn/completed", "child", "child-turn"));
        for i in 0..129 {
            record(turn("turn/completed", &format!("finished-{i}"), "turn"));
        }
        record(turn("turn/completed", "parent", "main-1"));
        record(turn("turn/started", "parent", "main-2"));
        let mut stale = collab(json!({"child":{"status":"running"}}));
        stale["params"]["turnId"] = "main-1".into();
        assert!(record(stale).is_none());
        assert_eq!(store.running().unwrap(), 1);
        record(turn("turn/completed", "parent", "main-2"));
        assert!(store.running_connections().unwrap().is_empty());
        assert_eq!(
            record(turn("turn/started", "child", "real-new-turn"))
                .unwrap()
                .1,
            1
        );
        record(turn("turn/completed", "child", "real-new-turn"));
        assert_eq!(store.running().unwrap(), 0);
    }
    #[test]
    fn child_thread_idle_and_successful_close_release_pending_leases() {
        let (store, _) = fixture();
        observe(
            &store,
            "owner",
            collab(json!({"child":{"status":"waitingForInput"}})),
        );
        assert_eq!(
            observe(
                &store,
                "owner",
                json!({"method":"thread/status/changed","params":{"threadId":"child","status":{"type":"idle"}}})
            ),
            (true, 0)
        );
        assert_eq!(
            observe(
                &store,
                "owner",
                collab(json!({"child":{"status":"running"}}))
            )
            .1,
            0
        );
        observe(
            &store,
            "owner",
            json!({"method":"thread/status/changed","params":{"threadId":"child","status":{"type":"active","activeFlags":["waitingOnApproval"]}}}),
        );
        let mut close = collab(Value::Null);
        close["params"]["item"]["tool"] = "closeAgent".into();
        assert_eq!(observe(&store, "owner", close), (true, 0));
    }
    #[test]
    fn process_cleanup_cannot_clear_a_replacement_clients_lease() {
        let (store, _) = fixture();
        observe(&store, "old", turn("turn/started", "child", "one"));
        observe(&store, "new", turn("turn/started", "child", "two"));
        assert!(store.clear_connection_usage("old").unwrap());
        assert_eq!(store.running_connections().unwrap()["codex"], vec!["api"]);
        assert!(store.clear_connection_usage("new").unwrap());
        assert!(store.running_connections().unwrap().is_empty());
        assert!(!store.clear_connection_usage("old").unwrap());
    }
    #[test]
    fn live_leases_never_survive_reopening_storage_or_modify_persisted_history() {
        let path = std::env::temp_dir().join(format!("sc-live-usage-{}.db", uuid::Uuid::new_v4()));
        {
            let store = Store::open(&path).unwrap();
            observe(
                &store,
                "owner",
                collab(json!({"child":{"status":"running"}})),
            );
            assert_eq!(store.running_connections().unwrap()["codex"], vec!["api"]);
            let conn = store.0.lock().unwrap();
            let durable: usize = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name='connection_usage'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(durable, 0);
        }
        let reopened = Store::open(&path).unwrap();
        assert!(reopened.running_connections().unwrap().is_empty());
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }
}
