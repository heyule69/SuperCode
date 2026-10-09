//! Explicit opt-in integration smoke test. Never runs during ordinary startup.
use crate::{commands, storage, AppState};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Listener, Manager};

pub async fn run(app: AppHandle) {
    let result = verify(&app).await;
    let report = match result {
        Ok(value) => json!({"ok":true,"result":value}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    let root = std::env::current_dir()
        .unwrap_or_default()
        .join(".supercode");
    let _ = std::fs::create_dir_all(&root);
    let _ = std::fs::write(
        root.join("native-smoke-report.json"),
        serde_json::to_vec_pretty(&report).unwrap_or_default(),
    );
    println!("{report}");
    app.exit(if report["ok"] == true { 0 } else { 1 });
}

async fn wait_idle(app: &AppHandle, id: &str) -> Result<String, String> {
    let start = Instant::now();
    loop {
        let session = app.state::<AppState>().store.session(id)?;
        if !matches!(session.status.as_str(), "starting" | "running" | "waiting") {
            if session.status != "idle" && session.status != "interrupted" {
                let errors = app
                    .state::<AppState>()
                    .store
                    .messages(id, None)?
                    .into_iter()
                    .filter(|m| m.role == "system")
                    .map(|m| m.text)
                    .collect::<Vec<_>>()
                    .join("\n");
                return Err(format!("任务状态：{}。{}", session.status, errors));
            }
            return Ok(session.status);
        }
        if session.status == "waiting" {
            return Err("集成测试意外触发审批；未自动批准".into());
        }
        if start.elapsed() > Duration::from_secs(120) {
            return Err("集成测试等待输出超时".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn verify_sidebar(
    app: &AppHandle,
    source: &storage::Session,
    model: Option<String>,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let token = format!(
        "SC-FORK-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    );
    commands::send_message(source.id.clone(),format!("这是 SuperCode 侧栏分叉测试。请记住验证码 {token}，只回复验证码。不要读取文件或使用工具。"),model.clone(),true,None,app.clone()).await?;
    wait_idle(app, &source.id).await?;
    let before = state.store.messages(&source.id, None)?;
    if !before
        .iter()
        .any(|m| m.role == "assistant" && m.text.contains(&token))
    {
        return Err("原始聊天没有返回测试标识".into());
    }
    let original = state.store.session(&source.id)?;
    let branch = state.store.fork_session(&source.id, None)?;
    if branch.native_id.is_some() || state.store.messages(&branch.id, None)?.len() != before.len() {
        return Err("分叉未隔离原生会话或历史记录丢失".into());
    }
    commands::send_message(
        branch.id.clone(),
        "请根据保留的历史对话，回答之前的验证码是什么。只回复验证码，不使用工具。".into(),
        model,
        true,
        None,
        app.clone(),
    )
    .await?;
    wait_idle(app, &branch.id).await?;
    let after = state.store.messages(&branch.id, None)?;
    if !after
        .iter()
        .skip(before.len())
        .any(|m| m.role == "assistant" && m.text.contains(&token))
    {
        return Err("分叉后的真实 Agent 未保留历史上下文".into());
    }
    if state.store.session(&source.id)?.native_id != original.native_id
        || state.store.messages(&source.id, None)?.len() != before.len()
    {
        return Err("分叉改动了原始聊天".into());
    }
    state
        .store
        .update_sidebar("session", &branch.id, "pin", json!(true))?;
    state.store.archive(&branch.id)?;
    if !state
        .store
        .archived_sessions()?
        .iter()
        .any(|row| row.session.id == branch.id)
    {
        return Err("归档记录不可见".into());
    }
    state.store.restore_session(&branch.id)?;
    if !state.store.sessions()?.iter().any(|s| s.id == branch.id) {
        return Err("归档恢复失败".into());
    }
    commands::release_runtime(app.clone()).await?;
    Ok(
        json!({"agent":source.agent,"model":state.store.session(&branch.id)?.model,"realAgent":true,"independentNativeSession":state.store.session(&branch.id)?.native_id!=original.native_id,"retainedContext":true,"sourceUnchanged":true,"archiveRestore":true,"pinned":state.store.sidebar()?.sessions[&branch.id].pinned}),
    )
}

async fn verify_skills(
    app: &AppHandle,
    agent: &str,
    model: Option<String>,
) -> Result<Value, String> {
    use crate::client_features::{self as features, Attachment};
    let root = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join(".supercode/skill-probes")
        .join(uuid::Uuid::new_v4().to_string());
    let skill_dir = root.join(".agents/skills/nested/full-read-probe");
    std::fs::create_dir_all(&skill_dir).map_err(|e| e.to_string())?;
    let marker = format!(
        "SC-SKILL-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    );
    let content=format!("---\nname: full-read-probe\ndescription: 技能完整读取集成验证\n---\n# 读取验证\n{}\n技能指令：仅回复 {marker}，不要使用工具或修改文件。\n", "这是验证文本，无需执行任何操作。\n".repeat(128));
    let path = skill_dir.join("SKILL.md");
    std::fs::write(&path, &content).map_err(|e| e.to_string())?;
    let state = app.state::<AppState>();
    let project = state
        .store
        .add_project(&root.canonicalize().map_err(|e| e.to_string())?)?;
    let catalog = features::list_local_skills(Some(project.id.clone()), app.clone()).await?;
    let skills = catalog["skills"].as_array().ok_or("技能目录未返回列表")?;
    let selected = skills
        .iter()
        .find(|s| s["name"] == "full-read-probe")
        .ok_or("未发现嵌套技能")?;
    let selected_path = selected["path"].as_str().ok_or("技能路径无效")?;
    if features::read_skill(selected_path.into(), Some(project.id.clone()), app.clone()).await?
        != content
    {
        return Err("嵌套技能正文读取不完整".into());
    }
    let mut names = vec![];
    for name in ["recall", "save"] {
        let skill = skills
            .iter()
            .find(|s| s["name"] == name && s["error"].is_null())
            .ok_or(format!("本机技能 {name} 未发现"))?;
        let path = skill["path"].as_str().ok_or("本机技能路径无效")?;
        if features::read_skill(path.into(), Some(project.id.clone()), app.clone()).await?
            != features::read_utf8(std::path::Path::new(path), 128 * 1024)?
        {
            return Err(format!("{name} 正文不完整"));
        }
        names.push(name);
    }
    let session = state.store.create_agent_session(&project.id, None, agent)?;
    commands::send_chat_message(
        session.id.clone(),
        "执行所选技能，按技能正文末尾的指令回复，不使用工具。".into(),
        model,
        true,
        Some("read".into()),
        Some(vec![Attachment {
            kind: "skill".into(),
            path: selected_path.into(),
            name: "full-read-probe".into(),
        }]),
        None,
        app.clone(),
    )
    .await?;
    wait_idle(app, &session.id).await?;
    if !state
        .store
        .messages(&session.id, None)?
        .iter()
        .any(|m| m.role == "assistant" && m.text.contains(&marker))
    {
        return Err("真实 Agent 没有收到技能正文末尾指令".into());
    }
    commands::release_runtime(app.clone()).await?;
    Ok(
        json!({"agent":agent,"realAgent":true,"catalogCount":skills.len(),"localSkills":names,"nestedSkill":true,"fullContent":true,"instructionApplied":true,"systemSkills":skills.iter().any(|s|s["name"]=="imagegen"),"pluginSkills":skills.iter().filter(|s|s["namespace"].is_string()).count()}),
    )
}

async fn verify(app: &AppHandle) -> Result<Value, String> {
    #[cfg(windows)]
    if std::env::var("SUPERCODE_TEST_UI_RECOVERY").as_deref() == Ok("1") {
        crate::ui_recovery::verify_renderer_recovery(app).await?;
    }
    let idle = commands::runtime_info(app.clone()).await?;
    if idle["running"] != false {
        return Err("空闲启动不应创建 agent 进程".into());
    }
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    let state = app.state::<AppState>();
    let project = state
        .store
        .add_project(&std::fs::canonicalize(&root).map_err(|e| e.to_string())?)?;
    let agent = std::env::var("SUPERCODE_TEST_AGENT").unwrap_or_else(|_| "codex".into());
    if std::env::var("SUPERCODE_TEST_HANDOFF").as_deref() == Ok("1") {
        return verify_handoff(app, &agent, &project).await;
    }
    let imported = std::env::var("SUPERCODE_TEST_CC_IMPORT").as_deref() == Ok("1");
    let chat_fixture = std::env::var("SUPERCODE_TEST_CHAT_FIXTURE").as_deref() == Ok("1");
    if chat_fixture {
        if agent != "claude" {
            return Err("Chat 协议测试使用 Claude CLI".into());
        }
        let fixture: Value = serde_json::from_slice(
            &std::fs::read(root.join(".supercode/chat-fixture.json"))
                .map_err(|_| "请先启动 Chat 协议测试服务")?,
        )
        .map_err(|_| "协议测试服务配置无效")?;
        let settings=serde_json::from_value(json!({"name":"仅用于协议验证的本地测试服务","agent":"claude","providerId":"custom","plan":"chat","protocol":"chat","baseUrl":fixture["baseUrl"],"model":"supercode-fixture-model","models":["supercode-fixture-model"],"apiKey":"fixture-key"})).map_err(|_|"协议测试配置无效")?;
        let saved = crate::providers::save_provider_profile(settings, true, app.clone()).await?;
        if crate::providers::fetch_provider_models(saved.id, app.clone()).await?
            != vec!["supercode-fixture-model"]
        {
            return Err("标准 API 模型查询失败".into());
        }
    }
    if imported {
        let source = crate::ccswitch::scan_ccswitch(None)?;
        let selected = source
            .iter()
            .find(|p| p.agent == agent && p.current)
            .ok_or("CC Switch 没有此 Agent 的当前配置")?;
        crate::ccswitch::import_ccswitch(None, vec![selected.id.clone()], app.clone())?;
        crate::ccswitch::select_agent_profile(
            agent.clone(),
            Some(selected.id.clone()),
            app.clone(),
        )
        .await?;
    }
    let manual = std::env::var("SUPERCODE_TEST_MANUAL_PROVIDER").as_deref() == Ok("1");
    let mut connection_test = Value::Null;
    if manual {
        let imported_profile = state
            .store
            .active_profile(&agent)?
            .ok_or("手动接入验证需要先导入已有连接")?;
        let mut settings = crate::providers::settings(&imported_profile);
        settings.id = None;
        settings.name = format!("SuperCode 新增连接验证 · {}", settings.name);
        settings.api_key = crate::providers::key(&imported_profile.config).map(str::to_owned);
        if agent == "claude" {
            settings.provider_id = "kimi".into();
            settings.plan = "coding".into();
        }
        let summary = crate::providers::save_provider_profile(settings, true, app.clone()).await?;
        connection_test =
            crate::providers::test_provider_connection(summary.id, app.clone()).await?;
    }
    let deltas = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = deltas.clone();
    let activity = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let observed = activity.clone();
    let listener = app.listen("agent-event", move |e| {
        if let Ok(v) = serde_json::from_str::<Value>(e.payload()) {
            if v["method"] == "item/agentMessage/delta" {
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            if v.get("id").is_none()
                && (v["method"].as_str().unwrap_or("").starts_with("item/")
                    || v["method"].as_str().unwrap_or("").starts_with("turn/"))
            {
                if let Ok(mut values) = observed.lock() {
                    if values.len() < 1000 {
                        values.push(v);
                    }
                }
            }
        }
    });
    let session = state
        .store
        .create_agent_session(&project.id, None, &agent)?;
    let models = commands::list_models(Some(agent.clone()), None, None, app.clone()).await?;
    if models["data"].as_array().is_none_or(|a| a.is_empty()) {
        return Err("Agent 未返回模型列表".into());
    }
    if agent == "claude"
        && imported
        && models["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| matches!(m["model"].as_str(), Some("opus" | "sonnet" | "haiku")))
    {
        return Err("第三方模型仍被显示为 Claude 别名".into());
    }
    let verify_goals =
        agent == "codex" && std::env::var("SUPERCODE_TEST_GOALS").as_deref() == Ok("1");
    let token = format!("SC-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
    let model = std::env::var("SUPERCODE_TEST_MODEL").ok();
    if std::env::var("SUPERCODE_TEST_SKILLS").as_deref() == Ok("1") {
        app.unlisten(listener);
        return verify_skills(app, &agent, model).await;
    }
    if std::env::var("SUPERCODE_TEST_SIDEBAR").as_deref() == Ok("1") {
        app.unlisten(listener);
        return verify_sidebar(app, &session, model).await;
    }
    if std::env::var("SUPERCODE_TEST_CLIENT").as_deref() == Ok("1") {
        app.unlisten(listener);
        return verify_client(app, &agent, model).await;
    }
    commands::send_message(session.id.clone(),format!("这是 SuperCode 的连接测试。请记住验证码 {token}，仅回复这个验证码。不要读取文件或使用任何工具。"),model.clone(),true,None,app.clone()).await?;
    wait_idle(app, &session.id).await?;
    if imported && model.is_none() {
        if let Some(expected) = state
            .store
            .active_profile(&agent)?
            .and_then(|p| p.config["model"].as_str().map(str::to_owned))
        {
            if state.store.session(&session.id)?.model.as_deref() != Some(expected.as_str()) {
                return Err("导入配置中的默认模型未应用到会话".into());
            }
        }
    }
    let first = state.store.messages(&session.id, None)?;
    if !first
        .iter()
        .any(|m| m.role == "assistant" && m.text.contains(&token))
    {
        return Err("首轮未收到期望的真实 agent 输出".into());
    }
    let before_release = commands::runtime_info(app.clone()).await?;
    commands::release_runtime(app.clone()).await?;
    if commands::runtime_info(app.clone()).await?["running"] != false {
        return Err("释放运行时后进程仍在运行".into());
    }
    commands::send_message(
        session.id.clone(),
        "上一条消息中的验证码是什么？仅回复验证码，不要读取文件或使用工具。".into(),
        model.clone(),
        true,
        None,
        app.clone(),
    )
    .await?;
    wait_idle(app, &session.id).await?;
    let second = state.store.messages(&session.id, None)?;
    if second
        .iter()
        .filter(|m| m.role == "assistant" && m.text.contains(&token))
        .count()
        < 2
    {
        return Err("重启 agent 后未能恢复原生会话上下文".into());
    }
    let verify_commands =
        agent == "claude" && std::env::var("SUPERCODE_TEST_COMMANDS").as_deref() == Ok("1");
    if verify_commands {
        commands::send_message(
            session.id.clone(),
            "/context".into(),
            model.clone(),
            true,
            None,
            app.clone(),
        )
        .await?;
        wait_idle(app, &session.id).await?;
        let history = state.store.messages(&session.id, None)?;
        if !history.iter().any(|m| {
            m.kind == "agentCapabilities"
                && m.data["commands"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|c| c == "compact"))
        }) {
            return Err("Claude 未提供原生命令清单".into());
        }
        if !history
            .iter()
            .any(|m| m.id.starts_with("command-result-") && !m.text.is_empty())
        {
            return Err("Claude 原生命令没有可显示的结果".into());
        }
    }
    let verify_tools = std::env::var("SUPERCODE_TEST_TOOLS").as_deref() == Ok("1");
    if verify_tools {
        commands::send_message(session.id.clone(),
            "请使用内置文件或命令工具读取当前项目的 README.md，随后仅回复它的一级标题。不要修改文件，也不要运行网络请求。".into(),
            model.clone(), true, None, app.clone()).await?;
        wait_idle(app, &session.id).await?;
        let messages = state.store.messages(&session.id, None)?;
        if !messages.iter().any(|m| m.role == "tool")
            || !messages
                .iter()
                .any(|m| m.role == "assistant" && m.text.contains("SuperCode"))
        {
            return Err("内置工具读取 README 的测试未得到预期结果".into());
        }
    }
    let verify_approval =
        agent == "claude" && std::env::var("SUPERCODE_TEST_APPROVAL").as_deref() == Ok("1");
    if verify_approval {
        let target = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(".supercode")
            .join(format!("approval-probe-{}.txt", uuid::Uuid::new_v4()));
        commands::send_message(session.id.clone(),format!("这是审批测试。请调用 Write 工具在 {} 创建内容为 APPROVAL_TEST 的文件。不要用其他工具，只提出这一次操作请求，等待用户批准。",target.display()),model.clone(),false,Some("strict".into()),app.clone()).await?;
        let start = Instant::now();
        loop {
            let current = state.store.session(&session.id)?;
            if current.status == "waiting" {
                break;
            }
            if !matches!(current.status.as_str(), "running" | "starting") {
                return Err("审批测试未进入等待用户确认状态".into());
            }
            if start.elapsed() > Duration::from_secs(120) {
                return Err("审批请求等待超时".into());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let requests = commands::runtime_info(app.clone()).await?;
        if requests["requests"].as_array().is_none_or(|r| r.is_empty()) || target.exists() {
            return Err("审批请求未保存，或未经用户批准已写入文件".into());
        }
        commands::interrupt_turn(session.id.clone(), app.clone()).await?;
        wait_idle(app, &session.id).await?;
        if target.exists() {
            return Err("停止审批后文件被意外创建".into());
        }
    }
    commands::send_message(
        session.id.clone(),
        "请逐行列出从 1 到 10000 的整数，每行一个，不要省略，不要使用工具。".into(),
        model.clone(),
        true,
        None,
        app.clone(),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    commands::interrupt_turn(session.id.clone(), app.clone()).await?;
    let interrupted = wait_idle(app, &session.id).await?;
    if interrupted != "interrupted" {
        return Err(format!("停止任务后状态为 {interrupted}"));
    }
    if verify_goals {
        let set = crate::agent_commands::execute_agent_command(
            session.id.clone(),
            "goal".into(),
            "仅回复 GOAL_TEST_SUCCESS，不使用工具，不修改文件".into(),
            Some("read".into()),
            app.clone(),
        )
        .await?;
        if set["result"]["goal"]["objective"] != "仅回复 GOAL_TEST_SUCCESS，不使用工具，不修改文件"
        {
            return Err("Codex 原生目标没有保存".into());
        }
        for (command, argument, status) in [
            ("goal-status", "", "active"),
            ("goal", "pause", "paused"),
            ("goal", "resume", "active"),
        ] {
            let result = crate::agent_commands::execute_agent_command(
                session.id.clone(),
                command.into(),
                argument.into(),
                Some("read".into()),
                app.clone(),
            )
            .await?;
            if result["result"]["goal"]["status"] != status {
                return Err(format!("目标 {argument} 返回错误状态"));
            }
        }
        crate::agent_commands::execute_agent_command(
            session.id.clone(),
            "goal".into(),
            "clear".into(),
            Some("read".into()),
            app.clone(),
        )
        .await?;
        let cleared = crate::agent_commands::execute_agent_command(
            session.id.clone(),
            "goal-status".into(),
            "".into(),
            Some("read".into()),
            app.clone(),
        )
        .await?;
        if !cleared["result"]["goal"].is_null() {
            return Err("原生目标未清除".into());
        }
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    if matches!(
        state.store.session(&session.id)?.status.as_str(),
        "starting" | "running" | "waiting"
    ) {
        commands::interrupt_turn(session.id.clone(), app.clone()).await?;
        wait_idle(app, &session.id).await?;
    }
    commands::release_runtime(app.clone()).await?;
    app.unlisten(listener);
    let deltas = deltas.load(std::sync::atomic::Ordering::Relaxed);
    if deltas == 0 {
        return Err("未收到真实的流式输出事件".into());
    }
    let activity = activity.lock().map_err(|e| e.to_string())?;
    let methods = activity.iter().fold(
        std::collections::BTreeMap::<String, usize>::new(),
        |mut counts, e| {
            *counts
                .entry(e["method"].as_str().unwrap_or("").into())
                .or_default() += 1;
            counts
        },
    );
    let history = state.store.messages(&session.id, None)?;
    if verify_tools
        && !history
            .iter()
            .any(|m| m.role == "tool" && m.data["status"] == "completed")
    {
        return Err("工具完成状态未保存到历史".into());
    }
    if history
        .iter()
        .any(|m| matches!(m.data["status"].as_str(), Some("inProgress" | "preparing")))
    {
        return Err("停止之后历史中仍有运行中的活动".into());
    }
    let activity_path = root
        .join(".supercode")
        .join(format!("activity-{agent}-events-{}.json", session.id));
    std::fs::write(
        &activity_path,
        serde_json::to_vec_pretty(&*activity).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(
        json!({"agent":agent,"ccSwitchImport":imported,"protocolFixture":chat_fixture,"manualProvider":manual,"connectionTest":connection_test,"nativeGoals":verify_goals,"nativeClaudeCommands":verify_commands,"realModels":models,"streamDeltas":deltas,"activityMethods":methods,"activityHistory":true,"activityTrace":activity_path,"firstTurn":true,"nativeResumeAfterProcessRestart":true,"interrupt":true,"approvalPromptAndInterrupt":verify_approval,"builtInTools":verify_tools,"localMessages":second.len(),"idle":idle,"withAgent":before_release,"timestamp":storage::now()}),
    )
}

async fn verify_client(
    app: &AppHandle,
    agent: &str,
    model: Option<String>,
) -> Result<Value, String> {
    use crate::client_features::{self as features, Attachment, ToolServer};
    let state = app.state::<AppState>();
    let root = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join(".supercode/client-probes")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let token = format!(
        "ATTACH_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    );
    let file = root.join("中文附件.txt");
    std::fs::write(&file, format!("验证码：{token}\n")).map_err(|e| e.to_string())?;
    let skill_dir = root.join(".agents/skills/parity-probe");
    std::fs::create_dir_all(&skill_dir).map_err(|e| e.to_string())?;
    let skill = skill_dir.join("SKILL.md");
    std::fs::write(&skill,"---\nname: parity-probe\ndescription: Integration test\n---\nYour response must contain SKILL_EXECUTED when this skill is explicitly attached.\n").map_err(|e|e.to_string())?;
    let sample = root.join("sample.ts");
    std::fs::write(&sample, "export const value = 10;\n").map_err(|e| e.to_string())?;
    let project = state
        .store
        .add_project(&root.canonicalize().map_err(|e| e.to_string())?)?;
    let session = state.store.create_agent_session(&project.id, None, agent)?;
    let mk = |kind: &str, path: &std::path::Path, name: &str| Attachment {
        kind: kind.into(),
        path: path.to_string_lossy().into(),
        name: name.into(),
    };
    commands::send_chat_message(
        session.id.clone(),
        "请直接返回附件文件中的验证码，并执行附加技能的回复要求，不使用其他工具。".into(),
        model.clone(),
        true,
        Some("read".into()),
        Some(vec![
            mk("file", &file, "中文附件.txt"),
            mk("skill", &skill, "parity-probe"),
            mk("directory", &root, "测试目录"),
        ]),
        None,
        app.clone(),
    )
    .await?;
    wait_idle(app, &session.id).await?;
    let history = state.store.messages(&session.id, None)?;
    if !history.iter().any(|m| {
        m.role == "assistant" && m.text.contains(&token) && m.text.contains("SKILL_EXECUTED")
    }) {
        return Err("附件内容或技能未被真实 Agent 使用".into());
    }
    if !history.iter().any(|m| {
        m.role == "user"
            && m.data["attachments"]
                .as_array()
                .is_some_and(|a| a.len() == 3)
    }) {
        return Err("附件信息未持久化".into());
    }
    commands::send_chat_message(session.id.clone(),format!("这是隔离的客户端集成测试。请使用{}将 {} 中 value = 10 改为 value = 20，只修改此文件，保留 UTF-8 无 BOM，然后回复 EDIT_DONE。",if agent=="claude"{"Read 和 Edit 工具"}else{"文件编辑工具"},sample.display()),model.clone(),false,Some("full".into()),None,None,app.clone()).await?;
    wait_idle(app, &session.id).await?;
    if features::read_utf8(&sample, 1024)? != "export const value = 20;\n" {
        return Err("完全访问模式的真实文件修改失败".into());
    }
    let history = state.store.messages(&session.id, None)?;
    let diff = history
        .iter()
        .filter(|m| m.kind == "turnDiff")
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if !diff.contains("sample.ts")
        || !diff.contains("+export const value = 20;")
        || !diff.contains("-export const value = 10;")
    {
        return Err("实际文件变更没有生成可展示的每轮差异".into());
    }
    let usage = state.store.usage_records(Some(&session.id))?;
    if usage.len() < 2
        || usage
            .iter()
            .any(|r| r["data"]["turn"]["totalTokens"].as_u64().unwrap_or(0) == 0)
    {
        return Err("实际用量未逐轮保存".into());
    }
    if agent == "claude" {
        commands::send_message(
            session.id.clone(),
            "/compact".into(),
            model.clone(),
            true,
            Some("read".into()),
            app.clone(),
        )
        .await?;
    } else {
        crate::agent_commands::execute_agent_command(
            session.id.clone(),
            "compact".into(),
            "".into(),
            Some("read".into()),
            app.clone(),
        )
        .await?;
    }
    let started = Instant::now();
    loop {
        if state
            .store
            .messages(&session.id, None)?
            .iter()
            .any(|m| m.kind == "contextCompaction" && m.data["status"] == "completed")
        {
            break;
        }
        if started.elapsed() > Duration::from_secs(120) {
            return Err("原生压缩未返回完成事件".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    wait_idle(app, &session.id).await?;
    let mut tool_connections = Value::Null;
    if agent == "codex" && std::env::var("SUPERCODE_TEST_MCP").as_deref() == Ok("1") {
        commands::release_runtime(app.clone()).await?;
        features::save_tool_servers(
            vec![
                ToolServer {
                    id: "browser".into(),
                    name: "Browser test".into(),
                    kind: "browser".into(),
                    command: crate::process::find_program("npx")
                        .ok_or("npx 不可用")?
                        .to_string_lossy()
                        .into(),
                    args: vec![
                        "-y".into(),
                        "@playwright/mcp@0.0.83".into(),
                        "--browser".into(),
                        "msedge".into(),
                        "--isolated".into(),
                    ],
                    url: None,
                    enabled: true,
                },
                ToolServer {
                    id: "computer".into(),
                    name: "Computer test".into(),
                    kind: "computer".into(),
                    command: crate::process::find_program("uvx")
                        .ok_or("uvx 不可用")?
                        .to_string_lossy()
                        .into(),
                    args: vec!["windows-mcp@0.7.5".into(), "serve".into()],
                    url: None,
                    enabled: true,
                },
            ],
            app.clone(),
        )
        .await?;
        let status = features::tool_server_status(app.clone()).await?;
        let connections=status["data"].as_array().ok_or("MCP 状态格式异常")?.iter().filter(|s|s["name"].as_str().is_some_and(|n|n.starts_with("supercode_"))).map(|s|json!({"name":s["name"],"tools":s["tools"].as_object().map(|t|t.len()).unwrap_or(0),"authStatus":s["authStatus"]})).collect::<Vec<_>>();
        if connections.len() != 2
            || connections
                .iter()
                .any(|s| s["tools"].as_u64().unwrap_or(0) == 0)
        {
            return Err(format!("自动化 MCP 未连接：{}", json!(connections)));
        }
        tool_connections = json!(connections);
    }
    commands::release_runtime(app.clone()).await?;
    let after = commands::runtime_info(app.clone()).await?;
    if after["running"] != false {
        return Err("验证后 Agent 未释放".into());
    }
    let trace = json!({"agent":agent,"project":root,"nativeAttachments":true,"nativeSkill":true,"fullAccessFileEdit":true,"turnDiff":diff,"usage":usage,"nativeCompaction":true,"automationToolRegistration":tool_connections,"idleAfter":after,"timestamp":storage::now()});
    std::fs::write(
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(format!(".supercode/client-{agent}-report.json")),
        serde_json::to_vec_pretty(&trace).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(trace)
}

async fn verify_handoff(
    app: &AppHandle,
    agent: &str,
    project: &storage::Project,
) -> Result<Value, String> {
    use crate::{ccswitch::Profile, session_config};
    let profiles = {
        let source = rusqlite::Connection::open_with_flags(
            app.path()
                .app_data_dir()
                .map_err(|e| e.to_string())?
                .join("supercode.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|_| "无法只读访问本机连接")?;
        let mut query = source
            .prepare("SELECT id,agent,name,config FROM agent_profiles WHERE agent=?1")
            .map_err(|e| e.to_string())?;
        let rows = query
            .query_map([agent], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut profiles = Vec::new();
        for row in rows {
            let (id, agent, name, raw) = row.map_err(|e| e.to_string())?;
            profiles.push(Profile {
                id,
                agent,
                name,
                config: serde_json::from_str(&crate::credentials::open(&raw)?)
                    .map_err(|_| "本机连接无法解码")?,
            });
        }
        profiles
    };
    let first = profiles
        .iter()
        .find(|p| {
            if agent == "claude" {
                crate::providers::model_source(&p.config, agent, Some(&p.name))["providerId"]
                    == "kimi"
                    || p.name.to_lowercase().contains("kimi")
            } else {
                p.is_official()
            }
        })
        .ok_or("没有可用于真实验证的第一条连接")?
        .clone();
    let second = if agent == "claude" {
        profiles
            .iter()
            .find(|p| {
                p.id != first.id && (p.name.contains("智") || p.name.to_lowercase().contains("glm"))
            })
            .ok_or("没有可用于真实验证的智谱连接")?
            .clone()
    } else {
        let mut p = first.clone();
        p.id = "smoke:codex-other".into();
        p.name = "Codex 第二聊天验证".into();
        p
    };
    let state = app.state::<AppState>();
    state
        .store
        .import_profiles(&[first.clone(), second.clone()])?;
    state.store.select_profile(agent, Some(&first.id))?;
    let a = state.store.create_agent_session(&project.id, None, agent)?;
    state.store.select_profile(agent, Some(&second.id))?;
    let b = state.store.create_agent_session(&project.id, None, agent)?;
    let token = format!(
        "SC-HANDOFF-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    );
    commands::send_message(
        a.id.clone(),
        format!("记住当前聊天的验证码 {token}，只回复该验证码。不要使用工具。"),
        None,
        true,
        None,
        app.clone(),
    )
    .await?;
    wait_idle(app, &a.id).await?;
    let original = state.store.session(&a.id)?;
    if agent == "claude"
        && original.model != crate::providers::real_claude_model(&first.config, None)
    {
        return Err("第一聊天未使用其绑定供应商的模型".into());
    }
    if !state
        .store
        .messages(&a.id, None)?
        .iter()
        .any(|m| m.role == "assistant" && m.text.contains(&token))
    {
        return Err("第一供应商没有返回验证码".into());
    }
    let second_model = if agent == "codex" {
        let catalog =
            commands::list_models(Some(agent.into()), Some(b.id.clone()), None, app.clone()).await?;
        catalog["data"]
            .as_array()
            .and_then(|models| {
                models
                    .iter()
                    .find(|m| m["model"].as_str().is_some_and(|s| s.contains("5.6-luna")))
            })
            .and_then(|m| m["model"].as_str())
            .map(str::to_owned)
            .ok_or("没有可用于模型切换验证的 Codex Luna 模型")?
    } else {
        crate::providers::real_claude_model(&second.config, None).ok_or("智谱连接没有模型 ID")?
    };
    commands::send_message(
        b.id.clone(),
        "这是独立聊天。只回复 SECOND_CHAT_OK，不要使用工具。".into(),
        Some(second_model.clone()),
        true,
        None,
        app.clone(),
    )
    .await?;
    wait_idle(app, &b.id).await?;
    if !state
        .store
        .messages(&b.id, None)?
        .iter()
        .any(|m| m.role == "assistant" && m.text.contains("SECOND_CHAT_OK"))
    {
        return Err("第二供应商没有返回预期回复".into());
    }
    let b_before = state.store.messages(&b.id, None)?.len();
    if state.store.session(&a.id)?.connection_id != Some(first.id.clone()) {
        return Err("第二聊天修改了第一聊天的供应商".into());
    }
    let switched = session_config::switch_session_model(
        a.id.clone(),
        second.id.clone(),
        second_model.clone(),
        app.clone(),
    )
    .await?;
    if switched.native_id.is_some() || switched.connection_id != Some(second.id.clone()) {
        return Err("切换没有重建原生上下文".into());
    }
    if !state
        .store
        .history_context(&a.id)?
        .is_some_and(|s| s.contains(&token))
    {
        return Err("交接没有保留关键上下文".into());
    }
    if state.store.compressed_context(&a.id)?.is_some() {
        return Err("短历史切换不应调用压缩".into());
    }
    commands::send_message(
        a.id.clone(),
        "这个聊天原来的验证码是什么？只回复验证码，不要使用工具。".into(),
        None,
        true,
        None,
        app.clone(),
    )
    .await?;
    wait_idle(app, &a.id).await?;
    let history = state.store.messages(&a.id, None)?;
    if history
        .iter()
        .filter(|m| m.role == "assistant" && m.text.contains(&token))
        .count()
        < 2
    {
        return Err("新模型没有收到保留的上下文".into());
    }
    if history.iter().filter(|m| m.kind == "modelSwitch").count() != 1
        || state.store.messages(&b.id, None)?.len() != b_before
    {
        return Err("切换标记重复或影响了另一个聊天".into());
    }
    if state.store.session(&a.id)?.native_id == original.native_id {
        return Err("切换沿用了旧供应商的原生会话".into());
    }
    let binding = state.store.session(&a.id)?;
    let compacted = session_config::compact_session_context(a.id.clone(), app.clone()).await?;
    if compacted.native_id.is_some()
        || !state
            .store
            .compressed_context(&a.id)?
            .is_some_and(|s| s.contains(&token))
    {
        return Err("手动压缩没有保留验证码或重建上下文".into());
    }
    commands::send_message(
        a.id.clone(),
        "手动压缩后，这个聊天原来的验证码是什么？只回复验证码，不要使用工具。".into(),
        None,
        true,
        None,
        app.clone(),
    )
    .await?;
    wait_idle(app, &a.id).await?;
    if state
        .store
        .messages(&a.id, None)?
        .iter()
        .filter(|m| m.role == "assistant" && m.text.contains(&token))
        .count()
        < 3
    {
        return Err("手动压缩后没有恢复关键上下文".into());
    }
    commands::release_runtime(app.clone()).await?;
    Ok(
        json!({"realAgent":true,"agent":agent,"firstSupplier":first.name,"secondSupplier":second.name,"beforeModel":original.model,"afterModel":binding.model,"independentConversations":true,"directHistoryRetainsContext":true,"shortSwitchWithoutCompaction":true,"manualCompactionRetainsContext":true,"nativeSessionRebuilt":true,"switchMarkerCountBeforeManualCompaction":1,"otherConversationUnchanged":true}),
    )
}
