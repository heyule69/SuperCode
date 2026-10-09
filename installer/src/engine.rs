use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Deserialize, Serialize)]
pub struct Entry {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub version: String,
    pub entries: Vec<Entry>,
}
#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct Progress {
    pub percent: f64,
    pub stage: String,
}
#[derive(Serialize, Clone, Debug)]
pub struct Installed {
    pub path: String,
    pub backup: String,
    pub version: String,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    state: String,
    files: Vec<String>,
    existed: Vec<String>,
}

pub trait Registration {
    fn snapshot(&self, target: &Path, backup: &Path) -> Result<()>;
    fn apply(&self, target: &Path, worker: &Path) -> Result<()>;
    fn restore(&self, target: &Path, backup: &Path) -> Result<()>;
}
fn err(context: &str, e: impl std::fmt::Display) -> String {
    format!("{context}：{e}")
}
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| err("无法记录安装状态", e))?;
    let mut file = File::create(path).map_err(|e| err("无法保存安装状态", e))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| err("无法保存安装状态", e))
}
fn owned(name: &str) -> bool {
    matches!(
        name,
        "supercode.exe" | "uninstall.exe" | "installation.json"
    ) || (name.starts_with("supercode-icon-")
        && name.ends_with(".ico")
        && name.len() == 31
        && name.as_bytes()[15..27].iter().all(u8::is_ascii_hexdigit))
}
fn payload_name(name: &str) -> bool {
    name == "registration.exe"
        || owned(name) && name != "uninstall.exe" && name != "installation.json"
}

pub fn validate_target(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.parent().is_none() || path.file_name().is_none() {
        return Err("请选择一个完整的安装文件夹，不能使用磁盘根目录。".into());
    }
    for part in path.components() {
        match part {
            Component::ParentDir | Component::CurDir => {
                return Err("安装目录不能包含相对路径。".into())
            }
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
                let reserved = [
                    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
                    "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7",
                    "LPT8", "LPT9",
                ];
                if name.ends_with(['.', ' '])
                    || name
                        .chars()
                        .any(|c| c.is_control() || "<>\"|?*:".contains(c))
                    || reserved.contains(&stem.as_str())
                {
                    return Err("安装目录包含 Windows 不支持的字符或名称。".into());
                }
                if name.eq_ignore_ascii_case("dev.supercode.desktop")
                    || name.eq_ignore_ascii_case(".supercode-backups")
                {
                    return Err("请选择独立的安装目录，不能使用聊天数据或备份目录。".into());
                }
            }
            _ => {}
        }
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        if !matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)))
        {
            return Err("请选择本机磁盘上的安装文件夹。".into());
        }
        if path.as_os_str().len() > 210 {
            return Err("安装目录过长，请选择更短的路径。".into());
        }
    }
    for ancestor in path.ancestors() {
        reject_link(ancestor)?;
    }
    Ok(())
}
fn reject_link(path: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mut link = meta.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            link |= meta.file_attributes() & 0x400 != 0;
        }
        if link {
            return Err(format!(
                "安装路径包含链接或重定向目录，请换一个位置：{}",
                path.display()
            ));
        }
    }
    Ok(())
}
fn verify_manifest(manifest: &Manifest) -> Result<u64> {
    let mut names = HashSet::new();
    let mut total = 0u64;
    for entry in &manifest.entries {
        if !payload_name(&entry.name)
            || !names.insert(&entry.name)
            || entry.sha256.len() != 64
            || !entry.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("安装包清单无效。".into());
        }
        total = total
            .checked_add(entry.bytes)
            .filter(|n| *n <= 256 * 1024 * 1024)
            .ok_or("安装包大小无效。")?;
    }
    if total == 0
        || !names.contains(&"supercode.exe".to_string())
        || !names.contains(&"registration.exe".to_string())
    {
        return Err("安装包缺少必要文件。".into());
    }
    Ok(total)
}

