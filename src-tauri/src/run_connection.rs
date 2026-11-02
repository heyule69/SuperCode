//! Structured retry signals only. Silence is not evidence of a disconnected model.
use serde_json::{json, Value};

fn count(value: &Value) -> Option<u64> { value.as_u64().filter(|n| (1..=100_000).contains(n)) }
fn retry_state(at: u64, attempt: &Value, max: &Value, delay: &Value, kind: &str) -> Value {
    let mut state = json!({"status":"retrying","at":at,"kind":kind});
    if let Some(value) = count(attempt) { state["attempt"] = json!(value); }
    if let Some(value) = count(max) { state["maxAttempts"] = json!(value); }
    if let Some(delay) = delay.as_f64().filter(|n| n.is_finite() && (0.0..=86_400_000.0).contains(n)) {
        state["retryAt"] = json!(at.saturating_add(delay.ceil() as u64));
    }
    state
}

fn kind(status: Option<u64>, category: &str) -> &'static str {
    match (status, category) {
        (Some(401 | 403), _) | (_, "authentication_failed" | "oauth_org_not_allowed") => "auth",
        (Some(402), _) | (_, "billing_error") => "billing",
        (Some(429), _) | (_, "rate_limit") => "rateLimit",
        (Some(500..=599), _) | (_, "server_error" | "overloaded") => "server",
        (_, "network" | "responseStreamDisconnected" | "responseStreamConnectionFailed" | "httpConnectionFailed") => "network",
        _ => "unknown",
    }
}

// Codex app-server ErrorNotification.willRetry, not a guessed stderr message:
// https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/v2/notification.rs
pub fn codex(event: &Value, at: u64) -> Option<Value> {
    let p = &event["params"];
    if event.get("id").is_some() || event["method"] != "error" || p["willRetry"] != true { return None; }
    let thread = p["threadId"].as_str().filter(|s| !s.is_empty())?;
    let turn = p["turnId"].as_str().filter(|s| !s.is_empty())?;
    let error = &p["error"];
    let info = &error["codexErrorInfo"];
    let category = info.as_str().or_else(|| info.as_object().and_then(|o| o.keys().next().map(String::as_str))).unwrap_or("");
    let status = info[category]["httpStatusCode"].as_u64();
    let reconnect = error["message"].as_str().unwrap_or("").strip_prefix("Reconnecting... ")
        .or_else(|| error["message"].as_str().unwrap_or("").strip_prefix("Reconnecting… "));
    let attempts = reconnect.and_then(|s| s.trim().split_once('/')).and_then(|(a,b)| Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?)))
        .filter(|(a,b)| (1..=100_000).contains(a) && a <= b && *b <= 100_000);
    let (attempt, max) = attempts.map(|(a,b)| (json!(a), json!(b))).unwrap_or_default();
    let category = if category.is_empty() && reconnect.is_some() { "network" } else { category };
    let connection = retry_state(at, &attempt, &max, &Value::Null, kind(status, category));
    Some(json!({"method":"turn/connection/updated","params":{"threadId":thread,"turnId":turn,"appSessionId":p["appSessionId"],"connection":connection}}))
}

// https://code.claude.com/docs/en/headless#handle-api-retries
pub fn claude(message: &Value, at: u64) -> Option<Value> {
    if message["type"] != "system" || message["subtype"] != "api_retry" { return None; }
    Some(retry_state(at, &message["attempt"], &message["max_retries"], &message["retry_delay_ms"],
        kind(message["error_status"].as_u64(), message["error"].as_str().unwrap_or(""))))
}

pub fn pi(message: &Value, at: u64) -> Option<Value> {
    match message["type"].as_str()? {
        "auto_retry_start" => Some(retry_state(at, &message["attempt"], &message["maxAttempts"], &message["delayMs"], "unknown")),
        "auto_retry_end" => Some(json!({"status":if message["success"] == true {"restored"} else {"failed"},"at":at,"kind":"unknown"})),
        _ => None,
    }
}

pub fn progress(event: &Value) -> bool {
    let p = &event["params"];
    let method = event["method"].as_str().unwrap_or("");
    if method.ends_with("/delta") || method.ends_with("Delta") {
        return p["delta"].as_str().is_some_and(|s| !s.is_empty());
    }
    if matches!(method, "item/started" | "item/updated" | "item/completed") {
        return !p["item"]["id"].is_null() && p["item"]["type"] != "userMessage";
    }
    method == "item/mcpToolCall/progress" && p["message"].as_str().is_some_and(|s| !s.is_empty())
}

