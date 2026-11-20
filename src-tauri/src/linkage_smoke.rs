//! Explicit integration check. Uses an isolated DB/workspace and real CLI/MCP
//! traffic. The installed database is read only; credentials never enter reports.
use crate::{
    ccswitch::Profile,
    linkage::{Choice, Configuration},
    storage::Store,
    AppState,
};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

async fn settled(app: &AppHandle, id: &str) -> Result<(), String> {
    let start = Instant::now();
    loop {
        let state = app.state::<AppState>();
        let root = state.store.session(id)?;
        let children = state.store.linkage_children(id)?;
        if root.status == "waiting" || children.iter().any(|s| s.status == "waiting") {
            return Err("遇到审批或提问，测试未代替用户作答".into());
        }
        if root.status == "failed" || children.iter().any(|s| s.status == "failed") {
            return Err("真实引擎任务失败，请查看隔离聊天的执行记录".into());
        }
        if root.status == "idle" && children.iter().all(|s| s.status == "idle") {
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(180) {
            return Err("真实联动等待超时".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
fn choice(profile: &Profile) -> Result<Choice, String> {
    Ok(Choice {
        agent_id: profile.agent.clone(),
        connection_id: profile.id.clone(),
        model_id: crate::providers::model_ids(&profile.config)
            .first()
            .cloned()
            .ok_or("测试连接没有模型")?,
        effort: None,
    })
}
async fn codex_stop_scope(
    app: &AppHandle,
    controller: &Profile,
    executor: &Profile,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let main = choice(controller)?;
    let execution = choice(executor)?;
    let dir = state.data_dir.join("codex scoped stop 测试");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let project = state.store.add_project(&dir)?;
    let root = state.store.create_configured_session(
        &project.id,
        Some(main.model_id.clone()),
        &main.agent_id,
        Some(&main.connection_id),
    )?;
    crate::linkage::configure_linkage(
        root.id.clone(),
        Configuration {
            mode: "linked".into(),
            controller: main,
            executor: execution.clone(),
        },
        app.clone(),
    )
    .await?;
    let worker = state.store.linkage_worker(&root.id)?;
    let other = state.store.create_configured_session(
        &project.id,
        Some(execution.model_id),
        "codex",
        Some(&execution.connection_id),
    )?;
    let client = state.runtime.get_for_session(app, Some(&worker.id)).await?;
    let other_client = state.runtime.get_for_session(app, Some(&other.id)).await?;
    state.store.bind_native(&worker.id, "stop-fixture-native")?;
    state
        .store
        .set_status(&worker.id, "running", Some("stop-fixture-turn"))?;
    state.store.observe_connection_usage("scope-fixture",Some(&worker.id),&execution.connection_id,&json!({"method":"turn/started","params":{"threadId":"child-fixture","turn":{"id":"child-turn"}}}))?;
    // The simulated background lease is a DB fixture. Both native app-server
    // processes and the scoped stop action below are real; no model turn or
    // external account operation is requested by this check.
    crate::commands::interrupt_turn(root.id.clone(), app.clone()).await?;
    if state.runtime.session_client(&worker.id).await.is_some()
        || state.store.session(&worker.id)?.status != "interrupted"
    {
        return Err("执行进程未按聊天范围停止".into());
    }
    if state.store.linkage_active(&root.id)? {
        return Err("停止后仍有执行方占用".into());
    }
    if state
        .runtime
        .session_client(&other.id)
        .await
        .is_none_or(|current| current.pid != other_client.pid)
    {
        return Err("停止执行方影响了另一个聊天".into());
    }
    state.runtime.for_session(&other.id).shutdown().await;
    Ok(
        json!({"realNativeProcesses":true,"workerPid":client.pid,"otherPid":other_client.pid,"workerStopped":true,"otherChatPreserved":true,"simulatedBackgroundLease":true}),
    )
}
async fn pair(app: &AppHandle, controller: &Profile, executor: &Profile) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let dir = state.data_dir.join(format!(
        "linkage {} to {} 测试",
        controller.agent, executor.agent
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let project = state.store.add_project(&dir)?;
    let main = choice(controller)?;
    let worker = choice(executor)?;
    let root = state.store.create_configured_session(
        &project.id,
        Some(main.model_id.clone()),
        &main.agent_id,
        Some(&main.connection_id),
    )?;
    crate::linkage::configure_linkage(
        root.id.clone(),
        Configuration {
            mode: "linked".into(),
            controller: main.clone(),
            executor: worker.clone(),
        },
        app.clone(),
    )
    .await?;
    let mut rounds = vec![];
    for round in 1..=2 {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let filename = format!("result-{round}.txt");
        std::fs::write(dir.join(format!("input-{round}.txt")), &token)
            .map_err(|e| e.to_string())?;
        let text=format!("这是联动集成验证第 {round} 轮。请调用 supercode_linkage 的 delegate_task，让配置的执行 Agent 读取当前目录 input-{round}.txt，并把完全相同的内容写入 {filename}，再读取验证；执行方最终报告必须包含 WORKER_ONLY_{round} 和实际文件内容。你不要自己读取文件或运行命令。必须调用 executor_wait 跟进直到执行完成；最后只用一句自然中文告诉我文件已生成，不复述执行方的原始标记。可用工具可能需要先发现。任务只允许访问当前测试目录。");
        let result=async {
            crate::commands::send_message(root.id.clone(),text,Some(main.model_id.clone()),false,Some("full".into()),app.clone()).await?;
            settled(app,&root.id).await?;
            let view=crate::linkage::get_linkage_state(root.id.clone(),app.clone()).await?;
            let child=view["worker"]["id"].as_str().ok_or("主控没有调用派活工具")?;
            if view["worker"]["agent"]!=worker.agent_id||view["worker"]["connectionId"]!=worker.connection_id||view["worker"]["model"]!=worker.model_id {return Err("执行方未使用指定引擎、模型和连接".into());}
            let actual=std::fs::read_to_string(dir.join(&filename)).map_err(|e|format!("执行结果文件未生成：{e}"))?;
            if actual.trim()!=token {return Err("执行结果文件内容与输入不同".into());}
            let root_messages=state.store.messages(&root.id,None)?;
            let last_user=root_messages.iter().rposition(|m|m.role=="user").ok_or("缺少主控输入")?;
            let reply=root_messages[last_user..].iter().filter(|m|m.role=="assistant").map(|m|m.text.as_str()).collect::<Vec<_>>().join("\n");
            if reply.is_empty()||reply.contains(&format!("WORKER_ONLY_{round}"))||reply.contains(&token) {return Err("主控未独立表达结果，或原始执行输出泄漏到主回复".into());}
            let child_session=state.store.session(child)?;
            if state.store.session_workspace(&child_session)?.path!=state.store.session_workspace(&state.store.session(&root.id)?)?.path {return Err("执行方项目目录不一致".into());}
            Ok::<Value,String>(json!({"round":round,"workerSessionId":child,"fileVerified":true,"independentReply":true,"nativeMcp":true}))
        }.await;
        match result {
            Ok(value) => rounds.push(value),
            Err(error) => {
                crate::commands::interrupt_turn(root.id.clone(), app.clone())
                    .await
                    .map_err(|stop| format!("{error}；清理失败：{stop}"))?;
                return Err(format!("{error}；隔离聊天 {}", root.id));
            }
        }
    }
    Ok(
        json!({"controller":controller.agent,"executor":executor.agent,"sessionId":root.id,"rounds":rounds}),
    )
}
pub async fn verify(app: &AppHandle, source: &Store) -> Result<Value, String> {
    let state = app.state::<AppState>();
    if let Some(path) = source.codex_path()? {
        state.store.set_setting("codex_path", &path)?;
    }
    crate::notifications::update_notification_preferences(
        app.clone(),
        serde_json::from_value(json!({"enabled":false})).map_err(|e| e.to_string())?,
    );
    let profiles = source.profiles()?;
    let mut selected = vec![];
    for agent in crate::agents::IDS {
        if crate::agents::resolve(app, agent).is_err() {
            continue;
        }
        let explicit = std::env::var(format!(
            "SUPERCODE_LINKAGE_TEST_PROFILE_{}",
            agent.to_uppercase()
        ))
        .ok();
        let profile = if let Some(id) = explicit {
            Some(source.profile(agent, &id)?.ok_or("指定的测试连接不存在")?)
        } else {
            profiles
                .iter()
                .find(|p| {
                    p.agent == agent
                        && !p.id.starts_with("magpie:")
                        && crate::providers::key(&p.config).is_some()
                })
                .cloned()
        };
        if let Some(profile) =
            profile.filter(|p| !crate::providers::model_ids(&p.config).is_empty())
        {
            selected.push(profile);
        }
    }
    let controller = selected
        .iter()
        .find(|p| p.agent == "claude")
        .cloned()
        .ok_or("联动实测需要现有 Claude API 连接；未修改本机配置")?;
    // OpenCode and Pi support the same documented Anthropic protocol. Test it
    // in private connections when there is no separate saved connection.
    for agent in ["opencode", "pi"] {
        if selected.iter().any(|p| p.agent == agent) || crate::agents::resolve(app, agent).is_err()
        {
            continue;
        }
        let mut profile = controller.clone();
        profile.id = format!("linkage-test-{agent}");
        profile.agent = agent.into();
        if !profile.config["baseUrl"].is_string() {
            profile.config["baseUrl"] = profile.config["env"]["ANTHROPIC_BASE_URL"].clone();
            profile.config["apiKey"] = json!(crate::providers::key(&controller.config));
            profile.config["protocol"] = "anthropic".into();
        }
        selected.push(profile);
    }
    state.store.import_profiles(&selected)?;
    let mut checks = vec![];
    if std::env::var_os("SUPERCODE_LINKAGE_TEST_STOP").is_some() {
        let executor = selected
            .iter()
            .find(|p| p.agent == "codex")
            .ok_or("停止范围验证缺少 Codex 连接")?;
        let stopped = codex_stop_scope(app, &controller, executor).await?;
        if std::env::var("SUPERCODE_LINKAGE_TEST_EXECUTOR").as_deref() == Ok("stop-only") {
            return Ok(json!({"allPassed":true,"stopScope":stopped,"isolatedData":state.data_dir}));
        }
        checks.push(json!({"ok":true,"stopScope":stopped}));
    }
    let target = std::env::var("SUPERCODE_LINKAGE_TEST_EXECUTOR").ok();
    for executor in &selected {
        if target
            .as_ref()
            .is_some_and(|target| target != &executor.agent)
        {
            continue;
        }
        let result = pair(app, &controller, executor).await;
        checks.push(match result{Ok(value)=>json!({"ok":true,"result":value}),Err(error)=>json!({"ok":false,"controller":controller.agent,"executor":executor.agent,"error":error})});
    }
    let missing = crate::agents::IDS
        .into_iter()
        .filter(|agent| !selected.iter().any(|p| p.agent == *agent))
        .collect::<Vec<_>>();
    Ok(
        json!({"allPassed":!checks.is_empty()&&checks.iter().all(|c|c["ok"]==true),"checks":checks,"unavailableEngines":missing,"isolatedData":state.data_dir}),
    )
}
