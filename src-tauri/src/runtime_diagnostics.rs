//! Always-on, credential-redacted application diagnostics.
use crate::{
    log_files::{Journal, RETENTION_DAYS},
    log_redaction, AppState,
};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{Arc, Mutex, RwLock},
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Default)]
pub struct Diagnostics {
    journal: Mutex<Option<Journal>>,
    secrets: Arc<RwLock<Vec<String>>>,
}

pub fn initialize(app: &AppHandle, data: &Path) -> Result<(), String> {
    let diagnostics = app.state::<Diagnostics>();
    let journal = Journal::new(data, crate::storage::now())?;
    let directory = journal.directory.clone();
    *diagnostics
        .secrets
        .write()
        .map_err(|_| "日志脱敏初始化失败")? = log_redaction::environment();
    *diagnostics.journal.lock().map_err(|_| "日志初始化失败")? = Some(journal);
    let secrets = diagnostics.secrets.clone();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Avoid the app/database/logger locks: a panic may happen while one is held.
        let message = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("Rust panic");
        let message = secrets
            .try_read()
            .map(|keys| log_redaction::text(message, &keys))
            .unwrap_or_else(|_| "Rust panic（脱敏锁不可用，已省略原始消息）".into());
        let value = json!({"timestamp":chrono::Utc::now().to_rfc3339(),"at":crate::storage::now(),
            "version":env!("CARGO_PKG_VERSION"),"level":"error","event":"rust_panic",
            "details":{"message":message,"location":info.location().map(|l|format!("{}:{}:{}",l.file(),l.line(),l.column()))}});
        if let Err(error) = crate::log_files::panic_record(&directory, &value) {
            eprintln!("崩溃日志保存失败：{error}");
        }
        previous(info);
    }));
    record(
        app,
        None,
        "logger_started",
        json!({"retentionDays":RETENTION_DAYS,"os":std::env::consts::OS,"arch":std::env::consts::ARCH}),
    );
    Ok(())
}

pub fn register_secrets(app: &AppHandle, config: &Value) {
    if let Some(diagnostics) = app.try_state::<Diagnostics>() {
        if let Ok(mut secrets) = diagnostics.secrets.write() {
            log_redaction::collect(config, &mut secrets, 0);
        }
    }
}
pub fn register_mcp_secrets(app: &AppHandle, config: &Value) {
    if let Some(diagnostics) = app.try_state::<Diagnostics>() {
        if let Ok(mut secrets) = diagnostics.secrets.write() {
            log_redaction::collect_mcp(config, &mut secrets, |name| std::env::var(name).ok());
        }
    }
}

pub fn error_type(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if ["context", "token limit", "prompt is too long", "上下文"]
        .iter()
        .any(|s| lower.contains(s))
    {
        "context_limit"
    } else if ["rate limit", "429", "quota", "额度"]
        .iter()
        .any(|s| lower.contains(s))
    {
        "rate_limit"
    } else if ["401", "403", "unauthorized", "authentication"]
        .iter()
        .any(|s| lower.contains(s))
    {
        "authentication"
    } else if ["timeout", "timed out", "stream idle", "超时"]
        .iter()
        .any(|s| lower.contains(s))
    {
        "timeout"
    } else if ["jsonl", "协议", "4 mb"].iter().any(|s| lower.contains(s)) {
        "protocol"
    } else if ["connection", "connect", "连接", "network"]
        .iter()
        .any(|s| lower.contains(s))
    {
        "connection"
    } else {
        "agent_error"
    }
}

fn turn_details(message: &Value) -> Option<(&'static str, Value)> {
    let turn = &message["params"]["turn"];
    match message["method"].as_str()? {
        "turn/started" => Some((
            "turn_started",
            json!({"turnId":turn["id"],"status":"running"}),
        )),
        "turn/completed" => {
            let status = match turn["status"].as_str() {
                Some("completed") => "completed",
                Some("interrupted") => "interrupted",
                _ => "failed",
            };
            Some((
                "turn_completed",
                json!({"turnId":turn["id"],"status":status,
                "errorType":turn["error"]["message"].as_str().map(error_type),"error":turn["error"]["message"]}),
            ))
        }
        "item/completed" if message["params"]["item"]["type"] == "contextCompaction" => Some((
            "context_compacted",
            json!({"turnId":message["params"]["turnId"]}),
        )),
        _ => None,
    }
}