fn response(event: &Value) -> bool {
    let method = event["method"].as_str().unwrap_or("");
    (matches!(method, "item/agentMessage/delta" | "item/plan/delta" | "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta") && progress(event))
        || method == "item/started" && matches!(event["params"]["item"]["type"].as_str(), Some("commandExecution" | "claudeToolCall" | "mcpToolCall" | "dynamicToolCall" | "fileChange" | "webSearch"))
        || matches!(method, "item/completed" | "item/updated") && matches!(event["params"]["item"]["type"].as_str(), Some("agentMessage" | "plan" | "reasoning"))
            && ["text", "summary", "content"].iter().any(|key| event["params"]["item"][key].as_str().is_some_and(|s| !s.is_empty()) || event["params"]["item"][key].as_array().is_some_and(|a| a.iter().any(|v| v.as_str().is_some_and(|s| !s.is_empty()) || v["text"].as_str().is_some_and(|s| !s.is_empty()))))
}

pub fn finish(marker: &mut Value, status: &str, at: u64) {
    if marker["connection"]["status"] == "retrying" || status == "interrupted" && marker["connection"]["status"] == "failed" {
        marker["connection"]["status"] = json!(match status { "completed" => "restored", "failed" => "failed", _ => "stopped" });
        marker["connection"]["at"] = json!(at);
        marker["connection"].as_object_mut().unwrap().remove("retryAt");
    }
}

