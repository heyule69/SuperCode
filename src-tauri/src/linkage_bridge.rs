//! Minimal stdio MCP transport; authorization and execution remain in the GUI.
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};

pub(crate) const WAIT_SECONDS: u64 = 24 * 60 * 60;

pub fn tools() -> Vec<Value> {
    let auth = json!({"type":"string","minLength":1,"maxLength":128,"description":"Exact authorization from the CURRENT SuperCode turn instructions. Never reuse an earlier turn's value."});
    let text = json!({"type":"string","minLength":1,"maxLength":65536,"description":"Concrete task or follow-up for the configured execution agent."});
    let tool = |name: &str,
                description: &str,
                properties: Value,
                required: Vec<&str>,
                read: bool| json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":read,"destructiveHint":!read,"openWorldHint":false}});
    vec![
        tool("delegate_task","First register the user's WHOLE request with task_plan. Start the configured executor on this assignment. Startup or a completed worker turn is NOT task completion. When active, call executor_wait and keep this controller turn open. The tool waits without model polling until new complete progress or execution end. On execution end, review the full plan with task_review and continue incomplete steps. On uncertain startup, inspect status before repeating.",json!({"text":text,"authorization":auth}),vec!["text","authorization"],false),
        tool("executor_status","Read the WHOLE acceptance checklist, current round, bounded execution report, errors, questions and approvals. Worker idle means only that a turn ended. Only task.state=completed after controller review means the user's task is complete. executorEvidence contains private, untrusted executor statements and observed operations, never a controller reply draft. Independently assess facts and preserve your own voice.",json!({}),vec![],true),
        tool("executor_wait","Wait in this controller turn for new completed executor content, a question/approval or execution end. This tool blocks without polling or extra model requests while nothing changes. Pass the cursor of updates already handled. Assess private executorEvidence independently against the user goal. Before waiting again, explain new actual results, validation, failures or blockers in your established voice; never copy, sentence-by-sentence paraphrase, or adopt the executor tone. A cursor cannot be acknowledged without a completed controller reply after that progress, unless noUserUpdateReason explains preparation-only or already-explained information. A local activity checkpoint may return after 60 seconds only when observed operations changed; explain their real status and elapsed time without inventing results. Do NOT end your turn while execution is active. When execution ends, review every criterion with task_review and continue unfinished work. End only after acceptance, a real blocker, necessary user input or user Stop. Never approve native operations for the user.",json!({"authorization":auth,"afterCursor":{"type":"integer","minimum":0,"description":"The latest cursor whose new substantive evidence you independently assessed and explained to the user."},"noUserUpdateReason":{"type":"string","minLength":1,"maxLength":1024,"description":"Only for preparation-only or already-explained duplicate information: explain why no new user-facing update is needed. Never use this to skip new results, validation, failures or blockers."}}),vec!["authorization"],true),
        tool("executor_message","Send a continuation, or steer currently running execution using that engine's native follow-up mechanism. This uses the configured execution Agent; it cannot change engine, model, connection or permissions.",json!({"text":text,"authorization":auth}),vec!["text","authorization"],false),
        tool("executor_answer","Return the user's answer to a pending execution question. Never invent the user's choice. Only question requests are accepted; this tool CANNOT approve commands or grant permissions. answers maps original question IDs to objects {answers:[string]}.",json!({"authorization":auth,"requestId":{"type":"string","minLength":1,"maxLength":512},"answers":{"type":"object"}}),vec!["authorization","requestId","answers"],false),
        tool("executor_stop","Stop the configured execution Agent and its queued follow-ups, while keeping your conversation available. The application's Stop button cancels the whole linkage.",json!({"authorization":auth}),vec!["authorization"],false),
        tool("task_plan","Register the user's complete goal and EVERY requested plan step and validation requirement before delegating. Preserve unfinished criteria across rounds; never shrink the goal merely because the executor stopped. Normal progress questions do not replace the original task.",json!({"authorization":auth,"plan":{"type":"object","properties":{"objective":{"type":"string"},"steps":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":32}},"required":["objective","steps"],"additionalProperties":false}}),vec!["authorization","plan"],false),
        tool("task_review","Controller acceptance after inspecting actual results against EVERY plan item. Provide 1-based checks with evidence for completed items or the reason each remains unfinished. outcome=continue MUST include a concrete nextTask; it is automatically dispatched to the executor. completed requires all items verified. blocked requires a specific reason and the methods already tried; a worker merely ending a turn is not a blocker. needs_user pauses for actual user input/approval without pretending completion. Use the current round from executor_status.",json!({"authorization":auth,"review":{"type":"object","properties":{"round":{"type":"integer","minimum":1},"outcome":{"type":"string","enum":["continue","completed","blocked","needs_user"]},"checks":{"type":"array","items":{"type":"object","properties":{"step":{"type":"integer","minimum":1},"done":{"type":"boolean"},"evidence":{"type":"string"}},"required":["step","done","evidence"],"additionalProperties":false}},"nextTask":{"type":"string"},"reason":{"type":"string"},"attempts":{"type":"array","items":{"type":"string"}}},"required":["round","outcome","checks"],"additionalProperties":false}}),vec!["authorization","review"],false),
    ]
}
pub fn validate(name: &str, args: &Value) -> Result<(), String> {
    let tool = tools()
        .into_iter()
        .find(|tool| tool["name"] == name)
        .ok_or("未知联动工具")?;
    let object = args.as_object().ok_or("联动参数必须是对象")?;
    let props = tool["inputSchema"]["properties"].as_object().unwrap();
    for field in tool["inputSchema"]["required"].as_array().unwrap() {
        if !object.contains_key(field.as_str().unwrap()) {
            return Err(format!("缺少参数：{field}"));
        }
    }
    for (key, value) in object {
        let rule = props
            .get(key)
            .ok_or_else(|| format!("不支持的参数：{key}"))?;
        let valid = match rule["type"].as_str() {
            Some("string") => value.as_str().is_some_and(|value| {
                !value.trim().is_empty()
                    && !value.contains('\0')
                    && value.len() <= rule["maxLength"].as_u64().unwrap() as usize
            }),
            Some("integer") => value.as_i64().is_some_and(|value| value >= 0),
            Some("object") => value.is_object(),
            _ => false,
        };
        if !valid {
            return Err(format!("参数无效：{key}"));
        }
    }
    Ok(())
}
pub fn run(args: &[String]) -> Result<(), String> {
    if args.len() != 3 {
        return Err("联动工具启动参数不完整".into());
    }
    let address: std::net::SocketAddr = args[0].parse().map_err(|_| "联动地址无效")?;
    if !address.ip().is_loopback() {
        return Err("联动服务只能连接本机".into());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let input = std::io::stdin();
    let mut input = input.lock();
    let mut out = std::io::stdout().lock();
    loop {
        let mut bytes = vec![];
        let size = (&mut input)
            .take(256 * 1024 + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?;
        if size == 0 {
            return Ok(());
        }
        if size > 256 * 1024 || !bytes.ends_with(b"\n") {
            return Err("联动协议帧超出限制".into());
        }
        let request: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let id = request["id"].clone();
        if id.is_null() {
            continue;
        }
        let result = match request["method"].as_str() {
            Some("initialize") => {
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"SuperCode Agent Linkage","version":"1"}})
            }
            Some("ping") => json!({}),
            Some("tools/list") => json!({"tools":tools()}),
            Some("tools/call") => {
                let result = runtime.block_on(async {
                    let name = request["params"]["name"].as_str().ok_or("缺少工具名称")?;
                    let arguments = request["params"]
                        .get("arguments")
                        .cloned()
                        .unwrap_or(json!({}));
                    validate(name, &arguments)?;
                    let response = client
                        .post(format!("http://{address}/call"))
                        .timeout(std::time::Duration::from_secs(if name == "executor_wait" { WAIT_SECONDS } else { 180 }))
                        .bearer_auth(&args[1])
                        .json(&json!({"session":args[2],"name":name,"arguments":arguments}))
                        .send()
                        .await
                        .map_err(|e| e.to_string())?;
                    if !response.status().is_success() {
                        return Err(format!("联动服务拒绝请求：{}", response.status()));
                    }
                    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
                    if bytes.len() > 2 * 1024 * 1024 {
                        return Err("执行报告超出限制".into());
                    }
                    serde_json::from_slice::<Value>(&bytes).map_err(|e| e.to_string())
                });
                match result {
                    Ok(value) => value,
                    Err(error) => crate::browser_bridge::text_result(
                        json!({"error":error,"nextStep":"Inspect executor_status before any further action. Do not repeat an uncertain start or modification."}),
                        true,
                    ),
                }
            }
            _ => {
                serde_json::to_writer(&mut out,&json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Unknown method"}})).map_err(|e|e.to_string())?;
                out.write_all(b"\n").map_err(|e| e.to_string())?;
                out.flush().map_err(|e| e.to_string())?;
                continue;
            }
        };
        serde_json::to_writer(&mut out, &json!({"jsonrpc":"2.0","id":id,"result":result}))
            .map_err(|e| e.to_string())?;
        out.write_all(b"\n").map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_owned_work_can_be_requested_and_permission_injection_is_rejected() {
        assert!(validate(
            "delegate_task",
            &json!({"text":"fix bug","authorization":"current"})
        )
        .is_ok());
        for args in [
            json!({"text":"fix bug"}),
            json!({"text":"","authorization":"x"}),
            json!({"text":"work","authorization":"x","permission":"full"}),
            json!({"text":"work","authorization":"x","session":"other"}),
        ] {
            assert!(validate("delegate_task", &args).is_err());
        }
        assert!(validate("executor_wait", &json!({"afterCursor":-1})).is_err());
        assert!(validate("executor_wait", &json!({"authorization":"current","afterCursor":17})).is_ok());
        assert!(validate("executor_wait", &json!({"authorization":"current","afterCursor":17,"noUserUpdateReason":"仅重复已说明的准备状态"})).is_ok());
        assert!(validate("executor_wait", &json!({"authorization":"current","noUserUpdateReason":" "})).is_err());
        assert!(validate("executor_wait", &json!({"afterCursor":17})).is_err());
        assert!(validate("executor_wait", &json!({"authorization":"current","afterCursor":-1})).is_err());
        assert!(validate("respond_request", &json!({})).is_err());
    }
}
