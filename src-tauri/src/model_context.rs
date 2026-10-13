//! Model-specific capacity metadata; runtime usage remains authoritative.
use serde_json::{json, Value};

fn catalog() -> &'static Value {
    static CATALOG: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../resources/model-contexts.json"))
            .expect("model context catalog")
    })
}

pub fn positive(value: &Value) -> Option<u64> {
    crate::usage_tokens::count(value).filter(|value| *value > 0 && *value <= 100_000_000)
}

pub fn metadata(row: &Value) -> Value {
    let capacity = [
        "contextWindow",
        "context_window",
        "context_length",
        "contextLength",
        "max_context_tokens",
        "max_input_tokens",
        "inputTokenLimit",
    ]
    .iter()
    .find_map(|key| positive(&row[*key]))
    .or_else(|| positive(&row["limit"]["context"]))
    .or_else(|| positive(&row["limits"]["context"]));
    let percent = positive(&row["effectiveContextWindowPercent"])
        .or_else(|| positive(&row["effective_context_window_percent"]))
        .filter(|percent| *percent <= 100);
    let mut result = json!({});
    if let Some(capacity) = capacity {
        result["contextWindow"] = json!(capacity);
    }
    if let Some(percent) = percent {
        result["effectiveContextWindowPercent"] = json!(percent);
    }
    result
}

pub fn published(model: &str) -> Option<u64> {
    let id = model.rsplit('/').next()?.to_ascii_lowercase();
    let (base, variant) = if let Some(base) = id.strip_suffix("[1m]") {
        (base, Some("1m"))
    } else if let Some(base) = id.strip_suffix("[256k]") {
        (base, Some("256k"))
    } else {
        (id.as_str(), None)
    };
    let entry = &catalog()["models"][base];
    match variant {
        Some("1m") => {
            positive(&entry["longContextWindow"]).or_else(|| positive(&entry["contextWindow"]))
        }
        Some("256k") => positive(&entry["contextWindow"]).map(|window| window.min(262_144)),
        _ => positive(&entry["contextWindow"]),
    }
}

fn declared(config: &Value, model: &str) -> Option<u64> {
    config["piProvider"]["models"]
        .as_array()
        .and_then(|models| models.iter().find(|row| row["id"] == model))
        .and_then(|row| positive(&row["contextWindow"]))
        .or_else(|| positive(&config["opencodeProvider"]["models"][model]["limit"]["context"]))
        .or_else(|| positive(&config["modelMetadata"][model]["contextWindow"]))
        .or_else(|| {
            let table = toml::from_str::<toml::Table>(config["config"].as_str()?).ok()?;
            if table.get("model")?.as_str()? != model {
                return None;
            }
            let value = table.get("model_context_window")?.as_integer()?;
            (value > 0 && value <= 100_000_000).then_some(value as u64)
        })
}

fn untagged(model: &str) -> &str {
    let lower = model.to_ascii_lowercase();
    if lower.ends_with("[1m]") {
        &model[..model.len() - 4]
    } else if lower.ends_with("[256k]") {
        &model[..model.len() - 6]
    } else {
        model
    }
}

fn endpoint_window(config: &Value, model: &str) -> Option<u64> {
    let model_base = config["piProvider"]["models"]
        .as_array()
        .and_then(|models| {
            models
                .iter()
                .find(|row| row["id"] == model || row["id"] == untagged(model))
        })
        .and_then(|row| row["baseUrl"].as_str());
    let base = model_base
        .map(str::to_owned)
        .or_else(|| crate::providers::configured_base_url(config))?;
    let url = reqwest::Url::parse(&base).ok()?;
    let host = url.host_str()?;
    let id = untagged(model).rsplit('/').next()?.to_ascii_lowercase();
    // A hosted model may have less capacity than its publisher's model card.
    // Decimal K/M limits are conservative ceilings; explicit route metadata
    // and subsequent native usage remain authoritative.
    catalog()["endpointLimits"]
        .as_array()?
        .iter()
        .find_map(|entry| {
            entry["hosts"]
                .as_array()?
                .iter()
                .any(|known| known.as_str() == Some(host))
                .then(|| positive(&entry["models"][&id]))
                .flatten()
        })
}