pub fn update(marker: &mut Value, event: &Value, at: u64) -> bool {
    let p = &event["params"];
    let turn = p["turnId"].as_str().or_else(|| p["turn"]["id"].as_str());
    if marker["turnId"].as_str() != turn || turn.is_none() || matches!(marker["status"].as_str(), Some("completed" | "failed" | "interrupted")) { return false; }
    if event["method"] == "turn/connection/updated" {
        let connection = &p["connection"];
        if !matches!(connection["status"].as_str(), Some("retrying" | "restored" | "failed" | "disconnected")) { return false; }
        marker["connection"] = connection.clone();
        return true;
    }
    if event["method"] == "turn/completed" {
        finish(marker, p["turn"]["status"].as_str().unwrap_or("failed"), at);
        return true;
    }
    if event["method"] == "error" && p["willRetry"] != true && marker["connection"]["status"] == "retrying" {
        finish(marker, "failed", at);
        return true;
    }
    if !progress(event) { return false; }
    marker["lastProgressAt"] = json!(at);
    if matches!(marker["connection"]["status"].as_str(), Some("retrying" | "failed")) && response(event) {
        marker["connection"]["status"] = json!("restored");
        marker["connection"]["at"] = json!(at);
        marker["connection"].as_object_mut().unwrap().remove("retryAt");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn marker() -> Value { json!({"id":"run-t","type":"runMarker","turnId":"t","status":"inProgress","startedAt":1000}) }
    fn retry() -> Value { json!({"method":"error","params":{"threadId":"n","turnId":"t","willRetry":true,"error":{"message":"Reconnecting... 2/5","codexErrorInfo":{"responseStreamDisconnected":{"httpStatusCode":503}},"additionalDetails":"do-not-persist-this-secret"}}}) }

    #[test]
    fn codex_recoverable_errors_keep_identity_counts_and_safe_categories() {
        let normalized = codex(&retry(), 2000).unwrap();
        assert_eq!(normalized["method"], "turn/connection/updated");
        assert_eq!(normalized["params"]["threadId"], "n");
        assert_eq!(normalized["params"]["turnId"], "t");
        assert_eq!(normalized["params"]["connection"], json!({"status":"retrying","at":2000,"kind":"server","attempt":2,"maxAttempts":5}));
        assert!(!normalized.to_string().contains("secret"));
        let mut raw = retry(); raw["params"]["willRetry"] = json!(false);
        assert!(codex(&raw, 2000).is_none());
        raw["params"]["willRetry"] = json!(true); raw["id"] = json!(1);
        assert!(codex(&raw, 2000).is_none());
    }

    #[test]
    fn missing_attempts_are_not_invented_and_http_auth_is_not_called_a_network_fault() {
        let mut raw = retry();
        raw["params"]["error"] = json!({"message":"opaque provider details","codexErrorInfo":{"responseStreamDisconnected":{"httpStatusCode":401}}});
        let state = codex(&raw, 2000).unwrap()["params"]["connection"].clone();
        assert_eq!(state["kind"], "auth");
        assert!(state.get("attempt").is_none());
        raw["params"]["error"] = json!({"message":"Reconnecting... 1/5"});
        assert_eq!(codex(&raw, 2000).unwrap()["params"]["connection"]["kind"], "network");
    }

    #[test]
    fn claude_delay_categories_and_pi_retry_end_are_structured() {
        let raw = json!({"type":"system","subtype":"api_retry","attempt":1,"max_retries":10,"retry_delay_ms":555.8,"error_status":429,"error":"rate_limit"});
        let state = claude(&raw, 1000).unwrap();
        assert_eq!(state["kind"], "rateLimit");
        assert_eq!(state["retryAt"], 1556);
        assert_eq!(state["maxAttempts"], 10);
        let pi_start = pi(&json!({"type":"auto_retry_start","attempt":1,"maxAttempts":3,"delayMs":2000,"errorMessage":"private"}), 1000).unwrap();
        assert_eq!(pi_start["retryAt"], 3000);
        assert!(!pi_start.to_string().contains("private"));
        assert_eq!(pi(&json!({"type":"auto_retry_end","success":true}), 4000).unwrap()["status"], "restored");
        assert_eq!(pi(&json!({"type":"auto_retry_end","success":false}), 4000).unwrap()["status"], "failed");
        assert!(pi(&json!({"type":"tool_execution_update","text":"reconnecting"}), 1000).is_none());
    }

    #[test]
    fn retry_details_have_finite_bounds() {
        let state = retry_state(1000, &json!(-2), &json!(999999999), &json!(1e30), "unknown");
        assert!(state.get("attempt").is_none());
        assert!(state.get("maxAttempts").is_none());
        assert!(state.get("retryAt").is_none());
    }

    #[test]
    fn local_tool_output_does_not_prove_reconnection_but_a_model_response_does() {
        let mut m = marker(); update(&mut m, &codex(&retry(), 2000).unwrap(), 2000);
        let local = json!({"method":"item/commandExecution/outputDelta","params":{"turnId":"t","delta":"local output"}});
        update(&mut m, &local, 3000);
        assert_eq!(m["connection"]["status"], "retrying");
        assert_eq!(m["lastProgressAt"], 3000);
        update(&mut m, &json!({"method":"item/updated","params":{"turnId":"t","item":{"id":"empty","type":"reasoning","summary":[""]}}}), 3500);
        assert_eq!(m["connection"]["status"], "retrying");
        let reply = json!({"method":"item/agentMessage/delta","params":{"turnId":"t","delta":"已恢复"}});
        update(&mut m, &reply, 4000);
        assert_eq!(m["connection"]["status"], "restored");
        assert_eq!(m["connection"]["at"], 4000);
        update(&mut m, &reply, 5000);
        assert_eq!(m["connection"]["at"], 4000);
    }

    #[test]
    fn stale_turns_terminal_history_and_status_noise_cannot_resurrect_retrying() {
        let mut m = marker(); let mut raw = codex(&retry(), 2000).unwrap(); raw["params"]["turnId"] = json!("old");
        assert!(!update(&mut m, &raw, 2000));
        assert!(!update(&mut m, &json!({"method":"thread/tokenUsage/updated","params":{"turnId":"t"}}), 2000));
        m["status"] = json!("completed"); raw["params"]["turnId"] = json!("t");
        assert!(!update(&mut m, &raw, 3000));
        assert!(m["connection"].is_null());
    }

    #[test]
    fn failure_stop_and_actual_process_disconnect_survive_finishing() {
        for (status, expected) in [("failed", "failed"), ("interrupted", "stopped"), ("completed", "restored")] {
            let mut m = marker(); update(&mut m, &codex(&retry(), 2000).unwrap(), 2000);
            finish(&mut m, status, 4000);
            assert_eq!(m["connection"]["status"], expected);
        }
        let mut m = marker(); update(&mut m, &codex(&retry(), 2000).unwrap(), 2000);
        update(&mut m, &json!({"method":"error","params":{"turnId":"t","willRetry":false}}), 3000);
        assert_eq!(m["connection"]["status"], "failed");
        finish(&mut m, "interrupted", 4000);
        assert_eq!(m["connection"]["status"], "stopped");
        update(&mut m, &json!({"method":"turn/connection/updated","params":{"turnId":"t","connection":{"status":"disconnected","at":5000,"kind":"network"}}}), 5000);
        finish(&mut m, "interrupted", 6000);
        assert_eq!(m["connection"]["status"], "disconnected");
    }

    #[test]
    fn activity_cache_and_persisted_marker_retain_latest_retry_state_without_extra_messages() {
        let mut buffer = crate::protocol::ActivityBuffer::default();
        buffer.update(&json!({"method":"turn/started","params":{"threadId":"n","turn":{"id":"t"}}}));
        let saved = buffer.update(&codex(&retry(), 2000).unwrap()).unwrap();
        let (_, _, kind, data) = crate::protocol::item_message(&saved).unwrap();
        assert_eq!(kind, "runMarker"); assert_eq!(data["connection"]["attempt"], 2);
        assert_eq!(buffer.snapshot("n").len(), 1);
        let finished = buffer.finish("failed");
        assert_eq!(finished[0]["connection"]["status"], "failed");
    }
}
