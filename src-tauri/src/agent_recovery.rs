//! Static compatibility evidence and reversible managed-install selection.
//! Inspect package metadata and saved paths; never execute an agent to recover it.
use crate::{
    agents::{self, AgentStatus, Launch},
    storage::Store,
    AppState,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, OnceLock},
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Compatibility {
    id: String,
    reference_version: Option<String>,
    covered_versions: Vec<String>,
    unsupported_below: Option<String>,
}
#[derive(Deserialize)]
struct Catalog {
    agents: Vec<Compatibility>,
}
fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../../resources/agent-compatibility.json"))
            .expect("invalid bundled agent compatibility catalog")
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryStatus {
    id: String,
    version: Option<String>,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_version: Option<String>,
    can_rollback: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_version: Option<String>,
    evidence: &'static str,
}

fn compatibility(id: &str, version: Option<&str>) -> (&'static str, Option<String>) {
    let Some(entry) = catalog().agents.iter().find(|entry| entry.id == id) else {
        return ("unverified", None);
    };
    let unsupported = version
        .and_then(|version| semver::Version::parse(version).ok())
        .zip(
            entry
                .unsupported_below
                .as_deref()
                .and_then(|version| semver::Version::parse(version).ok()),
        )
        .is_some_and(|(current, minimum)| current < minimum);
    let status = if unsupported {
        "unsupported"
    } else if version.is_some_and(|version| {
        entry
            .covered_versions
            .iter()
            .any(|covered| covered == version)
    }) {
        "covered"
    } else {
        "unverified"
    };
    (status, entry.reference_version.clone())
}

fn json_file(path: &Path) -> Result<serde_json::Value, String> {
    let metadata = fs::metadata(path).map_err(|_| "Agent 恢复记录不存在或无法读取")?;
    if !metadata.is_file() || metadata.len() > 128 * 1024 {
        return Err("Agent 恢复记录无效或过大".into());
    }
    let bytes = fs::read(path).map_err(|_| "Agent 恢复记录无法读取")?;
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err("Agent 恢复记录不是 UTF-8 无 BOM，未转换或修改".into());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "Agent 恢复记录不是 UTF-8，未转换或修改".to_owned())?;
    serde_json::from_str(text).map_err(|_| "Agent 恢复记录格式无效".into())
}

fn package_matches(id: &str, name: &str) -> bool {
    match id {
        "codex" => name == "@openai/codex" || name.starts_with("@openai/codex-win32-"),
        "claude" => {
            name == "@anthropic-ai/claude-code"
                || name.starts_with("@anthropic-ai/claude-code-win32-")
        }
        "opencode" => name == "opencode-ai" || name.starts_with("opencode-windows-"),
        "gemini" => name == "@google/gemini-cli",
        "pi" => matches!(
            name,
            "@earendil-works/pi-coding-agent" | "@mariozechner/pi-coding-agent"
        ),
        _ => false,
    }
}
fn static_version(id: &str, launch: &Launch) -> Option<String> {
    let path = launch
        .prefix
        .first()
        .map(Path::new)
        .unwrap_or(&launch.program);
    for parent in path.parent()?.ancestors().take(12) {
        let package = crate::agent_versions::package_name(id).ok()?;
        for manifest in [
            parent.join("package.json"),
            parent
                .join("node_modules")
                .join(package)
                .join("package.json"),
        ] {
            let Ok(value) = json_file(&manifest) else {
                continue;
            };
            if value["name"]
                .as_str()
                .is_some_and(|name| package_matches(id, name))
            {
                if let Some(version) = value["version"]
                    .as_str()
                    .and_then(|version| semver::Version::parse(version).ok())
                {
                    return Some(version.to_string());
                }
            }
        }
    }
    None
}

