use crate::AppState;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
pub fn goal_text(goal: &Value) -> String {
    if !goal.is_object() {
        return "当前会话没有目标。".into();
    }
    let status = match goal["status"].as_str().unwrap_or("") {
        "active" => "执行中",
        "paused" => "已暂停",
        "complete" => "已完成",
        "blocked" => "等待解决阻碍",
        "budgetLimited" => "已达到目标预算",
        "usageLimited" => "已达到用量限制",
        s => s,
    };
    format!(
        "目标：{}\n状态：{}\n已用 Token：{}",
        goal["objective"].as_str().unwrap_or(""),
        status,
        goal["tokensUsed"].as_u64().unwrap_or(0)
    )
}
#[tauri::command]
pub async fn execute_agent_command(
    session_id: String,
    command: String,
    argument: String,
    permission_mode: Option<String>,
    app: AppHandle,
) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let session = state.store.session(&session_id)?;
    let active = matches!(session.status.as_str(), "running" | "starting" | "waiting");
    let control = command == "goal-status"
        || (command == "goal"
            && matches!(
                argument.trim(),
                "" | "status" | "pause" | "resume" | "clear"
            ));
    if state.store.running()? > 0 && !(active && control) {
        return Err("请先停止或完成当前任务，再创建目标或压缩上下文".into());
    }
    let permission = permission_mode.as_deref().unwrap_or("ask");
    let (sandbox, approval, _) = crate::client_features::permission(permission)?;
    let reviewer = crate::client_features::approvals_reviewer(permission);
    if session.agent != "codex" {
        return Err("此原生命令只适用于 Codex，Claude 的界面命令请使用 /help 查看".into());
    }
    let project = state.store.session_workspace(&session)?;
    let client = state
        .runtime
        .get_for_session(&app, Some(&session_id))
        .await?;
    let native = if let Some(id) = session.native_id {
        if !active {
            client.request("thread/resume",json!({"threadId":id,"cwd":project.path,"approvalPolicy":approval,"approvalsReviewer":reviewer,"sandbox":sandbox,"model":session.model,"modelProvider":state.store.route("codex",Some(&session_id))?.provider()})).await?;
        }
        id
    } else {
        let result=client.request("thread/start",json!({"cwd":project.path,"approvalPolicy":approval,"approvalsReviewer":reviewer,"sandbox":sandbox,"model":session.model,"modelProvider":state.store.route("codex",Some(&session_id))?.provider(),"serviceName":"supercode"})).await?;
        let native = result["thread"]["id"]
            .as_str()
            .ok_or("Codex 没有返回会话 ID")?
            .to_owned();
        state.store.bind_native(&session_id, &native)?;
        native
    };
    let result = match command.as_str() {
        "goal-status" => {
            client
                .request("thread/goal/get", json!({"threadId":native}))
                .await?
        }
        "compact" => {
            client
                .request("thread/compact/start", json!({"threadId":native}))
                .await?
        }
        "goal" => match argument.trim() {
            "" | "status" => {
                client
                    .request("thread/goal/get", json!({"threadId":native}))
                    .await?
            }
            "clear" => {
                client
                    .request("thread/goal/clear", json!({"threadId":native}))
                    .await?
            }
            "pause" => {
                client
                    .request(
                        "thread/goal/set",
                        json!({"threadId":native,"status":"paused"}),
                    )
                    .await?
            }
            "resume" => {
                client
                    .request(
                        "thread/goal/set",
                        json!({"threadId":native,"status":"active"}),
                    )
                    .await?
            }
            objective => {
                if objective.len() > 128 * 1024 {
                    return Err("目标内容过长".into());
                }
                client
                    .request(
                        "thread/goal/set",
                        json!({"threadId":native,"objective":objective}),
                    )
                    .await?
            }
        },
        _ => return Err("未知原生命令，请使用 /help 查看可用命令".into()),
    };
    // Codex starts and continues goal turns itself. Never enqueue a duplicate UI turn.
    if command == "goal"
        && !matches!(
            argument.trim(),
            "" | "status" | "pause" | "resume" | "clear"
        )
        && session.title == "新会话"
    {
        state
            .store
            .rename(&session_id, &argument.chars().take(24).collect::<String>())?;
    }
    let text = if let Some(goal) = result.get("goal").filter(|g| !g.is_null()) {
        goal_text(goal)
    } else if command == "compact" {
        "已向 Codex 请求上下文压缩。".into()
    } else if argument == "clear" {
        "目标已清除。".into()
    } else {
        "当前会话没有目标。".into()
    };
    state.store.save_message(
        &format!("command-{}", uuid::Uuid::new_v4()),
        &session_id,
        "system",
        &text,
        "commandResult",
        &json!({"command":command,"result":result}),
    )?;
    Ok(json!({"text":text,"result":result,"session":state.store.session(&session_id)?}))
}