pub fn observe(app: &AppHandle, message: &Value) {
    let Some((event, details)) = turn_details(message) else {
        return;
    };
    let state = app.state::<AppState>();
    let session = message["params"]["appSessionId"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            message["params"]["threadId"]
                .as_str()
                .and_then(|native| state.store.for_native(native).ok())
                .map(|s| s.id)
        });
    record(app, session.as_deref(), event, details);
}

pub fn record(app: &AppHandle, session: Option<&str>, event: &str, details: Value) {
    let level = if details["status"] == "failed"
        || event.contains("failed")
        || event.contains("error")
        || event.contains("panic")
    {
        "error"
    } else {
        "info"
    };
    write(app, session, level, event, details);
}

pub fn message(
    app: &AppHandle,
    session: Option<&str>,
    source: &str,
    level: &str,
    event: &str,
    text: &str,
) {
    let text = app
        .try_state::<Diagnostics>()
        .and_then(|d| {
            d.secrets
                .read()
                .ok()
                .map(|keys| log_redaction::text(text, &keys))
        })
        .unwrap_or_else(|| "日志脱敏不可用，已省略详情".into());
    write(
        app,
        session,
        level,
        event,
        json!({"source":source,"message":text}),
    );
}

pub async fn stderr(
    app: Option<AppHandle>,
    session: Option<String>,
    source: &str,
    stream: impl tokio::io::AsyncRead + Unpin,
) {
    let mut reader = tokio::io::BufReader::new(stream);
    loop {
        match crate::runtime::read_frame(&mut reader).await {
            Ok(Some(frame)) => {
                if let Some(app) = &app {
                    match std::str::from_utf8(&frame) {
                        Ok(text) if !text.trim().is_empty() => message(
                            app,
                            session.as_deref(),
                            source,
                            "warn",
                            "agent_stderr",
                            text.trim(),
                        ),
                        Err(_) => message(
                            app,
                            session.as_deref(),
                            source,
                            "error",
                            "log_encoding_error",
                            "Agent 错误输出不是 UTF-8，已省略原始字节",
                        ),
                        _ => {}
                    }
                }
            }
            Ok(None) => break,
            Err(error) => {
                if let Some(app) = &app {
                    message(
                        app,
                        session.as_deref(),
                        source,
                        "error",
                        "stderr_read_error",
                        &error,
                    );
                }
                break;
            }
        }
    }
}

fn write(app: &AppHandle, session: Option<&str>, level: &str, event: &str, details: Value) {
    let Some(diagnostics) = app.try_state::<Diagnostics>() else {
        return;
    };
    let agent = app.try_state::<AppState>().and_then(|state| {
        session
            .and_then(|id| state.store.session(id).ok())
            .map(|s| s.agent)
    });
    let details = match diagnostics.secrets.read() {
        Ok(secrets) => log_redaction::value(details, &secrets),
        Err(_) => json!({"message":"日志脱敏不可用，已省略详情"}),
    };
    let now = crate::storage::now();
    let value = json!({"at":now,"timestamp":chrono::Utc::now().to_rfc3339(),"version":env!("CARGO_PKG_VERSION"),
        "level":level,"sessionId":session,"agent":agent,"event":event,"details":details});
    let result = (|| {
        let mut guard = diagnostics.journal.lock().map_err(|_| "日志写入锁不可用")?;
        let journal = guard.as_mut().ok_or("日志尚未初始化")?;
        journal.append(
            now,
            &value,
            level == "error" || event == "turn_completed" || event == "app_exited",
        )
    })();
    let _ = app.emit("runtime-log", format!("[SuperCode] {value}"));
    match result {
        Ok(warnings) => {
            for warning in warnings {
                eprintln!("{warning}");
                message(app, None, "logger", "warn", "logger_warning", &warning);
            }
        }
        Err(error) => {
            eprintln!("日志保存失败：{error}");
            let _ = app.emit("runtime-log", format!("日志保存失败：{error}"));
        }
    }
}

