//! Correlate native background Agent lifecycle events with their original tool row.
//! Launch acknowledgements are metadata, and never a completed child report.
use crate::protocol;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Default)]
pub struct Tasks {
    items: HashMap<String, Value>,
}
fn agent(item: &Value) -> bool {
    matches!(item["tool"].as_str(), Some("Agent" | "Task"))
}
fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let start = text.find(&format!("<{name}>"))? + name.len() + 2;
    let end = text[start..].rfind(&format!("</{name}>"))? + start;
    Some(&text[start..end])
}
fn status(value: &str) -> Option<&'static str> {
    match value {
        "pending" | "running" => Some("running"),
        "paused" => Some("waiting"),
        "completed" => Some("completed"),
        "failed" => Some("failed"),
        "stopped" | "killed" => Some("interrupted"),
        _ => None,
    }
}
fn step(item: &mut Value, mut next: Value) {
    if !item["subagentSteps"].is_array() {
        item["subagentSteps"] = json!([]);
    }
    let steps = item["subagentSteps"].as_array_mut().unwrap();
    if let Some(old) = steps.iter_mut().find(|old| old["id"] == next["id"]) {
        if next["status"] == "inProgress"
            && matches!(
                old["status"].as_str(),
                Some("completed" | "failed" | "interrupted")
            )
        {
            next["status"] = old["status"].clone();
        }
        *old = next;
    } else {
        steps.push(next);
    }
    // A small summary history, not a tool or agent concurrency limit. Never
    // retain child outputs, command contents or full invocation arguments here.
    let mut truncated = false;
    while steps.len() > 200
        || steps
            .iter()
            .map(|value| value.to_string().len())
            .sum::<usize>()
            > 32 * 1024
    {
        steps.remove(0);
        truncated = true;
    }
    if truncated {
        item["subagentStepsTruncated"] = json!(true);
    }
}
impl Tasks {
    fn remember(&mut self, id: String, item: Value) {
        self.items.insert(id.clone(), item);
        // Keep every live child. This is a terminal history cache budget, never
        // a limit on how many agents may run or be tracked concurrently.
        while self.items.len() > 200 {
            let finished = self
                .items
                .iter()
                .find(|(key, item)| {
                    *key != &id
                        && !matches!(item["subagentStatus"].as_str(), Some("running" | "waiting"))
                })
                .map(|(key, _)| key.clone());
            let Some(finished) = finished else {
                break;
            };
            self.items.remove(&finished);
        }
    }
    pub fn launch(&mut self, item: &mut Value, metadata: &Value, output: &str, now: i64) {
        if !agent(item) {
            return;
        }
        if let Some(old) = item["id"].as_str().and_then(|id| self.items.get(id)) {
            for key in ["subagentSteps", "subagentStepsTruncated"] {
                if let Some(value) = old.get(key) {
                    item[key] = value.clone();
                }
            }
        }
        let native = metadata["status"] == "async_launched" || metadata["isAsync"] == true;
        let legacy = output.starts_with("Async agent launched successfully.");
        if (!native && !legacy) || item["status"] == "failed" {
            let failed = item["status"] == "failed";
            if let Some(steps) = item["subagentSteps"].as_array_mut() {
                for value in steps
                    .iter_mut()
                    .filter(|value| value["status"] == "inProgress")
                {
                    value["status"] = json!(if failed { "interrupted" } else { "completed" });
                }
            }
            if let Some(id) = item["id"]
                .as_str()
                .map(str::to_owned)
                .filter(|id| self.items.contains_key(id))
            {
                item["subagentStatus"] = item["status"].clone();
                self.remember(id, item.clone());
            }
            return;
        }
        let task_id = metadata["agentId"].as_str().or_else(|| {
            output.lines().find_map(|line| {
                line.strip_prefix("agentId: ")
                    .and_then(|line| line.split_whitespace().next())
            })
        });
        let Some(id) = item["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
        else {
            return;
        };
        item["subagentAsync"] = json!(true);
        item["subagentStatus"] = self
            .items
            .get(&id)
            .map(|old| old["subagentStatus"].clone())
            .filter(|value| !value.is_null())
            .unwrap_or(json!("running"));
        item["subagentResult"] = self
            .items
            .get(&id)
            .map(|old| old["subagentResult"].clone())
            .unwrap_or(json!(""));
        item["subagentId"] = json!(task_id);
        item["subagentStartedAt"] = self
            .items
            .get(&id)
            .map(|old| old["subagentStartedAt"].clone())
            .unwrap_or(json!(now));
        item["subagentDurationMs"] = self
            .items
            .get(&id)
            .map(|old| old["subagentDurationMs"].clone())
            .unwrap_or(Value::Null);
        if let Some(model) = metadata["resolvedModel"].as_str() {
            item["model"] = json!(model);
        }
        self.remember(id, item.clone());
    }
    pub fn pending(&self) -> bool {
        self.items
            .values()
            .any(|item| matches!(item["subagentStatus"].as_str(), Some("running" | "waiting")))
    }
    pub fn observe(
        &mut self,
        message: &Value,
        tools: &HashMap<String, Value>,
        now: i64,
    ) -> Option<Value> {
        // Child assistant messages are native stream events correlated by the
        // parent Agent tool ID. They must not become primary conversation text.
        if matches!(message["type"].as_str(), Some("assistant" | "user")) {
            if let Some(id) = message["parent_tool_use_id"].as_str() {
                let mut item = self.items.get(id).or_else(|| tools.get(id))?.clone();
                if !agent(&item) {
                    return None;
                }
                let content = message["message"]["content"].as_array()?;
                let message_id = message["message"]["id"]
                    .as_str()
                    .or_else(|| message["uuid"].as_str())
                    .unwrap_or("child");
                let mut changed = false;
                for (index, block) in content.iter().enumerate() {
                    match block["type"].as_str() {
                        Some("thinking" | "redacted_thinking")
                            if message["type"] == "assistant" =>
                        {
                            step(
                                &mut item,
                                json!({"id":format!("{message_id}-reasoning-{index}"),"type":"reasoning","summary":[protocol::bounded(block["thinking"].as_str().unwrap_or(""),512)],"redacted":block["type"]=="redacted_thinking","status":"completed"}),
                            );
                            changed = true;
                        }
                        Some("tool_use")
                            if message["type"] == "assistant" && block["id"].is_string() =>
                        {
                            let target = ["file_path", "path", "pattern", "description"]
                                .iter()
                                .find_map(|key| block["input"][*key].as_str())
                                .unwrap_or("");
                            // The UI needs the operation and target, never the
                            // contents of Write/Edit/Bash or credentials.
                            step(
                                &mut item,
                                json!({"id":block["id"],"type":"claudeToolCall","tool":protocol::bounded(block["name"].as_str().unwrap_or("工具"),100),"arguments":{"path":protocol::bounded(target,256)},"status":"inProgress"}),
                            );
                            changed = true;
                        }
                        Some("tool_result") if message["type"] == "user" => {
                            if let Some(old) = item["subagentSteps"]
                                .as_array()
                                .and_then(|steps| {
                                    steps.iter().find(|step| step["id"] == block["tool_use_id"])
                                })
                                .cloned()
                            {
                                let mut finished = old;
                                finished["status"] = json!(if block["is_error"] == true {
                                    "failed"
                                } else {
                                    "completed"
                                });
                                step(&mut item, finished);
                                changed = true;
                            }
                        }
                        _ => {}
                    }
                }
                let report = content
                    .iter()
                    .filter(|block| block["type"] == "text")
                    .filter_map(|block| block["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                if report.trim().is_empty() && !changed {
                    return None;
                }
                item["subagentAsync"] = json!(true);
                if item["subagentStatus"].is_null() {
                    item["subagentStatus"] = json!("running");
                }
                if item["subagentStartedAt"].is_null() {
                    item["subagentStartedAt"] = json!(now);
                }
                if !report.trim().is_empty() && message["type"] == "assistant" {
                    item["subagentResult"] =
                        json!(protocol::bounded(&report, protocol::TEXT_LIMIT));
                }
                self.remember(id.to_owned(), item.clone());
                return Some(item);
            }
        }
        let system = message["type"] == "system";
        let synthetic = message["type"] == "user"
            && (message["origin"]["kind"] == "task-notification" || message["isSynthetic"] == true);
        let content = &message["message"]["content"];
        let blocks = content.as_array().map(|blocks| {
            blocks
                .iter()
                .filter(|block| block["type"] == "text")
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        });
        let text = content.as_str().or(blocks.as_deref()).unwrap_or("");
        let subtype = message["subtype"].as_str().unwrap_or("");
        if !system && !(synthetic && text.starts_with("<task-notification>")) {
            return None;
        }
        if system
            && !matches!(
                subtype,
                "task_started" | "task_progress" | "task_updated" | "task_notification"
            )
        {
            return None;
        }
        let task_id = message["task_id"].as_str().or_else(|| tag(text, "task-id"));
        let direct = message["tool_use_id"]
            .as_str()
            .or_else(|| tag(text, "tool-use-id"));
        let id = direct
            .filter(|id| self.items.contains_key(*id) || tools.get(*id).is_some_and(agent))
            .map(str::to_owned)
            .or_else(|| {
                task_id.and_then(|task| {
                    self.items
                        .iter()
                        .find(|(_, item)| item["subagentId"] == task)
                        .map(|(id, _)| id.clone())
                })
            })?;
        let mut item = self.items.get(&id).or_else(|| tools.get(&id))?.clone();
        if !agent(&item) {
            return None;
        }
        item["subagentAsync"] = json!(true);
        if item["subagentStartedAt"].is_null() {
            item["subagentStartedAt"] = json!(now);
        }
        if let Some(task) = task_id {
            item["subagentId"] = json!(protocol::bounded(task, 200));
        }
        let native_status = message["patch"]["status"]
            .as_str()
            .or_else(|| message["status"].as_str())
            .or_else(|| tag(text, "status"));
        let next = native_status
            .and_then(status)
            .or_else(|| matches!(subtype, "task_started" | "task_progress").then_some("running"));
        if let Some(next) = next {
            if item["subagentDurationMs"].is_null() || !matches!(next, "running" | "waiting") {
                item["subagentStatus"] = json!(next);
            }
            if !matches!(next, "running" | "waiting") && item["subagentDurationMs"].is_null() {
                item["subagentDurationMs"] = json!(now
                    .saturating_sub(item["subagentStartedAt"].as_i64().unwrap_or(now))
                    .max(0));
            }
            if !matches!(next, "running" | "waiting") {
                if let Some(steps) = item["subagentSteps"].as_array_mut() {
                    for step in steps
                        .iter_mut()
                        .filter(|step| step["status"] == "inProgress")
                    {
                        step["status"] = json!(if next == "completed" {
                            "completed"
                        } else {
                            "interrupted"
                        });
                    }
                }
            }
        }
        let report = if synthetic {
            tag(text, "result")
        } else {
            message["patch"]["result"]
                .as_str()
                .or_else(|| message["result"].as_str())
        };
        if let Some(report) = report.filter(|report| !report.trim().is_empty()) {
            item["subagentResult"] = json!(protocol::bounded(report, protocol::TEXT_LIMIT));
        } else if item["subagentResult"].is_null() {
            item["subagentResult"] = json!("");
        }
        self.remember(id, item.clone());
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_steps_are_correlated_deduplicated_bounded_and_keep_failures_without_tool_output() {
        let mut tasks = Tasks::default();
        let item = json!({"id":"parent-agent","tool":"Agent","status":"inProgress"});
        let tools = HashMap::from([("parent-agent".to_owned(), item)]);
        let child = json!({"type":"assistant","parent_tool_use_id":"parent-agent","message":{"id":"child-message","content":[{"type":"thinking","thinking":"检查边界"},{"type":"tool_use","id":"read-a","name":"Read","input":{"file_path":"fixture/README.md","secret":"DO_NOT_STORE"}}]}});
        let first = tasks.observe(&child, &tools, 1000).unwrap();
        assert_eq!(first["subagentSteps"].as_array().unwrap().len(), 2);
        assert_eq!(
            first["subagentSteps"][1]["arguments"]["path"],
            "fixture/README.md"
        );
        assert!(!first.to_string().contains("DO_NOT_STORE"));
        let result = json!({"type":"user","parent_tool_use_id":"parent-agent","message":{"content":[{"type":"tool_result","tool_use_id":"read-a","is_error":true,"content":"PRIVATE_TOOL_OUTPUT"}]}});
        let completed = tasks.observe(&result, &tools, 2000).unwrap();
        assert_eq!(completed["subagentSteps"][1]["status"], "failed");
        assert!(!completed.to_string().contains("PRIVATE_TOOL_OUTPUT"));
        let replayed = tasks.observe(&child, &tools, 3000).unwrap();
        assert_eq!(replayed["subagentSteps"].as_array().unwrap().len(), 2);
        assert_eq!(replayed["subagentSteps"][1]["status"], "failed");
        let mut unknown = result;
        unknown["parent_tool_use_id"] = json!("unrelated");
        assert!(tasks.observe(&unknown, &tools, 4000).is_none());
        for i in 0..250 {
            let child = json!({"type":"assistant","parent_tool_use_id":"parent-agent","message":{"content":[{"type":"tool_use","id":format!("read-{i}"),"name":"Read","input":{"file_path":"x".repeat(1000)}}]}});
            tasks.observe(&child, &tools, 5000);
        }
        let history = &tasks.items["parent-agent"];
        assert!(history["subagentSteps"].as_array().unwrap().len() <= 200);
        assert!(history["subagentSteps"].to_string().len() <= 33 * 1024);
        assert_eq!(history["subagentStepsTruncated"], true);
    }
    #[test]
    fn sync_success_or_failure_settles_forwarded_child_work_and_keeps_steps() {
        for status in ["completed", "failed"] {
            let mut tasks = Tasks::default();
            let mut parent = json!({"id":"agent","tool":"Agent","status":"inProgress"});
            let tools = HashMap::from([("agent".to_owned(), parent.clone())]);
            tasks.observe(&json!({"type":"assistant","parent_tool_use_id":"agent","message":{"content":[{"type":"tool_use","id":"read","name":"Read","input":{"file_path":"README.md"}}]}}),&tools,1000);
            assert!(tasks.pending());
            parent["status"] = json!(status);
            tasks.launch(&mut parent, &Value::Null, "final tool result", 2000);
            assert!(!tasks.pending());
            assert_eq!(parent["subagentStatus"], status);
            assert_eq!(parent["subagentSteps"][0]["tool"], "Read");
        }
    }
    #[test]
    fn async_launch_and_completion_preserve_native_child_steps() {
        let mut tasks = Tasks::default();
        let mut parent = json!({"id":"agent","tool":"Agent","status":"completed"});
        let tools = HashMap::from([("agent".to_owned(), parent.clone())]);
        tasks.observe(&json!({"type":"assistant","parent_tool_use_id":"agent","message":{"content":[{"type":"tool_use","id":"read","name":"Read","input":{"file_path":"README.md"}}]}}),&tools,1000);
        tasks.launch(
            &mut parent,
            &json!({"isAsync":true,"agentId":"child"}),
            "",
            2000,
        );
        assert_eq!(parent["subagentSteps"][0]["status"], "inProgress");
        let completed=tasks.observe(&json!({"type":"system","subtype":"task_notification","task_id":"child","status":"completed","result":"CHILD_DONE"}),&tools,3000).unwrap();
        assert_eq!(completed["subagentSteps"][0]["status"], "completed");
        assert_eq!(completed["subagentResult"], "CHILD_DONE");
        assert!(!tasks.pending());
    }
    #[test]
    fn asynchronous_launch_is_not_child_completion_and_correlated_report_replaces_metadata() {
        let mut tasks = Tasks::default();
        let mut item = json!({"id":"tool-a","type":"claudeToolCall","tool":"Agent","status":"completed","arguments":{"description":"normal-demo"},"output":"SDK internal metadata"});
        tasks.launch(&mut item, &json!({"isAsync":true,"status":"async_launched","agentId":"child-a","resolvedModel":"glm-5.3"}), "", 1000);
        assert!(tasks.pending());
        assert_eq!(item["subagentResult"], "");
        assert_eq!(item["subagentStatus"], "running");
        // The task ID also correlates notifications with no tool_use_id.
        let terminal = tasks.observe(&json!({"type":"system","subtype":"task_notification","task_id":"child-a","status":"completed","summary":"finished"}), &HashMap::new(), 4000).unwrap();
        assert_eq!(terminal["subagentDurationMs"], 3000);
        assert!(!tasks.pending());
        let notification = "<task-notification>\n<task-id>child-a</task-id><tool-use-id>tool-a</tool-use-id><status>completed</status><result># 完整报告\n\n```js\n  const v = 1;\n```\nCHILD_DONE</result></task-notification>";
        let untrusted = json!({"type":"user","message":{"content":notification}});
        assert!(tasks.observe(&untrusted, &HashMap::new(), 5000).is_none());
        let mut trusted = untrusted;
        trusted["origin"] = json!({"kind":"task-notification"});
        let report = tasks.observe(&trusted, &HashMap::new(), 5000).unwrap();
        assert_eq!(report["id"], "tool-a");
        assert!(report["subagentResult"]
            .as_str()
            .unwrap()
            .ends_with("CHILD_DONE"));
        assert!(report["subagentResult"]
            .as_str()
            .unwrap()
            .contains("  const v = 1;"));
        assert_eq!(report["subagentDurationMs"], 3000);
    }
    #[test]
    fn native_child_assistant_report_survives_launch_and_requires_matching_agent_id() {
        let mut tasks = Tasks::default();
        let item = json!({"id":"agent-tool","tool":"Agent","status":"inProgress"});
        let tools = HashMap::from([("agent-tool".to_owned(), item.clone())]);
        let mut message = json!({"type":"assistant","parent_tool_use_id":"unknown","message":{"content":[{"type":"text","text":"# 子代理\n完整报告\nCHILD_DONE"}]}});
        assert!(tasks.observe(&message, &tools, 1000).is_none());
        message["parent_tool_use_id"] = json!("agent-tool");
        let child = tasks.observe(&message, &tools, 1000).unwrap();
        assert_eq!(child["subagentStatus"], "running");
        assert!(tasks.pending());
        let mut acknowledged = item;
        tasks.launch(
            &mut acknowledged,
            &json!({"isAsync":true,"agentId":"child-a"}),
            "",
            2000,
        );
        assert_eq!(acknowledged["subagentResult"], child["subagentResult"]);
        let notification = json!({"type":"user","isSynthetic":true,"message":{"content":[{"type":"text","text":"<task-notification><task-id>child-a</task-id><status>completed</status><result>完整新报告\nCHILD_DONE</result></task-notification>"}]}});
        let final_item = tasks.observe(&notification, &tools, 3000).unwrap();
        assert!(!tasks.pending());
        assert_eq!(final_item["subagentResult"], "完整新报告\nCHILD_DONE");
    }
    #[test]
    fn task_updates_handle_failure_and_stopping_without_inventing_other_agent_results() {
        for (native, expected) in [
            ("failed", "failed"),
            ("killed", "interrupted"),
            ("stopped", "interrupted"),
        ] {
            let mut tasks = Tasks::default();
            let mut item = json!({"id":"tool-a","tool":"Task"});
            tasks.launch(
                &mut item,
                &json!({"isAsync":true,"agentId":"child-a"}),
                "",
                1000,
            );
            assert!(tasks.observe(&json!({"type":"system","subtype":"task_notification","task_id":"other","status":"completed"}),&HashMap::new(),2000).is_none());
            let result=tasks.observe(&json!({"type":"system","subtype":"task_updated","task_id":"child-a","patch":{"status":native}}),&HashMap::new(),2000).unwrap();
            assert_eq!(result["subagentStatus"], expected);
            assert!(!tasks.pending());
        }
    }
    #[test]
    fn native_start_before_tool_result_and_legacy_launch_keep_background_work_pending() {
        let mut tasks = Tasks::default();
        let item = json!({"id":"a","tool":"Agent","status":"inProgress"});
        let tools = HashMap::from([("a".to_owned(), item.clone())]);
        let started=tasks.observe(&json!({"type":"system","subtype":"task_started","tool_use_id":"a","task_id":"child-a"}),&tools,1000).unwrap();
        assert_eq!(started["subagentStatus"], "running");
        assert!(tasks.pending());
        let mut completed = item;
        completed["status"] = json!("completed");
        tasks.launch(
            &mut completed,
            &Value::Null,
            "Async agent launched successfully.\nagentId: child-a (internal ID)",
            2000,
        );
        assert_eq!(completed["subagentStartedAt"], 1000);
        assert_eq!(completed["subagentId"], "child-a");
        let mut bash = json!({"id":"shell","tool":"Bash"});
        tasks.launch(&mut bash, &json!({"isAsync":true}), "", 3000);
        assert!(bash["subagentAsync"].is_null());
    }
    #[test]
    fn completion_before_launch_ack_and_late_progress_do_not_revive_finished_children() {
        let mut tasks = Tasks::default();
        let mut item = json!({"id":"a","tool":"Agent"});
        let tools = HashMap::from([("a".to_owned(), item.clone())]);
        tasks.observe(
            &json!({"type":"system","subtype":"task_started","tool_use_id":"a","task_id":"child"}),
            &tools,
            1000,
        );
        tasks.observe(&json!({"type":"system","subtype":"task_updated","task_id":"child","patch":{"status":"completed"}}),&tools,2000);
        tasks.launch(
            &mut item,
            &json!({"isAsync":true,"agentId":"child"}),
            "",
            3000,
        );
        assert_eq!(item["subagentStatus"], "completed");
        assert_eq!(item["subagentDurationMs"], 1000);
        let late = tasks
            .observe(
                &json!({"type":"system","subtype":"task_progress","task_id":"child"}),
                &tools,
                4000,
            )
            .unwrap();
        assert_eq!(late["subagentStatus"], "completed");
        assert!(!tasks.pending());
    }
    #[test]
    fn terminal_cache_budget_does_not_limit_concurrent_child_tracking() {
        let mut tasks = Tasks::default();
        for i in 0..205 {
            let mut item = json!({"id":format!("tool-{i}"),"tool":"Agent"});
            tasks.launch(
                &mut item,
                &json!({"isAsync":true,"agentId":format!("child-{i}")}),
                "",
                1000,
            );
        }
        assert_eq!(tasks.items.len(), 205);
        for i in 0..205 {
            assert!(tasks.observe(&json!({"type":"system","subtype":"task_notification","task_id":format!("child-{i}"),"status":"completed"}),&HashMap::new(),2000).is_some());
        }
        assert!(!tasks.pending());
        assert_eq!(tasks.items.len(), 200);
    }
}
