use serde_json::{json, Value};

pub const TEXT_LIMIT: usize = 128 * 1024;

pub fn bounded(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… 输出过长，已截断", &text[..end])
}

pub fn item_message(item: &Value) -> Option<(String, String, String, Value)> {
    let kind = item.get("type")?.as_str()?;
    let (role, text) = match kind {
        "agentMessage" => ("assistant", item["text"].as_str().unwrap_or("").to_owned()),
        "commandExecution" => (
            "tool",
            format!(
                "{}\n{}",
                item["command"].as_str().unwrap_or("命令"),
                item["aggregatedOutput"].as_str().unwrap_or("")
            ),
        ),
        "fileChange" => (
            "tool",
            item["changes"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c["path"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
        ),
        "mcpToolCall" | "dynamicToolCall" => (
            "tool",
            format!(
                "{} / {}",
                item["server"].as_str().unwrap_or("工具"),
                item["tool"].as_str().unwrap_or("执行")
            ),
        ),
        "plan" => ("assistant", item["text"].as_str().unwrap_or("").to_owned()),
        "reasoning" => ("activity", reasoning_text(item)),
        "claudeToolCall" => ("tool", item["output"].as_str().unwrap_or("").to_owned()),
        "executionPlan" | "runMarker" => {
            ("activity", item["text"].as_str().unwrap_or("").to_owned())
        }
        "webSearch" => ("tool", item["query"].as_str().unwrap_or("").to_owned()),
        "imageView" => ("tool", item["path"].as_str().unwrap_or("").to_owned()),
        "collabAgentToolCall"
        | "collabToolCall"
        | "contextCompaction"
        | "enteredReviewMode"
        | "exitedReviewMode" => (
            "tool",
            item["prompt"]
                .as_str()
                .or_else(|| item["review"].as_str())
                .unwrap_or("")
                .to_owned(),
        ),
        _ => return None,
    };
    // Avoid duplicating large command output inside both text and JSON metadata.
    let mut data = item.clone();
    if let Some(object) = data.as_object_mut() {
        object.remove("aggregatedOutput");
        object.remove("output");
        object.remove("summary");
        object.remove("content");
    }
    // Media references are small durable metadata. Reserve room before bounding
    // verbose tool arguments/results, so history does not lose its screenshots.
    let media = data
        .as_object_mut()
        .and_then(|object| object.remove("media"));
    let mut media_budget = 16 * 1024;
    let media_limited = media
        .as_ref()
        .map(|value| bounded_value(value, &mut media_budget, 0));
    let mut budget = TEXT_LIMIT - (16 * 1024 - media_budget);
    let mut data_limited = bounded_value(&data, &mut budget, 0);
    if let Some(media) = media_limited {
        data_limited["media"] = media;
    }
    for key in ["id", "type", "status", "tool", "server", "turnId"] {
        if let Some(value) = data.get(key) {
            data_limited[key] = value
                .as_str()
                .map(|s| json!(bounded(s, 4096)))
                .unwrap_or_else(|| value.clone());
        }
    }
    Some((
        role.into(),
        bounded(&text, TEXT_LIMIT),
        kind.into(),
        data_limited,
    ))
}

fn bounded_value(value: &Value, remaining: &mut usize, depth: usize) -> Value {
    if depth > 12 || *remaining == 0 {
        return json!("… 已省略过长详情");
    }
    match value {
        Value::String(text) => {
            let limit = (*remaining).min(32 * 1024);
            let result = bounded(text, limit);
            *remaining = remaining.saturating_sub(result.len());
            json!(result)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(100)
                .map(|v| bounded_value(v, remaining, depth + 1))
                .collect(),
        ),
        Value::Object(values) => {
            if matches!(
                values.get("type").and_then(Value::as_str),
                Some("image" | "audio" | "image_url")
            ) {
                return json!({"type":"attachment","note":"附件二进制未加载"});
            }
            Value::Object(
                values
                    .iter()
                    .filter(|(k, _)| {
                        !matches!(
                            k.as_str(),
                            "signature" | "encryptedContent" | "encrypted_content"
                        )
                    })
                    .take(100)
                    .map(|(k, v)| (k.clone(), bounded_value(v, remaining, depth + 1)))
                    .collect(),
            )
        }
        _ => value.clone(),
    }
}

pub fn reasoning_text(item: &Value) -> String {
    let parts = |key: &str| {
        item[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default()
    };
    let summary = parts("summary");
    if summary.is_empty() {
        parts("content")
    } else {
        summary
    }
}

/// Cache streamed fragments without a SQLite write per token. One active turn,
/// at most 200 outstanding items and 2 MiB of serialized data.
#[derive(Default)]
pub struct ActivityBuffer {
    native: String,
    items: Vec<Value>,
}

fn append(item: &mut Value, key: &str, delta: &str) {
    item[key] = json!(bounded(
        &format!("{}{}", item[key].as_str().unwrap_or(""), delta),
        TEXT_LIMIT
    ));
}

fn append_part(item: &mut Value, key: &str, index: usize, delta: &str) {
    if index >= 64 {
        return;
    }
    if !item[key].is_array() {
        item[key] = json!([]);
    }
    let parts = item[key].as_array_mut().unwrap();
    while parts.len() <= index {
        parts.push(json!(""));
    }
    let old = parts[index].as_str().unwrap_or("");
    parts[index] = json!(bounded(&format!("{old}{delta}"), TEXT_LIMIT / 64));
}

impl ActivityBuffer {
    pub fn snapshot(&self, native: &str) -> Vec<Value> {
        if self.native == native {
            self.items.clone()
        } else {
            vec![]
        }
    }
    pub fn update(&mut self, event: &Value) -> Option<Value> {
        let p = &event["params"];
        let native = p["threadId"].as_str()?;
        if self.native != native {
            self.native = native.into();
            self.items.clear();
        }
        let method = event["method"].as_str()?;
        if method == "turn/started" {
            self.items.clear();
            let item = json!({"id":format!("run-{}",p["turn"]["id"].as_str().unwrap_or("")),"type":"runMarker","status":"inProgress","startedAt":millis(),"turnId":p["turn"]["id"]});
            self.replace(item.clone());
            return Some(item);
        }
        if method == "turn/plan/updated" {
            let item = json!({"id":format!("plan-{}",p["turnId"].as_str().unwrap_or("")),"type":"executionPlan","plan":p["plan"],"text":p["explanation"],"status":"inProgress","turnId":p["turnId"]});
            self.replace(item.clone());
            return Some(item);
        }
        if matches!(method, "item/started" | "item/updated" | "item/completed") {
            let mut item = p["item"].clone();
            let id = item["id"].as_str()?;
            if item_message(&item).is_none() {
                return None;
            }
            let previous = self.items.iter().find(|i| i["id"] == id);
            let mut merged = previous.cloned().unwrap_or_else(|| json!({}));
            for (key, value) in item.as_object()? {
                merged[key] = value.clone();
            }
            // Empty final reasoning fields must not erase summaries already streamed.
            if item["type"] == "reasoning" {
                for key in ["summary", "content"] {
                    if item[key].as_array().is_none_or(|a| a.is_empty()) {
                        if let Some(old) = previous {
                            merged[key] = old[key].clone();
                        }
                    }
                }
            }
            item = merged;
            item["turnId"] = p["turnId"].clone();
            if item["startedAt"].is_null() {
                item["startedAt"] = json!(millis());
            }
            if method == "item/completed" && item["durationMs"].is_null() {
                item["durationMs"] =
                    json!(millis().saturating_sub(item["startedAt"].as_u64().unwrap_or(millis())));
            }
            if item["status"].is_null() {
                item["status"] = json!(if method == "item/completed" {
                    "completed"
                } else {
                    "inProgress"
                });
            }
            if method == "item/completed" && item["status"] == "inProgress" {
                item["status"] = json!("completed");
            }
            self.replace(item.clone());
            if method == "item/completed" {
                self.items.retain(|i| i["id"] != item["id"]);
            }
            return Some(item);
        }
        let id = p["itemId"].as_str()?;
        let item = self.items.iter_mut().find(|i| i["id"] == id)?;
        let delta = p["delta"].as_str().unwrap_or("");
        match method {
            "item/agentMessage/delta" | "item/plan/delta" => append(item, "text", delta),
            "item/reasoning/summaryTextDelta" => append_part(
                item,
                "summary",
                p["summaryIndex"].as_u64().unwrap_or(0) as usize,
                delta,
            ),
            "item/reasoning/textDelta" => append_part(
                item,
                "content",
                p["contentIndex"].as_u64().unwrap_or(0) as usize,
                delta,
            ),
            "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" => {
                append(item, "aggregatedOutput", delta)
            }
            "item/claudeToolCall/inputDelta" => append(item, "inputText", delta),
            "item/mcpToolCall/progress" => append(
                item,
                "progress",
                &format!("{}\n", p["message"].as_str().unwrap_or("")),
            ),
            "item/commandExecution/terminalInteraction" => {
                append(item, "terminalInput", p["stdin"].as_str().unwrap_or(""))
            }
            _ => return None,
        }
        self.trim();
        None
    }
    fn replace(&mut self, item: Value) {
        if let Some(i) = self.items.iter_mut().find(|i| i["id"] == item["id"]) {
            *i = item;
        } else {
            self.items.push(item);
        }
        self.trim();
    }
    fn trim(&mut self) {
        while self.items.len() > 200
            || serde_json::to_vec(&self.items).map_or(0, |v| v.len()) > 2 * 1024 * 1024
        {
            self.items.remove(0);
        }
    }
    pub fn finish(&mut self, status: &str) -> Vec<Value> {
        self.items
            .drain(..)
            .map(|mut i| {
                if i["type"] == "runMarker" {
                    i["durationMs"] =
                        json!(millis().saturating_sub(i["startedAt"].as_u64().unwrap_or(millis())));
                }
                i["status"] = json!(if matches!(
                    i["type"].as_str(),
                    Some("executionPlan" | "runMarker")
                ) && status == "completed"
                {
                    "completed"
                } else if status == "failed" {
                    "failed"
                } else {
                    "interrupted"
                });
                i
            })
            .collect()
    }
}

fn millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn utf8_limits_do_not_split_chinese_or_emoji() {
        assert_eq!(bounded("你好🧋", 20), "你好🧋");
        let output = bounded("你好🧋", 7);
        assert!(output.starts_with("你好\n"));
        assert!(!output.contains('�'));
    }
    #[test]
    fn tool_events_preserve_status_without_output_duplication() {
        let (_,text,kind,data)=item_message(&json!({"type":"commandExecution","command":"git status","aggregatedOutput":"文件已修改","status":"completed"})).unwrap();
        assert!(text.contains("文件已修改"));
        assert_eq!(kind, "commandExecution");
        assert_eq!(data["status"], "completed");
        assert!(data.get("aggregatedOutput").is_none());
        assert!(item_message(&json!({"type":"unknownFutureEvent"})).is_none());
    }
    #[test]
    fn activity_buffer_preserves_streamed_summary_and_interrupted_output() {
        let mut buffer = ActivityBuffer::default();
        let event = |method: &str, params: Value| {
            let mut p = params;
            p["threadId"] = json!("n");
            p["turnId"] = json!("t");
            json!({"method":method,"params":p})
        };
        buffer.update(&event("turn/started", json!({"turn":{"id":"t"}})));
        buffer.update(&event(
            "item/started",
            json!({"item":{"id":"r","type":"reasoning","summary":[]}}),
        ));
        buffer.update(&event(
            "item/reasoning/summaryTextDelta",
            json!({"itemId":"r","summaryIndex":0,"delta":"检查 中文🧋"}),
        ));
        let final_item = buffer
            .update(&event(
                "item/completed",
                json!({"item":{"id":"r","type":"reasoning","summary":[]}}),
            ))
            .unwrap();
        assert_eq!(item_message(&final_item).unwrap().1, "检查 中文🧋");
        buffer.update(&event("item/started",json!({"item":{"id":"c","type":"commandExecution","command":"git status","status":"inProgress"}})));
        buffer.update(&event(
            "item/commandExecution/outputDelta",
            json!({"itemId":"c","delta":"部分输出"}),
        ));
        let snapshot = buffer.snapshot("n");
        assert!(snapshot
            .iter()
            .any(|i| i["id"] == "c" && i["aggregatedOutput"] == "部分输出"));
        assert!(buffer.snapshot("another-thread").is_empty());
        let unfinished = buffer.finish("interrupted");
        assert_eq!(unfinished.len(), 2);
        assert_eq!(unfinished[1]["status"], "interrupted");
        assert!(item_message(&unfinished[1]).unwrap().1.contains("部分输出"));
        assert!(buffer.finish("completed").is_empty());
    }
    #[test]
    fn activity_buffer_updates_inputs_without_duplicating_large_output() {
        let mut buffer = ActivityBuffer::default();
        let event = |method: &str, item: Value| json!({"method":method,"params":{"threadId":"n","turnId":"t","item":item}});
        buffer.update(&event("item/started",json!({"id":"tool","type":"claudeToolCall","tool":"Read","arguments":{},"status":"preparing"})));
        buffer.update(&json!({"method":"item/claudeToolCall/inputDelta","params":{"threadId":"n","itemId":"tool","delta":"{\"file_path\":\"中文.md\"}"}}));
        let complete = buffer.update(&event("item/completed",json!({"id":"tool","type":"claudeToolCall","output":"中文结果","status":"completed"}))).unwrap();
        let (_, text, _, data) = item_message(&complete).unwrap();
        assert_eq!(text, "中文结果");
        assert!(data.get("output").is_none());
        assert_eq!(data["tool"], "Read");
    }
}
