//! Stdio adapter. Native requests receive a private, per-turn input gate.
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{atomic::{AtomicBool, Ordering}, mpsc, Arc, Mutex},
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
    unavailable: Arc<AtomicBool>,
    gate: Arc<Mutex<Option<String>>>,
}
#[derive(Debug)]
enum CueFailure { Stopped, Unavailable(String) }
impl CueFailure {
    fn result(&self, builtin: bool) -> Value {
        if builtin {
            match self {
                Self::Stopped => crate::computer::takeover_result(false),
                Self::Unavailable(error) => crate::computer::control_unavailable_result(error),
            }
        } else {
            let message=match self {
                Self::Stopped=>"Desktop control is stopped; continue conversation or permitted non-control tools. Do not regain control without renewed user authorization.",
                Self::Unavailable(error)=>error,
            };
            json!({"isError":true,"content":[{"type":"text","text":message}]})
        }
    }
}
fn revoke_gate(gate: &Mutex<Option<String>>) {
    #[cfg(windows)]
    if let Some(name)=gate.lock().unwrap().as_deref() {
        let _=crate::computer_gate::Gate::revoke(name);
    }
    #[cfg(not(windows))]
    let _=gate;
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
        let unavailable=Arc::new(AtomicBool::new(false));
        let gate=Arc::new(Mutex::new(None::<String>));
        let reader_unavailable=unavailable.clone();
        let reader_gate=gate.clone();
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
            reader_unavailable.store(true,Ordering::Release);
            revoke_gate(&reader_gate);
            // The bundled server obeys this independent gate. Losing the cue
            // stops input while keeping MCP responses and conversation alive.
            // Unmanaged external servers require process shutdown for safety.
            if !builtin { let _ = done.send(Err("电脑提示连接已关闭".into())); }
        });
        Ok(Self {
            write,
            acknowledgements,
            unavailable,
            gate,
        })
    }
    fn fail(&self, message: String) -> CueFailure {
        self.unavailable.store(true,Ordering::Release);
        revoke_gate(&self.gate);
        // Retire this cue transport so the host can mark the grant revoked.
        // MCP stdio remains open; only a new user-authorized turn may rebuild it.
        let _=self.write.shutdown(std::net::Shutdown::Both);
        CueFailure::Unavailable(message)
    }
    fn ack(&self) -> Result<Option<String>, CueFailure> {
        let answer = self
            .acknowledgements
            .recv_timeout(Duration::from_secs(6))
            .map_err(|error| self.fail(match error {
                mpsc::RecvTimeoutError::Timeout=>"The desktop cue did not respond; computer control is cancelled. The conversation remains available.".into(),
                mpsc::RecvTimeoutError::Disconnected=>"The desktop cue connection closed; computer control is cancelled. The conversation remains available.".into(),
            }))?;
        if answer["ok"] == true {
            Ok(answer["gate"].as_str().map(str::to_owned))
        } else if answer["reason"] == "control_revoked" {
            let _=self.fail("Desktop control was revoked; renew user authorization before further input.".into());
            Err(CueFailure::Stopped)
        } else {
            let reason=answer["reason"].as_str().unwrap_or("control_not_ready");
            Err(self.fail(format!("Computer control is unavailable ({reason}); no computer operation was sent. Continue conversation or permitted non-control tools.")))
        }
    }
    fn set(
        &mut self,
        id: &str,
        active: bool,
        gate: Option<&str>,
    ) -> Result<Option<String>, CueFailure> {
        if self.unavailable.load(Ordering::Acquire) {
            return Err(CueFailure::Unavailable("The desktop cue connection is unavailable; computer control is cancelled. The conversation remains available.".into()));
        }
        writeln!(
            self.write,
            "{}",
            json!({"id":id,"active":active,"gate":gate})
        )
        .map_err(|e| self.fail(format!("The desktop cue connection failed: {e}. Computer control is cancelled; the conversation remains available.")))?;
        let result=self.ack();
        if let Ok(Some(name))=&result {
            if active { *self.gate.lock().unwrap()=Some(name.clone()); }
        }
        if active && result.is_ok() && self.unavailable.load(Ordering::Acquire) {
            revoke_gate(&self.gate);
            return Err(CueFailure::Unavailable("The desktop cue connection closed; computer control is cancelled. The conversation remains available.".into()));
        }
        result
    }
}
fn output(writer: &Mutex<std::io::Stdout>, bytes: &[u8]) -> Result<(), String> {
    let mut writer = writer.lock().map_err(|e| e.to_string())?;
    writer
        .write_all(bytes)
        .and_then(|_| writer.flush())
        .map_err(|e| e.to_string())
}
type PendingCalls=Mutex<std::collections::HashMap<String,Option<String>>>;
fn relay_output(mut reader: impl BufRead, pending: &PendingCalls, cue: &Mutex<Cue>, mut forward: impl FnMut(&[u8])->Result<(),String>) -> Result<(),String> {
    while let Some(bytes)=frame(&mut reader)? {
        let value:Value=serde_json::from_slice(&bytes).map_err(|_|"电脑 MCP 返回无效 JSON")?;
        if value.get("method").is_none() && (value.get("result").is_some() || value.get("error").is_some()) {
            if let Some(id)=call_id(&value) {
                if let Some(gate)=pending.lock().unwrap().remove(&id) {
                    // Completion acknowledgement is bookkeeping. A failed cue
                    // cannot discard a native response or close conversation.
                    let _=cue.lock().unwrap().set(&id,false,gate.as_deref());
                }
            }
        }
        forward(&bytes)?;
    }
    Ok(())
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
                            let result = error.result(builtin);
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
        let result=relay_output(BufReader::new(stdout),&pending,&cue,|bytes|output(&writer,bytes));
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
    fn fixture_cue(answer: Option<Value>) -> (Cue,TcpStream) {
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let write=TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (peer,_)=listener.accept().unwrap();
        let (tx,acknowledgements)=mpsc::channel();
        if let Some(answer)=answer { tx.send(answer).unwrap(); }
        drop(tx);
        (Cue { write,acknowledgements,unavailable:Arc::new(AtomicBool::new(false)),gate:Arc::new(Mutex::new(None)) },peer)
    }
    #[test]
    fn completion_cue_failure_preserves_tool_reply_and_following_conversation_frames() {
        let reply=b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"isError\":false,\"content\":[{\"type\":\"text\",\"text\":\"cancelled desktop operation\"}]}}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}\n";
        for answer in [None,Some(json!({"ok":false,"reason":"control_revoked"})),Some(json!({"ok":false,"reason":"cue_unavailable"}))] {
            let pending=Mutex::new(std::collections::HashMap::from([("1".into(),None)]));
            let (cue,_peer)=fixture_cue(answer);
            let cue=Mutex::new(cue);
            let mut output=Vec::new();
            relay_output(std::io::Cursor::new(reply),&pending,&cue,|bytes| { output.extend_from_slice(bytes);Ok(()) }).unwrap();
            assert_eq!(output,reply);
            assert!(pending.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn readiness_denial_is_not_reported_as_user_takeover_and_never_forwards_input() {
        for (reason,code) in [("control_revoked","user_takeover"),("cue_timeout","computer_control_unavailable"),("desktop_busy","computer_control_unavailable"),("missing_session","computer_control_unavailable"),("stale_grant","computer_control_unavailable")] {
            let (mut cue,_peer)=fixture_cue(Some(json!({"ok":false,"reason":reason})));
            let error=cue.set("attempt",true,None).unwrap_err();
            let result=error.result(true);
            assert_eq!(result["isError"],false);
            let detail:Value=serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
            assert_eq!(detail["code"],code);
            assert!(detail["nextStep"].as_str().unwrap().contains("Continue the user's task"));
            assert!(cue.unavailable.load(Ordering::Acquire));
            assert!(matches!(cue.set("retry",true,None),Err(CueFailure::Unavailable(_))));
        }
    }
    #[test]
    fn builtin_cue_disconnect_revokes_input_without_ending_native_transport() {
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address=listener.local_addr().unwrap().to_string();
        let gate_name=crate::computer_gate::event_name(&uuid::Uuid::new_v4().to_string());
        let gate=crate::computer_gate::Gate::create(&gate_name).unwrap();
        gate.arm().unwrap();
        let server_gate=gate_name.clone();
        let (close,closed)=mpsc::channel();
        let server=std::thread::spawn(move || {
            let (mut socket,_)=listener.accept().unwrap();
            let mut input=BufReader::new(socket.try_clone().unwrap());
            let mut line=String::new();
            input.read_line(&mut line).unwrap();socket.write_all(b"ok\n").unwrap();
            line.clear();input.read_line(&mut line).unwrap();
            writeln!(socket,"{}",json!({"ok":true,"gate":server_gate})).unwrap();
            closed.recv().unwrap();
        });
        let (done,result)=mpsc::channel();
        let mut cue=Cue::connect(&address,"fixture","codex",true,done).unwrap();
        assert_eq!(cue.set("first",true,None).unwrap().as_deref(),Some(gate_name.as_str()));
        close.send(()).unwrap();server.join().unwrap();
        assert!(matches!(cue.ack(),Err(CueFailure::Unavailable(_))));
        #[cfg(windows)] assert!(gate.stopped());
        assert!(matches!(cue.set("second",true,None),Err(CueFailure::Unavailable(_))));
        assert!(result.try_recv().is_err()); // No relay-fatal completion was sent.
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
