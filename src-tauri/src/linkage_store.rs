//! Persistent ownership, independent connections and shared project workspaces.
use crate::{
    linkage::Configuration,
    storage::{Session, Store},
};
use rusqlite::{params, Connection, OptionalExtension};

pub fn schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS linkage_configs(controller_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,configuration TEXT NOT NULL,worker_id TEXT);
        CREATE TABLE IF NOT EXISTS linkage_workers(worker_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,controller_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE);
        CREATE INDEX IF NOT EXISTS linkage_owner ON linkage_workers(controller_id);
        CREATE TABLE IF NOT EXISTS linkage_tasks(controller_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,task TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS linkage_updates(seq INTEGER PRIMARY KEY AUTOINCREMENT,controller_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,worker_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,input_seq INTEGER NOT NULL,round INTEGER NOT NULL,kind TEXT NOT NULL,event_key TEXT NOT NULL,message_id TEXT,delivered INTEGER NOT NULL DEFAULT 0,UNIQUE(controller_id,input_seq,round,kind,event_key));
        CREATE TABLE IF NOT EXISTS linkage_communications(controller_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,input_seq INTEGER NOT NULL,round INTEGER NOT NULL,cursor INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS linkage_pending_updates ON linkage_updates(controller_id,input_seq,round,delivered,seq);")
}
pub fn active_children(conn: &Connection, id: &str) -> Result<bool, String> {
    crate::connection_usage::schema(conn)?;
    conn.query_row("SELECT EXISTS(SELECT 1 FROM linkage_workers w JOIN sessions s ON s.id=w.worker_id WHERE w.controller_id=?1 AND (s.status IN ('starting','running','waiting') OR EXISTS(SELECT 1 FROM temp.connection_usage u WHERE u.session_id=s.id AND u.active=1)))", [id], |r| r.get(0)).map_err(|e|e.to_string())
}
impl Store {
    pub fn linkage_controller(&self, id: &str) -> Result<bool, String> {
        Ok(self.linkage_parent(id)?.is_none()
            && self.linkage_configuration(id)?.is_some_and(|(config, _)| config.enabled()))
    }
    pub fn linkage_active(&self, id: &str) -> Result<bool, String> {
        active_children(&*self.0.lock().map_err(|e| e.to_string())?, id)
    }
    pub fn session_has_native_tasks(&self, id: &str) -> Result<bool, String> {
        crate::connection_usage::session_tasks(&*self.0.lock().map_err(|e| e.to_string())?, id)
    }
    pub fn linkage_parent(&self, worker: &str) -> Result<Option<String>, String> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .query_row(
                "SELECT controller_id FROM linkage_workers WHERE worker_id=?1",
                [worker],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn linkage_configuration(
        &self,
        id: &str,
    ) -> Result<Option<(Configuration, Option<String>)>, String> {
        let row: Option<(String, Option<String>)> = self
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .query_row(
                "SELECT configuration,worker_id FROM linkage_configs WHERE controller_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        row.map(|(raw, worker)| {
            serde_json::from_str(&raw)
                .map(|config| (config, worker))
                .map_err(|e| format!("联动配置损坏：{e}"))
        })
        .transpose()
    }
    pub fn linkage_children(&self, parent: &str) -> Result<Vec<Session>, String> {
        let ids = {
            let conn = self.0.lock().map_err(|e| e.to_string())?;
            let mut q = conn
                .prepare("SELECT worker_id FROM linkage_workers WHERE controller_id=?1")
                .map_err(|e| e.to_string())?;
            let rows = q
                .query_map([parent], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?
        };
        ids.iter().map(|id| self.session(id)).collect()
    }
    pub fn save_linkage(&self, id: &str, config: &Configuration) -> Result<(), String> {
        let session = self.session(id)?;
        if self.linkage_parent(id)?.is_some() {
            return Err("执行会话不能再次创建联动".into());
        }
        if config.controller.agent_id != session.agent
            || Some(config.controller.model_id.as_str()) != session.model.as_deref()
            || Some(config.controller.connection_id.as_str()) != session.connection_id.as_deref()
        {
            return Err("主控配置与当前聊天不一致，请先切换聊天模型".into());
        }
        config.controller.validate()?;
        self.route_for(
            &config.controller.agent_id,
            &config.controller.connection_id,
        )?;
        if config.enabled() {
            config.executor.validate()?;
            self.route_for(&config.executor.agent_id, &config.executor.connection_id)?;
        }
        let previous = self.linkage_configuration(id)?;
        let worker = previous
            .as_ref()
            .filter(|(old, _)| old.executor == config.executor)
            .and_then(|(_, worker)| worker.as_deref());
        let reset = previous
            .as_ref()
            .map(|(old, _)| old != config)
            .unwrap_or(config.enabled());
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        crate::connection_usage::ensure_session_idle(&tx, id)?;
        if active_children(&tx, id)? {
            return Err("请先停止或完成执行 Agent 的任务".into());
        }
        tx.execute("INSERT INTO linkage_configs(controller_id,configuration,worker_id) VALUES(?1,?2,?3) ON CONFLICT(controller_id) DO UPDATE SET configuration=excluded.configuration,worker_id=excluded.worker_id",params![id,serde_json::to_string(config).map_err(|e|e.to_string())?,worker]).map_err(|e|e.to_string())?;
        if reset {
            tx.execute("UPDATE sessions SET native_id=NULL WHERE id=?1", [id])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn linkage_worker(&self, parent: &str) -> Result<Session, String> {
        let (config, worker) = self
            .linkage_configuration(parent)?
            .ok_or("当前聊天没有启用联动")?;
        if !config.enabled() {
            return Err("当前聊天没有启用联动".into());
        }
        if let Some(worker) = worker {
            return self.session(&worker);
        }
        self.route_for(&config.executor.agent_id, &config.executor.connection_id)?;
        let root = self.session(parent)?;
        let workspace = self.session_workspace(&root)?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        // Compare-and-set prevents concurrent tool starts from creating two workers.
        let changed=tx.execute("UPDATE linkage_configs SET worker_id=?2 WHERE controller_id=?1 AND worker_id IS NULL",params![parent,id]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("联动执行方已改变，请重新读取状态".into());
        }
        tx.execute("INSERT INTO sessions(id,project_id,title,agent,model,connection_id,status,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'idle',?7)",params![id,(!root.project_id.is_empty()).then_some(&root.project_id),format!("{} · 执行",crate::agents::name(&config.executor.agent_id)),config.executor.agent_id,config.executor.model_id,config.executor.connection_id,crate::storage::now()]).map_err(|e|e.to_string())?;
        tx.execute(
            "INSERT INTO session_workspaces(session_id,path) VALUES(?1,?2)",
            params![id, workspace.path],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO linkage_workers(worker_id,controller_id) VALUES(?1,?2)",
            params![id, parent],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        drop(conn);
        self.session(&id)
    }
}

pub fn delete_children(conn: &Connection, parent: &str) -> rusqlite::Result<()> {
    for sql in [
        "DELETE FROM followups WHERE session_id IN (SELECT worker_id FROM linkage_workers WHERE controller_id=?1)",
        "DELETE FROM messages WHERE session_id IN (SELECT worker_id FROM linkage_workers WHERE controller_id=?1)",
        "DELETE FROM usage_records WHERE session_id IN (SELECT worker_id FROM linkage_workers WHERE controller_id=?1)",
        "DELETE FROM sidebar_items WHERE kind='session' AND id IN (SELECT worker_id FROM linkage_workers WHERE controller_id=?1)",
        "DELETE FROM sessions WHERE id IN (SELECT worker_id FROM linkage_workers WHERE controller_id=?1)",
    ] { conn.execute(sql,[parent])?; }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linkage::Choice;
    fn fixture() -> (Store, Session, Configuration) {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store
            .add_project(std::path::Path::new("D:/linkage-fixture"))
            .unwrap();
        let root = store
            .create_configured_session(
                &project.id,
                Some("main-model".into()),
                "claude",
                Some("@local"),
            )
            .unwrap();
        let config = Configuration {
            mode: "linked".into(),
            controller: Choice {
                agent_id: "claude".into(),
                connection_id: "@local".into(),
                model_id: "main-model".into(),
                effort: None,
                service_tier: None,
            },
            executor: Choice {
                agent_id: "pi".into(),
                connection_id: "@local".into(),
                model_id: "worker-model".into(),
                effort: None,
                service_tier: None,
            },
        };
        (store, root, config)
    }
    #[test]
    fn workers_keep_independent_engine_and_shared_workspace_and_stay_out_of_sidebar() {
        let (store, root, config) = fixture();
        store.save_linkage(&root.id, &config).unwrap();
        let worker = store.linkage_worker(&root.id).unwrap();
        assert_eq!(worker.agent, "pi");
        assert_eq!(worker.model.as_deref(), Some("worker-model"));
        assert_eq!(store.linkage_worker(&root.id).unwrap().id, worker.id);
        assert_eq!(
            store.session_workspace(&root).unwrap().path,
            store.session_workspace(&worker).unwrap().path
        );
        assert_eq!(store.sessions().unwrap().len(), 1);
        assert_eq!(
            store.linkage_parent(&worker.id).unwrap().as_deref(),
            Some(root.id.as_str())
        );
        assert!(store.save_linkage(&worker.id, &config).is_err());
    }
    #[test]
    fn active_worker_blocks_reconfiguration_and_deletion_cleans_owned_history() {
        let (store, root, config) = fixture();
        store.save_linkage(&root.id, &config).unwrap();
        let worker = store.linkage_worker(&root.id).unwrap();
        store.set_status(&worker.id, "running", Some("t")).unwrap();
        assert!(store.save_linkage(&root.id, &config).is_err());
        assert!(store.delete_session(&root.id).is_err());
        store.set_status(&worker.id, "idle", None).unwrap();
        store
            .save_message(
                "m",
                &worker.id,
                "assistant",
                "actual worker report",
                "agentMessage",
                &serde_json::json!({}),
            )
            .unwrap();
        store.delete_session(&root.id).unwrap();
        assert!(store.session(&worker.id).is_err());
    }
    #[test]
    fn all_registered_engines_can_own_either_role_and_invalid_config_is_rejected() {
        for agent in crate::agents::IDS {
            let choice = Choice {
                agent_id: agent.into(),
                connection_id: "@local".into(),
                model_id: "model".into(),
                effort: None,
                service_tier: None,
            };
            assert!(choice.validate().is_ok());
        }
        let (_, _, mut config) = fixture();
        config.executor.agent_id = "unregistered".into();
        assert!(config.executor.validate().is_err());
    }
    #[test]
    fn native_background_work_remains_owned_by_the_controller_after_the_worker_turn_finishes() {
        let (store, root, mut config) = fixture();
        config.executor.agent_id = "codex".into();
        store.save_linkage(&root.id, &config).unwrap();
        let worker = store.linkage_worker(&root.id).unwrap();
        store.observe_connection_usage("worker-process",Some(&worker.id),"@local",&serde_json::json!({"method":"turn/started","params":{"threadId":"native-child","turn":{"id":"child-turn"}}})).unwrap();
        assert_eq!(store.session(&worker.id).unwrap().status, "idle");
        assert!(store.linkage_active(&root.id).unwrap());
        assert_eq!(store.active_session_ids().unwrap(), vec![root.id.clone()]);
        assert!(store.save_linkage(&root.id, &config).is_err());
        assert!(store.delete_session(&root.id).is_err());
        store.clear_connection_usage("worker-process").unwrap();
        assert!(!store.linkage_active(&root.id).unwrap());
        store.save_linkage(&root.id, &config).unwrap();
    }
    #[test]
    fn private_execution_results_do_not_create_extra_badges_or_archived_chats() {
        let (store, root, config) = fixture();
        store.save_linkage(&root.id, &config).unwrap();
        let worker = store.linkage_worker(&root.id).unwrap();
        store.0.lock().unwrap().execute("INSERT INTO sidebar_items(kind,id,unread) VALUES('session',?1,1) ON CONFLICT(kind,id) DO UPDATE SET unread=1",[&worker.id]).unwrap();
        assert_eq!(store.unread_result_count().unwrap(), 0);
        store.project_action(&root.project_id, "archive").unwrap();
        let archived = store.archived_sessions().unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].session.id, root.id);
    }
    #[test]
    fn tool_restrictions_apply_only_to_the_enabled_controller() {
        let (store, root, mut config) = fixture();
        assert!(!store.linkage_controller(&root.id).unwrap());
        store.save_linkage(&root.id, &config).unwrap();
        let worker = store.linkage_worker(&root.id).unwrap();
        assert!(store.linkage_controller(&root.id).unwrap());
        assert!(!store.linkage_controller(&worker.id).unwrap());
        config.mode = "single".into();
        store.save_linkage(&root.id, &config).unwrap();
        assert!(!store.linkage_controller(&root.id).unwrap());
    }
}
