//! Durable follow-ups. A single dispatcher shares the normal agent/approval path.
use crate::{
    client_features::Attachment,
    storage::{Session, Store},
    AppState,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, Notify};

#[derive(Default)]
pub struct Outbox {
    gate: Mutex<()>,
    wake: Notify,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    pub text: String,
    pub model: Option<String>,
    pub read_only: bool,
    pub permission_mode: Option<String>,
    pub attachments: Vec<Attachment>,
    pub effort: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Followup {
    pub id: String,
    pub session_id: String,
    pub payload: Payload,
    pub status: String,
    pub error: Option<String>,
}

impl Store {
    pub fn followups(&self, session: Option<&str>) -> Result<Vec<Followup>, String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let mut q=conn.prepare("SELECT id,session_id,payload,status,error FROM followups WHERE (?1 IS NULL OR session_id=?1) ORDER BY seq LIMIT 200").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([session], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.map(|r| {
            let (id, session_id, raw, status, error) = r.map_err(|e| e.to_string())?;
            Ok(Followup {
                id,
                session_id,
                payload: serde_json::from_str(&raw).map_err(|e| format!("排队消息损坏：{e}"))?,
                status,
                error,
            })
        })
        .collect()
    }
    fn followup(&self, id: &str) -> Result<Followup, String> {
        self.followups(None)?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or("排队消息已发送或移除".into())
    }
    fn put_followup(&self, session: &str, payload: &Payload) -> Result<String, String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let count:i64=conn.query_row("SELECT count(*) FROM followups WHERE session_id=?1 OR (SELECT count(*) FROM followups)>=200",[session],|r|r.get(0)).map_err(|e|e.to_string())?;
        if count >= 20 {
            return Err("排队消息过多，请先发送或移除一些消息".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute("INSERT INTO followups(id,session_id,payload,status) SELECT ?1,?2,?3,'queued' WHERE EXISTS(SELECT 1 FROM sessions WHERE id=?2 AND archived=0)",params![id,session,serde_json::to_string(payload).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
        Ok(id)
    }
    fn update_followup(&self, id: &str, status: &str, error: Option<&str>) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .execute(
                "UPDATE followups SET status=?2,error=?3 WHERE id=?1",
                params![id, status, error],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    fn erase_followup(&self, id: &str) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .execute("DELETE FROM followups WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn pause_followups(&self, session: &str) {
        if let Ok(c) = self.0.lock() {
            let _ = c.execute(
                "UPDATE followups SET status='paused' WHERE session_id=?1 AND status='queued'",
                [session],
            );
        }
    }
    fn next_followup(&self) -> Result<Option<String>, String> {
        self.0.lock().map_err(|e|e.to_string())?.query_row("SELECT f.id FROM followups f JOIN sessions s ON s.id=f.session_id WHERE f.status='queued' AND s.archived=0 AND s.status NOT IN ('starting','running','waiting') AND NOT EXISTS(SELECT 1 FROM followups p WHERE p.session_id=f.session_id AND p.seq<f.seq) ORDER BY f.seq LIMIT 1",[],|r|r.get(0)).optional().map_err(|e|e.to_string())
    }
}
fn changed(app: &AppHandle) {
    let _ = app.emit("followups-updated", ());
    app.state::<Outbox>().wake.notify_one();
}
pub(crate) fn native_consumed(app: &AppHandle, id: &str) {
    let _ = app.state::<AppState>().store.erase_followup(id);
    changed(app);
}
pub(crate) fn native_failed(app: &AppHandle, id: &str, error: &str) {
    let _ = app
        .state::<AppState>()
        .store
        .update_followup(id, "paused", Some(error));
    changed(app);
}
pub fn kick(app: &AppHandle) {
    app.state::<Outbox>().wake.notify_one();
}
fn validate(payload: &Payload, session: &Session) -> Result<(), String> {
    if payload.text.trim().is_empty() || payload.text.len() > 128 * 1024 {
        return Err("消息不能为空或超过 128 KB".into());
    }
    crate::client_features::validate_permission(
        &session.agent,
        payload.permission_mode.as_deref().unwrap_or("ask"),
    )?;
    crate::client_features::prepare_input(&payload.text, &payload.attachments)?;
    if session.native_id.is_some()
        && payload
            .model
            .as_deref()
            .is_some_and(|m| Some(m) != session.model.as_deref())
    {
        return Err("排队消息的模型必须与当前会话一致".into());
    }
    Ok(())
}
#[tauri::command]
pub fn list_followups(session_id: String, app: AppHandle) -> Result<Vec<Followup>, String> {
    app.state::<AppState>().store.followups(Some(&session_id))
}
#[tauri::command]
pub async fn followup_capabilities(
    session_id: String,
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    let session = app.state::<AppState>().store.session(&session_id)?;
    let mode = if session.agent == "opencode" {
        app.state::<AppState>()
            .native
            .steering_mode(&session.id)
            .await
    } else {
        "native"
    };
    Ok(json!({"steeringMode":mode,"queue":"client","editableUntilSubmitted":true}))
}
#[tauri::command]
pub async fn enqueue_followup(
    session_id: String,
    payload: Payload,
    app: AppHandle,
) -> Result<String, String> {
    crate::desktop_lifecycle::ensure_running(&app)?;
    // Inserts are serialized by SQLite. Do not wait for a slow agent startup
    // just to save another follow-up in the input box.
    let store = &app.state::<AppState>().store;
    validate(&payload, &store.session(&session_id)?)?;
    let id = store.put_followup(&session_id, &payload)?;
    changed(&app);
    Ok(id)
}
#[tauri::command]
pub async fn change_followup(
    id: String,
    action: String,
    text: Option<String>,
    app: AppHandle,
) -> Result<(), String> {
    let outbox = app.state::<Outbox>();
    let _gate = outbox.gate.lock().await;
    let store = &app.state::<AppState>().store;
    let row = store.followup(&id)?;
    if matches!(row.status.as_str(), "sending" | "steering") {
        return Err("这条消息正在发送，请稍候".into());
    }
    match action.as_str() {
        "remove" => store.erase_followup(&id)?,
        "edit" => {
            let mut payload = row.payload;
            payload.text = text.ok_or("请输入消息")?;
            validate(&payload, &store.session(&row.session_id)?)?;
            store
                .0
                .lock()
                .map_err(|e| e.to_string())?
                .execute(
                    "UPDATE followups SET payload=?2,error=NULL WHERE id=?1",
                    params![
                        id,
                        serde_json::to_string(&payload).map_err(|e| e.to_string())?
                    ],
                )
                .map_err(|e| e.to_string())?;
        }
        "pause" => store.update_followup(&id, "paused", None)?,
        "resume" => {
            store.0.lock().map_err(|e|e.to_string())?.execute("UPDATE followups SET status='queued',error=NULL WHERE session_id=?1 AND status IN ('paused','failed')",[row.session_id]).map_err(|e|e.to_string())?;
        }
        _ => return Err("未知的排队操作".into()),
    };
    changed(&app);
    Ok(())
}
async fn dispatch(app: &AppHandle, row: &Followup) -> Result<(), String> {
    let p = &row.payload;
    crate::commands::send_chat_message_inner(
        row.session_id.clone(),
        p.text.clone(),
        p.model.clone(),
        p.read_only,
        p.permission_mode.clone(),
        Some(p.attachments.clone()),
        p.effort.clone(),
        Some(row.id.clone()),
        app.clone(),
    )
    .await
    .map(|_| ())
}
#[tauri::command]
pub async fn steer_followup(
    id: String,
    expected_turn_id: String,
    app: AppHandle,
) -> Result<(), String> {
    let outbox = app.state::<Outbox>();
    let _gate = outbox.gate.lock().await;
    let store = &app.state::<AppState>().store;
    let row = store.followup(&id)?;
    if matches!(row.status.as_str(), "sending" | "steering") {
        return Err("这条消息正在发送".into());
    }
    let session = store.session(&row.session_id)?;
    if session.turn_id.as_deref() != Some(&expected_turn_id)
        || !matches!(session.status.as_str(), "running" | "waiting")
    {
        return Err("当前任务已结束或变化，请继续排队发送".into());
    }
    validate(&row.payload, &session)?;
    store.update_followup(&id, "steering", None)?;
    let native_mode = session.agent == "claude"
        || session.agent == "pi"
        || (session.agent == "opencode"
            && app
                .state::<AppState>()
                .native
                .steering_mode(&session.id)
                .await
                == "native");
    let result=async {
        let p=crate::client_features::prepare_input(&row.payload.text,&row.payload.attachments)?;
        let permission=row.payload.permission_mode.as_deref().unwrap_or(if row.payload.read_only {"read"}else{"ask"});
        if session.agent=="codex" {
            let client=app.state::<AppState>().runtime.client.lock().await.clone().ok_or("Agent 连接已关闭")?;
            {
                let settings=client.turn_settings.lock().await;
                let (_,_,signature)=settings.as_ref().filter(|(native,turn,_)|Some(native.as_str())==session.native_id.as_deref()&&turn==&expected_turn_id).ok_or("当前任务配置尚未就绪或已变化，请保留排队发送")?;
                crate::client_features::validate_steering_settings(signature,permission,row.payload.effort.as_deref())?;
            }
            let input=if p.codex.iter().any(|v|v["type"]=="skill") {let catalog=client.request("skills/list",json!({"cwds":[store.session_workspace(&session)?.path],"forceReload":false})).await?;crate::client_features::resolve_codex_skills(&p,&catalog)}else{p.codex};
            client.request("turn/steer",json!({"threadId":session.native_id,"expectedTurnId":expected_turn_id,"input":input})).await?;
        } else if session.agent=="claude" {
            app.state::<AppState>().claude.steer(&session.id,&expected_turn_id,&id,p.claude,permission,row.payload.effort.as_deref()).await?;
        } else if native_mode {
            app.state::<AppState>().native.steer(&app,&session.id,&expected_turn_id,&id,p.claude,permission,row.payload.effort.as_deref()).await?;
        } else {
            crate::commands::interrupt_turn(session.id.clone(),app.clone()).await?;
            tokio::time::timeout(std::time::Duration::from_secs(20),async{while store.running().unwrap_or(1)>0 {tokio::time::sleep(std::time::Duration::from_millis(40)).await;}}).await.map_err(|_|"停止任务超时，消息仍保留在队列".to_string())?;
            return dispatch(&app,&row).await;
        }
        let mut metadata=p.metadata;metadata["followupId"]=json!(id);metadata["steered"]=json!(true);
        store.save_message(&format!("user-{id}"),&row.session_id,"user",&row.payload.text,"userMessage",&metadata)?;
        Ok::<(),String>(())
    }.await;
    match &result {
        // Native submissions are retained until the CLI replay/response confirms consumption.
        Ok(()) if !native_mode => store.erase_followup(&id)?,
        Ok(()) => {}
        Err(e) => store.update_followup(&id, "failed", Some(e))?,
    };
    changed(&app);
    let _ = app.emit("workspace-updated", ());
    result
}
pub async fn watch(app: AppHandle) {
    loop {
        app.state::<Outbox>().wake.notified().await;
        loop {
            let outbox = app.state::<Outbox>();
            let _gate = outbox.gate.lock().await;
            let store = &app.state::<AppState>().store;
            if store.running().unwrap_or(1) > 0 {
                break;
            }
            let Ok(Some(id)) = store.next_followup() else {
                break;
            };
            let Ok(row) = store.followup(&id) else {
                break;
            };
            let _ = store.update_followup(&id, "sending", None);
            let _ = app.emit("followups-updated", ());
            match dispatch(&app, &row).await {
                Ok(()) => {
                    let _ = store.erase_followup(&id);
                }
                Err(e) => {
                    let _ = store.update_followup(&id, "failed", Some(&e));
                }
            }
            let _ = app.emit("followups-updated", ());
            let _ = app.emit("workspace-updated", ());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Store, Session) {
        let store =
            Store::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let p = store.add_project(std::path::Path::new(".")).unwrap();
        let s = store.create_session(&p.id, None).unwrap();
        (store, s)
    }
    fn payload(text: &str) -> Payload {
        Payload {
            text: text.into(),
            model: None,
            read_only: false,
            permission_mode: Some("ask".into()),
            attachments: vec![],
            effort: None,
        }
    }
    #[test]
    fn fifo_pausing_and_removal_preserve_unsent_messages() {
        let (store, s) = fixture();
        let a = store.put_followup(&s.id, &payload("第一条")).unwrap();
        let b = store.put_followup(&s.id, &payload("第二条")).unwrap();
        assert_eq!(store.next_followup().unwrap(), Some(a.clone()));
        store.pause_followups(&s.id);
        assert!(store.next_followup().unwrap().is_none());
        store.update_followup(&b, "queued", None).unwrap();
        assert!(store.next_followup().unwrap().is_none());
        store.erase_followup(&a).unwrap();
        assert_eq!(store.next_followup().unwrap(), Some(b));
    }
    #[test]
    fn active_session_cannot_dispatch_and_queue_is_bounded() {
        let (store, s) = fixture();
        for _ in 0..20 {
            store.put_followup(&s.id, &payload("文字")).unwrap();
        }
        assert!(store.put_followup(&s.id, &payload("溢出")).is_err());
        store.claim(&s.id).unwrap();
        assert!(store.next_followup().unwrap().is_none());
    }
    #[test]
    fn failure_blocks_later_rows_and_explicit_resume_allows_an_interrupted_session() {
        let (store, s) = fixture();
        let first = store.put_followup(&s.id, &payload("先发")).unwrap();
        let second = store.put_followup(&s.id, &payload("后发")).unwrap();
        store
            .update_followup(&first, "failed", Some("网络断开"))
            .unwrap();
        assert!(store.next_followup().unwrap().is_none());
        store.set_status(&s.id, "interrupted", None).unwrap();
        store.update_followup(&first, "queued", None).unwrap();
        assert_eq!(store.next_followup().unwrap(), Some(first.clone()));
        store.erase_followup(&first).unwrap();
        assert_eq!(store.next_followup().unwrap(), Some(second));
    }
    #[test]
    fn restart_pauses_pending_rows() {
        let path =
            std::env::temp_dir().join(format!("supercode-outbox-{}.db", uuid::Uuid::new_v4()));
        {
            let store = Store::open(&path).unwrap();
            let p = store.add_project(std::path::Path::new(".")).unwrap();
            let s = store.create_session(&p.id, None).unwrap();
            let id = store
                .put_followup(&s.id, &payload("恢复后不要重复发送"))
                .unwrap();
            store.update_followup(&id, "sending", None).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.followups(None).unwrap()[0].status, "paused");
        assert!(store.next_followup().unwrap().is_none());
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
