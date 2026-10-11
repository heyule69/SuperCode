//! Normalize native CC Switch provider declarations without importing tools or hooks.
use serde_json::{json, Map, Value};

fn fields(source: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|key| source.get(*key).map(|v| ((*key).into(), v.clone())))
            .collect(),
    )
}
fn model_id(id: &str) -> Result<String, String> {
    let id = id.trim();
    if id.is_empty() || id.len() > 200 || id.chars().any(char::is_control) {
        return Err("模型 ID 为空或格式无效".into());
    }
    Ok(id.into())
}
// Resolve explicit environment references in Rust. Never run credential commands
// or hand file/command expressions to a native agent during an import.
fn credential(value: &Value, bare_env: bool) -> Result<String, String> {
    if value.is_null() {
        return Ok(String::new());
    }
    let text = value.as_str().ok_or("密钥或请求头必须是字符串")?.trim();
    if text.starts_with('!') || text.contains("{file:") {
        return Err("此连接使用命令或文件读取凭据，请改为 API Key 或环境变量后读取".into());
    }
    let reference = text
        .strip_prefix("{env:")
        .and_then(|s| s.strip_suffix('}'))
        .or_else(|| text.strip_prefix("${").and_then(|s| s.strip_suffix('}')))
        .or_else(|| text.strip_prefix('$'));
    let resolved = if let Some(name) = reference {
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("凭据环境变量引用格式无效".into());
        }
        std::env::var(name)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .ok_or("当前设备缺少凭据环境变量，请设置变量或填写 API Key 后读取")?
    } else if bare_env {
        // Older Pi versions use a bare environment variable name.
        match std::env::var(text).ok().filter(|s| !s.is_empty()) {
            Some(value) => value,
            None if text
                .chars()
                .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
                && (text.ends_with("_KEY") || text.ends_with("_TOKEN")) =>
            {
                return Err("当前设备缺少凭据环境变量，请设置变量或填写 API Key 后读取".into());
            }
            None => text.into(),
        }
    } else {
        text.into()
    };
    if resolved.len() > 8192
        || resolved.chars().any(char::is_control)
        || resolved.starts_with('!')
        || resolved.contains("{file:")
        || resolved.contains("{env:")
        || resolved.contains("${")
    {
        return Err("密钥或请求头格式无效".into());
    }
    Ok(resolved)
}
fn headers(value: &Value, bare_env: bool) -> Result<Value, String> {
    if value.is_null() {
        return Ok(json!({}));
    }
    let entries = value.as_object().ok_or("请求头必须是对象")?;
    if entries.len() > 64 {
        return Err("请求头数量超过限制".into());
    }
    let mut out = Map::new();
    for (name, value) in entries {
        if name.parse::<reqwest::header::HeaderName>().is_err() {
            return Err("请求头名称无效".into());
        }
        out.insert(name.clone(), json!(credential(value, bare_env)?));
    }
    Ok(Value::Object(out))
}
fn pi_protocol(api: &str) -> Result<&'static str, String> {
    match api {
        "anthropic-messages" => Ok("anthropic"),
        "openai-responses" => Ok("responses"),
        "openai-completions" => Ok("chat"),
        _ => Err("暂不支持此 Pi API 协议；目前支持 Anthropic Messages、OpenAI Responses 和 OpenAI Completions".into()),
    }
}
fn fragment<'a>(raw: &'a Value, agent: &str, id: &str) -> Result<&'a Value, String> {
    let wrapper = if raw.get("providers").is_some() {
        Some("providers")
    } else if agent == "opencode" && raw.get("provider").is_some() {
        Some("provider")
    } else {
        None
    };
    if let Some(wrapper) = wrapper {
        raw[wrapper]
            .get(id)
            .filter(|v| v.is_object())
            .ok_or("配置中没有当前供应商的声明，未使用其他供应商的地址或凭据".into())
    } else {
        Ok(raw)
    }
}
pub(super) fn sanitize(agent: &str, id: &str, raw: &Value) -> Result<Value, String> {
    let source = fragment(raw, agent, id)?;
    let mut native;
    let (base, protocol, key, models) = if agent == "opencode" {
        if ["package", "settings", "canonical", "body"]
            .iter()
            .any(|key| source.get(*key).is_some())
        {
            return Err("此连接使用 OpenCode V2 原生声明，当前运行适配器需要 npm/options 格式，请使用 CC Switch 的 V1 兼容配置".into());
        }
        let npm = source["npm"]
            .as_str()
            .unwrap_or("@ai-sdk/openai-compatible");
        let protocol = match npm {
            "@ai-sdk/anthropic" => "anthropic",
            "@ai-sdk/openai" => "responses",
            "@ai-sdk/openai-compatible" => "chat",
            _ => return Err("暂不支持此 OpenCode SDK；目前支持 Anthropic、OpenAI Responses 和 OpenAI Compatible".into()),
        };
        let options = &source["options"];
        let base = super::imported_base_url(
            options["baseURL"]
                .as_str()
                .ok_or("OpenCode 缺少 options.baseURL")?,
        )?;
        let key = credential(&options["apiKey"], false)?;
        let entries = source["models"]
            .as_object()
            .filter(|v| !v.is_empty())
            .ok_or("OpenCode 缺少模型声明")?;
        if entries.len() > 200 {
            return Err("单个连接最多导入 200 个模型".into());
        }
        let mut models = Map::new();
        for (id, value) in entries {
            if !value.is_object() {
                return Err("OpenCode 模型声明必须是对象".into());
            }
            let model = model_id(id)?;
            let mut definition = fields(
                value,
                &[
                    "name",
                    "family",
                    "attachment",
                    "reasoning",
                    "temperature",
                    "tool_call",
                    "cost",
                    "limit",
                    "modalities",
                    "options",
                    "variants",
                    "interleaved",
                ],
            );
            if value.get("headers").is_some() {
                definition["headers"] = headers(&value["headers"], false)?;
            }
            models.insert(model, definition);
        }
        native = fields(source, &["name"]);
        native["npm"] = json!(npm);
        native["options"] = fields(options, &["setCacheKey", "timeout", "chunkTimeout"]);
        native["options"]["headers"] = headers(&options["headers"], false)?;
        native["models"] = json!(models);
        (
            base,
            protocol,
            key,
            models.keys().cloned().collect::<Vec<_>>(),
        )
    } else {
        let entries = source["models"]
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or("Pi 缺少 models 模型列表；内置模型或 OAuth 登录需在 Pi 中单独配置")?;
        if entries.len() > 200 {
            return Err("单个连接最多导入 200 个模型".into());
        }
        let base = source["baseUrl"]
            .as_str()
            .or_else(|| entries.iter().find_map(|m| m["baseUrl"].as_str()))
            .ok_or("Pi 缺少 baseUrl 连接地址")?;
        let base = super::imported_base_url(base)?;
        let api = source["api"]
            .as_str()
            .or_else(|| entries.iter().find_map(|m| m["api"].as_str()))
            .ok_or("Pi 缺少 API 协议")?;
        let protocol = pi_protocol(api)?;
        let key = credential(&source["apiKey"], true)?;
        let mut models = Vec::new();
        let mut definitions = Vec::new();
        for value in entries {
            let id = model_id(value["id"].as_str().ok_or("Pi 模型缺少 id")?)?;
            if models.contains(&id) {
                return Err("Pi 模型 ID 重复，请修正后读取".into());
            }
            let mut definition = fields(
                value,
                &[
                    "name",
                    "reasoning",
                    "input",
                    "contextWindow",
                    "maxTokens",
                    "cost",
                    "compat",
                    "thinkingLevelMap",
                    "inputLimits",
                    "promptCache",
                    "samplingParams",
                    "samplingParamsByThinkingLevel",
                ],
            );
            for field in ["contextWindow", "maxTokens"] {
                if value
                    .get(field)
                    .is_some_and(|v| v.as_u64().is_none_or(|n| n == 0))
                {
                    return Err("Pi 模型上下文或输出上限必须为正整数".into());
                }
            }
            if value.get("reasoning").is_some_and(|v| !v.is_boolean())
                || value.get("input").is_some_and(|v| {
                    v.as_array().is_none_or(|values| {
                        values.is_empty()
                            || values
                                .iter()
                                .any(|v| !matches!(v.as_str(), Some("text" | "image")))
                    })
                })
            {
                return Err("Pi 模型能力声明无效".into());
            }
            definition["id"] = json!(id);
            if source["baseUrl"].as_str().is_none() && value["baseUrl"].as_str().is_none()
                || source["api"].as_str().is_none() && value["api"].as_str().is_none()
            {
                return Err("Pi 模型缺少继承或单独声明的地址与 API 协议".into());
            }
            if let Some(url) = value["baseUrl"].as_str() {
                definition["baseUrl"] = json!(super::imported_base_url(url)?);
            }
            if let Some(api) = value["api"].as_str() {
                pi_protocol(api)?;
                definition["api"] = json!(api);
            }
            if value.get("headers").is_some() {
                definition["headers"] = headers(&value["headers"], true)?;
            }
            definitions.push(definition);
            models.push(id);
        }
        native = fields(source, &["name", "authHeader"]);
        native["api"] = json!(api);
        native["headers"] = headers(&source["headers"], true)?;
        native["models"] = json!(definitions);
        (base, protocol, key, models)
    };
    if native.to_string().contains("{file:") || native.to_string().contains("{env:") {
        return Err("模型声明含未解析的文件或环境引用，请改为明确配置后读取".into());
    }
    let requested = raw["model"].as_str().or(source["defaultModel"].as_str());
    let default = requested
        .and_then(|model| model.strip_prefix(&format!("{id}/")).or(Some(model)))
        .filter(|model| models.iter().any(|id| id == model))
        .unwrap_or(&models[0]);
    let mut config = json!({"baseUrl":base,"apiKey":key,"protocol":protocol,"model":default,"models":models,"source":"ccswitch"});
    config[if agent == "opencode" {
        "opencodeProvider"
    } else {
        "piProvider"
    }] = native;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn common_protocols_preserve_models_addresses_and_headers() {
        for (npm, api, protocol) in [
            ("@ai-sdk/anthropic", "anthropic-messages", "anthropic"),
            ("@ai-sdk/openai", "openai-responses", "responses"),
            ("@ai-sdk/openai-compatible", "openai-completions", "chat"),
        ] {
            let open = sanitize("opencode", "x", &json!({"npm":npm,"options":{"baseURL":"http://gateway.local/v1/","apiKey":"private-key","headers":{"X-Project":"private-header"}},"models":{"z":{"reasoning":false,"limit":{"context":200000,"output":4096}},"a":{"name":"另一个"}},"model":"z","plugins":["unsafe"],"permission":"allow","mcp":{"unsafe":{}}})).unwrap();
            assert_eq!(open["protocol"], protocol);
            assert_eq!(open["baseUrl"], "http://gateway.local/v1");
            assert_eq!(open["apiKey"], "private-key");
            assert_eq!(open["model"], "z");
            assert_eq!(open["models"], json!(["a", "z"]));
            assert_eq!(
                open["opencodeProvider"]["options"]["headers"]["X-Project"],
                "private-header"
            );
            assert!(open.get("plugins").is_none());
            assert!(open.get("permission").is_none());
            assert!(open.get("mcp").is_none());
            let pi = sanitize("pi", "x", &json!({"api":api,"baseUrl":"https://api.example/v1/","apiKey":"private-key","headers":{"X-Project":"private-header"},"models":[{"id":"z","contextWindow":262144,"maxTokens":8192,"reasoning":true,"input":["text","image"]},{"id":"a"}]})).unwrap();
            assert_eq!(pi["protocol"], protocol);
            assert_eq!(pi["models"], json!(["z", "a"]));
            assert_eq!(pi["piProvider"]["models"][0]["contextWindow"], 262144);
        }
    }
    #[test]
    fn full_configs_select_only_the_matching_provider_and_keep_pi_model_overrides() {
        let raw = json!({"providers":{"mine":{"baseUrl":"http://localhost:1234/v1","api":"openai-completions","apiKey":"mine-secret","models":[{"id":"m","api":"openai-responses","baseUrl":"https://other-model.example/v1"}]},"other":{"apiKey":"other-secret"}}});
        let safe = sanitize("pi", "mine", &raw).unwrap();
        assert_eq!(safe["apiKey"], "mine-secret");
        assert!(!safe.to_string().contains("other-secret"));
        assert_eq!(safe["piProvider"]["models"][0]["api"], "openai-responses");
        assert!(sanitize("pi", "missing", &raw).is_err());
    }
    #[test]
    fn unsupported_and_malformed_configs_never_become_broken_importable_rows() {
        for raw in [
            json!({"api":"unknown","baseUrl":"https://api.example","models":[{"id":"m"}]}),
            json!({"api":"openai-completions","baseUrl":"file:///private","models":[{"id":"m"}]}),
            json!({"api":"openai-completions","baseUrl":"https://api.example","models":[{"id":"m","contextWindow":0}]}),
            json!({"api":"openai-completions","baseUrl":"https://api.example","models":[{"id":"m"},{"id":"m"}]}),
            json!({"api":"openai-completions","models":[{"id":"a","baseUrl":"https://api.example"},{"id":"b"}]}),
        ] {
            assert!(sanitize("pi", "x", &raw).is_err());
        }
        let error = credential(&json!("!echo private-secret"), true).unwrap_err();
        assert!(!error.contains("private-secret"));
        assert!(credential(&json!("{file:C:/private-key}"), false).is_err());
        assert!(credential(
            &json!(format!(
                "${{SUPERCODE_MISSING_{}}}",
                uuid::Uuid::new_v4().simple()
            )),
            true
        )
        .unwrap_err()
        .contains("当前设备缺少"));
        assert!(headers(&json!({"X-Test":"private\r\nInjected: value"}), false).is_err());
        assert!(
            sanitize("opencode", "x", &json!({"package":"custom","models":{}}))
                .unwrap_err()
                .contains("V2")
        );
    }
    #[test]
    fn environment_credentials_resolve_without_mistaking_opencode_literals_for_pi_variables() {
        let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let value = std::env::var(variable).unwrap();
        for reference in [
            format!("{{env:{variable}}}"),
            format!("${{{variable}}}"),
            format!("${variable}"),
        ] {
            assert_eq!(credential(&json!(reference), false).unwrap(), value);
        }
        assert_eq!(credential(&json!(variable), true).unwrap(), value);
        assert_eq!(credential(&json!(variable), false).unwrap(), variable);
        let missing = format!(
            "SUPERCODE_MISSING_{}_API_KEY",
            uuid::Uuid::new_v4().simple().to_string().to_uppercase()
        );
        assert!(credential(&json!(missing), true)
            .unwrap_err()
            .contains("当前设备缺少"));
    }
}