struct Lock {
    file: Option<File>,
    path: PathBuf,
}
impl Drop for Lock {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}
fn lock(target: &Path) -> Result<Lock> {
    let path = target.join(".supercode-install.lock");
    reject_link(&path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = options
        .open(&path)
        .map_err(|e| err("另一个安装程序正在使用该目录", e))?;
    Ok(Lock {
        file: Some(file),
        path,
    })
}
fn ensure_not_running(target: &Path) -> Result<()> {
    let path = target.join("supercode.exe");
    if path.exists() {
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        options.open(&path).map_err(|_| {
            "SuperCode 正在运行或文件被占用。请先从托盘退出，再重试安装。".to_string()
        })?;
    }
    Ok(())
}
fn restore_files(target: &Path, backup: &Path, journal: &Journal) -> Result<()> {
    if journal.files.iter().any(|name| !owned(name))
        || journal
            .existed
            .iter()
            .any(|name| !journal.files.contains(name))
    {
        return Err("备份清单无效，已停止恢复。".into());
    }
    for name in &journal.files {
        let dest = target.join(name);
        reject_link(&dest)?;
        if journal.existed.contains(name) {
            let source = backup.join(name);
            reject_link(&source)?;
            fs::copy(source, dest).map_err(|e| err("恢复旧版失败", e))?;
        } else if dest.exists() {
            fs::remove_file(dest).map_err(|e| err("撤销新文件失败", e))?;
        }
    }
    Ok(())
}
fn recover(target: &Path, registration: &impl Registration) -> Result<()> {
    let backups = target.join(".supercode-backups");
    reject_link(&backups)?;
    if !backups.exists() {
        return Ok(());
    }
    let mut pending = vec![];
    for item in fs::read_dir(&backups).map_err(|e| err("无法读取备份", e))? {
        let path = item.map_err(|e| err("无法读取备份", e))?.path();
        reject_link(&path)?;
        let file = path.join("journal.json");
        reject_link(&file)?;
        if !file.exists() {
            continue;
        }
        let journal: Journal =
            serde_json::from_slice(&fs::read(&file).map_err(|e| err("无法读取备份清单", e))?)
                .map_err(|e| err("备份清单无效", e))?;
        if journal.state == "installing" {
            pending.push((path, journal));
        }
    }
    if pending.len() > 1 {
        return Err("发现多份未完成的安装，请先检查安装目录中的备份。".into());
    }
    for (backup, mut journal) in pending {
        restore_files(target, &backup, &journal)?;
        registration.restore(target, &backup)?;
        journal.state = "recovered".into();
        write_json(&backup.join("journal.json"), &journal)?;
    }
    Ok(())
}

pub fn install(
    target: &Path,
    archive: &[u8],
    manifest: &Manifest,
    registration: &impl Registration,
    mut emit: impl FnMut(Progress),
) -> Result<Installed> {
    validate_target(target)?;
    let total = verify_manifest(manifest)?;
    fs::create_dir_all(target).map_err(|e| err("无法创建安装目录", e))?;
    let _lock = lock(target)?;
    ensure_not_running(target)?;
    recover(target, registration)?;
    let stage = target.join(format!(".supercode-stage-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&stage).map_err(|e| err("无法创建临时安装目录", e))?;
    let notify = |emit: &mut dyn FnMut(Progress), percent, stage: &str| {
        emit(Progress {
            percent,
            stage: stage.into(),
        })
    };
    let result = (|| {
        let mut zip =
            zip::ZipArchive::new(Cursor::new(archive)).map_err(|e| err("安装包损坏", e))?;
        if zip.len() != manifest.entries.len() {
            return Err("安装包与清单不匹配。".into());
        }
        let mut written = 0u64;
        let mut last_percent = -1;
        notify(&mut emit, 0., "extracting");
        for entry in &manifest.entries {
            let mut source = zip
                .by_name(&entry.name)
                .map_err(|e| err("安装包缺少文件", e))?;
            if source.size() != entry.bytes {
                return Err("安装包文件大小不匹配。".into());
            }
            let mut output =
                File::create(stage.join(&entry.name)).map_err(|e| err("无法写入安装文件", e))?;
            let mut digest = Sha256::new();
            let mut bytes = 0u64;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = source
                    .read(&mut buffer)
                    .map_err(|e| err("无法解压安装文件", e))?;
                if count == 0 {
                    break;
                }
                output
                    .write_all(&buffer[..count])
                    .map_err(|e| err("无法写入安装文件", e))?;
                digest.update(&buffer[..count]);
                bytes += count as u64;
                written += count as u64;
                let percent = (written as f64 / total as f64 * 82.).floor() as i32;
                if percent > last_percent {
                    notify(&mut emit, percent as f64, "extracting");
                    last_percent = percent;
                }
            }
            output.sync_all().map_err(|e| err("无法保存安装文件", e))?;
            if bytes != entry.bytes || format!("{:x}", digest.finalize()) != entry.sha256 {
                return Err("文件校验失败，原版本未被覆盖。".into());
            }
        }
        ensure_not_running(target)?;
        let backup = target
            .join(".supercode-backups")
            .join(uuid::Uuid::new_v4().to_string());
        reject_link(&target.join(".supercode-backups"))?;
        fs::create_dir_all(&backup).map_err(|e| err("无法备份旧版", e))?;
        let mut files: Vec<String> = manifest
            .entries
            .iter()
            .filter(|e| e.name != "registration.exe")
            .map(|e| e.name.clone())
            .collect();
        files.extend(["uninstall.exe".into(), "installation.json".into()]);
        let mut journal = Journal {
            state: "installing".into(),
            files,
            existed: vec![],
        };
        for name in &journal.files {
            let original = target.join(name);
            reject_link(&original)?;
            if original.exists() {
                fs::copy(&original, backup.join(name)).map_err(|e| err("无法备份原文件", e))?;
                journal.existed.push(name.clone());
            }
        }
        registration.snapshot(target, &backup)?;
        write_json(&backup.join("journal.json"), &journal)?;
        notify(&mut emit, 86., "backed-up");
        let commit = (|| {
            // The executable is committed last; all payload hashes have already been checked.
            for entry in manifest
                .entries
                .iter()
                .filter(|e| e.name != "registration.exe" && e.name != "supercode.exe")
                .chain(
                    manifest
                        .entries
                        .iter()
                        .filter(|e| e.name == "supercode.exe"),
                )
            {
                let dest = target.join(&entry.name);
                if dest.exists() {
                    fs::remove_file(&dest).map_err(|e| err("无法更新原文件", e))?;
                }
                fs::rename(stage.join(&entry.name), dest)
                    .map_err(|e| err("无法提交安装文件", e))?;
            }
            notify(&mut emit, 90., "registering");
            registration.apply(target, &stage.join("registration.exe"))?;
            if !target.join("uninstall.exe").is_file() {
                return Err("卸载程序未能创建。".into());
            }
            notify(&mut emit, 97., "verifying");
            for entry in manifest
                .entries
                .iter()
                .filter(|e| e.name != "registration.exe")
            {
                let digest = hash_file(&target.join(&entry.name))?;
                if digest != entry.sha256 {
                    return Err("安装后校验失败。".into());
                }
            }
            write_json(&target.join("installation.json"), manifest)?;
            journal.state = "complete".into();
            write_json(&backup.join("journal.json"), &journal)?;
            Ok(Installed {
                path: target.to_string_lossy().into(),
                backup: backup.to_string_lossy().into(),
                version: manifest.version.clone(),
            })
        })();
        if let Err(error) = &commit {
            let rollback = restore_files(target, &backup, &journal)
                .and_then(|_| registration.restore(target, &backup));
            if let Err(failure) = rollback {
                return Err(format!(
                    "{error}\n{failure}\n备份保存在：{}",
                    backup.display()
                ));
            }
            journal.state = "rolled-back".into();
            write_json(&backup.join("journal.json"), &journal)?;
        }
        commit
    })();
    // This path was created here, and cleanup never crosses the checked target boundary.
    if stage.parent() == Some(target)
        && stage
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(".supercode-stage-"))
    {
        let _ = fs::remove_dir_all(&stage);
    }
    if result.is_ok() {
        notify(&mut emit, 100., "complete");
    }
    result
}
pub fn hash_file(path: &Path) -> Result<String> {
    let mut input = File::open(path).map_err(|e| err("无法读取安装文件", e))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|e| err("无法校验安装文件", e))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;
    struct TestRegistration {
        fail: bool,
    }
    impl Registration for TestRegistration {
        fn snapshot(&self, _: &Path, _: &Path) -> Result<()> {
            Ok(())
        }
        fn apply(&self, path: &Path, _: &Path) -> Result<()> {
            fs::write(path.join("uninstall.exe"), b"uninstall").unwrap();
            if self.fail {
                Err("registration failure".into())
            } else {
                Ok(())
            }
        }
        fn restore(&self, _: &Path, _: &Path) -> Result<()> {
            Ok(())
        }
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("supercode-installer-test-{}", uuid::Uuid::new_v4())),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.0.parent() == Some(std::env::temp_dir().as_path()) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }
    fn package() -> (Vec<u8>, Manifest) {
        let mut writer = zip::ZipWriter::new(Cursor::new(vec![]));
        let mut entries = vec![];
        for (name, data) in [
            ("supercode.exe", vec![42u8; 400_000]),
            ("registration.exe", vec![21u8; 200]),
        ] {
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(&data).unwrap();
            entries.push(Entry {
                name: name.into(),
                bytes: data.len() as u64,
                sha256: format!("{:x}", Sha256::digest(data)),
            });
        }
        (
            writer.finish().unwrap().into_inner(),
            Manifest {
                version: "0.1.0".into(),
                entries,
            },
        )
    }
    #[test]
    fn fresh_install_emits_measured_monotonic_progress_and_only_completes_after_registration() {
        let target = Fixture::new();
        let (zip, manifest) = package();
        let mut events = vec![];
        install(
            &target.0,
            &zip,
            &manifest,
            &TestRegistration { fail: false },
            |p| events.push(p),
        )
        .unwrap();
        assert!(events.len() > 6);
        assert_eq!(events.last().unwrap().percent, 100.);
        assert!(events.windows(2).all(|p| p[0].percent <= p[1].percent));
        assert!(target.0.join("uninstall.exe").exists());
        assert_eq!(
            hash_file(&target.0.join("supercode.exe")).unwrap(),
            manifest.entries[0].sha256
        );
    }
    #[test]
    fn upgrade_preserves_old_binary_and_unknown_files() {
        let target = Fixture::new();
        fs::create_dir(&target.0).unwrap();
        fs::write(target.0.join("supercode.exe"), b"old").unwrap();
        fs::write(target.0.join("user.txt"), b"personal").unwrap();
        let (zip, manifest) = package();
        let installed = install(
            &target.0,
            &zip,
            &manifest,
            &TestRegistration { fail: false },
            |_| {},
        )
        .unwrap();
        assert_eq!(
            fs::read(Path::new(&installed.backup).join("supercode.exe")).unwrap(),
            b"old"
        );
        assert_eq!(fs::read(target.0.join("user.txt")).unwrap(), b"personal");
    }
    #[test]
    fn failed_registration_restores_every_old_file_without_emitting_completion() {
        let target = Fixture::new();
        fs::create_dir(&target.0).unwrap();
        fs::write(target.0.join("supercode.exe"), b"old").unwrap();
        fs::write(target.0.join("uninstall.exe"), b"old-uninstall").unwrap();
        let (zip, manifest) = package();
        let mut events = vec![];
        assert!(install(
            &target.0,
            &zip,
            &manifest,
            &TestRegistration { fail: true },
            |p| events.push(p)
        )
        .is_err());
        assert_eq!(fs::read(target.0.join("supercode.exe")).unwrap(), b"old");
        assert_eq!(
            fs::read(target.0.join("uninstall.exe")).unwrap(),
            b"old-uninstall"
        );
        assert!(!target.0.join("installation.json").exists());
        assert!(events.iter().all(|p| p.percent < 100.));
    }
    #[test]
    fn corrupt_payload_never_overwrites_old_binary() {
        let target = Fixture::new();
        fs::create_dir(&target.0).unwrap();
        fs::write(target.0.join("supercode.exe"), b"old").unwrap();
        let (zip, mut manifest) = package();
        manifest.entries[0].sha256 = "0".repeat(64);
        assert!(install(
            &target.0,
            &zip,
            &manifest,
            &TestRegistration { fail: false },
            |_| {}
        )
        .is_err());
        assert_eq!(fs::read(target.0.join("supercode.exe")).unwrap(), b"old");
    }
    #[test]
    fn traversal_in_manifest_is_rejected_before_creating_target() {
        let target = Fixture::new();
        let (zip, mut manifest) = package();
        manifest.entries[0].name = "../victim.exe".into();
        assert!(install(
            &target.0,
            &zip,
            &manifest,
            &TestRegistration { fail: false },
            |_| {}
        )
        .is_err());
        assert!(!target.0.exists());
    }
    #[test]
    fn missing_or_extra_zip_entries_are_rejected() {
        let target = Fixture::new();
        let (zip, mut manifest) = package();
        manifest.entries.pop();
        assert!(install(
            &target.0,
            &zip,
            &manifest,
            &TestRegistration { fail: false },
            |_| {}
        )
        .is_err());
    }
    #[test]
    fn invalid_windows_paths_and_data_folders_are_rejected() {
        for value in [
            "relative",
            "C:\\",
            "C:\\folder\\..\\SuperCode",
            "C:\\dev.supercode.desktop\\app",
            "C:\\NUL",
            "C:\\foo:bar",
            "C:\\name.",
            "\\\\host\\share\\SuperCode",
        ] {
            assert!(validate_target(Path::new(value)).is_err(), "{value}");
        }
        assert!(validate_target(Path::new("D:\\安装软件\\SuperCode")).is_ok());
    }
    #[test]
    fn pending_commit_is_recovered_before_a_new_install_attempt() {
        let target = Fixture::new();
        fs::create_dir(&target.0).unwrap();
        let backup = target.0.join(".supercode-backups").join("pending");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("supercode.exe"), b"old").unwrap();
        fs::write(target.0.join("supercode.exe"), b"partial").unwrap();
        write_json(
            &backup.join("journal.json"),
            &Journal {
                state: "installing".into(),
                files: vec!["supercode.exe".into(), "uninstall.exe".into()],
                existed: vec!["supercode.exe".into()],
            },
        )
        .unwrap();
        let (zip, mut manifest) = package();
        manifest.entries[0].sha256 = "0".repeat(64);
        assert!(install(
            &target.0,
            &zip,
            &manifest,
            &TestRegistration { fail: false },
            |_| {}
        )
        .is_err());
        assert_eq!(fs::read(target.0.join("supercode.exe")).unwrap(), b"old");
    }
    #[test]
    fn forged_recovery_manifest_cannot_remove_unknown_files() {
        let target = Fixture::new();
        fs::create_dir(&target.0).unwrap();
        let backup = target.0.join(".supercode-backups").join("pending");
        fs::create_dir_all(&backup).unwrap();
        write_json(
            &backup.join("journal.json"),
            &Journal {
                state: "installing".into(),
                files: vec!["personal.txt".into()],
                existed: vec![],
            },
        )
        .unwrap();
        fs::write(target.0.join("personal.txt"), b"keep").unwrap();
        let (zip, manifest) = package();
        assert!(install(
            &target.0,
            &zip,
            &manifest,
            &TestRegistration { fail: false },
            |_| {}
        )
        .is_err());
        assert_eq!(fs::read(target.0.join("personal.txt")).unwrap(), b"keep");
    }
    #[cfg(windows)]
    #[test]
    fn running_binary_and_concurrent_installer_are_detected() {
        use std::os::windows::fs::OpenOptionsExt;
        let target = Fixture::new();
        fs::create_dir(&target.0).unwrap();
        fs::write(target.0.join("supercode.exe"), b"running").unwrap();
        let (zip, manifest) = package();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(target.0.join("supercode.exe"))
            .unwrap();
        assert!(install(
            &target.0,
            &zip,
            &manifest,
            &TestRegistration { fail: false },
            |_| {}
        )
        .unwrap_err()
        .contains("正在运行"));
        drop(held);
        let held = lock(&target.0).unwrap();
        assert!(install(
            &target.0,
            &zip,
            &manifest,
            &TestRegistration { fail: false },
            |_| {}
        )
        .is_err());
        drop(held);
        assert!(install(
            &target.0,
            &zip,
            &manifest,
            &TestRegistration { fail: false },
            |_| {}
        )
        .is_ok());
    }
}
