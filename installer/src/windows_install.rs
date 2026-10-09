use crate::engine::{write_json, Registration, Result};
use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use windows_registry::{Type, CURRENT_USER};

const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\SuperCode";
#[derive(Serialize, Deserialize)]
struct Value {
    name: String,
    kind: u32,
    bytes: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
struct Snapshot {
    values: Option<Vec<Value>>,
    desktop: bool,
    programs: bool,
}
pub struct WindowsRegistration {
    pub isolated: bool,
}
pub fn hidden_command(path: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(path);
    command.creation_flags(0x08000000);
    command
}
pub fn previous_install() -> Option<PathBuf> {
    registered_path(KEY)
}
fn registered_path(key: &str) -> Option<PathBuf> {
    CURRENT_USER
        .options()
        .read()
        .access(0x100)
        .open(key)
        .ok()
        .and_then(|key| {
            ["InstallLocation", ""]
                .into_iter()
                .filter_map(|name| key.get_string(name).ok())
                .find_map(|s| {
                    let s = s.trim().trim_matches('"').trim();
                    (!s.is_empty()).then(|| PathBuf::from(s))
                })
        })
}
fn shortcut_target(path: &Path) -> Result<PathBuf> {
    use windows::{
        core::{Interface, HSTRING},
        Win32::{
            Foundation::RPC_E_CHANGED_MODE,
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile,
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STGM_READ,
            },
            UI::Shell::{IShellLinkW, ShellLink},
        },
    };
    struct Com(bool);
    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CoUninitialize();
                }
            }
        }
    }
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialized.is_err() && initialized != RPC_E_CHANGED_MODE {
        return Err(initialized.to_string());
    }
    let _com = Com(initialized.is_ok());
    unsafe {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        file.Load(&HSTRING::from(path.as_os_str()), STGM_READ)
            .map_err(|e| e.to_string())?;
        let mut buffer = vec![0u16; 32768];
        link.GetPath(&mut buffer, std::ptr::null_mut(), 4)
            .map_err(|e| e.to_string())?;
        let end = buffer
            .iter()
            .position(|c| *c == 0)
            .ok_or("快捷方式路径无效")?;
        Ok(PathBuf::from(
            String::from_utf16(&buffer[..end]).map_err(|e| e.to_string())?,
        ))
    }
}
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    let a = fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    a.to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .replace('/', "\\")
        .trim_end_matches('\\')
        .eq_ignore_ascii_case(
            b.to_string_lossy()
                .trim_start_matches("\\\\?\\")
                .replace('/', "\\")
                .trim_end_matches('\\'),
        )
}
fn shortcut(name: &str) -> Result<PathBuf> {
    let key = CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders")
        .map_err(|e| e.to_string())?;
    Ok(PathBuf::from(key.get_string(name).map_err(|e| e.to_string())?).join("SuperCode.lnk"))
}
impl Registration for WindowsRegistration {
    fn snapshot(&self, target: &Path, backup: &Path) -> Result<()> {
        if self.isolated {
            return Ok(());
        }
        snapshot_registration(
            target,
            backup,
            KEY,
            [&shortcut("Desktop")?, &shortcut("Programs")?],
        )
    }
    fn apply(&self, target: &Path, worker: &Path) -> Result<()> {
        let mut command = hidden_command(worker);
        command.arg("/S");
        if self.isolated {
            command.arg("/NOREG");
        }
        let status = command
            .env(
                "SUPERCODE_INSTALL_TARGET",
                target.to_string_lossy().replace('/', "\\"),
            )
            .status()
            .map_err(|e| format!("无法创建卸载入口：{e}"))?;
        if !status.success() {
            return Err(format!(
                "创建快捷方式或卸载入口失败（{}）。",
                status.code().unwrap_or(-1)
            ));
        }
        if !self.isolated {
            if previous_install().is_none_or(|path| !same_path(&path, target)) {
                return Err("安装注册信息校验失败。".into());
            }
            for name in ["Desktop", "Programs"] {
                if !shortcut(name)?.is_file() {
                    return Err("快捷方式未能创建。".into());
                }
            }
        }
        Ok(())
    }
    fn restore(&self, _: &Path, backup: &Path) -> Result<()> {
        if self.isolated {
            return Ok(());
        }
        restore_registration(backup, KEY, [&shortcut("Desktop")?, &shortcut("Programs")?])
    }
}