pub fn for_model(config: &Value, model: &str) -> Option<u64> {
    // Route metadata constrains the actual endpoint even when a CLI-only
    // context suffix is present. A suffix cannot enlarge a declared small route.
    declared(config, model)
        .or_else(|| declared(config, untagged(model)))
        .or_else(|| endpoint_window(config, model))
        .or_else(|| published(model))
}

pub struct ClaudeContext {
    pub launch_model: Option<String>,
    pub context_window: Option<u64>,
}

fn custom_claude_id(model: &str) -> bool {
    let id = untagged(model).to_ascii_lowercase();
    !id.contains("claude-")
        && !matches!(
            id.as_str(),
            "default" | "best" | "fable" | "opus" | "sonnet" | "haiku" | "opusplan" | "inherit"
        )
}

pub fn effective_percent(config: &Value, model: &str) -> Option<u64> {
    let valid = |id: &str| {
        positive(&config["modelMetadata"][id]["effectiveContextWindowPercent"])
            .filter(|value| *value <= 100)
    };
    valid(model).or_else(|| valid(untagged(model)))
}

fn claude_window(config: &Value, model: &str) -> Option<u64> {
    let window = for_model(config, model)?;
    let percent = effective_percent(config, model).unwrap_or(100);
    window
        .checked_mul(percent)
        .map(|value| value / 100)
        .filter(|value| *value > 0)
}

