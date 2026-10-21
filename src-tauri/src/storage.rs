use rusqlite::{params, Connection, OptionalExtension};
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
    pub workspace_path: Option<String>,
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
pub struct HistoryRun {
    pub id: String,
    pub turn_key: String,
    pub data: Value,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_run: Option<HistoryRun>,
}

pub struct Store(pub(crate) Mutex<Connection>);

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn connection_in_use(conn: &Connection, id: &str) -> Result<bool> {
    crate::connection_usage::in_use(conn, id)
}

pub(crate) fn ensure_connection_idle(conn: &Connection, id: &str) -> Result<()> {
    if connection_in_use(conn, id)? {
        return Err("此连接正被运行中的任务使用，请等待任务完成或停止后再修改".into());
    }
    Ok(())
}

impl Store {
    pub fn open(path: &Path) -> std::result::Result<Self, rusqlite::Error> {
        crate::backup::before_open(path)?;
        let store = Self::from_connection(Connection::open(path)?)?;
        crate::backup::after_open(&store)?;
        Ok(store)
    }

    pub(crate) fn from_connection(
        mut conn: Connection,
    ) -> std::result::Result<Self, rusqlite::Error> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             PRAGMA cache_size=-2048;
             CREATE TABLE IF NOT EXISTS projects(id TEXT PRIMARY KEY,name TEXT NOT NULL,path TEXT NOT NULL UNIQUE);
              CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY,project_id TEXT REFERENCES projects(id),title TEXT NOT NULL,agent TEXT NOT NULL,model TEXT,native_id TEXT UNIQUE,status TEXT NOT NULL,updated_at INTEGER NOT NULL,turn_id TEXT,archived INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS messages(seq INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT NOT NULL,session_id TEXT NOT NULL REFERENCES sessions(id),role TEXT NOT NULL,text TEXT NOT NULL,kind TEXT NOT NULL,data TEXT NOT NULL,UNIQUE(session_id,id));
             CREATE INDEX IF NOT EXISTS messages_page ON messages(session_id,seq);
             CREATE INDEX IF NOT EXISTS messages_turn_context ON messages(session_id,seq) WHERE role='user' OR kind IN ('runMarker','modelSwitch');
             CREATE TABLE IF NOT EXISTS followups(seq INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT UNIQUE NOT NULL,session_id TEXT NOT NULL REFERENCES sessions(id),payload TEXT NOT NULL,status TEXT NOT NULL,error TEXT);
             CREATE INDEX IF NOT EXISTS followups_order ON followups(session_id,seq);
             CREATE TABLE IF NOT EXISTS followup_edits(followup_id TEXT PRIMARY KEY REFERENCES followups(id) ON DELETE CASCADE,token TEXT NOT NULL,previous_status TEXT NOT NULL);
             UPDATE followups SET status='paused',error='上次未发送完，请确认后继续' WHERE status IN ('queued','sending','steering','editing');
             DELETE FROM followup_edits;
             CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS sidebar_sections(id TEXT PRIMARY KEY,name TEXT NOT NULL,position INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS sidebar_items(kind TEXT NOT NULL,id TEXT NOT NULL,pinned INTEGER NOT NULL DEFAULT 0,unread INTEGER NOT NULL DEFAULT 0,section_id TEXT REFERENCES sidebar_sections(id) ON DELETE SET NULL,removed INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(kind,id));
             CREATE TABLE IF NOT EXISTS usage_records(session_id TEXT NOT NULL REFERENCES sessions(id),turn_id TEXT NOT NULL,agent TEXT NOT NULL,model TEXT NOT NULL,data TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(session_id,turn_id));
             CREATE INDEX IF NOT EXISTS usage_recent ON usage_records(updated_at);
             CREATE INDEX IF NOT EXISTS usage_session_recent ON usage_records(session_id,updated_at);
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
        // Rebuild only the legacy NOT NULL constraint. Keep every dependent row
        // and verify foreign keys before committing the atomic migration.
        let required: bool = conn.query_row(
            "SELECT \"notnull\" FROM pragma_table_info('sessions') WHERE name='project_id'",
            [],
            |r| r.get(0),
        )?;
        if required {
            conn.execute_batch("PRAGMA foreign_keys=OFF")?;
            let migrated = (|| -> rusqlite::Result<()> {
                let tx = conn.transaction()?;
                tx.execute_batch("CREATE TABLE sessions_optional(id TEXT PRIMARY KEY,project_id TEXT REFERENCES projects(id),title TEXT NOT NULL,agent TEXT NOT NULL,model TEXT,native_id TEXT UNIQUE,status TEXT NOT NULL,updated_at INTEGER NOT NULL,turn_id TEXT,archived INTEGER NOT NULL DEFAULT 0,connection_id TEXT);
                    INSERT INTO sessions_optional SELECT id,project_id,title,agent,model,native_id,status,updated_at,turn_id,archived,connection_id FROM sessions;
                    DROP TABLE sessions; ALTER TABLE sessions_optional RENAME TO sessions;")?;
                if tx.prepare("PRAGMA foreign_key_check")?.exists([])? {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                tx.commit()
            })();
            conn.execute_batch("PRAGMA foreign_keys=ON")?;
            migrated?;
        }
        conn.execute_batch("CREATE TABLE IF NOT EXISTS session_workspaces(session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,path TEXT NOT NULL)")?;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS sessions_connection ON sessions(connection_id)",
        )?;
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
        let orders: std::collections::HashMap<_, _> =
            ["claude", "codex", "opencode", "pi", "gemini"]
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
    pub(crate) fn save_model_metadata(
        &self,
        expected: &crate::ccswitch::Profile,
        metadata: &Value,
    ) -> Result<bool> {
        self.save_model_catalog(expected, metadata, None)
    }
    pub(crate) fn save_model_catalog(
        &self,
        expected: &crate::ccswitch::Profile,
        metadata: &Value,
        models: Option<&[String]>,
    ) -> Result<bool> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        // Catalog reads may finish during a turn. Preserve the live runtime's
        // configuration fingerprint and let a later refresh save the metadata.
        if connection_in_use(&conn, &expected.id)? {
            return Ok(false);
        }
        let raw: Option<String> = conn
            .query_row(
                "SELECT config FROM agent_profiles WHERE id=?1 AND agent=?2",
                params![expected.id, expected.agent],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(raw) = raw else {
            return Ok(false);
        };
        let mut config: Value = serde_json::from_str(&crate::credentials::open(&raw)?)
            .map_err(|_| "本地配置 JSON 无效")?;
        // An asynchronous catalog response must never undo a newer settings edit
        // or recreate a connection deleted while the request was in flight.
        if config != expected.config {
            return Ok(false);
        }
        config["modelMetadata"] = metadata.clone();
        if let Some(models) = models {
            config["models"] = serde_json::json!(models);
        }
        let sealed = crate::credentials::seal(&config.to_string())?;
        conn.execute(
            "UPDATE agent_profiles SET config=?1 WHERE id=?2 AND agent=?3",
            params![sealed, expected.id, expected.agent],
        )
        .map_err(|e| e.to_string())?;
        Ok(true)
    }
    pub fn import_profiles(&self, profiles: &[crate::ccswitch::Profile]) -> Result<()> {
        self.write_profiles(profiles, false, None)
    }
    pub fn save_profile(
        &self,
        profile: &crate::ccswitch::Profile,
        expected: Option<&crate::ccswitch::Profile>,
    ) -> Result<()> {
        self.write_profiles(std::slice::from_ref(profile), true, expected)
    }
    fn write_profiles(
        &self,
        profiles: &[crate::ccswitch::Profile],
        compare: bool,
        expected: Option<&crate::ccswitch::Profile>,
    ) -> Result<()> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for profile in profiles {
            if compare {
                let row: Option<(String, String, String)> = tx
                    .query_row(
                        "SELECT agent,name,config FROM agent_profiles WHERE id=?1",
                        [&profile.id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                let unchanged = match (expected, row) {
                    (None, None) => true,
                    (Some(expected), Some((agent, name, raw))) => {
                        expected.id == profile.id
                            && expected.agent == agent
                            && expected.name == name
                            && expected.config
                                == serde_json::from_str::<Value>(&crate::credentials::open(&raw)?)
                                    .map_err(|_| "本地配置 JSON 无效")?
                    }
                    _ => false,
                };
                if !unchanged {
                    return Err("连接已被其他窗口修改或删除，请刷新后再保存".into());
                }
            }
            // Claiming a turn uses the same Store lock. Check inside this
            // transaction so a concurrent send cannot start between the guard
            // and the update, and an import batch fails without partial edits.
            ensure_connection_idle(&tx, &profile.id)?;
            let bound: usize = tx
                .query_row(
                    "SELECT count(*) FROM sessions WHERE connection_id=?1",
                    [&profile.id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if bound > 0 {
                let old: Option<(String, String)> = tx
                    .query_row(
                        "SELECT agent,config FROM agent_profiles WHERE id=?1",
                        [&profile.id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                let foreign: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE connection_id=?1 AND agent<>?2)", params![profile.id,profile.agent], |row| row.get(0)).map_err(|e| e.to_string())?;
                if foreign
                    || old
                        .as_ref()
                        .is_some_and(|(agent, _)| agent != &profile.agent)
                {
                    return Err("连接已绑定聊天，不能更换 Agent，请新增连接".into());
                }
                let changed = if let Some((agent, raw)) = old {
                    let old = crate::ccswitch::Profile {
                        id: profile.id.clone(),
                        agent,
                        name: String::new(),
                        config: serde_json::from_str(&crate::credentials::open(&raw)?)
                            .map_err(|_| "已保存连接格式无效")?,
                    };
                    crate::session_config::routing_identity(&old)
                        != crate::session_config::routing_identity(profile)
                } else {
                    true
                };
                if changed {
                    // Idle chats keep their visible history and compression
                    // checkpoints. The next send creates a native session and
                    // injects portable history through the existing handoff.
                    tx.execute(
                        "UPDATE sessions SET native_id=NULL,turn_id=NULL WHERE connection_id=?1",
                        [&profile.id],
                    )
                    .map_err(|e| e.to_string())?;
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
        ensure_connection_idle(&tx, id)?;
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
    pub fn delete_profiles(&self, agent: &str, ids: &[String]) -> Result<()> {
        if !matches!(agent, "claude" | "codex" | "opencode" | "pi" | "gemini") {
            return Err("Agent 无效".into());
        }
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        if ids.is_empty()
            || ids.len() > 500
            || unique.len() != ids.len()
            || ids
                .iter()
                .any(|id| id.is_empty() || matches!(id.as_str(), "@local" | "@official"))
        {
            return Err("请选择有效且不重复的供应商连接".into());
        }
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        // Validate the whole batch under the same lock that claims turns. No
        // selected row is removed when another row is busy or belongs elsewhere.
        for id in ids {
            let owner: Option<(String, String, String)> = tx
                .query_row(
                    "SELECT agent,name,config FROM agent_profiles WHERE id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if owner.as_ref().map(|(agent, _, _)| agent.as_str()) != Some(agent) {
                return Err("连接不存在或不属于当前 Agent，请刷新供应商列表".into());
            }
            let (owner, name, raw) = owner.unwrap();
            let profile = crate::ccswitch::Profile {
                id: id.clone(),
                agent: owner,
                name,
                config: serde_json::from_str(&crate::credentials::open(&raw)?)
                    .map_err(|_| "本地配置 JSON 无效")?,
            };
            if profile.is_official() {
                return Err("官方账号请通过账号管理移除，不能批量删除供应商连接".into());
            }
            ensure_connection_idle(&tx, id)?;
        }
        for id in ids {
            tx.execute(
                "DELETE FROM agent_profiles WHERE id=?1 AND agent=?2",
                params![id, agent],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM settings WHERE key=?1 AND value=?2",
                params![format!("profile_{agent}"), id],
            )
            .map_err(|e| e.to_string())?;
        }
        // Keep historical chats and their binding. route_for reports a missing
        // connection and asks the user to choose a model rather than silently
        // routing those chats through another provider or credential.
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

    pub fn session_workspace(&self, session: &Session) -> Result<Project> {
        if !session.project_id.is_empty() {
            return self.project(&session.project_id);
        }
        let path = session
            .workspace_path
            .as_ref()
            .ok_or("聊天工作目录不存在")?;
        Ok(Project {
            id: String::new(),
            name: "聊天文件".into(),
            path: path.clone(),
        })
    }

    pub fn workspace(&self, project_id: &str, session_id: Option<&str>) -> Result<Project> {
        if !project_id.is_empty() {
            return self.project(project_id);
        }
        self.session_workspace(&self.session(session_id.ok_or("请先发送消息创建聊天工作目录")?)?)
    }

    pub fn create_projectless_session(
        &self,
        root: &Path,
        model: Option<String>,
        agent: &str,
        connection_id: Option<&str>,
    ) -> Result<Session> {
        if !root.is_absolute() {
            return Err("聊天目录必须是绝对路径".into());
        }
        if !matches!(agent, "codex" | "claude" | "opencode" | "pi" | "gemini") {
            return Err("该 Agent 尚未接入执行".into());
        }
        let connection = match connection_id {
            Some(id) => self.route_for(agent, id)?.id,
            None => self.default_connection(agent)?,
        };
        let id = uuid::Uuid::new_v4().to_string();
        let folder = root
            .join(chrono::Local::now().format("%Y-%m-%d").to_string())
            .join(format!("chat-{id}"));
        std::fs::create_dir_all(&folder).map_err(|e| format!("无法创建聊天目录：{e}"))?;
        let result = (|| -> Result<()> {
            let mut conn = self.0.lock().map_err(|e| e.to_string())?;
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO sessions(id,project_id,title,agent,model,status,updated_at,connection_id) VALUES(?1,NULL,'新会话',?2,?3,'idle',?4,?5)", params![id,agent,model,now(),connection]).map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO session_workspaces(session_id,path) VALUES(?1,?2)",
                params![id, folder.to_string_lossy()],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir(&folder);
        }
        result?;
        self.session(&id)
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
        if !matches!(agent, "codex" | "claude" | "opencode" | "pi" | "gemini") {
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
        let mut stmt=conn.prepare("SELECT id,project_id,title,agent,model,native_id,status,updated_at,turn_id,connection_id,(SELECT path FROM session_workspaces WHERE session_id=sessions.id) FROM sessions WHERE archived=0 AND (project_id IS NULL OR project_id NOT IN (SELECT id FROM sidebar_items WHERE kind='project' AND removed=1)) ORDER BY coalesce((SELECT pinned FROM sidebar_items WHERE kind='session' AND id=sessions.id),0) DESC,updated_at DESC LIMIT 500").map_err(|e|e.to_string())?;
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
            project_id: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            workspace_path: r.get(10)?,
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
        self.0.lock().map_err(|e|e.to_string())?.query_row("SELECT id,project_id,title,agent,model,native_id,status,updated_at,turn_id,connection_id,(SELECT path FROM session_workspaces WHERE session_id=sessions.id) FROM sessions WHERE id=?1",[id],Self::session_row).map_err(|e|e.to_string())
    }

    pub fn for_native(&self, native: &str) -> Result<Session> {
        self.0.lock().map_err(|e|e.to_string())?.query_row("SELECT id,project_id,title,agent,model,native_id,status,updated_at,turn_id,connection_id,(SELECT path FROM session_workspaces WHERE session_id=sessions.id) FROM sessions WHERE native_id=?1",[native],Self::session_row).map_err(|e|e.to_string())
    }

    pub fn claim(&self, id: &str) -> Result<()> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let changed=conn.execute("UPDATE sessions SET status='starting',turn_id=NULL,updated_at=?2 WHERE id=?1 AND archived=0 AND status NOT IN ('starting','running','waiting')",params![id,now()]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("该会话已有运行中的任务，或已归档".into());
        }
        Ok(())
    }

    pub fn claim_configuration_change(&self, id: &str) -> Result<()> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        crate::connection_usage::ensure_session_idle(&tx, id)?;
        let changed = tx.execute("UPDATE sessions SET status='starting',turn_id=NULL,updated_at=?2 WHERE id=?1 AND archived=0 AND status NOT IN ('starting','running','waiting')", params![id, now()]).map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("会话已归档或状态已改变".into());
        }
        tx.commit().map_err(|e| e.to_string())
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
        crate::connection_usage::ensure_session_idle(&conn, id)?;
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

    pub fn has_subagent(&self, session: &str, child: &str) -> Result<bool> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE session_id=?1 AND kind IN ('collabAgentToolCall','collabToolCall') AND json_valid(data) AND (EXISTS(SELECT 1 FROM json_each(data,'$.receiverThreadIds') WHERE value=?2) OR EXISTS(SELECT 1 FROM json_each(data,'$.agentsStates') WHERE key=?2)))",
            params![session, child], |row| row.get(0),
        ).map_err(|e| e.to_string())
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
                    history_run: None,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        items.reverse();
        // A page can begin inside a long turn, after its lifecycle marker. Carry
        // that marker as context, not an extra row: row count and cursor must still
        // describe exactly the 100 persisted messages requested by pagination.
        if let Some(first) = items
            .first_mut()
            .filter(|m| m.role != "user" && m.kind != "modelSwitch")
        {
            let context = if first.kind == "runMarker" {
                Some((
                    first.seq,
                    first.id.clone(),
                    first.kind.clone(),
                    first.data.to_string(),
                ))
            } else {
                conn.query_row(
                "SELECT seq,id,kind,data FROM messages WHERE session_id=?1 AND seq<?2 AND (role='user' OR kind IN ('runMarker','modelSwitch')) ORDER BY seq DESC LIMIT 1",
                params![id, first.seq],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
            ).optional().map_err(|e| e.to_string())?
            };
            if let Some((seq, marker_id, _, raw)) =
                context.filter(|(_, _, kind, _)| kind == "runMarker")
            {
                let data: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                let same_turn = first.data["turnId"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .map(|id| data["turnId"].as_str() == Some(id))
                    .unwrap_or(true);
                if same_turn {
                    // Match the same UI turn key used when its user message is
                    // eventually loaded, preserving manually expanded details.
                    let boundary = conn.query_row(
                        "SELECT id,role,data FROM messages WHERE session_id=?1 AND seq<?2 AND (role='user' OR kind IN ('runMarker','modelSwitch')) AND kind<>'runMarker' ORDER BY seq DESC LIMIT 1",
                        params![id, seq],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                    ).optional().map_err(|e| e.to_string())?;
                    let turn_key = boundary
                        .map(|(id, role, raw)| {
                            let metadata: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                            if role == "user" {
                                metadata["clientMessageId"]
                                    .as_str()
                                    .map(str::to_owned)
                                    .unwrap_or(id)
                            } else {
                                id
                            }
                        })
                        .unwrap_or_else(|| marker_id.clone());
                    first.history_run = Some(HistoryRun {
                        id: marker_id,
                        turn_key,
                        data,
                    });
                }
            }
        }
        Ok(items)
    }

    pub fn running(&self) -> Result<usize> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        crate::connection_usage::running(&conn)
    }

    pub fn fail_active(&self) {
        if let Ok(conn) = self.0.lock() {
            let _=conn.execute("UPDATE messages SET data=json_set(data,'$.status','interrupted') WHERE json_valid(data) AND json_extract(data,'$.status') IN ('inProgress','running','preparing') AND session_id IN (SELECT id FROM sessions WHERE status IN ('starting','running','waiting'))",[]);
            let _=conn.execute("UPDATE sessions SET status='interrupted',turn_id=NULL WHERE status IN ('starting','running','waiting')",[]);
        }
    }

    pub fn fail_session(&self, id: &str) {
        if let Ok(conn) = self.0.lock() {
            let _ = conn.execute("UPDATE messages SET data=json_set(data,'$.status','interrupted') WHERE session_id=?1 AND json_valid(data) AND json_extract(data,'$.status') IN ('inProgress','running','preparing') AND EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND status IN ('starting','running','waiting'))", [id]);
            let _ = conn.execute("UPDATE sessions SET status='interrupted',turn_id=NULL WHERE id=?1 AND status IN ('starting','running','waiting')", [id]);
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
        let existing: Option<String> = conn
            .query_row(
                "SELECT data FROM usage_records WHERE session_id=?1 AND turn_id=?2",
                params![session.id, turn],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let existing = existing.and_then(|text| serde_json::from_str::<Value>(&text).ok());
        // Cumulative counters belong to one native thread, not the portable
        // conversation that can switch suppliers, workspaces or native IDs.
        if data.is_object() {
            data["counterEpoch"] = serde_json::json!(session.native_id);
        }
        if data["total"].as_object().is_none_or(|v| v.is_empty()) {
            if let Some(mut existing) = existing {
                for key in ["contextTokens", "modelContextWindow"] {
                    if let Some(value) = data.get(key) {
                        existing[key] = value.clone();
                    }
                }
                if data["last"]
                    .as_object()
                    .is_some_and(|value| !value.is_empty())
                {
                    existing["last"] = data["last"].clone();
                }
                data = existing;
            } else {
                data["cumulative"] = serde_json::json!(false);
                data["turn"] = serde_json::json!({});
            }
            conn.execute("INSERT INTO usage_records VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session_id,turn_id) DO UPDATE SET data=excluded.data,updated_at=excluded.updated_at", params![session.id,turn,session.agent,session.model.as_deref().unwrap_or(""),data.to_string(),now()]).map_err(|e|e.to_string())?;
            return Ok(());
        }
        // Billing and context can arrive in separate events. Missing metadata
        // does not erase the latest measured context; explicit null still resets it.
        if let Some(existing) = existing {
            for key in ["contextTokens", "modelContextWindow"] {
                if data.get(key).is_none() {
                    if let Some(value) = existing.get(key) {
                        data[key] = value.clone();
                    }
                }
            }
            if data["last"].as_object().is_none_or(|last| last.is_empty())
                && !data["contextTokens"].is_null()
            {
                data["last"] = existing["last"].clone();
            }
        }
        if data["cumulative"] == true {
            let prior: Option<String> = conn.query_row("SELECT data FROM usage_records WHERE session_id=?1 AND json_extract(data,'$.counterEpoch') IS ?3 AND rowid<COALESCE((SELECT rowid FROM usage_records WHERE session_id=?1 AND turn_id=?2),9223372036854775807) ORDER BY rowid DESC LIMIT 1", params![session.id,turn,session.native_id], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
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
        conn.execute("INSERT INTO usage_records VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session_id,turn_id) DO UPDATE SET data=excluded.data,model=excluded.model,updated_at=CASE WHEN usage_records.data=excluded.data THEN usage_records.updated_at ELSE excluded.updated_at END", params![session.id,turn,session.agent,session.model.as_deref().unwrap_or(""),data.to_string(),now()]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn usage_records(&self, session_id: Option<&str>) -> Result<Vec<Value>> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let sql = if session_id.is_some() {
            "SELECT u.session_id,u.turn_id,u.agent,u.model,u.data,u.updated_at,s.title FROM usage_records u JOIN sessions s ON s.id=u.session_id WHERE u.session_id=?1 ORDER BY u.updated_at DESC,u.rowid DESC LIMIT 1000"
        } else {
            "SELECT u.session_id,u.turn_id,u.agent,u.model,u.data,u.updated_at,s.title FROM usage_records u JOIN sessions s ON s.id=u.session_id WHERE ?1 IS NULL ORDER BY u.updated_at DESC,u.rowid DESC LIMIT 1000"
        };
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt.query_map([session_id], |r| { let data: String=r.get(4)?; Ok(serde_json::json!({"sessionId":r.get::<_,String>(0)?,"turnId":r.get::<_,String>(1)?,"agent":r.get::<_,String>(2)?,"model":r.get::<_,String>(3)?,"data":serde_json::from_str::<Value>(&data).unwrap_or(Value::Null),"at":r.get::<_,i64>(5)?,"title":r.get::<_,String>(6)?})) }).map_err(|e|e.to_string())?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn subagent_lookup_is_scoped_to_declared_children_of_one_parent() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(Path::new("D:/子代理测试")).unwrap();
        let parent = store.create_session(&project.id, None).unwrap();
        let other = store.create_session(&project.id, None).unwrap();
        let child = "child\".'";
        store
            .save_message(
                "spawn",
                &parent.id,
                "tool",
                "",
                "collabAgentToolCall",
                &serde_json::json!({"receiverThreadIds":[child]}),
            )
            .unwrap();
        assert!(store.has_subagent(&parent.id, child).unwrap());
        assert!(!store.has_subagent(&other.id, child).unwrap());
        assert!(!store.has_subagent(&parent.id, "undeclared").unwrap());
        store
            .save_message(
                "wait",
                &parent.id,
                "tool",
                "",
                "collabToolCall",
                &serde_json::json!({"agentsStates":{"second":{"status":"completed"}}}),
            )
            .unwrap();
        assert!(store.has_subagent(&parent.id, "second").unwrap());
    }

    #[test]
    fn projectless_chats_have_durable_independent_workspaces_for_all_agents() {
        let root =
            std::env::temp_dir().join(format!("supercode-workspaces-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("test.db");
        let store = Store::open(&db).unwrap();
        let mut chats = Vec::new();
        for agent in ["codex", "claude", "opencode", "pi"] {
            let chat = store
                .create_projectless_session(&root.join("files"), None, agent, Some("@local"))
                .unwrap();
            assert!(chat.project_id.is_empty());
            let workspace = store.session_workspace(&chat).unwrap();
            assert!(Path::new(&workspace.path).is_dir());
            assert!(Path::new(&workspace.path).starts_with(root.join("files")));
            std::fs::write(
                Path::new(&workspace.path).join("result.txt"),
                agent.as_bytes(),
            )
            .unwrap();
            chats.push(chat);
        }
        assert!(store.projects().unwrap().is_empty());
        assert_eq!(store.sessions().unwrap().len(), 4);
        assert_ne!(chats[0].workspace_path, chats[1].workspace_path);
        let fork = store.fork_session(&chats[0].id, None).unwrap();
        assert_eq!(fork.workspace_path, chats[0].workspace_path);
        store.archive(&chats[1].id).unwrap();
        assert_eq!(
            store.archived_sessions().unwrap()[0].project_name,
            "无项目聊天"
        );
        store.restore_session(&chats[1].id).unwrap();
        let project = store.add_project(&root).unwrap();
        store.bind_native(&chats[2].id, "native").unwrap();
        store.move_session(&chats[2].id, &project.id).unwrap();
        let moved = store.session(&chats[2].id).unwrap();
        assert_eq!(moved.project_id, project.id);
        assert!(moved.native_id.is_none());
        assert_eq!(store.session_workspace(&moved).unwrap().path, project.path);
        drop(store);
        let reopened = Store::open(&db).unwrap();
        let workspace = reopened.workspace("", Some(&chats[0].id)).unwrap();
        assert_eq!(
            std::fs::read_to_string(Path::new(&workspace.path).join("result.txt")).unwrap(),
            "codex"
        );
        assert!(reopened.workspace("", Some("missing")).is_err());
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_session_constraint_migration_preserves_history_and_foreign_keys() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE projects(id TEXT PRIMARY KEY,name TEXT NOT NULL,path TEXT NOT NULL UNIQUE);
            CREATE TABLE sessions(id TEXT PRIMARY KEY,project_id TEXT NOT NULL REFERENCES projects(id),title TEXT NOT NULL,agent TEXT NOT NULL,model TEXT,native_id TEXT UNIQUE,status TEXT NOT NULL,updated_at INTEGER NOT NULL,turn_id TEXT,archived INTEGER NOT NULL DEFAULT 0);
            INSERT INTO projects VALUES('p','旧项目','D:/legacy');
            INSERT INTO sessions(id,project_id,title,agent,status,updated_at) VALUES('old','p','旧聊天','codex','idle',1);") .unwrap();
        // Add history before migration, as an existing user's database would have.
        conn.execute_batch("CREATE TABLE messages(seq INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT NOT NULL,session_id TEXT NOT NULL REFERENCES sessions(id),role TEXT NOT NULL,text TEXT NOT NULL,kind TEXT NOT NULL,data TEXT NOT NULL,UNIQUE(session_id,id));
            INSERT INTO messages(id,session_id,role,text,kind,data) VALUES('m','old','user','保留内容','userMessage','{}');
            CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT INTO settings VALUES('theme','dark');").unwrap();
        let store = Store::from_connection(conn).unwrap();
        assert_eq!(store.session("old").unwrap().project_id, "p");
        assert_eq!(store.setting("theme").unwrap().as_deref(), Some("dark"));
        let conn = store.0.lock().unwrap();
        assert_eq!(
            conn.query_row(
                "SELECT text FROM messages WHERE session_id='old'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "保留内容"
        );
        assert!(!conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap());
        assert!(conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, bool>(0))
            .unwrap());
        assert!(conn.execute("INSERT INTO messages(id,session_id,role,text,kind,data) VALUES('bad','missing','user','x','userMessage','{}')", []).is_err());
    }

    use super::*;
    #[test]
    fn all_agents_can_claim_multiple_sessions_and_failure_is_local() {
        let store = std::sync::Arc::new(
            Store::from_connection(Connection::open_in_memory().unwrap()).unwrap(),
        );
        let project = store.add_project(Path::new("D:/并发会话")).unwrap();
        let mut sessions = Vec::new();
        for agent in ["codex", "claude", "opencode", "pi"] {
            for _ in 0..2 {
                sessions.push(
                    store
                        .create_agent_session(&project.id, None, agent)
                        .unwrap(),
                );
            }
        }
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(sessions.len()));
        let tasks = sessions
            .iter()
            .map(|session| {
                let (store, barrier, id) = (store.clone(), barrier.clone(), session.id.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    store.claim(&id)
                })
            })
            .collect::<Vec<_>>();
        for task in tasks {
            task.join().unwrap().unwrap();
        }
        assert_eq!(store.running().unwrap(), 8);
        for session in &sessions {
            assert!(store.claim(&session.id).is_err());
        }
        store
            .set_status(&sessions[1].id, "waiting", Some("approval"))
            .unwrap();
        store.fail_session(&sessions[0].id);
        assert_eq!(
            store.session(&sessions[0].id).unwrap().status,
            "interrupted"
        );
        assert_eq!(store.session(&sessions[1].id).unwrap().status, "waiting");
        assert_eq!(store.running().unwrap(), 7);
        store.claim(&sessions[0].id).unwrap();
        assert_eq!(store.running().unwrap(), 8);
    }
    #[test]
    fn concurrent_duplicate_sends_claim_a_session_only_once() {
        let store = std::sync::Arc::new(
            Store::from_connection(Connection::open_in_memory().unwrap()).unwrap(),
        );
        let project = store.add_project(Path::new("D:/重复发送")).unwrap();
        let id = store.create_session(&project.id, None).unwrap().id;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let tasks = (0..2)
            .map(|_| {
                let (store, barrier, id) = (store.clone(), barrier.clone(), id.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    store.claim(&id).is_ok()
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            tasks
                .into_iter()
                .filter_map(|task| task.join().ok())
                .filter(|claimed| *claimed)
                .count(),
            1
        );
    }
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
    fn context_only_usage_survives_reload_without_erasing_billing_or_reusing_another_turn() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let p = store.add_project(Path::new("D:/context-fixture")).unwrap();
        let s = store.create_session(&p.id, Some("glm".into())).unwrap();
        store.record_usage(&s, "one", serde_json::json!({"total":{"inputTokens":1000,"outputTokens":200},"last":{},"cumulative":false,"contextTokens":300,"costUsd":0.25})).unwrap();
        store.record_usage(&s, "one", serde_json::json!({"total":{},"last":{},"contextTokens":400,"modelContextWindow":128000})).unwrap();
        let record = store.usage_records(Some(&s.id)).unwrap().remove(0);
        assert_eq!(record["data"]["contextTokens"], 400);
        assert_eq!(record["data"]["modelContextWindow"], 128000);
        assert_eq!(record["data"]["turn"]["inputTokens"], 1000);
        assert_eq!(record["data"]["costUsd"], 0.25);
        store
            .record_usage(
                &s,
                "two",
                serde_json::json!({"total":{},"last":{},"contextTokens":12345}),
            )
            .unwrap();
        let record = store.usage_records(Some(&s.id)).unwrap().remove(0);
        assert_eq!(record["turnId"], "two");
        assert_eq!(record["data"]["contextTokens"], 12345);
        assert_eq!(record["data"]["turn"], serde_json::json!({}));
        assert!(record["data"]["costUsd"].is_null());
        store
            .record_usage(
                &s,
                "two",
                serde_json::json!({"total":{},"last":{},"contextTokens":null}),
            )
            .unwrap();
        assert!(store.usage_records(Some(&s.id)).unwrap()[0]["data"]["contextTokens"].is_null());
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
    #[test]
    fn cumulative_usage_baselines_follow_native_threads_across_rebinding() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(Path::new("D:/usage-epochs")).unwrap();
        let chat = store.create_session(&project.id, None).unwrap();
        let usage = |total| serde_json::json!({"cumulative":true,"total":{"totalTokens":total}});
        store.bind_native(&chat.id, "old-thread").unwrap();
        store
            .record_usage(&store.session(&chat.id).unwrap(), "old", usage(100))
            .unwrap();
        store.bind_native(&chat.id, "new-thread").unwrap();
        let current = store.session(&chat.id).unwrap();
        store
            .record_usage(&current, "new-first", usage(200))
            .unwrap();
        store
            .record_usage(&current, "new-next", usage(350))
            .unwrap();
        store
            .record_usage(&current, "new-first", usage(200))
            .unwrap();
        let rows = store.usage_records(Some(&chat.id)).unwrap();
        for (id, total) in [("old", 100), ("new-first", 200), ("new-next", 150)] {
            assert_eq!(
                rows.iter().find(|row| row["turnId"] == id).unwrap()["data"]["turn"]["totalTokens"],
                total
            );
        }
    }
    #[test]
    fn separately_reported_context_and_billing_restore_for_every_agent() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(Path::new("D:/usage-fixture")).unwrap();
        for agent in ["claude", "codex", "opencode", "pi", "gemini"] {
            let session = store
                .create_configured_session(&project.id, None, agent, Some("@local"))
                .unwrap();
            let context = serde_json::json!({"total":{},"last":{"inputTokens":12000,"outputTokens":300,"totalTokens":12300},"contextTokens":12300,"modelContextWindow":128000});
            store.record_usage(&session, "turn", context).unwrap();
            let bill = serde_json::json!({"total":{"inputTokens":60000,"outputTokens":900},"last":{},"cumulative":agent=="codex"});
            store.record_usage(&session, "turn", bill.clone()).unwrap();
            let record = store.usage_records(Some(&session.id)).unwrap().remove(0);
            assert_eq!(record["data"]["contextTokens"], 12300);
            assert_eq!(record["data"]["modelContextWindow"], 128000);
            assert_eq!(record["data"]["last"]["totalTokens"], 12300);
            assert_eq!(record["data"]["total"]["inputTokens"], 60000);
            store.record_usage(&session, "other", bill.clone()).unwrap();
            assert!(
                store.usage_records(Some(&session.id)).unwrap()[0]["data"]["contextTokens"]
                    .is_null()
            );
            let mut reset = bill;
            reset["contextTokens"] = Value::Null;
            store.record_usage(&session, "turn", reset).unwrap();
            let records = store.usage_records(Some(&session.id)).unwrap();
            let record = records.iter().find(|r| r["turnId"] == "turn").unwrap();
            assert!(record["data"]["contextTokens"].is_null());
            assert_eq!(record["data"]["last"], serde_json::json!({}));
        }
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
    fn delayed_model_metadata_cannot_overwrite_settings_or_recreate_a_deleted_connection() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let original = crate::ccswitch::Profile {
            id: "models".into(),
            agent: "pi".into(),
            name: "Original".into(),
            config: serde_json::json!({"apiKey":"old-key","model":"glm-5.3"}),
        };
        store.import_profiles(&[original.clone()]).unwrap();
        let metadata = serde_json::json!({"glm-5.3":{"contextWindow":1000000}});
        assert!(store.save_model_metadata(&original, &metadata).unwrap());
        let mut newer = store.profile("pi", "models").unwrap().unwrap();
        assert_eq!(newer.config["modelMetadata"], metadata);
        let models = vec!["gateway/new-model".to_owned()];
        assert!(store.save_model_catalog(&newer, &metadata, Some(&models)).unwrap());
        newer = store.profile("pi", "models").unwrap().unwrap();
        assert_eq!(newer.config["models"], serde_json::json!(models));
        assert_eq!(newer.config["model"], "glm-5.3");
        newer.config["apiKey"] = "new-key".into();
        newer.name = "New name".into();
        store.import_profiles(&[newer.clone()]).unwrap();
        assert!(!store.save_model_metadata(&original, &metadata).unwrap());
        assert!(!store.save_model_catalog(&original, &metadata, Some(&models)).unwrap());
        let saved = store.profile("pi", "models").unwrap().unwrap();
        assert_eq!(saved.config["apiKey"], "new-key");
        assert_eq!(saved.name, "New name");
        store.delete_profile("models").unwrap();
        assert!(!store.save_model_metadata(&newer, &metadata).unwrap());
        assert!(!store.save_model_catalog(&newer, &metadata, Some(&models)).unwrap());
        assert!(store.profile("pi", "models").unwrap().is_none());
    }
    fn connection_fixture(id: &str, agent: &str) -> crate::ccswitch::Profile {
        crate::ccswitch::Profile {
            id: id.into(),
            agent: agent.into(),
            name: id.into(),
            config: serde_json::json!({"baseUrl":"https://api.example.test/v1","protocol":"responses","model":"fixture-model","apiKey":"old-key"}),
        }
    }
    #[test]
    fn idle_route_edits_reset_native_session_but_keep_history_and_summary() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let original = connection_fixture("editable", "pi");
        store.import_profiles(&[original.clone()]).unwrap();
        let project = store
            .add_project(Path::new("D:/route-edit-fixture"))
            .unwrap();
        let chat = store
            .create_configured_session(
                &project.id,
                Some("fixture-model".into()),
                "pi",
                Some("editable"),
            )
            .unwrap();
        store.bind_native(&chat.id, "original-native").unwrap();
        store
            .save_message(
                "history",
                &chat.id,
                "user",
                "keep my goal",
                "userMessage",
                &serde_json::json!({}),
            )
            .unwrap();
        store.0.lock().unwrap().execute("INSERT INTO session_context(session_id,summary,through_seq) VALUES(?1,'preserved summary',1)", [&chat.id]).unwrap();
        let mut edit = original.clone();
        edit.name = "renamed".into();
        edit.config["apiKey"] = "new-key".into();
        store.import_profiles(&[edit.clone()]).unwrap();
        assert_eq!(
            store.session(&chat.id).unwrap().native_id.as_deref(),
            Some("original-native")
        );
        edit.config["protocol"] = "gemini".into();
        edit.config["baseUrl"] = "https://gateway.example/v1beta".into();
        store.import_profiles(&[edit.clone()]).unwrap();
        let saved = store.session(&chat.id).unwrap();
        assert!(saved.native_id.is_none());
        assert!(saved.turn_id.is_none());
        assert_eq!(saved.connection_id.as_deref(), Some("editable"));
        assert_eq!(saved.model, chat.model);
        assert_eq!(
            store.messages(&chat.id, None).unwrap()[0].text,
            "keep my goal"
        );
        assert!(store
            .compressed_context(&chat.id)
            .unwrap()
            .unwrap()
            .contains("preserved summary"));
        assert_eq!(
            store.route("pi", Some(&chat.id)).unwrap().config,
            edit.config
        );
        store.bind_native(&chat.id, "updated-native").unwrap();
        store.claim(&chat.id).unwrap();
        edit.config["protocol"] = "chat".into();
        assert!(store
            .import_profiles(&[connection_fixture("new", "pi"), edit])
            .is_err());
        assert!(store.profile("pi", "new").unwrap().is_none());
        assert_eq!(
            store.session(&chat.id).unwrap().native_id.as_deref(),
            Some("updated-native")
        );
        assert_eq!(
            store.profile("pi", "editable").unwrap().unwrap().config["protocol"],
            "gemini"
        );
    }
    #[test]
    fn profile_saves_compare_existing_config_and_never_resurrect_deleted_credentials() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let original = connection_fixture("saved", "pi");
        store.save_profile(&original, None).unwrap();
        let project = store
            .add_project(Path::new("D:/compare-and-save-fixture"))
            .unwrap();
        let chat = store
            .create_configured_session(&project.id, None, "pi", Some("saved"))
            .unwrap();
        store.bind_native(&chat.id, "native-before-delete").unwrap();
        let mut draft = original.clone();
        draft.name = "stale edit".into();
        let mut newer = original.clone();
        newer.config["apiKey"] = "other-window-key".into();
        store.save_profile(&newer, Some(&original)).unwrap();
        assert!(store
            .save_profile(&draft, Some(&original))
            .unwrap_err()
            .contains("其他窗口"));
        assert_eq!(
            store.profile("pi", "saved").unwrap().unwrap().config["apiKey"],
            "other-window-key"
        );
        store.delete_profiles("pi", &["saved".into()]).unwrap();
        assert!(store.save_profile(&draft, Some(&original)).is_err());
        assert!(store.profile("pi", "saved").unwrap().is_none());
        // Explicit import remains an upsert chosen by the user.
        store.import_profiles(&[draft.clone()]).unwrap();
        assert!(store.profile("pi", "saved").unwrap().is_some());
        assert!(store.session(&chat.id).unwrap().native_id.is_none());
        assert!(store.save_profile(&original, None).is_err());
    }
    #[test]
    fn official_account_in_batch_prevents_all_connection_deletions() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let api = connection_fixture("api", "codex");
        let account = crate::ccswitch::Profile {
            id: "account".into(),
            agent: "codex".into(),
            name: "account".into(),
            config: serde_json::json!({"official":true,"accountId":"official-fixture","model":"fixture-model"}),
        };
        store.import_profiles(&[api, account]).unwrap();
        let order = store.connection_orders().unwrap();
        assert!(store
            .delete_profiles("codex", &["api".into(), "account".into()])
            .unwrap_err()
            .contains("账号管理"));
        assert!(store.profile("codex", "api").unwrap().is_some());
        assert!(store.profile("codex", "account").unwrap().is_some());
        assert_eq!(store.connection_orders().unwrap(), order);
    }
    #[test]
    fn batch_provider_delete_is_atomic_keeps_history_and_reconciles_all_defaults() {
        for agent in ["claude", "codex", "opencode", "pi", "gemini"] {
            let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
            let a = connection_fixture("a", agent);
            let b = connection_fixture("b", agent);
            let other = connection_fixture("foreign", if agent == "pi" { "codex" } else { "pi" });
            store.import_profiles(&[a, b, other]).unwrap();
            store.select_profile(agent, Some("a")).unwrap();
            let project = store
                .add_project(Path::new("D:/batch-delete-fixture"))
                .unwrap();
            let chat = store
                .create_configured_session(
                    &project.id,
                    Some("fixture-model".into()),
                    agent,
                    Some("a"),
                )
                .unwrap();
            store
                .save_message(
                    "original-message",
                    &chat.id,
                    "assistant",
                    "keep historical content",
                    "text",
                    &serde_json::json!({}),
                )
                .unwrap();
            let order = store.connection_orders().unwrap();
            for ids in [
                vec!["a".into(), "foreign".into()],
                vec!["a".into(), "missing".into()],
                vec!["a".into(), "a".into()],
                vec!["a".into(), "@local".into()],
            ] {
                assert!(store.delete_profiles(agent, &ids).is_err());
                assert!(store.profile(agent, "a").unwrap().is_some());
                assert_eq!(store.connection_orders().unwrap(), order);
            }
            for status in ["starting", "running", "waiting"] {
                store.set_status(&chat.id, status, Some("turn")).unwrap();
                assert!(store
                    .delete_profiles(agent, &["b".into(), "a".into()])
                    .unwrap_err()
                    .contains("运行中的任务"));
                assert!(store.profile(agent, "b").unwrap().is_some());
            }
            store.set_status(&chat.id, "idle", None).unwrap();
            store
                .delete_profiles(agent, &["a".into(), "b".into()])
                .unwrap();
            assert!(store.profile(agent, "a").unwrap().is_none());
            assert!(store.profile(agent, "b").unwrap().is_none());
            assert!(store.active_profile(agent).unwrap().is_none());
            assert!(store.connection_orders().unwrap()[agent]
                .iter()
                .all(|id| id != "a" && id != "b"));
            assert_eq!(
                store.session(&chat.id).unwrap().connection_id.as_deref(),
                Some("a")
            );
            assert!(store
                .route(agent, Some(&chat.id))
                .err()
                .unwrap()
                .contains("已删除或不匹配"));
            assert_eq!(
                store.messages(&chat.id, None).unwrap()[0].text,
                "keep historical content"
            );
        }
    }
    #[test]
    fn active_connection_is_locked_while_other_connections_remain_editable() {
        for agent in ["codex", "claude", "opencode", "pi"] {
            let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
            let active = connection_fixture("active", agent);
            let unrelated = connection_fixture("unrelated", agent);
            store
                .import_profiles(&[active.clone(), unrelated.clone()])
                .unwrap();
            let project = store
                .add_project(Path::new("D:/connection-guards"))
                .unwrap();
            let chat = store
                .create_configured_session(&project.id, None, agent, Some(&active.id))
                .unwrap();
            let mut active_edit = active.clone();
            active_edit.name = "Edited name".into();
            active_edit.config["apiKey"] = "new-key".into();
            for status in ["starting", "running", "waiting"] {
                store
                    .set_status(&chat.id, status, Some("turn-fixture"))
                    .unwrap();
                assert!(store
                    .import_profiles(&[active_edit.clone()])
                    .unwrap_err()
                    .contains("运行中的任务"));
                assert!(store
                    .delete_profile(&active.id)
                    .unwrap_err()
                    .contains("运行中的任务"));
                assert_eq!(
                    store.profile(agent, &active.id).unwrap().unwrap().config,
                    active.config
                );
                let mut other_edit = unrelated.clone();
                other_edit.name = format!("Edited during {status}");
                other_edit.config["apiKey"] = "updated-unrelated-key".into();
                store
                    .import_profiles(&[other_edit.clone(), connection_fixture(status, agent)])
                    .unwrap();
                assert_eq!(
                    store.profile(agent, &unrelated.id).unwrap().unwrap().name,
                    other_edit.name
                );
                store.select_profile(agent, Some(status)).unwrap();
                let mut order = store.connection_orders().unwrap()[agent].clone();
                order.reverse();
                store.reorder_connections(agent, &order).unwrap();
                store.delete_profile(status).unwrap();
                assert_eq!(
                    store.session(&chat.id).unwrap().connection_id.as_deref(),
                    Some("active")
                );
                assert_eq!(store.session(&chat.id).unwrap().status, status);
                assert_eq!(
                    store.session(&chat.id).unwrap().turn_id.as_deref(),
                    Some("turn-fixture")
                );
                assert_eq!(
                    store.route(agent, Some(&chat.id)).unwrap().config,
                    active.config
                );
            }
            store.set_status(&chat.id, "idle", None).unwrap();
            store.import_profiles(&[active_edit.clone()]).unwrap();
            assert_eq!(
                store.profile(agent, &active.id).unwrap().unwrap().config["apiKey"],
                "new-key"
            );
            // Inactive history allows rerouting through a fresh native session.
            active_edit.config["baseUrl"] = "https://another.example.test/v1".into();
            store.import_profiles(&[active_edit]).unwrap();
            assert!(store
                .delete_profile(&active.id)
                .unwrap_err()
                .contains("还有聊天使用此连接"));
        }
    }
    #[test]
    fn reimport_batch_rolls_back_all_changes_when_one_connection_is_active() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let active = connection_fixture("active", "codex");
        let unrelated = connection_fixture("unrelated", "codex");
        store
            .import_profiles(&[active.clone(), unrelated.clone()])
            .unwrap();
        let project = store.add_project(Path::new("D:/import-guards")).unwrap();
        let chat = store
            .create_configured_session(&project.id, None, "codex", Some(&active.id))
            .unwrap();
        store.claim(&chat.id).unwrap();
        let before = store.connection_orders().unwrap();
        let mut edited = unrelated;
        edited.name = "Should roll back".into();
        assert!(store
            .import_profiles(&[edited, connection_fixture("new", "codex"), active])
            .is_err());
        assert_eq!(
            store.profile("codex", "unrelated").unwrap().unwrap().name,
            "unrelated"
        );
        assert!(store.profile("codex", "new").unwrap().is_none());
        assert_eq!(store.connection_orders().unwrap(), before);
    }
    #[test]
    fn concurrent_connection_edit_and_turn_claim_have_a_serial_order() {
        let store = std::sync::Arc::new(
            Store::from_connection(Connection::open_in_memory().unwrap()).unwrap(),
        );
        let original = connection_fixture("active", "pi");
        store.import_profiles(&[original.clone()]).unwrap();
        let project = store
            .add_project(Path::new("D:/claim-edit-guards"))
            .unwrap();
        let chat = store
            .create_configured_session(&project.id, None, "pi", Some(&original.id))
            .unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let claim = {
            let store = store.clone();
            let barrier = barrier.clone();
            let id = chat.id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.claim(&id)
            })
        };
        let mut updated = original;
        updated.config["apiKey"] = "new-key".into();
        let edit = {
            let store = store.clone();
            let barrier = barrier.clone();
            let updated = updated.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.import_profiles(&[updated])
            })
        };
        claim.join().unwrap().unwrap();
        let edited_first = edit.join().unwrap().is_ok();
        let route = store.route("pi", Some(&chat.id)).unwrap();
        assert_eq!(
            route.config["apiKey"],
            if edited_first { "new-key" } else { "old-key" }
        );
        assert_eq!(store.session(&chat.id).unwrap().status, "starting");
        assert!(store.import_profiles(&[updated]).is_err());
    }
    #[test]
    fn catalog_metadata_does_not_change_an_active_connection_fingerprint() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let profile = connection_fixture("models", "codex");
        store.import_profiles(&[profile.clone()]).unwrap();
        let project = store.add_project(Path::new("D:/catalog-guards")).unwrap();
        let chat = store
            .create_configured_session(&project.id, None, "codex", Some(&profile.id))
            .unwrap();
        let before = store.route("codex", Some(&chat.id)).unwrap().fingerprint();
        let metadata = serde_json::json!({"fixture-model":{"contextWindow":1000000}});
        for status in ["starting", "running", "waiting"] {
            store.set_status(&chat.id, status, None).unwrap();
            assert!(!store.save_model_metadata(&profile, &metadata).unwrap());
            assert!(!store.save_model_catalog(&profile, &metadata, Some(&["gateway/new-model".into()])).unwrap());
            assert_eq!(
                store.route("codex", Some(&chat.id)).unwrap().fingerprint(),
                before
            );
        }
        store.set_status(&chat.id, "idle", None).unwrap();
        assert!(store.save_model_metadata(&profile, &metadata).unwrap());
        assert_ne!(
            store.route("codex", Some(&chat.id)).unwrap().fingerprint(),
            before
        );
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
    fn long_history_pages_carry_lifecycle_without_changing_rows_or_cursor() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(Path::new("D:/历史/长对话")).unwrap();
        let session = store.create_session(&project.id, None).unwrap();
        store
            .save_message(
                "u",
                &session.id,
                "user",
                "构建页面",
                "userMessage",
                &serde_json::json!({"clientMessageId":"client-u"}),
            )
            .unwrap();
        let marker = serde_json::json!({"turnId":"long","status":"completed","durationMs":123400});
        store
            .save_message(
                "run-long",
                &session.id,
                "activity",
                "",
                "runMarker",
                &marker,
            )
            .unwrap();
        for n in 0..225 {
            store
                .save_message(
                    &format!("m{n}"),
                    &session.id,
                    "assistant",
                    "执行过程",
                    "agentMessage",
                    &serde_json::json!({"turnId":"long"}),
                )
                .unwrap();
        }
        let newest = store.messages(&session.id, None).unwrap();
        assert_eq!(newest.len(), 100);
        assert_eq!(newest[0].id, "m125");
        let context = newest[0].history_run.as_ref().unwrap();
        assert_eq!(context.id, "run-long");
        assert_eq!(context.turn_key, "client-u");
        assert_eq!(context.data, marker);
        assert!(newest[1..]
            .iter()
            .all(|message| message.history_run.is_none()));
        let middle = store.messages(&session.id, Some(newest[0].seq)).unwrap();
        assert_eq!(middle.len(), 100);
        assert_eq!(middle[0].id, "m25");
        assert_eq!(middle[0].history_run.as_ref().unwrap().data, marker);
        let oldest = store.messages(&session.id, Some(middle[0].seq)).unwrap();
        assert_eq!(oldest.len(), 27);
        assert_eq!(oldest[0].id, "u");
        assert!(oldest.iter().all(|message| message.history_run.is_none()));
        let combined = oldest
            .iter()
            .chain(&middle)
            .chain(&newest)
            .collect::<Vec<_>>();
        assert_eq!(combined.len(), 227);
        assert!(combined.windows(2).all(|rows| rows[0].seq < rows[1].seq));
        // The metadata is read-only context, not a rewrite of old JSON records.
        let raw: String = store
            .0
            .lock()
            .unwrap()
            .query_row("SELECT data FROM messages WHERE id='m125'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&raw).unwrap(),
            serde_json::json!({"turnId":"long"})
        );
        // The marker itself may be the first row of another page, with its
        // initiating user on the previous page. The logical turn key stays stable.
        let start_page = store
            .messages(&session.id, Some(oldest[0].seq + 101))
            .unwrap();
        assert_eq!(start_page[0].id, "run-long");
        assert_eq!(
            start_page[0].history_run.as_ref().unwrap().turn_key,
            "client-u"
        );
    }

    #[test]
    fn page_context_preserves_terminal_errors_and_respects_user_and_model_boundaries() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(Path::new("D:/历史/运行状态")).unwrap();
        let session = store.create_session(&project.id, None).unwrap();
        for status in ["failed", "interrupted", "inProgress"] {
            store
                .save_message(
                    "run-old",
                    &session.id,
                    "activity",
                    "",
                    "runMarker",
                    &serde_json::json!({"turnId":"old","status":status}),
                )
                .unwrap();
            for n in 0..101 {
                store
                    .save_message(
                        &format!("m{n}"),
                        &session.id,
                        "assistant",
                        "部分输出",
                        "agentMessage",
                        &serde_json::json!({"turnId":"old"}),
                    )
                    .unwrap();
            }
            let page = store.messages(&session.id, None).unwrap();
            assert_eq!(page[0].history_run.as_ref().unwrap().data["status"], status);
        }
        for (role, kind) in [("user", "userMessage"), ("system", "modelSwitch")] {
            store
                .save_message(
                    &format!("boundary-{kind}"),
                    &session.id,
                    role,
                    "新一轮",
                    kind,
                    &Value::Null,
                )
                .unwrap();
            for n in 0..101 {
                store
                    .save_message(
                        &format!("{kind}-{n}"),
                        &session.id,
                        "assistant",
                        "新的进度",
                        "agentMessage",
                        &Value::Null,
                    )
                    .unwrap();
            }
            assert!(store
                .messages(&session.id, None)
                .unwrap()
                .iter()
                .all(|message| message.history_run.is_none()));
        }
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
    #[test]
    fn repeated_user_text_preserves_distinct_client_acknowledgements() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(Path::new("D:/项目/消息顺序")).unwrap();
        let session = store.create_session(&project.id, None).unwrap();
        let mut identities = Vec::new();
        for _ in 0..2 {
            let client_id = uuid::Uuid::new_v4().to_string();
            crate::client_features::validate_client_message_id(Some(&client_id)).unwrap();
            let metadata = serde_json::json!({"clientMessageId": client_id});
            store
                .save_message(
                    &crate::client_features::user_message_id(&metadata),
                    &session.id,
                    "user",
                    "继续",
                    "userMessage",
                    &metadata,
                )
                .unwrap();
            identities.push(client_id);
        }
        let messages = store.messages(&session.id, None).unwrap();
        assert_eq!(messages.len(), 2);
        assert_ne!(messages[0].id, messages[1].id);
        for (message, client_id) in messages.iter().zip(identities) {
            assert_eq!(message.text, "继续");
            assert_eq!(message.data["clientMessageId"], client_id);
        }
    }
}
