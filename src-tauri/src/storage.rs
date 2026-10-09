use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::Value;
use std::{path::Path, sync::Mutex};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub agent: String,
    pub model: Option<String>,
    pub connection_id: Option<String>,
    pub native_id: Option<String>,
    pub status: String,
    pub updated_at: i64,
    pub turn_id: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub seq: i64,
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub text: String,
    pub kind: String,
    pub data: Value,
}

pub struct Store(pub(crate) Mutex<Connection>);

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

impl Store {
    pub fn open(path: &Path) -> std::result::Result<Self, rusqlite::Error> {
        Self::from_connection(Connection::open(path)?)
    }

    pub(crate) fn from_connection(
        mut conn: Connection,
    ) -> std::result::Result<Self, rusqlite::Error> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             PRAGMA cache_size=-2048;
             CREATE TABLE IF NOT EXISTS projects(id TEXT PRIMARY KEY,name TEXT NOT NULL,path TEXT NOT NULL UNIQUE);
             CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY,project_id TEXT NOT NULL REFERENCES projects(id),title TEXT NOT NULL,agent TEXT NOT NULL,model TEXT,native_id TEXT UNIQUE,status TEXT NOT NULL,updated_at INTEGER NOT NULL,turn_id TEXT,archived INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS messages(seq INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT NOT NULL,session_id TEXT NOT NULL REFERENCES sessions(id),role TEXT NOT NULL,text TEXT NOT NULL,kind TEXT NOT NULL,data TEXT NOT NULL,UNIQUE(session_id,id));
             CREATE INDEX IF NOT EXISTS messages_page ON messages(session_id,seq);
             CREATE TABLE IF NOT EXISTS followups(seq INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT UNIQUE NOT NULL,session_id TEXT NOT NULL REFERENCES sessions(id),payload TEXT NOT NULL,status TEXT NOT NULL,error TEXT);
             CREATE INDEX IF NOT EXISTS followups_order ON followups(session_id,seq);
             UPDATE followups SET status='paused',error='上次未发送完，请确认后继续' WHERE status IN ('queued','sending','steering');
             CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS sidebar_sections(id TEXT PRIMARY KEY,name TEXT NOT NULL,position INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS sidebar_items(kind TEXT NOT NULL,id TEXT NOT NULL,pinned INTEGER NOT NULL DEFAULT 0,unread INTEGER NOT NULL DEFAULT 0,section_id TEXT REFERENCES sidebar_sections(id) ON DELETE SET NULL,removed INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(kind,id));
             CREATE TABLE IF NOT EXISTS usage_records(session_id TEXT NOT NULL REFERENCES sessions(id),turn_id TEXT NOT NULL,agent TEXT NOT NULL,model TEXT NOT NULL,data TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(session_id,turn_id));
             CREATE TABLE IF NOT EXISTS session_context(session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,summary TEXT NOT NULL,through_seq INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS agent_profiles(id TEXT PRIMARY KEY,agent TEXT NOT NULL,name TEXT NOT NULL,config TEXT NOT NULL);
             UPDATE messages SET data=json_set(data,'$.status','interrupted') WHERE json_valid(data) AND json_extract(data,'$.status') IN ('inProgress','running','preparing') AND session_id IN (SELECT id FROM sessions WHERE status IN ('starting','running','waiting'));
             UPDATE sessions SET status='interrupted',turn_id=NULL WHERE status IN ('starting','running','waiting');",
        )?;
        let has_connection = conn
            .prepare("PRAGMA table_info(sessions)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|c| c == "connection_id");
        if !has_connection {
            conn.execute("ALTER TABLE sessions ADD COLUMN connection_id TEXT", [])?;
        }
        #[cfg(windows)]
        {
            let tx = conn.transaction()?;
            let old = {
                let mut query = tx.prepare(
                    "SELECT id,config FROM agent_profiles WHERE config NOT LIKE 'dpapi:%'",
                )?;
                let rows = query
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };
            for (id, raw) in old {
                let encrypted = crate::credentials::seal(&raw).map_err(|e| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(e)))
                })?;
                tx.execute(
                    "UPDATE agent_profiles SET config=?2 WHERE id=?1",
                    params![id, encrypted],
                )?;
            }
            tx.commit()?;
        }
        let store = Self(Mutex::new(conn));
        store.pin_legacy_connections().map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(e)))
        })?;
        store.initialize_connection_orders().map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(e)))
        })?;
        Ok(store)
    }

    pub fn codex_path(&self) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .query_row(
                "SELECT value FROM settings WHERE key='codex_path'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn profiles(&self) -> Result<Vec<crate::ccswitch::Profile>> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id,agent,name,config FROM agent_profiles ORDER BY agent,name")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut profiles = vec![];
        for row in rows {
            let (id, agent, name, raw) = row.map_err(|e| e.to_string())?;
            profiles.push(crate::ccswitch::Profile {
                id,
                agent,
                name,
                config: serde_json::from_str(&crate::credentials::open(&raw)?)
                    .map_err(|_| "本地配置 JSON 无效")?,
            });
        }
        let orders: std::collections::HashMap<_, _> = ["claude", "codex", "opencode", "pi"]
            .into_iter()
            .map(|agent| {
                let ranks: std::collections::HashMap<_, _> =
                    crate::connection_order::order(&conn, agent)?
                        .into_iter()
                        .enumerate()
                        .map(|(rank, id)| (id, rank))
                        .collect();
                Ok((agent, ranks))
            })
            .collect::<Result<_>>()?;
        profiles.sort_by_cached_key(|p| {
            (
                p.agent.clone(),
                orders
                    .get(p.agent.as_str())
                    .and_then(|ranks| ranks.get(&p.id).copied())
                    .unwrap_or(usize::MAX),
            )
        });
        Ok(profiles)
    }
    pub fn profile(&self, agent: &str, id: &str) -> Result<Option<crate::ccswitch::Profile>> {
        use rusqlite::OptionalExtension;
        let row: Option<(String, String)> = self
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .query_row(
                "SELECT name,config FROM agent_profiles WHERE id=?1 AND agent=?2",
                params![id, agent],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        row.map(|(name, raw)| {
            Ok(crate::ccswitch::Profile {
                id: id.into(),
                agent: agent.into(),
                name,
                config: serde_json::from_str(&crate::credentials::open(&raw)?)
                    .map_err(|_| "本地配置 JSON 无效")?,
            })
        })
        .transpose()
    }
    pub fn import_profiles(&self, profiles: &[crate::ccswitch::Profile]) -> Result<()> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for profile in profiles {
            let bound: usize = tx
                .query_row(
                    "SELECT count(*) FROM sessions WHERE connection_id=?1",
                    [&profile.id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if bound > 0 {
                let (agent, raw): (String, String) = tx
                    .query_row(
                        "SELECT agent,config FROM agent_profiles WHERE id=?1",
                        [&profile.id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(|e| e.to_string())?;
                let old = crate::ccswitch::Profile {
                    id: profile.id.clone(),
                    agent,
                    name: String::new(),
                    config: serde_json::from_str(&crate::credentials::open(&raw)?)
                        .map_err(|_| "已保存连接格式无效")?,
                };
                if crate::session_config::routing_identity(&old)
                    != crate::session_config::routing_identity(profile)
                {
                    return Err(
                        "此连接已绑定聊天。更换供应商、API 地址或协议请添加新连接；密钥仍可更新"
                            .into(),
                    );
                }
            }
            let config = crate::credentials::seal(&profile.config.to_string())?;
            tx.execute("INSERT INTO agent_profiles(id,agent,name,config) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET name=excluded.name,agent=excluded.agent,config=excluded.config",params![profile.id,profile.agent,profile.name,config]).map_err(|e|e.to_string())?;
        }
        crate::connection_order::reconcile(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn active_profile(&self, agent: &str) -> Result<Option<crate::ccswitch::Profile>> {
        use rusqlite::OptionalExtension;
        let id: Option<String> = self
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                [format!("profile_{agent}")],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match id {
            Some(id) => self.profile(agent, &id),
            None => Ok(None),
        }
    }
    pub fn delete_profile(&self, id: &str) -> Result<()> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let bound: usize = tx
            .query_row(
                "SELECT count(*) FROM sessions WHERE connection_id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if bound > 0 {
            return Err(
                "还有聊天使用此连接，请先切换这些聊天的供应商，或删除聊天后再移除连接".into(),
            );
        }
        tx.execute(
            "DELETE FROM settings WHERE key IN ('profile_claude','profile_codex') AND value=?1",
            [id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM agent_profiles WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        crate::connection_order::reconcile(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn select_profile(&self, agent: &str, id: Option<&str>) -> Result<()> {
        self.make_connection_default(agent, id.unwrap_or(crate::session_config::LOCAL))
    }
    pub fn official(&self, agent: &str) -> Result<bool> {
        use rusqlite::OptionalExtension;
        let value: Option<String> = self
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                [format!("official_{agent}")],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(value.as_deref() == Some("1"))
    }
    pub fn use_official(&self, agent: &str) -> Result<()> {
        if !matches!(agent, "claude" | "codex") {
            return Err("此 Agent 尚不支持官方账号".into());
        }
        self.make_connection_default(agent, crate::session_config::OFFICIAL)
    }

    pub fn load_mcp(&self) -> Result<bool> {
        use rusqlite::OptionalExtension;
        let value: Option<String> = self
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .query_row("SELECT value FROM settings WHERE key='load_mcp'", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(value.as_deref() == Some("true"))
    }

    pub fn configure_codex(&self, path: Option<&str>, load_mcp: bool) -> Result<()> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        if let Some(path) = path {
            conn.execute("INSERT INTO settings(key,value) VALUES('codex_path',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [path])
        } else {
            conn.execute("DELETE FROM settings WHERE key='codex_path'", [])
        }.map_err(|e| e.to_string())?;
        conn.execute("INSERT INTO settings(key,value) VALUES('load_mcp',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [if load_mcp { "true" } else { "false" }])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id,name,path FROM projects WHERE id NOT IN (SELECT id FROM sidebar_items WHERE kind='project' AND removed=1) ORDER BY name")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Project {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    path: r.get(2)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows
    }

    pub fn add_project(&self, path: &Path) -> Result<Project> {
        let project = Project {
            id: uuid::Uuid::new_v4().to_string(),
            name: path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            path: path.to_string_lossy().into_owned(),
        };
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO projects(id,name,path) VALUES(?1,?2,?3)",
            params![project.id, project.name, project.path],
        )
        .map_err(|e| e.to_string())?;
        let saved = conn
            .query_row(
                "SELECT id,name,path FROM projects WHERE path=?1",
                [project.path],
                |r| {
                    Ok(Project {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        path: r.get(2)?,
                    })
                },
            )
            .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE sidebar_items SET removed=0 WHERE kind='project' AND id=?1",
            [&saved.id],
        )
        .map_err(|e| e.to_string())?;
        Ok(saved)
    }

    pub fn project(&self, id: &str) -> Result<Project> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .query_row("SELECT id,name,path FROM projects WHERE id=?1", [id], |r| {
                Ok(Project {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    path: r.get(2)?,
                })
            })
            .map_err(|_| "项目不存在".into())
    }

    pub fn create_session(&self, project_id: &str, model: Option<String>) -> Result<Session> {
        self.create_agent_session(project_id, model, "codex")
    }

    pub fn create_agent_session(
        &self,
        project_id: &str,
        model: Option<String>,
        agent: &str,
    ) -> Result<Session> {
        self.create_configured_session(project_id, model, agent, None)
    }

    pub fn create_configured_session(
        &self,
        project_id: &str,
        model: Option<String>,
        agent: &str,
        connection_id: Option<&str>,
    ) -> Result<Session> {
        if !matches!(agent, "codex" | "claude" | "opencode" | "pi") {
            return Err("该 Agent 尚未接入执行".into());
        }
        self.project(project_id)?;
        let connection = match connection_id {
            Some(id) => self.route_for(agent, id)?.id,
            None => self.default_connection(agent)?,
        };
        let id = uuid::Uuid::new_v4().to_string();
        self.0.lock().map_err(|e|e.to_string())?.execute(
            "INSERT INTO sessions(id,project_id,title,agent,model,status,updated_at,connection_id) VALUES(?1,?2,'新会话',?5,?3,'idle',?4,?6)",params![id,project_id,model,now(),agent,connection]
        ).map_err(|e|e.to_string())?;
        self.session(&id)
    }

    pub fn sessions(&self) -> Result<Vec<Session>> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let mut stmt=conn.prepare("SELECT id,project_id,title,agent,model,native_id,status,updated_at,turn_id,connection_id FROM sessions WHERE archived=0 AND project_id NOT IN (SELECT id FROM sidebar_items WHERE kind='project' AND removed=1) ORDER BY coalesce((SELECT pinned FROM sidebar_items WHERE kind='session' AND id=sessions.id),0) DESC,updated_at DESC LIMIT 500").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map([], Self::session_row)
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string());
        rows
    }

    pub(crate) fn session_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
        Ok(Session {
            id: r.get(0)?,
            project_id: r.get(1)?,
            title: r.get(2)?,
            agent: r.get(3)?,
            model: r.get(4)?,
            native_id: r.get(5)?,
            status: r.get(6)?,
            updated_at: r.get(7)?,
            turn_id: r.get(8)?,
            connection_id: r.get(9)?,
        })
    }

    pub fn session(&self, id: &str) -> Result<Session> {
        self.0.lock().map_err(|e|e.to_string())?.query_row("SELECT id,project_id,title,agent,model,native_id,status,updated_at,turn_id,connection_id FROM sessions WHERE id=?1",[id],Self::session_row).map_err(|e|e.to_string())
    }

    pub fn for_native(&self, native: &str) -> Result<Session> {
        self.0.lock().map_err(|e|e.to_string())?.query_row("SELECT id,project_id,title,agent,model,native_id,status,updated_at,turn_id,connection_id FROM sessions WHERE native_id=?1",[native],Self::session_row).map_err(|e|e.to_string())
    }

    pub fn claim(&self, id: &str) -> Result<()> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let active: usize = conn
            .query_row(
                "SELECT count(*) FROM sessions WHERE status IN ('starting','running','waiting')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if active > 0 {
            return Err("当前已有任务运行，请先完成或停止它".into());
        }
        let changed=conn.execute("UPDATE sessions SET status='starting',turn_id=NULL,updated_at=?2 WHERE id=?1 AND archived=0 AND status NOT IN ('starting','running','waiting')",params![id,now()]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("该会话已有运行中的任务，或已归档".into());
        }
        Ok(())
    }

    pub fn bind_native(&self, id: &str, native: &str) -> Result<()> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "UPDATE sessions SET native_id=?2 WHERE id=?1",
                params![id, native],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_model(&self, id: &str, model: Option<&str>) -> Result<()> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "UPDATE sessions SET model=?2 WHERE id=?1",
                params![id, model],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(crate) fn restore_native_binding(&self, id: &str, native: Option<&str>) -> Result<()> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "UPDATE sessions SET native_id=?2 WHERE id=?1",
                params![id, native],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_status(&self, id: &str, status: &str, turn: Option<&str>) -> Result<()> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "UPDATE sessions SET status=?2,turn_id=?3,updated_at=?4 WHERE id=?1",
                params![id, status, turn, now()],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn acknowledge_turn_start(&self, id: &str, turn: &str) -> Result<()> {
        // Preserve a native start/completion that arrived before the RPC response.
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "UPDATE sessions SET turn_id=?2,updated_at=?3 WHERE id=?1 AND status='starting'",
                params![id, turn, now()],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn complete_turn(&self, id: &str, turn: &str, status: &str) -> Result<()> {
        if turn.is_empty() || !matches!(status, "completed" | "failed" | "interrupted") {
            return Err("任务完成状态无效".into());
        }
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        // Ignore duplicate completion events and events belonging to an older turn.
        let changed = tx.execute(
            "UPDATE sessions SET status=?3,turn_id=NULL,updated_at=?4 WHERE id=?1 AND turn_id=?2",
            params![id, turn, if status == "completed" { "idle" } else { status }, now()],
        ).map_err(|e| e.to_string())?;
        if changed > 0 && status != "interrupted" {
            tx.execute(
                "INSERT INTO sidebar_items(kind,id,unread) VALUES('session',?1,1) ON CONFLICT(kind,id) DO UPDATE SET unread=1",
                [id],
            ).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn mark_session_read(&self, id: &str, through_seq: i64) -> Result<bool> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        // A delayed read receipt cannot clear a newer result or a running turn.
        let changed = conn.execute(
            "UPDATE sidebar_items SET unread=0 WHERE kind='session' AND id=?1 AND unread=1
             AND EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND status NOT IN ('starting','running','waiting'))
             AND COALESCE((SELECT MAX(seq) FROM messages WHERE session_id=?1),0)<=?2",
            params![id, through_seq],
        ).map_err(|e| e.to_string())?;
        Ok(changed > 0)
    }

    pub fn rename(&self, id: &str, title: &str) -> Result<()> {
        let title: String = title.trim().chars().take(100).collect();
        if title.is_empty() {
            return Err("会话名称不能为空".into());
        }
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "UPDATE sessions SET title=?2 WHERE id=?1",
                params![id, title],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn archive(&self, id: &str) -> Result<()> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let changed=conn.execute("UPDATE sessions SET archived=1 WHERE id=?1 AND status NOT IN ('starting','running','waiting')",[id]).map_err(|e|e.to_string())?;
        if changed == 0 {
            return Err("请先停止运行中的任务".into());
        }
        Ok(())
    }

    pub fn save_message(
        &self,
        id: &str,
        session: &str,
        role: &str,
        text: &str,
        kind: &str,
        data: &Value,
    ) -> Result<()> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        conn.execute("INSERT INTO messages(id,session_id,role,text,kind,data) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session_id,id) DO UPDATE SET text=excluded.text,kind=excluded.kind,data=excluded.data",params![id,session,role,text,kind,data.to_string()]).map_err(|e|e.to_string())?;
        Ok(())
    }

    pub fn messages(&self, id: &str, before: Option<i64>) -> Result<Vec<Message>> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let mut stmt=conn.prepare("SELECT seq,id,session_id,role,text,kind,data FROM messages WHERE session_id=?1 AND seq<?2 ORDER BY seq DESC LIMIT 100").map_err(|e|e.to_string())?;
        let mut items = stmt
            .query_map(params![id, before.unwrap_or(i64::MAX)], |r| {
                let raw: String = r.get(6)?;
                Ok(Message {
                    seq: r.get(0)?,
                    id: r.get(1)?,
                    session_id: r.get(2)?,
                    role: r.get(3)?,
                    text: r.get(4)?,
                    kind: r.get(5)?,
                    data: serde_json::from_str(&raw).unwrap_or(Value::Null),
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        items.reverse();
        Ok(items)
    }

    pub fn running(&self) -> Result<usize> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .query_row(
                "SELECT count(*) FROM sessions WHERE status IN ('starting','running','waiting')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }

    pub fn fail_active(&self) {
        if let Ok(conn) = self.0.lock() {
            let _=conn.execute("UPDATE messages SET data=json_set(data,'$.status','interrupted') WHERE json_valid(data) AND json_extract(data,'$.status') IN ('inProgress','running','preparing') AND session_id IN (SELECT id FROM sessions WHERE status IN ('starting','running','waiting'))",[]);
            let _=conn.execute("UPDATE sessions SET status='interrupted',turn_id=NULL WHERE status IN ('starting','running','waiting')",[]);
        }
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().map_err(|e| e.to_string())?.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn set_automation_configuration(&self, installations: &str, servers: &str) -> Result<()> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (key, value) in [
            ("automation_installations", installations),
            ("tool_servers", servers),
        ] {
            tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn commit_agent_install(&self, agent: &str, launch: &str, select: bool) -> Result<()> {
        if !crate::agents::IDS.contains(&agent) {
            return Err("Agent 无效".into());
        }
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let active: usize = tx
            .query_row(
                "SELECT count(*) FROM sessions WHERE status IN ('starting','running','waiting')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if active > 0 {
            return Err("任务已开始，原 Agent 保持使用，请空闲时重试更新".into());
        }
        let mut keys = vec![format!("agent_install_{agent}")];
        if select {
            keys.push(format!("agent_path_{agent}"));
        }
        for key in keys {
            tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,launch]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn record_usage(&self, session: &Session, turn: &str, mut data: Value) -> Result<()> {
        use rusqlite::OptionalExtension;
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        if data["cumulative"] == true {
            let prior: Option<String> = conn.query_row("SELECT data FROM usage_records WHERE session_id=?1 AND rowid<COALESCE((SELECT rowid FROM usage_records WHERE session_id=?1 AND turn_id=?2),9223372036854775807) ORDER BY rowid DESC LIMIT 1", params![session.id,turn], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
            let prior: Value = prior
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(Value::Null);
            for key in [
                "inputTokens",
                "cachedInputTokens",
                "cacheWriteInputTokens",
                "outputTokens",
                "reasoningOutputTokens",
                "totalTokens",
            ] {
                let raw = data["total"][key].as_u64().unwrap_or(0);
                let before = prior["total"][key].as_u64().unwrap_or(0);
                data["turn"][key] =
                    serde_json::json!(if raw >= before { raw - before } else { raw });
            }
        } else {
            data["turn"] = data["total"].clone();
        }
        conn.execute("INSERT INTO usage_records VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session_id,turn_id) DO UPDATE SET data=excluded.data,model=excluded.model", params![session.id,turn,session.agent,session.model.as_deref().unwrap_or(""),data.to_string(),now()]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn usage_records(&self, session_id: Option<&str>) -> Result<Vec<Value>> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare("SELECT u.session_id,u.turn_id,u.agent,u.model,u.data,u.updated_at,s.title FROM usage_records u JOIN sessions s ON s.id=u.session_id WHERE (?1 IS NULL OR u.session_id=?1) ORDER BY u.updated_at DESC,u.rowid DESC LIMIT 1000").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([session_id], |r| { let data: String=r.get(4)?; Ok(serde_json::json!({"sessionId":r.get::<_,String>(0)?,"turnId":r.get::<_,String>(1)?,"agent":r.get::<_,String>(2)?,"model":r.get::<_,String>(3)?,"data":serde_json::from_str::<Value>(&data).unwrap_or(Value::Null),"at":r.get::<_,i64>(5)?,"title":r.get::<_,String>(6)?})) }).map_err(|e|e.to_string())?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_update_never_switches_selection_during_an_active_turn() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store.commit_agent_install("pi", "old", true).unwrap();
        let project = store.add_project(Path::new("D:/测试")).unwrap();
        let session = store.create_session(&project.id, None).unwrap();
        store.claim(&session.id).unwrap();
        assert!(store.commit_agent_install("pi", "new", true).is_err());
        assert_eq!(
            store.setting("agent_install_pi").unwrap().as_deref(),
            Some("old")
        );
        assert_eq!(
            store.setting("agent_path_pi").unwrap().as_deref(),
            Some("old")
        );
    }
    #[test]
    fn start_acknowledgement_does_not_revive_completed_or_newer_turns() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(Path::new("D:/测试")).unwrap();
        let session = store.create_session(&project.id, None).unwrap();
        store.set_status(&session.id, "idle", Some("old")).unwrap();
        store.claim(&session.id).unwrap();
        assert!(store.session(&session.id).unwrap().turn_id.is_none());
        store.acknowledge_turn_start(&session.id, "first").unwrap();
        assert_eq!(store.session(&session.id).unwrap().status, "starting");
        store
            .set_status(&session.id, "running", Some("first"))
            .unwrap();
        store
            .complete_turn(&session.id, "first", "completed")
            .unwrap();
        store.acknowledge_turn_start(&session.id, "first").unwrap();
        assert_eq!(store.session(&session.id).unwrap().status, "idle");
        store.claim(&session.id).unwrap();
        store
            .set_status(&session.id, "running", Some("second"))
            .unwrap();
        store.acknowledge_turn_start(&session.id, "first").unwrap();
        assert_eq!(
            store.session(&session.id).unwrap().turn_id.as_deref(),
            Some("second")
        );
    }
    #[test]
    fn repair_install_commits_path_and_record_together_or_keeps_previous_selection() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store.commit_agent_install("pi", "old", true).unwrap();
        store.0.lock().unwrap().execute_batch("CREATE TRIGGER reject_agent_override BEFORE UPDATE ON settings WHEN NEW.key='agent_path_pi' BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        assert!(store.commit_agent_install("pi", "new", true).is_err());
        assert_eq!(
            store.setting("agent_install_pi").unwrap().as_deref(),
            Some("old")
        );
        assert_eq!(
            store.setting("agent_path_pi").unwrap().as_deref(),
            Some("old")
        );
        assert!(store.commit_agent_install("unknown", "new", false).is_err());
    }
    #[test]
    fn automation_installation_and_enabled_server_commit_together() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store
            .set_automation_configuration("old-installation", "old-server")
            .unwrap();
        store.0.lock().unwrap().execute_batch("CREATE TRIGGER reject_automation_update BEFORE UPDATE ON settings WHEN NEW.key='tool_servers' BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        assert!(store
            .set_automation_configuration("new-installation", "new-server")
            .is_err());
        assert_eq!(
            store
                .setting("automation_installations")
                .unwrap()
                .as_deref(),
            Some("old-installation")
        );
        assert_eq!(
            store.setting("tool_servers").unwrap().as_deref(),
            Some("old-server")
        );
    }
    #[test]
    fn first_send_binds_the_draft_connection_without_changing_defaults() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(Path::new("D:/draft-test")).unwrap();
        let before = store.default_connection("claude").unwrap();
        let session = store
            .create_configured_session(
                &project.id,
                Some("sonnet".into()),
                "claude",
                Some(crate::session_config::OFFICIAL),
            )
            .unwrap();
        assert_eq!(
            session.connection_id.as_deref(),
            Some(crate::session_config::OFFICIAL)
        );
        assert_eq!(store.default_connection("claude").unwrap(), before);
        assert!(store.messages(&session.id, None).unwrap().is_empty());
        assert_eq!(store.sessions().unwrap().len(), 1);
    }
    #[test]
    fn invalid_draft_connection_does_not_leave_an_empty_chat() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(Path::new("D:/draft-test")).unwrap();
        assert!(store
            .create_configured_session(
                &project.id,
                Some("k3".into()),
                "claude",
                Some("removed-profile")
            )
            .is_err());
        assert!(store.sessions().unwrap().is_empty());
    }
    #[test]
    fn cumulative_usage_updates_deduplicate_and_do_not_subtract_later_turns() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(Path::new("D:/usage-test")).unwrap();
        let session = store.create_session(&project.id, None).unwrap();
        let usage = |input, output| serde_json::json!({"cumulative":true,"total":{"inputTokens":input,"outputTokens":output,"totalTokens":input+output}});
        store.record_usage(&session, "t1", usage(100, 20)).unwrap();
        store.record_usage(&session, "t2", usage(250, 50)).unwrap();
        store.record_usage(&session, "t2", usage(300, 60)).unwrap();
        store.record_usage(&session, "t1", usage(100, 20)).unwrap();
        let rows = store.usage_records(Some(&session.id)).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["data"]["turn"]["inputTokens"], 200);
        assert_eq!(rows[1]["data"]["turn"]["totalTokens"], 120);
        store.record_usage(&session, "t3", usage(30, 10)).unwrap();
        assert_eq!(
            store.usage_records(Some(&session.id)).unwrap()[0]["data"]["turn"]["totalTokens"],
            40
        );
    }
    #[cfg(windows)]
    #[test]
    fn old_plaintext_connections_migrate_without_losing_credentials() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE agent_profiles(id TEXT PRIMARY KEY,agent TEXT NOT NULL,name TEXT NOT NULL,config TEXT NOT NULL)").unwrap();
        conn.execute(
            "INSERT INTO agent_profiles VALUES('old','claude','旧连接',?1)",
            [serde_json::json!({"model":"k3","apiKey":"old-fixture-key"}).to_string()],
        )
        .unwrap();
        let store = Store::from_connection(conn).unwrap();
        let raw: String = store
            .0
            .lock()
            .unwrap()
            .query_row("SELECT config FROM agent_profiles", [], |r| r.get(0))
            .unwrap();
        assert!(raw.starts_with("dpapi:"));
        assert!(!raw.contains("old-fixture-key"));
        assert_eq!(
            store.profiles().unwrap()[0].config["apiKey"],
            "old-fixture-key"
        );
    }
    #[test]
    fn single_connection_lookup_does_not_decode_unrelated_profiles() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        store
            .import_profiles(&[crate::ccswitch::Profile {
                id: "selected".into(),
                agent: "claude".into(),
                name: "中文连接".into(),
                config: serde_json::json!({"model":"fixture-model","apiKey":"fixture-key"}),
            }])
            .unwrap();
        store.select_profile("claude", Some("selected")).unwrap();
        store
            .0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO agent_profiles VALUES('unrelated','pi','损坏的其他连接',?1)",
                [crate::credentials::seal("{").unwrap()],
            )
            .unwrap();
        assert!(store.profiles().is_err());
        let profile = store.profile("claude", "selected").unwrap().unwrap();
        assert_eq!(profile.name, "中文连接");
        assert_eq!(profile.config["apiKey"], "fixture-key");
        assert_eq!(
            store.active_profile("claude").unwrap().unwrap().id,
            "selected"
        );
        assert_eq!(
            store.route_for("claude", "selected").unwrap().config["model"],
            "fixture-model"
        );
        assert!(store.profile("codex", "selected").unwrap().is_none());
        assert!(store.profile("claude", "missing").unwrap().is_none());
        assert!(store.route_for("codex", "selected").is_err());
    }
    #[test]
    fn encrypted_connections_crud_and_official_selection() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let profile = crate::ccswitch::Profile {
            id: "api:test".into(),
            agent: "claude".into(),
            name: "中文连接".into(),
            config: serde_json::json!({"model":"k3","apiKey":"credential-fixture"}),
        };
        store.import_profiles(&[profile]).unwrap();
        let raw: String = store
            .0
            .lock()
            .unwrap()
            .query_row("SELECT config FROM agent_profiles", [], |r| r.get(0))
            .unwrap();
        #[cfg(windows)]
        assert!(raw.starts_with("dpapi:") && !raw.contains("credential-fixture"));
        assert_eq!(
            store.profiles().unwrap()[0].config["apiKey"],
            "credential-fixture"
        );
        store.select_profile("claude", Some("api:test")).unwrap();
        assert!(store.active_profile("claude").unwrap().is_some());
        store.use_official("claude").unwrap();
        assert!(store.official("claude").unwrap());
        assert!(store.active_profile("claude").unwrap().is_none());
        store.select_profile("claude", Some("api:test")).unwrap();
        assert!(!store.official("claude").unwrap());
        store.delete_profile("api:test").unwrap();
        assert!(store.active_profile("claude").unwrap().is_none());
        assert!(store.profiles().unwrap().is_empty());
    }
    #[test]
    fn pagination_and_duplicate_completion_keep_order() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(Path::new("D:/项目/测试 空间")).unwrap();
        assert_eq!(
            project.id,
            store
                .add_project(Path::new("D:/项目/测试 空间"))
                .unwrap()
                .id
        );
        let session = store.create_session(&project.id, None).unwrap();
        for n in 0..125 {
            store
                .save_message(
                    &format!("m{n}"),
                    &session.id,
                    "assistant",
                    "中文输出",
                    "agentMessage",
                    &Value::Null,
                )
                .unwrap();
        }
        store
            .save_message(
                "m124",
                &session.id,
                "assistant",
                "最终输出",
                "agentMessage",
                &Value::Null,
            )
            .unwrap();
        let last = store.messages(&session.id, None).unwrap();
        assert_eq!(last.len(), 100);
        assert_eq!(last.last().unwrap().text, "最终输出");
        let old = store.messages(&session.id, Some(last[0].seq)).unwrap();
        assert_eq!(old.len(), 25);
        assert!(old.last().unwrap().seq < last[0].seq);
        store.claim(&session.id).unwrap();
        assert!(store.claim(&session.id).is_err());
        assert!(store.archive(&session.id).is_err());
        store.set_status(&session.id, "idle", None).unwrap();
        store.archive(&session.id).unwrap();
        assert!(store.sessions().unwrap().is_empty());
    }
}