fn launch(id: &str, text: &str) -> Result<Launch, String> {
    let launch: Launch =
        serde_json::from_str(text).map_err(|_| "上一 Agent 配置无效".to_owned())?;
    let prefix_valid = if matches!(id, "pi" | "gemini") {
        launch.prefix.len() == 1
            && launch.prefix.first().is_some_and(|path| {
                Path::new(path).is_absolute()
                    && matches!(
                        Path::new(path)
                            .extension()
                            .and_then(|extension| extension.to_str()),
                        Some("js" | "mjs")
                    )
            })
    } else {
        launch.prefix.is_empty()
    };
    if !prefix_valid || !launch.valid() {
        return Err("上一 Agent 程序已移动或配置无效，未更改当前安装".into());
    }
    Ok(launch)
}

fn managed_root(data: &Path, id: &str, current: &Launch) -> Result<PathBuf, String> {
    let data = agents::ordinary(data.canonicalize().map_err(|_| "Agent 数据目录无效")?);
    let base = agents::ordinary(
        data.join("agents")
            .join(id)
            .canonicalize()
            .map_err(|_| "没有可回滚的独立安装")?,
    );
    if !base.starts_with(&data) {
        return Err("Agent 安装目录越界".into());
    }
    let candidate = current
        .prefix
        .first()
        .map(Path::new)
        .unwrap_or(&current.program);
    let relative = candidate
        .strip_prefix(&base)
        .map_err(|_| "当前安装不是 SuperCode 管理的版本")?;
    if relative
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("Agent 安装路径无效".into());
    }
    let Some(Component::Normal(root)) = relative.components().next() else {
        return Err("Agent 安装路径无效".into());
    };
    let uuid = root
        .to_str()
        .and_then(|root| uuid::Uuid::parse_str(root).ok())
        .ok_or("Agent 安装目录无效")?;
    if root.to_str() != Some(uuid.to_string().as_str()) {
        return Err("Agent 安装目录无效".into());
    }
    let root = base.join(root);
    let canonical = agents::ordinary(root.canonicalize().map_err(|_| "Agent 安装目录不存在")?);
    if canonical != root {
        return Err("Agent 安装目录不能通过链接跳转".into());
    }
    Ok(root)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Previous {
    agent: String,
    previous: Option<String>,
    previous_override: Option<String>,
    previous_resolved: Option<Launch>,
}
struct Rollback {
    current: String,
    previous: Previous,
    target: Launch,
}
fn rollback_record(store: &Store, data: &Path, id: &str) -> Result<Rollback, String> {
    if !agents::IDS.contains(&id) {
        return Err("Agent 无效".into());
    }
    let current = store
        .setting(&format!("agent_install_{id}"))?
        .ok_or("没有可回滚的独立安装")?;
    let current_launch: Launch =
        serde_json::from_str(&current).map_err(|_| "Agent 安装记录无效")?;
    let root = managed_root(data, id, &current_launch)?;
    let path = root.join("previous-installation.json");
    if agents::ordinary(path.canonicalize().map_err(|_| "没有上一安装记录")?).parent()
        != Some(root.as_path())
    {
        return Err("Agent 恢复记录不能跳转到安装目录之外".into());
    }
    let previous: Previous =
        serde_json::from_value(json_file(&path)?).map_err(|_| "Agent 恢复记录格式无效")?;
    if previous.agent != id {
        return Err("Agent 恢复记录不匹配".into());
    }
    let target = if let Some(value) = previous
        .previous_override
        .as_deref()
        .filter(|value| *value != "null")
    {
        launch(id, value)?
    } else if let Some(value) = previous.previous_resolved.as_ref() {
        launch(
            id,
            &serde_json::to_string(value).map_err(|_| "Agent 恢复记录无效")?,
        )?
    } else if let Some(value) = previous.previous.as_deref() {
        launch(id, value)?
    } else {
        return Err("这是首次安装，没有上一版本可回滚".into());
    };
    Ok(Rollback {
        current,
        previous,
        target,
    })
}