fn snapshot_registration(
    target: &Path,
    backup: &Path,
    registry: &str,
    links: [&Path; 2],
) -> Result<()> {
    if let Some(previous) = registered_path(registry) {
        if !same_path(&previous, target) {
            return Err(format!(
                "已安装在 {}。请使用原目录更新，或先卸载旧版再更换目录。",
                previous.display()
            ));
        }
    }
    let values = match CURRENT_USER.options().read().access(0x100).open(registry) {
        Ok(key) => Some(
            key.values()
                .map_err(|e| e.to_string())?
                .map(|(name, value)| Value {
                    name,
                    kind: value.ty().into(),
                    bytes: value.to_vec(),
                })
                .collect(),
        ),
        Err(e) if e.code().0 as u32 == 0x80070002 => None,
        Err(e) => return Err(format!("无法备份卸载信息：{e}")),
    };
    let [desktop, programs] = links;
    for (path, name) in [(desktop, "desktop.lnk"), (programs, "programs.lnk")] {
        if path.exists() {
            // Legacy installations may have no usable uninstall entry. Verify
            // the actual link target instead of treating our own link as a conflict.
            if !same_path(
                &shortcut_target(path)
                    .map_err(|e| format!("无法检查快捷方式 {}：{e}", path.display()))?,
                &target.join("supercode.exe"),
            ) {
                return Err(format!(
                    "同名快捷方式 {} 指向其他程序，无法覆盖。",
                    path.display()
                ));
            }
            fs::copy(path, backup.join(name)).map_err(|e| e.to_string())?;
        }
    }
    write_json(
        &backup.join("registration.json"),
        &Snapshot {
            values,
            desktop: desktop.exists(),
            programs: programs.exists(),
        },
    )
}
fn restore_registration(backup: &Path, registry: &str, links: [&Path; 2]) -> Result<()> {
    let snapshot: Snapshot = serde_json::from_slice(
        &fs::read(backup.join("registration.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if CURRENT_USER
        .options()
        .read()
        .access(0x100)
        .open(registry)
        .is_ok()
    {
        CURRENT_USER
            .remove_tree(registry)
            .map_err(|e| e.to_string())?;
    }
    if let Some(values) = snapshot.values {
        let key = CURRENT_USER
            .options()
            .read()
            .write()
            .create()
            .access(0x100)
            .open(registry)
            .map_err(|e| e.to_string())?;
        for value in values {
            key.set_bytes(value.name, Type::from(value.kind), &value.bytes)
                .map_err(|e| e.to_string())?;
        }
    }
    for (path, saved, existed) in [
        (links[0], "desktop.lnk", snapshot.desktop),
        (links[1], "programs.lnk", snapshot.programs),
    ] {
        if existed {
            fs::copy(backup.join(saved), path).map_err(|e| e.to_string())?;
        } else if path.exists() {
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn create_link(path: &Path, target: &Path) {
        use windows::{
            core::{Interface, HSTRING},
            Win32::{
                System::Com::{
                    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile,
                    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
                },
                UI::Shell::{IShellLinkW, ShellLink},
            },
        };
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
            {
                let link: IShellLinkW =
                    CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
                link.SetPath(&HSTRING::from(target.as_os_str())).unwrap();
                let file: IPersistFile = link.cast().unwrap();
                file.Save(&HSTRING::from(path.as_os_str()), true).unwrap();
            }
            CoUninitialize();
        }
    }
    #[test]
    fn legacy_shortcuts_without_registration_upgrade_and_roll_back() {
        let root =
            std::env::temp_dir().join(format!("supercode-link-test-{}", uuid::Uuid::new_v4()));
        let target = root.join("旧版目录 with spaces");
        let backup = root.join("backup");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir(&backup).unwrap();
        fs::write(target.join("supercode.exe"), b"old exe").unwrap();
        let links = [root.join("desktop.lnk"), root.join("programs.lnk")];
        for path in &links {
            create_link(path, &target.join("supercode.exe"));
        }
        let original = fs::read(&links[0]).unwrap();
        let registry = format!(
            r"Software\SuperCode\InstallerTests\{}",
            uuid::Uuid::new_v4()
        );
        snapshot_registration(&target, &backup, &registry, [&links[0], &links[1]]).unwrap();
        assert_eq!(original, fs::read(backup.join("desktop.lnk")).unwrap());
        let key = CURRENT_USER.create(&registry).unwrap();
        key.set_string("InstallLocation", target.to_string_lossy())
            .unwrap();
        drop(key);
        fs::write(&links[0], b"new shortcut").unwrap();
        restore_registration(&backup, &registry, [&links[0], &links[1]]).unwrap();
        assert_eq!(original, fs::read(&links[0]).unwrap());
        assert!(CURRENT_USER.open(&registry).is_err());
        create_link(&links[1], &root.join("other.exe"));
        let unrelated = fs::read(&links[1]).unwrap();
        assert!(
            snapshot_registration(&target, &backup, &registry, [&links[0], &links[1]]).is_err()
        );
        assert_eq!(unrelated, fs::read(&links[1]).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn quoted_legacy_registry_paths_and_raw_values_survive_rollback() {
        let root =
            std::env::temp_dir().join(format!("supercode-registry-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let registry = format!(
            r"Software\SuperCode\InstallerTests\{}",
            uuid::Uuid::new_v4()
        );
        let key = CURRENT_USER.create(&registry).unwrap();
        let location = format!("\"{}\"", root.display());
        key.set_string("InstallLocation", &location).unwrap();
        key.set_u32("EstimatedSize", 123).unwrap();
        assert!(same_path(&registered_path(&registry).unwrap(), &root));
        key.set_string("", &location).unwrap();
        key.set_string("InstallLocation", "  \"\"  ").unwrap();
        assert!(same_path(&registered_path(&registry).unwrap(), &root));
        key.set_string("InstallLocation", &location).unwrap();
        let links = [root.join("desktop.lnk"), root.join("programs.lnk")];
        snapshot_registration(&root, &root, &registry, [&links[0], &links[1]]).unwrap();
        key.set_string("InstallLocation", "C:\\Wrong").unwrap();
        drop(key);
        restore_registration(&root, &registry, [&links[0], &links[1]]).unwrap();
        let restored = CURRENT_USER.open(&registry).unwrap();
        assert_eq!(restored.get_string("InstallLocation").unwrap(), location);
        assert_eq!(restored.get_u32("EstimatedSize").unwrap(), 123);
        drop(restored);
        CURRENT_USER.remove_tree(&registry).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn installation_identity_handles_windows_path_aliases() {
        assert!(same_path(
            Path::new("C:\\Example\\SuperCode"),
            Path::new("c:/example/SuperCode/")
        ));
        assert!(!same_path(
            Path::new("C:\\Example\\SuperCode"),
            Path::new("C:\\Example\\Other")
        ));
    }
    #[test]
    fn registration_worker_without_native_target_cannot_fall_back_to_production() {
        let root = std::env::temp_dir().join(format!(
            "supercode-registration-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let exe = root.join("registration.exe");
        fs::write(&exe, include_bytes!("../generated/registration.exe")).unwrap();
        let formal = previous_install().map(|p| p.join("uninstall.exe"));
        let before = formal
            .as_ref()
            .filter(|p| p.is_file())
            .map(|p| crate::engine::hash_file(p).unwrap());
        let status = hidden_command(&exe)
            .args(["/S", "/NOREG"])
            .env_remove("SUPERCODE_INSTALL_TARGET")
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(7));
        assert_eq!(
            before,
            formal
                .as_ref()
                .filter(|p| p.is_file())
                .map(|p| crate::engine::hash_file(p).unwrap())
        );
        assert!(!root.join("uninstall.exe").exists());
        if root.parent() == Some(std::env::temp_dir().as_path()) {
            fs::remove_dir_all(root).unwrap();
        }
    }
}
