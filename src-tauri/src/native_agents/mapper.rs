//! Normalized native stream events; incremental text stays in the activity cache.
use super::{event, protocol, Turn};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct Mapper {
    pub(super) items: HashMap<String, Value>,
    pub(super) dirty: HashSet<String>,
    pub(super) message: u64,
    pub(super) failed: bool,
    pub(super) failure: Option<String>,
    pub(super) usage: Value,
    pub(super) priced: bool,
    pub(super) model_context_window: Option<u64>,
    pub(super) acp_phase: String,
    pub(super) acp_segment: u64,
    pub(super) media_segment: u64,
}
impl Mapper {
    pub(super) fn map(&mut self, t: &Turn, v: &Value) -> Vec<Value> {
        let mut out = vec![];
        let mut item = None;
        let mut delta = None;
        if t.agent == "pi" {
            match v["type"].as_str().unwrap_or("") {
                "message_start" if v["message"]["role"] == "assistant" => self.message += 1,
                "message_update" => {
                    let e = &v["assistantMessageEvent"];
                    let usage = v
                        .get("usage")
                        .filter(|u| u.is_object())
                        .or_else(|| v["message"].get("usage").filter(|u| u.is_object()))
                        .or_else(|| e["partial"].get("usage").filter(|u| u.is_object()));
                    if let Some(usage) = usage {
                        if self.observe_pi_usage(usage) {
                            out.push(event(
                                t,
                                "thread/tokenUsage/updated",
                                json!({"tokenUsage":self.usage}),
                            ));
                        }
                    }
                    let index = e["contentIndex"].as_u64().unwrap_or(0);
                    let id = format!("{}-msg-{}-{index}", t.turn, self.message);
                    match e["type"].as_str().unwrap_or("") {
                        "text_start" | "text_delta" | "thinking_start" | "thinking_delta" => {
                            let thought = e["type"].as_str().unwrap().starts_with("thinking");
                            let existing = self.items.contains_key(&id);
                            let row=self.items.entry(id.clone()).or_insert_with(||if thought{json!({"id":id,"type":"reasoning","summary":[""],"status":"inProgress"})}else{json!({"id":id,"type":"agentMessage","messageId":format!("{}-msg-{}", t.turn, self.message),"text":"","status":"inProgress"})});
                            let old = if thought {
                                row["summary"][0].as_str().unwrap_or("")
                            } else {
                                row["text"].as_str().unwrap_or("")
                            };
                            let text = protocol::bounded(
                                &format!("{old}{}", e["delta"].as_str().unwrap_or("")),
                                protocol::TEXT_LIMIT,
                            );
                            if thought {
                                row["summary"][0] = text.into();
                            } else {
                                row["text"] = text.into();
                            }
                            if existing {
                                self.dirty.insert(id.clone());
                                delta = Some(event(
                                    t,
                                    if thought {
                                        "item/reasoning/summaryTextDelta"
                                    } else {
                                        "item/agentMessage/delta"
                                    },
                                    json!({"itemId":id,"delta":e["delta"].as_str().unwrap_or(""),"summaryIndex":0}),
                                ));
                            } else {
                                item = Some(row.clone());
                            }
                        }
                        _ => {}
                    }
                }
                "message_end" if v["message"]["role"] == "assistant" => {
                    let m = &v["message"];
                    self.failed |= m["stopReason"] == "error";
                    if let Some(blocks) = m["content"].as_array() {
                        for (i, b) in blocks.iter().enumerate() {
                            let id = format!("{}-msg-{}-{i}", t.turn, self.message);
                            let row = match b["type"].as_str() {
                                Some("text") => {
                                    json!({"id":id,"type":"agentMessage","messageId":format!("{}-msg-{}", t.turn, self.message),"text":b["text"],"status":"completed"})
                                }
                                Some("thinking") => {
                                    json!({"id":id,"type":"reasoning","summary":[b["thinking"]],"status":"completed"})
                                }
                                Some("image" | "audio" | "video") => {
                                    json!({"id":id,"type":"agentMessage","messageId":format!("{}-msg-{}", t.turn, self.message),"text":"","contentItems":[b],"status":"completed"})
                                }
                                _ => continue,
                            };
                            self.items.remove(&id);
                            self.dirty.remove(&id);
                            out.push(event(t, "item/completed", json!({"item":row})));
                        }
                    }
                    if let Some(error) = m["errorMessage"].as_str() {
                        self.failure = Some(protocol::bounded(error, 4096));
                        out.push(event(t, "error", json!({"error":{"message":error}})));
                    }
                    if m["usage"].is_object() {
                        out.push(event(
                            t,
                            "thread/tokenUsage/updated",
                            json!({"tokenUsage":self.add_usage(&m["usage"])}),
                        ));
                    }
                }
                "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
                    let id = format!(
                        "{}-tool-{}",
                        t.turn,
                        v["toolCallId"].as_str().unwrap_or("unknown")
                    );
                    let end = v["type"] == "tool_execution_end";
                    let row=self.items.entry(id.clone()).or_insert(json!({"id":id,"type":"claudeToolCall","tool":pi_tool(v["toolName"].as_str().unwrap_or("Tool")),"arguments":tool_args(&v["args"]),"cwd":t.client.cwd,"status":"inProgress"}));
                    if !end && v["partialResult"]["content"].is_array() {
                        row["output"] = blocks_text(&v["partialResult"]["content"]).into();
                        row["contentItems"] = v["partialResult"]["content"].clone();
                    }
                    if end {
                        apply_pi_result(row, &v["result"], v["isError"] == true);
                        row["status"] = if v["isError"] == true {
                            "failed"
                        } else {
                            "completed"
                        }
                        .into();
                    }
                    item = Some(row.clone());
                }
                "compaction_start" => {
                    item = Some(
                        json!({"id":format!("{}-compact",t.turn),"type":"contextCompaction","status":"inProgress"}),
                    )
                }
                "compaction_end" => {
                    if v["aborted"] != true && v["errorMessage"].is_null() {
                        if !self.usage.is_object() {
                            self.usage = json!({"total":{},"last":{}});
                        }
                        self.usage["contextTokens"] = Value::Null;
                        self.usage["last"] = json!({});
                        out.push(event(
                            t,
                            "thread/tokenUsage/updated",
                            json!({"tokenUsage":self.usage}),
                        ));
                    }
                    let status = if v["aborted"] == true {
                        "interrupted"
                    } else if !v["errorMessage"].is_null() {
                        "failed"
                    } else {
                        "completed"
                    };
                    item = Some(
                        json!({"id":format!("{}-compact",t.turn),"type":"contextCompaction","status":status,"error":v["errorMessage"]}),
                    )
                }
                _ => {}
            }
        } else if v["method"] == "session/update" && v["params"]["sessionId"] == t.native {
            let u = &v["params"]["update"];
            match u["sessionUpdate"].as_str().unwrap_or("") {
                "agent_message_chunk" | "agent_thought_chunk" => {
                    let thought = u["sessionUpdate"] == "agent_thought_chunk";
                    if !thought
                        && matches!(
                            u["content"]["type"].as_str(),
                            Some("image" | "audio" | "video" | "resource" | "resource_link")
                        )
                    {
                        self.media_segment += 1;
                        out.push(event(t, "item/completed", json!({"item":{"id":format!("{}-media-{}",t.turn,self.media_segment),"type":"agentMessage","text":"","contentItems":[u["content"]],"status":"completed"}})));
                        return out;
                    }
                    let phase = if thought { "thought" } else { "msg" };
                    if u["messageId"].as_str().is_none() && self.acp_phase != phase {
                        let old = self
                            .items
                            .iter()
                            .filter(|(_, row)| {
                                matches!(row["type"].as_str(), Some("agentMessage" | "reasoning"))
                            })
                            .map(|(id, _)| id.clone())
                            .collect::<Vec<_>>();
                        for id in old {
                            if let Some(mut row) = self.items.remove(&id) {
                                row["status"] = "completed".into();
                                out.push(event(t, "item/completed", json!({"item":row})));
                            }
                        }
                        self.acp_segment += 1;
                        self.acp_phase = phase.into();
                    }
                    let id = format!(
                        "{}-{}-{}",
                        t.turn,
                        if thought { "thought" } else { "msg" },
                        u["messageId"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| self.acp_segment.to_string())
                    );
                    let existing = self.items.contains_key(&id);
                    let row = self.items.entry(id.clone()).or_insert_with(|| {
                        if thought {
                            json!({"id":id,"type":"reasoning","summary":[""],"status":"inProgress"})
                        } else {
                            json!({"id":id,"type":"agentMessage","text":"","status":"inProgress"})
                        }
                    });
                    let d = u["content"]["text"].as_str().unwrap_or("");
                    let old = if thought {
                        row["summary"][0].as_str().unwrap_or("")
                    } else {
                        row["text"].as_str().unwrap_or("")
                    };
                    let text = protocol::bounded(&format!("{old}{d}"), protocol::TEXT_LIMIT);
                    if thought {
                        row["summary"][0] = text.into();
                    } else {
                        row["text"] = text.into();
                    }
                    delta = Some(event(
                        t,
                        if thought {
                            "item/reasoning/summaryTextDelta"
                        } else {
                            "item/agentMessage/delta"
                        },
                        json!({"itemId":id,"delta":d,"summaryIndex":0}),
                    ));
                    if !existing {
                        item = Some(row.clone());
                        delta = None;
                    } else {
                        self.dirty.insert(id.clone());
                    }
                }
                "tool_call" | "tool_call_update" => {
                    self.acp_phase = "tool".into();
                    let old = self
                        .items
                        .iter()
                        .filter(|(_, row)| {
                            matches!(row["type"].as_str(), Some("agentMessage" | "reasoning"))
                        })
                        .map(|(id, _)| id.clone())
                        .collect::<Vec<_>>();
                    for id in old {
                        if let Some(mut row) = self.items.remove(&id) {
                            row["status"] = "completed".into();
                            out.push(event(t, "item/completed", json!({"item":row})));
                        }
                    }
                    let id = format!(
                        "{}-tool-{}",
                        t.turn,
                        u["toolCallId"].as_str().unwrap_or("unknown")
                    );
                    let row=self.items.entry(id.clone()).or_insert(json!({"id":id,"type":"claudeToolCall","tool":acp_tool(u),"arguments":tool_args(&u["rawInput"]),"status":"inProgress"}));
                    update_acp_tool(row, u);
                    if let Some(c) = u.get("rawInput") {
                        row["arguments"] = tool_args(c);
                    }
                    if let Some(c) = u.get("content") {
                        row["output"] = blocks_text(c).into();
                        row["contentItems"] = c.clone();
                    }
                    if let Some(status) = acp_tool_status(u) {
                        row["status"] = status.into();
                    }
                    item = Some(row.clone());
                }
                "plan" => {
                    item = Some(
                        json!({"id":format!("{}-plan",t.turn),"type":"executionPlan","text":u["entries"].to_string(),"status":"inProgress"}),
                    )
                }
                "usage_update" => {
                    let usage = self.observe_acp_usage(u);
                    out.push(event(
                        t,
                        "thread/tokenUsage/updated",
                        json!({"tokenUsage":usage}),
                    ));
                }
                _ => {}
            }
        }
        if let Some(mut row) = item {
            row["turnId"] = t.turn.clone().into();
            let method = if matches!(
                row["status"].as_str(),
                Some("completed" | "failed" | "interrupted")
            ) {
                self.items.remove(row["id"].as_str().unwrap_or(""));
                self.dirty.remove(row["id"].as_str().unwrap_or(""));
                "item/completed"
            } else {
                "item/started"
            };
            out.push(event(t, method, json!({"item":row})));
        }
        if let Some(delta) = delta {
            out.push(delta);
        }
        out
    }
    pub(super) fn checkpoint(&mut self, t: &Turn) -> Vec<Value> {
        self.dirty
            .drain()
            .filter_map(|id| {
                self.items
                    .get(&id)
                    .map(|row| event(t, "item/updated", json!({"item":row})))
            })
            .collect()
    }
    pub(super) fn finish(&mut self, interrupted: bool) -> Vec<Value> {
        self.dirty.clear();
        self.items
            .drain()
            .map(|(_, mut v)| {
                v["status"] = if interrupted {
                    "interrupted"
                } else {
                    "completed"
                }
                .into();
                v
            })
            .collect()
    }
    pub(super) fn add_usage(&mut self, native: &Value) -> Value {
        let last = crate::usage_tokens::pi_usage(native);
        if !self.usage.is_object() {
            self.usage = json!({"total":{},"last":{},"contextTokens":null,"cumulative":false});
        }
        if crate::usage_tokens::pi_context(&last).is_none() {
            return self.usage.clone();
        }
        self.observe_pi_usage(native);
        for key in [
            "inputTokens",
            "outputTokens",
            "cachedInputTokens",
            "cacheWriteInputTokens",
            "totalTokens",
        ] {
            self.usage["total"][key] = json!(self.usage["total"][key]
                .as_u64()
                .unwrap_or(0)
                .saturating_add(last[key].as_u64().unwrap_or(0)));
        }
        if self.priced {
            if let Some(cost) = native["cost"]["total"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
            {
                self.usage["costUsd"] = json!(self.usage["costUsd"].as_f64().unwrap_or(0.0) + cost);
            }
        }
        self.usage.clone()
    }
    pub(super) fn observe_acp_usage(&mut self, native: &Value) -> Value {
        let used = crate::usage_tokens::count(&native["used"]);
        self.model_context_window =
            crate::model_context::positive(&native["size"]).or(self.model_context_window);
        json!({"total":{},"last":{},"contextTokens":used,"modelContextWindow":self.model_context_window,"cumulative":false})
    }
    pub(super) fn observe_pi_usage(&mut self, native: &Value) -> bool {
        let last = crate::usage_tokens::pi_usage(native);
        let context = crate::usage_tokens::pi_context(&last);
        let Some(context) = context else {
            return false;
        };
        if !self.usage.is_object() {
            self.usage = json!({"total":{},"cumulative":false});
        }
        let changed = self.usage["last"] != last || self.usage["contextTokens"] != json!(context);
        self.usage["last"] = last;
        self.usage["contextTokens"] = json!(context);
        if let Some(capacity) = self.model_context_window {
            self.usage["modelContextWindow"] = json!(capacity);
        }
        changed
    }
}
pub(super) fn apply_pi_result(row: &mut Value, result: &Value, is_error: bool) {
    row["output"] = blocks_text(&result["content"]).into();
    row["contentItems"] = result["content"].clone();
    let structured = &result["structuredContent"];
    if let Some(code) = structured["exit_code"].as_i64() {
        row["exitCode"] = code.into();
    }
    if let Some(seconds) = structured["wall_time_seconds"]
        .as_f64()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
    {
        row["durationMs"] = json!((seconds * 1000.0).round() as u64);
    }
    if !is_error {
        if let (Some(path), Some(patch)) = (
            row["arguments"]["file_path"].as_str(),
            result["details"]["patch"].as_str(),
        ) {
            if let Some(change) = crate::file_changes::patch_change(path, patch) {
                row["changes"] = json!([change]);
                crate::file_changes::summarize(row);
            }
        }
    }
    if result["details"]["supercodeFileChange"].is_object() {
        row["fileSnapshot"] = result["details"]["supercodeFileChange"].clone();
    }
}
pub(super) fn acp_tool(v: &Value) -> &str {
    if v["rawInput"]["subagent_type"].is_string() && v["rawInput"]["prompt"].is_string() {
        return "Task";
    }
    match v["kind"].as_str() {
        Some("read") => "Read",
        Some("edit") => "Edit",
        Some("execute") => "Bash",
        Some("search") => "Grep",
        _ => v["title"].as_str().unwrap_or("Tool"),
    }
}
pub(super) fn update_acp_tool(row: &mut Value, update: &Value) {
    // OpenCode replaces a Task's title with its description after approval.
    // Keep the native tool identity so progress and completion still reach its row.
    if matches!(row["tool"].as_str(), Some("Task" | "task")) {
        return;
    }
    if update["kind"].is_string() || update["rawInput"]["subagent_type"].is_string() {
        row["tool"] = acp_tool(update).into();
    }
}
pub(super) fn acp_tool_status(update: &Value) -> Option<&str> {
    match update["status"].as_str() {
        Some("failed") if blocks_text(&update["content"]).trim() == "Task cancelled" => {
            Some("interrupted")
        }
        Some(status @ ("completed" | "failed")) => Some(status),
        _ => None,
    }
}
pub(super) fn blocks_text(v: &Value) -> String {
    let text = v
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|b| b["text"].as_str().or_else(|| b["content"]["text"].as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    protocol::bounded(&text, protocol::TEXT_LIMIT)
}
pub(super) fn pi_tool(name: &str) -> &str {
    match name {
        "read" => "Read",
        "bash" => "Bash",
        "write" => "Write",
        "edit" => "Edit",
        "grep" => "Grep",
        "find" | "ls" => "Glob",
        _ => name,
    }
}
pub(super) fn tool_args(v: &Value) -> Value {
    let mut args = v.clone();
    if let Some(path) = v["file_path"]
        .as_str()
        .or(v["filePath"].as_str())
        .or(v["path"].as_str())
    {
        args["file_path"] = path.into();
    }
    args
}
