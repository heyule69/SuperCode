//! Consistent SQLite snapshots and validated restore on the next application start.
use crate::{storage::Store, AppState};
use rusqlite::{Connection, DatabaseName, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

pub const SCHEMA_VERSION: u32 = 1;
const DB_LIMIT: u64 = 512 * 1024 * 1024;
const FRONTEND_LIMIT: u64 = 8 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: u32,
    schema_version: u32,
    app_version: String,
    database_sha256: String,
    frontend_sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredFrontend {
    pub token: String,
    pub entries: BTreeMap<String, String>,
}

fn hash_file(path: &Path) -> Result<String> {
    let mut reader = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn frontend_bytes(entries: &BTreeMap<String, String>) -> Result<Vec<u8>> {
    if entries.len() > 512
        || entries
            .keys()
            .any(|key| !key.starts_with("supercode.") || key.len() > 1024)
    {
        return Err("备份中的界面设置无效".into());
    }
    let bytes = serde_json::to_vec(entries).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > FRONTEND_LIMIT {
        return Err("界面设置与草稿超过 8 MB".into());
    }
    Ok(bytes)
}
fn validate_database(path: &Path, credentials: bool) -> Result<()> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "备份数据库无法读取")?;
    let valid: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|_| "备份数据库损坏")?;
    let schema: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if valid != "ok" || schema > SCHEMA_VERSION {
        return Err("备份损坏或来自较新的数据库版本".into());
    }
    let invalid: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type IN ('trigger','view') OR upper(sql) LIKE '%VIRTUAL TABLE%')", [], |row| row.get(0)).map_err(|e| e.to_string())?;
    if invalid {
        return Err("备份包含不支持的数据库结构".into());
    }
    for table in ["sessions", "messages", "settings", "agent_profiles"] {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("所选文件不是 SuperCode 备份".into());
        }
    }
    let broken = conn
        .prepare("PRAGMA foreign_key_check")
        .map_err(|e| e.to_string())?
        .exists([])
        .map_err(|e| e.to_string())?;
    if broken {
        return Err("备份的聊天关联数据损坏".into());
    }
    if credentials {
        let mut statement = conn
            .prepare("SELECT config FROM agent_profiles")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let config = crate::credentials::open(&row.map_err(|e| e.to_string())?)
                .map_err(|_| "备份凭据只能由原 Windows 用户恢复")?;
            let config: serde_json::Value =
                serde_json::from_str(&config).map_err(|_| "备份连接配置不是有效 JSON")?;
            if !config.is_object() {
                return Err("备份连接配置无效".into());
            }
        }
    }
    Ok(())
}

