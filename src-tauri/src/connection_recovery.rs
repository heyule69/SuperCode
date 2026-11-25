//! Display unreadable connections without changing their encrypted source.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MARKER: &str = "_supercodeUnavailableConnection";
pub(crate) fn read(raw: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(&crate::credentials::open(raw)?)
        .map_err(|_| "本地连接配置无法读取，请重新配置连接")?;
    if !value.is_object() || value.get(MARKER).is_some() {
        return Err("本地连接配置无效，请重新配置连接".into());
    }
    Ok(value)
}
pub(crate) fn display(raw: &str) -> Value {
    read(raw).unwrap_or_else(|_| json!({
        (MARKER): {"sha256":format!("{:x}",Sha256::digest(raw.as_bytes())),
            "reason":"此连接配置无法解密或读取。原聊天和加密配置已保留，请重新填写连接地址、模型和 API Key 后保存。"},
        "providerId":"custom","source":"unavailable"
    }))
}
pub(crate) fn reason(config: &Value) -> Option<&str> {
    config[MARKER]["reason"].as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ccswitch::Profile, storage::Store};
    use rusqlite::{params, Connection};
    const BROKEN: &str = "dpapi:AAECAwQFBgcICQ==";
    fn fixture() -> (Store, String) {
        assert!(crate::credentials::open(BROKEN).is_err());
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let project = store
            .add_project(std::path::Path::new("D:/connection-recovery-fixture"))
            .unwrap();
        let chat = store.create_session(&project.id, None).unwrap();
        store
            .save_message(
                "history",
                &chat.id,
                "user",
                "必须保留的历史",
                "userMessage",
                &Value::Null,
            )
            .unwrap();
        {
            let conn = store.0.lock().unwrap();
            conn.execute(
                "INSERT INTO agent_profiles VALUES('broken','codex','旧连接',?1)",
                [BROKEN],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO agent_profiles VALUES('claude-broken','claude','旧 Claude',?1)",
                [BROKEN],
            )
            .unwrap();
            conn.execute("INSERT OR REPLACE INTO settings VALUES('connection_order_codex','[\"broken\",\"@official\"]')",[]).unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO settings VALUES('profile_codex','broken')",
                [],
            )
            .unwrap();
            conn.execute("UPDATE sessions SET connection_id='broken',native_id='original-native' WHERE id=?1",[&chat.id]).unwrap();
        }
        // Exercise the same initialization path as opening an existing database.
        let store = Store::from_connection(store.0.into_inner().unwrap()).unwrap();
        (store, chat.id)
    }
    fn repaired() -> Profile {
        Profile {
            id: "broken".into(),
            agent: "codex".into(),
            name: "旧连接".into(),
            config: json!({"providerId":"custom","protocol":"responses","baseUrl":"https://fixture.example/v1","apiKey":"new-fixture-key","model":"fixture-model"}),
        }
    }
    fn raw(store: &Store) -> String {
        store
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT config FROM agent_profiles WHERE id='broken'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
    #[test]
    fn startup_and_display_keep_unreadable_connections_ciphertext_chats_and_defaults() {
        let (store, chat) = fixture();
        let profiles = store.profiles_for_display().unwrap();
        assert_eq!(profiles.len(), 2);
        for profile in &profiles {
            assert!(!profile.is_official());
            let summary = serde_json::to_value(profile.summary(true)).unwrap();
            assert!(summary["credentialError"].is_string());
            assert_eq!(summary["hasCredential"], false);
            assert!(!summary.to_string().contains(BROKEN));
        }
        assert_eq!(store.default_connection("codex").unwrap(), "broken");
        assert!(store.route("codex", Some(&chat)).is_err());
        assert_eq!(
            store.session(&chat).unwrap().native_id.as_deref(),
            Some("original-native")
        );
        assert_eq!(
            store.messages(&chat, None).unwrap()[0].text,
            "必须保留的历史"
        );
        assert_eq!(raw(&store), BROKEN);
        assert_eq!(
            store
                .0
                .lock()
                .unwrap()
                .query_row::<i64, _, _>("SELECT count(*) FROM credential_recovery", [], |r| r
                    .get(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn explicit_repair_preserves_ciphertext_and_chat_binding_then_survives_restart() {
        let (store, chat) = fixture();
        let previous = store.profile_for_editing("broken").unwrap();
        let mut settings = crate::providers::settings(&previous);
        settings.base_url = "https://fixture.example/v1".into();
        settings.model = "fixture-model".into();
        settings.api_key = Some("new-fixture-key".into());
        let replacement = crate::providers::make_profile(settings, Some(&previous)).unwrap();
        assert_eq!(replacement.id, previous.id);
        assert!(reason(&replacement.config).is_none());
        store.save_profile(&replacement, Some(&previous)).unwrap();
        assert_ne!(raw(&store), BROKEN);
        assert_eq!(
            store
                .0
                .lock()
                .unwrap()
                .query_row::<String, _, _>(
                    "SELECT config FROM credential_recovery WHERE profile_id='broken'",
                    [],
                    |r| r.get(0)
                )
                .unwrap(),
            BROKEN
        );
        let session = store.session(&chat).unwrap();
        assert_eq!(session.connection_id.as_deref(), Some("broken"));
        assert!(session.native_id.is_none());
        assert_eq!(store.messages(&chat, None).unwrap().len(), 1);
        let store = Store::from_connection(store.0.into_inner().unwrap()).unwrap();
        assert_eq!(
            store.route("codex", Some(&chat)).unwrap().config["apiKey"],
            "new-fixture-key"
        );
        assert_eq!(store.default_connection("codex").unwrap(), "broken");
        assert!(store
            .profiles_for_display()
            .unwrap()
            .iter()
            .find(|p| p.id == "broken")
            .unwrap()
            .summary(true)
            .credential_error
            .is_none());
    }
    #[test]
    fn concurrent_repair_busy_turn_and_placeholder_save_never_overwrite_encrypted_source() {
        let (store, chat) = fixture();
        let previous = store.profile_for_editing("broken").unwrap();
        assert!(store.save_profile(&previous, Some(&previous)).is_err());
        store
            .0
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET status='running' WHERE id=?1", [&chat])
            .unwrap();
        assert!(store.save_profile(&repaired(), Some(&previous)).is_err());
        assert_eq!(raw(&store), BROKEN);
        store
            .0
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET status='idle' WHERE id=?1", [&chat])
            .unwrap();
        store.save_profile(&repaired(), Some(&previous)).unwrap();
        let saved = raw(&store);
        assert!(store.save_profile(&repaired(), Some(&previous)).is_err());
        assert_eq!(raw(&store), saved);
    }
    #[test]
    fn unreadable_other_connections_do_not_block_healthy_routes_or_explicit_removal() {
        let (store, chat) = fixture();
        let mut good = repaired();
        good.id = "good".into();
        store.import_profiles(&[good]).unwrap();
        assert_eq!(
            store.route_for("codex", "good").unwrap().config["apiKey"],
            "new-fixture-key"
        );
        store.delete_profiles("codex", &["broken".into()]).unwrap();
        assert!(store.route("codex", Some(&chat)).is_err());
        assert_eq!(store.messages(&chat, None).unwrap().len(), 1);
    }
    #[test]
    fn legacy_chats_are_not_reassigned_when_their_connection_cannot_be_identified() {
        let (store, chat) = fixture();
        store
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET connection_id=NULL WHERE id=?1",
                params![chat],
            )
            .unwrap();
        let store = Store::from_connection(store.0.into_inner().unwrap()).unwrap();
        assert!(store.session(&chat).unwrap().connection_id.is_none());
        assert_eq!(raw(&store), BROKEN);
    }
    #[test]
    fn normal_database_open_preserves_history_when_windows_cannot_decrypt_a_connection() {
        let (store, chat) = fixture();
        let root = std::env::temp_dir().join(format!(
            "supercode-credential-recovery-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("supercode.db");
        {
            let conn = store.0.lock().unwrap();
            conn.pragma_update(None, "user_version", crate::backup::SCHEMA_VERSION)
                .unwrap();
            conn.backup(
                rusqlite::MAIN_DB,
                &path,
                None::<fn(rusqlite::backup::Progress)>,
            )
            .unwrap();
        }
        drop(store);
        let opened = Store::open(&path).unwrap();
        assert_eq!(
            opened.messages(&chat, None).unwrap()[0].text,
            "必须保留的历史"
        );
        assert_eq!(raw(&opened), BROKEN);
        assert!(opened
            .profile_for_editing("broken")
            .unwrap()
            .summary(true)
            .credential_error
            .is_some());
        drop(opened);
        assert!(root.starts_with(std::env::temp_dir()));
        for entry in std::fs::read_dir(&root).unwrap() {
            let path = entry.unwrap().path();
            assert_eq!(path.parent(), Some(root.as_path()));
            assert!(path.is_file());
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
}
