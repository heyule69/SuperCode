//! Opt-in tests against our own local page and isolated application database.
use super::*;
use crate::{process, runtime};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

struct Adapter {
    _child: tokio::process::Child,
    _job: process::JobGuard,
    input: tokio::process::ChildStdin,
    output: BufReader<tokio::process::ChildStdout>,
    id: u64,
}
impl Adapter {
    fn start(app: &AppHandle, agent: &str) -> Result<Self, String> {
        let server = crate::desktop_control::wrap(app, server()?, agent)?;
        let mut command = process::command(
            std::path::Path::new(&server.command),
            &server.args.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let job = process::JobGuard::attach(&child)?;
        Ok(Self {
            input: child.stdin.take().ok_or("No adapter stdin")?,
            output: BufReader::new(child.stdout.take().ok_or("No adapter stdout")?),
            _child: child,
            _job: job,
            id: 0,
        })
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.id += 1;
        let mut bytes = serde_json::to_vec(
            &json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params}),
        )
        .unwrap();
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|e| e.to_string())?;
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(35), self.output.read_line(&mut line))
            .await
            .map_err(|_| "Browser stdio timeout")?
            .map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if value["id"] != self.id || value.get("error").is_some() {
            return Err(format!("Browser protocol failed: {value}"));
        }
        Ok(value["result"].clone())
    }
    async fn tool(&mut self, name: &str, args: Value) -> Result<Value, String> {
        self.request("tools/call", json!({"name":name,"arguments":args}))
            .await
    }
}
fn read(value: &Value) -> Result<Value, String> {
    if value["isError"] == true {
        return Err(format!(
            "Browser tool failed: {}",
            value["content"][0]["text"]
        ));
    }
    serde_json::from_str(
        value["content"][0]["text"]
            .as_str()
            .ok_or("Missing snapshot")?,
    )
    .map_err(|e| e.to_string())
}
fn reference(value: &Value, name: &str) -> Result<String, String> {
    value["elements"]
        .as_array()
        .ok_or("Missing elements")?
        .iter()
        .find(|el| el["name"] == name)
        .and_then(|el| el["ref"].as_str())
        .map(str::to_owned)
        .ok_or(format!("Missing control: {name}"))
}
fn renderer() -> Tab {
    Tab {
        session: "renderer".into(),
        label: "main".into(),
        window: "main".into(),
        visible: Arc::new(AtomicBool::new(true)),
        mount: Mutex::new(String::new()),
        focused: AtomicBool::new(false),
        hwnd: 0,
        scale: AtomicU64::new(1f64.to_bits()),
        rect: Mutex::new((0., 0., 1400., 900.)),
        used: Mutex::new(Instant::now()),
        layout_revision: AtomicU64::new(0),
        revision: AtomicU64::new(0),
        observation: Mutex::new(None),
        operation: tokio::sync::Mutex::new(()),
        status: Mutex::new("idle".into()),
        action: Mutex::new(String::new()),
        location: Mutex::new((String::new(), String::new())),
    }
}
async fn select(app: &AppHandle, session: &str) -> Result<(), String> {
    let target = renderer();
    let start = Instant::now();
    while evaluate(
        app,
        &target,
        "!!document.querySelector('[aria-label=\"打开内置浏览器\"]')".into(),
        None,
        None,
    )
    .await?
        != true
    {
        if start.elapsed() > Duration::from_secs(15) {
            return Err("Application renderer did not load".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    app.emit_to("main", "desktop-navigate", json!({"sessionId":session}))
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    while evaluate(
        app,
        &target,
        "localStorage.getItem('supercode.session')".into(),
        None,
        None,
    )
    .await?
        != session
    {
        if start.elapsed() > Duration::from_secs(8) {
            return Err("Application did not select isolated browser chat".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
async fn show_panel(app: &AppHandle, page: &Tab, shown: bool) -> Result<(), String> {
    let selector = if shown {
        "打开内置浏览器"
    } else {
        "关闭浏览器"
    };
    if evaluate(
        app, &renderer(),
        format!("(()=>{{const button=document.querySelector('[aria-label=\"{selector}\"]');if(!button)return false;button.click();return true;}})()"),
        None, None,
    ).await? != true {
        return Err(format!("Browser panel control missing: {selector}"));
    }
    let start = Instant::now();
    loop {
        let mounted = evaluate(
            app,
            &renderer(),
            "!!document.querySelector('[aria-label=\"内置浏览器\"]')".into(),
            None,
            None,
        )
        .await?
            == true;
        if page.visible.load(Ordering::Acquire) == shown && mounted == shown {
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(5) {
            return Err(format!("Browser panel visibility did not become {shown}"));
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}
struct Fixture(tokio::task::JoinHandle<()>);
impl Drop for Fixture {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn fixture() -> Result<(String, Fixture), String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let url = format!(
        "http://{}/",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    let html="<!doctype html><meta charset=utf-8><meta http-equiv=Content-Security-Policy content=\"require-trusted-types-for 'script'\"><title>SuperCode Browser Test</title><style>body{font:16px sans-serif;padding:24px;background:#f8f9fb;color:#24272b;min-height:1200px}button,input{display:block;margin:18px 0;padding:14px;border:1px solid #bbc5d1;border-radius:8px}button{background:#e7f0fa}h1{font-size:24px}output{display:block}</style><h1>SuperCode Browser Test</h1><p id=status>WAITING</p><button onclick=\"document.querySelector('#status').textContent='CLICKED';document.querySelector('output').textContent=Number(document.querySelector('output').textContent)+1\">SC Browser Button</button><input aria-label=\"SC Browser Input\"><output>0</output>";
    let router = axum::Router::new().route(
        "/",
        axum::routing::get(move || async move { axum::response::Html(html) }),
    );
    Ok((
        url,
        Fixture(tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        })),
    ))
}
pub(super) async fn verify(app: &AppHandle) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let project = state.store.add_project(&state.data_dir)?;
    let main = app.get_window("main").ok_or("Missing main window")?;
    main.set_size(tauri::LogicalSize::new(1500., 900.))
        .map_err(|e| e.to_string())?;
    main.show().map_err(|e| e.to_string())?;
    let (address, _fixture) = fixture().await?;
    let mut reports = vec![];
    for agent in ["codex", "claude", "opencode", "pi"] {
        println!("Browser lifecycle regression: {agent}");
        let session = state.store.create_agent_session(&project.id, None, agent)?;
        let native = format!("browser-regression-{agent}");
        state.store.bind_native(&session.id, &native)?;
        select(app, &session.id).await?;
        authorize_session(app, agent, &session.id);
        runtime::publish_event(
            app,
            json!({"method":"turn/started","params":{"threadId":native,"turn":{"id":"fixture"}}}),
        );
        let mut adapter = Adapter::start(app, agent)?;
        adapter.request("initialize", json!({})).await?;
        if adapter.request("tools/list", json!({})).await?["tools"]
            .as_array()
            .map(Vec::len)
            != Some(6)
        {
            return Err("Builtin browser tool export failed".into());
        }
        let first = adapter
            .tool("browser_navigate", json!({"url":address}))
            .await?;
        let first_snapshot = read(&first)?;
        if !first["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["type"] == "image")
        {
            return Err(format!(
                "Agent browser snapshot did not contain a real screenshot: {}",
                first["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|v| v["type"] == "text")
                    .map(|v| v["text"].as_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        let first = first_snapshot;
        let tab = tab(app, &session.id)?;
        let rect = *tab.rect.lock().unwrap();
        if !tab.visible.load(Ordering::Acquire) || rect.0 < 500. || rect.2 < 300. {
            return Err(format!(
                "Right-side native browser was not presented: {rect:?}"
            ));
        }
        let revision = tab.revision.load(Ordering::Acquire);
        let action = adapter.tool(
            "browser_click",
            json!({"snapshotId":first["snapshotId"],"ref":reference(&first,"SC Browser Button")?}),
        );
        let dismiss = async {
            // Hide the actual rendered panel while the click is in flight.
            tokio::time::sleep(Duration::from_millis(60)).await;
            show_panel(app, &tab, false).await
        };
        let (clicked, dismissed) = tokio::join!(action, dismiss);
        dismissed?;
        let clicked = clicked?;
        if !clicked["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["type"] == "image")
        {
            return Err("Hidden browser did not return a real screenshot".into());
        }
        let clicked = read(&clicked)?;
        if tab.visible.load(Ordering::Acquire)
            || app.get_webview(&tab.label).is_none()
            || !Arc::ptr_eq(&tab, &super::tab(app, &session.id)?)
            || *tab.rect.lock().unwrap() != rect
            || tab.revision.load(Ordering::Acquire) != revision
        {
            return Err("Closing the panel destroyed or resized the active browser page".into());
        }
        let marker = format!("中文-{agent}");
        read(&adapter.tool("browser_fill",json!({"snapshotId":clicked["snapshotId"],"ref":reference(&clicked,"SC Browser Input")?,"text":marker})).await?)?;
        if evaluate(
            app,
            &tab,
            "document.querySelector('input').value".into(),
            None,
            None,
        )
        .await?
            != marker
            || evaluate(
                app,
                &tab,
                "document.querySelector('output').textContent".into(),
                None,
                None,
            )
            .await?
                != "1"
        {
            return Err("Input or single click did not reach the hidden page".into());
        }
        if tab.visible.load(Ordering::Acquire) {
            return Err("A background action reopened the dismissed panel".into());
        }
        show_panel(app, &tab, true).await?;
        if !Arc::ptr_eq(&tab, &super::tab(app, &session.id)?)
            || evaluate(
                app,
                &tab,
                "document.querySelector('input').value".into(),
                None,
                None,
            )
            .await?
                != marker
            || evaluate(
                app,
                &tab,
                "document.querySelector('output').textContent".into(),
                None,
                None,
            )
            .await?
                != "1"
        {
            return Err("Reopening the panel did not preserve its page and input".into());
        }
        let filled = read(&adapter.tool("browser_snapshot", json!({})).await?)?;
        if evaluate(
            app,
            &tab,
            "!!document.querySelector('#__supercode-browser-cue [data-pointer]')".into(),
            None,
            None,
        )
        .await?
            != true
        {
            return Err("Browser operation pointer missing".into());
        }
        let changed = adapter
            .tool(
                "browser_click",
                json!({"snapshotId":first["snapshotId"],"ref":"e1"}),
            )
            .await?;
        if changed["isError"] != true {
            return Err("Old snapshot was accepted".into());
        }
        read(
            &adapter
                .tool(
                    "browser_scroll",
                    json!({"snapshotId":filled["snapshotId"],"deltaY":220}),
                )
                .await?,
        )?;
        if evaluate(app, &tab, "scrollY".into(), None, None)
            .await?
            .as_f64()
            .unwrap_or(0.)
            < 1.
        {
            return Err("Browser scroll did not reach page".into());
        }
        // Exercise the actual React Escape handler, while browser-native focus is tested separately.
        evaluate(
            app,
            &renderer(),
            "window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));true"
                .into(),
            None,
            None,
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        let late = read(&adapter.tool("browser_snapshot", json!({})).await?)?;
        if late["code"] != "user_takeover" || *tab.status.lock().unwrap() != "stopped" {
            return Err("Esc did not revoke the current browser turn".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        if evaluate(
            app,
            &tab,
            "!!document.getElementById('__supercode-browser-cue')".into(),
            None,
            None,
        )
        .await?
            == true
        {
            return Err("Blue cue remained after takeover".into());
        }
        authorize_session(app, agent, &session.id);
        runtime::publish_event(
            app,
            json!({"method":"turn/started","params":{"threadId":native,"turn":{"id":"fixture-next"}}}),
        );
        let (endpoint, token) = state_endpoint(app)?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .map_err(|e| e.to_string())?;
        let id = format!("cancel-before-dispatch-{agent}");
        client
            .post(format!("http://{endpoint}/cancel"))
            .bearer_auth(&token)
            .json(&json!({"agent":agent,"id":id}))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let cancelled = client
            .post(format!("http://{endpoint}/call"))
            .bearer_auth(&token)
            .json(&json!({"agent":agent,"id":id,"name":"browser_snapshot","arguments":{}}))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json::<Value>()
            .await
            .map_err(|e| e.to_string())?;
        if read(&cancelled)?["code"] != "user_takeover" {
            return Err("A cancelled queued call still ran".into());
        }
        show_panel(app, &tab, false).await?;
        let warm = read(&adapter.tool("browser_snapshot", json!({})).await?)?;
        if tab.visible.load(Ordering::Acquire) {
            return Err(
                "The first tool reopened a panel closed while the agent was preparing".into(),
            );
        }
        #[cfg(windows)]
        if agent == "pi" {
            let before = evaluate(app, &tab, "scrollY".into(), None, None).await?;
            let (started, ready) = tokio::sync::oneshot::channel();
            let (done, finished) = tokio::sync::oneshot::channel();
            app.run_on_main_thread(move || {
                let _ = started.send(());
                std::thread::sleep(Duration::from_millis(1000));
                let _ = done.send(());
            })
            .map_err(|e| e.to_string())?;
            ready.await.map_err(|e| e.to_string())?;
            let action = adapter.tool(
                "browser_scroll",
                json!({"snapshotId":warm["snapshotId"],"deltaY":100}),
            );
            let take_over = async {
                tokio::time::sleep(Duration::from_millis(40)).await;
                let start = Instant::now();
                crate::browser_native::verify_escape_callback()?;
                if start.elapsed() > Duration::from_millis(750) {
                    return Err(
                        "Browser Escape callback waited for the blocked UI thread".to_string()
                    );
                }
                Ok(())
            };
            let (result, stopped) = tokio::join!(action, take_over);
            stopped?;
            if read(&result?)?["code"] != "user_takeover" {
                return Err("Queued browser command did not report takeover".into());
            }
            finished.await.map_err(|e| e.to_string())?;
            if evaluate(app, &tab, "scrollY".into(), None, None).await? != before {
                return Err(
                    "A browser action dispatched after Escape while the UI was busy".into(),
                );
            }
        } else {
            crate::browser_native::verify_escape_callback()?;
        }
        if app
            .state::<Browser>()
            .lease
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|l| l.gate.load(Ordering::Acquire))
        {
            return Err("Native Escape callback did not revoke control synchronously".into());
        }
        if read(&adapter.tool("browser_snapshot", json!({})).await?)?["code"] != "user_takeover"
            || *tab.status.lock().unwrap() != "stopped"
        {
            return Err("A hidden browser action was accepted after native Escape".into());
        }
        runtime::publish_event(
            app,
            json!({"method":"turn/completed","params":{"threadId":native,"turn":{"id":"fixture-next","status":"completed"}}}),
        );
        tab.visible.store(false, Ordering::Release);
        if let Some(view) = app.get_webview(&tab.label) {
            let _ = view.close();
        }
        app.state::<Browser>()
            .tabs
            .lock()
            .unwrap()
            .remove(&session.id);
        reports.push(json!({"agent":agent,"stdio":true,"visibleRightPane":true,"closedPanelContinues":true,"closedBeforeFirstTool":true,"samePageOnReopen":true,"hiddenEsc":true,"clickAndUnicode":true,"pointer":true,"scroll":true,"escAndLateCalls":true,"warmNextTurn":true,"nativeEscapeCallback":true}));
    }
    let (endpoint, token) = state_endpoint(app)?;
    let bad = reqwest::Client::new()
        .post(format!("http://{endpoint}/call"))
        .bearer_auth("invalid")
        .json(&json!({"agent":"codex","id":"test","name":"browser_snapshot","arguments":{}}))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let _ = token;
    if bad.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Err("Invalid browser token accepted".into());
    }
    Ok(json!({"fixtureOnly":true,"agents":reports,"invalidTokenRejected":true}))
}
fn state_endpoint(app: &AppHandle) -> Result<(String, String), String> {
    app.state::<Browser>()
        .endpoint
        .lock()
        .unwrap()
        .clone()
        .ok_or("Missing endpoint".into())
}

#[cfg(windows)]
async fn inspect_panels_and_escape(
    app: &AppHandle,
    session: &str,
    agent: &str,
) -> Result<Value, String> {
    let page = tab(app, session)?;
    // Use the rendered app controls so this also detects accidental browser unmounts.
    evaluate(
        app,
        &renderer(),
        "document.querySelector('[aria-label=\"打开工作空间\"]')?.click();true".into(),
        None,
        None,
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    if evaluate(app, &renderer(), "!!document.querySelector('[aria-label=\"工作空间\"]') && !!document.querySelector('[aria-label=\"内置浏览器\"]') && !document.querySelector('[aria-label=\"内置浏览器\"]').classList.contains('context-panel')".into(), None, None).await? != true {
        return Err("Browser and file changes are not separate visible panels".into());
    }
    let original = app
        .get_webview(&page.label)
        .ok_or("Native page missing")?
        .label()
        .to_owned();
    // Adding a child webview must not break commands that target its parent window.
    let context =
        serde_json::to_string(&json!({"sessionId":session,"settingsOpen":false})).unwrap();
    evaluate(app, &renderer(), format!("(async()=>{{await window.__TAURI_INTERNALS__.invoke('update_notification_context',{{context:{context}}});return true}})()"), None, None).await?;
    crate::desktop_lifecycle::show_window(app, &page.window)?;
    for label in ["全屏查看网页", "返回分屏", "收起工作空间", "打开工作空间"]
    {
        let script = format!("document.querySelector('[aria-label=\"{label}\"]')?.click();true");
        evaluate(app, &renderer(), script, None, None).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        if !page.visible.load(Ordering::Acquire) || app.get_webview(&original).is_none() {
            return Err(format!("Panel action {label} discarded the native browser"));
        }
        if label != "收起工作空间" && evaluate(app, &renderer(), "(()=>{const b=document.querySelector('[aria-label=\"内置浏览器\"]')?.getBoundingClientRect(),r=document.querySelector('[aria-label=\"工作空间\"]')?.getBoundingClientRect();return !!b&&!!r&&b.right<=r.left+1})()".into(), None, None).await? != true {
            return Err(format!("Browser covers the file changes after {label}"));
        }
    }
    authorize_session(app, agent, session);
    let native = app
        .state::<AppState>()
        .store
        .session(session)?
        .native_id
        .ok_or("Missing test binding")?;
    runtime::publish_event(
        app,
        json!({"method":"turn/started","params":{"threadId":native,"turn":{"id":"fixture-actual-escape"}}}),
    );
    let mut adapter = Adapter::start(app, agent)?;
    adapter.request("initialize", json!({})).await?;
    let snapshot = read(&adapter.tool("browser_snapshot", json!({})).await?)?;
    if active_tab(app).is_none() || snapshot["snapshotId"].as_str().is_none() {
        return Err("Actual Escape test did not acquire a live browser lease".into());
    }
    let before = evaluate(app, &page, "scrollY".into(), None, None).await?;
    let root = std::env::current_dir()
        .unwrap_or_default()
        .join(".supercode");
    std::fs::write(
        root.join("browser-ui-test.json"),
        serde_json::to_vec(&json!({"state":"ready","sessionId":session,"fixtureOnly":true}))
            .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let start = Instant::now();
    while *page.status.lock().unwrap() != "stopped" {
        if start.elapsed() > Duration::from_secs(180) {
            stop(app, session, false);
            return Err("Waiting for actual Esc input timed out".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let late = read(
        &adapter
            .tool(
                "browser_scroll",
                json!({"snapshotId":snapshot["snapshotId"],"deltaY":300}),
            )
            .await?,
    )?;
    if late["code"] != "user_takeover"
        || evaluate(app, &page, "scrollY".into(), None, None).await? != before
    {
        return Err("A browser action ran after actual Escape input".into());
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    if evaluate(
        app,
        &page,
        "!!document.getElementById('__supercode-browser-cue')".into(),
        None,
        None,
    )
    .await?
        == true
    {
        return Err("Browser cue remained after actual Escape input".into());
    }
    Ok(
        json!({"independentPanels":true,"pagePreserved":true,"actualEscapeInput":true,"lateActionBlocked":true,"cueReleased":true,"fixtureOnly":true}),
    )
}

#[cfg(windows)]
pub(super) async fn model(app: &AppHandle, agent: &str) -> Result<Value, String> {
    if !matches!(agent, "claude" | "codex") {
        return Err("Browser model test supports Claude and Codex".into());
    }
    let path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("supercode.db");
    let text = path.to_string_lossy();
    let source_path =
        std::path::PathBuf::from(format!("\\\\localhost\\{}$\\{}", &text[..1], &text[3..]));
    let source = crate::storage::Store(Mutex::new(
        rusqlite::Connection::open_with_flags(
            source_path,
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
        id: format!("browser-model-regression-{agent}"),
        agent: agent.into(),
        name: "Isolated browser regression".into(),
        config: route.config,
    };
    state.store.import_profiles(&[profile.clone()])?;
    state.store.select_profile(agent, Some(&profile.id))?;
    crate::agents::resolve(app, agent)?;
    let project = state.store.add_project(&state.data_dir)?;
    let session = state
        .store
        .create_agent_session(&project.id, model.clone(), agent)?;
    select(app, &session.id).await?;
    let (address, _fixture) = fixture().await?;
    let marker = format!("中文-SC-{}", uuid::Uuid::new_v4().simple());
    let prompt=format!("Isolated browser regression. Use ONLY supercode_browser browser_* tools to open {address} in SuperCode's right-side browser. Obtain a snapshot, click SC Browser Button exactly once, fill SC Browser Input with the literal text {marker}, inspect a new snapshot and reply with {marker}. Use the current snapshotId and observed ref for each action. Do not control any other page, open another browser, run commands, read files, install anything, or use desktop tools. The page is a local test fixture, not a user website.");
    crate::commands::send_message(
        session.id.clone(),
        prompt,
        model.clone(),
        false,
        Some("full".into()),
        app.clone(),
    )
    .await?;
    let start = Instant::now();
    loop {
        let status = state.store.session(&session.id)?.status;
        if status == "waiting" {
            return Err("模型提出审批或问题，未自动批准".into());
        }
        if matches!(status.as_str(), "failed" | "interrupted") {
            return Err(format!(
                "Real browser model task {status}; session={}",
                session.id
            ));
        }
        if status == "idle" {
            break;
        }
        if start.elapsed() > Duration::from_secs(180) {
            let _ = crate::commands::interrupt_turn(session.id.clone(), app.clone()).await;
            return Err("Real browser model task timed out".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let tab = tab(app, &session.id)?;
    let input = evaluate(
        app,
        &tab,
        "document.querySelector('input')?.value".into(),
        None,
        None,
    )
    .await?;
    let clicks = evaluate(
        app,
        &tab,
        "document.querySelector('output')?.textContent".into(),
        None,
        None,
    )
    .await?;
    let messages = state.store.messages(&session.id, None)?;
    let verified = messages
        .iter()
        .any(|m| m.role == "assistant" && m.text.contains(&marker));
    if input != marker || clicks != "1" || !verified {
        return Err(format!("Real model browser result failed: inputMatch={}, clicks={clicks}, finalMatch={verified}, session={}",input==marker,session.id));
    }
    let calls: Vec<Value> = messages
        .iter()
        .filter(|m| m.kind == "claudeToolCall" || m.kind == "mcpToolCall")
        .map(
            |m| json!({"tool":m.data.get("tool").or(m.data.get("name")),"status":m.data["status"]}),
        )
        .collect();
    if *tab.status.lock().unwrap() != "idle"
        || evaluate(
            app,
            &tab,
            "!!document.getElementById('__supercode-browser-cue')".into(),
            None,
            None,
        )
        .await?
            == true
    {
        return Err("Browser cue remained after real model completion".into());
    }
    let panel_test = if std::env::args().any(|arg| arg == "--browser-ui-test") {
        inspect_panels_and_escape(app, &session.id, agent).await?
    } else {
        json!({"tested":false})
    };
    state.claude.shutdown().await;
    state.runtime.shutdown().await;
    Ok(
        json!({"modelTurn":true,"agent":agent,"visiblePageVerified":true,"singleClickVerified":true,"unicodeInputVerified":true,"finalAnswerVerified":true,"completionReleasedControl":true,"elapsedSeconds":start.elapsed().as_secs(),"calls":calls,"panelTest":panel_test}),
    )
}
