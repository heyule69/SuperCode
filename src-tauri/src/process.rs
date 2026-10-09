use std::path::{Path, PathBuf};
use tokio::process::{Child, Command};

pub fn find_program(name: &str) -> Option<PathBuf> {
    if matches!(name, "codex" | "claude") {
        return crate::agents::local(name).map(|launch| launch.program);
    }
    for directory in std::env::split_paths(&std::env::var_os("PATH")?) {
        let suffixes: &[&str] = if cfg!(windows) {
            &[".exe", ".cmd", ""]
        } else {
            &[""]
        };
        for suffix in suffixes {
            let path = directory.join(format!("{name}{suffix}"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

pub fn command(program: &Path, args: &[&str]) -> Command {
    let mut command = if cfg!(windows)
        && program
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("cmd"))
    {
        // Only fixed internal flags go through cmd. Prompts always use stdin JSON.
        let mut c = Command::new("cmd.exe");
        c.args(["/d", "/s", "/c"]);
        c.arg(format!("\"\"{}\" {}\"", program.display(), args.join(" ")));
        c
    } else {
        let mut c = Command::new(program);
        c.args(args);
        c
    };
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command.kill_on_drop(true);
    command
}

pub fn codex_config_overrides(load_mcp: bool) -> Result<Vec<String>, String> {
    config_overrides(&read_codex_config()?, load_mcp)
}

pub fn read_codex_config() -> Result<String, String> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(|p| PathBuf::from(p).join(".codex"))
        });
    let Some(home) = home else {
        return Ok(String::new());
    };
    let bytes = match std::fs::read(home.join("config.toml")) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(format!("无法读取 Codex 配置：{e}")),
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| "Codex 配置不是 UTF-8，未转换或修改编码")?;
    Ok(text.to_owned())
}

fn config_overrides(text: &str, load_mcp: bool) -> Result<Vec<String>, String> {
    let config: toml::Table =
        toml::from_str(text).map_err(|_| "Codex 配置 TOML 格式无效，未修改原文件")?;
    let mut overrides = vec![];
    if config.get("service_tier").and_then(toml::Value::as_str) == Some("priority") {
        overrides.push("service_tier=\"fast\"".into());
    }
    if !load_mcp {
        if let Some(servers) = config.get("mcp_servers").and_then(toml::Value::as_table) {
            for name in servers.keys() {
                if !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    return Err("全局 MCP 服务名含不支持的字符，无法生成 CLI 覆盖参数。可开启加载 MCP 或修正原配置。".into());
                }
                overrides.push(format!("mcp_servers.{name}.enabled=false"));
            }
        }
    }
    Ok(overrides)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overrides_validate_mcp_keys_and_never_include_credentials() {
        let text = "service_tier='priority'\n[mcp_servers.\"tool-search\"]\ncommand='tool'\n[mcp_servers.\"tool-search\".env]\nAPI_KEY='private'\n";
        let args = config_overrides(text, false).unwrap();
        assert_eq!(
            args,
            vec![
                "service_tier=\"fast\"",
                "mcp_servers.tool-search.enabled=false"
            ]
        );
        assert_eq!(
            config_overrides(text, true).unwrap(),
            vec!["service_tier=\"fast\""]
        );
        assert!(config_overrides("not valid toml", false).is_err());
        assert!(
            config_overrides("[mcp_servers.\"tool.with.dot\"]\ncommand='tool'", false).is_err()
        );
    }
}

#[cfg(windows)]
pub struct JobGuard(isize);
#[cfg(not(windows))]
pub struct JobGuard;

impl JobGuard {
    pub fn attach(child: &Child) -> Result<Self, String> {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::{
                Foundation::*,
                System::{JobObjects::*, Threading::*},
            };
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            ) == 0
            {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error.to_string());
            }
            let process = OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE,
                0,
                child.id().ok_or("进程已经退出")?,
            );
            if process.is_null() {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error.to_string());
            }
            let assigned = AssignProcessToJobObject(job, process);
            CloseHandle(process);
            if assigned == 0 {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(format!("无法管理 agent 子进程：{error}"));
            }
            Ok(Self(job as isize))
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Ok(Self)
        }
    }
}

#[cfg(windows)]
impl Drop for JobGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}
