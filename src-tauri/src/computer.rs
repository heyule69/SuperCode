//! SuperCode's bundled desktop MCP. Native calls run in disposable, timed workers;
//! a stalled accessibility provider cannot break the MCP connection.
use crate::{client_features::ToolServer, process};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::{BufRead, BufReader, Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub const VERSION: &str = "1";
pub const TOOL_COUNT: usize = 9;
const MAX_FRAME: usize = 8 * 1024 * 1024;
const OBSERVATION_AGE: Duration = Duration::from_secs(60);

pub fn server() -> Result<ToolServer, String> {
    Ok(ToolServer {
        id: "computer".into(),
        name: "电脑 · SuperCode 内置".into(),
        kind: "computer".into(),
        command: std::env::current_exe()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned(),
        args: vec!["--computer-mcp".into()],
        url: None,
        enabled: cfg!(windows),
    })
}

/// Upgrade only the reserved managed entry. Custom servers are never rewritten.
pub fn with_builtin(mut saved: Vec<ToolServer>, builtin: ToolServer) -> Vec<ToolServer> {
    let enabled = saved
        .iter()
        .find(|s| s.id == "computer" && s.kind == "computer")
        .map(|s| s.enabled)
        .unwrap_or(builtin.enabled);
    if saved
        .iter()
        .any(|s| s.id == "computer" && s.kind != "computer")
    {
        return saved;
    }
    saved.retain(|s| s.id != "computer");
    saved.push(ToolServer { enabled, ..builtin });
    saved
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str], read: bool) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},
        "annotations":{"readOnlyHint":read,"destructiveHint":!read,"idempotentHint":read,"openWorldHint":!read}})
}
pub fn tools() -> Vec<Value> {
    let window = json!({"type":"string","description":"Exact windowId from desktop_list_windows; do not guess."});
    let sid = json!({"type":"string","description":"snapshotId from the latest desktop_snapshot; expires after 60 seconds or a modifying action."});
    let coord = json!({"type":"integer","minimum":0,"description":"Pixel in the returned screenshot, NOT desktop coordinates."});
    vec![
        tool("desktop_list_windows", "List visible desktop windows and the foreground window. Built into SuperCode; no Python or external desktop MCP is needed.", json!({}), &[], true),
        tool("desktop_focus_window", "Bring an observed window to the foreground. Fails if Windows refuses focus. Returns a fresh screenshot and snapshotId. If the user changed focus, stop instead of refocusing.", json!({"windowId":window}), &["windowId"], false),
        tool("desktop_snapshot", "Read the foreground window: screenshot and bounded accessibility tree. Select a windowId or use the foreground window. Does not change focus. Snapshot coordinates are image pixels. If the window is obscured, focus it first. UIA failure is reported alongside the screenshot.", json!({"windowId":window,"includeImage":{"type":"boolean","default":true},"includeTree":{"type":"boolean","default":true}}), &[], true),
        tool("desktop_move", "Move the visible pointer smoothly to an observed screenshot coordinate. Verify the window remains foreground.", json!({"snapshotId":sid,"x":coord,"y":coord}), &["snapshotId","x","y"], false),
        tool("desktop_click", "Click a coordinate in a recent snapshot. Requires the same foreground window and geometry. Returns a fresh screenshot and snapshotId; use the returned ID for the next action. Never blindly retry a failed action.", json!({"snapshotId":sid,"x":coord,"y":coord,"button":{"type":"string","enum":["left","right","middle"],"default":"left"},"count":{"type":"integer","minimum":1,"maximum":2,"default":1}}), &["snapshotId","x","y"], false),
        tool("desktop_drag", "Drag between two screenshot coordinates. Releases the mouse button on interruption. Returns a fresh screenshot and snapshotId; use the new ID for the next action.", json!({"snapshotId":sid,"fromX":coord,"fromY":coord,"toX":coord,"toY":coord}), &["snapshotId","fromX","fromY","toX","toY"], false),
        tool("desktop_scroll", "Scroll at an observed screenshot point. vertical/horizontal are wheel notches, -10 to 10; positive vertical is up, positive horizontal is right. Returns a fresh screenshot and snapshotId; use the new ID for the next action.", json!({"snapshotId":sid,"x":coord,"y":coord,"vertical":{"type":"integer","minimum":-10,"maximum":10,"default":0},"horizontal":{"type":"integer","minimum":-10,"maximum":10,"default":0}}), &["snapshotId","x","y"], false),
        tool("desktop_type", "Type literal Unicode into the focused control of an observed foreground window. Does not use the clipboard, run shell commands, or submit Enter. Returns a fresh screenshot and snapshotId to verify the input.", json!({"windowId":window,"text":{"type":"string","maxLength":4096}}), &["windowId","text"], false),
        tool("desktop_key", "Press a key/chord in the observed foreground window, e.g. CTRL+A, ENTER, TAB, ALT+LEFT. Supported modifiers CTRL/ALT/SHIFT/WIN; keys A-Z, 0-9, F1-F12, ENTER, TAB, ESC, SPACE, BACKSPACE, DELETE, arrows, HOME, END, PAGEUP, PAGEDOWN. Returns a fresh screenshot and snapshotId; use the new ID for the next action.", json!({"windowId":window,"keys":{"type":"string","maxLength":64}}), &["windowId","keys"], false),
    ]
}

