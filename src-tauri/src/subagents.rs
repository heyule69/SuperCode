use crate::{protocol, storage::Message, AppState};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, State};

pub fn references_child(data: &Value, child: &str) -> bool {
    data["receiverThreadIds"].as_array().is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(child)))
        || data["agentsStates"].as_object().is_some_and(|states| states.contains_key(child))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentHistory {
    messages: Vec<Message>,
    model: Option<String>,
    effort: Option<String>,
    status: Option<String>,
    turn_id: Option<String>,
    truncated: bool,
}

// Read existing native history on demand. This never resumes a turn or starts
// another agent. Keep the newest records within the same small display budget.
fn history(response: &Value, session: &str) -> SubagentHistory {
    let thread = &response["thread"];
    let turns = thread["turns"].as_array().map(Vec::as_slice).unwrap_or_default();
    let latest = turns.last();
    let mut messages = Vec::new();
    let mut bytes = 0;
    let mut truncated = false;
    'turns: for turn in turns.iter().rev() {
        let id = turn["id"].as_str().unwrap_or_default();
        let status = turn["status"].as_str().unwrap_or("inProgress");
        if let Some(items) = turn["items"].as_array() {
            for item in items.iter().rev() {
                let converted = if item["type"] == "userMessage" {
                    let text = item["content"].as_array().map(|content| content.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
                    Some(("user".into(), protocol::bounded(&text, protocol::TEXT_LIMIT), "userMessage".into(), json!({"type":"userMessage"})))
                } else { protocol::item_message(item) };
                if let Some((role, text, kind, mut data)) = converted {
                    if messages.len() >= 199 || bytes + text.len() + data.to_string().len() > 2 * 1024 * 1024 {
                        truncated = true;
                        break;
                    }
                    data["turnId"] = json!(id);
                    if data.get("status").is_none() {
                        data["status"] = json!(if status == "inProgress" { "inProgress" } else { "completed" });
                    }
                    bytes += text.len() + data.to_string().len();
                    messages.push(Message { seq: 0, id:item["id"].as_str().unwrap_or_default().into(), session_id:session.into(), role, text, kind, data, history_run:None });
                }
            }
        }
        let mut data = json!({"turnId":id,"status":status});
        if let Some(ms) = turn["durationMs"].as_u64() { data["durationMs"] = json!(ms); }
        messages.push(Message { seq:0, id:format!("run-{id}"), session_id:session.into(), role:"activity".into(), text:String::new(), kind:"runMarker".into(), data, history_run:None });
        if truncated || messages.len() >= 200 { truncated |= turns.first() != Some(turn); break 'turns; }
    }
    messages.reverse();
    for (index, message) in messages.iter_mut().enumerate() { message.seq = index as i64 + 1; }
    SubagentHistory {
        messages,
        model:thread["model"].as_str().map(str::to_owned),
        effort:thread["reasoningEffort"].as_str().map(str::to_owned),
        status:latest.and_then(|turn| turn["status"].as_str()).map(str::to_owned),
        turn_id:latest.and_then(|turn| turn["id"].as_str()).map(str::to_owned),
        truncated,
    }
}

#[tauri::command]
pub async fn get_subagent_messages(session_id: String, thread_id: String, app: AppHandle, state: State<'_, AppState>) -> Result<SubagentHistory, String> {
    let session = state.store.session(&session_id)?;
    if session.agent != "codex" { return Err("此引擎通过主会话返回子代理结果。".into()); }
    let live_reference = state.activity.existing(&session_id).is_some_and(|slot| {
        slot.lock().ok().is_some_and(|buffer| session.native_id.as_ref().is_some_and(|native| {
            buffer.snapshot(native).iter().any(|item| matches!(item["type"].as_str(), Some("collabAgentToolCall" | "collabToolCall")) && references_child(item, &thread_id))
        }))
    });
    if !live_reference && !state.store.has_subagent(&session_id, &thread_id)? {
        return Err("此子代理不属于当前会话。".into());
    }
    let client = state.runtime.get_for_session(&app, Some(&session_id)).await?;
    let response = client.request("thread/read", json!({"threadId":thread_id,"includeTurns":true})).await?;
    Ok(history(&response, &session_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_requires_an_actual_receiver() {
        assert!(references_child(&json!({"receiverThreadIds":["child"]}), "child"));
        assert!(references_child(&json!({"agentsStates":{"child":{"status":"completed"}}}), "child"));
        assert!(!references_child(&json!({"senderThreadId":"child","prompt":"child"}), "child"));
        assert!(!references_child(&Value::Null, "child"));
    }
    #[test]
    fn native_history_keeps_phases_and_child_lifecycle() {
        let result = history(&json!({"thread":{"model":"actual-child-model","turns":[{"id":"t","status":"completed","durationMs":17000,"items":[{"id":"a","type":"agentMessage","text":"进度","phase":"commentary"},{"id":"b","type":"agentMessage","text":"最终结果","phase":"final_answer"}]}]}}), "parent");
        assert_eq!(result.messages.len(),3);
        assert_eq!(result.messages[0].kind,"runMarker");
        assert_eq!(result.messages[0].data["durationMs"],17000);
        assert_eq!(result.messages[2].data["phase"],"final_answer");
        assert_eq!(result.status.as_deref(),Some("completed"));
        assert_eq!(result.model.as_deref(),Some("actual-child-model"));
    }
    #[test]
    fn history_is_bounded_and_does_not_invent_a_model_or_time() {
        let items:Vec<_> = (0..350).map(|i|json!({"id":format!("m{i}"),"type":"agentMessage","text":"内容"})).collect();
        let result=history(&json!({"thread":{"turns":[{"id":"t","status":"inProgress","items":items}]}}),"parent");
        assert_eq!(result.messages.len(),200);
        assert!(result.truncated);
        assert!(result.model.is_none());
        assert!(result.messages[0].data.get("durationMs").is_none());
        assert_eq!(result.messages.last().unwrap().id,"m349");
    }
}
