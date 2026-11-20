//! Explicit per-turn speed selection. Reasoning effort remains independent.
use serde_json::{json, Value};
use tauri::AppHandle;

pub(crate) fn tier<'a>(
    agent: &str,
    official: bool,
    value: Option<&'a str>,
) -> Result<Option<&'a str>, String> {
    if value.is_some_and(|value| !matches!(value, "default" | "fast" | "ultrafast")) {
        return Err("速度档位无效".into());
    }
    if agent != "codex" || !official {
        if value.is_some() {
            return Err("速度设置仅适用于 Codex 官方账号，请重新选择模型".into());
        }
        return Ok(None);
    }
    // Always reset a preceding Fast turn when the user returns to Standard.
    Ok(Some(value.unwrap_or("default")))
}

#[cfg(test)]
pub(crate) fn supports_fast(model: &Value) -> bool {
    supports(model, "fast")
}

pub(crate) fn supports(model: &Value, tier: &str) -> bool {
    if tier == "default" {
        return true;
    }
    model["serviceTiers"].as_array().is_some_and(|tiers| {
        tiers
            .iter()
            .any(|option| option["id"] == tier || tier == "fast" && option["id"] == "priority")
    }) || tier == "fast"
        && model["additionalSpeedTiers"]
            .as_array()
            .is_some_and(|tiers| tiers.iter().any(|tier| tier == "fast"))
}

pub(crate) fn find_model<'a>(catalog: &'a Value, model: &str) -> Option<&'a Value> {
    catalog["data"]
        .as_array()?
        .iter()
        .find(|row| row["model"] == model || row["id"] == model)
}

// Capability reads happen only when the user opens an official model picker.
// They start no thread, generation, tools or approval flow.
#[tauri::command]
pub async fn get_codex_model_speeds(
    connection_id: String,
    model: String,
    app: AppHandle,
) -> Result<Value, String> {
    if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
        return Err("模型标识无效".into());
    }
    let route = crate::chat_connection::available_route(&app, "codex", None, Some(&connection_id))
        .await?
        .ok_or("请先登录官方账号")?;
    let official = route.config["official"] == true
        || route
            .profile
            .as_ref()
            .is_some_and(crate::ccswitch::Profile::is_official);
    if !official {
        return Err("速度能力仅适用于 Codex 官方账号".into());
    }
    let mut cursor = Value::Null;
    for _ in 0..10 {
        let catalog = crate::accounts::read_codex_value(
            &app,
            route.config["accountId"].as_str(),
            "model/list",
            json!({"cursor":cursor,"limit":100}),
        )
        .await?;
        if let Some(row) = find_model(&catalog, &model) {
            let tiers: Vec<_> = ["default", "fast", "ultrafast"]
                .into_iter()
                .filter(|tier| supports(row, tier))
                .collect();
            return Ok(json!({"tiers":tiers}));
        }
        cursor = catalog["nextCursor"].clone();
        if cursor.as_str().is_none_or(str::is_empty) {
            break;
        }
    }
    Err(missing_model(&model))
}

pub(crate) fn missing_model(model: &str) -> String {
    format!("当前接入的 Codex 未返回模型 {model}，请在 Agent 设置中更新 Codex 后重试")
}

pub(crate) fn apply(turn: &mut Value, tier: Option<&str>) {
    if let Some(tier) = tier {
        turn["serviceTier"] = json!(tier);
    }
}

pub(crate) fn validate_steering(signature: &str, tier: Option<&str>) -> Result<(), String> {
    let settings: Value = serde_json::from_str(signature).map_err(|_| "活动任务配置无效")?;
    if settings[0] != tier.unwrap_or("default") {
        return Err("引导消息的速度与当前任务不同，请保留排队发送".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speed_and_reasoning_are_forwarded_independently_and_standard_resets_fast() {
        let mut turn = json!({"model":"model","effort":"ultra"});
        apply(&mut turn, tier("codex", true, Some("fast")).unwrap());
        assert_eq!(turn["serviceTier"], "fast");
        assert_eq!(turn["effort"], "ultra");
        apply(&mut turn, tier("codex", true, Some("ultrafast")).unwrap());
        assert_eq!(turn["serviceTier"], "ultrafast");
        assert_eq!(turn["effort"], "ultra");
        apply(&mut turn, tier("codex", true, None).unwrap());
        assert_eq!(turn["serviceTier"], "default");
        let mut api = json!({});
        apply(&mut api, tier("codex", false, None).unwrap());
        assert!(api.get("serviceTier").is_none());
        assert!(tier("codex", false, Some("fast")).is_err());
        assert!(tier("claude", true, Some("fast")).is_err());
        assert!(tier("codex", true, Some("arbitrary")).is_err());
    }
    #[test]
    fn uses_only_advertised_native_capabilities() {
        assert!(supports_fast(&json!({"serviceTiers":[{"id":"priority"}]})));
        assert!(supports_fast(&json!({"serviceTiers":[{"id":"fast"}]})));
        assert!(supports_fast(&json!({"additionalSpeedTiers":["fast"]})));
        assert!(!supports_fast(
            &json!({"serviceTiers":[{"id":"ultrafast"}]})
        ));
        assert!(!supports_fast(&json!({})));
        assert!(supports(
            &json!({"serviceTiers":[{"id":"ultrafast"}]}),
            "ultrafast"
        ));
        assert!(!supports(
            &json!({"serviceTiers":[{"id":"priority"}]}),
            "ultrafast"
        ));
    }
    #[test]
    fn gpt_62_sol_and_astra_can_advertise_different_account_speed_permissions() {
        let catalog = json!({"data":[
            {"id":"gpt-6.2-sol","model":"gpt-6.2-sol","serviceTiers":[{"id":"priority"},{"id":"ultrafast"}]},
            {"id":"gpt-6.2-astra","model":"gpt-6.2-astra","serviceTiers":[{"id":"priority"}]}
        ]});
        assert!(supports(
            find_model(&catalog, "gpt-6.2-sol").unwrap(),
            "ultrafast"
        ));
        assert!(!supports(
            find_model(&catalog, "gpt-6.2-astra").unwrap(),
            "ultrafast"
        ));
        assert!(find_model(&catalog, "gpt-6.1-sol").is_none());
    }
    #[test]
    fn steering_cannot_silently_drop_a_different_speed() {
        let signature = json!(["fast", "ask", "ultra"]).to_string();
        assert!(validate_steering(&signature, Some("fast")).is_ok());
        assert!(validate_steering(&signature, None).is_err());
        assert!(crate::client_features::validate_steering_settings(
            &signature,
            "ask",
            Some("ultra")
        )
        .is_ok());
    }
}
