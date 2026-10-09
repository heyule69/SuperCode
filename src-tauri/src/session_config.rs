//! A connection belongs to a conversation. Defaults only affect new conversations.
use crate::{
    ccswitch::Profile,
    storage::{Session, Store},
    AppState,
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

pub const LOCAL: &str = "@local";
pub const OFFICIAL: &str = "@official";

// A conservative portable-history budget, not a claim about a model's token window.
// Native sessions let their Agent manage compaction; cross-connection histories are
// normalized because vendor-specific tool/thinking blocks may be incompatible.
pub fn portable_history(input: &str) -> bool {
    input.len() <= 32 * 1024
        && serde_json::from_str::<Value>(input)
            .ok()
            .is_some_and(|v| v["olderHistoryMayBeOmitted"] == false)
}
fn context_mode(
    force: bool,
    same_connection: bool,
    native: bool,
    input: Option<&str>,
) -> &'static str {
    if !force && same_connection && native {
        "native"
    } else if input.is_none() || !force && input.is_some_and(portable_history) {
        "history"
    } else {
        "summary"
    }
}

pub fn routing_identity(profile: &Profile) -> Value {
    let config = &profile.config;
    let codex =
        toml::from_str::<toml::Table>(config["config"].as_str().unwrap_or("")).unwrap_or_default();
    let provider = codex
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .unwrap_or("openai");
    let endpoint = codex
        .get("model_providers")
        .and_then(|c| c.get(provider))
        .and_then(|c| c.get("base_url"))
        .and_then(toml::Value::as_str);
    let base = config["baseUrl"]
        .as_str()
        .or_else(|| config["env"]["ANTHROPIC_BASE_URL"].as_str())
        .or(endpoint)
        .unwrap_or("");
    json!({"agent":profile.agent,"base":base.trim_end_matches('/'),"protocol":config["protocol"].as_str().unwrap_or(if profile.agent=="claude" {"anthropic"} else {"responses"}),"official":profile.is_official()})
}

pub struct Route {
    pub id: String,
    pub config: Value,
    pub profile: Option<Profile>,
}
impl Route {
    pub fn source(&self, agent: &str) -> Value {
        let mut source = crate::providers::model_source(
            &self.config,
            agent,
            self.profile.as_ref().map(|p| p.name.as_str()),
        );
        if agent == "codex"
            && (self.id == OFFICIAL
                || self.profile.as_ref().is_some_and(Profile::is_official)
                || self.id == LOCAL
                    && source["providerId"] == "openai"
                    && crate::providers::key(&self.config).is_none())
        {
            source["connectionName"] = "ChatGPT 官方账号".into();
        }
        source
    }
    pub fn provider(&self) -> Option<String> {
        if self.config["official"] == true
            || self.profile.as_ref().is_some_and(Profile::is_official)
        {
            return Some("openai".into());
        }
        toml::from_str::<toml::Table>(self.config["config"].as_str().unwrap_or(""))
            .ok()
            .and_then(|c| {
                c.get("model_provider")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
            })
    }
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.id.hash(&mut hash);
        self.config.to_string().hash(&mut hash);
        hash.finish()
    }
}

