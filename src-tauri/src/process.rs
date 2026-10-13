use std::path::{Path, PathBuf};
use tokio::process::{Child, Command};

// These belong to the hosting Codex task, not an independently launched CLI.
// Keep user configuration (including CODEX_HOME) and let each agent's explicit
// launch/turn settings select its permissions and runtime.
const HOST_TASK_ENV: &[&str] = &[
    "CODEX_APP_TOOLS_PIPE_PATH", "CODEX_CLI_PATH", "CODEX_THREAD_ID", "CODEX_SESSION_ID",
    "CODEX_INTERNAL_ORIGINATOR_OVERRIDE", "CODEX_PERMISSION_PROFILE",
    "CODEX_WINDOWS_SANDBOX_PACKAGE_FAMILY", "CODEX_TASK_WORKSPACE_VERIFYING_IDENTITY",
    "CODEX_MCP_NODE_PATH", "CODEX_SHELL", "CODEX_CI",
];

fn isolate_host_task(command: &mut Command) {
    for name in HOST_TASK_ENV { command.env_remove(name); }
}

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
    isolate_host_task(&mut command);
    command.kill_on_drop(true);
    command
}

pub fn codex_config_overrides(load_mcp: bool) -> Result<Vec<String>, String> {
    codex_config_overrides_for(&read_codex_config()?, load_mcp)
}

pub(crate) fn codex_config_overrides_for(text: &str, load_mcp: bool) -> Result<Vec<String>, String> {
    config_overrides(text, load_mcp)
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
    read_codex_file(&home.join("config.toml"))
}

pub(crate) fn read_codex_project_config(project: Option<&Path>) -> Result<String, String> {
    project.map(|project| read_codex_file(&project.join(".codex/config.toml"))).unwrap_or_else(|| Ok(String::new()))
}

fn read_codex_file(path: &Path) -> Result<String, String> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > 1024 * 1024 => return Err("Codex 配置超过 1 MB，未读取或修改".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(format!("无法读取 Codex 配置：{error}")),
        _ => {}
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(format!("无法读取 Codex 配置：{e}")),
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| "Codex 配置不是 UTF-8，未转换或修改编码")?;
    Ok(text.to_owned())
}

/// Project settings and CLI overrides can replace the global MCP transport.
/// Keep this snapshot private and use it only to register diagnostic secrets.
pub(crate) fn codex_mcp_configuration(global: &str, project: &str, overrides: &[String]) -> Result<serde_json::Value, String> {
    fn merge(target: &mut toml::Table, source: toml::Table) {
        for (key, value) in source {
            if let (Some(toml::Value::Table(existing)), toml::Value::Table(next)) = (target.get_mut(&key), &value) {
                merge(existing, next.clone());
            } else {
                target.insert(key, value);
            }
        }
    }
    let parse = |text: &str| toml::from_str::<toml::Table>(text).map_err(|_| "Codex 配置 TOML 格式无效，未修改原文件".to_owned());
    let mut config = parse(global)?;
    merge(&mut config, parse(project)?);
    for value in overrides {
        // Codex CLI paths allow bare plugin IDs such as name@market, which
        // are not valid bare TOML keys. Quote only the path, never the value.
        let parsed = parse(value).or_else(|_| {
            let (key, value) = value.split_once('=').ok_or("Codex MCP 覆盖配置无效".to_owned())?;
            let key = key.split('.').map(|part| {
                let part = part.trim();
                if part.starts_with('"') && part.ends_with('"') { part.to_owned() }
                else { toml::Value::String(part.to_owned()).to_string() }
            }).collect::<Vec<_>>().join(".");
            parse(&format!("{key}={value}"))
        })?;
        merge(&mut config, parsed);
    }
    let mut mcp = toml::Table::new();
    for key in ["mcp_servers", "plugins"] {
        if let Some(value) = config.remove(key) { mcp.insert(key.to_owned(), value); }
    }
    serde_json::to_value(mcp).map_err(|_| "Codex MCP 配置无效".into())
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
    fn final_mcp_configuration_registers_global_project_managed_and_extension_secrets_before_stderr() {
        let global = "[mcp_servers.native.http_headers]\nX-Custom-Credential='global-value-123'\n";
        let project = "[mcp_servers.project.env]\nCUSTOM_ACCESS='project-value-456'\n";
        let overrides = vec![
            "mcp_servers.managed.args=['--token','managed-value-789']".into(),
            "mcp_servers.extension.http_headers={X-Opaque='extension-value-321'}".into(),
            "mcp_servers.extension.bearer_token_env_var='MCP_AUTH_ENV'".into(),
            "mcp_servers.native.enabled=false".into(),
            "plugins.installed@market.enabled=true".into(),
            "plugins.installed@market.mcp_servers.native.http_headers={X-Opaque='plugin-value-987'}".into(),
        ];
        let config = codex_mcp_configuration(global, project, &overrides).unwrap();
        let mut secrets = Vec::new();
        crate::log_redaction::collect_mcp(&config, &mut secrets, |name| (name == "MCP_AUTH_ENV").then(|| "resolved-value-654".to_owned()));
        // Deliberately no key names or Bearer prefix: pattern-only redaction
        // cannot protect this actual transport error.
        let output = crate::log_redaction::text("Rejected global-value-123 project-value-456 managed-value-789 extension-value-321 resolved-value-654 plugin-value-987", &secrets);
        assert_eq!(output, "Rejected [REDACTED] [REDACTED] [REDACTED] [REDACTED] [REDACTED] [REDACTED]");
        assert_eq!(config["mcp_servers"]["native"]["enabled"], false);
        assert!(codex_mcp_configuration("invalid toml", "", &[]).is_err());
    }
    #[test]
    fn external_commands_do_not_inherit_the_hosting_task_transport_or_permission_profile() {
        let mut command = Command::new("agent");
        command.env("CODEX_THREAD_ID", "hosting-task").env("CODEX_HOME", "user-config");
        isolate_host_task(&mut command);
        let environment = command.as_std().get_envs().collect::<std::collections::HashMap<_,_>>();
        for name in HOST_TASK_ENV { assert_eq!(environment.get(std::ffi::OsStr::new(name)), Some(&None)); }
        assert_eq!(environment.get(std::ffi::OsStr::new("CODEX_HOME")), Some(&Some(std::ffi::OsStr::new("user-config"))));
    }
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
        Self::attach_pid(child.id().ok_or("进程已经退出")?)
    }

    pub fn attach_std(child: &std::process::Child) -> Result<Self, String> {
        Self::attach_pid(child.id())
    }

    fn attach_pid(pid: u32) -> Result<Self, String> {
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
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
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
            let _ = pid;
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
