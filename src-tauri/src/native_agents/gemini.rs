//! Gemini CLI 0.63 ACP configuration. Managed state never shares ~/.gemini.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub(super) fn mode(permission: &str) -> &'static str {
    match permission {
        "read" | "summary" => "plan",
        "full" => "yolo",
        _ => "default",
    }
}
pub(super) fn startup_model(mut config: Value, model: Option<&str>) -> Value {
    if let Some(model) = model {
        config["model"] = json!(model);
    }
    config
}

pub(super) fn api_endpoint(config: &Value) -> (String, String) {
    let base = crate::providers::gemini_api_base(
        config["baseUrl"]
            .as_str()
            .unwrap_or("https://generativelanguage.googleapis.com")
            .trim_end_matches('/'),
    );
    for version in ["v1beta", "v1alpha", "v1"] {
        if let Some(root) = base.strip_suffix(&format!("/{version}")) {
            return (root.to_owned(), version.to_owned());
        }
    }
    (base, "v1beta".into())
}

pub(super) fn authentication(config: &Value) -> Value {
    let (base, _) = api_endpoint(config);
    let official = reqwest::Url::parse(&base)
        .ok()
        .is_some_and(|url| url.host_str() == Some("generativelanguage.googleapis.com"));
    let mut params = json!({
        "methodId": if official { "gemini-api-key" } else { "gateway" },
        "_meta": {"api-key": crate::providers::key(config).unwrap_or("")}
    });
    if !official {
        params["_meta"]["gateway"] = json!({"baseUrl":base});
    }
    params
}

pub(super) fn home(root: &Path, session: Option<&str>, transient: bool) -> PathBuf {
    if transient {
        root.join("agents/gemini-runtime/transient")
            .join(uuid::Uuid::new_v4().to_string())
    } else {
        // Imported session IDs are data, never path components.
        root.join("agents/gemini-runtime/sessions").join(format!(
            "{:x}",
            Sha256::digest(session.unwrap_or("default").as_bytes())
        ))
    }
}

pub(super) fn policy(permission: &str) -> Value {
    let mut rules = vec![];
    match permission {
        "read" | "summary" => {
            rules.push(json!({"toolName":"*","decision":"deny","priority":900}));
            if permission == "read" {
                rules.push(json!({
                    "toolName":["read_file","read_many_files","list_directory","glob","grep_search","search_file_content","google_web_search","web_fetch"],
                    "decision":"allow","priority":950
                }));
            }
        }
        "strict" => {
            rules.push(json!({"toolName":"*","decision":"ask_user","priority":900}));
        }
        "full" => {}
        _ => {
            rules.push(json!({"toolName":"*","decision":"ask_user","priority":900}));
            rules.push(json!({
                "toolName":["read_file","read_many_files","list_directory","glob","grep_search","search_file_content"],
                "decision":"allow","priority":950
            }));
        }
    }
    json!({"rule":rules})
}

