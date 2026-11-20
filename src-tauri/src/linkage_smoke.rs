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

async fn settled(app: &AppHandle, id: &str, progress_dir: Option<&std::path::Path>) -> Result<Option<Value>, String> {
    let start = Instant::now();
    let mut progress_reply=None;
    loop {
        let state = app.state::<AppState>();
        let root = state.store.session(id)?;
        let children = state.store.linkage_children(id)?;
        let task = state.store.linkage_task(id)?;
        if let Some(dir)=progress_dir.filter(|_|progress_reply.is_none()) {
            if root.status=="idle" && children.iter().any(|child|child.status=="running") && dir.join("stage-1.txt").is_file() && task.as_ref().is_some_and(|task|task.state=="working") {
                let messages=state.store.messages(id,None)?;
                let (_,updates)=state.store.linkage_updates(id)?;
                let stage_cursor=updates.iter().find(|update|update.text.as_ref().is_some_and(|text|text.contains("WORKER_PROGRESS_1"))).map(|update|update.cursor);
                if let Some(wake)=messages.iter().rposition(|message|message.kind=="linkageProgress") {
                    // A preparatory worker message can also be complete. The
                    // host must wait for the actual stage checkpoint, not just
                    // for an earlier "execution started" acknowledgement.
                    let checkpoint_seen=stage_cursor.is_some_and(|cursor|messages[wake].data["linkageProgress"]["cursor"].as_i64().is_some_and(|seen|seen>=cursor))
                        || messages[wake..].iter().filter(|message|message.role=="tool").any(|message|serde_json::from_str::<Value>(&message.text).ok().is_some_and(|report|report["updates"].as_array().is_some_and(|updates|updates.iter().any(|update|update["text"].as_str().is_some_and(|text|text.contains("WORKER_PROGRESS_1"))))));
                    let reply=messages[wake..].iter().filter(|message|message.role=="assistant").map(|message|message.text.as_str()).collect::<Vec<_>>().join("\n");
                    if checkpoint_seen && !reply.trim().is_empty() {
                        if reply.contains("WORKER_PROGRESS") {return Err("主控直接复述了执行方的原始进展标记".into());}
                        std::fs::write(dir.join("continue.flag"),"host verified controller progress").map_err(|e|e.to_string())?;
                        progress_reply=Some(json!({"reply":reply,"stageCheckpointSeen":true,"workerStillRunning":true,"taskStillWorking":true,"controllerRested":true}));
                    }
                }
            }
        }
        if root.status == "waiting" || children.iter().any(|s| s.status == "waiting") {
            return Err("遇到审批或提问，测试未代替用户作答".into());
        }
        if root.status == "failed" || children.iter().any(|s| s.status == "failed") {
            return Err("真实引擎任务失败，请查看隔离聊天的执行记录".into());
        }
        if task.as_ref().is_some_and(|task|matches!(task.state.as_str(),"blocked"|"waiting_user"|"cancelled") || task.review_error.is_some()) {
            return Err("整项任务未验收完成，请检查隔离聊天的验收状态".into());
        }
        if root.status == "idle" && children.iter().all(|s| s.status == "idle") && task.as_ref().is_some_and(|task|task.state=="completed") {
            return Ok(progress_reply);
        }
        let timeout = if std::env::var_os("SUPERCODE_LINKAGE_TEST_SUPERVISION").is_some() || progress_dir.is_some() { 600 } else { 180 };
        if start.elapsed() > Duration::from_secs(timeout) {
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
        effort: (std::env::var_os("SUPERCODE_LINKAGE_TEST_SUPERVISION").is_some() || std::env::var_os("SUPERCODE_LINKAGE_TEST_PROGRESS").is_some()).then(||"low".into()),
        service_tier: None,
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
    let supervision = std::env::var_os("SUPERCODE_LINKAGE_TEST_SUPERVISION").is_some();
    let progress_test = std::env::var_os("SUPERCODE_LINKAGE_TEST_PROGRESS").is_some();
    for round in 1..=if supervision || progress_test {1} else {2} {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let filename = format!("result-{round}.txt");
        std::fs::write(dir.join(format!("input-{round}.txt")), &token)
            .map_err(|e| e.to_string())?;
        let text=if progress_test {
            format!("这是阶段进展的真实联动验证。完整计划两项：1. 执行方读取 input-{round}.txt 原样复制为 stage-{round}.txt 并核对；2. 执行方把 stage-{round}.txt 原样复制为 {filename} 并回读核对。请 task_plan 登记两项，再一次 delegate_task 派发两项完整工作。委派时明确要求：第一项用一条 Python 命令完成；第一项完成后先发一段完整简短的进展说明，包含私有标记 WORKER_PROGRESS_{round}，接着执行第二条 Python 命令，等待当前目录 continue.flag 存在（每 0.25 秒检查，最多 240 秒），然后复制并核对第二项文件。continue.flag 由测试程序自动创建，是为了验证主控先向用户表达进展、执行方仍继续运行；不要要求用户创建它。执行方最终报告包含 WORKER_ONLY_{round} 和真实文件内容。你在首次派发后调用 executor_wait 交给软件监听并结束本轮；之后收到阶段进展时，必须用一句自然中文说明第一项已完成、第二项还在继续，并调用 executor_wait 交接后结束本轮；你不能复制私有标记或文件内容。等执行方实际结束，再逐项 task_review，全部验证后才 completed。你不要自己运行命令或读写文件。任务只允许访问当前隔离测试目录。")
        } else if supervision {
            format!("这是主控自动续派集成验证，完整计划恰有两项：1. 执行方读取 input-{round}.txt，等待 3 秒后原样写入 stage-{round}.txt；2. 执行方读取 stage-{round}.txt，原样写入 {filename} 并重新读取核对。请先用 task_plan 登记两项完整清单，再 delegate_task 仅派发第一项，明确让执行方第一项完成就停、不要提前执行第二项。这是一次特意模拟提前结束的测试：第一项派发成功后，请直接用一句中文回复已开始并结束你这一轮，不调用 wait，不标记完成；软件随后会在执行方结束时安排你验收并续派第二项。进入后续验收阶段时，你必须读取真实状态并通过 task_review 的 continue 实际派发第二项，全部完成再逐项验收为 completed。执行方最终报告必须包含 WORKER_ONLY_{round} 和实际文件内容，你不能自己运行命令或读取文件，也不要向用户复述原始标记或文件内容。只允许访问当前测试目录。")
        } else {
            format!("这是联动集成验证第 {round} 轮。先通过 task_plan 登记文件生成和验证清单，再调用 supercode_linkage 的 delegate_task，让配置的执行 Agent 读取当前目录 input-{round}.txt，并把完全相同的内容写入 {filename}，再读取验证；执行方最终报告必须包含 WORKER_ONLY_{round} 和实际文件内容。你不要自己读取文件或运行命令。必须调用 executor_wait 跟进，执行方结束后调用 task_review 对全部清单逐项验收，全部通过才标记 completed。最后只用一句自然中文告诉我文件已生成，不复述执行方的原始标记。可用工具可能需要先发现。任务只允许访问当前测试目录。")
        };
        let text = if supervision {format!("{text}\n为减少测试开销，请在派发时要求执行方每个阶段使用一条 Python 命令完成复制、字节核对和输出，随后简短报告即可。")} else if progress_test {format!("{text}\n整个测试最终也只回复一句自然中文说明两项验收完成，不展开内部测试流程。")} else {text};
        let result=async {
            crate::commands::send_chat_message_inner(root.id.clone(),text,Some(main.model_id.clone()),false,Some("full".into()),None,main.effort.clone(),None,None,main.service_tier.clone(),None,app.clone()).await?;
            let progress_delivery=settled(app,&root.id,progress_test.then_some(dir.as_path())).await?;
            if progress_test && progress_delivery.is_none() {return Err("执行方仍运行时没有收到主控独立表达的阶段进展".into());}
            let view=crate::linkage::get_linkage_state(root.id.clone(),app.clone()).await?;
            let child=view["worker"]["id"].as_str().ok_or("主控没有调用派活工具")?;
            if view["worker"]["agent"]!=worker.agent_id||view["worker"]["connectionId"]!=worker.connection_id||view["worker"]["model"]!=worker.model_id {return Err("执行方未使用指定引擎、模型和连接".into());}
            let actual=std::fs::read_to_string(dir.join(&filename)).map_err(|e|format!("执行结果文件未生成：{e}"))?;
            if actual.trim()!=token {return Err("执行结果文件内容与输入不同".into());}
            let task=state.store.linkage_task(&root.id)?.ok_or("没有整项验收清单")?;
            if task.state!="completed" || task.steps.iter().any(|step|!step.done || step.evidence.trim().is_empty()) {return Err("完整清单没有逐项验收通过".into());}
            if supervision && (task.round<2 || std::fs::read_to_string(dir.join(format!("stage-{round}.txt"))).ok().as_deref()!=Some(token.as_str())) {return Err("只执行第一项后，没有续派完成剩余项".into());}
            let root_messages=state.store.messages(&root.id,None)?;
            let last_user=root_messages.iter().rposition(|m|m.role=="user").ok_or("缺少主控输入")?;
            let review_turns=root_messages[last_user..].iter().filter(|m|m.kind=="linkageReview").count();
            let progress_turns=root_messages[last_user..].iter().filter(|m|m.kind=="linkageProgress").count();
            if supervision && review_turns==0 {return Err("主控提前结束后没有自动安排验收".into());}
            let reply=root_messages[last_user..].iter().filter(|m|m.role=="assistant").map(|m|m.text.as_str()).collect::<Vec<_>>().join("\n");
            if reply.is_empty()||reply.contains(&format!("WORKER_ONLY_{round}"))||reply.contains("WORKER_PROGRESS")||reply.contains(&token) {return Err("主控未独立表达结果，或原始执行输出泄漏到主回复".into());}
            let child_session=state.store.session(child)?;
            if state.store.session_workspace(&child_session)?.path!=state.store.session_workspace(&state.store.session(&root.id)?)?.path {return Err("执行方项目目录不一致".into());}
            Ok::<Value,String>(json!({"round":round,"workerSessionId":child,"fileVerified":true,"independentReply":true,"nativeMcp":true,"accepted":true,"executionRounds":task.round,"automaticReviewTurns":review_turns,"automaticProgressTurns":progress_turns,"progressDelivery":progress_delivery}))
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