fn preflight_database(path: &Path) -> Result<()> {
    // Run the same complete migration used at startup on our private staged copy.
    // Table names and a checksum alone cannot establish schema compatibility.
    validate_database(path, true)?;
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    let prepared =
        Store::from_connection(conn).map_err(|_| "备份数据库无法迁移为当前版本，原数据已保留")?;
    // CREATE IF NOT EXISTS also accepts a table with missing columns. Validate
    // the complete current schema before it can replace the live database.
    {
        let conn = prepared.0.lock().map_err(|e| e.to_string())?;
        for query in [
            "SELECT id,name,path FROM projects LIMIT 0",
            "SELECT id,project_id,title,agent,model,native_id,status,updated_at,turn_id,archived,connection_id FROM sessions LIMIT 0",
            "SELECT seq,id,session_id,role,text,kind,data FROM messages LIMIT 0",
            "SELECT seq,id,session_id,payload,status,error FROM followups LIMIT 0",
            "SELECT followup_id,token,previous_status FROM followup_edits LIMIT 0",
            "SELECT key,value FROM settings LIMIT 0",
            "SELECT id,name,position FROM sidebar_sections LIMIT 0",
            "SELECT kind,id,pinned,unread,section_id,removed FROM sidebar_items LIMIT 0",
            "SELECT session_id,turn_id,agent,model,data,updated_at FROM usage_records LIMIT 0",
            "SELECT session_id,summary,through_seq FROM session_context LIMIT 0",
            "SELECT id,agent,name,config FROM agent_profiles LIMIT 0",
            "SELECT session_id,path FROM session_workspaces LIMIT 0",
        ] {
            conn.prepare(query).map_err(|_| "备份数据库结构与当前版本不兼容，原数据已保留")?;
        }
    }
    after_open(&prepared).map_err(|e| e.to_string())?;
    drop(prepared);
    validate_database(path, true)?;
    Ok(())
}
struct CheckedDatabase(PathBuf);
impl Drop for CheckedDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}
fn checked_copy(path: &Path) -> Result<CheckedDatabase> {
    validate_database(path, true)?;
    let temporary = path
        .parent()
        .ok_or("备份目录无效")?
        .join(format!("preflight-{}.db", uuid::Uuid::new_v4()));
    let checked = CheckedDatabase(temporary);
    let original = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    original
        .backup(DatabaseName::Main, &checked.0, None)
        .map_err(|e| e.to_string())?;
    preflight_database(&checked.0)?;
    Ok(checked)
}
fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::File::create(path).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}
fn owned_directory(path: &Path, parent: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    let Ok(root) = parent.canonicalize() else {
        return false;
    };
    path.canonicalize()
        .is_ok_and(|resolved| resolved.parent() == Some(root.as_path()))
}
fn cleanup_fixed(path: &Path, parent: &Path) {
    if !owned_directory(path, parent) {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let preflight = name
            .strip_prefix("preflight-")
            .and_then(|name| name.split_once(".db"))
            .is_some_and(|(id, suffix)| {
                uuid::Uuid::parse_str(id).is_ok() && matches!(suffix, "" | "-wal" | "-shm")
            });
        if !preflight
            && ![
                "database.db",
                "database.db-wal",
                "database.db-shm",
                "frontend.json",
                "completed.json",
            ]
            .contains(&name.as_ref())
        {
            continue;
        }
        // Unknown files and links are retained; cleanup never follows them.
        if entry
            .file_type()
            .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
        {
            let _ = fs::remove_file(entry.path());
        }
    }
    let _ = fs::remove_dir(path);
}
fn retire_completed(parent: &Path) {
    let pending = parent.join("pending-restore");
    if owned_directory(&pending, parent) && !pending.join("manifest.json").exists() {
        let retired = parent.join(format!("completed-restore-{}", uuid::Uuid::new_v4()));
        // Both resolved source and destination parents are the managed data root.
        if fs::rename(&pending, &retired).is_ok() {
            cleanup_fixed(&retired, parent);
        } else {
            cleanup_fixed(&pending, parent);
        }
    }
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten().take(1024) {
            if entry
                .file_name()
                .to_string_lossy()
                .strip_prefix("completed-restore-")
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
            {
                cleanup_fixed(&entry.path(), parent);
            }
        }
    }
}