impl Store {
    pub fn default_connection(&self, agent: &str) -> Result<String, String> {
        Ok(if let Some(p) = self.active_profile(agent)? {
            p.id
        } else if self.official(agent)? {
            OFFICIAL.into()
        } else {
            LOCAL.into()
        })
    }
    pub fn pin_legacy_connections(&self) -> Result<(), String> {
        let rows = {
            let conn = self.0.lock().map_err(|e| e.to_string())?;
            let mut query = conn
                .prepare("SELECT id,agent,model FROM sessions WHERE connection_id IS NULL")
                .map_err(|e| e.to_string())?;
            let rows = query
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?
        };
        let profiles = self.profiles()?;
        for (id, agent, model) in rows {
            let matches: Vec<_> = profiles
                .iter()
                .filter(|p| {
                    p.agent == agent
                        && model
                            .as_ref()
                            .is_some_and(|m| crate::providers::model_ids(&p.config).contains(m))
                })
                .collect();
            let connection = if matches.len() == 1 {
                matches[0].id.clone()
            } else {
                self.default_connection(&agent)?
            };
            self.0
                .lock()
                .map_err(|e| e.to_string())?
                .execute(
                    "UPDATE sessions SET connection_id=?2 WHERE id=?1 AND connection_id IS NULL",
                    params![id, connection],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn route(&self, agent: &str, session_id: Option<&str>) -> Result<Route, String> {
        let id = if let Some(session_id) = session_id {
            let s = self.session(session_id)?;
            if s.agent != agent {
                return Err("会话与 Agent 不匹配".into());
            }
            s.connection_id.ok_or("会话尚未绑定连接")?
        } else {
            self.default_connection(agent)?
        };
        self.route_for(agent, &id)
    }
    pub fn route_for(&self, agent: &str, id: &str) -> Result<Route, String> {
        let profile = if matches!(id, LOCAL | OFFICIAL) {
            None
        } else {
            Some(
                self.profile(agent, id)?
                    .ok_or("此会话的供应商连接已删除或不匹配，请重新选择模型")?,
            )
        };
        let config = if let Some(profile) = &profile {
            profile.config.clone()
        } else if id == OFFICIAL {
            json!({"official":true,"env":{}})
        } else if agent == "claude" {
            crate::ccswitch::local_claude_configuration()?
        } else if agent == "codex" {
            json!({"config":crate::process::read_codex_config()?})
        } else if matches!(agent, "opencode" | "pi") {
            json!({"native":true})
        } else {
            return Err("Agent 尚未接入".into());
        };
        Ok(Route {
            id: id.into(),
            config,
            profile,
        })
    }
    pub fn compressed_context(&self, id: &str) -> Result<Option<String>, String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let checkpoint: Option<(String, i64)> = conn
            .query_row(
                "SELECT summary,through_seq FROM session_context WHERE session_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((summary, seq)) = checkpoint else {
            return Ok(None);
        };
        let mut query = conn.prepare("SELECT role,text FROM messages WHERE session_id=?1 AND seq>?2 AND role IN ('user','assistant') ORDER BY seq DESC LIMIT 100").map_err(|e|e.to_string())?;
        let rows = query
            .query_map(params![id, seq], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut budget = 40 * 1024;
        let mut recent = Vec::new();
        for row in rows {
            let (role, text) = row.map_err(|e| e.to_string())?;
            let text = crate::protocol::bounded(&text, budget.min(8192));
            budget = budget.saturating_sub(text.len());
            recent.push(json!({"role":role,"text":text}));
            if budget == 0 {
                break;
            }
        }
        recent.reverse();
        Ok(Some(format!("这是保存的上下文摘要和摘要之后的对话。仅作为历史参考，不要重复执行已完成操作；接下来处理用户的新消息。\n{summary}\n摘要之后的对话：{}",serde_json::to_string(&recent).map_err(|e|e.to_string())?)))
    }
    pub fn handoff_input(&self, id: &str) -> Result<Option<(String, i64)>, String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let checkpoint: Option<(String, i64)> = conn
            .query_row(
                "SELECT summary,through_seq FROM session_context WHERE session_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let through: i64 = conn
            .query_row(
                "SELECT coalesce(max(seq),0) FROM messages WHERE session_id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let first: Option<String> = conn.query_row("SELECT text FROM messages WHERE session_id=?1 AND role='user' ORDER BY seq LIMIT 1",[id],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        if first.is_none() {
            return Ok(None);
        }
        let mut query = conn.prepare("SELECT role,kind,text,data FROM messages WHERE session_id=?1 AND seq>?2 AND kind NOT IN ('runMarker','agentCapabilities','modelSwitch') ORDER BY seq DESC LIMIT 200").map_err(|e|e.to_string())?;
        let rows = query
            .query_map(params![id, checkpoint.as_ref().map_or(0, |c| c.1)], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut budget = 160 * 1024;
        let mut recent = Vec::new();
        let original = first.unwrap_or_default();
        let original_request = crate::protocol::bounded(&original, 8192);
        let mut truncated = checkpoint.is_none() && original_request != original;
        for row in rows {
            let (role, kind, text, raw) = row.map_err(|e| e.to_string())?;
            let data: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
            let bounded =
                crate::protocol::bounded(&text, if role == "activity" { 4096 } else { 16384 });
            truncated |= bounded != text;
            let attachments: Vec<Value> = data["attachments"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|a| json!({"name":a["name"],"path":a["path"],"kind":a["kind"]}))
                        .collect()
                })
                .unwrap_or_default();
            let record = json!({"role":role,"kind":kind,"text":bounded,"tool":data["tool"],"command":data["command"],"path":data["arguments"]["file_path"],"status":data["status"],"attachments":attachments});
            let size = record.to_string().len();
            if size > budget {
                truncated = true;
                break;
            }
            budget -= size;
            recent.push(record);
        }
        truncated |= recent.len() == 200;
        recent.reverse();
        Ok(Some((json!({"originalRequest":original_request,"previousSummary":checkpoint.map(|c|c.0),"recentHistory":recent,"olderHistoryMayBeOmitted":truncated}).to_string(),through)))
    }
    pub fn commit_model_switch(
        &self,
        session: &Session,
        connection: &str,
        model: &str,
        summary: Option<(&str, i64)>,
        marker: &Value,
    ) -> Result<Session, String> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let native = if marker["contextMode"] == "native"
            && session.connection_id.as_deref() == Some(connection)
        {
            session.native_id.as_deref()
        } else {
            None
        };
        let changed = tx.execute("UPDATE sessions SET connection_id=?2,model=?3,native_id=?8,status='idle',turn_id=NULL,updated_at=?4 WHERE id=?1 AND status='starting' AND connection_id=?5 AND model IS ?6 AND native_id IS ?7",params![session.id,connection,model,crate::storage::now(),session.connection_id,session.model,session.native_id,native]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("会话状态已改变，切换已取消".into());
        }
        if let Some((summary, seq)) = summary {
            tx.execute("INSERT INTO session_context(session_id,summary,through_seq) VALUES(?1,?2,?3) ON CONFLICT(session_id) DO UPDATE SET summary=excluded.summary,through_seq=excluded.through_seq",params![session.id,summary,seq]).map_err(|e|e.to_string())?;
        }
        if summary.is_some() || marker["hasHistory"] == true {
            let text = if summary.is_some() {
                "上下文已压缩"
            } else {
                "上下文已保留"
            };
            tx.execute("INSERT INTO messages(id,session_id,role,text,kind,data) VALUES(?1,?2,'system',?3,'modelSwitch',?4)",params![format!("switch-{}",uuid::Uuid::new_v4()),session.id,text,marker.to_string()]).map_err(|e|e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        drop(conn);
        self.session(&session.id)
    }
}

#[tauri::command]
pub async fn switch_session_model(
    session_id: String,
    connection_id: String,
    model: String,
    app: AppHandle,
) -> Result<Session, String> {
    change_session_model(session_id, connection_id, model, false, app).await
}
#[tauri::command]
pub async fn compact_session_context(
    session_id: String,
    app: AppHandle,
) -> Result<Session, String> {
    let session = app.state::<AppState>().store.session(&session_id)?;
    let model = session.model.ok_or("先发送一条消息，再压缩上下文")?;
    change_session_model(
        session_id,
        session.connection_id.unwrap_or(LOCAL.into()),
        model,
        true,
        app,
    )
    .await
}
async fn change_session_model(
    session_id: String,
    connection_id: String,
    model: String,
    force_compact: bool,
    app: AppHandle,
) -> Result<Session, String> {
    let agents = app.state::<crate::agents::Agents>();
    let _agent_operation = agents
        .operation
        .try_lock()
        .map_err(|_| "Agent 正在安装、更新或测试，请稍后切换模型")?;
    let state = app.state::<AppState>();
    let session = state.store.session(&session_id)?;
    let connection = if connection_id.is_empty() {
        LOCAL
    } else {
        &connection_id
    };
    let model = model.trim();
    if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
        return Err("模型 ID 无效".into());
    }
    let next = state.store.route_for(&session.agent, connection)?;
    if !force_compact
        && session.connection_id.as_deref() == Some(connection)
        && session.model.as_deref() == Some(model)
    {
        return Ok(session);
    }
    let previous = state.store.route(&session.agent, Some(&session_id))?;
    state.store.claim(&session_id)?;
    let _ = app.emit("workspace-updated", ());
    let result = async {
        let input = state.store.handoff_input(&session_id)?;
        let mode = context_mode(force_compact, previous.id == next.id, session.native_id.is_some(), input.as_ref().map(|(s,_)|s.as_str()));
        let summary = if mode == "summary" { if let Some((input, seq)) = &input { Some((crate::handoff::summarize(&app,&session,&previous,input).await?,*seq)) } else { None } } else { None };
        let marker = json!({"from":previous.source(&session.agent),"to":next.source(&session.agent),"fromModel":session.model,"toModel":model,"status":"completed","compactOnly":force_compact,"contextMode":mode,"hasHistory":input.is_some()});
        state.store.commit_model_switch(&session,connection,model,summary.as_ref().map(|(s,seq)|(s.as_str(),*seq)),&marker)
    }.await;
    if result.is_err() {
        // No binding/native id/history has been changed. A failed handoff is retryable.
        let _ = state
            .store
            .set_status(&session_id, &session.status, session.turn_id.as_deref());
    }
    let _ = app.emit("workspace-updated", ());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn store() -> Store {
        Store::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap()
    }
    fn profiles(store: &Store) {
        store.import_profiles(&[
            Profile { id:"kimi".into(),agent:"claude".into(),name:"Kimi".into(),config:json!({"model":"k3","env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com/coding/","ANTHROPIC_MODEL":"k3","ANTHROPIC_AUTH_TOKEN":"test-kimi-key"}}) },
            Profile { id:"glm".into(),agent:"claude".into(),name:"智谱".into(),config:json!({"model":"glm-test","env":{"ANTHROPIC_BASE_URL":"https://open.bigmodel.cn/api/anthropic","ANTHROPIC_MODEL":"glm-test","ANTHROPIC_AUTH_TOKEN":"test-glm-key"}}) },
        ]).unwrap();
    }
    #[test]
    fn switches_only_summarize_when_portable_history_requires_it() {
        let short =
            json!({"recentHistory":[{"text":"keep prior work"}],"olderHistoryMayBeOmitted":false})
                .to_string();
        let long = json!({"recentHistory":[{"text":"x".repeat(33 * 1024)}],"olderHistoryMayBeOmitted":false}).to_string();
        let truncated = json!({"olderHistoryMayBeOmitted":true}).to_string();
        assert_eq!(context_mode(false, true, true, Some(&long)), "native");
        assert_eq!(context_mode(false, false, true, Some(&short)), "history");
        assert_eq!(context_mode(false, false, true, Some(&long)), "summary");
        assert_eq!(
            context_mode(false, false, false, Some(&truncated)),
            "summary"
        );
        assert_eq!(context_mode(true, true, true, Some(&short)), "summary");
        assert_eq!(context_mode(false, false, false, None), "history");
        assert!(!portable_history("not JSON"));
    }
    #[test]
    fn native_model_switch_keeps_native_history_and_portable_switch_keeps_tools() {
        let store = store();
        profiles(&store);
        let project = store.add_project(Path::new("D:/route-test")).unwrap();
        store.select_profile("claude", Some("kimi")).unwrap();
        let session = store
            .create_agent_session(&project.id, Some("k3".into()), "claude")
            .unwrap();
        store.bind_native(&session.id, "original-native").unwrap();
        store
            .save_message(
                "u1",
                &session.id,
                "user",
                "continue my work",
                "userMessage",
                &Value::Null,
            )
            .unwrap();
        store
            .save_message(
                "t1",
                &session.id,
                "activity",
                "saved source",
                "commandExecution",
                &json!({"tool":"Write","arguments":{"file_path":"src/main.ts"}}),
            )
            .unwrap();
        let before = store.session(&session.id).unwrap();
        store.claim(&session.id).unwrap();
        let native = store
            .commit_model_switch(
                &before,
                "kimi",
                "other-model",
                None,
                &json!({"contextMode":"native","hasHistory":true}),
            )
            .unwrap();
        assert_eq!(native.native_id.as_deref(), Some("original-native"));
        assert!(store.compressed_context(&session.id).unwrap().is_none());
        store.claim(&session.id).unwrap();
        let portable = store
            .commit_model_switch(
                &native,
                "glm",
                "glm-test",
                None,
                &json!({"contextMode":"history","hasHistory":true}),
            )
            .unwrap();
        assert!(portable.native_id.is_none());
        assert!(store.compressed_context(&session.id).unwrap().is_none());
        let history = store.history_context(&session.id).unwrap().unwrap();
        assert!(
            history.contains("continue my work")
                && history.contains("saved source")
                && history.contains("src/main.ts")
        );
        let (input, _) = store.handoff_input(&session.id).unwrap().unwrap();
        assert!(portable_history(&input));
        assert_eq!(context_mode(false, false, false, Some(&input)), "history");
        assert_eq!(store.transcript(&session.id).unwrap().len(), 1);
    }
    #[test]
    fn bounded_messages_cannot_be_mistaken_for_complete_portable_history() {
        let store = store();
        profiles(&store);
        let project = store.add_project(Path::new("D:/route-test")).unwrap();
        let session = store
            .create_agent_session(&project.id, None, "claude")
            .unwrap();
        store
            .save_message(
                "u1",
                &session.id,
                "user",
                "goal",
                "userMessage",
                &Value::Null,
            )
            .unwrap();
        store
            .save_message(
                "t1",
                &session.id,
                "activity",
                &"x".repeat(4200),
                "commandExecution",
                &Value::Null,
            )
            .unwrap();
        let (input, _) = store.handoff_input(&session.id).unwrap().unwrap();
        assert!(input.len() < 32 * 1024);
        assert!(!portable_history(&input));
    }
    #[test]
    fn conversations_pin_defaults_and_keep_separate_routes() {
        let store = store();
        profiles(&store);
        let project = store.add_project(Path::new("D:/route-test")).unwrap();
        store.select_profile("claude", Some("kimi")).unwrap();
        let a = store
            .create_agent_session(&project.id, Some("k3".into()), "claude")
            .unwrap();
        store.select_profile("claude", Some("glm")).unwrap();
        let b = store
            .create_agent_session(&project.id, Some("glm-test".into()), "claude")
            .unwrap();
        assert_eq!(store.route("claude", Some(&a.id)).unwrap().id, "kimi");
        assert_eq!(store.route("claude", Some(&b.id)).unwrap().id, "glm");
        assert!(store.route("codex", Some(&a.id)).is_err());
        assert!(store.delete_profile("kimi").is_err());
        assert_ne!(
            store.route("claude", Some(&a.id)).unwrap().fingerprint(),
            store.route("claude", Some(&b.id)).unwrap().fingerprint()
        );
        assert!(!serde_json::to_string(&store.sessions().unwrap())
            .unwrap()
            .contains("test-kimi-key"));
    }
    #[test]
    fn bound_connection_allows_key_rotation_but_requires_new_route_for_endpoint_changes() {
        let store = store();
        profiles(&store);
        let project = store.add_project(Path::new("D:/route-test")).unwrap();
        store.select_profile("claude", Some("kimi")).unwrap();
        store
            .create_agent_session(&project.id, Some("k3".into()), "claude")
            .unwrap();
        let mut profile = store
            .profiles()
            .unwrap()
            .into_iter()
            .find(|p| p.id == "kimi")
            .unwrap();
        profile.config["env"]["ANTHROPIC_AUTH_TOKEN"] = json!("rotated-key");
        store.import_profiles(&[profile.clone()]).unwrap();
        profile.config["env"]["ANTHROPIC_BASE_URL"] = json!("https://another-provider.invalid");
        assert!(store.import_profiles(&[profile]).is_err());
        let retained = store.route("claude", None).unwrap();
        assert_eq!(
            retained.config["env"]["ANTHROPIC_AUTH_TOKEN"],
            "rotated-key"
        );
        assert_eq!(
            retained.config["env"]["ANTHROPIC_BASE_URL"],
            "https://api.kimi.com/coding/"
        );
    }
    #[test]
    fn compressed_switch_preserves_history_and_fork_keeps_newer_dialogue() {
        let store = store();
        profiles(&store);
        let project = store.add_project(Path::new("D:/route-test")).unwrap();
        store.select_profile("claude", Some("kimi")).unwrap();
        let a = store
            .create_agent_session(&project.id, Some("k3".into()), "claude")
            .unwrap();
        store.bind_native(&a.id, "original-native").unwrap();
        store
            .save_message(
                "u1",
                &a.id,
                "user",
                "original goal",
                "userMessage",
                &Value::Null,
            )
            .unwrap();
        store
            .save_message(
                "a1",
                &a.id,
                "assistant",
                "completed work",
                "agentMessage",
                &Value::Null,
            )
            .unwrap();
        let (_, through) = store.handoff_input(&a.id).unwrap().unwrap();
        let before = store.session(&a.id).unwrap();
        store.claim(&a.id).unwrap();
        let switched = store
            .commit_model_switch(
                &before,
                "glm",
                "glm-test",
                Some(("summary with original goal", through)),
                &json!({"toModel":"glm-test"}),
            )
            .unwrap();
        assert_eq!(switched.connection_id.as_deref(), Some("glm"));
        assert_eq!(switched.native_id, None);
        assert_eq!(store.default_connection("claude").unwrap(), "kimi");
        assert_eq!(store.messages(&a.id, None).unwrap().len(), 3);
        store
            .save_message(
                "u2",
                &a.id,
                "user",
                "new instruction after checkpoint",
                "userMessage",
                &Value::Null,
            )
            .unwrap();
        let fork = store.fork_session(&a.id, None).unwrap();
        assert_eq!(fork.connection_id.as_deref(), Some("glm"));
        let context = store.history_context(&fork.id).unwrap().unwrap();
        assert!(context.contains("summary with original goal"));
        assert!(context.contains("new instruction after checkpoint"));
        store.delete_session(&fork.id).unwrap();
    }
    #[test]
    fn stale_switch_cannot_replace_route_or_checkpoint() {
        let store = store();
        profiles(&store);
        let project = store.add_project(Path::new("D:/route-test")).unwrap();
        store.select_profile("claude", Some("kimi")).unwrap();
        let a = store
            .create_agent_session(&project.id, Some("k3".into()), "claude")
            .unwrap();
        store.bind_native(&a.id, "original-native").unwrap();
        store.claim(&a.id).unwrap();
        assert!(store
            .commit_model_switch(
                &a,
                "glm",
                "glm-test",
                Some(("invalid summary", 0)),
                &Value::Null
            )
            .is_err());
        assert_eq!(
            store.session(&a.id).unwrap().native_id.as_deref(),
            Some("original-native")
        );
        assert_eq!(store.route("claude", Some(&a.id)).unwrap().id, "kimi");
        assert!(store.compressed_context(&a.id).unwrap().is_none());
    }
    #[test]
    fn legacy_connections_are_inferred_once_and_survive_reopen() {
        let path = std::env::temp_dir().join(format!("sc-routes-{}.db", uuid::Uuid::new_v4()));
        let store = Store::open(&path).unwrap();
        profiles(&store);
        let project = store.add_project(Path::new("D:/route-test")).unwrap();
        store.select_profile("claude", Some("glm")).unwrap();
        let a = store
            .create_agent_session(&project.id, Some("k3".into()), "claude")
            .unwrap();
        store
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET connection_id=NULL WHERE id=?1",
                [&a.id],
            )
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.session(&a.id).unwrap().connection_id.as_deref(),
            Some("kimi")
        );
        store.use_official("claude").unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.route("claude", Some(&a.id)).unwrap().id, "kimi");
        drop(store);
        std::fs::remove_file(path).unwrap();
    }
}