pub fn cleanup(app: &AppHandle) {
    let Some(diagnostics) = app.try_state::<Diagnostics>() else {
        return;
    };
    let result = diagnostics
        .journal
        .lock()
        .map_err(|_| "日志清理锁不可用".to_owned())
        .and_then(|mut guard| {
            guard
                .as_mut()
                .ok_or("日志尚未初始化".to_owned())?
                .cleanup_due(crate::storage::now())
        });
    if let Err(error) = result {
        eprintln!("日志清理失败：{error}");
        message(app, None, "logger", "warn", "log_cleanup_error", &error);
    }
}

#[tauri::command]
pub async fn save_runtime_diagnostics(app: AppHandle, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let destination=Path::new(&path);
        if !destination.is_absolute() || !matches!(destination.extension().and_then(|s|s.to_str()),Some("jsonl"|"txt")) {return Err("请选择 .jsonl 或 .txt 日志文件".into());}
        let diagnostics=app.state::<Diagnostics>();
        let snapshot={
            let mut guard=diagnostics.journal.lock().map_err(|_|"日志正在使用")?;
            let journal=guard.as_mut().ok_or("日志尚未初始化")?;
            let parent=destination.parent().ok_or("导出位置无效")?.canonicalize().map_err(|e|e.to_string())?;
            if parent.starts_with(&journal.directory) {return Err("请导出到日志文件夹之外的位置".into());}
            journal.snapshot(crate::storage::now())?
        };
        let header=json!({"event":"diagnostics_export","version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,
            "arch":std::env::consts::ARCH,"at":crate::storage::now(),"retentionDays":RETENTION_DAYS});
        snapshot.export(destination,&header)?;
        Ok(path)
    }).await.map_err(|e|e.to_string())?
}

#[tauri::command]
pub fn open_runtime_log_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let directory = {
        let diagnostics = app.state::<Diagnostics>();
        let guard = diagnostics.journal.lock().map_err(|_| "日志正在使用")?;
        guard.as_ref().ok_or("日志尚未初始化")?.directory.clone()
    };
    app.opener()
        .open_path(directory.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn report_frontend_error(
    app: AppHandle,
    source: String,
    message: String,
    stack: Option<String>,
    command: Option<String>,
) {
    record(
        &app,
        None,
        "frontend_error",
        json!({"source":crate::protocol::bounded(&source,64),"message":message,
        "stack":stack,"command":command.map(|s|crate::protocol::bounded(&s,128))}),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upstream_stream_idle_is_reported_as_a_timeout_with_the_original_reason() {
        let message = json!({"method":"turn/completed","params":{"turn":{"id":"turn","status":"failed","error":{"message":"API Error: upstream stream idle for 3m0s"}}}});
        let (event, details) = turn_details(&message).unwrap();
        assert_eq!(event, "turn_completed");
        assert_eq!(details["status"], "failed");
        assert_eq!(details["errorType"], "timeout");
        assert_eq!(details["error"], "API Error: upstream stream idle for 3m0s");
        assert_eq!(error_type("Agent session is idle"), "agent_error");
    }
    #[test]
    fn errors_are_useful_but_redacted_and_conversations_are_excluded() {
        let message = json!({"method":"turn/completed","params":{"turn":{"id":"turn","status":"failed","error":{"message":"401 authentication failed: private-secret"}},"input":"private-prompt","settings":{"apiKey":"key"}}});
        let (event, details) = turn_details(&message).unwrap();
        assert_eq!(event, "turn_completed");
        assert_eq!(details["errorType"], "authentication");
        let details = log_redaction::value(details, &["private-secret".into()]);
        assert!(details["error"]
            .as_str()
            .unwrap()
            .contains("authentication failed"));
        for secret in ["private-secret", "private-prompt", "apiKey"] {
            assert!(!details.to_string().contains(secret));
        }
        assert!(turn_details(
            &json!({"method":"item/agentMessage/delta","params":{"delta":"private"}})
        )
        .is_none());
    }
    #[test]
    fn compaction_and_interruption_have_distinct_events() {
        let (_, details) = turn_details(
            &json!({"method":"turn/completed","params":{"turn":{"id":"t","status":"interrupted"}}}),
        )
        .unwrap();
        assert_eq!(details["status"], "interrupted");
        assert_eq!(turn_details(&json!({"method":"item/completed","params":{"turnId":"t","item":{"type":"contextCompaction"}}})).unwrap().0,"context_compacted");
    }
}
