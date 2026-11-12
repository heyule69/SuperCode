//! Claude role mappings belong to the selected SuperCode connection, not the CLI's global settings.
use crate::{ccswitch::Profile, AppState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

pub(crate) const MODEL_ENV: &[&str] = &[
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL_FORCE",
    "ANTHROPIC_REASONING_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
];

#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Follow,
    Mapped,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Mapping {
    pub mode: Mode,
    pub sonnet: String,
    pub opus: String,
    pub haiku: String,
    pub fable: String,
    pub subagent: String,
    pub force_subagent: bool,
}

pub(crate) fn mapping(config: &Value) -> Result<Mapping, String> {
    if let Some(saved) = config.get("claudeModels") {
        return serde_json::from_value(saved.clone()).map_err(|_| "Claude 模型映射格式无效".into());
    }
    let env = &config["env"];
    let get = |key: &str| env[key].as_str().unwrap_or("").to_owned();
    let mut result = Mapping {
        sonnet: get(MODEL_ENV[0]),
        opus: get(MODEL_ENV[1]),
        haiku: get(MODEL_ENV[2]),
        fable: get(MODEL_ENV[3]),
        subagent: get(MODEL_ENV[4]),
        force_subagent: matches!(
            env["CLAUDE_CODE_SUBAGENT_MODEL_FORCE"].as_str(),
            Some("1" | "true")
        ),
        ..Mapping::default()
    };
    if result.haiku.is_empty() {
        result.haiku = get("ANTHROPIC_SMALL_FAST_MODEL");
    }
    let roles = !result.sonnet.is_empty()
        || !result.opus.is_empty()
        || !result.haiku.is_empty()
        || !result.fable.is_empty()
        || !get("ANTHROPIC_REASONING_MODEL").is_empty();
    // Older SuperCode connections wrote the saved main model as a static child
    // default. Treat that generated value as follow-main, while retaining CC Switch's explicit mappings.
    let generated = !result.force_subagent
        && config["source"] != "ccswitch"
        && !roles
        && config["model"].as_str() == Some(result.subagent.as_str());
    if roles || result.force_subagent || !result.subagent.is_empty() && !generated {
        result.mode = Mode::Mapped;
    } else {
        result.subagent.clear();
    }
    Ok(result)
}

fn validate(mut value: Mapping) -> Result<Mapping, String> {
    for model in [
        &mut value.sonnet,
        &mut value.opus,
        &mut value.haiku,
        &mut value.fable,
        &mut value.subagent,
    ] {
        *model = model.trim().to_owned();
        if model.len() > 200 || model.chars().any(char::is_control) {
            return Err("映射模型 ID 长度不能超过 200 字节，且不能包含控制字符".into());
        }
        if matches!(
            model.to_ascii_lowercase().as_str(),
            "sonnet" | "opus" | "haiku" | "fable" | "inherit" | "opusplan"
        ) {
            return Err("请填写供应商的真实模型 ID，不要将角色名映射回自身".into());
        }
    }
    Ok(value)
}

pub(crate) fn apply(config: &mut Value, selected: Option<&str>) -> Result<(), String> {
    if config["official"] == true {
        return Ok(());
    }
    let Some(selected) = selected.filter(|model| !model.is_empty()) else {
        return Ok(());
    };
    let settings = mapping(config)?;
    config["claudeModels"] = serde_json::to_value(&settings).map_err(|error| error.to_string())?;
    if !config["env"].is_object() {
        config["env"] = json!({});
    }
    let env = config["env"].as_object_mut().unwrap();
    let values = [
        &settings.sonnet,
        &settings.opus,
        &settings.haiku,
        &settings.fable,
        &settings.subagent,
    ];
    for (key, value) in MODEL_ENV[..5].iter().zip(values) {
        env.insert(
            (*key).into(),
            json!(if settings.mode == Mode::Follow || value.is_empty() {
                selected
            } else {
                value
            }),
        );
    }
    env.insert("ANTHROPIC_MODEL".into(), json!(selected));
    if settings.mode == Mode::Follow {
        env.insert("CLAUDE_CODE_SUBAGENT_MODEL_FORCE".into(), json!("1"));
        for key in ["ANTHROPIC_REASONING_MODEL", "ANTHROPIC_SMALL_FAST_MODEL"] {
            env.insert(key.into(), json!(selected));
        }
    } else if settings.force_subagent {
        env.insert("CLAUDE_CODE_SUBAGENT_MODEL_FORCE".into(), json!("1"));
    } else {
        env.remove("CLAUDE_CODE_SUBAGENT_MODEL_FORCE");
    }
    Ok(())
}

// Preserve both explicit host settings and legacy imported env values when
// editing the same route. A different endpoint starts with follow-main settings.
pub(crate) fn preserve(config: &mut Value, previous: &Profile) -> Result<(), String> {
    config["claudeModels"] =
        serde_json::to_value(mapping(&previous.config)?).map_err(|error| error.to_string())?;
    if config["env"].is_object() {
        for key in MODEL_ENV {
            if let Some(value) = previous.config["env"][*key].as_str() {
                config["env"][*key] = json!(value);
            }
        }
    }
    Ok(())
}

fn profile(app: &AppHandle, id: &str) -> Result<Profile, String> {
    let profile = app
        .state::<AppState>()
        .store
        .profile("claude", id)?
        .ok_or("Claude 连接不存在")?;
    if profile.is_official() {
        return Err("官方账号沿用 Claude 的原生模型选择规则".into());
    }
    Ok(profile)
}

#[tauri::command]
pub fn get_claude_model_mapping(id: String, app: AppHandle) -> Result<Mapping, String> {
    mapping(&profile(&app, &id)?.config)
}

#[tauri::command]
pub fn save_claude_model_mapping(
    id: String,
    mapping: Mapping,
    app: AppHandle,
) -> Result<(), String> {
    let mapping = validate(mapping)?;
    let previous = profile(&app, &id)?;
    let mut next = previous.clone();
    next.config["claudeModels"] =
        serde_json::to_value(mapping).map_err(|error| error.to_string())?;
    app.state::<AppState>()
        .store
        .save_profile(&next, Some(&previous))?;
    let _ = app.emit("workspace-updated", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_child_defaults_follow_the_selected_chat_model_for_all_role_aliases() {
        let original = json!({"model":"auto/old","source":"magpie","env":{"ANTHROPIC_MODEL":"auto/old","CLAUDE_CODE_SUBAGENT_MODEL":"auto/old"}});
        let mut config = original.clone();
        assert!(mapping(&config).unwrap().mode == Mode::Follow);
        apply(&mut config, Some("cursor/chosen")).unwrap();
        for key in &MODEL_ENV[..5] {
            assert_eq!(config["env"][*key], "cursor/chosen");
        }
        assert_eq!(config["env"]["CLAUDE_CODE_SUBAGENT_MODEL_FORCE"], "1");
        assert_eq!(original["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "auto/old");
    }
    #[test]
    fn explicit_mappings_survive_main_model_changes_and_blank_roles_follow_main() {
        let mut config = json!({"claudeModels":{"mode":"mapped","opus":"v/large","haiku":"v/small","subagent":"v/child"}});
        apply(&mut config, Some("v/main")).unwrap();
        assert_eq!(config["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "v/large");
        assert_eq!(config["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "v/small");
        assert_eq!(config["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "v/child");
        assert_eq!(config["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "v/main");
        assert!(!config["env"]
            .as_object()
            .unwrap()
            .contains_key("CLAUDE_CODE_SUBAGENT_MODEL_FORCE"));
        apply(&mut config, Some("v/next-main")).unwrap();
        assert_eq!(config["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "v/child");
        assert_eq!(
            config["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"],
            "v/next-main"
        );
    }
    #[test]
    fn imported_explicit_child_and_native_accounts_keep_their_intent() {
        let imported = json!({"source":"ccswitch","model":"v/main","env":{"CLAUDE_CODE_SUBAGENT_MODEL":"v/main"}});
        assert!(mapping(&imported).unwrap().mode == Mode::Mapped);
        let mut official = json!({"official":true,"env":{}});
        let original = official.clone();
        apply(&mut official, Some("opus")).unwrap();
        assert_eq!(official, original);
        assert!(mapping(&json!({"claudeModels":{"mode":"invalid"}})).is_err());
    }

    #[test]
    fn cc_switch_child_roles_and_force_setting_survive_import_and_connection_edits() {
        let raw = json!({"model":"opus","env":{"ANTHROPIC_BASE_URL":"https://fixture.example/v1","ANTHROPIC_AUTH_TOKEN":"fixture-key",
            "ANTHROPIC_DEFAULT_OPUS_MODEL":"v/large","ANTHROPIC_DEFAULT_SONNET_MODEL":"v/medium","ANTHROPIC_DEFAULT_HAIKU_MODEL":"v/small",
            "CLAUDE_CODE_SUBAGENT_MODEL":"v/child","CLAUDE_CODE_SUBAGENT_MODEL_FORCE":"1"}});
        let config = crate::ccswitch::sanitize_provider("claude", "fixture", raw).unwrap();
        assert_eq!(config["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "v/child");
        let previous = Profile {
            id: "fixture".into(),
            agent: "claude".into(),
            name: "Fixture".into(),
            config,
        };
        let edited =
            crate::providers::make_profile(crate::providers::settings(&previous), Some(&previous))
                .unwrap();
        let mut config = edited.config;
        assert_eq!(config["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "v/small");
        apply(&mut config, Some("v/new-main")).unwrap();
        assert_eq!(config["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "v/large");
        assert_eq!(config["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "v/child");
        assert_eq!(config["env"]["CLAUDE_CODE_SUBAGENT_MODEL_FORCE"], "1");
        let mut changed = crate::providers::settings(&previous);
        changed.base_url = "https://different.example/v1".into();
        let changed = crate::providers::make_profile(changed, Some(&previous)).unwrap();
        assert!(mapping(&changed.config).unwrap().mode == Mode::Follow);
    }

    #[test]
    fn editing_the_saved_main_keeps_generated_follow_mode_instead_of_fixing_the_old_child() {
        let settings = serde_json::from_value(json!({"name":"Fixture","agent":"claude","providerId":"custom","plan":"anthropic","protocol":"anthropic","baseUrl":"https://fixture.example/v1","model":"v/old","models":[],"apiKey":"fixture-key"})).unwrap();
        let previous = crate::providers::make_profile(settings, None).unwrap();
        let mut settings = crate::providers::settings(&previous);
        settings.model = "v/new".into();
        let edited = crate::providers::make_profile(settings, Some(&previous)).unwrap();
        let mut config = edited.config;
        assert!(mapping(&config).unwrap().mode == Mode::Follow);
        apply(&mut config, Some("v/chosen-in-chat")).unwrap();
        apply(&mut config, Some("v/another-chat-choice")).unwrap();
        assert_eq!(
            config["env"]["CLAUDE_CODE_SUBAGENT_MODEL"],
            "v/another-chat-choice"
        );
        assert_eq!(
            config["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
            "v/another-chat-choice"
        );
    }
    #[test]
    fn model_ids_are_validated_instead_of_leaking_invalid_aliases_or_controls() {
        let mut value = Mapping::default();
        value.sonnet = "sonnet".into();
        assert!(validate(value.clone()).is_err());
        value.sonnet = "bad\nid".into();
        assert!(validate(value.clone()).is_err());
        value.sonnet = " v/real ".into();
        assert_eq!(validate(value).unwrap().sonnet, "v/real");
    }
}
