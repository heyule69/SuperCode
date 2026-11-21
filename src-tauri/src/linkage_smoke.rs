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
        if progress_dir.is_some() && root.status == "idle" && task.as_ref().is_some_and(|task| task.live()) {
            return Err("主控在整项任务验收前结束了当前轮次".into());
        }
        if std::env::var_os("SUPERCODE_LINKAGE_TEST_DESIGN").is_some() && root.status=="idle" && task.is_none() {
            return Err("主控未登记或委派设计调查任务就结束了回复".into());
        }
        if let Some(dir)=progress_dir.filter(|_|progress_reply.is_none()) {
            if root.status=="running" && children.iter().any(|child|child.status=="running") && dir.join("stage-1.txt").is_file() && task.as_ref().is_some_and(|task|task.state=="working") {
                let messages=state.store.messages(id,None)?;
                let checkpoint=messages.iter().rposition(|message|message.role!="assistant" && (message.text.contains("WORKER_PROGRESS_1") || message.data.to_string().contains("WORKER_PROGRESS_1")));
                if let Some(checkpoint)=checkpoint {
                    let reply=messages[checkpoint+1..].iter().filter(|message|message.role=="assistant").map(|message|message.text.as_str()).collect::<Vec<_>>().join("\n");
                    if !reply.trim().is_empty() {
                        if reply.contains("WORKER_PROGRESS") {return Err("主控直接复述了执行方的原始进展标记".into());}
                        std::fs::write(dir.join("continue.flag"),"host verified continuous controller progress").map_err(|e|e.to_string())?;
                        progress_reply=Some(json!({"reply":reply,"stageCheckpointSeen":true,"workerStillRunning":true,"taskStillWorking":true,"controllerStillRunning":true}));
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
        let timeout = if std::env::var_os("SUPERCODE_LINKAGE_TEST_SUPERVISION").is_some() || std::env::var_os("SUPERCODE_LINKAGE_TEST_DESIGN").is_some() || progress_dir.is_some() { 600 } else { 180 };
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
        effort: (std::env::var_os("SUPERCODE_LINKAGE_TEST_SUPERVISION").is_some() || std::env::var_os("SUPERCODE_LINKAGE_TEST_PROGRESS").is_some() || std::env::var_os("SUPERCODE_LINKAGE_TEST_DESIGN").is_some()).then(||"low".into()),
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
    if std::env::var_os("SUPERCODE_LINKAGE_TEST_DESIGN").is_some() { return design_pair(app,controller,executor).await; }
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
    let mut worker_identity: Option<(String, String)> = None;
    let supervision = std::env::var_os("SUPERCODE_LINKAGE_TEST_SUPERVISION").is_some();
    let progress_test = std::env::var_os("SUPERCODE_LINKAGE_TEST_PROGRESS").is_some();
    for round in 1..=if supervision || progress_test {1} else {2} {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let filename = format!("result-{round}.txt");
        std::fs::write(dir.join(format!("input-{round}.txt")), &token)
            .map_err(|e| e.to_string())?;
        let text=if progress_test {
            format!("这是阶段进展的真实联动验证。完整计划两项：1. 执行方读取 input-{round}.txt 原样复制为 stage-{round}.txt 并核对；2. 执行方把 stage-{round}.txt 原样复制为 {filename} 并回读核对。请 task_plan 登记两项，再一次 delegate_task 派发两项完整工作。委派时明确要求：第一项用一条 Python 命令完成；第一项完成后先发一段完整简短的进展说明，包含私有标记 WORKER_PROGRESS_{round}，接着执行第二条 Python 命令，等待当前目录 continue.flag 存在（每 0.25 秒检查，最多 240 秒），然后复制并核对第二项文件。continue.flag 由测试程序自动创建，是为了验证主控先向用户表达进展、执行方仍继续运行；不要要求用户创建它。执行方最终报告包含 WORKER_ONLY_{round} 和真实文件内容。你在首次派发后调用 executor_wait 在同一轮中等待；收到阶段进展时，必须用一句自然中文说明第一项已完成、第二项还在继续，然后调用 executor_wait 继续等待，不结束当前主控轮次；你不能复制私有标记或文件内容。等执行方实际结束，再逐项 task_review，全部验证后才 completed。你不要自己运行命令或读写文件。任务只允许访问当前隔离测试目录。")
        } else if supervision {
            format!("这是主控连续监督和续派集成验证，完整计划恰有两项：1. 执行方读取 input-{round}.txt，等待 3 秒后原样写入 stage-{round}.txt；2. 执行方读取 stage-{round}.txt，原样写入 {filename} 并重新读取核对。请先用 task_plan 登记两项完整清单，再 delegate_task 仅派发第一项，明确让执行方第一项完成就停、不要提前执行第二项。这是一次特意模拟提前结束的测试：第一项派发成功后，请调用 executor_wait 在当前主控轮次中持续等待，不提前结束回复或标记完成；执行方结束后，你必须读取真实状态并通过 task_review 的 continue 实际派发第二项，全部完成再逐项验收为 completed。执行方最终报告必须包含 WORKER_ONLY_{round} 和实际文件内容，你不能自己运行命令或读取文件，也不要向用户复述原始标记或文件内容。只允许访问当前测试目录。")
        } else {
            format!("这是联动集成验证第 {round} 轮。先通过 task_plan 登记文件生成和验证清单，再调用 supercode_linkage 的 delegate_task，让配置的执行 Agent 读取当前目录 input-{round}.txt，并把完全相同的内容写入 {filename}，再读取验证；执行方最终报告必须包含 WORKER_ONLY_{round} 和实际文件内容。你不要自己读取文件或运行命令。必须调用 executor_wait 跟进，执行方结束后调用 task_review 对全部清单逐项验收，全部通过才标记 completed。最后只用一句自然中文告诉我文件已生成，不复述执行方的原始标记。可用工具可能需要先发现。任务只允许访问当前测试目录。")
        };
        let text = if supervision {format!("{text}\n为减少测试开销，请在派发时要求执行方每个阶段使用一条 Python 命令完成复制、字节核对和输出，随后简短报告即可。")} else if progress_test {
            let silence = std::env::var("SUPERCODE_LINKAGE_TEST_SILENCE_SECONDS").ok().map(|value| value.parse::<u64>().map_err(|_| "测试静默时长无效")).transpose()?.unwrap_or(0);
            format!("{text}\n派发时要求第一条 Python 命令先 time.sleep({silence}) 秒再复制文件，保持这段工具执行期间静默，模拟没有新消息的长任务。若命令工具有超时参数，需设为至少 300 秒；执行方可以等待工具，但这段静默期间不要另外发送阶段消息。整个测试最终也只回复一句自然中文说明两项验收完成，不展开内部测试流程。")
        } else {text};
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
            if (supervision || progress_test) && (review_turns>0 || progress_turns>0) {return Err("连续跟进期间主控提前结束并被重新启动".into());}
            let reply=root_messages[last_user..].iter().filter(|m|m.role=="assistant").map(|m|m.text.as_str()).collect::<Vec<_>>().join("\n");
            if reply.is_empty()||reply.contains(&format!("WORKER_ONLY_{round}"))||reply.contains("WORKER_PROGRESS")||reply.contains(&token) {return Err("主控未独立表达结果，或原始执行输出泄漏到主回复".into());}
            let child_session=state.store.session(child)?;
            let native_id=child_session.native_id.as_deref().ok_or("执行方缺少原生会话 ID")?;
            if let Some((session_id,native))=&worker_identity {
                if session_id!=child || native!=native_id { return Err("同一聊天重复派活没有复用原执行方会话".into()); }
            } else { worker_identity=Some((child.to_owned(),native_id.to_owned())); }
            if state.store.session_workspace(&child_session)?.path!=state.store.session_workspace(&state.store.session(&root.id)?)?.path {return Err("执行方项目目录不一致".into());}
            Ok::<Value,String>(json!({"round":round,"workerSessionId":child,"workerNativeSessionId":native_id,"fileVerified":true,"independentReply":true,"nativeMcp":true,"accepted":true,"executionRounds":task.round,"automaticReviewTurns":review_turns,"automaticProgressTurns":progress_turns,"progressDelivery":progress_delivery}))
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
async fn design_pair(app: &AppHandle, controller: &Profile, executor: &Profile) -> Result<Value,String> {
    if std::env::var_os("SUPERCODE_LINKAGE_TEST_CONTROLLER_DESIGN").is_some() { return controller_design_pair(app,controller,executor).await; }
    let supplied_context=std::env::var_os("SUPERCODE_LINKAGE_TEST_EXECUTOR_CONTEXT").is_some();
    let executor_direction=supplied_context || std::env::var_os("SUPERCODE_LINKAGE_TEST_EXECUTOR_DIRECTION").is_some();
    let state=app.state::<AppState>();
    let dir=state.data_dir.join(format!("design-{}-to-{}",controller.agent,executor.agent));
    std::fs::create_dir_all(dir.join("src")).map_err(|e|e.to_string())?;
    let source=format!("// WORKER_ONLY_SOURCE_{}\nexport function calculateShipping(order: {{total:number, vip:boolean}}) {{\n  return order.total >= 99 ? 0 : 12;\n}}\n",uuid::Uuid::new_v4());
    std::fs::write(dir.join("src/shipping.ts"),&source).map_err(|e|e.to_string())?;
    let policy="# 运费调研与约束\n已发布的满 99 元免邮门槛保持不变，VIP 沿用同一运费规则。小额订单收取 12 元引发投诉；成本核算允许把基础运费降低到 8 元，但不能降为 0 元。这里只讨论调整方案，尚未要求修改实现。\n";
    if executor_direction { std::fs::write(dir.join("PRODUCT_RULES.md"),policy).map_err(|e|e.to_string())?; }
    // Give the fixture its own repository boundary so native discovery cannot
    // mistake the hosting SuperCode checkout for the project under inspection.
    let git=crate::process::find_program("git").ok_or("设计集成验证需要 Git 创建隔离项目")?;
    for args in [&["init","--quiet"][..],&["add","--","src/shipping.ts"][..]] {
        let result=crate::process::command(&git,args).current_dir(&dir).output().await.map_err(|e|e.to_string())?;
        if !result.status.success() { return Err("无法初始化隔离测试项目的 Git 边界".into()); }
    }
    if executor_direction && !crate::process::command(&git,&["add","--","PRODUCT_RULES.md"]).current_dir(&dir).output().await.map_err(|e|e.to_string())?.status.success() { return Err("无法登记隔离项目的产品约束".into()); }
    let project=state.store.add_project(&dir)?;
    let main=choice(controller)?;
    let root=state.store.create_configured_session(&project.id,Some(main.model_id.clone()),&main.agent_id,Some(&main.connection_id))?;
    crate::linkage::configure_linkage(root.id.clone(),Configuration{mode:"linked".into(),controller:main.clone(),executor:choice(executor)?},app.clone()).await?;
    let result=async {
        // This request contains no delegation hint: role routing must be the
        // application's default, including a search-only/design-only task.
        let prompt=if supplied_context { "你看看这个项目的运费怎么调整更合适？资料都给你：当前 total >= 99 时免邮，否则收 12 元；VIP 沿用相同规则。免邮门槛保持 99，成本核算允许基础费降到 8 元，但不能降为 0。先给我调整方案，暂不创建或修改项目文件。" } else if executor_direction { "你看看这个项目的运费该怎么调整比较合适？先给我方案，暂不创建或修改项目文件。" } else { "请在当前项目里找到运费计算逻辑，分析现在的免邮规则，设计改成满 200 免邮、VIP 无条件免邮的方案，并列出需要验证的边界情况。先给方案，不创建或修改任何项目文件。请用简洁自然的中文说明。" };
        crate::commands::send_chat_message_inner(root.id.clone(),prompt.into(),Some(main.model_id.clone()),false,Some("full".into()),None,Some("low".into()),None,None,None,None,app.clone()).await?;
        settled(app,&root.id,None).await?;
        let worker=state.store.linkage_worker(&root.id)?;
        let primary=state.store.messages(&root.id,None)?;
        let secondary=state.store.messages(&worker.id,None)?;
        let forbidden=["Read","Glob","Grep","Bash","Write","Edit","LS","read","grep","glob","bash","ls","find","read_file","read_many_files","list_directory","run_shell_command","exec_command","apply_patch"];
        let violations:Vec<_>=primary.iter().filter(|message|matches!(message.kind.as_str(),"commandExecution"|"fileChange") || message.data["tool"].as_str().is_some_and(|name|forbidden.contains(&name))).map(|message|message.kind.clone()).collect();
        if !violations.is_empty() { return Err(format!("主控仍直接使用项目工具：{violations:?}")); }
        let investigated=secondary.iter().any(|message|message.role=="tool" && (message.text.contains("calculateShipping") || message.data.to_string().contains("calculateShipping")));
        if !investigated && !supplied_context { return Err("执行方没有实际查找和读取源代码".into()); }
        let reply=primary.iter().filter(|message|message.role=="assistant").map(|message|message.text.as_str()).collect::<Vec<_>>().join("\n");
        if !reply.contains("99") || (if executor_direction { !reply.contains('8') || reply.contains("200") } else { !reply.contains("200") || !reply.to_ascii_uppercase().contains("VIP") }) || reply.contains("WORKER_ONLY_SOURCE") { return Err("主控未如实整理执行方基于项目事实给出的方案，或泄漏原始标记".into()); }
        let mut assignment=None;
        if executor_direction {
            let delegation=primary.iter().find(|message|message.role=="tool" && message.data["tool"].as_str().is_some_and(|tool|tool.ends_with("delegate_task"))).ok_or("主控没有委派更改方向的分析任务")?;
            let text=delegation.data["arguments"]["text"].as_str().ok_or("缺少执行方收到的任务内容")?;
            if !["方向","方案","建议","分析"].iter().any(|word|text.contains(word)) || !supplied_context && ["99","200","12","8元","8 元"].iter().any(|word|text.contains(word)) { return Err("主控预设了用户没有规定的项目方向，或没有让执行方判断方案".into()); }
            assignment=Some(text.to_owned());
            if !supplied_context && primary.iter().filter(|message|message.data["tool"].as_str().is_some_and(|tool|tool.ends_with("task_plan"))).any(|message|["99","200","12","8元","8 元"].iter().any(|word|message.data["arguments"]["plan"].to_string().contains(word))) { return Err("验收登记夹带了主控自行选择的业务改动".into()); }
            let worker_reply=secondary.iter().filter(|message|message.role=="assistant").map(|message|message.text.as_str()).collect::<Vec<_>>().join("\n");
            if !worker_reply.contains('8') || !worker_reply.contains("99") { return Err("项目更改方向没有由执行方基于产品约束给出".into()); }
            if std::fs::read_to_string(dir.join("PRODUCT_RULES.md")).map_err(|e|e.to_string())?!=policy { return Err("只给方案时改写了产品约束".into()); }
        }
        if std::fs::read_to_string(dir.join("src/shipping.ts")).map_err(|e|e.to_string())? != source
            || std::fs::read_dir(&dir).map_err(|e|e.to_string())?.count()!=if executor_direction {3} else {2}
            || std::fs::read_dir(dir.join("src")).map_err(|e|e.to_string())?.count()!=1 { return Err("只做方案时项目文件发生了变化".into()); }
        let task=state.store.linkage_task(&root.id)?.ok_or("没有登记并验收设计任务")?;
        if task.state!="completed" || task.steps.iter().any(|step|!step.done) { return Err("设计方案未完成逐项验收".into()); }
        Ok(json!({"controller":controller.agent,"executor":executor.agent,"controllerTurns":primary.iter().filter(|message|message.kind=="runMarker").count(),"controllerProjectToolCalls":violations.len(),"workerSearchedSource":investigated,"executorChoseDirection":executor_direction,"suppliedContext":supplied_context,"assignment":assignment,"projectUnchanged":true,"accepted":true,"reply":reply,"sessionId":root.id}))
    }.await;
    if result.is_err() { crate::commands::interrupt_turn(root.id.clone(),app.clone()).await?; }
    result
}

async fn controller_design_pair(app:&AppHandle,controller:&Profile,executor:&Profile)->Result<Value,String> {
    let state=app.state::<AppState>();
    let dir=state.data_dir.join("controller-design-from-chat");
    std::fs::create_dir_all(&dir).map_err(|e|e.to_string())?;
    let project=state.store.add_project(&dir)?;
    let main=choice(controller)?;
    let root=state.store.create_configured_session(&project.id,Some(main.model_id.clone()),&main.agent_id,Some(&main.connection_id))?;
    crate::linkage::configure_linkage(root.id.clone(),Configuration{mode:"linked".into(),controller:main.clone(),executor:choice(executor)?},app.clone()).await?;
    let result=async {
        let prompt="这轮明确由主 Agent 亲自出方案，副 Agent 暂时不用参与。现状和需求已经完整提供：订单有 total 和 vip，当前运费逻辑为 total >= 99 ? 0 : 12，vip 没有参与计算。请设计改成满 200 免邮、VIP 无条件免邮的规则，并列出边界情况。直接基于这些信息思考并简洁回答，不查询项目、不读写文件、不执行命令、不做实际测试。";
        crate::commands::send_chat_message_inner(root.id.clone(),prompt.into(),Some(main.model_id.clone()),false,Some("full".into()),None,Some("low".into()),None,None,None,None,app.clone()).await?;
        let started=Instant::now();
        loop {
            let current=state.store.session(&root.id)?;
            if current.status=="idle" { break; }
            if matches!(current.status.as_str(),"waiting"|"failed"|"interrupted") { return Err("主控自行设计时未正常完成，测试没有代替用户批准或回答".into()); }
            if started.elapsed()>Duration::from_secs(180) { return Err("主控自行设计的真实验证超时".into()); }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let messages=state.store.messages(&root.id,None)?;
        let project_tools=["Read","Glob","Grep","Bash","Write","Edit","LS","read","grep","glob","bash","ls","find","read_file","read_many_files","list_directory","run_shell_command","exec_command","apply_patch"];
        if !state.store.linkage_children(&root.id)?.is_empty() || messages.iter().any(|message|matches!(message.kind.as_str(),"commandExecution"|"fileChange") || message.data["tool"].as_str().is_some_and(|name|project_tools.contains(&name))) { return Err("资料完整且指定主控设计时，仍委派执行或调用了项目工具".into()); }
        let reply=messages.iter().filter(|message|message.role=="assistant").map(|message|message.text.as_str()).collect::<Vec<_>>().join("\n");
        if !reply.contains("200") || !reply.to_ascii_uppercase().contains("VIP") || std::fs::read_dir(&dir).map_err(|e|e.to_string())?.next().is_some() { return Err("主控没有根据已有资料给出方案，或操作了项目文件".into()); }
        Ok(json!({"controller":controller.agent,"executor":executor.agent,"controllerDesignedFromChat":true,"delegated":false,"controllerProjectToolCalls":0,"projectUnchanged":true,"reply":reply,"sessionId":root.id}))
    }.await;
    if result.is_err() { crate::commands::interrupt_turn(root.id.clone(),app.clone()).await?; }
    result
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
    let controller_agent=std::env::var("SUPERCODE_LINKAGE_TEST_CONTROLLER").unwrap_or_else(|_|"claude".into());
    let controller = selected
        .iter()
        .find(|p| p.agent == controller_agent)
        .cloned()
        .ok_or("联动实测缺少指定主控的现有 API 连接；未修改本机配置")?;
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