fn frame(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    limited_frame(reader, MAX_FRAME)
}
fn limited_frame(reader: &mut impl BufRead, limit: usize) -> Result<Option<Value>, String> {
    let mut bytes = Vec::new();
    let n = reader
        .take((limit + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(None);
    }
    if n > limit || !bytes.ends_with(b"\n") {
        return Err("MCP frame too large or incomplete".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| e.to_string())
}
type Output = Arc<Mutex<std::io::Stdout>>;
fn send(out: &Output, value: Value) {
    if let Ok(mut out) = out.lock() {
        let _ = serde_json::to_writer(&mut *out, &value);
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    }
}
fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn text_result(value: Value) -> Value {
    json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
}
pub(crate) fn takeover_result(action_may_have_run: bool) -> Value {
    text_result(
        json!({"status":"stopped","code":"user_takeover","actionMayHaveRun":action_may_have_run,
        "message":"User took control. Desktop operation cancelled; the tool connection is still available.",
        "nextStep":"Stop and wait for a new user request. Do not retry, refocus, or use shell commands to regain control."}),
    )
}
fn settle(cancel: &dyn Fn() -> bool) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(200) {
        if cancel() {
            return Err("Desktop operation cancelled".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[derive(Default)]
struct Engine {
    observation: Option<(String, Instant, Value)>,
}
impl Engine {
    fn arguments(name: &str, args: &Value) -> Result<(), String> {
        let schema = tools()
            .into_iter()
            .find(|t| t["name"] == name)
            .ok_or("Unknown desktop tool")?;
        let args = args.as_object().ok_or("Arguments must be an object")?;
        let props = schema["inputSchema"]["properties"].as_object().unwrap();
        for required in schema["inputSchema"]["required"].as_array().unwrap() {
            if !args.contains_key(required.as_str().unwrap()) {
                return Err(format!("Missing {}", required));
            }
        }
        for (k, v) in args {
            let rule = props
                .get(k)
                .ok_or_else(|| format!("Unknown argument: {k}"))?;
            let valid = match rule["type"].as_str() {
                Some("string") => v.as_str().is_some_and(|s| {
                    !s.contains('\0')
                        && s.chars().count() <= rule["maxLength"].as_u64().unwrap_or(128) as usize
                }),
                Some("boolean") => v.is_boolean(),
                Some("integer") => v.as_i64().is_some_and(|n| {
                    n >= rule["minimum"].as_i64().unwrap_or(i64::MIN)
                        && n <= rule["maximum"].as_i64().unwrap_or(32768)
                }),
                _ => false,
            };
            if !valid
                || rule["enum"]
                    .as_array()
                    .is_some_and(|values| !values.contains(v))
            {
                return Err(format!("Invalid argument: {k}"));
            }
        }
        Ok(())
    }
    fn call(
        &mut self,
        name: &str,
        mut args: Value,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Value, String> {
        Self::arguments(name, &args)?;
        if !cfg!(windows) {
            return Err("Built-in desktop control currently requires Windows".into());
        }
        if matches!(
            name,
            "desktop_move" | "desktop_click" | "desktop_drag" | "desktop_scroll"
        ) {
            let (sid, created, state) = self
                .observation
                .as_ref()
                .ok_or("Take a fresh desktop_snapshot first")?;
            if args["snapshotId"] != *sid || created.elapsed() > OBSERVATION_AGE {
                return Err("Snapshot expired; take a fresh desktop_snapshot".into());
            }
            for (x, y) in if name == "desktop_drag" {
                vec![("fromX", "fromY"), ("toX", "toY")]
            } else {
                vec![("x", "y")]
            } {
                if args[x].as_i64().unwrap() >= state["imageWidth"].as_i64().unwrap_or(0)
                    || args[y].as_i64().unwrap() >= state["imageHeight"].as_i64().unwrap_or(0)
                {
                    return Err("Coordinate lies outside the snapshot".into());
                }
            }
            args["observed"] = state.clone();
        }
        // Consume observation before side effects; uncertainty is never retried automatically.
        if !matches!(
            name,
            "desktop_list_windows" | "desktop_snapshot" | "desktop_move"
        ) {
            self.observation = None;
        }
        if name == "desktop_snapshot" {
            return self.snapshot(args, cancel);
        }
        let timeout = if name == "desktop_type" {
            Duration::from_secs(
                5 + args["text"].as_str().unwrap_or("").chars().count() as u64 / 250,
            )
        } else {
            Duration::from_secs(8)
        };
        let value = native_call(name, &args, timeout, cancel)?;
        if !matches!(name, "desktop_list_windows" | "desktop_move") {
            // Give the UI a bounded, cancellable paint interval. This captures
            // evidence, not proof that navigation/business intent has completed.
            match settle(cancel).and_then(|_| self.snapshot(json!({"includeTree":false}), cancel)) {
                Ok(mut fresh) => {
                    let mut metadata: Value =
                        serde_json::from_str(fresh["content"][0]["text"].as_str().unwrap())
                            .map_err(|e| e.to_string())?;
                    metadata["completed"] = json!(true);
                    metadata["action"] = json!({"name":name,"result":value});
                    metadata["verification"] = json!({"status":"observation_returned","settleMilliseconds":200,"nextStep":"Check the screenshot before deciding the intended result succeeded. Navigation may still be loading."});
                    fresh["content"][0]["text"] = json!(metadata.to_string());
                    return Ok(fresh);
                }
                Err(error) => {
                    return Ok(text_result(
                        json!({"completed":true,"action":value,"observationWarning":error,"nextStep":"The action already ran. Do not repeat it. Obtain a fresh snapshot before another action; if the user took control, stop and wait."}),
                    ));
                }
            }
        }
        Ok(text_result(value))
    }
    fn snapshot(&mut self, args: Value, cancel: &dyn Fn() -> bool) -> Result<Value, String> {
        let mut value = native_call("desktop_snapshot", &args, Duration::from_secs(8), cancel)?;
        let sid = uuid::Uuid::new_v4().to_string();
        value["snapshotId"] = json!(sid);
        let image = value.as_object_mut().and_then(|v| v.remove("image"));
        if args["includeTree"] != false {
            match native_call(
                "tree",
                &json!({"windowId":value["windowId"]}),
                Duration::from_secs(3),
                cancel,
            ) {
                Ok(tree) => value["elements"] = tree,
                Err(e) => value["accessibilityWarning"] = json!(e),
            }
        }
        if cancel() {
            return Err("Desktop operation cancelled".into());
        }
        self.observation = Some((
            sid,
            Instant::now(),
            json!({"windowId":value["windowId"],"bounds":value["bounds"],"imageWidth":value["imageWidth"],"imageHeight":value["imageHeight"]}),
        ));
        let mut result = text_result(value);
        if let Some(image) = image {
            result["content"]
                .as_array_mut()
                .unwrap()
                .push(json!({"type":"image","mimeType":"image/jpeg","data":image}));
        }
        Ok(result)
    }
}

fn native_call(
    name: &str,
    args: &Value,
    timeout: Duration,
    cancel: &dyn Fn() -> bool,
) -> Result<Value, String> {
    if cancel() {
        return Err("Desktop operation cancelled".into());
    }
    let mut command =
        std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .arg("--computer-worker")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let _job = match process::JobGuard::attach_std(&child) {
        Ok(job) => job,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let (tx, rx) = mpsc::sync_channel(1);
    let output = child.stdout.take().ok_or("Missing desktop worker output")?;
    std::thread::spawn(move || {
        let value = frame(&mut BufReader::new(output));
        let _ = tx.send(value);
    });
    let request = json!({"name":name,"arguments":args});
    let written = (|| {
        let mut input = child.stdin.take().ok_or("Missing desktop worker input")?;
        serde_json::to_writer(&mut input, &request).map_err(|e| e.to_string())?;
        input.write_all(b"\n").map_err(|e| e.to_string())
    })();
    let started = Instant::now();
    let result = if let Err(e) = written {
        Err(e)
    } else {
        loop {
            if cancel() {
                break Err("Desktop operation cancelled".into());
            }
            if started.elapsed() > timeout {
                break Err(format!("Desktop {name} timed out; no action was retried"));
            }
            match rx.recv_timeout(Duration::from_millis(15)) {
                Ok(Ok(Some(v))) => {
                    break if let Some(e) = v["error"].as_str() {
                        Err(e.into())
                    } else {
                        Ok(v["result"].clone())
                    }
                }
                Ok(Ok(None)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err("Desktop worker exited without a result".into())
                }
                Ok(Err(e)) => break Err(e),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    result
}

pub fn worker() {
    let result = match limited_frame(&mut BufReader::new(std::io::stdin().lock()), 64 * 1024) {
        Ok(Some(v)) => {
            crate::computer_native::execute(v["name"].as_str().unwrap_or(""), &v["arguments"])
        }
        _ => Err("Invalid desktop worker request".into()),
    };
    let value = match result {
        Ok(result) => json!({"result":result}),
        Err(error) => json!({"error":error}),
    };
    let mut output = std::io::stdout().lock();
    let _ = serde_json::to_writer(&mut output, &value);
    let _ = output.write_all(b"\n");
}

pub fn run() -> Result<(), String> {
    let out: Output = Arc::new(Mutex::new(std::io::stdout()));
    let closed = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(Mutex::new(HashSet::<String>::new()));
    let (tx, rx) = mpsc::sync_channel::<Value>(32);
    let (reader_out, reader_closed, reader_cancelled) =
        (out.clone(), closed.clone(), cancelled.clone());
    std::thread::spawn(move || {
        let mut input = BufReader::new(std::io::stdin().lock());
        loop {
            match limited_frame(&mut input, 64 * 1024) {
                Ok(Some(v)) => {
                    if v["method"] == "notifications/cancelled" {
                        let mut ids = reader_cancelled.lock().unwrap();
                        if ids.len() < 64 {
                            ids.insert(v["params"]["requestId"].to_string());
                        }
                    } else if let Err(mpsc::TrySendError::Full(v)) = tx.try_send(v) {
                        if v.get("id").is_some() {
                            send(
                                &reader_out,
                                error(v["id"].clone(), -32000, "Desktop request queue is full"),
                            );
                        }
                    }
                }
                Err(_) => {
                    send(
                        &reader_out,
                        error(Value::Null, -32700, "Invalid JSON-RPC frame"),
                    );
                    break;
                }
                Ok(None) => break,
            }
        }
        reader_closed.store(true, Ordering::Release);
    });
    let mut engine = Engine::default();
    let mut lease: Option<String> = None;
    while let Ok(v) = rx.recv() {
        if closed.load(Ordering::Acquire) {
            break;
        }
        let Some(id) = v.get("id").cloned() else {
            continue;
        };
        if v["jsonrpc"] != "2.0" || !(id.is_string() || id.as_i64().is_some()) {
            send(&out, error(Value::Null, -32600, "Invalid JSON-RPC request"));
            continue;
        }
        let result = match v["method"].as_str() {
            Some("initialize") => Ok(
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"SuperCode Computer","version":VERSION},"instructions":"SuperCode's bundled desktop engine. Discover windows, focus the intended window, then observe before acting. Use screenshot image pixels and fresh snapshot IDs; verify actions. User approvals remain required according to the Agent's policy. SuperCode arms visible blue desktop/pointer cues and a native stop mechanism before calls. User keyboard/mouse input or Esc immediately stops control. Stop and wait for a new user request after takeover; never refocus to regain control or use shell/PowerShell to bypass desktop tools. Modifying actions return a fresh screenshot and snapshotId. Use that new ID; old IDs expire after actions. A foreground mismatch means control changed: stop instead of retrying focus."}),
            ),
            Some("ping") => Ok(json!({})),
            Some("tools/list") => Ok(json!({"tools":tools()})),
            Some("tools/call") => {
                #[cfg(windows)]
                let gate = {
                    if let Some(name) = v["_supercodeGate"].as_str() {
                        if lease.as_deref() != Some(name) {
                            engine.observation = None;
                            lease = Some(name.into());
                        }
                        // Single execution thread; each disposable worker inherits
                        // this exact gate, even if the host has already started a later turn.
                        std::env::set_var(crate::computer_gate::ENVIRONMENT, name);
                    }
                    std::env::var(crate::computer_gate::ENVIRONMENT)
                        .ok()
                        .and_then(|name| crate::computer_gate::Gate::open(&name).ok())
                };
                let key = id.to_string();
                #[cfg(windows)]
                if v.get("_supercodeGate").is_some() && gate.is_none() {
                    send(
                        &out,
                        json!({"jsonrpc":"2.0","id":id,"result":takeover_result(false)}),
                    );
                    continue;
                }
                let cancel = || {
                    #[cfg(windows)]
                    if gate.as_ref().is_some_and(|gate| gate.stopped()) {
                        return true;
                    }
                    closed.load(Ordering::Acquire) || cancelled.lock().unwrap().contains(&key)
                };
                let result = engine.call(
                    v["params"]["name"].as_str().unwrap_or(""),
                    v["params"].get("arguments").cloned().unwrap_or(json!({})),
                    &cancel,
                );
                cancelled.lock().unwrap().remove(&key);
                Ok(result.unwrap_or_else(|e| {
                    #[cfg(windows)]
                    if gate.as_ref().is_some_and(|gate| gate.stopped()) {
                        return takeover_result(true);
                    }
                    json!({"isError":true,"content":[{"type":"text","text":e}]})
                }))
            }
            _ => Err(error(id.clone(), -32601, "Method not found")),
        };
        send(
            &out,
            match result {
                Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                Err(e) => e,
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn takeover_is_a_control_result_with_no_instruction_to_reconnect_or_retry() {
        let result = takeover_result(false);
        assert_eq!(result["isError"], false);
        let detail: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(detail["code"], "user_takeover");
        assert_eq!(detail["actionMayHaveRun"], false);
        assert_eq!(detail["status"], "stopped");
    }
    #[test]
    fn paint_settling_can_be_cancelled_without_waiting_for_the_timeout() {
        let started = Instant::now();
        assert!(settle(&|| true).is_err());
        assert!(started.elapsed() < Duration::from_millis(100));
    }
    #[test]
    fn fresh_install_and_upgrade_have_builtin_tools_without_dependencies() {
        let builtin = server().unwrap();
        let fresh = with_builtin(vec![], builtin.clone());
        assert_eq!(fresh[0].args, ["--computer-mcp"]);
        let old = ToolServer {
            command: "uvx".into(),
            args: vec!["windows-mcp".into()],
            enabled: false,
            ..builtin.clone()
        };
        let migrated = with_builtin(vec![old], builtin.clone());
        assert!(!migrated[0].enabled);
        assert_eq!(migrated[0].command, builtin.command);
        let custom = ToolServer {
            id: "other".into(),
            kind: "custom".into(),
            ..builtin.clone()
        };
        assert_eq!(with_builtin(vec![custom], builtin).len(), 2);
    }
    #[test]
    fn validates_tool_schemas_and_bounds_before_native_side_effects() {
        assert_eq!(tools().len(), TOOL_COUNT);
        assert!(Engine::arguments(
            "desktop_type",
            &json!({"windowId":"1:abcd","text":"你好 🧡"})
        )
        .is_ok());
        for (name, args) in [
            ("desktop_click", json!({"snapshotId":"x","x":-1,"y":2})),
            ("desktop_type", json!({"windowId":"a","text":"a\u{0}b"})),
            (
                "desktop_scroll",
                json!({"snapshotId":"x","x":0,"y":0,"vertical":11}),
            ),
            (
                "desktop_key",
                json!({"windowId":"x","keys":"A","shell":"x"}),
            ),
        ] {
            assert!(Engine::arguments(name, &args).is_err());
        }
    }
    #[test]
    fn frames_preserve_unicode_and_reject_incomplete_data() {
        assert_eq!(
            frame(&mut std::io::Cursor::new(b"{\"id\":7}\n"))
                .unwrap()
                .unwrap()["id"],
            7
        );
        assert!(frame(&mut std::io::Cursor::new(b"{\"id\":7}")).is_err());
    }
}