fn snapshot(conn: &Connection, directory: &Path, reason: &str) -> Result<PathBuf> {
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let path = directory.join(format!(
        "{reason}-{}-{}.db",
        crate::storage::now(),
        uuid::Uuid::new_v4()
    ));
    if let Err(error) = conn.backup(DatabaseName::Main, &path, None) {
        let _ = fs::remove_file(&path);
        return Err(error.to_string());
    }
    Ok(path)
}
fn retain_snapshots(directory: &Path) -> Result<()> {
    let mut files = fs::read_dir(directory)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry.path().extension().is_some_and(|ext| ext == "db")
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| std::cmp::Reverse(entry.metadata().and_then(|m| m.modified()).ok()));
    for entry in files.into_iter().skip(5) {
        fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn startup(path: &Path) -> Result<()> {
    let parent = path.parent().ok_or("数据库目录无效")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    retire_completed(parent);
    let pending = parent.join("pending-restore");
    let manifest_path = pending.join("manifest.json");
    if manifest_path.is_file() {
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(&manifest_path).map_err(|e| e.to_string())?)
                .map_err(|_| "待恢复备份记录损坏")?;
        let database = pending.join("database.db");
        let frontend = fs::read(pending.join("frontend.json")).map_err(|e| e.to_string())?;
        if manifest.format != 1
            || hash_file(&database)? != manifest.database_sha256
            || hash(&frontend) != manifest.frontend_sha256
        {
            return Err("待恢复备份校验失败，原数据已保留".into());
        }
        let normalized = checked_copy(&database)?;
        let entries: BTreeMap<String, String> =
            serde_json::from_slice(&frontend).map_err(|_| "备份草稿无效")?;
        frontend_bytes(&entries)?;
        let mut conn = Connection::open(path).map_err(|e| e.to_string())?;
        if conn
            .query_row::<bool, _, _>(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='sessions')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?
        {
            snapshot(&conn, &parent.join("backups"), "before-restore")?;
        }
        conn.restore(
            DatabaseName::Main,
            &normalized.0,
            None::<fn(rusqlite::backup::Progress)>,
        )
        .map_err(|e| e.to_string())?;
        // Frontend writes occur before React reads preferences or creates a draft map.
        let restored = RestoredFrontend {
            token: uuid::Uuid::new_v4().to_string(),
            entries,
        };
        write_synced(
            &parent.join("restored-frontend.json"),
            &serde_json::to_vec(&restored).map_err(|e| e.to_string())?,
        )?;
        drop(normalized);
        // Commit once. An interrupted or locked-file cleanup cannot replay the
        // restored database or prevent a later independent restore.
        fs::rename(&manifest_path, pending.join("completed.json")).map_err(|e| e.to_string())?;
        retire_completed(parent);
    }
    if path.is_file() {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if version > SCHEMA_VERSION {
            return Err("数据库来自较新版本，请使用较新的 SuperCode 打开".into());
        }
        let known: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='sessions')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if known && version < SCHEMA_VERSION {
            snapshot(&conn, &parent.join("backups"), "before-upgrade")?;
        }
    }
    if parent.join("backups").is_dir() {
        retain_snapshots(&parent.join("backups"))?;
    }
    Ok(())
}
pub fn before_open(path: &Path) -> rusqlite::Result<()> {
    startup(path).map_err(|error| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error)))
    })
}
pub fn after_open(store: &Store) -> rusqlite::Result<()> {
    store
        .0
        .lock()
        .map_err(|_| rusqlite::Error::InvalidQuery)?
        .pragma_update(None, "user_version", SCHEMA_VERSION)
}

