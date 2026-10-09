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
    CURRENT_USER
        .open(KEY)
        .ok()
        .and_then(|key| {
            key.get_string("InstallLocation")
                .or_else(|_| key.get_string(""))
                .ok()
        })
        .filter(|s| !s.is_empty())
        .map(|s| PathBuf::from(s.trim_matches('"')))
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
        if let Some(previous) = previous_install() {
            if !same_path(&previous, target) {
                return Err(format!(
                    "已安装在 {}。请使用原目录更新，或先卸载旧版再更换目录。",
                    previous.display()
                ));
            }
        }
        let values = match CURRENT_USER.open(KEY) {
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
        let desktop = shortcut("Desktop")?;
        let programs = shortcut("Programs")?;
        for (path, name) in [(&desktop, "desktop.lnk"), (&programs, "programs.lnk")] {
            if path.exists() {
                if previous_install().is_none() {
                    return Err("发现同名快捷方式，请先移开再安装，以免覆盖。".into());
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
        let snapshot: Snapshot = serde_json::from_slice(
            &fs::read(backup.join("registration.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if CURRENT_USER.open(KEY).is_ok() {
            CURRENT_USER.remove_tree(KEY).map_err(|e| e.to_string())?;
        }
        if let Some(values) = snapshot.values {
            let key = CURRENT_USER.create(KEY).map_err(|e| e.to_string())?;
            for value in values {
                key.set_bytes(value.name, Type::from(value.kind), &value.bytes)
                    .map_err(|e| e.to_string())?;
            }
        }
        for (name, saved, existed) in [
            ("Desktop", "desktop.lnk", snapshot.desktop),
            ("Programs", "programs.lnk", snapshot.programs),
        ] {
            let path = shortcut(name)?;
            if existed {
                fs::copy(backup.join(saved), path).map_err(|e| e.to_string())?;
            } else if path.exists() {
                fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