pub(super) async fn configure(
    home: &Path,
    config: &Value,
    permission: &str,
) -> Result<(Vec<String>, Vec<(String, String)>), String> {
    let settings_dir = home.join(".gemini");
    tokio::fs::create_dir_all(&settings_dir)
        .await
        .map_err(|e| format!("无法创建 Gemini 会话目录：{e}"))?;
    let settings_file = settings_dir.join("supercode-settings.json");
    let defaults_file = settings_dir.join("supercode-defaults.json");
    let policy_file = settings_dir.join("supercode-policy.toml");
    let auth = authentication(config);
    let settings = json!({
        "security":{"auth":{"selectedType":auth["methodId"]},"disableAlwaysAllow":true},
        "general":{"enableAutoUpdate":false,"plan":{"enabled":true}},
        "telemetry":{"enabled":false},
        "privacy":{"usageStatisticsEnabled":false},
        "hooksConfig":{"enabled":false},
        "advanced":{"ignoreLocalEnv":true},
        "admin":{"extensions":{"enabled":false},"skills":{"enabled":permission!="summary"},"mcp":{"enabled":permission!="summary"}}
    });
    tokio::fs::write(
        &settings_file,
        serde_json::to_vec(&settings).map_err(|e| e.to_string())?,
    )
    .await
    .map_err(|e| e.to_string())?;
    tokio::fs::write(&defaults_file, b"{}")
        .await
        .map_err(|e| e.to_string())?;
    let policy = toml::to_string(&policy(permission)).map_err(|e| e.to_string())?;
    tokio::fs::write(&policy_file, policy.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let (base, version) = api_endpoint(config);
    let mut args = vec![
        "--acp".into(),
        "--approval-mode".into(),
        mode(permission).into(),
        "--admin-policy".into(),
        crate::agents::ordinary(policy_file)
            .to_string_lossy()
            .into_owned(),
    ];
    if let Some(model) = config["model"].as_str() {
        if model.is_empty() || model.len() > 200 || model.chars().any(char::is_control) {
            return Err("Gemini 模型 ID 无效".into());
        }
        // A single equals-form argument keeps even a leading '-' literal.
        args.push(format!("--model={model}"));
    }
    let env = vec![
        (
            "GEMINI_CLI_HOME".into(),
            crate::agents::ordinary(home.to_owned())
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "GEMINI_CLI_SYSTEM_SETTINGS_PATH".into(),
            crate::agents::ordinary(settings_file)
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "GEMINI_CLI_SYSTEM_DEFAULTS_PATH".into(),
            crate::agents::ordinary(defaults_file)
                .to_string_lossy()
                .into_owned(),
        ),
        (
            "GEMINI_API_KEY".into(),
            crate::providers::key(config).unwrap_or("").into(),
        ),
        (
            "GEMINI_MODEL".into(),
            config["model"].as_str().unwrap_or("").into(),
        ),
        ("GOOGLE_GEMINI_BASE_URL".into(), base),
        ("GOOGLE_GENAI_API_VERSION".into(), version),
        ("GEMINI_FORCE_FILE_STORAGE".into(), "true".into()),
        ("GOOGLE_GENAI_USE_VERTEXAI".into(), "false".into()),
        ("GOOGLE_GENAI_USE_GCA".into(), "false".into()),
        ("GEMINI_CLI_CUSTOM_HEADERS".into(), "".into()),
        ("GEMINI_SYSTEM_MD".into(), "".into()),
        ("NO_BROWSER".into(), "true".into()),
    ];
    Ok((args, env))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_version_and_authentication_do_not_duplicate_api_path() {
        let config = json!({"baseUrl":"https://gateway.invalid/google/v1beta/","apiKey":"fixture"});
        assert_eq!(
            api_endpoint(&config),
            ("https://gateway.invalid/google".into(), "v1beta".into())
        );
        assert_eq!(authentication(&config)["methodId"], "gateway");
        assert_eq!(
            authentication(&config)["_meta"]["gateway"]["baseUrl"],
            "https://gateway.invalid/google"
        );
        assert_eq!(authentication(&json!({}))["methodId"], "gemini-api-key");
        assert_eq!(
            authentication(&json!({"baseUrl":"https://gateway.invalid"}))["_meta"]["api-key"],
            ""
        );
    }

    #[test]
    fn managed_modes_enforce_read_only_and_summary_even_in_plan_mode() {
        for permission in ["read", "summary"] {
            assert_eq!(mode(permission), "plan");
            assert_eq!(policy(permission)["rule"][0]["decision"], "deny");
            assert_eq!(policy(permission)["rule"][0]["toolName"], "*");
        }
        assert_eq!(policy("summary")["rule"].as_array().unwrap().len(), 1);
        assert_eq!(policy("strict")["rule"][0]["decision"], "ask_user");
        assert_eq!(mode("full"), "yolo");
        assert!(policy("full")["rule"].as_array().unwrap().is_empty());
        assert_eq!(mode("edit"), "default");
        let text = toml::to_string(&policy("read")).unwrap();
        assert!(toml::from_str::<toml::Table>(&text).is_ok());
    }

    #[test]
    fn session_homes_are_separate_and_untrusted_ids_never_become_paths() {
        let root = Path::new("managed");
        let first = home(root, Some("../outside"), false);
        assert_eq!(
            first.parent().unwrap(),
            root.join("agents/gemini-runtime/sessions")
        );
        assert_eq!(first.file_name().unwrap().to_string_lossy().len(), 64);
        assert_ne!(first, home(root, Some("other"), false));
        assert_ne!(first, home(root, Some("../outside"), true));
        assert_eq!(first, home(root, Some("../outside"), false));
    }
    #[tokio::test]
    async fn selected_model_is_available_before_initialize_and_cannot_inject_cli_flags() {
        let root =
            std::env::temp_dir().join(format!("supercode-gemini-model-{}", uuid::Uuid::new_v4()));
        let config = startup_model(
            json!({"model":"profile-default","apiKey":"fixture-key"}),
            Some("--literal-custom-model"),
        );
        let (args, env) = configure(&root, &config, "summary").await.unwrap();
        assert!(args.contains(&"--model=--literal-custom-model".into()));
        assert!(!args.contains(&"--literal-custom-model".into()));
        assert!(env.contains(&("GEMINI_MODEL".into(), "--literal-custom-model".into())));
        assert_eq!(startup_model(config.clone(), None), config);
        let settings = tokio::fs::read(root.join(".gemini/supercode-settings.json"))
            .await
            .unwrap();
        assert!(!String::from_utf8(settings).unwrap().contains("fixture-key"));
        let mut invalid = config;
        invalid["model"] = json!("model\n--yolo");
        assert!(configure(&root, &invalid, "summary").await.is_err());
        let resolved = root.canonicalize().unwrap();
        assert_eq!(
            resolved.parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        tokio::fs::remove_dir_all(resolved).await.unwrap();
    }
}
