//! Explicit developer check: isolated SuperCode DB, real native CLIs, two minimal model turns per adapter.
use crate::{agents, commands, AppState};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
async fn idle(app: &AppHandle, id: &str) -> Result<(), String> {
    let start = Instant::now();
    loop {
        let s = app.state::<AppState>().store.session(id)?;
        if s.status == "waiting" {
            return Err("测试遇到审批或提问，未自动批准".into());
        }
        if s.status == "failed" {
            let texts = app
                .state::<AppState>()
                .store
                .messages(id, None)?
                .into_iter()
                .filter(|m| m.role == "system")
                .map(|m| m.text)
                .collect::<Vec<_>>();
            return Err(texts.join("\n"));
        }
        if s.status == "idle" {
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(180) {
            let _ = commands::interrupt_turn(id.into(), app.clone()).await;
            return Err("测试等待输出超时".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
async fn verify(app: &AppHandle) -> Result<Value, String> {
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
    let state = app.state::<AppState>();
    for id in agents::IDS {
        for prefix in ["agent_install_", "agent_path_"] {
            if let Some(v) = source.setting(&format!("{prefix}{id}"))? {
                state.store.set_setting(&format!("{prefix}{id}"), &v)?;
            }
        }
    }
    if std::env::args().any(|arg| arg == "--provider-accounts-test") {
        if let Some(path) = source.codex_path()? {
            state.store.set_setting("codex_path", &path)?;
        }
        let accounts = crate::accounts::list_provider_accounts(Some(true), app.clone()).await?;
        if accounts.iter().any(|account| account["error"].is_string()) {
            return Err("真实官方登录检测失败".into());
        }
        let mut catalog_guards = vec![];
        for account in &accounts {
            let agent = account["agent"].as_str().ok_or("账号 Agent 缺失")?;
            if account["loggedIn"] == false {
                state.store.use_official(agent)?;
                let catalog = commands::list_models(Some(agent.into()), None, app.clone()).await?;
                if catalog["data"]
                    .as_array()
                    .is_none_or(|models| !models.is_empty())
                    || catalog["source"]["available"] != false
                {
                    return Err("没有官方登录时仍返回官方模型".into());
                }
                catalog_guards.push(json!({"agent":agent,"emptyOfficialCatalog":true}));
            }
        }
        let main_runtime_started = state.runtime.client.lock().await.is_some();
        if main_runtime_started {
            return Err("账号检测启动了主任务连接".into());
        }
        return Ok(
            json!({"accounts":accounts,"catalogGuards":catalog_guards,"isolatedData":state.data_dir,"mainRuntimeStarted":main_runtime_started}),
        );
    }
    // Actual install/test when missing. Existing local executables and accounts stay untouched.
    if std::env::args().any(|arg| arg == "--agent-updates-test") {
        if let Some(path) = source.codex_path()? {
            state.store.set_setting("codex_path", &path)?;
        }
        return verify_updates(app).await;
    }
    let cache = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join(".supercode/agents-test-installations.json");
    let mut cached: std::collections::BTreeMap<String, String> = std::fs::read(&cache)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    for (key, value) in &cached {
        if state.store.setting(key)?.is_none() {
            state.store.set_setting(key, value)?;
        }
    }
    for id in ["opencode", "pi"] {
        if agents::resolve(app, id).is_err() {
            agents::install_agent(id.into(), None, None, app.clone()).await?;
        }
        agents::test_agent(id.into(), app.clone()).await?;
        if let Some(value) = state.store.setting(&format!("agent_install_{id}"))? {
            cached.insert(format!("agent_install_{id}"), value);
        }
        std::fs::write(
            &cache,
            serde_json::to_vec(&cached).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    if std::env::args().any(|v| v == "--install-all-agents") {
        for id in agents::IDS {
            agents::install_agent(id.into(), Some(true), None, app.clone()).await?;
        }
    }
    let selected = source
        .active_profile("claude")?
        .or_else(|| {
            source
                .profiles()
                .ok()?
                .into_iter()
                .find(|p| p.agent == "claude" && crate::providers::key(&p.config).is_some())
        })
        .ok_or("集成测试需要已配置的 Claude 兼容 API 连接")?;
    let mut results = vec![];
    for agent in ["opencode", "pi"] {
        let mut profile = selected.clone();
        profile.id = format!("smoke-{agent}");
        profile.agent = agent.into();
        if !profile.config["baseUrl"].is_string() {
            profile.config["baseUrl"] = profile.config["env"]["ANTHROPIC_BASE_URL"].clone();
            profile.config["apiKey"] = json!(crate::providers::key(&selected.config));
            profile.config["protocol"] = "anthropic".into();
        }
        state.store.import_profiles(&[profile.clone()])?;
        state.store.select_profile(agent, Some(&profile.id))?;
        let model = crate::providers::model_ids(&profile.config)
            .into_iter()
            .next()
            .ok_or("测试连接无模型")?;
        let dir = state.data_dir.join(format!("project {agent} 测试"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let project = state.store.add_project(&dir)?;
        let session = state
            .store
            .create_agent_session(&project.id, Some(model.clone()), agent)?;
        if std::env::args().any(|arg| arg == "--agents-mcp-test") {
            let server = serde_json::from_str::<Vec<crate::client_features::ToolServer>>(
                &source.setting("tool_servers")?.unwrap_or("[]".into()),
            )
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|s| s.id == "browser" && s.enabled)
            .ok_or("请先安装并启用浏览器自动化")?;
            state.store.set_setting(
                "tool_servers",
                &serde_json::to_string(&vec![server]).map_err(|e| e.to_string())?,
            )?;
            let token = format!("MCP-{}", uuid::Uuid::new_v4().simple());
            let html = format!("<!doctype html><meta charset=utf-8><title>SuperCode MCP test</title><p>{token}</p>");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .map_err(|e| e.to_string())?;
            let address = listener.local_addr().map_err(|e| e.to_string())?;
            let router = axum::Router::new().route(
                "/",
                axum::routing::get(move || {
                    let html = html.clone();
                    async move { axum::response::Html(html) }
                }),
            );
            let page = tokio::spawn(async move {
                let _ = axum::serve(listener, router).await;
            });
            let sent=commands::send_message(session.id.clone(),format!("Use the browser_navigate tool from the supercode_browser MCP server to open http://{address}/. Discover the MCP tool with tool_search or codemode if necessary. Reply with the exact marker displayed on that page. Do not read files, run shell commands or visit any other URL."),Some(model.clone()),true,Some("full".into()),app.clone()).await;
            let result = match sent {
                Ok(_) => idle(app, &session.id).await,
                Err(e) => Err(e),
            };
            page.abort();
            result?;
            let messages = state.store.messages(&session.id, None)?;
            if !messages
                .iter()
                .any(|m| m.role == "assistant" && m.text.contains(&token))
                || !messages.iter().any(|m| {
                    m.kind == "claudeToolCall"
                        && m.data["status"] == "completed"
                        && (m.data["tool"]
                            .as_str()
                            .is_some_and(|s| s.contains("browser") || s == "codemode"))
                })
            {
                return Err(format!("{agent} 未通过真实浏览器 MCP 调用"));
            }
            results.push(
                json!({"agent":agent,"realModel":true,"realBrowserMcp":true,"localFixture":true}),
            );
            continue;
        }
        let token = format!("SC-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        commands::send_message(session.id.clone(),format!("Remember this test token: {token}. Reply with that token only. Do not read files or use tools."),Some(model.clone()),true,Some("read".into()),app.clone()).await?;
        idle(app, &session.id).await?;
        let first = state.store.messages(&session.id, None)?;
        if !first
            .iter()
            .any(|m| m.role == "assistant" && m.text.contains(&token))
        {
            return Err(format!("{agent} 没有返回测试标识"));
        }
        let native = state.store.session(&session.id)?.native_id;
        commands::send_message(session.id.clone(),"What was the token in the previous message? Reply with that token only. Do not use tools.".into(),Some(model.clone()),true,Some("read".into()),app.clone()).await?;
        idle(app, &session.id).await?;
        let second = state.store.messages(&session.id, None)?;
        if !second
            .iter()
            .skip(first.len())
            .any(|m| m.role == "assistant" && m.text.contains(&token))
        {
            return Err(format!("{agent} 会话恢复未保留上下文"));
        }
        let resume = state.store.session(&session.id)?.native_id == native;
        std::fs::write(dir.join("marker.txt"), token.as_bytes()).map_err(|e| e.to_string())?;
        commands::send_message(session.id.clone(),"Read marker.txt using your read tool, then reply with its exact content. Do not read other files or use other tools.".into(),Some(model.clone()),true,Some("read".into()),app.clone()).await?;
        idle(app, &session.id).await?;
        let tools = state.store.messages(&session.id, None)?;
        if !tools.iter().skip(second.len()).any(|m| {
            m.kind == "claudeToolCall"
                && m.data["tool"] == "Read"
                && m.data["status"] == "completed"
        }) {
            return Err(format!("{agent} 没有实际完成读取工具"));
        }
        commands::send_message(session.id.clone(),"Use the write tool to create approval-sentinel.txt containing TEST. Do not use bash or other tools.".into(),Some(model.clone()),false,Some("ask".into()),app.clone()).await?;
        let start = Instant::now();
        loop {
            let s = state.store.session(&session.id)?;
            if s.status == "waiting" {
                break;
            }
            if !matches!(s.status.as_str(), "starting" | "running")
                || start.elapsed() > Duration::from_secs(120)
            {
                return Err(format!("{agent} 未发出工具审批"));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        if dir.join("approval-sentinel.txt").exists()
            || commands::pending_requests(app.clone()).await?.is_empty()
        {
            return Err(format!("{agent} 审批没有正确阻止工具"));
        }
        commands::interrupt_turn(session.id.clone(), app.clone()).await?;
        let start = Instant::now();
        while matches!(
            state.store.session(&session.id)?.status.as_str(),
            "starting" | "running" | "waiting"
        ) {
            if start.elapsed() > Duration::from_secs(20) {
                return Err(format!("{agent} 停止超时"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if dir.join("approval-sentinel.txt").exists()
            || !commands::pending_requests(app.clone()).await?.is_empty()
        {
            return Err(format!("{agent} 停止后仍有审批或文件写入"));
        }
        // Summary uses the same real CLI on a fresh session, and keeps history on failure.
        let compacted =
            crate::session_config::compact_session_context(session.id.clone(), app.clone()).await?;
        if compacted.native_id.is_some()
            || !state
                .store
                .history_context(&session.id)?
                .is_some_and(|s| s.contains(&token))
        {
            return Err(format!("{agent} 压缩未保留标识"));
        }
        results.push(json!({"agent":agent,"model":model,"realCli":true,"realModel":true,"resume":resume,"realReadTool":true,"approvalForwarded":true,"stoppedWithoutApproving":true,"contextSummary":true,"messages":tools.len(),"usageRecords":state.store.usage_records(Some(&session.id))?.len()}));
    }
    Ok(json!({"adapters":results,"agents":agents::list_agents(app.clone()).await?}))
}
async fn verify_updates(app: &AppHandle) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let latest = crate::agent_versions::check_agent_updates(Some(true), app.clone()).await?;
    let mut reports = vec![];
    for release in latest {
        let id = release.id;
        if let Some(only) =
            std::env::args().find_map(|arg| arg.strip_prefix("--updates-agent=").map(str::to_owned))
        {
            if id != only {
                continue;
            }
        }
        if let Some(error) = release.error {
            return Err(format!("{id}：{error}"));
        }
        let version = release.latest_version.ok_or("最新版本缺失")?;
        let previous = agents::resolve(app, &id).ok();
        let missing = std::env::args().any(|arg| arg == format!("--missing-agent={id}"));
        if missing {
            if agents::local(&id).is_some() {
                return Err("未安装测试需要无本机自动检测程序的 Agent".into());
            }
            state
                .store
                .0
                .lock()
                .map_err(|e| e.to_string())?
                .execute(
                    "DELETE FROM settings WHERE key IN (?1,?2)",
                    rusqlite::params![format!("agent_path_{id}"), format!("agent_install_{id}")],
                )
                .map_err(|e| e.to_string())?;
            if agents::resolve(app, &id).is_ok() {
                return Err("隔离测试未模拟未安装状态".into());
            }
        }
        let old_pid = if id == "codex" && previous.is_some() {
            Some(state.runtime.get(app).await?.pid)
        } else {
            None
        };
        let rows = if missing {
            agents::install_agent(id.clone(), None, Some(version.clone()), app.clone()).await?
        } else {
            agents::update_agent(id.clone(), Some(version.clone()), app.clone()).await?
        };
        let installed = agents::resolve(app, &id)?;
        let current = rows.iter().find(|row| row.id == id).ok_or("安装状态缺失")?;
        if installed.source != "managed"
            || !installed.program.starts_with(&state.data_dir)
            || current.phase != "ready"
        {
            return Err(format!("{id} 未使用隔离安装的新版"));
        }
        let root = state.data_dir.join("agents").join(&id);
        let backup = std::fs::read_dir(&root)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|e| e.path().join("previous-installation.json"))
            .find(|p| p.is_file())
            .ok_or("缺少旧安装备份记录")?;
        let saved: Value =
            serde_json::from_slice(&std::fs::read(&backup).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if previous.as_ref().is_some_and(|v| !v.program.is_file())
            || saved["version"]
                .as_str()
                .and_then(crate::agent_versions::installed_version)
                .map(|v| v.to_string())
                .as_deref()
                != Some(version.as_str())
        {
            return Err("旧程序未保留或备份版本不匹配".into());
        }
        let warm_replaced = if let Some(pid) = old_pid {
            if state.runtime.client.lock().await.is_some() {
                return Err("更新后仍持有旧 Codex 连接".into());
            }
            let next = state.runtime.get(app).await?;
            let changed = next.pid != pid;
            state.runtime.shutdown().await;
            if !changed {
                return Err("更新未更换旧进程".into());
            }
            Some(changed)
        } else {
            None
        };
        let selected = state.store.setting(&format!("agent_path_{id}"))?;
        if agents::update_agent(id.clone(), Some("0.0.0".into()), app.clone())
            .await
            .is_ok()
            || state.store.setting(&format!("agent_path_{id}"))? != selected
            || agents::resolve(app, &id)?.program != installed.program
        {
            return Err("失败更新覆盖了已选定版本".into());
        }
        if agents::list_agents(app.clone())
            .await?
            .iter()
            .find(|row| row.id == id)
            .is_none_or(|row| row.phase != "updateFailed")
        {
            return Err("更新失败未保留可使用状态".into());
        }
        reports.push(json!({"agent":id,"version":version,"realDownloadInstall":true,"nativeConnectionTest":true,"missingInstallTested":missing,"selectedManaged":true,"oldProgramPreserved":true,"backup":backup,"failedUpdatePreserved":true,"warmConnectionReplaced":warm_replaced}));
        let _ = std::fs::write(
            std::env::current_dir()
                .unwrap_or_default()
                .join(".supercode/agent-updates-progress.json"),
            serde_json::to_vec_pretty(&reports).unwrap_or_default(),
        );
    }
    Ok(json!({"agents":reports,"isolatedData":state.data_dir}))
}
pub async fn run(app: AppHandle) {
    let result = verify(&app).await;
    let report = match result {
        Ok(v) => json!({"ok":true,"result":v}),
        Err(e) => json!({"ok":false,"error":e}),
    };
    let dir = std::env::current_dir()
        .unwrap_or_default()
        .join(".supercode");
    let _ = std::fs::write(
        dir.join(
            if std::env::args().any(|arg| arg == "--agent-updates-test") {
                "agent-updates-report.json"
            } else if std::env::args().any(|arg| arg == "--provider-accounts-test") {
                "provider-accounts-report.json"
            } else {
                "agents-smoke-report.json"
            },
        ),
        serde_json::to_vec_pretty(&report).unwrap_or_default(),
    );
    app.exit(if report["ok"] == true { 0 } else { 1 });
}