impl Store {
    fn restore_agent_configuration(&self, id: &str, record: &Rollback) -> Result<(), String> {
        let mut connection = self.0.lock().map_err(|error| error.to_string())?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        if crate::connection_usage::running(&transaction)? > 0 {
            return Err("请先完成或停止当前任务及后台子任务".into());
        }
        let key = format!("agent_install_{id}");
        let current: Option<String> = transaction
            .query_row("SELECT value FROM settings WHERE key=?1", [&key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|error| error.to_string())?;
        if current.as_deref() != Some(record.current.as_str()) {
            return Err("Agent 安装已变化，请刷新后重试".into());
        }
        for (key, value) in [
            (key, record.previous.previous.as_deref()),
            (
                format!("agent_path_{id}"),
                record.previous.previous_override.as_deref(),
            ),
        ] {
            if let Some(value) = value {
                transaction.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, value])
            } else { transaction.execute("DELETE FROM settings WHERE key=?1", [key]) }.map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }
}

#[tauri::command]
pub async fn list_agent_recovery(app: AppHandle) -> Result<Vec<RecoveryStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        agents::IDS
            .iter()
            .map(|id| {
                let version = agents::resolve(&app, id)
                    .ok()
                    .and_then(|launch| static_version(id, &launch));
                let (status, reference_version) = compatibility(id, version.as_deref());
                let previous = rollback_record(&state.store, &state.data_dir, id).ok();
                Ok(RecoveryStatus {
                    id: (*id).to_owned(),
                    version,
                    status,
                    reference_version,
                    can_rollback: previous.is_some(),
                    previous_version: previous
                        .and_then(|record| static_version(id, &record.target)),
                    evidence: "controlled_protocol_fixtures",
                })
            })
            .collect()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn rollback_agent(id: String, app: AppHandle) -> Result<Vec<AgentStatus>, String> {
    if !agents::IDS.contains(&id.as_str()) {
        return Err("Agent 无效".into());
    }
    let manager = app.state::<agents::Agents>();
    let _operation = manager
        .operation
        .try_write()
        .map_err(|_| "正在安装、更新或测试 Agent")?;
    let state = app.state::<AppState>();
    if state.store.running()? > 0 {
        return Err("请先完成或停止当前任务及后台子任务".into());
    }
    let record = rollback_record(&state.store, &state.data_dir, &id)?;
    state.store.restore_agent_configuration(&id, &record)?;
    state.runtime.shutdown().await;
    state.claude.shutdown().await;
    state.native.shutdown().await;
    agents::progress(
        &app,
        &id,
        "ready",
        "已恢复上一安装配置",
        None,
        None,
        Arc::new(AtomicBool::new(false)),
    );
    let _ = app.emit("workspace-updated", ());
    Ok(agents::IDS
        .iter()
        .map(|id| {
            let mut status = agents::status(&app, id);
            status.version = agents::resolve(&app, id)
                .ok()
                .and_then(|launch| static_version(id, &launch));
            status
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fixture {
        root: PathBuf,
        store: Store,
        current: Launch,
        old: Launch,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("supercode-recovery-{}", uuid::Uuid::new_v4()));
            let old = Self::executable(
                &root
                    .join("agents/codex")
                    .join(uuid::Uuid::new_v4().to_string()),
                "0.161.0",
            );
            let current = Self::executable(
                &root
                    .join("agents/codex")
                    .join(uuid::Uuid::new_v4().to_string()),
                "0.162.0",
            );
            let store =
                Store::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
            store
                .set_setting(
                    "agent_install_codex",
                    &serde_json::to_string(&current).unwrap(),
                )
                .unwrap();
            let fixture = Self {
                root,
                store,
                current,
                old,
            };
            fixture.snapshot(
                Some(serde_json::to_string(&fixture.old).unwrap()),
                None,
                Some(fixture.old.clone()),
            );
            fixture
        }
        fn executable(root: &Path, version: &str) -> Launch {
            fs::create_dir_all(root.join("node_modules/@openai/codex")).unwrap();
            // A minimal PE header is inspected by Launch::valid; this fixture is
            // never executable or started by any test.
            let mut bytes = vec![0; 256];
            bytes[..2].copy_from_slice(b"MZ");
            bytes[0x3c..0x40].copy_from_slice(&128u32.to_le_bytes());
            bytes[128..132].copy_from_slice(b"PE\0\0");
            bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
            let program = root.join("codex.exe");
            fs::write(&program, bytes).unwrap();
            fs::write(
                root.join("node_modules/@openai/codex/package.json"),
                serde_json::to_vec(&json!({"name":"@openai/codex","version":version})).unwrap(),
            )
            .unwrap();
            Launch {
                program,
                prefix: vec![],
                source: "managed".into(),
            }
        }
        fn snapshot(
            &self,
            previous: Option<String>,
            previous_override: Option<String>,
            resolved: Option<Launch>,
        ) {
            fs::write(self.current.program.parent().unwrap().join("previous-installation.json"),
                serde_json::to_vec(&json!({"agent":"codex","previous":previous,"previousOverride":previous_override,"previousResolved":resolved,"version":"codex-cli 0.162.0"})).unwrap()).unwrap();
        }
        fn record(&self) -> Rollback {
            rollback_record(&self.store, &self.root, "codex").unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Ok(root) = self.root.canonicalize() {
                let temporary = std::env::temp_dir().canonicalize().unwrap();
                assert_eq!(root.parent(), Some(temporary.as_path()));
                assert!(root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("supercode-recovery-"));
                fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn compatibility_distinguishes_fixture_coverage_unknown_releases_and_explicitly_unsupported_versions(
    ) {
        assert_eq!(compatibility("opencode", Some("1.18.23")).0, "covered");
        assert_eq!(compatibility("opencode", Some("1.18.24")).0, "unverified");
        assert_eq!(compatibility("codex", Some("0.162.0")).0, "unverified");
        assert_eq!(compatibility("pi", Some("0.60.0")).0, "unsupported");
        assert_eq!(compatibility("pi", Some("1.0.4")).0, "unverified");
        assert_eq!(compatibility("pi", None).0, "unverified");
        assert_eq!(compatibility("gemini", Some("0.63.0")).0, "covered");
        assert_eq!(compatibility("gemini", Some("0.62.0")).0, "unsupported");
        assert_eq!(compatibility("gemini", Some("0.64.0")).0, "unverified");
        let evidence: serde_json::Value =
            serde_json::from_str(include_str!("../../resources/agent-compatibility.json")).unwrap();
        assert_eq!(evidence["evidenceScope"], "controlled_protocol_fixtures");
        assert_eq!(evidence["liveAgentValidation"], false);
        assert_eq!(catalog().agents.len(), agents::IDS.len());
    }

    #[test]
    fn gemini_recovery_reads_bundled_metadata_and_preserves_node_script_arguments() {
        let fixture = Fixture::new();
        let package = fixture.root.join("node_modules/@google/gemini-cli");
        let script = package.join("bundle/gemini.js");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&script, b"// fixture, never executed").unwrap();
        fs::write(
            package.join("package.json"),
            br#"{"name":"@google/gemini-cli","version":"0.63.0"}"#,
        )
        .unwrap();
        let selected = Launch {
            program: fixture.old.program.clone(),
            prefix: vec![script.to_string_lossy().into_owned()],
            source: "managed".into(),
        };
        assert_eq!(
            static_version("gemini", &selected).as_deref(),
            Some("0.63.0")
        );
        let saved = serde_json::to_string(&selected).unwrap();
        assert_eq!(launch("gemini", &saved).unwrap().prefix, selected.prefix);
        assert!(launch("codex", &saved).is_err());
    }

    #[test]
    fn rollback_restores_both_settings_atomically_preserves_files_and_reads_versions_without_execution(
    ) {
        let fixture = Fixture::new();
        let override_launch =
            Fixture::executable(&fixture.root.join("external-selection"), "0.160.0");
        let old_override = serde_json::to_string(&override_launch).unwrap();
        fixture.snapshot(
            Some(serde_json::to_string(&fixture.old).unwrap()),
            Some(old_override.clone()),
            Some(override_launch),
        );
        fixture
            .store
            .set_setting(
                "agent_path_codex",
                &serde_json::to_string(&fixture.current).unwrap(),
            )
            .unwrap();
        fixture.store.set_setting("unrelated", "preserved").unwrap();
        let record = fixture.record();
        assert_eq!(
            static_version("codex", &record.target).as_deref(),
            Some("0.160.0")
        );
        fixture
            .store
            .restore_agent_configuration("codex", &record)
            .unwrap();
        assert_eq!(
            fixture
                .store
                .setting("agent_install_codex")
                .unwrap()
                .as_deref(),
            record.previous.previous.as_deref()
        );
        assert_eq!(
            fixture
                .store
                .setting("agent_path_codex")
                .unwrap()
                .as_deref(),
            Some(old_override.as_str())
        );
        assert_eq!(
            fixture.store.setting("unrelated").unwrap().as_deref(),
            Some("preserved")
        );
        assert!(fixture.current.program.is_file() && fixture.old.program.is_file());
        assert!(fixture
            .store
            .restore_agent_configuration("codex", &record)
            .is_err());
    }

    #[test]
    fn rollback_preserves_absent_and_explicit_null_overrides_and_recovers_a_broken_new_binary() {
        let fixture = Fixture::new();
        fs::remove_file(&fixture.current.program).unwrap();
        fixture
            .store
            .set_setting("agent_path_codex", "current-selection")
            .unwrap();
        let record = fixture.record();
        fixture
            .store
            .restore_agent_configuration("codex", &record)
            .unwrap();
        assert_eq!(fixture.store.setting("agent_path_codex").unwrap(), None);
        let fixture = Fixture::new();
        fixture.snapshot(
            Some(serde_json::to_string(&fixture.old).unwrap()),
            Some("null".into()),
            Some(fixture.old.clone()),
        );
        fixture
            .store
            .restore_agent_configuration("codex", &fixture.record())
            .unwrap();
        assert_eq!(
            fixture
                .store
                .setting("agent_path_codex")
                .unwrap()
                .as_deref(),
            Some("null")
        );
    }

    #[test]
    fn active_background_children_block_rollback_inside_the_same_transaction() {
        let fixture = Fixture::new();
        let record = fixture.record();
        assert_eq!(fixture.store.running().unwrap(), 0);
        fixture.store.0.lock().unwrap().execute("INSERT INTO temp.connection_usage(owner,session_id,connection_id,thread_id,turn_id,active,definitive) VALUES('worker',NULL,'route','child','turn',1,1)", []).unwrap();
        assert!(fixture
            .store
            .restore_agent_configuration("codex", &record)
            .unwrap_err()
            .contains("后台子任务"));
        assert_eq!(
            fixture
                .store
                .setting("agent_install_codex")
                .unwrap()
                .as_deref(),
            Some(record.current.as_str())
        );
        assert_eq!(fixture.store.setting("agent_path_codex").unwrap(), None);
    }

    #[test]
    fn rollback_rejects_cross_agent_records_missing_previous_targets_and_non_utf8_without_modifying_settings(
    ) {
        let fixture = Fixture::new();
        let current = fixture.store.setting("agent_install_codex").unwrap();
        let path = fixture
            .current
            .program
            .parent()
            .unwrap()
            .join("previous-installation.json");
        fs::write(
            &path,
            serde_json::to_vec(
                &json!({"agent":"claude","previous":serde_json::to_string(&fixture.old).unwrap()}),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(rollback_record(&fixture.store, &fixture.root, "codex").is_err());
        fixture.snapshot(None, None, None);
        assert!(rollback_record(&fixture.store, &fixture.root, "codex").is_err());
        fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(rollback_record(&fixture.store, &fixture.root, "codex")
            .err()
            .unwrap()
            .contains("UTF-8"));
        assert!(managed_root(
            &fixture.root,
            "codex",
            &Launch {
                program: fixture.root.join("agents/codex/not-a-uuid/codex.exe"),
                prefix: vec![],
                source: "managed".into()
            }
        )
        .is_err());
        assert_eq!(
            fixture.store.setting("agent_install_codex").unwrap(),
            current
        );
    }
}
