//! On-demand in-process Chat Completions adapter for Claude's native tool loop.
//! Bound only to loopback, authenticated with a per-turn token, and stopped with that turn.
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    routing::post,
    Json, Router,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{collections::BTreeMap, convert::Infallible, sync::Arc};
use tokio::sync::watch;

pub struct Bridge {
    pub base_url: String,
    pub token: String,
    task: tokio::task::JoinHandle<()>,
    cancel: watch::Sender<bool>,
}
#[derive(Clone)]
struct Connection {
    config: Value,
    token: String,
    http: reqwest::Client,
    cancel: watch::Receiver<bool>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
        self.task.abort();
    }
}
impl Bridge {
    pub async fn start(config: Value) -> Result<Self, String> {
        crate::providers::validate_url(config["baseUrl"].as_str().ok_or("标准 API 缺少地址")?)?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "无法启动本地 API 适配器")?;
        let port = listener
            .local_addr()
            .map_err(|_| "无法读取本地 API 端口")?
            .port();
        let token = uuid::Uuid::new_v4().to_string();
        let (cancel, rx) = watch::channel(false);
        let state = Arc::new(Connection {
            config,
            token: token.clone(),
            http: crate::providers::client()?,
            cancel: rx,
        });
        let router = Router::new()
            .route("/v1/messages", post(messages))
            .route("/v1/messages/count_tokens", post(count_tokens))
            .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
            .with_state(state);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Ok(Self {
            base_url: format!("http://127.0.0.1:{port}"),
            token,
            task,
            cancel,
        })
    }
}
fn authorized(headers: &HeaderMap, state: &Connection) -> bool {
    headers.get("x-api-key").and_then(|v| v.to_str().ok()) == Some(&state.token)
        || headers.get("authorization").and_then(|v| v.to_str().ok())
            == Some(format!("Bearer {}", state.token).as_str())
}
fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({"type":"error","error":{"type":"api_error","message":message}})),
    )
        .into_response()
}
async fn count_tokens(
    State(state): State<Arc<Connection>>,
    headers: HeaderMap,
    Json(_): Json<Value>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(
            StatusCode::UNAUTHORIZED,
            "Unauthorized local adapter request",
        );
    }
    // No fabricated token count. Claude can use its own fallback when the provider lacks this endpoint.
    error(
        StatusCode::NOT_FOUND,
        "This Chat Completions provider does not expose Anthropic token counting",
    )
}
fn content_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.into();
    }
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}
pub fn to_chat(body: &Value) -> Result<Value, String> {
    let mut messages = Vec::new();
    if let Some(system) = body.get("system") {
        let text = content_text(system);
        if !text.is_empty() {
            messages.push(json!({"role":"system","content":text}));
        }
    }
    for message in body["messages"]
        .as_array()
        .ok_or("Messages array missing")?
    {
        let role = message["role"].as_str().unwrap_or("user");
        if let Some(text) = message["content"].as_str() {
            messages.push(json!({"role":role,"content":text}));
            continue;
        }
        let blocks = message["content"]
            .as_array()
            .ok_or("Unsupported message content")?;
        let mut text = Vec::new();
        let mut calls = Vec::new();
        let mut results = Vec::new();
        let mut thinking = String::new();
        for block in blocks {
            match block["type"].as_str().unwrap_or("") {
                "text" => text.push(json!({"type":"text","text":block["text"]})),
                "thinking" => thinking.push_str(block["thinking"].as_str().unwrap_or("")),
                "tool_use" => calls.push(json!({"id":block["id"],"type":"function","function":{"name":block["name"],"arguments":block["input"].to_string()}})),
                "tool_result" => results.push(json!({"role":"tool","tool_call_id":block["tool_use_id"],"content":content_text(&block["content"])})),
                "image" => {
                    let source=&block["source"];
                    let url=if source["type"]=="base64" {format!("data:{};base64,{}",source["media_type"].as_str().unwrap_or("image/png"),source["data"].as_str().unwrap_or(""))} else {source["url"].as_str().unwrap_or("").into()};
                    text.push(json!({"type":"image_url","image_url":{"url":url}}));
                }
                "redacted_thinking" => {},
                _ => return Err(format!("标准 API 适配暂不支持 {} 内容块",block["type"].as_str().unwrap_or("unknown"))),
            }
        }
        // Tool results must follow the assistant's calls before another user message.
        messages.extend(results);
        if !text.is_empty() || !calls.is_empty() {
            let content = if text.iter().all(|t| t["type"] == "text") {
                json!(text
                    .iter()
                    .filter_map(|t| t["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n"))
            } else {
                json!(text)
            };
            let mut value = json!({"role":role,"content":content});
            if !calls.is_empty() {
                value["tool_calls"] = json!(calls);
            }
            if !thinking.is_empty() {
                value["reasoning_content"] = json!(thinking);
            }
            messages.push(value);
        }
    }
    let mut out = json!({"model":body["model"],"messages":messages,"stream":body["stream"].as_bool().unwrap_or(false)});
    if let Some(max) = body["max_tokens"].as_u64() {
        out["max_tokens"] = json!(max.min(32768));
    }
    for field in ["temperature", "top_p"] {
        if let Some(v) = body.get(field) {
            out[field] = v.clone();
        }
    }
    if let Some(stop) = body
        .get("stop_sequences")
        .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
    {
        out["stop"] = stop.clone();
    }
    if let Some(tools) = body["tools"].as_array().filter(|a| !a.is_empty()) {
        out["tools"]=json!(tools.iter().map(|t| json!({"type":"function","function":{"name":t["name"],"description":t["description"].as_str().unwrap_or(""),"parameters":t["input_schema"]}})).collect::<Vec<_>>());
    }
    if let Some(choice) = body.get("tool_choice") {
        out["tool_choice"] = match choice["type"].as_str().unwrap_or("auto") {
            "tool" => json!({"type":"function","function":{"name":choice["name"]}}),
            "any" => json!("required"),
            "none" => json!("none"),
            _ => json!("auto"),
        };
    }
    Ok(out)
}
fn stop_reason(reason: &str) -> &str {
    match reason {
        "tool_calls" | "function_call" => "tool_use",
        "length" => "max_tokens",
        _ => "end_turn",
    }
}
fn from_chat(value: Value, model: &str) -> Result<Value, String> {
    if value.get("error").is_some() {
        return Err("供应商返回 API 错误".into());
    }
    let choice = &value["choices"][0];
    let m = &choice["message"];
    if !m.is_object() {
        return Err("供应商没有返回消息".into());
    }
    let mut blocks = Vec::new();
    if let Some(t) = m["reasoning_content"]
        .as_str()
        .or(m["reasoning"].as_str())
        .filter(|s| !s.is_empty())
    {
        blocks.push(json!({"type":"thinking","thinking":t,"signature":""}));
    }
    if let Some(t) = m["content"].as_str().filter(|s| !s.is_empty()) {
        blocks.push(json!({"type":"text","text":t}));
    }
    if let Some(calls) = m["tool_calls"].as_array() {
        for call in calls {
            let input = serde_json::from_str::<Value>(
                call["function"]["arguments"].as_str().unwrap_or("{}"),
            )
            .map_err(|_| "供应商返回无效工具参数")?;
            blocks.push(json!({"type":"tool_use","id":call["id"],"name":call["function"]["name"],"input":input}));
        }
    }
    Ok(
        json!({"id":value["id"].as_str().unwrap_or("supercode-message"),"type":"message","role":"assistant","model":value["model"].as_str().unwrap_or(model),"content":blocks,"stop_reason":stop_reason(choice["finish_reason"].as_str().unwrap_or("stop")),"stop_sequence":null,"usage":{"input_tokens":value["usage"]["prompt_tokens"].as_u64().unwrap_or(0),"output_tokens":value["usage"]["completion_tokens"].as_u64().unwrap_or(0)}}),
    )
}
fn sse(value: Value) -> Event {
    Event::default()
        .event(value["type"].as_str().unwrap_or("error"))
        .data(value.to_string())
}
#[derive(Default)]
struct Tool {
    index: usize,
    id: String,
    name: String,
    args: String,
    started: bool,
    total: usize,
}
#[derive(Default)]
struct StreamState {
    next: usize,
    text: Option<usize>,
    thinking: Option<usize>,
    tools: BTreeMap<usize, Tool>,
    output_tokens: u64,
    input_tokens: u64,
    reason: String,
    total: usize,
}
impl StreamState {
    fn delta(&mut self, value: &Value) -> Result<Vec<Value>, String> {
        if value.get("error").is_some() {
            return Err("供应商流返回 API 错误".into());
        }
        let mut events = Vec::new();
        let delta = &value["choices"][0]["delta"];
        if let Some(n) = value["usage"]["completion_tokens"].as_u64() {
            self.output_tokens = n;
        }
        if let Some(n) = value["usage"]["prompt_tokens"].as_u64() {
            self.input_tokens = n;
        }
        if let Some(reason) = value["choices"][0]["finish_reason"].as_str() {
            self.reason = reason.into();
        }
        for (kind, source) in [
            (
                "thinking",
                delta["reasoning_content"]
                    .as_str()
                    .or(delta["reasoning"].as_str()),
            ),
            ("text", delta["content"].as_str()),
        ] {
            if let Some(content) = source.filter(|s| !s.is_empty()) {
                self.total += content.len();
                if self.total > 4 * 1024 * 1024 {
                    return Err("模型单次输出超过 4 MiB，已停止接收".into());
                }
                let slot = if kind == "text" {
                    &mut self.text
                } else {
                    &mut self.thinking
                };
                let index = if let Some(index) = *slot {
                    index
                } else {
                    let index = self.next;
                    self.next += 1;
                    *slot = Some(index);
                    let block = if kind == "text" {
                        json!({"type":"text","text":""})
                    } else {
                        json!({"type":"thinking","thinking":"","signature":""})
                    };
                    events.push(
                        json!({"type":"content_block_start","index":index,"content_block":block}),
                    );
                    index
                };
                events.push(json!({"type":"content_block_delta","index":index,"delta":if kind=="text"{json!({"type":"text_delta","text":content})}else{json!({"type":"thinking_delta","thinking":content})}}));
            }
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for call in calls {
                let upstream = call["index"].as_u64().unwrap_or(0) as usize;
                if !self.tools.contains_key(&upstream) {
                    if self.tools.len() >= 64 {
                        return Err("模型一次请求超过 64 个工具调用".into());
                    }
                    let index = self.next;
                    self.next += 1;
                    self.tools.insert(
                        upstream,
                        Tool {
                            index,
                            ..Default::default()
                        },
                    );
                }
                let tool = self.tools.get_mut(&upstream).unwrap();
                if let Some(id) = call["id"].as_str() {
                    tool.id.push_str(id);
                }
                if let Some(name) = call["function"]["name"].as_str() {
                    tool.name.push_str(name);
                }
                if let Some(args) = call["function"]["arguments"].as_str() {
                    tool.args.push_str(args);
                    tool.total += args.len();
                }
                if tool.total > 1024 * 1024 || tool.name.len() > 200 || tool.id.len() > 200 {
                    return Err("工具参数超过限制".into());
                }
                if !tool.started && !tool.id.is_empty() && !tool.name.is_empty() {
                    tool.started = true;
                    events.push(json!({"type":"content_block_start","index":tool.index,"content_block":{"type":"tool_use","id":tool.id,"name":tool.name,"input":{}}}));
                }
                if tool.started && !tool.args.is_empty() {
                    events.push(json!({"type":"content_block_delta","index":tool.index,"delta":{"type":"input_json_delta","partial_json":std::mem::take(&mut tool.args)}}));
                }
            }
        }
        Ok(events)
    }
    fn finish(&self) -> Result<Vec<Value>, String> {
        if self.reason.is_empty() {
            return Err("供应商流提前关闭，未返回完成状态".into());
        }
        if self.tools.values().any(|t| !t.started) {
            return Err("供应商工具调用缺少名称或 ID".into());
        }
        let mut events = (0..self.next)
            .map(|index| json!({"type":"content_block_stop","index":index}))
            .collect::<Vec<_>>();
        events.push(json!({"type":"message_delta","delta":{"stop_reason":stop_reason(&self.reason),"stop_sequence":null},"usage":{"output_tokens":self.output_tokens}}));
        events.push(json!({"type":"message_stop"}));
        Ok(events)
    }
}
async fn messages(
    State(state): State<Arc<Connection>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !authorized(&headers, &state) {
        return error(
            StatusCode::UNAUTHORIZED,
            "Unauthorized local adapter request",
        );
    }
    if *state.cancel.borrow() {
        return error(StatusCode::GONE, "Agent turn stopped");
    }
    let chat = match to_chat(&body) {
        Ok(v) => v,
        Err(e) => return error(StatusCode::BAD_REQUEST, &e),
    };
    let model = body["model"].as_str().unwrap_or("").to_owned();
    let url = format!(
        "{}/chat/completions",
        state.config["baseUrl"]
            .as_str()
            .unwrap_or("")
            .trim_end_matches('/')
    );
    let mut request = state.http.post(url).json(&chat);
    if let Some(key) = crate::providers::key(&state.config) {
        request = request.bearer_auth(key);
    }
    let mut cancellation = state.cancel.clone();
    let response = tokio::select! {_=cancellation.changed()=>return error(StatusCode::GONE,"Agent turn stopped"), result=request.send()=>match result{Ok(v)=>v,Err(_)=>return error(StatusCode::BAD_GATEWAY,"连接供应商失败或超时")}};
    if !response.status().is_success() {
        return error(
            response.status(),
            &format!(
                "供应商返回 HTTP {}，请检查 API 地址、Key 与模型权限",
                response.status().as_u16()
            ),
        );
    }
    if body["stream"] != true {
        let value = tokio::select! {_=cancellation.changed()=>return error(StatusCode::GONE,"Agent turn stopped"),value=crate::providers::read_json(response)=>value};
        return match value.and_then(|v| from_chat(v, &model)) {
            Ok(v) => Json(v).into_response(),
            Err(e) => error(StatusCode::BAD_GATEWAY, &e),
        };
    }
    if !response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("text/event-stream"))
    {
        return error(
            StatusCode::BAD_GATEWAY,
            "供应商没有返回 SSE 流，请确认模型支持流式工具调用",
        );
    }
    let stream = async_stream::stream! {
        yield Ok::<Event,Infallible>(sse(json!({"type":"message_start","message":{"id":format!("msg_{}",uuid::Uuid::new_v4().simple()),"type":"message","role":"assistant","model":model,"content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0}}})));
        let mut chunks=response.bytes_stream();let mut buffer=Vec::<u8>::new();let mut translator=StreamState::default();let mut failed=false;
        'read: loop {
            let next=tokio::select!{_=cancellation.changed()=>{failed=true;break;},next=chunks.next()=>next};
            let Some(chunk)=next else{break;};let chunk=match chunk{Ok(v)=>v,Err(_)=>{yield Ok(sse(json!({"type":"error","error":{"type":"api_error","message":"供应商流连接中断"}})));failed=true;break;}};
            buffer.extend_from_slice(&chunk);if buffer.len()>2*1024*1024 {yield Ok(sse(json!({"type":"error","error":{"type":"api_error","message":"供应商单个流事件过大"}})));failed=true;break;}
            while let Some(position)=buffer.iter().position(|b|*b==b'\n') {
                let line=buffer.drain(..=position).collect::<Vec<_>>();let line=match std::str::from_utf8(&line){Ok(v)=>v.trim(),Err(_)=>{failed=true;break 'read;}};
                let Some(data)=line.strip_prefix("data:").map(str::trim) else{continue;};if data=="[DONE]"{break 'read;}if data.is_empty(){continue;}
                let result=serde_json::from_str::<Value>(data).map_err(|_|"供应商流 JSON 无效".to_owned()).and_then(|v|translator.delta(&v));
                match result{Ok(events)=>for event in events{yield Ok(sse(event));},Err(message)=>{yield Ok(sse(json!({"type":"error","error":{"type":"api_error","message":message}})));failed=true;break 'read;}}
            }
        }
        if !failed {match translator.finish(){Ok(events)=>for event in events{yield Ok(sse(event));},Err(message)=>yield Ok(sse(json!({"type":"error","error":{"type":"api_error","message":message}})))};}
    };
    Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translates_tool_history_and_keeps_real_model_id() {
        let input = json!({"model":"kimi-k2.5","max_tokens":1024,"stream":true,"system":[{"type":"text","text":"system"}],"tools":[{"name":"Read","input_schema":{"type":"object"}}],"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"summary"},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"README.md"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"file text"},{"type":"text","text":"continue"}]}]});
        let chat = to_chat(&input).unwrap();
        assert_eq!(chat["model"], "kimi-k2.5");
        assert_eq!(chat["messages"][1]["reasoning_content"], "summary");
        assert_eq!(chat["messages"][2]["role"], "tool");
        assert_eq!(chat["messages"][2]["tool_call_id"], "t1");
        assert_eq!(chat["messages"][3]["content"], "continue");
    }
    #[test]
    fn streaming_reasoning_text_and_fragmented_tools_are_distinct() {
        let mut state = StreamState::default();
        let a=state.delta(&json!({"choices":[{"delta":{"reasoning_content":"think","tool_calls":[{"index":0,"id":"t1","function":{"name":"Read","arguments":"{\"file_"}}]}}]})).unwrap();
        assert!(a.iter().any(|e| e["delta"]["type"] == "thinking_delta"));
        let b=state.delta(&json!({"choices":[{"delta":{"content":"reply","tool_calls":[{"index":0,"function":{"arguments":"path\":\"README.md\"}"}}]},"finish_reason":"tool_calls"}]})).unwrap();
        assert!(b.iter().any(|e| e["delta"]["type"] == "input_json_delta"));
        assert_eq!(
            state.finish().unwrap().last().unwrap()["type"],
            "message_stop"
        );
        let missing = StreamState::default();
        assert!(missing.finish().is_err());
    }
    #[tokio::test]
    async fn loopback_requires_token_and_shuts_down_on_drop() {
        let b = Bridge::start(json!({"baseUrl":"http://127.0.0.1:1/v1"}))
            .await
            .unwrap();
        let url = format!("{}/v1/messages", b.base_url);
        let client = reqwest::Client::new();
        let denied = client.post(&url).json(&json!({})).send().await.unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        drop(b);
        tokio::task::yield_now().await;
        assert!(client.post(url).json(&json!({})).send().await.is_err());
    }
    #[tokio::test]
    async fn http_roundtrip_stream_and_nonstream_preserve_model_auth_and_tools() {
        async fn upstream(headers: HeaderMap, Json(body): Json<Value>) -> Response {
            assert_eq!(headers.get("authorization").unwrap(), "Bearer fixture-key");
            assert_eq!(body["model"], "provider-real-model");
            assert_eq!(body["tools"][0]["function"]["name"], "Read");
            if body["stream"] == true {
                let chunks = [
                    json!({"choices":[{"delta":{"reasoning_content":"检查项目"}}]}),
                    json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"Read","arguments":"{\"file_path\":\"README.md\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":8}}),
                ];
                let text = chunks
                    .into_iter()
                    .map(|v| format!("data: {v}\n\n"))
                    .collect::<String>()
                    + "data: [DONE]\n\n";
                ([("content-type", "text/event-stream")], text).into_response()
            } else {
                Json(json!({"id":"fixture-response","model":"provider-real-model","choices":[{"message":{"content":"中文结果"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":8}})).into_response()
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/v1/chat/completions", post(upstream)),
            )
            .await
            .unwrap();
        });
        let bridge =
            Bridge::start(json!({"baseUrl":format!("http://{address}/v1"),"apiKey":"fixture-key"}))
                .await
                .unwrap();
        let client = reqwest::Client::new();
        for stream in [false, true] {
            let response=client.post(format!("{}/v1/messages",bridge.base_url)).header("x-api-key",&bridge.token).json(&json!({"model":"provider-real-model","stream":stream,"messages":[{"role":"user","content":"read"}],"tools":[{"name":"Read","input_schema":{"type":"object"}}]})).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            if stream {
                let text = response.text().await.unwrap();
                assert!(text.contains("thinking_delta"));
                assert!(text.contains("input_json_delta"));
                assert!(text.contains("tool_use"));
                assert!(text.contains("message_stop"));
            } else {
                let value = response.json::<Value>().await.unwrap();
                assert_eq!(value["model"], "provider-real-model");
                assert_eq!(value["content"][0]["text"], "中文结果");
            }
        }
        drop(bridge);
        server.abort();
    }
}
