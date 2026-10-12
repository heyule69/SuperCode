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
        Some("1m") => positive(&entry["longContextWindow"]).or(Some(1_000_000)),
        Some("256k") => Some(262_144),
        _ => positive(&entry["contextWindow"]),
    }
}

pub fn for_model(config: &Value, model: &str) -> Option<u64> {
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
        .or_else(|| published(model))
}

pub fn catalog_row(config: &Value, model: &str) -> Value {
    let mut row = json!({});
    if let Some(capacity) = for_model(config, model) {
        row["contextWindow"] = json!(capacity);
    }
    if let Some(percent) =
        positive(&config["modelMetadata"][model]["effectiveContextWindowPercent"])
            .filter(|p| *p <= 100)
    {
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
}
