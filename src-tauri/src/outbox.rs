//! Durable follow-ups ordered within each chat, dispatched independently across chats.
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
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowupEdit {
    pub row: Followup,
    pub token: String,
}

impl Store {
    fn begin_followup_edit(&self, id: &str) -> Result<FollowupEdit, String> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let (session_id, raw, status, error): (String, String, String, Option<String>) = tx
            .query_row("SELECT session_id,payload,status,error FROM followups WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .optional().map_err(|e| e.to_string())?.ok_or("排队消息已发送或移除")?;
        if !matches!(status.as_str(), "queued" | "paused" | "failed") {
            return Err("这条消息正在发送或已在其他窗口编辑".into());
        }
        let payload = serde_json::from_str(&raw).map_err(|e| format!("排队消息损坏：{e}"))?;
        let token = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO followup_edits(followup_id,token,previous_status) VALUES(?1,?2,?3)", params![id, token, status]).map_err(|e| e.to_string())?;
        tx.execute("UPDATE followups SET status='editing' WHERE id=?1", [id]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(FollowupEdit { row: Followup { id: id.into(), session_id, payload, status, error }, token })
    }
    fn finish_followup_edit(&self, id: &str, token: &str, payload: Option<&Payload>) -> Result<(), String> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let previous: String = tx.query_row("SELECT e.previous_status FROM followup_edits e JOIN followups f ON f.id=e.followup_id WHERE e.followup_id=?1 AND e.token=?2 AND f.status='editing'", params![id, token], |r| r.get(0))
            .optional().map_err(|e| e.to_string())?.ok_or("编辑已失效，原消息仍保留在队列中，请重新打开编辑")?;
        if let Some(payload) = payload {
            // Update the same row: its ID and FIFO position never change.
            tx.execute("UPDATE followups SET payload=?2,status='queued',error=NULL WHERE id=?1", params![id, serde_json::to_string(payload).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        } else {
            tx.execute("UPDATE followups SET status=?2 WHERE id=?1", params![id, previous]).map_err(|e| e.to_string())?;
        }
        tx.execute("DELETE FROM followup_edits WHERE followup_id=?1", [id]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
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
            let _ = c.execute("UPDATE followup_edits SET previous_status='paused' WHERE followup_id IN (SELECT id FROM followups WHERE session_id=?1)", [session]);
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
pub async fn begin_followup_edit(id: String, app: AppHandle) -> Result<FollowupEdit, String> {
    let outbox = app.state::<Outbox>();
    let _gate = outbox.gate.lock().await;
    let edit = app.state::<AppState>().store.begin_followup_edit(&id)?;
    changed(&app);
    Ok(edit)
}
#[tauri::command]
pub async fn finish_followup_edit(id: String, token: String, payload: Option<Payload>, app: AppHandle) -> Result<(), String> {
    let store = &app.state::<AppState>().store;
    if let Some(payload) = &payload {
        let row = store.followup(&id)?;
        let session = store.session(&row.session_id)?;
        validate(payload, &session)?;
        crate::chat_connection::check(&app, &session.agent, Some(&session.id), None).await?;
    }
    let outbox = app.state::<Outbox>();
    let _gate = outbox.gate.lock().await;
    store.finish_followup_edit(&id, &token, payload.as_ref())?;
    changed(&app);
    Ok(())
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
    let session = store.session(&session_id)?;
    validate(&payload, &session)?;
    crate::chat_connection::check(&app, &session.agent, Some(&session_id), None).await?;
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
    if matches!(row.status.as_str(), "sending" | "steering" | "editing") {
        return Err("这条消息正在发送或编辑，请稍候".into());
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
            let session = store.session(&row.session_id)?;
            crate::chat_connection::check(&app, &session.agent, Some(&session.id), None).await?;
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
        None,
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
    if matches!(row.status.as_str(), "sending" | "steering" | "editing") {
        return Err("这条消息正在发送或编辑".into());
    }
    let session = store.session(&row.session_id)?;
    if session.turn_id.as_deref() != Some(&expected_turn_id)
        || !matches!(session.status.as_str(), "running" | "waiting")
    {
        return Err("当前任务已结束或变化，请继续排队发送".into());
    }
    validate(&row.payload, &session)?;
    crate::chat_connection::check(&app, &session.agent, Some(&session.id), None).await?;
    store.update_followup(&id, "steering", None)?;
    drop(_gate);
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
            let client=app.state::<AppState>().runtime.session_client(&session.id).await.ok_or("Agent 连接已关闭")?;
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
            tokio::time::timeout(std::time::Duration::from_secs(20),async{while store.session(&session.id).is_ok_and(|s|matches!(s.status.as_str(),"starting"|"running"|"waiting")) {tokio::time::sleep(std::time::Duration::from_millis(40)).await;}}).await.map_err(|_|"停止任务超时，消息仍保留在队列".to_string())?;
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
            let Ok(Some(id)) = store.next_followup() else {
                break;
            };
            let Ok(row) = store.followup(&id) else {
                break;
            };
            let _ = store.update_followup(&id, "sending", None);
            let _ = app.emit("followups-updated", ());
            let dispatch_app = app.clone();
            drop(_gate);
            tauri::async_runtime::spawn(async move {
            let store = &dispatch_app.state::<AppState>().store;
            match dispatch(&dispatch_app, &row).await {
                Ok(()) => {
                    let _ = store.erase_followup(&id);
                }
                Err(e) => {
                    let _ = store.update_followup(&id, "failed", Some(&e));
                }
            }
            let _ = dispatch_app.emit("followups-updated", ());
            let _ = dispatch_app.emit("workspace-updated", ());
            kick(&dispatch_app);
            });
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
    fn queued_and_starting_sessions_do_not_block_other_conversations() {
        let (store, a) = fixture();
        let b = store.create_agent_session(&a.project_id, None, "codex").unwrap();
        let c = store.create_agent_session(&a.project_id, None, "claude").unwrap();
        store.claim(&a.id).unwrap();
        store.put_followup(&a.id, &payload("只等自己的上一轮")).unwrap();
        let first = store.put_followup(&b.id, &payload("B 立即发送")).unwrap();
        let later = store.put_followup(&b.id, &payload("B 的第二条")).unwrap();
        let other = store.put_followup(&c.id, &payload("C 同时发送")).unwrap();
        assert_eq!(store.next_followup().unwrap(), Some(first.clone()));
        store.update_followup(&first, "sending", None).unwrap();
        assert_eq!(store.next_followup().unwrap(), Some(other.clone()));
        store.claim(&b.id).unwrap();
        store.update_followup(&other, "sending", None).unwrap();
        store.claim(&c.id).unwrap();
        assert_eq!(store.running().unwrap(), 3);
        store.erase_followup(&first).unwrap();
        assert!(store.next_followup().unwrap().is_none());
        store.set_status(&b.id, "idle", None).unwrap();
        assert_eq!(store.next_followup().unwrap(), Some(later));
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
    fn editing_holds_fifo_position_and_saving_replaces_the_complete_payload() {
        let (store, s) = fixture();
        let first = store.put_followup(&s.id, &payload("先发")).unwrap();
        let second = store.put_followup(&s.id, &payload("后发")).unwrap();
        let edit = store.begin_followup_edit(&first).unwrap();
        assert!(store.next_followup().unwrap().is_none());
        let other = store.create_agent_session(&s.project_id, None, "claude").unwrap();
        let independent = store.put_followup(&other.id, &payload("不受编辑影响")).unwrap();
        assert_eq!(store.next_followup().unwrap(), Some(independent));
        let mut replacement = payload("修改后的第一条");
        replacement.effort = Some("high".into());
        replacement.attachments = vec![Attachment { kind: "image".into(), path: "image.png".into(), name: "图片".into() }];
        store.finish_followup_edit(&first, &edit.token, Some(&replacement)).unwrap();
        let rows = store.followups(Some(&s.id)).unwrap();
        assert_eq!((rows[0].id.as_str(), rows[1].id.as_str()), (first.as_str(), second.as_str()));
        assert_eq!(rows[0].payload.text, "修改后的第一条");
        assert_eq!(rows[0].payload.attachments[0].path, "image.png");
        assert_eq!(rows[0].payload.effort.as_deref(), Some("high"));
        assert_eq!(store.next_followup().unwrap(), Some(first));
    }
    #[test]
    fn editing_rejects_stale_tokens_duplicate_editors_and_already_submitted_rows() {
        let (store, s) = fixture();
        let id = store.put_followup(&s.id, &payload("原消息")).unwrap();
        let edit = store.begin_followup_edit(&id).unwrap();
        assert!(store.begin_followup_edit(&id).is_err());
        assert!(store.finish_followup_edit(&id, "another-window", Some(&payload("错误修改"))).is_err());
        assert_eq!(store.followup(&id).unwrap().payload.text, "原消息");
        store.finish_followup_edit(&id, &edit.token, None).unwrap();
        assert!(store.finish_followup_edit(&id, &edit.token, Some(&payload("重复发送"))).is_err());
        store.update_followup(&id, "sending", None).unwrap();
        assert!(store.begin_followup_edit(&id).is_err());
    }
    #[test]
    fn cancelling_restores_only_its_original_state_and_respects_stop_during_edit() {
        let (store, s) = fixture();
        let id = store.put_followup(&s.id, &payload("原消息")).unwrap();
        store.update_followup(&id, "failed", Some("断开")).unwrap();
        let edit = store.begin_followup_edit(&id).unwrap();
        store.finish_followup_edit(&id, &edit.token, None).unwrap();
        assert_eq!(store.followup(&id).unwrap().status, "failed");
        assert_eq!(store.followup(&id).unwrap().error.as_deref(), Some("断开"));
        store.update_followup(&id, "queued", None).unwrap();
        let edit = store.begin_followup_edit(&id).unwrap();
        store.pause_followups(&s.id);
        store.finish_followup_edit(&id, &edit.token, None).unwrap();
        assert_eq!(store.followup(&id).unwrap().status, "paused");
        assert!(store.next_followup().unwrap().is_none());
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
            let editable = store.put_followup(&s.id, &payload("重启后保留编辑中的原文")).unwrap();
            store.begin_followup_edit(&editable).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.followups(None).unwrap()[0].status, "paused");
        assert_eq!(store.followups(None).unwrap()[1].status, "paused");
        assert_eq!(store.followups(None).unwrap()[1].payload.text, "重启后保留编辑中的原文");
        assert_eq!(store.0.lock().unwrap().query_row::<i64, _, _>("SELECT count(*) FROM followup_edits", [], |r| r.get(0)).unwrap(), 0);
        assert!(store.next_followup().unwrap().is_none());
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
