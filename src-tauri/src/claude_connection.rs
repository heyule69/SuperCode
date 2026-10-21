//! Apply the same connection identity to interactive turns and summaries.
use serde_json::Value;
use std::path::Path;
use tokio::process::Command;

const MANAGED_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_REASONING_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
];

pub(crate) fn configure(
    command: &mut Command,
    data_dir: &Path,
    route_id: &str,
    config: &Value,
) -> Result<(), String> {
    if route_id != crate::session_config::LOCAL {
        for key in MANAGED_ENV {
            command.env_remove(key);
        }
    }
    if let Some(env) = config["env"].as_object() {
        for (key, value) in env {
            if let Some(value) = value.as_str() {
                command.env(key, value);
            }
        }
    }
    crate::official_accounts::configure_in(command, data_dir, "claude", config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{collections::BTreeMap, ffi::OsStr};

    #[test]
    fn managed_account_replaces_inherited_credentials_and_cloud_routing() {
        let id = uuid::Uuid::new_v4().to_string();
        let base = Path::new("D:/SuperCode-connection-fixture");
        let mut command = Command::new("unused-fixture");
        for key in MANAGED_ENV {
            command.env(key, "inherited-fixture");
        }
        command.env("CLAUDE_CONFIG_DIR", "other-account");
        configure(
            &mut command,
            base,
            "account:fixture",
            &json!({"official":true,"accountId":id,"env":{}}),
        )
        .unwrap();
        let env = command.as_std().get_envs().collect::<BTreeMap<_, _>>();
        for key in MANAGED_ENV {
            assert_eq!(env.get(OsStr::new(key)), Some(&None), "{key}");
        }
        let directory = crate::official_accounts::account_directory(base, "claude", &id).unwrap();
        assert_eq!(
            env.get(OsStr::new("CLAUDE_CONFIG_DIR")),
            Some(&Some(directory.as_os_str()))
        );
    }

    #[test]
    fn managed_api_applies_only_selected_connection_after_cleanup() {
        let mut command = Command::new("unused-fixture");
        command.env("CLAUDE_CODE_OAUTH_TOKEN", "inherited-fixture");
        configure(&mut command, Path::new("."), "api-fixture", &json!({"env":{"ANTHROPIC_AUTH_TOKEN":"selected-fixture","ANTHROPIC_BASE_URL":"http://127.0.0.1:1234","ignored":123}})).unwrap();
        let env = command.as_std().get_envs().collect::<BTreeMap<_, _>>();
        assert_eq!(env.get(OsStr::new("CLAUDE_CODE_OAUTH_TOKEN")), Some(&None));
        assert_eq!(
            env.get(OsStr::new("ANTHROPIC_AUTH_TOKEN")),
            Some(&Some(OsStr::new("selected-fixture")))
        );
        assert_eq!(
            env.get(OsStr::new("ANTHROPIC_BASE_URL")),
            Some(&Some(OsStr::new("http://127.0.0.1:1234")))
        );
        assert!(!env.contains_key(OsStr::new("ignored")));
        assert!(configure(
            &mut command,
            Path::new("."),
            "api-fixture",
            &json!({"accountId":uuid::Uuid::nil().to_string()})
        )
        .is_err());
    }

    #[test]
    fn local_connection_preserves_user_environment() {
        let mut command = Command::new("unused-fixture");
        command.env("CLAUDE_CODE_USE_VERTEX", "user-fixture");
        configure(
            &mut command,
            Path::new("."),
            crate::session_config::LOCAL,
            &json!({}),
        )
        .unwrap();
        let env = command.as_std().get_envs().collect::<BTreeMap<_, _>>();
        assert_eq!(
            env.get(OsStr::new("CLAUDE_CODE_USE_VERTEX")),
            Some(&Some(OsStr::new("user-fixture")))
        );
        assert!(!env.contains_key(OsStr::new("ANTHROPIC_API_KEY")));
    }
}