fn export(
    conn: &Connection,
    entries: BTreeMap<String, String>,
    destination: &Path,
    parent: &Path,
) -> Result<()> {
    if !destination.is_absolute()
        || destination
            .extension()
            .is_none_or(|ext| ext != "supercode-backup")
    {
        return Err("请选择 .supercode-backup 备份文件".into());
    }
    let bytes = frontend_bytes(&entries)?;
    let database = snapshot(conn, parent, "export")?;
    let result = (|| {
        if fs::metadata(&database).map_err(|e| e.to_string())?.len() > DB_LIMIT {
            return Err("数据库超过备份大小限制".into());
        }
        let manifest = Manifest {
            format: 1,
            schema_version: SCHEMA_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            database_sha256: hash_file(&database)?,
            frontend_sha256: hash(&bytes),
        };
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|_| "无法创建备份，请选择新文件名")?;
        let mut archive = ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        archive
            .start_file("database.db", options)
            .map_err(|e| e.to_string())?;
        std::io::copy(
            &mut fs::File::open(&database).map_err(|e| e.to_string())?,
            &mut archive,
        )
        .map_err(|e| e.to_string())?;
        archive
            .start_file("frontend.json", options)
            .map_err(|e| e.to_string())?;
        archive.write_all(&bytes).map_err(|e| e.to_string())?;
        archive
            .start_file("manifest.json", options)
            .map_err(|e| e.to_string())?;
        archive
            .write_all(&serde_json::to_vec(&manifest).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        archive
            .finish()
            .map_err(|e| e.to_string())?
            .sync_all()
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    let _ = fs::remove_file(database);
    result
}
fn stage(source: &Path, parent: &Path) -> Result<()> {
    let file = fs::File::open(source).map_err(|_| "无法读取备份文件")?;
    if file.metadata().map_err(|e| e.to_string())?.len() > DB_LIMIT + FRONTEND_LIMIT {
        return Err("备份文件过大".into());
    }
    let mut archive = ZipArchive::new(file).map_err(|_| "备份文件格式无效")?;
    if archive.len() != 3 {
        return Err("备份内容不完整".into());
    }
    // Fixed members only: never extract archive-controlled paths or symlinks.
    let staging = parent.join(format!("restore-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        for (name, limit) in [
            ("database.db", DB_LIMIT),
            ("frontend.json", FRONTEND_LIMIT),
            ("manifest.json", 8192),
        ] {
            let member = archive.by_name(name).map_err(|_| "备份缺少必要文件")?;
            if member.is_dir() || member.size() > limit {
                return Err("备份条目超过大小限制".into());
            }
            let mut output = fs::File::create(staging.join(name)).map_err(|e| e.to_string())?;
            if std::io::copy(&mut member.take(limit + 1), &mut output).map_err(|e| e.to_string())?
                > limit
            {
                return Err("备份解压大小无效".into());
            }
            output.sync_all().map_err(|e| e.to_string())?;
        }
        let manifest: Manifest = serde_json::from_slice(
            &fs::read(staging.join("manifest.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|_| "备份记录无效")?;
        let frontend = fs::read(staging.join("frontend.json")).map_err(|e| e.to_string())?;
        if manifest.format != 1
            || manifest.schema_version > SCHEMA_VERSION
            || hash_file(&staging.join("database.db"))? != manifest.database_sha256
            || hash(&frontend) != manifest.frontend_sha256
        {
            return Err("备份完整性校验失败".into());
        }
        frontend_bytes(&serde_json::from_slice(&frontend).map_err(|_| "备份草稿无效")?)?;
        preflight_database(&staging.join("database.db"))?;
        // Preflight normalizes interrupted tasks and legacy credentials, so bind
        // the pending record to the normalized bytes that will actually restore.
        let manifest = Manifest {
            database_sha256: hash_file(&staging.join("database.db"))?,
            schema_version: SCHEMA_VERSION,
            ..manifest
        };
        write_synced(
            &staging.join("manifest.json"),
            &serde_json::to_vec(&manifest).map_err(|e| e.to_string())?,
        )?;
        retire_completed(parent);
        let pending = parent.join("pending-restore");
        if pending.exists() {
            return Err(if pending.join("manifest.json").is_file() {
                "已有待恢复备份，请先重新打开应用完成恢复"
            } else {
                "旧恢复临时文件暂时被占用，请稍后重试"
            }
            .into());
        }
        fs::rename(&staging, &pending).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        for name in ["database.db", "frontend.json", "manifest.json"] {
            let _ = fs::remove_file(staging.join(name));
        }
        let _ = fs::remove_dir(staging);
    }
    result
}

#[tauri::command]
pub async fn export_data_backup(
    path: String,
    entries: BTreeMap<String, String>,
    app: AppHandle,
) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        // A dedicated SQLite reader observes a consistent WAL snapshot without
        // holding the application's storage mutex while hashing/compressing it.
        let conn = Connection::open_with_flags(
            state.data_dir.join("supercode.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| e.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        export(
            &conn,
            entries,
            Path::new(&path),
            &state.data_dir.join("backups"),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn prepare_data_restore(path: String, app: AppHandle) -> Result<()> {
    let agents = app.state::<crate::agents::Agents>();
    let _gate = agents
        .operation
        .try_write()
        .map_err(|_| "正在进行配置或安装操作")?;
    if app.state::<AppState>().store.running()? > 0 {
        return Err("请先完成或停止全部任务再恢复备份".into());
    }
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        stage(Path::new(&path), &handle.state::<AppState>().data_dir)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn restored_frontend(app: AppHandle) -> Result<Option<RestoredFrontend>> {
    let path = app
        .state::<AppState>()
        .data_dir
        .join("restored-frontend.json");
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > FRONTEND_LIMIT + 1024 {
        return Err("恢复设置过大".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "恢复设置无效".into())
}
#[tauri::command]
pub fn acknowledge_frontend_restore(token: String, app: AppHandle) -> Result<()> {
    if let Some(saved) = restored_frontend(app.clone())? {
        if saved.token == token {
            fs::remove_file(
                app.state::<AppState>()
                    .data_dir
                    .join("restored-frontend.json"),
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("supercode-backup-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        root
    }

    // Owned UUID test trees only. Check the boundary before recursive cleanup.
    fn remove_test_root(root: &Path) {
        assert!(owned_directory(root, &std::env::temp_dir()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("supercode-backup-"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_hash_valid_incompatible_schema_and_invalid_profile_json() {
        let root = temporary_root();
        let path = root.join("supercode.db");
        let live = Store::open(&path).unwrap();
        live.set_setting("live", "preserved").unwrap();
        for (index, invalid) in [
            "ALTER TABLE messages DROP COLUMN text",
            "ALTER TABLE agent_profiles DROP COLUMN name",
            "INSERT INTO agent_profiles(id,agent,name,config) VALUES('invalid','codex','invalid','not JSON')",
        ].iter().enumerate() {
            let invalid_store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
            invalid_store.0.lock().unwrap().execute_batch(invalid).unwrap();
            let archive = root.join(format!("invalid-{index}.supercode-backup"));
            export(&invalid_store.0.lock().unwrap(), BTreeMap::new(), &archive, &root.join("backups")).unwrap();
            assert!(stage(&archive, &root).is_err(), "must reject {invalid}");
            assert!(!root.join("pending-restore").exists());
            assert_eq!(live.setting("live").unwrap().as_deref(), Some("preserved"));
        }
        drop(live);
        remove_test_root(&root);
    }

    #[test]
    fn startup_preflight_never_changes_live_or_pending_bytes_on_failure() {
        let root = temporary_root();
        let path = root.join("supercode.db");
        let live = Store::open(&path).unwrap();
        live.set_setting("live", "preserved").unwrap();
        let archive = root.join("saved.supercode-backup");
        export(
            &live.0.lock().unwrap(),
            BTreeMap::new(),
            &archive,
            &root.join("backups"),
        )
        .unwrap();
        stage(&archive, &root).unwrap();
        let pending = root.join("pending-restore");
        let database = pending.join("database.db");
        let conn = Connection::open(&database).unwrap();
        conn.execute_batch("ALTER TABLE agent_profiles DROP COLUMN name")
            .unwrap();
        drop(conn);
        let manifest_path = pending.join("manifest.json");
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest.database_sha256 = hash_file(&database).unwrap();
        write_synced(&manifest_path, &serde_json::to_vec(&manifest).unwrap()).unwrap();
        let before = hash_file(&database).unwrap();
        assert!(before_open(&path).is_err());
        assert_eq!(hash_file(&database).unwrap(), before);
        assert_eq!(live.setting("live").unwrap().as_deref(), Some("preserved"));
        assert!(!fs::read_dir(&pending).unwrap().flatten().any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with("preflight-")));
        drop(live);
        remove_test_root(&root);
    }

    #[test]
    fn completed_orphan_cleanup_never_replays_data_and_allows_later_restore() {
        let root = temporary_root();
        let path = root.join("supercode.db");
        let live = Store::open(&path).unwrap();
        live.set_setting("live", "newer").unwrap();
        let archive = root.join("saved.supercode-backup");
        export(
            &live.0.lock().unwrap(),
            BTreeMap::new(),
            &archive,
            &root.join("backups"),
        )
        .unwrap();
        let pending = root.join("pending-restore");
        fs::create_dir(&pending).unwrap();
        fs::write(pending.join("database.db"), b"obsolete payload").unwrap();
        fs::write(pending.join("frontend.json"), b"{}").unwrap();
        fs::write(pending.join("completed.json"), b"{}").unwrap();
        fs::write(pending.join("unknown.txt"), b"retain unknown files").unwrap();
        before_open(&path).unwrap();
        assert!(!pending.exists());
        assert_eq!(live.setting("live").unwrap().as_deref(), Some("newer"));
        assert!(fs::read_dir(&root)
            .unwrap()
            .flatten()
            .any(|entry| entry.path().join("unknown.txt").is_file()));
        stage(&archive, &root).unwrap();
        assert!(pending.join("manifest.json").is_file());
        drop(live);
        remove_test_root(&root);
    }

    #[cfg(windows)]
    #[test]
    fn locked_completed_payload_is_retried_without_blocking_startup() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = temporary_root();
        let path = root.join("supercode.db");
        let live = Store::open(&path).unwrap();
        live.set_setting("live", "preserved").unwrap();
        let archive = root.join("saved.supercode-backup");
        export(
            &live.0.lock().unwrap(),
            BTreeMap::new(),
            &archive,
            &root.join("backups"),
        )
        .unwrap();
        let pending = root.join("pending-restore");
        fs::create_dir(&pending).unwrap();
        let payload = pending.join("database.db");
        fs::write(&payload, b"completed payload").unwrap();
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&payload)
            .unwrap();
        before_open(&path).unwrap();
        assert_eq!(live.setting("live").unwrap().as_deref(), Some("preserved"));
        drop(held);
        // Regardless of whether Windows allowed the parent rename while locked,
        // releasing the file lets the next restore retire the old payload.
        stage(&archive, &root).unwrap();
        assert!(pending.join("manifest.json").is_file());
        drop(live);
        remove_test_root(&root);
    }
    #[test]
    fn restores_consistent_wal_snapshot_and_frontend_before_migration() {
        let parent =
            std::env::temp_dir().join(format!("supercode-backup-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&parent).unwrap();
        let path = parent.join("supercode.db");
        let store = Store::from_connection(Connection::open(&path).unwrap()).unwrap();
        store.set_setting("test-value", "before").unwrap();
        let backup = parent.join("saved.supercode-backup");
        let reader = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        export(
            &reader,
            BTreeMap::from([("supercode.draft.v2:chat".into(), "draft".into())]),
            &backup,
            &parent.join("backups"),
        )
        .unwrap();
        drop(reader);
        store.set_setting("test-value", "after").unwrap();
        stage(&backup, &parent).unwrap();
        drop(store);
        before_open(&path).unwrap();
        let reopened = Store::open(&path).unwrap();
        assert_eq!(
            reopened.setting("test-value").unwrap().as_deref(),
            Some("before")
        );
        let frontend: RestoredFrontend =
            serde_json::from_slice(&fs::read(parent.join("restored-frontend.json")).unwrap())
                .unwrap();
        assert_eq!(frontend.entries["supercode.draft.v2:chat"], "draft");
        assert!(parent.join("backups").is_dir());
        drop(reopened);
        fs::remove_dir_all(&parent).unwrap();
    }
    #[test]
    fn rejects_unknown_schema_untrusted_frontend_and_corrupted_database() {
        assert!(frontend_bytes(&BTreeMap::from([("other.app".into(), "data".into())])).is_err());
        let parent =
            std::env::temp_dir().join(format!("supercode-backup-invalid-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&parent).unwrap();
        let path = parent.join("invalid.db");
        fs::write(&path, b"invalid").unwrap();
        assert!(validate_database(&path, false).is_err());
        fs::remove_file(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(conn);
        assert!(before_open(&path).is_err());
        fs::remove_dir_all(parent).unwrap();
    }
    #[test]
    fn rejects_tampered_archive_without_staging_or_changing_live_data() {
        let parent = std::env::temp_dir().join(format!(
            "supercode-backup-tampered-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&parent).unwrap();
        let path = parent.join("supercode.db");
        let store = Store::open(&path).unwrap();
        store.set_setting("live", "preserved").unwrap();
        let output = parent.join("tampered.supercode-backup");
        let mut zip = ZipWriter::new(fs::File::create(&output).unwrap());
        let options = SimpleFileOptions::default();
        for (name, bytes) in [
            ("database.db", b"tampered".as_slice()),
            ("frontend.json", b"{}".as_slice()),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(
            &serde_json::to_vec(&Manifest {
                format: 1,
                schema_version: 1,
                app_version: "0.2.2".into(),
                database_sha256: "0".repeat(64),
                frontend_sha256: hash(b"{}"),
            })
            .unwrap(),
        )
        .unwrap();
        zip.finish().unwrap();
        assert!(stage(&output, &parent).is_err());
        assert!(!parent.join("pending-restore").exists());
        assert_eq!(store.setting("live").unwrap().as_deref(), Some("preserved"));
        drop(store);
        fs::remove_dir_all(parent).unwrap();
    }
}
