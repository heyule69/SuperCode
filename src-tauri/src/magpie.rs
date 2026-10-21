//! User-triggered Magpie integration. Its app and upstream credentials remain independent.
use crate::{ccswitch::Profile, providers, storage::Store, AppState};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};

type Result<T> = std::result::Result<T, String>;
const REVISION: &str = "magpie_revision";
const AGENTS: [&str; 5] = ["claude", "codex", "opencode", "pi", "gemini"];
pub(crate) fn managed(profile: &Profile) -> bool {
    profile.config["providerId"] == "magpie" && profile.config["source"] == "magpie"
}
fn revision(conn: &Connection) -> Result<u64> {
    conn.query_row("SELECT value FROM settings WHERE key=?1", [REVISION], |r| {
        r.get::<_, String>(0)
    })
    .optional()
    .map_err(|e| e.to_string())?
    .map(|s| s.parse().map_err(|_| "Magpie 连接状态无效，请刷新".into()))
    .unwrap_or(Ok(0))
}
fn advance(conn: &Connection, old: u64) -> Result<()> {
    let next = old.checked_add(1).ok_or("Magpie 连接状态已达上限")?;
    conn.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![REVISION,next.to_string()]).map_err(|e| e.to_string())?;
    Ok(())
}
fn settings(agent: &str, base: &str, models: &[String]) -> providers::Settings {
    let protocol = match agent {
        "claude" => "anthropic",
        "codex" => "responses",
        "gemini" => "gemini",
        _ => "chat",
    };
    providers::Settings {
        id: Some(format!("magpie:{agent}")),
        name: "Magpie".into(),
        agent: agent.into(),
        provider_id: "magpie".into(),
        plan: protocol.into(),
        protocol: protocol.into(),
        base_url: if matches!(protocol, "responses" | "chat") {
            format!("{base}/v1")
        } else {
            base.into()
        },
        model: models.first().cloned().unwrap_or_default(),
        models: models.into(),
        api_key: None,
        has_credential: false,
        official: false,
    }
}
impl Store {
    pub(crate) fn magpie_revision(&self) -> Result<u64> {
        let conn = self.0.lock().map_err(|e| e.to_string())?;
        revision(&conn)
    }
    pub(crate) fn add_magpie(
        &self,
        expected: u64,
        base: &str,
        models: &[String],
        metadata: &Value,
    ) -> Result<usize> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        if revision(&tx)? != expected {
            return Err("Magpie 连接已被其他窗口修改，请刷新后重试".into());
        }
        let mut added = 0;
        for agent in AGENTS {
            let id = format!("magpie:{agent}");
            let old: Option<(String, String)> = tx
                .query_row(
                    "SELECT agent,config FROM agent_profiles WHERE id=?1",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if let Some((owner, raw)) = old {
                let config = serde_json::from_str(&crate::credentials::open(&raw)?)
                    .map_err(|_| "Magpie 连接数据无效")?;
                if owner != agent
                    || !managed(&Profile {
                        id,
                        agent: owner,
                        name: String::new(),
                        config,
                    })
                {
                    return Err("Magpie 连接标识已被其他配置使用，未覆盖现有连接".into());
                }
                continue; // Never reset a saved model or mutate a running connection.
            }
            // Build and seal one route at a time; do not clone a large catalog five times in memory.
            let mut profile = providers::make_profile(settings(agent, base, models), None)?;
            crate::storage::ensure_connection_idle(&tx, &profile.id)?;
            profile.config["source"] = json!("magpie");
            profile.config["modelMetadata"] = metadata.clone();
            let raw = crate::credentials::seal(&profile.config.to_string())?;
            tx.execute(
                "INSERT INTO agent_profiles(id,agent,name,config) VALUES(?1,?2,?3,?4)",
                params![profile.id, profile.agent, profile.name, raw],
            )
            .map_err(|e| e.to_string())?;
            // Re-adding a stable ID must start fresh native sessions for its retained chats.
            tx.execute("UPDATE sessions SET native_id=NULL,turn_id=NULL WHERE connection_id=?1", [&profile.id]).map_err(|e|e.to_string())?;
            let mut order = crate::connection_order::order(&tx, agent)?;
            order.retain(|value| value != &profile.id);
            order.insert(0, profile.id);
            crate::connection_order::write_order(&tx, agent, &order)?;
            added += 1;
        }
        if added > 0 {
            advance(&tx, expected)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(added)
    }
    pub(crate) fn remove_magpie(&self) -> Result<usize> {
        let mut conn = self.0.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut query = tx
            .prepare("SELECT id,agent,name,config FROM agent_profiles")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut ids = Vec::new();
        for row in rows {
            let (id, agent, name, raw) = row.map_err(|e| e.to_string())?;
            let config = serde_json::from_str(&crate::credentials::open(&raw)?)
                .map_err(|_| "本地连接数据无效")?;
            let profile = Profile {
                id,
                agent,
                name,
                config,
            };
            if managed(&profile) {
                crate::storage::ensure_connection_idle(&tx, &profile.id)?;
                ids.push(profile.id);
            }
        }
        drop(query);
        for id in &ids {
            tx.execute("DELETE FROM agent_profiles WHERE id=?1", [id])
                .map_err(|e| e.to_string())?;
        }
        // Historical sessions keep their binding and visible messages. Never terminate Magpie.
        crate::connection_order::reconcile(&tx)?;
        advance(&tx, revision(&tx)?)?; // Also invalidate an in-flight add when no row exists yet.
        tx.commit().map_err(|e| e.to_string())?;
        Ok(ids.len())
    }
}

#[derive(Deserialize, Default)]
struct Preferences {
    #[serde(default)]
    port: u16,
}
fn read_port(path: &Path) -> Result<u16> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(3425),
        Err(_) => return Err("无法读取 Magpie 设置，请检查其文件权限".into()),
    };
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取 Magpie 设置")?;
    if bytes.len() > 1024 * 1024 {
        return Err("Magpie 设置超过 1MiB，未读取或修改".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "Magpie 设置不是 UTF-8，未更改编码")?;
    if text.starts_with('\u{feff}') {
        return Err("Magpie 设置含 UTF-8 BOM，未更改原文件".into());
    }
    let settings: Preferences =
        serde_json::from_str(text).map_err(|_| "Magpie 设置格式无效，请在 Magpie 中检查")?;
    Ok(if settings.port == 0 {
        3425
    } else {
        settings.port
    })
}
#[derive(Debug)]
struct Gateway {
    base: String,
    socket: String,
}
fn gateway(address: &str) -> Result<Gateway> {
    let url =
        reqwest::Url::parse(&format!("http://{address}")).map_err(|_| "Magpie 本机地址无效")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Magpie 本机地址无效".into());
    }
    let host = match url.host_str() {
        Some("localhost" | "127.0.0.1" | "0.0.0.0") => "127.0.0.1",
        Some("[::1]" | "[::]") => "[::1]",
        _ => return Err("自动接入只连接本机 Magpie，请在 Magpie 中启用本机网关".into()),
    };
    let port = url
        .port()
        .filter(|port| *port > 0)
        .ok_or("Magpie 本机地址需要有效端口")?;
    let socket = format!("{host}:{port}");
    Ok(Gateway {
        base: format!("http://{socket}"),
        socket,
    })
}
fn config_path(
    program: Option<&Path>,
    home: Option<PathBuf>,
    xdg: Option<PathBuf>,
) -> Option<PathBuf> {
    let installed = xdg
        .filter(|p| p.is_absolute())
        .map(|p| p.join("magpie"))
        .or_else(|| home.as_ref().map(|p| p.join(".config/magpie")));
    if let Some(parent) = program.and_then(Path::parent) {
        let data = parent.join("data");
        let shared = home.as_ref().is_some_and(|h| {
            [h.clone(), h.join("Downloads"), h.join("Desktop")]
                .iter()
                .any(|p| p == parent)
        });
        let installed_exists = installed.as_ref().is_some_and(|p| {
            p.join("settings.json").is_file() || p.join("providers.json").is_file()
        });
        if parent.join(".portable").is_file()
            || data.join("settings.json").is_file() && !(shared && installed_exists)
        {
            return Some(data.join("settings.json"));
        }
    }
    installed.map(|p| p.join("settings.json"))
}
fn executable(path: PathBuf) -> Option<PathBuf> {
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    #[cfg(windows)]
    {
        if !path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("exe"))
        {
            return None;
        }
        let mut magic = [0; 2];
        if std::fs::File::open(&path)
            .ok()?
            .read_exact(&mut magic)
            .is_err()
            || magic != *b"MZ"
        {
            return None;
        }
    }
    path.canonicalize().ok()
}
fn registered_executable(command: &str) -> Option<PathBuf> {
    let rest = command.trim().strip_prefix('"')?;
    let (program, _) = rest.split_once('"')?;
    executable(PathBuf::from(program))
}
fn discover() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        // Magpie registers its own GUI here. Read only the executable; never run registry shell text.
        for hive in [
            windows_registry::CURRENT_USER,
            windows_registry::LOCAL_MACHINE,
        ] {
            if let Some(path) = hive
                .open(r"Software\Classes\magpie\shell\open\command")
                .ok()
                .and_then(|key| key.get_string("").ok())
                .and_then(|command| registered_executable(&command))
            {
                return Some(path);
            }
        }
    }
    let name = if cfg!(windows) {
        "magpie.exe"
    } else {
        "magpie"
    };
    let mut candidates = Vec::new();
    for (variable, suffix) in [
        ("USERPROFILE", ".local/bin"),
        ("LOCALAPPDATA", "Programs/Magpie"),
        ("LOCALAPPDATA", "Magpie"),
        ("LOCALAPPDATA", "Microsoft/WinGet/Links"),
    ] {
        if let Some(dir) = std::env::var_os(variable) {
            candidates.push(PathBuf::from(dir).join(suffix).join(name));
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(
            std::env::split_paths(&paths)
                .take(128)
                .map(|dir| dir.join(name)),
        );
    }
    candidates.into_iter().find_map(executable)
}
fn local_gateway(program: Option<&Path>) -> Result<Gateway> {
    if let Ok(address) = std::env::var("MAGPIE_ADDR") {
        if !address.trim().is_empty() {
            return gateway(address.trim());
        }
    }
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let path = config_path(
        program,
        home,
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
    );
    let port = path
        .as_ref()
        .map(|p| read_port(p))
        .transpose()?
        .unwrap_or(3425);
    gateway(&format!("127.0.0.1:{port}"))
}
fn launch_command(program: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command
        .args(["gui", "providers"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for name in crate::process::HOST_TASK_ENV { command.env_remove(name); }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    // std::process::Child is deliberately not owned by an Agent runtime or killed on drop.
    command
}
fn launch(program: &Path) -> Result<()> {
    let mut child = launch_command(program).spawn().map_err(|_| "无法打开 Magpie，请从官方下载并运行桌面版")?;
    std::thread::sleep(Duration::from_millis(150));
    if child.try_wait().map_err(|_| "无法检查 Magpie 启动结果")?.is_some_and(|status|!status.success()) {
        return Err("Magpie 桌面版未能打开，请确认下载的是桌面版而非仅命令行版本。".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Store {
        Store::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }
    fn models() -> Vec<String> {
        vec!["vendor/first".into(), "group/daily[1m]".into()]
    }
    fn add(store: &Store) {
        assert_eq!(
            store
                .add_magpie(
                    store.magpie_revision().unwrap(),
                    "http://127.0.0.1:3425",
                    &models(),
                    &json!({})
                )
                .unwrap(),
            5
        );
    }
    fn ordinary(id: &str, agent: &str) -> Profile {
        let mut s = settings(agent, "https://example.com", &models());
        s.id = Some(id.into());
        s.provider_id = "custom".into();
        s.api_key = Some("fixture-only".into());
        providers::make_profile(s, None).unwrap()
    }
    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("supercode-magpie-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn adds_all_routes_atomically_without_api_inputs_and_preserves_model_choice() {
        let store = store();
        let metadata = json!({"vendor/first":{"displayName":"首个模型","contextWindow":1000000,"defaultReasoningEffort":"high"}});
        assert_eq!(
            store
                .add_magpie(0, "http://127.0.0.1:3425", &models(), &metadata)
                .unwrap(),
            5
        );
        for agent in AGENTS {
            let p = store
                .profile(agent, &format!("magpie:{agent}"))
                .unwrap()
                .unwrap();
            assert!(managed(&p));
            assert_eq!(providers::key(&p.config), Some("magpie"));
            assert!(!p.is_official());
            assert_eq!(p.config["modelMetadata"], metadata);
            assert_eq!(providers::model_ids(&p.config), models());
            assert_eq!(store.connection_orders().unwrap()[agent][0], p.id);
            assert!(providers::ensure_editable(&p)
                .unwrap_err()
                .contains("只支持添加或移除"));
            assert_eq!(
                p.config["baseUrl"],
                if matches!(agent, "codex" | "opencode" | "pi") {
                    "http://127.0.0.1:3425/v1"
                } else {
                    "http://127.0.0.1:3425"
                }
            );
        }
        let original = store.profile("codex", "magpie:codex").unwrap().unwrap();
        let mut selected = original.clone();
        selected.config["model"] = json!("group/daily[1m]");
        store.save_profile(&selected, Some(&original)).unwrap();
        assert_eq!(
            store
                .add_magpie(1, "http://127.0.0.1:4000", &models(), &json!({}))
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .profile("codex", "magpie:codex")
                .unwrap()
                .unwrap()
                .config,
            selected.config
        );
        assert_eq!(store.magpie_revision().unwrap(), 1);
    }
    #[test]
    fn collisions_and_invalid_catalogs_roll_back_every_route() {
        let store = store();
        let original = ordinary("magpie:pi", "pi");
        store.import_profiles(&[original.clone()]).unwrap();
        let before = store.connection_orders().unwrap();
        assert!(store
            .add_magpie(0, "http://127.0.0.1:3425", &models(), &json!({}))
            .unwrap_err()
            .contains("未覆盖"));
        assert_eq!(store.profiles().unwrap().len(), 1);
        assert_eq!(
            store.profile("pi", "magpie:pi").unwrap().unwrap().config,
            original.config
        );
        assert_eq!(store.connection_orders().unwrap(), before);
        assert!(store
            .add_magpie(0, "http://127.0.0.1:3425", &[], &json!({}))
            .is_err());
        assert_eq!(store.magpie_revision().unwrap(), 0);
    }
    #[test]
    fn removing_invalidates_inflight_add_and_restores_prior_defaults_without_deleting_history() {
        let store = store();
        let original = ordinary("my-api", "codex");
        store.import_profiles(&[original]).unwrap();
        store.select_profile("codex", Some("my-api")).unwrap();
        let before = store.connection_orders().unwrap();
        add(&store);
        let project = store.add_project(Path::new("D:/magpie-history")).unwrap();
        let chat = store
            .create_configured_session(&project.id, None, "codex", Some("magpie:codex"))
            .unwrap();
        store
            .save_message("saved", &chat.id, "user", "保留聊天", "message", &json!({}))
            .unwrap();
        store.bind_native(&chat.id,"old-magpie-native").unwrap();
        assert_eq!(store.remove_magpie().unwrap(), 5);
        assert_eq!(store.connection_orders().unwrap(), before);
        assert_eq!(store.messages(&chat.id, None).unwrap()[0].text, "保留聊天");
        assert_eq!(
            store.session(&chat.id).unwrap().connection_id.as_deref(),
            Some("magpie:codex")
        );
        assert!(store
            .add_magpie(1, "http://127.0.0.1:3425", &models(), &json!({}))
            .is_err());
        assert_eq!(store.profiles().unwrap().len(), 1);
        let stale = store.magpie_revision().unwrap();
        assert_eq!(store.remove_magpie().unwrap(), 0);
        assert!(store
            .add_magpie(stale, "http://127.0.0.1:3425", &models(), &json!({}))
            .is_err());
        add(&store);
        assert!(store.session(&chat.id).unwrap().native_id.is_none());
    }
    #[test]
    fn any_busy_agent_blocks_the_entire_removal() {
        let store = store();
        add(&store);
        let project = store.add_project(Path::new("D:/magpie-busy")).unwrap();
        let chat = store
            .create_configured_session(&project.id, None, "gemini", Some("magpie:gemini"))
            .unwrap();
        for status in ["starting", "running", "waiting"] {
            store.set_status(&chat.id, status, None).unwrap();
            assert!(store.remove_magpie().is_err());
            assert_eq!(store.profiles().unwrap().len(), 5);
            assert_eq!(store.magpie_revision().unwrap(), 1);
        }
        store.set_status(&chat.id, "idle", None).unwrap();
        assert_eq!(store.remove_magpie().unwrap(), 5);
    }
    #[test]
    fn stale_add_cannot_override_a_newer_window() {
        let store = store();
        add(&store);
        assert!(store
            .add_magpie(0, "http://127.0.0.1:9000", &models(), &json!({}))
            .is_err());
        assert_eq!(store.profiles().unwrap().len(), 5);
    }
    #[test]
    fn settings_are_read_only_bounded_utf8_and_follow_the_actual_port() {
        let folder = Folder::new();
        let path = folder.0.join("settings.bin");
        assert_eq!(read_port(&path).unwrap(), 3425);
        for (input, expected) in [
            (r#"{"port":0}"#, 3425),
            (r#"{"port":4567,"lanKey":"not-copied"}"#, 4567),
        ] {
            std::fs::write(&path, input.as_bytes()).unwrap();
            assert_eq!(read_port(&path).unwrap(), expected);
            assert_eq!(std::fs::read(&path).unwrap(), input.as_bytes());
        }
        for input in [
            vec![0xff],
            b"\xef\xbb\xbf{}".to_vec(),
            b"{\"port\":65536}".to_vec(),
            vec![b' '; 1024 * 1024 + 1],
        ] {
            std::fs::write(&path, &input).unwrap();
            assert!(read_port(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), input);
        }
    }
    #[test]
    fn only_local_gateway_addresses_are_used() {
        for address in ["localhost:3425", "0.0.0.0:3425", "127.0.0.1:3425"] {
            assert_eq!(gateway(address).unwrap().base, "http://127.0.0.1:3425");
        }
        assert_eq!(gateway("[::1]:4567").unwrap().base, "http://[::1]:4567");
        for address in [
            "remote.example:3425",
            "127.0.0.1:0",
            "user:secret@127.0.0.1:3425",
            "127.0.0.1:3425/api",
            "127.0.0.1:3425?key=x",
            "127.0.0.1",
        ] {
            assert!(gateway(address).is_err(), "{address}");
        }
    }
    #[test]
    fn portable_marker_and_xdg_paths_do_not_need_configuration_forms() {
        let folder = Folder::new();
        let exe = folder.0.join("magpie.exe");
        let home = folder.0.join("home");
        let xdg = folder.0.join("custom");
        assert_eq!(
            config_path(Some(&exe), Some(home.clone()), Some(xdg.clone())),
            Some(xdg.join("magpie/settings.json"))
        );
        std::fs::write(folder.0.join(".portable"), b"").unwrap();
        assert_eq!(
            config_path(Some(&exe), Some(home), None),
            Some(folder.0.join("data/settings.json"))
        );
    }
    #[test]
    fn registry_arguments_are_never_executed_as_shell_text() {
        let folder = Folder::new();
        let exe = folder.0.join("Magpie with spaces.exe");
        std::fs::write(&exe, b"MZfixture").unwrap();
        let registered = format!("\"{}\" \"%1\" & should-not-run", exe.display());
        assert_eq!(registered_executable(&registered), exe.canonicalize().ok());
        for text in [
            "magpie.exe --anything",
            "cmd.exe /c magpie",
            "\"relative.exe\"",
            "\"C:/missing.exe\"",
        ] {
            assert!(registered_executable(text).is_none());
        }
        let cmd = launch_command(&exe);
        assert_eq!(cmd.get_program(), exe.as_os_str());
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), vec!["gui", "providers"]);
    }
    #[tokio::test]
    async fn automatic_connection_fetches_one_large_directory_with_safe_metadata() {
        use axum::{routing::get, Json, Router};
        let payload = json!({"data":(0..5000).map(|i|json!({"id":format!("vendor/model-{i:04}"),"display_name":format!("模型{i}"),"context_length":1000000,"supported_reasoning_levels":[{"effort":"high"}],"default_reasoning_level":"high","apiKey":"not-copied"})).collect::<Vec<_>>()});
        let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = requests.clone();
        let router = Router::new().route(
            "/v1/models",
            get(move |headers: axum::http::HeaderMap| async move {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                assert_eq!(headers["authorization"], "Bearer magpie");
                Json(payload)
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let (models, metadata) =
            providers::fetch_models(&settings("codex", &base, &[]), Some("magpie"))
                .await
                .unwrap();
        server.abort();
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(models.len(), 5000);
        assert!(!metadata.to_string().contains("not-copied"));
        let store = store();
        assert_eq!(store.add_magpie(0, &base, &models, &metadata).unwrap(), 5);
        for p in store.profiles().unwrap() {
            assert_eq!(providers::model_ids(&p.config).len(), 5000);
            assert_eq!(
                p.config["modelMetadata"]["vendor/model-4999"]["displayName"],
                "模型4999"
            );
        }
    }
}
async fn listening(target: &Gateway) -> bool {
    tokio::time::timeout(
        Duration::from_millis(450),
        tokio::net::TcpStream::connect(&target.socket),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}
#[tauri::command]
pub async fn open_magpie() -> Result<()> {
    tokio::task::spawn_blocking(|| {
        launch(&discover().ok_or("未找到 Magpie，请先下载并运行一次桌面版，然后重试。")?)
    })
    .await
    .map_err(|_| "打开 Magpie 的任务已中断".to_owned())?
}
#[tauri::command]
pub async fn use_magpie(app: AppHandle) -> Result<Value> {
    let expected = app.state::<AppState>().store.magpie_revision()?;
    let (program, target) = tokio::task::spawn_blocking(|| {
        let program = discover();
        let target = local_gateway(program.as_deref())?;
        Ok::<_, String>((program, target))
    })
    .await
    .map_err(|_| "Magpie 检测已中断")??;
    if !listening(&target).await {
        let program =
            program.ok_or("未连接到 Magpie。请先下载并打开 Magpie，在其中添加供应商后再使用。")?;
        tokio::task::spawn_blocking(move || launch(&program))
            .await
            .map_err(|_| "打开 Magpie 的任务已中断")??;
        tokio::time::timeout(Duration::from_secs(12), async {
            while !listening(&target).await {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .map_err(|_| "Magpie 尚未启动本机网关，请打开 Magpie 检查后重试。")?;
    }
    let (models, metadata) =
        providers::fetch_models(&settings("codex", &target.base, &[]), Some("magpie"))
            .await
            .map_err(|error| {
                format!("无法读取 Magpie 模型，请在 Magpie 中检查供应商和网关。{error}")
            })?;
    let count = models.len();
    let app2 = app.clone();
    let added = tokio::task::spawn_blocking(move || {
        app2.state::<AppState>()
            .store
            .add_magpie(expected, &target.base, &models, &metadata)
    })
    .await
    .map_err(|_| "Magpie 连接保存已中断")??;
    let _ = app.emit("workspace-updated", ());
    Ok(json!({"models":count,"added":added}))
}
#[tauri::command]
pub async fn remove_magpie(app: AppHandle) -> Result<()> {
    let app2 = app.clone();
    tokio::task::spawn_blocking(move || app2.state::<AppState>().store.remove_magpie())
        .await
        .map_err(|_| "Magpie 连接移除已中断")??;
    let _ = app.emit("workspace-updated", ());
    Ok(())
}
