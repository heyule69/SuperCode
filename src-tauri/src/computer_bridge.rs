//! Stdio adapter. Native requests receive a private, per-turn input gate.
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

const MAX_FRAME: usize = 8 * 1024 * 1024;
fn frame(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, String> {
    let mut bytes = Vec::new();
    let n = reader
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(None);
    }
    if n > MAX_FRAME || !bytes.ends_with(b"\n") {
        return Err("电脑 MCP 协议帧超出限制或不完整".into());
    }
    Ok(Some(bytes))
}
fn call_id(value: &Value) -> Option<String> {
    value
        .get("id")
        .filter(|id| id.is_string() || id.is_number())
        .map(Value::to_string)
        .filter(|id| id.len() <= 256)
}
struct Cue {
    write: TcpStream,
    acknowledgements: mpsc::Receiver<Value>,
}
impl Cue {
    fn connect(
        address: &str,
        token: &str,
        agent: &str,
        builtin: bool,
        done: mpsc::Sender<Result<(), String>>,
    ) -> Result<Self, String> {
        let address: SocketAddr = address.parse().map_err(|_| "电脑提示连接地址无效")?;
        if !address.ip().is_loopback() {
            return Err("电脑提示只能连接本机".into());
        }
        let mut write = TcpStream::connect_timeout(&address, Duration::from_secs(3))
            .map_err(|e| e.to_string())?;
        write
            .set_read_timeout(Some(Duration::from_secs(6)))
            .map_err(|e| e.to_string())?;
        write
            .set_write_timeout(Some(Duration::from_secs(3)))
            .map_err(|e| e.to_string())?;
        write.set_nodelay(true).map_err(|e| e.to_string())?;
        let mut read = BufReader::new(write.try_clone().map_err(|e| e.to_string())?);
        writeln!(
            write,
            "{}",
            json!({"token":token,"agent":agent,"session":std::env::var("SUPERCODE_SESSION_ID").ok(),"builtin":builtin})
        )
        .map_err(|e| e.to_string())?;
        let mut answer = Vec::new();
        (&mut read)
            .take(16)
            .read_until(b'\n', &mut answer)
            .map_err(|e| e.to_string())?;
        if answer != b"ok\n" {
            return Err("电脑操作提示连接被拒绝".into());
        }
        write.set_read_timeout(None).map_err(|e| e.to_string())?;
        // Windows socket clones retain their own receive timeout. Clear it on
        // the actual reader too: model thinking and UIA calls may take minutes.
        read.get_ref()
            .set_read_timeout(None)
            .map_err(|e| e.to_string())?;
        let (tx, acknowledgements) = mpsc::channel();
        std::thread::spawn(move || {
            loop {
                let mut answer = Vec::new();
                if (&mut read)
                    .take(2049)
                    .read_until(b'\n', &mut answer)
                    .is_err()
                    || answer.is_empty()
                    || answer.len() > 2048
                {
                    break;
                }
                let value = match answer.as_slice() {
                    b"ok\n" => json!({"ok":true}),
                    b"blocked\n" => json!({"ok":false}),
                    _ => match serde_json::from_slice::<Value>(&answer) {
                        Ok(value) if value["ok"].is_boolean() => value,
                        _ => break,
                    },
                };
                if tx.send(value).is_err() {
                    break;
                }
            }
            // Host exit is a transport failure. User takeover keeps native transport alive.
            let _ = done.send(Err("电脑操作已停止或提示连接已关闭".into()));
        });
        Ok(Self {
            write,
            acknowledgements,
        })
    }
    fn ack(&self) -> Result<Option<String>, String> {
        let answer = self
            .acknowledgements
            .recv_timeout(Duration::from_secs(6))
            .map_err(|_| "电脑控制连接未及时回应".to_string())?;
        if answer["ok"] == true {
            Ok(answer["gate"].as_str().map(str::to_owned))
        } else {
            Err(
                "电脑控制未就绪或用户已接管，请回到 SuperCode 查看提示。不要重试或重新抢占焦点"
                    .into(),
            )
        }
    }
    fn set(
        &mut self,
        id: &str,
        active: bool,
        gate: Option<&str>,
    ) -> Result<Option<String>, String> {
        writeln!(
            self.write,
            "{}",
            json!({"id":id,"active":active,"gate":gate})
        )
        .map_err(|e| e.to_string())?;
        self.ack()
    }
}
fn output(writer: &Mutex<std::io::Stdout>, bytes: &[u8]) -> Result<(), String> {
    let mut writer = writer.lock().map_err(|e| e.to_string())?;
    writer
        .write_all(bytes)
        .and_then(|_| writer.flush())
        .map_err(|e| e.to_string())
}

