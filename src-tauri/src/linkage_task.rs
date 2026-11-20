//! The controller owns acceptance of the whole request, independently of turns.
use crate::storage::Store;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Plan {
    pub objective: String,
    pub steps: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Session;
    use serde_json::json;
    fn fixture() -> (Store, Session, Session) {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store.add_project(std::path::Path::new(".")).unwrap();
        let root = store.create_session(&project.id, None).unwrap();
        let worker = store.create_session(&project.id, None).unwrap();
        store
            .0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO linkage_workers(worker_id,controller_id) VALUES(?1,?2)",
                params![worker.id, root.id],
            )
            .unwrap();
        store
            .save_message(
                "request",
                &root.id,
                "user",
                "按计划完成三个步骤",
                "userMessage",
                &json!({}),
            )
            .unwrap();
        (store, root, worker)
    }
    fn plan() -> Plan {
        Plan {
            objective: "完整实现并验证".into(),
            steps: vec!["实现第一步".into(), "实现第二步".into(), "运行验证".into()],
        }
    }
    fn checks(done: usize) -> Vec<Check> {
        (1..=3)
            .map(|step| Check {
                step,
                done: step <= done,
                evidence: if step <= done {
                    format!("已核对步骤 {step} 的文件和验证输出")
                } else {
                    format!("步骤 {step} 尚未执行，仍需完成")
                },
            })
            .collect()
    }
    fn review(round: u64, outcome: &str, done: usize) -> Review {
        Review {
            round,
            outcome: outcome.into(),
            checks: checks(done),
            next_task: (outcome == "continue").then(|| "完成第二步与验证，报告输出".into()),
            reason: None,
            attempts: vec![],
        }
    }
    fn ended(store: &Store, root: &Session, turn: &str) {
        store.finish_linkage_work(&root.id, turn).unwrap();
    }
    fn queued(store: &Store, session: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        store
            .0
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO followups(id,session_id,payload,status) VALUES(?1,?2,'{}','queued')",
                params![id, session],
            )
            .unwrap();
        id
    }
    fn dequeue(store: &Store, id: &str) {
        store
            .0
            .lock()
            .unwrap()
            .execute("DELETE FROM followups WHERE id=?1", [id])
            .unwrap();
    }

    #[test]
    fn a_worker_stopping_after_step_one_requires_review_and_dispatch_of_the_remaining_steps() {
        let (store, root, _) = fixture();
        store.plan_linkage_task(&root.id, plan()).unwrap();
        store.begin_linkage_work(&root.id, "先完成第一步").unwrap();
        ended(&store, &root, "turn-one");
        let first = store.linkage_task(&root.id).unwrap().unwrap();
        assert_eq!(first.state, "needs_review");
        assert!(first.live());
        assert!(!store.linkage_completion_ready(&root.id).unwrap());
        assert!(store
            .review_linkage_task(&root.id, review(1, "completed", 1))
            .is_err());
        assert!(store.begin_linkage_work(&root.id, "直接跳过验收").is_err());
        let partial = store
            .review_linkage_task(&root.id, review(1, "continue", 1))
            .unwrap();
        assert_eq!(partial.steps.iter().filter(|step| step.done).count(), 1);
        let next = partial.next_task.unwrap();
        let second = store.begin_linkage_work(&root.id, &next).unwrap();
        assert_eq!(second.round, 2);
        assert!(second.steps[0].done);
        assert!(!second.steps[1].done);
        ended(&store, &root, "turn-two");
        let accepted = store
            .review_linkage_task(&root.id, review(2, "completed", 3))
            .unwrap();
        assert_eq!(accepted.state, "completed");
        assert!(!accepted.live());
        assert!(store.linkage_completion_ready(&root.id).unwrap());
        assert!(store.claim_linkage_review(&root.id).unwrap().is_none());
    }
    #[test]
    fn acceptance_receipt_does_not_feed_private_worker_evidence_back_as_the_final_reply() {
        let (store, root, _) = fixture();
        store.plan_linkage_task(&root.id, plan()).unwrap();
        store.begin_linkage_work(&root.id, "完成整份任务").unwrap();
        ended(&store, &root, "one");
        let mut result = review(1, "completed", 3);
        result.checks[0].evidence = "WORKER_ONLY_PRIVATE：真实文件与测试已核对".into();
        let task = store.review_linkage_task(&root.id, result).unwrap();
        let receipt = task.review_receipt();
        assert_eq!(receipt["task"]["state"], "completed");
        assert_eq!(receipt["task"]["verified"], 3);
        assert!(!receipt.to_string().contains("WORKER_ONLY_PRIVATE"));
        assert!(task.steps[0].evidence.contains("WORKER_ONLY_PRIVATE"));
    }
    #[test]
    fn review_requires_all_unique_items_and_concrete_evidence() {
        let (store, root, _) = fixture();
        store.plan_linkage_task(&root.id, plan()).unwrap();
        store.begin_linkage_work(&root.id, "完成三项").unwrap();
        ended(&store, &root, "one");
        let mut incomplete = review(1, "completed", 3);
        incomplete.checks.pop();
        assert!(store.review_linkage_task(&root.id, incomplete).is_err());
        let mut duplicate = review(1, "completed", 3);
        duplicate.checks[2].step = 1;
        assert!(store.review_linkage_task(&root.id, duplicate).is_err());
        let mut empty = review(1, "completed", 3);
        empty.checks[1].evidence = " ".into();
        assert!(store.review_linkage_task(&root.id, empty).is_err());
        assert!(store
            .review_linkage_task(&root.id, review(0, "completed", 3))
            .is_err());
        assert!(store
            .linkage_task(&root.id)
            .unwrap()
            .unwrap()
            .steps
            .iter()
            .all(|step| !step.done));
    }
    #[test]
    fn stopping_is_not_a_blocker_and_blocked_needs_actual_attempts_and_remaining_items() {
        let (store, root, _) = fixture();
        store.plan_linkage_task(&root.id, plan()).unwrap();
        store.begin_linkage_work(&root.id, "完成三项").unwrap();
        ended(&store, &root, "one");
        let mut blocked = review(1, "blocked", 1);
        assert!(store.review_linkage_task(&root.id, blocked).is_err());
        blocked = review(1, "blocked", 1);
        blocked.reason = Some("缺少测试系统账号，无法验证登录后的保存行为".into());
        assert!(store.review_linkage_task(&root.id, blocked).is_err());
        blocked = review(1, "blocked", 1);
        blocked.reason = Some("缺少测试系统账号，无法验证登录后的保存行为".into());
        blocked.attempts =
            vec!["检查项目文档、现有环境配置和本地测试凭据，未找到可用测试账号".into()];
        let task = store.review_linkage_task(&root.id, blocked).unwrap();
        assert_eq!(task.state, "blocked");
        assert_eq!(task.steps.iter().filter(|step| !step.done).count(), 2);
        assert!(store.plan_linkage_task(&root.id, plan()).is_err());
    }
    #[test]
    fn each_ended_round_schedules_at_most_one_controller_acceptance_not_polling() {
        let (store, root, _) = fixture();
        store.plan_linkage_task(&root.id, plan()).unwrap();
        store.begin_linkage_work(&root.id, "完成第一步").unwrap();
        ended(&store, &root, "one");
        assert_eq!(store.claim_linkage_review(&root.id).unwrap(), Some(1));
        assert_eq!(store.claim_linkage_review(&root.id).unwrap(), None);
        store
            .linkage_review_error(&root.id, 1, "主控 API 暂不可用")
            .unwrap();
        assert_eq!(store.claim_linkage_review(&root.id).unwrap(), None);
        assert_eq!(
            store.linkage_task(&root.id).unwrap().unwrap().state,
            "needs_review"
        );
        let next = store
            .review_linkage_task(&root.id, review(1, "continue", 1))
            .unwrap()
            .next_task
            .unwrap();
        store.begin_linkage_work(&root.id, &next).unwrap();
        ended(&store, &root, "two");
        assert_eq!(store.claim_linkage_review(&root.id).unwrap(), Some(2));
    }
    #[test]
    fn user_stop_cancels_late_completion_and_old_review_requests() {
        let (store, root, _) = fixture();
        store.plan_linkage_task(&root.id, plan()).unwrap();
        store.begin_linkage_work(&root.id, "完成任务").unwrap();
        store.cancel_linkage_task(&root.id).unwrap();
        ended(&store, &root, "late");
        assert_eq!(
            store.linkage_task(&root.id).unwrap().unwrap().state,
            "cancelled"
        );
        assert!(store.claim_linkage_review(&root.id).unwrap().is_none());
        assert!(store
            .review_linkage_task(&root.id, review(1, "completed", 3))
            .is_err());
        assert!(store.begin_linkage_work(&root.id, "继续").is_err());
    }
    #[test]
    fn active_children_and_queued_work_cannot_be_accepted_as_finished() {
        let (store, root, worker) = fixture();
        store.plan_linkage_task(&root.id, plan()).unwrap();
        store.begin_linkage_work(&root.id, "完成任务").unwrap();
        store
            .set_status(&worker.id, "running", Some("one"))
            .unwrap();
        ended(&store, &root, "one");
        assert_eq!(
            store.linkage_task(&root.id).unwrap().unwrap().state,
            "working"
        );
        assert!(store
            .review_linkage_task(&root.id, review(1, "completed", 3))
            .is_err());
        store.set_status(&worker.id, "idle", None).unwrap();
        let queued = queued(&store, &worker.id);
        ended(&store, &root, "one");
        assert_eq!(
            store.linkage_task(&root.id).unwrap().unwrap().state,
            "working"
        );
        dequeue(&store, &queued);
        ended(&store, &root, "last");
        assert_eq!(
            store.linkage_task(&root.id).unwrap().unwrap().state,
            "needs_review"
        );
    }
    #[test]
    fn queued_user_input_takes_priority_without_consuming_the_review_round() {
        let (store, root, _) = fixture();
        store.plan_linkage_task(&root.id, plan()).unwrap();
        store.begin_linkage_work(&root.id, "完成第一步").unwrap();
        ended(&store, &root, "one");
        let pending = queued(&store, &root.id);
        assert_eq!(store.claim_linkage_review(&root.id).unwrap(), None);
        dequeue(&store, &pending);
        assert_eq!(store.claim_linkage_review(&root.id).unwrap(), Some(1));
    }
    #[test]
    fn internal_acceptance_does_not_forge_user_messages_or_shrink_the_original_goal() {
        let (store, root, _) = fixture();
        store.plan_linkage_task(&root.id, plan()).unwrap();
        store
            .save_chat_input(
                "internal",
                &root.id,
                "请验收剩余任务",
                &json!({"linkageReview":{"round":1}}),
            )
            .unwrap();
        let messages = store.messages(&root.id, None).unwrap();
        assert_eq!(messages.iter().filter(|m| m.role == "user").count(), 1);
        assert_eq!(messages.last().unwrap().kind, "linkageReview");
        let mut smaller = plan();
        smaller.steps.truncate(1);
        assert!(store.plan_linkage_task(&root.id, smaller).is_err());
        let mut larger = plan();
        larger.steps.push("补充验证".into());
        assert_eq!(
            store
                .plan_linkage_task(&root.id, larger)
                .unwrap()
                .steps
                .len(),
            4
        );
    }
    #[test]
    fn paused_for_user_input_remains_incomplete_and_can_resume_after_an_answer() {
        let (store, root, worker) = fixture();
        store.plan_linkage_task(&root.id, plan()).unwrap();
        store.begin_linkage_work(&root.id, "完成任务").unwrap();
        store
            .set_status(&worker.id, "waiting", Some("one"))
            .unwrap();
        let mut pause = review(1, "needs_user", 1);
        pause.reason = Some("需要用户在原始审批卡中允许测试命令".into());
        assert_eq!(
            store.review_linkage_task(&root.id, pause).unwrap().state,
            "waiting_user"
        );
        store.set_status(&worker.id, "idle", None).unwrap();
        ended(&store, &root, "one");
        assert_eq!(
            store.linkage_task(&root.id).unwrap().unwrap().state,
            "needs_review"
        );
        assert!(store
            .review_linkage_task(&root.id, review(1, "continue", 1))
            .is_ok());
    }
    #[test]
    fn startup_errors_preserve_the_goal_for_controller_judgment_and_restart_keeps_progress() {
        let (store, root, _) = fixture();
        store.plan_linkage_task(&root.id, plan()).unwrap();
        store.begin_linkage_work(&root.id, "执行").unwrap();
        store
            .fail_linkage_work(&root.id, 1, "指定执行 Agent 尚未安装")
            .unwrap();
        assert_eq!(
            store.linkage_task(&root.id).unwrap().unwrap().state,
            "needs_review"
        );
        let partial = store
            .review_linkage_task(&root.id, review(1, "continue", 1))
            .unwrap();
        let conn = store.0.into_inner().unwrap();
        let restored = Store::from_connection(conn).unwrap();
        let task = restored.linkage_task(&root.id).unwrap().unwrap();
        assert_eq!(task.steps[0].evidence, partial.steps[0].evidence);
        assert!(task.steps[0].done);
        assert_eq!(task.steps.len(), 3);
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub text: String,
    pub done: bool,
    pub evidence: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub objective: String,
    pub steps: Vec<Step>,
    pub state: String,
    pub round: u64,
    pub input_seq: i64,
    pub review_dispatched_round: u64,
    pub finished_turn: Option<String>,
    pub next_task: Option<String>,
    pub reason: Option<String>,
    pub attempts: Vec<String>,
    pub review_error: Option<String>,
    pub execution_error: Option<String>,
}
impl Task {
    pub fn review_receipt(&self) -> serde_json::Value {
        // The controller has already inspected evidence. Its final tool result
        // is its acceptance decision, without returning the raw worker report.
        serde_json::json!({"task":{"state":self.state,"round":self.round,"verified":self.steps.iter().filter(|step|step.done).count(),"total":self.steps.len()},"pending":self.steps.iter().filter(|step|!step.done).map(|step|step.text.as_str()).collect::<Vec<_>>(),"reason":self.reason,"nextAction":"Explain the outcome naturally in your own voice. Do not repeat private worker markers, raw reports or internal monitoring details."})
    }
    pub fn live(&self) -> bool {
        matches!(self.state.as_str(), "working" | "needs_review")
    }
    pub fn closed(&self) -> bool {
        matches!(self.state.as_str(), "completed" | "blocked" | "cancelled")
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub step: usize,
    pub done: bool,
    pub evidence: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Review {
    pub round: u64,
    pub outcome: String,
    pub checks: Vec<Check>,
    pub next_task: Option<String>,
    pub reason: Option<String>,
    #[serde(default)]
    pub attempts: Vec<String>,
}
fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}
fn read(conn: &Connection, id: &str) -> Result<Option<Task>, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT task FROM linkage_tasks WHERE controller_id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(|e| format!("任务验收记录损坏：{e}")))
        .transpose()
}
fn write(conn: &Connection, id: &str, task: &Task) -> Result<(), String> {
    conn.execute("INSERT INTO linkage_tasks(controller_id,task) VALUES(?1,?2) ON CONFLICT(controller_id) DO UPDATE SET task=excluded.task",params![id,serde_json::to_string(task).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    Ok(())
}
fn worker_busy(conn: &Connection, id: &str) -> Result<bool, String> {
    Ok(crate::linkage_store::active_children(conn,id)? || conn.query_row("SELECT EXISTS(SELECT 1 FROM followups f JOIN linkage_workers w ON f.session_id=w.worker_id WHERE w.controller_id=?1 AND f.status IN ('queued','sending','steering'))",[id],|row|row.get(0)).map_err(|e|e.to_string())?)
}
impl Store {
    pub fn linkage_completion_ready(&self, id: &str) -> Result<bool, String> {
        if self.linkage_parent(id)?.is_some() {
            return Ok(false);
        }
        Ok(self
            .linkage_task(id)?
            .is_none_or(|task| task.state == "completed"))
    }
    pub fn linkage_task(&self, id: &str) -> Result<Option<Task>, String> {
        read(&*self.0.lock().map_err(|e| e.to_string())?, id)
    }
    pub fn plan_linkage_task(&self, id: &str, plan: Plan) -> Result<Task, String> {
        if !valid_text(&plan.objective, 8192)
            || plan.steps.is_empty()
            || plan.steps.len() > 32
            || plan.steps.iter().any(|s| !valid_text(s, 2048))
        {
            return Err("请提供完整目标和 1–32 条可验收步骤".into());
        }
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let input_seq: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq),0) FROM messages WHERE session_id=?1 AND role='user'",
                [id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if input_seq == 0 {
            return Err("没有可追踪的用户任务".into());
        }
        let previous = read(&tx, id)?;
        if previous
            .as_ref()
            .is_some_and(|task| task.closed() && task.input_seq == input_seq)
        {
            return Err("当前用户任务已结束，不能在没有新用户请求时重开执行".into());
        }
        let mut task = Task {
            objective: plan.objective.clone(),
            steps: plan
                .steps
                .iter()
                .map(|text| Step {
                    text: text.clone(),
                    done: false,
                    evidence: String::new(),
                })
                .collect(),
            state: "working".into(),
            round: 0,
            input_seq,
            review_dispatched_round: 0,
            finished_turn: None,
            next_task: None,
            reason: None,
            attempts: vec![],
            review_error: None,
            execution_error: None,
        };
        if let Some(previous) =
            previous.filter(|task| !task.closed() && task.input_seq == input_seq)
        {
            if plan.objective != previous.objective
                || plan.steps.len() < previous.steps.len()
                || previous
                    .steps
                    .iter()
                    .zip(&plan.steps)
                    .any(|(old, new)| old.text != *new)
            {
                return Err("不能删减当前用户任务的未完成清单；请逐项验收并继续剩余工作".into());
            }
            task = previous;
            for text in plan.steps.iter().skip(task.steps.len()) {
                task.steps.push(Step {
                    text: text.clone(),
                    done: false,
                    evidence: String::new(),
                });
            }
        } else if worker_busy(&tx, id)? {
            return Err("执行方仍在工作，请先跟进当前计划".into());
        }
        write(&tx, id, &task)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(task)
    }
    pub fn begin_linkage_work(&self, id: &str, text: &str) -> Result<Task, String> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut task = read(&tx, id)?.ok_or("请先调用 task_plan 登记用户的完整目标和验收清单")?;
        if task.closed()
            || task.state != "working"
            || task.round > 0 && task.next_task.as_deref() != Some(text)
        {
            return Err("请先调用 task_review 验收本轮，并明确继续执行的剩余任务".into());
        }
        if worker_busy(&tx, id)? {
            return Err("执行方仍有任务，请先等待本轮结果".into());
        }
        task.round += 1;
        task.next_task = None;
        task.finished_turn = None;
        task.review_error = None;
        task.execution_error = None;
        write(&tx, id, &task)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(task)
    }
    pub fn finish_linkage_work(&self, id: &str, turn: &str) -> Result<(), String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        let Some(mut task) = read(&conn, id)? else {
            return Ok(());
        };
        if task.round == 0
            || task.closed()
            || task.next_task.is_some()
            || task.finished_turn.as_deref() == Some(turn)
            || worker_busy(&conn, id)?
        {
            return Ok(());
        }
        task.finished_turn = Some(turn.into());
        task.state = "needs_review".into();
        write(&conn, id, &task)
    }
    pub fn fail_linkage_work(&self, id: &str, round: u64, error: &str) -> Result<(), String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        if let Some(mut task) =
            read(&conn, id)?.filter(|task| task.round == round && !task.closed())
        {
            task.execution_error = Some(crate::protocol::bounded(error, 4096));
            if !worker_busy(&conn, id)? {
                task.state = "needs_review".into();
            }
            write(&conn, id, &task)?;
        }
        Ok(())
    }
    pub fn review_linkage_task(&self, id: &str, review: Review) -> Result<Task, String> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut task = read(&tx, id)?.ok_or("请先登记完整任务计划")?;
        if task.closed() || task.round == 0 || task.round != review.round {
            return Err("任务或执行轮次已改变，请重新读取 executor_status 后验收".into());
        }
        if !matches!(
            review.outcome.as_str(),
            "continue" | "completed" | "blocked" | "needs_user"
        ) {
            return Err("验收结论无效".into());
        }
        if review.outcome != "needs_user"
            && (!matches!(task.state.as_str(), "needs_review" | "waiting_user")
                || worker_busy(&tx, id)?)
        {
            return Err("执行方尚未结束，请先等待或处理审批、提问".into());
        }
        if review.checks.len() != task.steps.len() {
            return Err("必须逐项核对完整计划，不能只验收已经完成的第一步".into());
        }
        let mut seen = std::collections::HashSet::new();
        for check in &review.checks {
            if check.step == 0
                || check.step > task.steps.len()
                || !seen.insert(check.step)
                || !valid_text(&check.evidence, 4096)
            {
                return Err("每项都需要唯一序号和明确的完成证据或未完成原因".into());
            }
            let step = &mut task.steps[check.step - 1];
            step.done = check.done;
            step.evidence = check.evidence.clone();
        }
        let all_done = task.steps.iter().all(|step| step.done);
        if review.outcome == "completed" && !all_done {
            return Err("仍有未完成的计划项，必须继续派发，不能交付完成".into());
        }
        if matches!(review.outcome.as_str(), "blocked" | "needs_user") && all_done {
            return Err("请如实标记尚未完成或未验证的项目".into());
        }
        if review.outcome == "continue"
            && (all_done
                || !review
                    .next_task
                    .as_deref()
                    .is_some_and(|text| valid_text(text, 65536)))
        {
            return Err("请明确派发尚未完成的步骤及验收要求".into());
        }
        if matches!(review.outcome.as_str(), "blocked" | "needs_user")
            && !review
                .reason
                .as_deref()
                .is_some_and(|text| valid_text(text, 4096))
        {
            return Err("请明确说明实际阻碍或需要用户提供的信息".into());
        }
        if review.outcome == "blocked"
            && (review.attempts.is_empty()
                || review.attempts.len() > 16
                || review.attempts.iter().any(|text| !valid_text(text, 2048)))
        {
            return Err("认定无法继续前，必须说明已尝试的方法和结果".into());
        }
        task.state = match review.outcome.as_str() {
            "continue" => "working",
            "needs_user" => "waiting_user",
            value => value,
        }
        .into();
        task.next_task = if review.outcome == "continue" {
            review.next_task
        } else {
            None
        };
        task.reason = review.reason;
        task.attempts = review.attempts;
        task.review_error = None;
        task.execution_error = None;
        write(&tx, id, &task)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(task)
    }
    pub fn claim_linkage_review(&self, id: &str) -> Result<Option<u64>, String> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let Some(mut task) = read(&tx, id)? else {
            return Ok(None);
        };
        let idle: bool = tx
            .query_row(
            "SELECT status='idle' AND archived=0 AND NOT EXISTS(SELECT 1 FROM followups WHERE session_id=?1 AND status IN ('queued','sending','steering')) FROM sessions WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !idle
            || worker_busy(&tx, id)?
            || task.state != "needs_review"
            || task.review_dispatched_round >= task.round
        {
            return Ok(None);
        }
        task.review_dispatched_round = task.round;
        write(&tx, id, &task)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Some(task.round))
    }
    pub fn linkage_review_error(&self, id: &str, round: u64, error: &str) -> Result<(), String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        if let Some(mut task) =
            read(&conn, id)?.filter(|task| task.round == round && !task.closed())
        {
            task.review_error = Some(crate::protocol::bounded(error, 4096));
            write(&conn, id, &task)?;
        }
        Ok(())
    }
    pub fn defer_linkage_review(&self, id: &str, round: u64) -> Result<(), String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        if let Some(mut task) = read(&conn, id)?.filter(|task| {
            task.round == round
                && task.state == "needs_review"
                && task.review_dispatched_round == round
        }) {
            task.review_dispatched_round = 0;
            write(&conn, id, &task)?;
        }
        Ok(())
    }
    pub fn cancel_linkage_task(&self, id: &str) -> Result<(), String> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        if let Some(mut task) = read(&conn, id)?.filter(|task| !task.closed()) {
            task.state = "cancelled".into();
            task.next_task = None;
            write(&conn, id, &task)?;
        }
        Ok(())
    }
}
