//! Opt-in checks against installed, real agents; test data is isolated from the workspace DB.
use crate::{commands, outbox, storage, AppState};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Listener, Manager};

async fn idle(app: &AppHandle, id: &str) -> Result<(), String> {
    let start = Instant::now();
    loop {
        let s = app.state::<AppState>().store.session(id)?;
        if s.status == "waiting" {
            return Err("测试触发审批或提问，未自动批准".into());
        }
        if s.status == "idle"
            && app
                .state::<AppState>()
                .store
                .followups(Some(id))?
                .is_empty()
        {
            return Ok(());
        }
        if matches!(s.status.as_str(), "failed" | "interrupted") {
            return Err(format!("任务状态 {}，请检查隔离测试记录", s.status));
        }
        if start.elapsed() > Duration::from_secs(180) {
            return Err("等待真实 Agent 超时".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
async fn interrupted(app: &AppHandle, id: &str) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let s = app.state::<AppState>().store.session(id)?;
            if s.status == "interrupted" {
                return Ok::<(), String>(());
            }
            if matches!(s.status.as_str(), "failed" | "waiting" | "idle") {
                return Err(format!("停止测试状态异常：{}", s.status));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| "停止测试状态超时".to_owned())?
}
async fn verify(app: &AppHandle) -> Result<Value, String> {
    let source = storage::Store(std::sync::Mutex::new(
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
    for agent in crate::agents::IDS {
        for prefix in ["agent_install_", "agent_path_"] {
            let key = format!("{prefix}{agent}");
            if let Some(v) = source.setting(&key)? {
                state.store.set_setting(&key, &v)?;
            }
        }
    }
    if let Some(path) = source.codex_path()? {
        state.store.set_setting("codex_path", &path)?;
    }
    let mut reports = vec![];
    for agent in ["claude", "codex", "opencode", "pi"] {
        let audit = std::env::args().any(|v| v == "--input-audit-only");
        if let Some(selected) =
            std::env::args().find_map(|v| v.strip_prefix("--audit-agent=").map(str::to_owned))
        {
            if selected != agent {
                continue;
            }
        }
        if std::env::args().any(|v| v == "--native-followups-only")
            && !matches!(agent, "opencode" | "pi")
        {
            continue;
        }
        let mut route = source.route(agent, None)?;
        if matches!(agent, "opencode" | "pi") && route.profile.is_none() {
            // Exercise the user's already-configured API connection, instead of
            // OpenCode's unconfigured free tier or a missing Pi account.
            let base = source.route("claude", None)?;
            let mut profile = base.profile.ok_or("原生适配器测试需要已配置的 API 连接")?;
            profile.agent = agent.into();
            if !profile.config["baseUrl"].is_string() {
                profile.config["baseUrl"] = profile.config["env"]["ANTHROPIC_BASE_URL"].clone();
                profile.config["apiKey"] = json!(crate::providers::key(&profile.config));
                profile.config["protocol"] = json!("anthropic");
            }
            route.config = profile.config.clone();
            route.profile = Some(profile);
        }
        // Read credentials through the existing DPAPI store and never print them.
        if let Some(mut profile) = route.profile {
            profile.id = format!("followup-{agent}");
            state.store.import_profiles(&[profile.clone()])?;
            state.store.select_profile(agent, Some(&profile.id))?;
        }
        let dir = state.data_dir.join(format!("{agent}-followup-test"));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let project = state.store.add_project(&dir)?;
        let selected = route.config["model"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                crate::providers::model_ids(&route.config)
                    .into_iter()
                    .next()
            });
        let session = state
            .store
            .create_agent_session(&project.id, selected.clone(), agent)?;
        let permission = if agent == "codex" { "ask" } else { "read" };
        commands::send_chat_message(
            session.id.clone(),
            "这是自动化测试。只回复 WARM-FIRST，不使用工具，不读取或修改任何文件。".into(),
            selected.clone(),
            permission == "read",
            Some(permission.into()),
            None,
            None,
            app.clone(),
        )
        .await?;
        idle(app, &session.id).await?;
        let first = commands::runtime_info(app.clone()).await?["pid"]
            .as_u64()
            .ok_or("回复后进程已释放")?;
        let current = state.store.session(&session.id)?;
        commands::send_chat_message(
            session.id.clone(),
            "只回复 WARM-SECOND，不使用工具。".into(),
            current.model.clone(),
            permission == "read",
            Some(permission.into()),
            None,
            None,
            app.clone(),
        )
        .await?;
        idle(app, &session.id).await?;
        let second = commands::runtime_info(app.clone()).await?["pid"]
            .as_u64()
            .ok_or("第二轮后进程已释放")?;
        if first != second {
            return Err(format!("{agent} 未复用相同进程"));
        }
        let current = state.store.session(&session.id)?;
        let payload = |text: &str| outbox::Payload {
            text: text.into(),
            model: current.model.clone(),
            read_only: permission == "read",
            permission_mode: Some(permission.into()),
            attachments: vec![],
            effort: None,
        };
        outbox::enqueue_followup(
            session.id.clone(),
            payload("只回复 QUEUE-THIRD，不使用工具。"),
            app.clone(),
        )
        .await?;
        outbox::enqueue_followup(
            session.id.clone(),
            payload("只回复 QUEUE-FOURTH，不使用工具。"),
            app.clone(),
        )
        .await?;
        idle(app, &session.id).await?;
        let items = state.store.messages(&session.id, None)?;
        let replies: Vec<_> = items
            .iter()
            .filter(|m| m.role == "assistant")
            .map(|m| m.text.clone())
            .collect();
        if !replies.iter().any(|s| s.contains("QUEUE-THIRD"))
            || !replies.iter().any(|s| s.contains("QUEUE-FOURTH"))
        {
            return Err(format!("{agent} 排队消息未收到真实回复"));
        }
        let users: Vec<_> = items.iter().filter(|m| m.role == "user").collect();
        if users.len() != 4
            || !users[2].text.contains("QUEUE-THIRD")
            || !users[3].text.contains("QUEUE-FOURTH")
        {
            return Err(format!("{agent} 排队顺序或去重异常"));
        }
        commands::send_chat_message(
            session.id.clone(),
            "这是运行中引导测试。请写一篇约 2000 字的春天景色描写，不调用工具，不读取文件。".into(),
            current.model.clone(),
            permission == "read",
            Some(permission.into()),
            None,
            None,
            app.clone(),
        )
        .await?;
        // Codex's start response only reserves a turn; test steering after the
        // native start event, rather than relying on an optimistic UI status.
        let target = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let active = state.store.session(&session.id)?;
                if matches!(active.status.as_str(), "running" | "waiting") {
                    return active.turn_id.ok_or("引导测试没有活动任务 ID".to_string());
                }
                if active.status != "starting" {
                    return Err(format!("{agent} 引导测试任务在补充前已结束"));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| format!("{agent} 引导测试没有收到原生启动通知"))??;
        let native = state
            .store
            .session(&session.id)?
            .native_id
            .ok_or("原生会话 ID 缺失")?;
        let completed = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let captured = completed.clone();
        let expected = target.clone();
        let listener = app.listen("agent-event", move |event| {
            if let Ok(v) = serde_json::from_str::<Value>(event.payload()) {
                if v["method"] == "turn/completed"
                    && v["params"]["threadId"] == native
                    && v["params"]["turn"]["id"] == expected
                {
                    let mut rows = captured.lock().unwrap();
                    if rows.len() < 16 {
                        rows.push(v["params"]["turn"].clone());
                    }
                }
            }
        });
        let capability = outbox::followup_capabilities(session.id.clone(), app.clone()).await?;
        let steer_id = outbox::enqueue_followup(
            session.id.clone(),
            payload("改变要求：立即结束景色描写，只回复 STEER-OK，不使用工具。"),
            app.clone(),
        )
        .await?;
        if audit {
            if outbox::steer_followup(steer_id.clone(), "outdated-turn".into(), app.clone())
                .await
                .is_ok()
            {
                return Err("过期任务 ID 被接受".into());
            }
            if state
                .store
                .followups(Some(&session.id))?
                .iter()
                .all(|v| v.id != steer_id)
            {
                return Err("过期引导请求丢失排队消息".into());
            }
            let mut changed_settings =
                payload("配置变化测试：此消息必须保留，不能在当前只读任务中发送。");
            changed_settings.permission_mode = Some("full".into());
            changed_settings.read_only = false;
            let changed_id =
                outbox::enqueue_followup(session.id.clone(), changed_settings, app.clone()).await?;
            if outbox::steer_followup(changed_id.clone(), target.clone(), app.clone())
                .await
                .is_ok()
            {
                return Err("引导静默忽略权限变化".into());
            }
            if !state
                .store
                .followups(Some(&session.id))?
                .iter()
                .any(|r| r.id == changed_id && r.status == "failed")
            {
                return Err("配置变化引导未保留消息".into());
            }
            outbox::change_followup(changed_id, "remove".into(), None, app.clone()).await?;
        }
        outbox::steer_followup(steer_id.clone(), target, app.clone()).await?;
        idle(app, &session.id).await?;
        app.unlisten(listener);
        let after_steer = commands::runtime_info(app.clone()).await?["pid"].as_u64();
        let completed = completed.lock().unwrap().clone();
        if audit
            && (capability["steeringMode"] != "native"
                || after_steer != Some(second)
                || completed.len() != 1
                || completed[0]["status"] != "completed")
        {
            return Err(format!(
                "{agent} 原生引导状态/进程复用异常：{}",
                json!({"mode":capability,"samePid":after_steer==Some(second),"completion":completed})
            ));
        }
        if !state
            .store
            .messages(&session.id, None)?
            .iter()
            .any(|m| m.role == "assistant" && m.text.contains("STEER-OK"))
        {
            return Err(format!("{agent} 没有执行引导消息"));
        }
        let followup_messages = state
            .store
            .messages(&session.id, None)?
            .iter()
            .filter(|m| m.id == format!("user-{steer_id}"))
            .count();
        if followup_messages != 1 {
            return Err(format!("{agent} 引导用户消息去重异常"));
        }
        let mut stop_reports = vec![];
        if audit {
            for immediate in [false, true] {
                let native = state
                    .store
                    .session(&session.id)?
                    .native_id
                    .ok_or("会话 ID 缺失")?;
                let output = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let observed = output.clone();
                let listener = app.listen("agent-event", move |event| {
                    if let Ok(v) = serde_json::from_str::<Value>(event.payload()) {
                        if v["params"]["threadId"] == native
                            && v["method"] == "item/started"
                            && matches!(
                                v["params"]["item"]["type"].as_str(),
                                Some("agentMessage" | "reasoning")
                            )
                        {
                            observed.store(true, std::sync::atomic::Ordering::Release);
                        }
                    }
                });
                commands::send_chat_message(
                    session.id.clone(),
                    "这是停止测试，写一篇约 2000 字的景色描写，不使用工具。".into(),
                    current.model.clone(),
                    permission == "read",
                    Some(permission.into()),
                    None,
                    None,
                    app.clone(),
                )
                .await?;
                if !immediate {
                    tokio::time::timeout(Duration::from_secs(90), async {
                        while !output.load(std::sync::atomic::Ordering::Acquire) {
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                    })
                    .await
                    .map_err(|_| format!("{agent} 停止测试未收到真实输出"))?;
                }
                if agent == "pi" && !immediate {
                    let pending_id = outbox::enqueue_followup(
                        session.id.clone(),
                        payload("只回复 CANCELED-SHOULD-NOT-RUN，不使用工具。"),
                        app.clone(),
                    )
                    .await?;
                    let active = state
                        .store
                        .session(&session.id)?
                        .turn_id
                        .ok_or("Pi 停止测试任务 ID 缺失")?;
                    outbox::steer_followup(pending_id, active, app.clone()).await?;
                }
                commands::interrupt_turn(session.id.clone(), app.clone()).await?;
                interrupted(app, &session.id).await?;
                if agent == "pi"
                    && state.store.messages(&session.id, None)?.iter().any(|m| {
                        m.role == "assistant" && m.text.contains("CANCELED-SHOULD-NOT-RUN")
                    })
                {
                    return Err("Pi 停止后执行了内部排队消息".into());
                }
                app.unlisten(listener);
                let after_stop = commands::runtime_info(app.clone()).await?["pid"].as_u64();
                if !immediate && after_stop != Some(second) {
                    return Err(format!("{agent} 正常停止后未保留连接"));
                }
                commands::send_chat_message(
                    session.id.clone(),
                    "只回复 STOP-RESUME-OK，不使用工具。".into(),
                    current.model.clone(),
                    permission == "read",
                    Some(permission.into()),
                    None,
                    None,
                    app.clone(),
                )
                .await?;
                idle(app, &session.id).await?;
                if !state
                    .store
                    .messages(&session.id, None)?
                    .iter()
                    .rev()
                    .take(8)
                    .any(|m| m.role == "assistant" && m.text.contains("STOP-RESUME-OK"))
                {
                    return Err(format!("{agent} 停止后续聊失败"));
                }
                let after_resume = commands::runtime_info(app.clone()).await?["pid"].as_u64();
                if !immediate && after_resume != Some(second) {
                    return Err(format!("{agent} 停止后续聊重新连接"));
                }
                stop_reports.push(json!({"immediate":immediate,"interrupted":true,"resumed":true,"samePid":after_stop==after_resume,"pidRetained":after_stop.is_some(),"cancelledNativeQueuedInput":agent=="pi"&&!immediate}));
            }
        }
        reports.push(json!({"agent":agent,"realAgent":true,"samePid":first==second,"samePidAfterSteer":after_steer==Some(second),"nativeMode":capability["steeringMode"],"singleCompletion":completed.len()==1,"completion":completed,"fifo":true,"steer":true,"stopResume":stop_reports,"followupUserMessages":followup_messages,"userMessagesBeforeSteer":users.len(),"repliesBeforeSteer":replies.len()}));
        let _ = std::fs::write(
            std::env::current_dir().unwrap_or_default().join(if audit {
                ".supercode/agent-input-progress.json"
            } else {
                ".supercode/followup-smoke-progress.json"
            }),
            serde_json::to_vec_pretty(&reports).unwrap_or_default(),
        );
        commands::release_runtime(app.clone()).await?;
    }
    Ok(json!({"adapters":reports,"isolatedData":state.data_dir}))
}
pub async fn run(app: AppHandle) {
    let result = verify(&app).await;
    let report = match result {
        Ok(value) => json!({"ok":true,"result":value}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    let root = std::env::current_dir()
        .unwrap_or_default()
        .join(".supercode");
    let _ = std::fs::write(
        root.join(if std::env::args().any(|v| v == "--input-audit-only") {
            "agent-input-report.json"
        } else {
            "followup-smoke-report.json"
        }),
        serde_json::to_vec_pretty(&report).unwrap_or_default(),
    );
    app.state::<AppState>().claude.shutdown().await;
    app.state::<AppState>().native.shutdown().await;
    app.state::<AppState>().runtime.shutdown().await;
    app.exit(if report["ok"] == true { 0 } else { 1 });
}