/// Called before Tauri initialization: this mode creates no window or tray.
pub fn run(args: &[String]) -> Result<(), String> {
    if args.len() < 4 {
        return Err("电脑 MCP 启动参数不完整".into());
    }
    let (tx, rx) = mpsc::channel();
    let builtin = args.get(4).is_some_and(|arg| arg == "--computer-mcp");
    let cue = Arc::new(Mutex::new(Cue::connect(
        &args[0],
        &args[1],
        &args[2],
        builtin,
        tx.clone(),
    )?));
    let mut command = std::process::Command::new(&args[3]);
    command
        .env(
            crate::computer_gate::ENVIRONMENT,
            crate::computer_gate::event_name(&args[1]),
        )
        .env("SUPERCODE_COMPUTER_CUE_ADDRESS", &args[0])
        .env("SUPERCODE_COMPUTER_AGENT", &args[2])
        .args(&args[4..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("电脑 MCP 无法启动：{e}"))?;
    let _job = match crate::process::JobGuard::attach_std(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let mut input = child.stdin.take().ok_or("电脑 MCP 缺少输入管道")?;
    let stdout = child.stdout.take().ok_or("电脑 MCP 缺少输出管道")?;
    let pending = Arc::new(Mutex::new(std::collections::HashMap::<
        String,
        Option<String>,
    >::new()));
    let writer = Arc::new(Mutex::new(std::io::stdout()));
    let input_cue = cue.clone();
    let input_pending = pending.clone();
    let input_writer = writer.clone();
    let input_done = tx.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let mut reader = BufReader::new(std::io::stdin());
            while let Some(mut bytes) = frame(&mut reader)? {
                let mut value: Value =
                    serde_json::from_slice(&bytes).map_err(|_| "电脑 MCP 收到无效 JSON")?;
                if value["method"] == "tools/call" {
                    let id = call_id(&value).ok_or("电脑工具请求缺少有效 ID")?;
                    if input_pending.lock().unwrap().len() >= 128 {
                        return Err("同时运行的电脑工具过多".into());
                    }
                    let gate = match input_cue.lock().unwrap().set(&id, true, None) {
                        Ok(gate) => gate,
                        Err(error) => {
                            let result = if builtin {
                                crate::computer::takeover_result(false)
                            } else {
                                json!({"isError":true,"content":[{"type":"text","text":error}]})
                            };
                            let mut answer = serde_json::to_vec(
                                &json!({"jsonrpc":"2.0","id":value["id"],"result":result}),
                            )
                            .map_err(|e| e.to_string())?;
                            answer.push(b'\n');
                            output(&input_writer, &answer)?;
                            continue;
                        }
                    };
                    if builtin {
                        // Model-provided metadata is always overwritten by the trusted host.
                        value["_supercodeGate"] = json!(gate);
                        bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
                        bytes.push(b'\n');
                    }
                    input_pending.lock().unwrap().insert(id, gate);
                }
                input
                    .write_all(&bytes)
                    .and_then(|_| input.flush())
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        let _ = input_done.send(result);
    });
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let mut reader = BufReader::new(stdout);
            while let Some(bytes) = frame(&mut reader)? {
                let value: Value =
                    serde_json::from_slice(&bytes).map_err(|_| "电脑 MCP 返回无效 JSON")?;
                if value.get("method").is_none()
                    && (value.get("result").is_some() || value.get("error").is_some())
                {
                    if let Some(id) = call_id(&value) {
                        let finished = pending.lock().unwrap().remove(&id);
                        if let Some(gate) = finished {
                            cue.lock().unwrap().set(&id, false, gate.as_deref())?;
                        }
                    }
                }
                output(&writer, &bytes)?;
            }
            Ok(())
        })();
        let _ = tx.send(result);
    });
    let result = rx.recv().map_err(|e| e.to_string())?;
    // Either pipe ending shuts down the whole relay, including a stalled server.
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_preserve_unicode_images_and_native_requests_byte_for_byte() {
        for bytes in [
            b"{\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"Snapshot\"}}\n".as_slice(),
            "{\"method\":\"elicitation/create\",\"params\":{\"message\":\"需要回答\"}}\r\n"
                .as_bytes(),
        ] {
            assert_eq!(
                frame(&mut std::io::Cursor::new(bytes)).unwrap().unwrap(),
                bytes
            );
        }
        assert!(frame(&mut std::io::Cursor::new(b"incomplete")).is_err());
        assert!(frame(&mut std::io::Cursor::new(vec![b'a'; MAX_FRAME + 1])).is_err());
    }
    #[test]
    fn string_and_numeric_ids_remain_distinct_and_arbitrary_ids_are_rejected() {
        assert_ne!(call_id(&json!({"id":1})), call_id(&json!({"id":"1"})));
        for id in [Value::Null, json!({}), json!("a".repeat(300))] {
            assert!(call_id(&json!({"id":id})).is_none());
        }
    }
    #[test]
    fn a_slow_tool_and_idle_model_do_not_close_the_cue_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut input = BufReader::new(socket.try_clone().unwrap());
            // Handshake, start of tool, then end after a slow tool/model pause.
            for _ in 0..3 {
                let mut line = String::new();
                assert!(input.read_line(&mut line).unwrap() > 0);
                socket.write_all(b"ok\n").unwrap();
            }
        });
        let (done, result) = mpsc::channel();
        let mut cue = Cue::connect(&address, "fixture", "claude", false, done).unwrap();
        cue.set("fixture-call", true, None).unwrap();
        assert!(matches!(
            result.recv_timeout(Duration::from_secs(7)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        cue.set("fixture-call", false, None).unwrap();
        server.join().unwrap();
        // Actual host disconnect still ends the relay immediately.
        assert!(result
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_err());
    }
}
