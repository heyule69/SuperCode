//! Isolated native/stdio fixture checks. Does not run or approve a real Agent.
use crate::{client_features::ToolServer, desktop_control, process, runtime, AppState};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const SERVER: &str = r#"
const rl=require('node:readline').createInterface({input:process.stdin});
const send=v=>process.stdout.write(JSON.stringify(v)+'\n');
let pending;
rl.on('line',line=>{
 const v=JSON.parse(line);
 if(v.method==='initialize')send({jsonrpc:'2.0',id:v.id,result:{protocolVersion:'2024-11-05',capabilities:{tools:{}},serverInfo:{name:'desktop-control-fixture',version:'1'}}});
 if(v.method==='tools/list')send({jsonrpc:'2.0',id:v.id,result:{tools:[]}});
 if(v.method==='tools/call'&&v.params.name==='Snapshot'){
  pending=v.id;send({jsonrpc:'2.0',id:'fixture-question',method:'elicitation/create',params:{message:'测试提问 · fixture'}});
 }
 if(v.id==='fixture-question'&&v.result)send({jsonrpc:'2.0',id:pending,result:{content:[{type:'text',text:'管道测试通过 · fixture'}]}});
 if(v.method==='tools/call'&&v.params.name==='Hang'){
  const child=require('node:child_process').spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{windowsHide:true,stdio:'ignore'});
  send({jsonrpc:'2.0',method:'fixture/child',params:{pid:child.pid}});
 }
 // Hang deliberately has no response, for immediate cancellation/disconnect checks.
});
"#;
struct Fixture {
    child: tokio::process::Child,
    _job: process::JobGuard,
    input: tokio::process::ChildStdin,
    output: BufReader<tokio::process::ChildStdout>,
}
impl Fixture {
    async fn tool(&mut self, id: u64, name: &str, arguments: Value) -> Result<Value, String> {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})).await?;
        let v = self.receive_with_timeout(Duration::from_secs(16)).await?;
        if v["id"] != id || v.get("error").is_some() || v["result"]["isError"] == true {
            return Err(format!("{name}: {v}"));
        }
        Ok(v["result"].clone())
    }
    fn start(server: &ToolServer) -> Result<Self, String> {
        let mut command = process::command(
            Path::new(&server.command),
            &server.args.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let job = process::JobGuard::attach(&child)?;
        let input = child.stdin.take().ok_or("测试管道缺少输入")?;
        let output = BufReader::new(child.stdout.take().ok_or("测试管道缺少输出")?);
        Ok(Self {
            child,
            _job: job,
            input,
            output,
        })
    }
    async fn send(&mut self, value: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|e| e.to_string())
    }
    async fn receive(&mut self) -> Result<Value, String> {
        self.receive_with_timeout(Duration::from_secs(8)).await
    }
    async fn receive_with_timeout(&mut self, timeout: Duration) -> Result<Value, String> {
        let mut text = String::new();
        let n = tokio::time::timeout(timeout, self.output.read_line(&mut text))
            .await
            .map_err(|_| "测试管道等待超时")?
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("测试管道已关闭".into());
        }
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
async fn settle() {
    tokio::time::sleep(Duration::from_millis(250)).await;
}
/// Opt-in real model regression, restricted to this program's native fixture.
/// Source credentials are read-only and imported into the isolated encrypted DB.
#[cfg(windows)]
async fn verify_model(app: &AppHandle, agent: &str) -> Result<Value, String> {
    if !matches!(agent, "claude" | "codex") {
        return Err("Model regression supports Claude or Codex".into());
    }
    let path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("supercode.db");
    let path_text = path.to_string_lossy();
    // MSIX redirection can expose a stale Roaming DB to the development host.
    let host_path = if path_text.as_bytes().get(1) == Some(&b':') {
        std::path::PathBuf::from(format!(
            "\\\\localhost\\{}$\\{}",
            &path_text[..1],
            &path_text[3..]
        ))
    } else {
        path
    };
    let source = crate::storage::Store(std::sync::Mutex::new(
        rusqlite::Connection::open_with_flags(
            host_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| e.to_string())?,
    ));
    let state = app.state::<AppState>();
    for prefix in ["agent_install_", "agent_path_"] {
        if let Some(value) = source.setting(&format!("{prefix}{agent}"))? {
            state
                .store
                .set_setting(&format!("{prefix}{agent}"), &value)?;
        }
    }
    if agent == "codex" {
        if let Some(path) = source.codex_path()? {
            state.store.set_setting("codex_path", &path)?;
        }
    }
    let existing = source.sessions()?.into_iter().find(|s| s.agent == agent);
    let route = source.route(agent, existing.as_ref().map(|s| s.id.as_str()))?;
    let model = existing.and_then(|s| s.model).or_else(|| {
        crate::providers::model_ids(&route.config)
            .into_iter()
            .next()
    });
    let profile = crate::ccswitch::Profile {
        id: format!("computer-model-regression-{agent}"),
        agent: agent.into(),
        name: "Isolated computer regression".into(),
        config: route.config,
    };
    state.store.import_profiles(&[profile.clone()])?;
    state.store.select_profile(agent, Some(&profile.id))?;
    crate::agents::resolve(app, agent)?;
    let fixture = crate::computer_smoke::Fixture::start()?;
    fixture.focus();
    settle().await;
    let project = state.store.add_project(&state.data_dir)?;
    let session = state
        .store
        .create_agent_session(&project.id, model.clone(), agent)?;
    let marker = format!("SC-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let prompt = format!("Test only the native test window with windowId {}. Use ONLY the supercode_computer desktop tools. Discover windows, focus this exact window, take a screenshot, click SC Native Button exactly once, click the edit box, and type the literal text {marker}. Inspect the returned screenshot and reply with the text displayed in the edit box. Do not press Enter, control other windows, open apps, execute commands, read files, use shell/PowerShell, or install anything. This is an isolated test fixture, not a user application.", fixture.id());
    crate::commands::send_message(
        session.id.clone(),
        prompt,
        model.clone(),
        false,
        Some("full".into()),
        app.clone(),
    )
    .await?;
    let started = std::time::Instant::now();
    loop {
        let status = state.store.session(&session.id)?.status;
        if status == "waiting" {
            return Err("真实模型测试请求审批或提问，未自动批准".into());
        }
        if matches!(status.as_str(), "failed" | "interrupted") {
            return Err(format!("Real {agent} computer task {status}"));
        }
        if status == "idle" {
            break;
        }
        if started.elapsed() > Duration::from_secs(180) {
            let _ = crate::commands::interrupt_turn(session.id.clone(), app.clone()).await;
            return Err("Real model computer task timed out".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let messages = state.store.messages(&session.id, None)?;
    let answer_verified = messages
        .iter()
        .any(|m| m.role == "assistant" && m.text.contains(&marker));
    let calls: Vec<Value> = messages
        .iter()
        .filter(|m| m.kind == "claudeToolCall" || m.kind == "mcpToolCall")
        .map(
            |m| json!({"tool":m.data.get("tool").or(m.data.get("name")),"status":m.data["status"]}),
        )
        .collect();
    if fixture.clicks() != 1 || fixture.text() != marker || !answer_verified {
        return Err(format!("Real {agent} result not verified: clicks={}, textMatch={}, finalMatch={answer_verified}, isolatedSession={}", fixture.clicks(), fixture.text()==marker,session.id));
    }
    settle().await;
    if desktop_control::active(app) {
        return Err("Cue remained after real model completion".into());
    }
    state.claude.shutdown().await;
    state.runtime.shutdown().await;
    Ok(
        json!({"modelTurn":true,"agent":agent,"model":model,"sessionId":session.id,"nativeClickVerified":true,"nativeTextVerified":true,"finalAnswerVerified":true,"cueClosedAfterCompletion":true,"elapsedSeconds":started.elapsed().as_secs(),"calls":calls}),
    )
}
#[cfg(windows)]
async fn verify_builtin(app: &AppHandle) -> Result<Value, String> {
    let native_fixture = crate::computer_smoke::Fixture::start()?;
    let project = app
        .state::<AppState>()
        .store
        .add_project(&app.state::<AppState>().data_dir)?;
    let mut reports = vec![];
    for agent in ["codex", "claude", "opencode", "pi"] {
        let session =
            app.state::<AppState>()
                .store
                .create_agent_session(&project.id, None, agent)?;
        let native = format!("builtin-regression-{agent}");
        app.state::<AppState>()
            .store
            .bind_native(&session.id, &native)?;
        desktop_control::authorize_session(app, agent, &session.id)?;
        runtime::publish_event(
            app,
            json!({"method":"turn/started","params":{"threadId":native,"turn":{"id":"builtin-regression"}}}),
        );
        native_fixture.focus();
        settle().await;
        let mut pipe = Fixture::start(&desktop_control::wrap(
            app,
            crate::computer::server()?,
            agent,
        )?)?;
        pipe.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}})).await?;
        let init = pipe.receive().await?;
        if init["result"]["serverInfo"]["name"] != "SuperCode Computer" {
            return Err("Wrong built-in MCP server".into());
        }
        let result = pipe.tool(2, "desktop_list_windows", json!({})).await?;
        if !desktop_control::cue_ready(app) {
            return Err(format!(
                "{agent}: blue edges, pointer and global stop hooks are not ready"
            ));
        }
        let list: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap())
            .map_err(|e| e.to_string())?;
        let id = native_fixture.id();
        if !list["windows"]
            .as_array()
            .is_some_and(|v| v.iter().any(|w| w["windowId"] == id))
        {
            return Err("Native fixture missing from window list".into());
        }
        pipe.tool(3, "desktop_focus_window", json!({"windowId":id}))
            .await?;
        let result = pipe
            .tool(4, "desktop_snapshot", json!({"windowId":id}))
            .await?;
        let snapshot: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap())
            .map_err(|e| e.to_string())?;
        use base64::Engine;
        let image = result["content"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["type"] == "image")
            .ok_or("No native screenshot")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(image["data"].as_str().unwrap())
            .map_err(|e| e.to_string())?;
        image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        if !snapshot["elements"]
            .to_string()
            .contains("SC Native Button")
        {
            return Err(format!(
                "Native UIA read failed: {}",
                snapshot.get("accessibilityWarning").unwrap_or(&Value::Null)
            ));
        }
        let before = native_fixture.clicks();
        let pointer_before = native_fixture.pointer();
        let (x, y) = native_fixture.point(native_fixture.button, &snapshot)?;
        let clicked = pipe
            .tool(
                5,
                "desktop_click",
                json!({"snapshotId":snapshot["snapshotId"],"x":x,"y":y}),
            )
            .await?;
        settle().await;
        if native_fixture.clicks() != before + 1 {
            return Err("Native click did not reach the fixture".into());
        }
        let fresh: Value = serde_json::from_str(clicked["content"][0]["text"].as_str().unwrap())
            .map_err(|e| e.to_string())?;
        if fresh["action"]["result"]["input"] != "uia_invoke"
            || native_fixture.pointer() != pointer_before
        {
            return Err(
                "Accessible button click moved the system pointer instead of invoking UIA".into(),
            );
        }
        if fresh["snapshotId"] == snapshot["snapshotId"]
            || fresh["snapshotId"].as_str().is_none()
            || !clicked["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["type"] == "image")
        {
            return Err("Action did not return a fresh observation".into());
        }
        // The next click uses the automatically returned screenshot, without a separate snapshot.
        let snapshot = fresh;
        let (x, y) = native_fixture.point(native_fixture.edit, &snapshot)?;
        pipe.tool(
            7,
            "desktop_click",
            json!({"snapshotId":snapshot["snapshotId"],"x":x,"y":y}),
        )
        .await?;
        pipe.tool(8, "desktop_key", json!({"windowId":id,"keys":"CTRL+A"}))
            .await?;
        pipe.tool(
            9,
            "desktop_type",
            json!({"windowId":id,"text":"SuperCode 测试 🧡"}),
        )
        .await?;
        settle().await;
        if native_fixture.text() != "SuperCode 测试 🧡" {
            return Err(format!("Unicode input mismatch: {}", native_fixture.text()));
        }
        let before_escape = native_fixture.escapes();
        pipe.tool(17, "desktop_key", json!({"windowId":id,"keys":"ESC"}))
            .await?;
        if !desktop_control::active(app)
            || !desktop_control::cue_ready(app)
            || native_fixture.escapes() != before_escape + 1
        {
            return Err("Agent's own Esc incorrectly stopped the control session".into());
        }
        // The consumed old snapshot must fail without another click.
        pipe.send(json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"desktop_click","arguments":{"snapshotId":snapshot["snapshotId"],"x":x,"y":y}}})).await?;
        if pipe.receive().await?["result"]["isError"] != true {
            return Err("Stale snapshot was accepted".into());
        }
        let result = pipe
            .tool(
                11,
                "desktop_snapshot",
                json!({"windowId":id,"includeTree":false}),
            )
            .await?;
        let snapshot: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap())
            .map_err(|e| e.to_string())?;
        let (x, y) = native_fixture.point(native_fixture.edit, &snapshot)?;
        let before = native_fixture.wheels();
        pipe.tool(
            12,
            "desktop_scroll",
            json!({"snapshotId":snapshot["snapshotId"],"x":x,"y":y,"vertical":-1}),
        )
        .await?;
        settle().await;
        if native_fixture.wheels() != before + 1 {
            return Err("Native scroll did not reach fixture".into());
        }
        let result = pipe
            .tool(
                13,
                "desktop_snapshot",
                json!({"windowId":id,"includeTree":false}),
            )
            .await?;
        let snapshot: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap())
            .map_err(|e| e.to_string())?;
        let (x, y) = native_fixture.point(native_fixture.edit, &snapshot)?;
        let before = native_fixture.drags();
        pipe.tool(
            14,
            "desktop_drag",
            json!({"snapshotId":snapshot["snapshotId"],"fromX":x-180,"fromY":y,"toX":x-60,"toY":y}),
        )
        .await?;
        settle().await;
        if native_fixture.drags() <= before {
            return Err("Native drag did not reach fixture".into());
        }
        pipe.send(json!({"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"desktop_snapshot","arguments":{"windowId":id}}})).await?;
        pipe.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":15,"reason":"fixture cancellation"}})).await?;
        let cancelled = pipe.receive_with_timeout(Duration::from_secs(5)).await?;
        if cancelled["id"] != 15
            || cancelled["result"]["isError"] != true
            || !cancelled["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .contains("cancelled")
        {
            return Err("Native worker cancellation failed".into());
        }
        pipe.send(json!({"jsonrpc":"2.0","id":16,"method":"ping","params":{}}))
            .await?;
        if pipe.receive().await?["id"] != 16 {
            return Err("MCP did not recover after cancellation".into());
        }
        if !desktop_control::active(app) {
            return Err("Desktop cue vanished between calls".into());
        }
        runtime::publish_event(
            app,
            json!({"method":"turn/completed","params":{"threadId":native,"turn":{"id":"builtin-regression","status":"completed"}}}),
        );
        settle().await;
        if desktop_control::authorize_session(app, agent, &session.id)? {
            return Err("Normal next turn unnecessarily restarted the native transport".into());
        }
        runtime::publish_event(
            app,
            json!({"method":"turn/started","params":{"threadId":native,"turn":{"id":"builtin-second"}}}),
        );
        pipe.tool(22, "desktop_list_windows", json!({})).await?;
        if !desktop_control::cue_ready(app) {
            return Err("Warm transport did not receive the new turn's control lease".into());
        }
        pipe.send(json!({"jsonrpc":"2.0","id":23,"method":"tools/call","params":{"name":"desktop_click","arguments":{"snapshotId":snapshot["snapshotId"],"x":x,"y":y}}})).await?;
        if pipe.receive().await?["result"]["isError"] != true {
            return Err("Previous turn's screenshot was accepted after a new control lease".into());
        }
        pipe.tool(18, "desktop_key", json!({"windowId":id,"keys":"CTRL+A"}))
            .await?;
        let previous_length = native_fixture.text_length();
        pipe.send(json!({"jsonrpc":"2.0","id":19,"method":"tools/call","params":{"name":"desktop_type","arguments":{"windowId":id,"text":"x".repeat(4096)}}})).await?;
        let typing_started = std::time::Instant::now();
        while native_fixture.text_length() == previous_length
            && typing_started.elapsed() < Duration::from_secs(3)
        {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let before_stop = native_fixture.text_length();
        if before_stop == previous_length || before_stop >= 4096 {
            return Err(format!(
                "Could not observe in-flight native typing: {before_stop}"
            ));
        }
        let input_counts = crate::desktop_glow::input_counts();
        let stopped_at = std::time::Instant::now();
        native_fixture.user_input(agent == "claude", true)?;
        let cancelled = pipe.receive_with_timeout(Duration::from_secs(3)).await?;
        let takeover: Value = serde_json::from_str(
            cancelled["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or("{}"),
        )
        .map_err(|e| e.to_string())?;
        if cancelled["id"] != 19 || takeover["code"] != "user_takeover" {
            return Err(format!(
                "Expected takeover result, got {cancelled}; input counts {input_counts:?}"
            ));
        }
        pipe.send(json!({"jsonrpc":"2.0","id":20,"method":"ping","params":{}}))
            .await?;
        if pipe.receive().await?["id"] != 20
            || pipe.child.try_wait().map_err(|e| e.to_string())?.is_some()
        {
            return Err("Takeover incorrectly disconnected the native transport".into());
        }
        pipe.send(json!({"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"desktop_focus_window","arguments":{"windowId":id}}})).await?;
        let rejected = pipe.receive().await?;
        let stopped: Value = serde_json::from_str(
            rejected["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or("{}"),
        )
        .map_err(|e| e.to_string())?;
        if stopped["code"] != "user_takeover" || stopped["actionMayHaveRun"] != false {
            return Err("Late native action was allowed after takeover".into());
        }
        let stop_ms = stopped_at.elapsed().as_millis();
        settle().await;
        let stopped_length = native_fixture.text_length();
        settle().await;
        if native_fixture.text_length() != stopped_length
            || stopped_length >= 4096
            || desktop_control::cue_ready(app)
        {
            return Err("Native typing continued after user takeover".into());
        }
        runtime::publish_event(
            app,
            json!({"method":"turn/completed","params":{"threadId":native,"turn":{"id":"builtin-second","status":"completed"}}}),
        );
        settle().await;
        if desktop_control::active(app) {
            return Err("Desktop cue remained after completion".into());
        }
        reports.push(json!({"agent":agent,"nativeWindowRead":true,"screenshotDecoded":true,"accessibilityRead":true,"clickVerified":true,"unicodeInputVerified":true,"scrollVerified":true,"dragVerified":true,"cancelledWorkerRecovered":true,"staleSnapshotRejected":true,"persistentCue":true,"nativeCueReady":true,"freshObservationAfterAction":true,"nativeTypingStopped":true,"transportAliveAfterTakeover":true,"lateNativeActionBlocked":true,"stopMilliseconds":stop_ms,"typedBeforeStop":before_stop,"typedAfterStop":stopped_length}));
    }
    let mut discovery = vec![];
    if std::env::args().any(|a| a == "--check-agent-discovery") {
        let source = crate::storage::Store(std::sync::Mutex::new(
            rusqlite::Connection::open_with_flags(
                app.path()
                    .app_data_dir()
                    .map_err(|e| e.to_string())?
                    .join("supercode.db"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .map_err(|e| e.to_string())?,
        ));
        for agent in ["codex", "claude"] {
            for prefix in ["agent_install_", "agent_path_"] {
                if let Some(value) = source.setting(&format!("{prefix}{agent}"))? {
                    app.state::<AppState>()
                        .store
                        .set_setting(&format!("{prefix}{agent}"), &value)?;
                }
            }
            if agent == "codex" {
                if let Some(path) = source.codex_path()? {
                    app.state::<AppState>()
                        .store
                        .set_setting("codex_path", &path)?;
                }
            }
            discovery.push(crate::automation::verify_agent(app, agent).await?);
        }
    }
    Ok(
        json!({"builtInEngine":true,"modelTurn":false,"agents":reports,"nativeAgentDiscovery":discovery}),
    )
}
async fn verify_real(app: &AppHandle, program: String) -> Result<Value, String> {
    let main = app.get_webview_window("main").ok_or("测试主窗口不存在")?;
    main.set_title("SuperCode MCP Regression Test")
        .map_err(|e| e.to_string())?;
    let original = ToolServer {
        id: "computer".into(),
        name: "Real MCP regression test".into(),
        kind: "computer".into(),
        command: program,
        args: vec!["serve".into()],
        url: None,
        enabled: true,
    };
    let project = app
        .state::<AppState>()
        .store
        .add_project(&app.state::<AppState>().data_dir)?;
    let mut reports = vec![];
    for agent in ["codex", "claude", "opencode", "pi"] {
        let session =
            app.state::<AppState>()
                .store
                .create_agent_session(&project.id, None, agent)?;
        let native = format!("real-mcp-regression-{agent}");
        app.state::<AppState>()
            .store
            .bind_native(&session.id, &native)?;
        desktop_control::authorize_session(app, agent, &session.id)?;
        runtime::publish_event(
            app,
            json!({"method":"turn/started","params":{"threadId":native,"turn":{"id":"regression"}}}),
        );
        main.show().map_err(|e| e.to_string())?;
        main.unminimize().map_err(|e| e.to_string())?;
        main.set_focus().map_err(|e| e.to_string())?;
        settle().await;
        let mut fixture = Fixture::start(&desktop_control::wrap(app, original.clone(), agent)?)?;
        fixture.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","clientInfo":{"name":"supercode-regression","version":"1"},"capabilities":{}}})).await?;
        fixture.receive().await?;
        fixture
            .send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await?;
        fixture
            .send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
            .await?;
        let tools = fixture.receive().await?;
        if agent == "codex" {
            tokio::time::sleep(Duration::from_secs(7)).await;
        }
        for (id, name, arguments) in [
            (3, "Snapshot", json!({"use_vision":false,"use_dom":false})),
            (
                4,
                "App",
                json!({"mode":"switch","name":"SuperCode MCP Regression Test"}),
            ),
            (
                5,
                "Snapshot",
                json!({"use_vision":true,"use_dom":false,"use_annotation":false}),
            ),
        ] {
            fixture.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})).await?;
            let result = loop {
                let value = fixture
                    .receive_with_timeout(Duration::from_secs(30))
                    .await?;
                if value["id"] == id {
                    break value;
                }
            };
            if result.get("error").is_some() || result["result"]["isError"] == true {
                return Err(format!("{agent} / {name}: {}", result["error"]));
            }
            if name == "App"
                && !result["result"]
                    .to_string()
                    .to_lowercase()
                    .contains("switched")
            {
                return Err(format!("{agent} 测试窗口未切换成功"));
            }
            reports.push(json!({"agent":agent,"tool":name,"responseBytes":serde_json::to_vec(&result).map_err(|e|e.to_string())?.len(),"ok":true}));
        }
        runtime::publish_event(
            app,
            json!({"method":"turn/completed","params":{"threadId":native,"turn":{"id":"regression","status":"completed"}}}),
        );
        settle().await;
        drop(fixture);
        if tools["result"]["tools"].as_array().is_none() {
            return Err("MCP 工具列表无效".into());
        }
    }
    Ok(json!({"realMcp":true,"modelTurn":false,"calls":reports}))
}
async fn verify(app: &AppHandle) -> Result<Value, String> {
    #[cfg(windows)]
    let native_fixture = crate::computer_smoke::Fixture::start()?;
    let main = app.get_webview_window("main").ok_or("测试主窗口不存在")?;
    let node = process::find_program("node").ok_or("测试需要本机 Node")?;
    let original = ToolServer {
        id: "computer".into(),
        name: "Test fixture".into(),
        kind: "computer".into(),
        command: node.to_string_lossy().into_owned(),
        args: vec!["-e".into(), SERVER.into()],
        url: None,
        enabled: true,
    };
    let project = app
        .state::<AppState>()
        .store
        .add_project(&app.state::<AppState>().data_dir)?;
    let mut reports = vec![];
    for agent in ["codex", "claude", "opencode", "pi"] {
        let session =
            app.state::<AppState>()
                .store
                .create_agent_session(&project.id, None, agent)?;
        let native = format!("desktop-control-fixture-{agent}");
        app.state::<AppState>()
            .store
            .bind_native(&session.id, &native)?;
        app.state::<AppState>()
            .store
            .set_status(&session.id, "running", Some("fixture-turn"))?;
        desktop_control::authorize_session(app, agent, &session.id)?;
        runtime::publish_event(
            app,
            json!({"method":"turn/started","params":{"threadId":native,"turn":{"id":"fixture-turn"}}}),
        );
        main.show().map_err(|e| e.to_string())?;
        main.unminimize().map_err(|e| e.to_string())?;
        main.set_focus().map_err(|e| e.to_string())?;
        settle().await;
        let server = desktop_control::wrap(app, original.clone(), agent)?;
        if server.args.first().map(String::as_str) != Some("--computer-mcp-bridge") {
            return Err("没有接入电脑提示桥接".into());
        }
        // Incorrect local tokens cannot create an activity or move the window.
        let mut unauthenticated = tokio::net::TcpStream::connect(&server.args[1])
            .await
            .map_err(|e| e.to_string())?;
        unauthenticated
            .write_all(b"{\"token\":\"wrong\",\"agent\":\"codex\"}\n")
            .await
            .map_err(|e| e.to_string())?;
        let mut rejected = BufReader::new(unauthenticated);
        let mut answer = String::new();
        let n = tokio::time::timeout(Duration::from_secs(3), rejected.read_line(&mut answer))
            .await
            .map_err(|_| "无效认证没有及时断开")?
            .map_err(|e| e.to_string())?;
        if n != 0 {
            return Err("错误 token 未被拒绝".into());
        }
        let mut fixture = Fixture::start(&server)?;
        fixture
            .send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;
        if fixture.receive().await?["id"] != 1 {
            return Err("初始化透传失败".into());
        }
        fixture
            .send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
            .await?;
        fixture.receive().await?;
        if main.is_minimized().map_err(|e| e.to_string())? || desktop_control::active(app) {
            return Err("初始化或工具列表错误触发了桌面切换".into());
        }
        fixture.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"Snapshot","arguments":{}}})).await?;
        let question = fixture.receive().await?;
        settle().await;
        if !main.is_minimized().map_err(|e| e.to_string())? || !desktop_control::active(app) {
            return Err(format!("{agent} 工具执行前没有让出桌面或点亮提示"));
        }
        if question["method"] != "elicitation/create"
            || question["params"]["message"] != "测试提问 · fixture"
        {
            return Err("原生 MCP 提问没有完整透传".into());
        }
        runtime::publish_event(
            app,
            json!({"id":"fixture-agent-question","method":"item/tool/requestUserInput","params":{"threadId":native,"turnId":"fixture-turn","questions":[]}}),
        );
        settle().await;
        if main.is_minimized().map_err(|e| e.to_string())? || desktop_control::active(app) {
            return Err("原生 Agent 提问没有切回聊天并暂停光效".into());
        }
        // Test fixture response, never an Agent approval.
        fixture.send(json!({"jsonrpc":"2.0","id":"fixture-question","result":{"action":"accept","content":{}}})).await?;
        let result = fixture.receive().await?;
        if result["id"] != 3 || result["result"]["content"][0]["text"] != "管道测试通过 · fixture"
        {
            return Err("工具结果没有完整透传".into());
        }
        fixture.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"Hang","arguments":{}}})).await?;
        let descendant = fixture.receive().await?["params"]["pid"]
            .as_u64()
            .ok_or("测试子进程没有 PID")? as u32;
        settle().await;
        if !desktop_control::active(app) || !main.is_minimized().map_err(|e| e.to_string())? {
            return Err("回答后的下一次工具调用没有再次切到桌面".into());
        }
        if !desktop_control::cue_ready(app) {
            return Err("Global stop hooks and native blue cue are not ready".into());
        }
        #[cfg(windows)]
        native_fixture.focus();
        settle().await;
        // Exercise actual Windows hooks. For Codex, also keep the UI busy while Esc cancels the tool.
        let mut ui_done = None;
        if agent == "codex" {
            let (started, ready) = tokio::sync::oneshot::channel();
            let (done, finished) = tokio::sync::oneshot::channel();
            app.run_on_main_thread(move || {
                let _ = started.send(());
                std::thread::sleep(Duration::from_millis(1200));
                let _ = done.send(());
            })
            .map_err(|e| e.to_string())?;
            ready.await.map_err(|e| e.to_string())?;
            ui_done = Some(finished);
        }
        let stopped_at = std::time::Instant::now();
        #[cfg(windows)]
        native_fixture.user_input(agent == "claude", true)?;
        #[cfg(not(windows))]
        desktop_control::takeover_reason(app, true, "fixture_escape");
        let status = tokio::time::timeout(Duration::from_secs(3), fixture.child.wait())
            .await
            .map_err(|_| "停止后电脑 MCP 桥接仍未退出")?
            .map_err(|e| e.to_string())?;
        let stop_ms = stopped_at.elapsed().as_millis();
        if ui_done.is_some() && stop_ms >= 900 {
            return Err(format!("Esc waited for the UI thread: {stop_ms} ms"));
        }
        if desktop_control::cue_ready(app) {
            return Err("Input gate stayed armed after user takeover".into());
        }
        if let Some(done) = ui_done {
            done.await.map_err(|e| e.to_string())?;
        }
        settle().await;
        let manual_input = agent == "claude";
        if status.success()
            || desktop_control::active(app)
            || (!manual_input && main.is_minimized().map_err(|e| e.to_string())?)
        {
            return Err("停止操作没有退出工具、移除光效和恢复聊天".into());
        }
        if manual_input && !main.is_minimized().map_err(|e| e.to_string())? {
            return Err("Mouse/keyboard takeover stole focus back to chat".into());
        }
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::{
                Foundation::CloseHandle,
                System::Threading::{
                    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                },
            };
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, descendant);
            if !process.is_null() {
                let mut code = 0;
                let running = GetExitCodeProcess(process, &mut code) != 0 && code == 259;
                CloseHandle(process);
                if running {
                    return Err("停止操作后仍有 MCP 派生进程残留".into());
                }
            }
        }
        // Native automatic retries cannot authorize another takeover.
        runtime::publish_event(
            app,
            json!({"method":"turn/started","params":{"threadId":native,"turn":{"id":"unsolicited-retry"}}}),
        );
        // A reconnect/retry from the interrupted turn must not take control back.
        let mut late = Fixture::start(&server)?;
        late.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;
        late.receive().await?;
        late.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"Hang","arguments":{}}})).await?;
        let rejected = late.receive().await?;
        if rejected["result"]["isError"] != true
            || desktop_control::active(app)
            || desktop_control::cue_ready(app)
            || (!manual_input && main.is_minimized().map_err(|e| e.to_string())?)
        {
            return Err("Late call reclaimed control after takeover".into());
        }
        drop(late);
        runtime::publish_event(
            app,
            json!({"method":"turn/completed","params":{"threadId":native,"turn":{"id":"fixture-turn","status":"interrupted"}}}),
        );
        reports.push(json!({"agent":agent,"initializationQuiet":true,"handoffBeforeCall":true,"questionsPassedThrough":true,"attentionRestoresChat":true,"hungToolStopped":true,"nativeCueReady":true,"stopInput":if agent=="claude" {"mouseMove"} else {"Esc"},"stopMilliseconds":stop_ms,"lateCallsBlocked":true,"uiBusyStop":agent=="codex"}));
    }
    Ok(json!({"fixtureOnly":true,"agents":reports}))
}
pub async fn run(app: AppHandle) {
    let args: Vec<String> = std::env::args().collect();
    let model = args
        .iter()
        .position(|arg| arg == "--model-computer")
        .and_then(|i| args.get(i + 1));
    let real = args
        .iter()
        .position(|arg| arg == "--real-computer-mcp")
        .and_then(|index| args.get(index + 1))
        .cloned();
    #[cfg(windows)]
    let builtin = args.iter().any(|arg| arg == "--builtin-computer");
    #[cfg(not(windows))]
    let builtin = false;
    let result = if let Some(agent) = model {
        #[cfg(windows)]
        {
            verify_model(&app, agent).await
        }
        #[cfg(not(windows))]
        {
            let _ = agent;
            Err("Windows only".into())
        }
    } else if builtin {
        #[cfg(windows)]
        {
            verify_builtin(&app).await
        }
        #[cfg(not(windows))]
        {
            Err("Windows only".into())
        }
    } else if let Some(program) = real {
        verify_real(&app, program).await
    } else {
        verify(&app).await
    };
    let report = match result {
        Ok(result) => json!({"ok":true,"result":result}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    let root = std::env::current_dir()
        .unwrap_or_default()
        .join(".supercode");
    let _ = std::fs::create_dir_all(&root);
    let _ = std::fs::write(
        root.join("desktop-control-smoke-report.json"),
        serde_json::to_vec_pretty(&report).unwrap_or_default(),
    );
    println!("{report}");
    app.exit(if report["ok"] == true { 0 } else { 1 });
}
