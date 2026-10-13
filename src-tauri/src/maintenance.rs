//! Explicit cleanup of retired managed installs. Chats, attachments and external files stay intact.
use crate::{
    agents::{Launch, IDS},
    storage::Store,
    AppState,
};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

type Result<T> = std::result::Result<T, String>;
#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    id: String,
    label: String,
    bytes: u64,
    reclaimable_bytes: u64,
    files: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    categories: Vec<Category>,
    pending_restore: bool,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleaned {
    removed_bytes: u64,
    removed_files: u64,
}
#[derive(Default, Clone, Copy)]
struct Size {
    bytes: u64,
    files: u64,
}
impl Size {
    fn add(&mut self, other: Self) {
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.files = self.files.saturating_add(other.files);
    }
}

fn linked(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
fn size(path: &Path, strict: bool, depth: usize, visited: &mut usize) -> Result<Size> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Size::default()),
        Err(error) => return Err(error.to_string()),
    };
    if linked(&metadata) {
        return if strict {
            Err("旧安装包含链接，未清理该目录".into())
        } else {
            Ok(Size::default())
        };
    }
    *visited += 1;
    if depth > 64 || *visited > 500_000 {
        return Err("目录过大，无法安全统计或清理".into());
    }
    if metadata.is_file() {
        return Ok(Size {
            bytes: metadata.len(),
            files: 1,
        });
    }
    let mut total = Size::default();
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            total.add(size(
                &entry.map_err(|e| e.to_string())?.path(),
                strict,
                depth + 1,
                visited,
            )?);
        }
    }
    Ok(total)
}
fn measure(path: &Path, strict: bool) -> Result<Size> {
    size(path, strict, 0, &mut 0)
}
fn json_file(path: &Path) -> Result<Value> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if linked(&metadata) || !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err("安装记录无效或包含链接".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) || std::str::from_utf8(&bytes).is_err() {
        return Err("安装记录不是 UTF-8 无 BOM，未修改其编码".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "安装记录 JSON 无效".into())
}
fn launch(raw: &str) -> Result<Option<Launch>> {
    if raw == "null" {
        return Ok(None);
    }
    serde_json::from_str(raw)
        .map(Some)
        .map_err(|_| "Agent 路径记录损坏，未清理安装".into())
}
fn install_root(base: &Path, path: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(base).ok()?;
    let mut parts = relative.components();
    let agent = parts.next()?.as_os_str().to_str()?;
    let id = parts.next()?.as_os_str().to_str()?;
    if !IDS.contains(&agent) || uuid::Uuid::parse_str(id).is_err() {
        return None;
    }
    Some(base.join(agent).join(id))
}
fn protect_launch(base: &Path, launch: &Launch, roots: &mut BTreeSet<PathBuf>) {
    for path in std::iter::once(launch.program.as_path()).chain(launch.prefix.iter().map(Path::new))
    {
        let normalized = crate::agents::ordinary(path.to_path_buf());
        if let Some(root) = install_root(base, &normalized) {
            roots.insert(root);
        }
        if let Ok(canonical) = path.canonicalize() {
            if let Some(root) = install_root(base, &crate::agents::ordinary(canonical)) {
                roots.insert(root);
            }
        }
    }
}
fn protected(store: &Store, base: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut roots = BTreeSet::new();
    for id in IDS {
        for key in [format!("agent_install_{id}"), format!("agent_path_{id}")] {
            if let Some(raw) = store.setting(&key)? {
                if let Some(launch) = launch(&raw)? {
                    protect_launch(base, &launch, &mut roots);
                }
            }
        }
    }
    // Preserve only the direct rollback target of each selected installation,
    // rather than recursively retaining every version ever installed.
    for root in roots.clone() {
        let path = root.join("previous-installation.json");
        if !path.exists() {
            continue;
        }
        let metadata = json_file(&path)?;
        for key in ["previous", "previousOverride"] {
            if let Some(raw) = metadata[key].as_str() {
                if let Some(launch) = launch(raw)? {
                    protect_launch(base, &launch, &mut roots);
                }
            }
        }
        if !metadata["previousResolved"].is_null() {
            let launch: Launch = serde_json::from_value(metadata["previousResolved"].clone())
                .map_err(|_| "回滚路径记录损坏，未清理安装")?;
            protect_launch(base, &launch, &mut roots);
        }
    }
    Ok(roots)
}
fn retired(store: &Store, data: &Path) -> Result<Vec<(PathBuf, Size)>> {
    let base = data.join("agents");
    if !base.exists() {
        return Ok(vec![]);
    }
    if linked(&fs::symlink_metadata(&base).map_err(|e| e.to_string())?) {
        return Err("安装目录包含链接，未清理".into());
    }
    let base = crate::agents::ordinary(base.canonicalize().map_err(|e| e.to_string())?);
    let protected = protected(store, &base)?;
    let mut candidates = vec![];
    for agent in IDS {
        let directory = base.join(agent);
        let metadata = match fs::symlink_metadata(&directory) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        };
        if linked(&metadata) || !metadata.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_none_or(|n| uuid::Uuid::parse_str(n).is_err())
                || protected.contains(&path)
            {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if linked(&metadata) || !metadata.is_dir() {
                continue;
            }
            let Ok(record) = json_file(&path.join("previous-installation.json")) else {
                continue;
            };
            if record["agent"] != agent || !record["version"].is_string() {
                continue;
            }
            let Ok(bytes) = measure(&path, true) else {
                continue;
            };
            let canonical =
                crate::agents::ordinary(path.canonicalize().map_err(|e| e.to_string())?);
            let expected =
                crate::agents::ordinary(directory.canonicalize().map_err(|e| e.to_string())?)
                    .join(path.file_name().unwrap());
            if canonical != expected || canonical.parent() != Some(expected.parent().unwrap()) {
                continue;
            }
            candidates.push((canonical, bytes));
        }
    }
    Ok(candidates)
}
fn inspect(store: &Store, data: &Path) -> Result<Summary> {
    let retired = retired(store, data)?;
    let reclaimed = retired.iter().map(|(_, size)| size.bytes).sum();
    let mut categories = vec![];
    for (id, label, relative) in [
        ("agents", "Agent 安装与缓存", "agents"),
        ("automation", "自动化工具", "automation"),
        ("attachments", "聊天附件", "attachments"),
        ("media", "媒体预览缓存", "media"),
        ("backups", "数据备份", "backups"),
        ("logs", "诊断日志", "logs"),
    ] {
        let size = measure(&data.join(relative), false)?;
        categories.push(Category {
            id: id.into(),
            label: label.into(),
            bytes: size.bytes,
            files: size.files,
            reclaimable_bytes: if id == "agents" { reclaimed } else { 0 },
        });
    }
    let mut database = Size::default();
    for name in ["supercode.db", "supercode.db-wal", "supercode.db-shm"] {
        database.add(measure(&data.join(name), false)?);
    }
    categories.push(Category {
        id: "database".into(),
        label: "聊天数据库".into(),
        bytes: database.bytes,
        files: database.files,
        reclaimable_bytes: 0,
    });
    let workspaces = {
        let conn = store.0.lock().map_err(|e| e.to_string())?;
        let mut query = conn.prepare("SELECT DISTINCT w.path FROM session_workspaces w JOIN sessions s ON s.id=w.session_id WHERE s.project_id IS NULL").map_err(|e| e.to_string())?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    let mut chats = Size::default();
    for path in workspaces {
        chats.add(measure(Path::new(&path), false)?);
    }
    categories.push(Category {
        id: "workspaces".into(),
        label: "无项目聊天文件".into(),
        bytes: chats.bytes,
        files: chats.files,
        reclaimable_bytes: 0,
    });
    Ok(Summary {
        categories,
        pending_restore: data.join("pending-restore/manifest.json").is_file(),
    })
}
fn clean(store: &Store, data: &Path) -> Result<Cleaned> {
    if store.running()? != 0 {
        return Err("仍有任务或后台子任务运行，请空闲时清理".into());
    }
    let metadata = fs::symlink_metadata(data).map_err(|e| e.to_string())?;
    if linked(&metadata) {
        return Err("数据目录包含链接，未执行清理".into());
    }
    let mut result = Cleaned::default();
    for (path, _) in retired(store, data)? {
        let bytes = measure(&path, true)?;
        fs::remove_dir_all(&path).map_err(|e| format!("旧安装清理失败：{e}"))?;
        result.removed_bytes += bytes.bytes;
        result.removed_files += bytes.files;
    }
    Ok(result)
}
#[tauri::command]
pub async fn storage_summary(app: AppHandle) -> Result<Summary> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        inspect(&state.store, &state.data_dir)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn clean_retired_storage(app: AppHandle) -> Result<Cleaned> {
    let agents = app.state::<crate::agents::Agents>();
    let _operation = agents
        .operation
        .try_write()
        .map_err(|_| "Agent 正在启动、安装或配置，请稍后清理")?;
    let worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = worker.state::<AppState>();
        clean(&state.store, &state.data_dir)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (Store, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("supercode-maintenance-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        (
            Store::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap(),
            root,
        )
    }
    fn install(data: &Path) -> (PathBuf, Launch) {
        let root = data
            .join("agents/codex")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let program = root.join("fixture.exe");
        fs::write(&program, b"fixture bytes; never executable").unwrap();
        fs::write(root.join("previous-installation.json"),serde_json::to_vec(&json!({"agent":"codex","version":"fixture","previous":null,"previousOverride":null,"previousResolved":null})).unwrap()).unwrap();
        (
            root,
            Launch {
                program,
                prefix: vec![],
                source: "managed".into(),
            },
        )
    }
    fn remove_fixture(root: &Path) {
        assert_eq!(
            root.canonicalize().unwrap().parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cleanup_preserves_selected_rollback_external_unmarked_and_chat_files() {
        let (store, root) = fixture();
        let (current, current_launch) = install(&root);
        let (previous, previous_launch) = install(&root);
        let (retired, _) = install(&root);
        store
            .set_setting(
                "agent_install_codex",
                &serde_json::to_string(&current_launch).unwrap(),
            )
            .unwrap();
        fs::write(current.join("previous-installation.json"),serde_json::to_vec(&json!({"agent":"codex","version":"fixture","previous":serde_json::to_string(&previous_launch).unwrap(),"previousOverride":null,"previousResolved":previous_launch})).unwrap()).unwrap();
        let external = root.join("external.exe");
        fs::write(&external, b"user program").unwrap();
        store
            .set_setting(
                "agent_path_codex",
                &serde_json::to_string(&Launch {
                    program: external.clone(),
                    prefix: vec![],
                    source: "configured".into(),
                })
                .unwrap(),
            )
            .unwrap();
        let unmarked = root
            .join("agents/codex")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&unmarked).unwrap();
        fs::write(unmarked.join("user.txt"), b"preserved").unwrap();
        fs::create_dir_all(root.join("attachments")).unwrap();
        fs::write(root.join("attachments/chat.png"), b"attachment").unwrap();
        let before = inspect(&store, &root).unwrap();
        assert!(
            before
                .categories
                .iter()
                .find(|c| c.id == "agents")
                .unwrap()
                .reclaimable_bytes
                > 0
        );
        let result = clean(&store, &root).unwrap();
        assert!(result.removed_files >= 2);
        assert!(!retired.exists());
        assert!(current.exists() && previous.exists() && external.exists() && unmarked.exists());
        assert!(root.join("attachments/chat.png").exists());
        assert_eq!(
            inspect(&store, &root)
                .unwrap()
                .categories
                .iter()
                .find(|c| c.id == "agents")
                .unwrap()
                .reclaimable_bytes,
            0
        );
        remove_fixture(&root);
    }
    #[test]
    fn background_child_and_corrupt_selection_block_cleanup_before_any_deletion() {
        let (store, root) = fixture();
        let (old, _) = install(&root);
        store.observe_connection_usage("owner",Some("chat"),"api",&json!({"method":"turn/started","params":{"threadId":"child","turn":{"id":"turn"}}})).unwrap();
        assert!(clean(&store, &root).is_err());
        assert!(old.exists());
        store.clear_connection_usage("owner").unwrap();
        store
            .set_setting("agent_install_codex", "invalid-json")
            .unwrap();
        assert!(clean(&store, &root).is_err());
        assert!(old.exists());
        remove_fixture(&root);
    }
}
