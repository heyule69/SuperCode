//! Completed worker messages wake the controller; deltas and tools never do.
//! The queue contains references to private worker history, not user replies.
use crate::storage::Store;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

struct Notice {
    kind: &'static str,
    key: String,
    message_id: Option<String>,
}
fn notice(event: &Value) -> Option<Notice> {
    let method = event["method"].as_str()?;
    if method == "item/completed" {
        let item = &event["params"]["item"];
        if item["type"] != "agentMessage" || item["text"].as_str()?.trim().is_empty() {
            return None;
        }
        let id = item["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 1024)?;
        return Some(Notice {
            kind: "progress",
            key: id.into(),
            message_id: Some(id.into()),
        });
    }
    if method == "serverRequest/resolved" {
        return None;
    }
    let id = event
        .get("id")
        .filter(|id| id.is_string() || id.is_number())?;
    let key = serde_json::to_string(id)
        .ok()
        .filter(|id| id.len() <= 2048)?;
    Some(Notice {
        kind: if method == "item/tool/requestUserInput" {
            "question"
        } else {
            "approval"
        },
        key,
        message_id: None,
    })
}

pub fn has_update(event: &Value) -> bool {
    notice(event).is_some()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub cursor: i64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Store {
    pub fn record_linkage_update(
        &self,
        parent: &str,
        worker: &str,
        event: &Value,
    ) -> Result<bool, String> {
        let Some(notice) = notice(event) else {
            return Ok(false);
        };
        let Some(task) = self
            .linkage_task(parent)?
            .filter(|task| !task.closed() && task.round > 0)
        else {
            return Ok(false);
        };
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let current: Option<(Option<String>, Option<String>, String)> = conn.query_row(
            "SELECT s.native_id,s.turn_id,s.status FROM sessions s JOIN linkage_configs c ON c.worker_id=s.id WHERE c.controller_id=?1 AND s.id=?2 AND s.archived=0",
            params![parent, worker], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional().map_err(|e| e.to_string())?;
        let Some((native, turn, status)) = current else {
            return Ok(false);
        };
        let owner = event["params"]["appSessionId"].as_str();
        let scoped_request = notice.kind != "progress" && owner == Some(worker);
        if !matches!(status.as_str(), "running" | "waiting")
            || owner.is_some_and(|owner| owner != worker)
            || !scoped_request
                && (event["params"]["threadId"]
                    .as_str()
                    .is_some_and(|id| native.as_deref() != Some(id))
                    || event["params"]["turnId"]
                        .as_str()
                        .is_some_and(|id| turn.as_deref() != Some(id)))
        {
            return Ok(false);
        }
        // The current request/round identity keeps resumed or superseded work
        // from consuming a notification belonging to another user task.
        conn.execute("INSERT INTO linkage_updates(controller_id,worker_id,input_seq,round,kind,event_key,message_id) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(controller_id,input_seq,round,kind,event_key) DO NOTHING",
            params![parent,worker,task.input_seq,task.round,notice.kind,notice.key,notice.message_id]).map(|count|count==1).map_err(|e|e.to_string())
    }
    pub fn linkage_updates(&self, parent: &str) -> Result<(i64, Vec<Update>), String> {
        let Some(task) = self.linkage_task(parent)? else {
            return Ok((0, vec![]));
        };
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let mut query = conn.prepare("SELECT u.seq,u.kind,m.text FROM linkage_updates u JOIN linkage_configs c ON c.controller_id=u.controller_id AND c.worker_id=u.worker_id LEFT JOIN messages m ON m.session_id=u.worker_id AND m.id=u.message_id WHERE u.controller_id=?1 AND u.input_seq=?2 AND u.round=?3 ORDER BY u.seq DESC LIMIT 4").map_err(|e|e.to_string())?;
        let rows = query
            .query_map(params![parent, task.input_seq, task.round], |row| {
                Ok(Update {
                    cursor: row.get(0)?,
                    kind: row.get(1)?,
                    text: row
                        .get::<_, Option<String>>(2)?
                        .map(|text| crate::protocol::bounded(&text, 4096)),
                })
            })
            .map_err(|e| e.to_string())?;
        let mut updates = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let cursor = updates.first().map_or(0, |update| update.cursor);
        updates.reverse();
        Ok((cursor, updates))
    }
    pub fn acknowledge_linkage_updates(&self, parent: &str, cursor: i64) -> Result<(), String> {
        let Some(task) = self.linkage_task(parent)? else {
            return Ok(());
        };
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let latest: i64 = conn.query_row("SELECT COALESCE(MAX(seq),0) FROM linkage_updates WHERE controller_id=?1 AND input_seq=?2 AND round=?3",params![parent,task.input_seq,task.round],|row|row.get(0)).map_err(|e|e.to_string())?;
        if cursor < 0 || cursor > latest {
            return Err("进展游标无效，请先读取 executor_status".into());
        }
        conn.execute("UPDATE linkage_updates SET delivered=1 WHERE controller_id=?1 AND input_seq=?2 AND round=?3 AND seq<=?4",params![parent,task.input_seq,task.round,cursor]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn claim_linkage_update(&self, parent: &str) -> Result<Option<(u64, i64)>, String> {
        let Some(task) = self.linkage_task(parent)?.filter(|task| {
            matches!(task.state.as_str(), "working" | "waiting_user") && task.round > 0
        }) else {
            return Ok(None);
        };
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let idle: bool = tx.query_row("SELECT status='idle' AND archived=0 AND NOT EXISTS(SELECT 1 FROM followups WHERE session_id=?1 AND status IN ('queued','sending','steering')) FROM sessions WHERE id=?1",[parent],|row|row.get(0)).map_err(|e|e.to_string())?;
        if !idle {
            return Ok(None);
        }
        let cursor: Option<i64> = tx.query_row("SELECT MAX(u.seq) FROM linkage_updates u JOIN linkage_configs c ON c.controller_id=u.controller_id AND c.worker_id=u.worker_id WHERE u.controller_id=?1 AND u.input_seq=?2 AND u.round=?3 AND u.delivered=0",params![parent,task.input_seq,task.round],|row|row.get(0)).map_err(|e|e.to_string())?;
        let Some(cursor) = cursor else {
            return Ok(None);
        };
        tx.execute("UPDATE linkage_updates SET delivered=1 WHERE controller_id=?1 AND input_seq=?2 AND round=?3 AND seq<=?4",params![parent,task.input_seq,task.round,cursor]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Some((task.round, cursor)))
    }
    pub fn defer_linkage_update(
        &self,
        parent: &str,
        round: u64,
        cursor: i64,
    ) -> Result<(), String> {
        let Some(task) = self
            .linkage_task(parent)?
            .filter(|task| task.round == round && !task.closed())
        else {
            return Ok(());
        };
        self.0.lock().map_err(|e|e.to_string())?.execute("UPDATE linkage_updates SET delivered=0 WHERE controller_id=?1 AND input_seq=?2 AND round=?3 AND seq=?4",params![parent,task.input_seq,round,cursor]).map_err(|e|e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{linkage_task::Plan, storage::Session};
    use rusqlite::Connection;
    use serde_json::json;
    fn fixture() -> (Store, Session, Session) {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(std::path::Path::new(".")).unwrap();
        let root = store.create_session(&project.id, None).unwrap();
        let worker = store.create_session(&project.id, None).unwrap();
        {
            let conn = store.0.lock().unwrap();
            conn.execute("INSERT INTO linkage_configs(controller_id,configuration,worker_id) VALUES(?1,'{}',?2)",params![root.id,worker.id]).unwrap();
            conn.execute(
                "INSERT INTO linkage_workers(worker_id,controller_id) VALUES(?1,?2)",
                params![worker.id, root.id],
            )
            .unwrap();
        }
        store
            .save_message(
                "human",
                &root.id,
                "user",
                "完成全部两个阶段",
                "userMessage",
                &json!({}),
            )
            .unwrap();
        store
            .plan_linkage_task(
                &root.id,
                Plan {
                    objective: "完整目标".into(),
                    steps: vec!["第一阶段".into(), "第二阶段".into()],
                },
            )
            .unwrap();
        store.begin_linkage_work(&root.id, "完成所有阶段").unwrap();
        store.bind_native(&worker.id, "worker-native").unwrap();
        store
            .set_status(&worker.id, "running", Some("worker-turn"))
            .unwrap();
        (store, root, worker)
    }
    fn message(id: &str, text: &str) -> Value {
        json!({"method":"item/completed","params":{"threadId":"worker-native","turnId":"worker-turn","item":{"type":"agentMessage","id":id,"text":text}}})
    }
    fn progress(store: &Store, root: &Session, worker: &Session, id: &str) -> i64 {
        let text = format!("WORKER_RAW_{id}: 第一阶段文件已保存；第二阶段仍在执行。");
        store
            .save_message(
                id,
                &worker.id,
                "assistant",
                &text,
                "agentMessage",
                &json!({"status":"completed"}),
            )
            .unwrap();
        assert!(store
            .record_linkage_update(&root.id, &worker.id, &message(id, &text))
            .unwrap());
        store.linkage_updates(&root.id).unwrap().0
    }
    #[test]
    fn only_complete_messages_notify_not_deltas_reasoning_or_tool_logs() {
        let (store, root, worker) = fixture();
        let base = message("one", "已完成第一步");
        for method in [
            "item/started",
            "item/updated",
            "item/agentMessage/delta",
            "thread/tokenUsage/updated",
        ] {
            let mut event = base.clone();
            event["method"] = json!(method);
            assert!(!store
                .record_linkage_update(&root.id, &worker.id, &event)
                .unwrap());
        }
        for kind in [
            "reasoning",
            "commandExecution",
            "claudeToolCall",
            "fileChange",
        ] {
            let mut event = base.clone();
            event["params"]["item"]["type"] = json!(kind);
            assert!(!store
                .record_linkage_update(&root.id, &worker.id, &event)
                .unwrap());
        }
        assert!(!store
            .record_linkage_update(&root.id, &worker.id, &message("blank", "  "))
            .unwrap());
        let cursor = progress(&store, &root, &worker, "one");
        assert!(!store
            .record_linkage_update(&root.id, &worker.id, &message("one", "已完成第一步"))
            .unwrap());
        assert_eq!(store.linkage_updates(&root.id).unwrap().0, cursor);
        let task = store.linkage_task(&root.id).unwrap().unwrap();
        assert_eq!(task.state, "working");
        assert!(task.steps.iter().all(|step| !step.done));
    }
    #[test]
    fn a_controller_reply_can_finish_while_the_worker_continues_and_each_update_is_consumed_once() {
        let (store, root, worker) = fixture();
        let first = progress(&store, &root, &worker, "first");
        assert_eq!(
            store.claim_linkage_update(&root.id).unwrap(),
            Some((1, first))
        );
        assert_eq!(store.session(&worker.id).unwrap().status, "running");
        assert_eq!(store.claim_linkage_update(&root.id).unwrap(), None);
        store
            .set_status(&root.id, "running", Some("reply"))
            .unwrap();
        let second = progress(&store, &root, &worker, "second");
        let latest = progress(&store, &root, &worker, "third");
        assert!(latest > second);
        assert_eq!(store.claim_linkage_update(&root.id).unwrap(), None);
        store.set_status(&root.id, "idle", None).unwrap();
        assert_eq!(
            store.claim_linkage_update(&root.id).unwrap(),
            Some((1, latest))
        );
        assert_eq!(store.claim_linkage_update(&root.id).unwrap(), None);
        assert!(!store.linkage_completion_ready(&root.id).unwrap());
    }
    #[test]
    fn handoff_acknowledges_only_the_observed_cursor_and_preserves_a_concurrent_new_message() {
        let (store, root, worker) = fixture();
        let first = progress(&store, &root, &worker, "first");
        let next = progress(&store, &root, &worker, "next");
        store.acknowledge_linkage_updates(&root.id, first).unwrap();
        assert_eq!(
            store.claim_linkage_update(&root.id).unwrap(),
            Some((1, next))
        );
        assert!(store
            .acknowledge_linkage_updates(&root.id, next + 100)
            .is_err());
        assert!(store.acknowledge_linkage_updates(&root.id, -1).is_err());
        for _ in 0..50 {
            assert_eq!(store.claim_linkage_update(&root.id).unwrap(), None);
        }
    }
    #[test]
    fn execution_end_takes_priority_over_progress_and_requires_whole_task_acceptance() {
        let (store, root, worker) = fixture();
        progress(&store, &root, &worker, "final");
        store.set_status(&worker.id, "idle", None).unwrap();
        store.finish_linkage_work(&root.id, "worker-turn").unwrap();
        assert_eq!(store.claim_linkage_update(&root.id).unwrap(), None);
        assert_eq!(store.claim_linkage_review(&root.id).unwrap(), Some(1));
        assert_eq!(store.claim_linkage_review(&root.id).unwrap(), None);
        assert_eq!(
            store.linkage_task(&root.id).unwrap().unwrap().state,
            "needs_review"
        );
    }
    #[test]
    fn queued_user_input_takes_priority_and_user_stop_prevents_late_progress_from_restarting_work()
    {
        let (store, root, worker) = fixture();
        let cursor = progress(&store, &root, &worker, "first");
        store.0.lock().unwrap().execute("INSERT INTO followups(id,session_id,payload,status) VALUES('human-input',?1,'{}','queued')",[&root.id]).unwrap();
        assert_eq!(store.claim_linkage_update(&root.id).unwrap(), None);
        store
            .0
            .lock()
            .unwrap()
            .execute("DELETE FROM followups WHERE id='human-input'", [])
            .unwrap();
        assert_eq!(
            store.claim_linkage_update(&root.id).unwrap(),
            Some((1, cursor))
        );
        store.cancel_linkage_task(&root.id).unwrap();
        assert!(!store
            .record_linkage_update(&root.id, &worker.id, &message("late", "迟到的完成说明"))
            .unwrap());
        assert_eq!(store.claim_linkage_update(&root.id).unwrap(), None);
        assert!(!store.linkage_completion_ready(&root.id).unwrap());
    }
    #[test]
    fn pending_questions_and_approvals_notify_once_without_answering_or_approving_them() {
        let (store, root, worker) = fixture();
        store
            .set_status(&worker.id, "waiting", Some("worker-turn"))
            .unwrap();
        let question = json!({"method":"item/tool/requestUserInput","id":"question","params":{"threadId":"worker-native","turnId":"worker-turn"}});
        let approval = json!({"method":"item/commandExecution/requestApproval","id":17,"params":{"threadId":"worker-native","turnId":"worker-turn"}});
        for event in [&question, &approval] {
            assert!(store
                .record_linkage_update(&root.id, &worker.id, event)
                .unwrap());
            assert!(!store
                .record_linkage_update(&root.id, &worker.id, event)
                .unwrap());
        }
        let (_, updates) = store.linkage_updates(&root.id).unwrap();
        assert_eq!(
            updates
                .iter()
                .map(|update| update.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["question", "approval"]
        );
        assert!(updates.iter().all(|update| update.text.is_none()));
        assert!(store.claim_linkage_update(&root.id).unwrap().is_some());
        assert_eq!(store.session(&worker.id).unwrap().status, "waiting");
        assert!(!store
            .record_linkage_update(
                &root.id,
                &worker.id,
                &json!({"method":"serverRequest/resolved","id":17})
            )
            .unwrap());
    }
    #[test]
    fn progress_is_private_data_not_a_forged_user_message_or_an_automatic_user_reply() {
        let (store, root, worker) = fixture();
        progress(&store, &root, &worker, "private");
        store
            .save_chat_input(
                "wake",
                &root.id,
                "内部通知",
                &json!({"linkageProgress":{"round":1,"cursor":1}}),
            )
            .unwrap();
        let root_messages = store.messages(&root.id, None).unwrap();
        assert_eq!(
            root_messages
                .iter()
                .filter(|message| message.role == "user")
                .count(),
            1
        );
        assert!(!root_messages
            .iter()
            .any(|message| message.text.contains("WORKER_RAW")));
        assert!(!root_messages
            .iter()
            .any(|message| message.role == "assistant"));
        assert_eq!(root_messages.last().unwrap().kind, "linkageProgress");
        assert!(store.linkage_updates(&root.id).unwrap().1[0]
            .text
            .as_ref()
            .unwrap()
            .contains("WORKER_RAW"));
    }
    #[test]
    fn a_controller_service_failure_stays_visible_when_the_worker_later_finishes() {
        let (store, root, worker) = fixture();
        progress(&store, &root, &worker, "first");
        assert!(store.claim_linkage_update(&root.id).unwrap().is_some());
        store
            .linkage_review_error(&root.id, 1, "主控服务无法连接，尚未整理进展")
            .unwrap();
        store.set_status(&root.id, "failed", None).unwrap();
        store.set_status(&worker.id, "idle", None).unwrap();
        store.finish_linkage_work(&root.id, "worker-turn").unwrap();
        let task = store.linkage_task(&root.id).unwrap().unwrap();
        assert_eq!(task.state, "needs_review");
        assert!(task.review_error.unwrap().contains("主控服务无法连接"));
        assert_eq!(store.claim_linkage_update(&root.id).unwrap(), None);
        assert_eq!(store.claim_linkage_review(&root.id).unwrap(), None);
    }
    #[test]
    fn old_native_turns_and_child_threads_cannot_notify_for_the_current_assignment() {
        let (store, root, worker) = fixture();
        for field in ["threadId", "turnId"] {
            let mut event = message("old", "过期内容");
            event["params"][field] = json!("another");
            assert!(!store
                .record_linkage_update(&root.id, &worker.id, &event)
                .unwrap());
        }
        assert_eq!(store.linkage_updates(&root.id).unwrap().0, 0);
    }
    #[test]
    fn explicit_request_owner_survives_native_boot_aliases_but_never_borrows_another_chat() {
        let (store, root, worker) = fixture();
        let mut approval = json!({"id":"approval","method":"item/commandExecution/requestApproval","params":{"appSessionId":worker.id,"threadId":"boot-alias","turnId":"boot-turn"}});
        assert!(store
            .record_linkage_update(&root.id, &worker.id, &approval)
            .unwrap());
        approval["id"] = json!("other-request");
        approval["params"]["appSessionId"] = json!("other-chat");
        approval["params"]["threadId"] = json!("worker-native");
        assert!(!store
            .record_linkage_update(&root.id, &worker.id, &approval)
            .unwrap());
    }
    #[test]
    fn notifications_survive_storage_reopen_and_only_a_superseding_input_can_defer_delivery() {
        let (store, root, worker) = fixture();
        let cursor = progress(&store, &root, &worker, "durable");
        let restored = Store::from_connection(store.0.into_inner().unwrap()).unwrap();
        assert_eq!(
            restored.claim_linkage_update(&root.id).unwrap(),
            Some((1, cursor))
        );
        assert_eq!(restored.claim_linkage_update(&root.id).unwrap(), None);
        restored.defer_linkage_update(&root.id, 1, cursor).unwrap();
        assert_eq!(
            restored.claim_linkage_update(&root.id).unwrap(),
            Some((1, cursor))
        );
        assert_eq!(restored.claim_linkage_update(&root.id).unwrap(), None);
    }
}
