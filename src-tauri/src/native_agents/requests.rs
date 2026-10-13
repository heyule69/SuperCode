//! Native JSONL request transport and process lifecycle.
use super::*;

pub(super) fn rebind_public_requests(
    requests: &mut HashMap<String, Value>,
    previous: &str,
    native: &str,
    session: &str,
) -> Vec<Value> {
    if previous == native {
        return vec![];
    }
    requests
        .values_mut()
        .filter_map(|request| {
            if request["params"]["threadId"] != previous {
                return None;
            }
            // Only the host's presentation binding changes. Native IDs, raw
            // request payloads and reply correlation remain exactly as received.
            request["params"]["threadId"] = json!(native);
            request["params"]["appSessionId"] = json!(session);
            Some(request.clone())
        })
        .collect()
}

type Reply = oneshot::Sender<Result<Value, String>>;
pub(super) struct Client {
    pub(super) diagnostics: Option<(AppHandle, String, Option<String>)>,
    pub(super) cwd: String,
    pub(super) pid: u32,
    pub(super) input: Mutex<ChildStdin>,
    pub(super) child: Mutex<Child>,
    pub(super) _job: process::JobGuard,
    pub(super) pending: Mutex<HashMap<String, Reply>>,
    pub(super) sequence: AtomicU64,
    pub(super) pi: bool,
    pub(super) alive: AtomicBool,
    pub(super) concurrent_prompt: AtomicBool,
    transient_home: Option<std::path::PathBuf>,
}
impl Client {
    pub(super) async fn start(
        launch: &Launch,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        pi: bool,
        diagnostics: Option<(AppHandle, String, Option<String>)>,
    ) -> Result<(Arc<Self>, mpsc::Receiver<Value>), String> {
        let mut command = launch.command(&[]);
        let gemini = args.first().is_some_and(|arg| arg == "--acp");
        if gemini {
            // A managed API connection must not inherit Google login, Vertex,
            // API keys, hooks or another CLI's settings from its parent process.
            for name in [
                "GEMINI_API_KEY",
                "GOOGLE_API_KEY",
                "GOOGLE_APPLICATION_CREDENTIALS",
                "GOOGLE_GENAI_USE_VERTEXAI",
                "GOOGLE_GENAI_USE_GCA",
                "GOOGLE_VERTEX_BASE_URL",
                "GOOGLE_CLOUD_PROJECT",
                "GOOGLE_CLOUD_PROJECT_ID",
                "GOOGLE_CLOUD_LOCATION",
                "GOOGLE_GEMINI_BASE_URL",
                "GOOGLE_GENAI_API_VERSION",
                "GEMINI_CLI_CUSTOM_HEADERS",
                "GEMINI_API_KEY_AUTH_MECHANISM",
                "GEMINI_SYSTEM_MD",
                "GEMINI_FORCE_FILE_STORAGE",
                "GEMINI_CLI_USE_COMPUTE_ADC",
                "GEMINI_CLI_HOME",
                "GEMINI_CLI_SYSTEM_SETTINGS_PATH",
                "GEMINI_CLI_SYSTEM_DEFAULTS_PATH",
            ] {
                command.env_remove(name);
            }
        }
        let transient_home = gemini
            .then(|| {
                let home = std::path::PathBuf::from(
                    env.iter()
                        .find(|(key, _)| key == "GEMINI_CLI_HOME")?
                        .1
                        .clone(),
                );
                let name = home.file_name()?.to_str()?;
                (uuid::Uuid::parse_str(name).is_ok() && home.parent()?.file_name()? == "transient")
                    .then_some(home)
            })
            .flatten();
        if let Some((app, _, _)) = &diagnostics {
            let values = env
                .iter()
                .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                .collect::<serde_json::Map<_, _>>();
            crate::runtime_diagnostics::register_secrets(app, &Value::Object(values));
        }
        command
            .args(args)
            .current_dir(cwd)
            .envs(env.iter().cloned())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| format!("无法启动 Agent：{e}"))?;
        let job = process::JobGuard::attach(&child)?;
        let input = child.stdin.take().ok_or("Agent 无输入管道")?;
        let output = child.stdout.take().ok_or("Agent 无输出管道")?;
        let stderr = child.stderr.take().ok_or("Agent 无错误管道")?;
        let client = Arc::new(Self {
            diagnostics: diagnostics.clone(),
            cwd: cwd.to_string_lossy().into_owned(),
            pid: child.id().unwrap_or(0),
            input: Mutex::new(input),
            child: Mutex::new(child),
            _job: job,
            pending: Mutex::new(HashMap::new()),
            sequence: AtomicU64::new(1),
            alive: AtomicBool::new(true),
            pi,
            concurrent_prompt: AtomicBool::new(false),
            transient_home,
        });
        let reader = client.clone();
        let (tx, rx) = mpsc::channel(16);
        if let Some((app, source, session)) = &diagnostics {
            crate::runtime_diagnostics::record(
                app,
                session.as_deref(),
                "agent_process_started",
                json!({"source":source,"pid":client.pid}),
            );
        }
        // Drain diagnostics without retaining unbounded output in memory.
        tokio::spawn(async move {
            let (app, source, session) = diagnostics
                .map(|(app, source, session)| (Some(app), source, session))
                .unwrap_or((
                    None,
                    if pi {
                        "pi"
                    } else if gemini {
                        "gemini"
                    } else {
                        "opencode"
                    }
                    .into(),
                    None,
                ));
            crate::runtime_diagnostics::stderr(app, session, &source, stderr).await;
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(output);
            loop {
                let value = match runtime::read_frame(&mut lines).await {
                    Ok(Some(line)) => match serde_json::from_slice::<Value>(&line) {
                        Ok(v) => v,
                        Err(error) => {
                            reader.log("protocol_parse_error", &error.to_string());
                            continue;
                        }
                    },
                    Ok(None) => break,
                    Err(error) => {
                        reader.log("protocol_read_error", &error);
                        break;
                    }
                };
                if is_response(&value, pi) {
                    let key = value["id"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value["id"].to_string());
                    if let Some(reply) = reader.pending.lock().await.remove(&key) {
                        let result = response(&value, pi);
                        if let Err(error) = &result {
                            reader.log("agent_request_error", error);
                        }
                        let _ = reply.send(result);
                    }
                    continue;
                }
                if tx.send(value).await.is_err() {
                    break;
                }
            }
            let intentional = !reader.alive.swap(false, Ordering::AcqRel);
            let exit_code = reader
                .child
                .lock()
                .await
                .try_wait()
                .ok()
                .flatten()
                .and_then(|s| s.code());
            if let Some((app, source, session)) = &reader.diagnostics {
                crate::runtime_diagnostics::record(
                    app,
                    session.as_deref(),
                    "agent_process_output_closed",
                    json!({"source":source,"pid":reader.pid,"exitCode":exit_code,"intentional":intentional}),
                );
            }
            for (_, reply) in reader.pending.lock().await.drain() {
                let _ = reply.send(Err("Agent 连接已关闭".into()));
            }
            let _ = tx.send(json!({"type":"supercode_disconnected"})).await;
        });
        Ok((client, rx))
    }
    pub(super) fn log(&self, event: &str, text: &str) {
        if let Some((app, source, session)) = &self.diagnostics {
            crate::runtime_diagnostics::message(
                app,
                session.as_deref(),
                source,
                "error",
                event,
                text,
            );
        }
    }
    pub(super) async fn write(&self, value: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        self.input
            .lock()
            .await
            .write_all(&bytes)
            .await
            .map_err(|e| e.to_string())
    }
    pub(super) async fn request(
        &self,
        method: &str,
        params: Value,
        seconds: u64,
    ) -> Result<Value, String> {
        let (id, rx) = self.begin_request(method, params).await?;
        let result = tokio::time::timeout(Duration::from_secs(seconds), rx).await;
        self.pending.lock().await.remove(&id);
        result
            .map_err(|_| format!("Agent 请求 {method} 超时"))?
            .map_err(|_| "Agent 已退出".to_owned())?
    }
    pub(super) async fn begin_request(
        &self,
        method: &str,
        mut params: Value,
    ) -> Result<(String, oneshot::Receiver<Result<Value, String>>), String> {
        let id = self.sequence.fetch_add(1, Ordering::Relaxed).to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        let value = if self.pi {
            params["type"] = method.into();
            params["id"] = id.clone().into();
            params
        } else {
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        };
        if let Err(e) = self.write(value).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }
        Ok((id, rx))
    }
    pub(super) async fn close(&self) {
        self.alive.store(false, Ordering::Release);
        let _ = self.child.lock().await.kill().await;
        if let Some(home) = &self.transient_home {
            // The exact owned UUID directory must still be beneath its expected
            // parent before recursive cleanup. Never remove a reparse target.
            let expected = home.parent().and_then(|p| p.canonicalize().ok());
            let resolved = home.canonicalize().ok();
            let ordinary = std::fs::symlink_metadata(home).ok().is_some_and(|m| {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    m.file_attributes() & 0x400 == 0 && m.is_dir()
                }
                #[cfg(not(windows))]
                {
                    m.is_dir() && !m.file_type().is_symlink()
                }
            });
            if let (true, Some(resolved), Some(expected)) = (ordinary, resolved, expected) {
                if resolved.parent() == Some(expected.as_path()) {
                    let _ = tokio::fs::remove_dir_all(&resolved).await;
                }
            }
        }
    }
}
pub(super) fn is_response(v: &Value, pi: bool) -> bool {
    if pi {
        v["type"] == "response"
    } else {
        v.get("id").is_some() && v.get("method").is_none()
    }
}
pub(super) fn response(v: &Value, pi: bool) -> Result<Value, String> {
    if pi {
        if v["success"] == false {
            Err(protocol::bounded(
                v["error"].as_str().unwrap_or("Pi 请求失败"),
                4096,
            ))
        } else {
            Ok(v["data"].clone())
        }
    } else if v.get("error").is_some() {
        Err(protocol::bounded(
            v["error"]["message"].as_str().unwrap_or("ACP 请求失败"),
            4096,
        ))
    } else {
        Ok(v["result"].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_requests_rebind_to_actual_native_session_without_changing_wire_or_children() {
        for agent in ["pi", "opencode", "gemini"] {
            let raw = if agent == "pi" {
                json!({"type":"extension_ui_request","method":"confirm","id":"native-request","title":"Startup"})
            } else {
                json!({"jsonrpc":"2.0","method":"session/request_permission","id":41,"params":{"sessionId":"actual-native","options":[{"kind":"allow_once","optionId":"once"}]}})
            };
            let id = format!("{agent}:chat:startup");
            let mut table = HashMap::from([
                (
                    id.clone(),
                    json!({"id":id,"method":"claude/tool/requestApproval","params":{"threadId":"temporary-chat-id","turnId":"same-turn","nativeRequest":raw}}),
                ),
                (
                    "child".into(),
                    json!({"id":"child","params":{"threadId":"different-child","nativeRequest":{"id":"child-wire"}}}),
                ),
            ]);
            let emitted =
                rebind_public_requests(&mut table, "temporary-chat-id", "actual-native", "chat");
            assert_eq!(emitted.len(), 1);
            assert_eq!(emitted[0]["id"], id);
            assert_eq!(emitted[0]["params"]["threadId"], "actual-native");
            assert_eq!(emitted[0]["params"]["appSessionId"], "chat");
            assert_eq!(emitted[0]["params"]["turnId"], "same-turn");
            assert_eq!(emitted[0]["params"]["nativeRequest"], raw);
            assert_eq!(table[&id], emitted[0]);
            assert_eq!(table["child"]["params"]["threadId"], "different-child");
            let reply =
                super::super::permission_reply(&table[&id], &json!({"decision":"accept"}), agent)
                    .unwrap();
            assert_eq!(reply["id"], raw["id"]);
            if agent == "pi" {
                assert_eq!(reply["confirmed"], true);
            } else {
                assert_eq!(reply["result"]["outcome"]["optionId"], "once");
            }
            assert!(
                rebind_public_requests(&mut table, "actual-native", "actual-native", "chat")
                    .is_empty()
            );
        }
    }
}