/// Applies only to the child process. Provider IDs and persisted chat settings
/// stay intact. Native usage reports remain authoritative over this fallback.
pub fn configure_claude(
    command: &mut tokio::process::Command,
    config: &Value,
    selected_model: Option<&str>,
) -> ClaudeContext {
    for key in [
        "DISABLE_COMPACT",
        "DISABLE_AUTO_COMPACT",
        "CLAUDE_CODE_DISABLE_UNKNOWN_MODEL_WINDOW_ENFORCEMENT",
        "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
        "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
        "CLAUDE_CODE_DISABLE_1M_CONTEXT",
        "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE",
    ] {
        command.env_remove(key);
    }
    command.env("CLAUDE_AUTOCOMPACT_PCT_OVERRIDE", "90");
    let mut result = ClaudeContext {
        launch_model: selected_model.map(str::to_owned),
        context_window: None,
    };
    if config["official"] == true {
        return result;
    }
    let mapping_keys = [
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
        "ANTHROPIC_REASONING_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
        "CLAUDE_CODE_SUBAGENT_MODEL",
    ];
    let mappings = mapping_keys
        .into_iter()
        .filter_map(|key| {
            let model = config["env"][key].as_str()?;
            custom_claude_id(model).then(|| (key, model, claude_window(config, model)))
        })
        .collect::<Vec<_>>();
    let variant = |id: &str, window: Option<u64>| {
        if window.is_some_and(|value| value >= 1_000_000) {
            format!("{}[1m]", untagged(id))
        } else {
            untagged(id).to_owned()
        }
    };
    let selected_capacity = selected_model
        .filter(|model| custom_claude_id(model))
        .and_then(|model| claude_window(config, model));
    let (Some(selected), Some(capacity)) = (selected_model, selected_capacity) else {
        // An unknown or native main model cannot justify a larger global
        // window. Its custom children still need their own verified 1M variant.
        for (key, model, window) in &mappings {
            command.env(key, variant(model, *window));
        }
        let unknown_main = selected_model.is_none_or(custom_claude_id);
        let small = mappings
            .iter()
            .map(|(_, _, window)| *window)
            .filter(|window| window.is_none_or(|value| value < 1_000_000))
            .map(|window| window.unwrap_or(200_000))
            .min();
        if let Some(small) = small {
            // Recognized Claude IDs ignore MAX_CONTEXT with compaction on.
            // An unknown main keeps its native default ceiling or a smaller
            // child limit, never an unverified increase to 1M.
            let safe = if unknown_main {
                small.min(200_000)
            } else {
                small
            };
            command.env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", safe.to_string());
        }
        if mappings
            .iter()
            .any(|(_, _, window)| window.is_some_and(|value| value >= 1_000_000))
        {
            command.env("CLAUDE_CODE_AUTO_COMPACT_WINDOW", "1000000");
        }
        return result;
    };
    let has_native_mapping = mapping_keys
        .iter()
        .filter_map(|key| config["env"][*key].as_str())
        .any(|model| !custom_claude_id(model));
    let uniform = !has_native_mapping
        && mappings
            .iter()
            .all(|(_, _, window)| *window == Some(capacity));
    let mut effective_capacity = capacity;
    if uniform {
        // [256k] is selector metadata, not a Claude Code wire-model suffix.
        if selected.to_ascii_lowercase().ends_with("[256k]") {
            result.launch_model = Some(untagged(selected).to_owned());
        }
        for (key, model, _) in &mappings {
            if model.to_ascii_lowercase().ends_with("[256k]") {
                command.env(key, untagged(model));
            }
        }
        command.env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", capacity.to_string());
        command.env(
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
            capacity.min(1_000_000).to_string(),
        );
        // Unknown custom [1m] IDs otherwise ignore MAX_CONTEXT. Treat the
        // suffix as an untagged custom ID while retaining proactive compaction.
        if capacity != 1_000_000
            && (selected.to_ascii_lowercase().ends_with("[1m]")
                || mappings
                    .iter()
                    .any(|(_, id, _)| id.to_ascii_lowercase().ends_with("[1m]")))
        {
            command.env("CLAUDE_CODE_DISABLE_1M_CONTEXT", "1");
        }
    } else {
        // MAX_CONTEXT is inherited by subagents. In a mixed family, only
        // models with confirmed 1M capacity receive Claude's CLI-only suffix.
        // The CLI strips it before sending the provider's real model ID.
        result.launch_model = Some(variant(selected, Some(capacity)));
        for (key, model, window) in &mappings {
            command.env(key, variant(model, *window));
        }
        let small = std::iter::once(Some(capacity))
            .chain(mappings.iter().map(|(_, _, window)| *window))
            .filter(|window| window.is_none_or(|value| value < 1_000_000))
            .map(|window| window.unwrap_or(200_000))
            .min();
        if let Some(small) = small {
            command.env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", small.to_string());
            if capacity < 1_000_000 {
                effective_capacity = capacity.min(small);
            }
        }
        let window = std::iter::once(capacity)
            .chain(mapping_keys.iter().filter_map(|key| {
                config["env"][*key]
                    .as_str()
                    .and_then(|model| claude_window(config, model))
            }))
            .max()
            .unwrap_or(capacity);
        command.env(
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
            window.min(1_000_000).max(100_000).to_string(),
        );
    }
    if let Some(model) = &result.launch_model {
        command.env("ANTHROPIC_MODEL", model);
    }
    result.context_window = Some(if !uniform && capacity >= 1_000_000 {
        1_000_000
    } else {
        effective_capacity
    });
    result
}

pub fn catalog_row(config: &Value, model: &str) -> Value {
    let mut row = json!({});
    if let Some(capacity) = for_model(config, model) {
        row["contextWindow"] = json!(capacity);
    }
    if let Some(percent) = effective_percent(config, model) {
        row["effectiveContextWindowPercent"] = json!(percent);
    }
    row
}

pub fn api_catalog(value: &Value) -> Value {
    let mut result = json!({});
    for row in value["data"].as_array().into_iter().flatten().take(200) {
        if let Some(id) = row["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 200 && !id.chars().any(char::is_control))
        {
            let safe = metadata(row);
            if !safe.as_object().unwrap().is_empty() {
                result[id] = safe;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinct_windows_and_explicit_kimi_variants_are_not_one_global_limit() {
        assert_eq!(published("glm-5.3-flash"), Some(1_000_000));
        assert_eq!(published("glm-5-turbo"), Some(200_000));
        assert_eq!(published("k3"), Some(262_144));
        assert_eq!(published("k3[1m]"), Some(1_048_576));
        assert_eq!(published("kimi-for-coding-highspeed"), Some(262_144));
        assert_eq!(published("MiniMax-M2.7"), Some(204_800));
        assert_eq!(published("unknown"), None);
        assert_eq!(published("auto"), None);
        assert_eq!(published("private[1m]"), None);
        assert_eq!(published("glm-5-turbo[1m]"), Some(200_000));
        assert_eq!(published("MiniMax-M2.7[256k]"), Some(204_800));
    }
    #[test]
    fn native_and_api_metadata_override_the_published_capacity() {
        let config = json!({"piProvider":{"models":[{"id":"glm-5.3","contextWindow":32768}]},"modelMetadata":{"glm-5.3":{"contextWindow":65536}}});
        assert_eq!(for_model(&config, "glm-5.3"), Some(32768));
        assert_eq!(
            for_model(
                &json!({"opencodeProvider":{"models":{"k3":{"limit":{"context":1048576}}}}}),
                "k3"
            ),
            Some(1048576)
        );
        assert_eq!(
            for_model(
                &json!({"modelMetadata":{"k3":{"contextWindow":1048576}}}),
                "k3"
            ),
            Some(1048576)
        );
        assert_eq!(
            for_model(
                &json!({"config":"model='private'\nmodel_context_window=64000"}),
                "private"
            ),
            Some(64000)
        );
    }
    #[test]
    fn hosted_limits_follow_the_actual_endpoint_and_never_the_supplier_label() {
        for base in [
            "https://dashscope.aliyuncs.com/apps/anthropic",
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        ] {
            let config = json!({"providerId":"custom","baseUrl":base});
            assert_eq!(for_model(&config, "MiniMax-M3[1m]"), Some(192000));
            assert_eq!(
                catalog_row(&config, "MiniMax-M2.7")["contextWindow"],
                192000
            );
            assert_eq!(for_model(&config, "deepseek-v4-pro"), Some(1000000));
            assert_eq!(for_model(&config, "qwen3.8-max"), Some(1000000));
        }
        let env =
            json!({"env":{"ANTHROPIC_BASE_URL":"https://dashscope.aliyuncs.com/apps/anthropic"}});
        assert_eq!(for_model(&env, "MiniMax-M3"), Some(192000));
        let toml = json!({"config":"model='MiniMax-M3'\nmodel_provider='gateway'\n[model_providers.gateway]\nbase_url='https://dashscope.aliyuncs.com/compatible-mode/v1'"});
        assert_eq!(for_model(&toml, "MiniMax-M3"), Some(192000));
        for base in [
            "https://api.minimax.io/anthropic",
            "https://dashscope.aliyuncs.com.attacker.invalid/apps/anthropic",
            "https://gateway.invalid/v1",
        ] {
            assert_eq!(
                for_model(&json!({"providerId":"qwen","baseUrl":base}), "MiniMax-M3"),
                Some(1000000)
            );
        }
        let declared = json!({"baseUrl":"https://dashscope.aliyuncs.com/apps/anthropic","modelMetadata":{"MiniMax-M3":{"contextWindow":64000}}});
        assert_eq!(for_model(&declared, "MiniMax-M3[1m]"), Some(64000));
        let (runtime, env) = claude_command(&env, "MiniMax-M3");
        assert_eq!(runtime.context_window, Some(192000));
        assert_eq!(
            env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"].as_deref(),
            Some("192000")
        );
    }
    #[test]
    fn pi_per_model_endpoints_constrain_the_selected_model_instead_of_its_parent_provider() {
        let config = json!({"baseUrl":"https://api.minimax.io/anthropic","piProvider":{"models":[{"id":"MiniMax-M3","baseUrl":"https://dashscope.aliyuncs.com/apps/anthropic"}]}});
        assert_eq!(for_model(&config, "MiniMax-M3"), Some(192000));
        let reversed = json!({"baseUrl":"https://dashscope.aliyuncs.com/apps/anthropic","piProvider":{"models":[{"id":"MiniMax-M3","baseUrl":"https://api.minimax.io/anthropic"}]}});
        assert_eq!(for_model(&reversed, "MiniMax-M3"), Some(1000000));
        let explicit = json!({"baseUrl":"https://api.minimax.io/anthropic","piProvider":{"models":[{"id":"MiniMax-M3","baseUrl":"https://dashscope.aliyuncs.com/apps/anthropic","contextWindow":64000}]}});
        assert_eq!(for_model(&explicit, "MiniMax-M3"), Some(64000));
    }
    #[test]
    fn variant_catalog_rows_preserve_valid_base_model_effective_capacity() {
        let config = json!({"modelMetadata":{"k3":{"contextWindow":1000000,"effectiveContextWindowPercent":80},"k3[1m]":{"effectiveContextWindowPercent":101}}});
        let row = catalog_row(&config, "k3[1m]");
        assert_eq!(row["contextWindow"], 1000000);
        assert_eq!(row["effectiveContextWindowPercent"], 80);
        assert_eq!(claude_window(&config, "k3[1m]"), Some(800000));
    }
    #[test]
    fn limits_are_safe_bounded_metadata_even_for_unknown_models() {
        let result = api_catalog(
            &json!({"data":[{"id":"future","context_length":"512000","apiKey":"private"},{"id":"new","max_input_tokens":1048576},{"id":"bad","contextWindow":-1}]}),
        );
        assert_eq!(result["future"]["contextWindow"], 512000);
        assert_eq!(result["new"]["contextWindow"], 1048576);
        assert!(result.get("bad").is_none());
        assert!(!result.to_string().contains("private"));
        assert_eq!(
            metadata(&json!({"context_window":500000,"effective_context_window_percent":95})),
            json!({"contextWindow":500000,"effectiveContextWindowPercent":95})
        );
        for value in [
            json!(0),
            json!(-1),
            json!(true),
            json!(1.5),
            json!(100000001),
        ] {
            assert!(positive(&value).is_none());
        }
    }
    fn claude_command(
        config: &Value,
        model: &str,
    ) -> (
        ClaudeContext,
        std::collections::BTreeMap<String, Option<String>>,
    ) {
        let mut command = tokio::process::Command::new("test-only-never-launched");
        for key in [
            "DISABLE_COMPACT",
            "DISABLE_AUTO_COMPACT",
            "CLAUDE_CODE_DISABLE_UNKNOWN_MODEL_WINDOW_ENFORCEMENT",
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
            "CLAUDE_CODE_DISABLE_1M_CONTEXT",
        ] {
            command.env(key, "1");
        }
        for (key, value) in config["env"].as_object().into_iter().flatten() {
            if let Some(value) = value.as_str() {
                command.env(key, value);
            }
        }
        let result = configure_claude(&mut command, config, Some(model));
        let env = command
            .as_std()
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect();
        (result, env)
    }
    #[test]
    fn claude_declares_each_known_provider_window_without_changing_storage_model_ids() {
        for (model, window) in [
            ("glm-5.3", 1_000_000),
            ("glm-5-turbo", 200_000),
            ("k3", 262_144),
            ("MiniMax-M2.7", 204_800),
            ("qwen3.6-plus", 1_000_000),
        ] {
            let config = json!({"model":model,"env":{"ANTHROPIC_MODEL":model,"CLAUDE_CODE_SUBAGENT_MODEL":model,"DISABLE_AUTO_COMPACT":"1"}});
            let before = config.clone();
            let (runtime, env) = claude_command(&config, model);
            assert_eq!(runtime.launch_model.as_deref(), Some(model));
            assert_eq!(runtime.context_window, Some(window));
            assert_eq!(
                env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"].as_deref(),
                Some(window.to_string().as_str())
            );
            assert_eq!(
                env["CLAUDE_AUTOCOMPACT_PCT_OVERRIDE"].as_deref(),
                Some("90")
            );
            for key in [
                "DISABLE_COMPACT",
                "DISABLE_AUTO_COMPACT",
                "CLAUDE_CODE_DISABLE_UNKNOWN_MODEL_WINDOW_ENFORCEMENT",
            ] {
                assert_eq!(env[key], None);
            }
            assert_eq!(config, before);
        }
    }
    #[test]
    fn mixed_claude_family_preserves_large_main_and_small_or_unknown_child_windows() {
        let config = json!({"model":"glm-5.3","env":{"ANTHROPIC_DEFAULT_OPUS_MODEL":"glm-5.3","ANTHROPIC_DEFAULT_HAIKU_MODEL":"glm-5-turbo","CLAUDE_CODE_SUBAGENT_MODEL":"private-child"}});
        let (runtime, env) = claude_command(&config, "glm-5.3");
        assert_eq!(runtime.launch_model.as_deref(), Some("glm-5.3[1m]"));
        assert_eq!(runtime.context_window, Some(1_000_000));
        assert_eq!(
            env["ANTHROPIC_DEFAULT_OPUS_MODEL"].as_deref(),
            Some("glm-5.3[1m]")
        );
        assert_eq!(
            env["ANTHROPIC_DEFAULT_HAIKU_MODEL"].as_deref(),
            Some("glm-5-turbo")
        );
        assert_eq!(
            env["CLAUDE_CODE_SUBAGENT_MODEL"].as_deref(),
            Some("private-child")
        );
        assert_eq!(
            env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"].as_deref(),
            Some("200000")
        );
        assert_eq!(env["CLAUDE_CODE_DISABLE_1M_CONTEXT"], None);
        let smaller = json!({"modelMetadata":{"glm-5-turbo":{"contextWindow":32768}},"env":{"ANTHROPIC_DEFAULT_HAIKU_MODEL":"glm-5-turbo"}});
        let (_, env) = claude_command(&smaller, "glm-5.3");
        assert_eq!(
            env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"].as_deref(),
            Some("32768")
        );
    }
    #[test]
    fn selector_only_context_suffixes_do_not_reach_provider_model_ids() {
        let config = json!({"env":{"CLAUDE_CODE_SUBAGENT_MODEL":"k3[256k]"}});
        let (context, env) = claude_command(&config, "k3[256k]");
        assert_eq!(context.launch_model.as_deref(), Some("k3"));
        assert_eq!(context.context_window, Some(262144));
        assert_eq!(env["CLAUDE_CODE_SUBAGENT_MODEL"].as_deref(), Some("k3"));
        assert_eq!(env["ANTHROPIC_MODEL"].as_deref(), Some("k3"));
        assert_eq!(config["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "k3[256k]");
    }
    #[test]
    fn official_native_models_and_unknown_ids_keep_their_native_windows() {
        for (config, model) in [
            (json!({"official":true}), "claude-sonnet-5-5"),
            (json!({}), "claude-opus-4-6[1m]"),
            (json!({}), "private[1m]"),
            (json!({}), "private"),
        ] {
            let (runtime, env) = claude_command(&config, model);
            assert_eq!(runtime.launch_model.as_deref(), Some(model));
            assert_eq!(runtime.context_window, None);
            assert_eq!(env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], None);
            assert_eq!(env["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], None);
            assert_eq!(env["CLAUDE_CODE_DISABLE_1M_CONTEXT"], None);
        }
        let mixed = json!({"env":{"ANTHROPIC_DEFAULT_HAIKU_MODEL":"claude-haiku-4-5"}});
        let (runtime, env) = claude_command(&mixed, "glm-5.3");
        assert_eq!(runtime.launch_model.as_deref(), Some("glm-5.3[1m]"));
        assert_eq!(
            env["ANTHROPIC_DEFAULT_HAIKU_MODEL"].as_deref(),
            Some("claude-haiku-4-5")
        );
        assert_eq!(env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], None);
        assert_eq!(env["DISABLE_COMPACT"], None);
        assert_eq!(env["CLAUDE_CODE_DISABLE_1M_CONTEXT"], None);
    }
    #[test]
    fn explicit_small_metadata_and_effective_percent_constrain_cli_context_variants() {
        let config = json!({"modelMetadata":{"glm-5.3":{"contextWindow":100000,"effectiveContextWindowPercent":80}}});
        assert_eq!(for_model(&config, "glm-5.3[1m]"), Some(100000));
        let (runtime, env) = claude_command(&config, "glm-5.3[1m]");
        assert_eq!(runtime.launch_model.as_deref(), Some("glm-5.3[1m]"));
        assert_eq!(runtime.context_window, Some(80000));
        assert_eq!(
            env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"].as_deref(),
            Some("80000")
        );
        assert_eq!(env["CLAUDE_CODE_DISABLE_1M_CONTEXT"].as_deref(), Some("1"));
        let private = json!({"modelMetadata":{"private":{"contextWindow":64000}}});
        let (runtime, _) = claude_command(&private, "private");
        assert_eq!(runtime.context_window, Some(64000));
        let mixed = json!({"modelMetadata":{"private":{"contextWindow":400000}},"env":{"CLAUDE_CODE_SUBAGENT_MODEL":"unknown-child"}});
        let (runtime, env) = claude_command(&mixed, "private");
        assert_eq!(runtime.context_window, Some(200000));
        assert_eq!(
            env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"].as_deref(),
            Some("200000")
        );
    }
    #[test]
    fn unknown_main_keeps_native_default_while_known_children_get_their_verified_window() {
        let config = json!({"env":{"ANTHROPIC_DEFAULT_SONNET_MODEL":"glm-5.3","CLAUDE_CODE_SUBAGENT_MODEL":"glm-5.3"}});
        let before = config.clone();
        let (runtime, env) = claude_command(&config, "unknown-main");
        assert_eq!(runtime.launch_model.as_deref(), Some("unknown-main"));
        assert_eq!(runtime.context_window, None);
        assert_eq!(
            env["ANTHROPIC_DEFAULT_SONNET_MODEL"].as_deref(),
            Some("glm-5.3[1m]")
        );
        assert_eq!(
            env["CLAUDE_CODE_SUBAGENT_MODEL"].as_deref(),
            Some("glm-5.3[1m]")
        );
        assert_eq!(env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], None);
        assert_eq!(
            env["CLAUDE_CODE_AUTO_COMPACT_WINDOW"].as_deref(),
            Some("1000000")
        );
        assert_eq!(config, before);
        let small = json!({"modelMetadata":{"private-child":{"contextWindow":32768}},"env":{"ANTHROPIC_DEFAULT_SONNET_MODEL":"glm-5.3","CLAUDE_CODE_SUBAGENT_MODEL":"private-child"}});
        let (runtime, env) = claude_command(&small, "unknown-main");
        assert_eq!(runtime.context_window, None);
        assert_eq!(
            env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"].as_deref(),
            Some("32768")
        );
        assert_eq!(
            env["CLAUDE_CODE_SUBAGENT_MODEL"].as_deref(),
            Some("private-child")
        );
    }
    #[test]
    fn native_main_and_official_account_limits_are_not_enlarged_by_custom_child_metadata() {
        let config = json!({"env":{"ANTHROPIC_DEFAULT_HAIKU_MODEL":"glm-5.3"}});
        let (runtime, env) = claude_command(&config, "claude-opus-4-6");
        assert_eq!(runtime.launch_model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(runtime.context_window, None);
        assert_eq!(
            env["ANTHROPIC_DEFAULT_HAIKU_MODEL"].as_deref(),
            Some("glm-5.3[1m]")
        );
        assert_eq!(env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], None);
        assert_eq!(env["CLAUDE_CODE_DISABLE_1M_CONTEXT"], None);
        let official = json!({"official":true,"env":{"ANTHROPIC_DEFAULT_HAIKU_MODEL":"glm-5.3"}});
        let (runtime, env) = claude_command(&official, "claude-opus-4-6");
        assert_eq!(runtime.context_window, None);
        assert_eq!(
            env["ANTHROPIC_DEFAULT_HAIKU_MODEL"].as_deref(),
            Some("glm-5.3")
        );
        assert_eq!(env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], None);
        assert_eq!(env["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], None);
    }
}
