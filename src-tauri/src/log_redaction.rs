//! Redact credentials before truncation or persistence, including non-JSON stderr.
use serde_json::Value;

const MASK: &str = "[REDACTED]";
fn bounded(text: &str) -> String {
    const BYTES: usize = 8192;
    if text.len() <= BYTES {
        return text.to_owned();
    }
    let mut end = BYTES - '…'.len_utf8();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}
fn sensitive(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    key == "key"
        || key.ends_with("apikey")
        || key.ends_with("token")
        || key.ends_with("password")
        || key.ends_with("secret")
        || key.contains("authorization")
        || key.ends_with("cookie")
        || key.ends_with("privatekey")
}
fn remember(secret: &str, secrets: &mut Vec<String>) {
    if secret.len() >= 4 && secret.len() <= 4096 && secrets.len() < 512 && !secrets.iter().any(|known| known == secret) {
        secrets.push(secret.to_owned());
    }
}
pub fn collect(value: &Value, secrets: &mut Vec<String>, depth: usize) {
    if depth > 12 || secrets.len() >= 512 {
        return;
    }
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if sensitive(key) {
                    if let Some(secret) = value.as_str() { remember(secret, secrets); }
                } else if key == "config" {
                    if let Some(text) = value.as_str() {
                        if let Ok(table) = toml::from_str::<toml::Table>(text) {
                            if let Ok(parsed) = serde_json::to_value(table) {
                                collect(&parsed, secrets, depth + 1);
                            }
                        } else {
                            collect(value, secrets, depth + 1);
                        }
                    } else {
                        collect(value, secrets, depth + 1);
                    }
                } else {
                    collect(value, secrets, depth + 1);
                }
            }
        }
        Value::Array(items) => {
            for value in items.iter().take(256) {
                collect(value, secrets, depth + 1);
            }
        }
        Value::String(text) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                collect(&parsed, secrets, depth + 1);
            }
        }
        _ => {}
    }
}
/// MCP transports may use arbitrary environment/header names. Register values,
/// including resolved environment references, without emitting the configuration.
pub fn collect_mcp(config: &Value, secrets: &mut Vec<String>, environment: impl Fn(&str) -> Option<String>) {
    fn credential(value: &str, secrets: &mut Vec<String>) {
        remember(value, secrets);
        if let Some((scheme, token)) = value.split_once(' ') {
            if scheme.eq_ignore_ascii_case("bearer") || scheme.eq_ignore_ascii_case("basic") {
                remember(token.trim(), secrets);
            }
        }
    }
    fn visit(value: &Value, secrets: &mut Vec<String>, environment: &impl Fn(&str) -> Option<String>, depth: usize) {
        if depth > 12 || secrets.len() >= 512 { return; }
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    match key.as_str() {
                        "env" | "http_headers" | "headers" => {
                            if let Some(fields) = value.as_object() {
                                for value in fields.values().filter_map(Value::as_str) { credential(value, secrets); }
                            }
                        }
                        "env_http_headers" => {
                            if let Some(fields) = value.as_object() {
                                for name in fields.values().filter_map(Value::as_str) {
                                    if let Some(value) = environment(name) { credential(&value, secrets); }
                                }
                            }
                        }
                        "bearer_token_env_var" => {
                            if let Some(value) = value.as_str().and_then(environment) { credential(&value, secrets); }
                        }
                        "args" => {
                            if let Some(args) = value.as_array() {
                                for (index, arg) in args.iter().enumerate() {
                                    let Some(arg) = arg.as_str() else { continue; };
                                    if let Some((flag, value)) = arg.split_once('=') {
                                        if sensitive(flag) { credential(value, secrets); }
                                        visit(&Value::String(value.to_owned()), secrets, environment, depth + 1);
                                    } else if sensitive(arg) && arg.starts_with('-') {
                                        if let Some(value) = args.get(index + 1).and_then(Value::as_str) { credential(value, secrets); }
                                    } else if matches!(arg, "--header" | "--headers" | "-H") {
                                        if let Some((_, value)) = args.get(index + 1).and_then(Value::as_str).and_then(|value| value.split_once(':')) {
                                            credential(value.trim(), secrets);
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    visit(value, secrets, environment, depth + 1);
                }
            }
            Value::Array(values) => {
                for value in values.iter().take(256) { visit(value, secrets, environment, depth + 1); }
            }
            Value::String(value) => {
                if let Ok(url) = tauri::Url::parse(value) {
                    if let Some(password) = url.password() { credential(password, secrets); }
                    for (_, value) in url.query_pairs().filter(|(key, _)| sensitive(key)) { credential(&value, secrets); }
                } else if let Ok(table) = toml::from_str::<toml::Table>(value) {
                    if let Ok(value) = serde_json::to_value(table) { visit(&value, secrets, environment, depth + 1); }
                }
            }
            _ => {}
        }
    }
    collect(config, secrets, 0);
    visit(config, secrets, &environment, 0);
}
pub fn environment() -> Vec<String> {
    let mut secrets = Vec::new();
    for (key, value) in std::env::vars() {
        if sensitive(&key) && value.len() >= 4 && value.len() <= 4096 && secrets.len() < 512 {
            secrets.push(value);
        }
    }
    secrets
}
fn mask_assignments(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !(bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'-'))
        {
            i += 1;
        }
        let key = &lower[start..i];
        let mut value = i;
        if sensitive(key) {
            while value < bytes.len()
                && (bytes[value].is_ascii_whitespace() || matches!(bytes[value], b'\'' | b'"'))
            {
                value += 1;
            }
            if value >= bytes.len() || !matches!(bytes[value], b':' | b'=') {
                continue;
            }
            value += 1;
            while value < bytes.len() && bytes[value].is_ascii_whitespace() {
                value += 1;
            }
        } else if matches!(key, "bearer" | "basic")
            && value < bytes.len()
            && bytes[value].is_ascii_whitespace()
        {
            while value < bytes.len() && bytes[value].is_ascii_whitespace() {
                value += 1;
            }
        } else if key.starts_with("sk-") || key.starts_with("eyj") {
            value = start;
        } else {
            continue;
        }
        if value >= bytes.len() {
            continue;
        }
        let quote = matches!(bytes[value], b'\'' | b'"').then_some(bytes[value]);
        let begin = value + usize::from(quote.is_some());
        let mut end = begin;
        if let Some(quote) = quote {
            while end < bytes.len()
                && (bytes[end] != quote || (end > begin && bytes[end - 1] == b'\\'))
            {
                end += 1;
            }
        } else {
            // Authorization includes a scheme followed by a credential.
            if lower[begin..].starts_with("bearer ") || lower[begin..].starts_with("basic ") {
                end += lower[begin..].find(' ').unwrap() + 1;
            }
            while end < bytes.len()
                && !bytes[end].is_ascii_whitespace()
                && !matches!(bytes[end], b',' | b';' | b'&' | b'"' | b'\'' | b'}')
            {
                end += 1;
            }
        }
        if end > begin {
            ranges.push((begin, end));
            i = end;
        }
    }
    let mut output = text.to_owned();
    for (start, end) in ranges.into_iter().rev() {
        output.replace_range(start..end, MASK);
    }
    output
}
fn sanitize(value: &mut Value, secrets: &[String], depth: usize) {
    if depth > 12 {
        *value = Value::String("[OMITTED]".into());
        return;
    }
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if sensitive(key)
                    || matches!(
                        key.as_str(),
                        "prompt" | "messages" | "conversation" | "input" | "arguments" | "env"
                    )
                {
                    *value = MASK.into();
                } else {
                    sanitize(value, secrets, depth + 1);
                }
            }
        }
        Value::Array(items) => {
            for value in items {
                sanitize(value, secrets, depth + 1);
            }
        }
        Value::String(text) => {
            for secret in secrets {
                *text = text.replace(secret, MASK);
            }
            if let Some(start) = text.find('{') {
                if let Ok(mut nested) = serde_json::from_str::<Value>(&text[start..]) {
                    sanitize(&mut nested, secrets, depth + 1);
                    *text = format!("{}{nested}", &text[..start]);
                }
            }
            *text = bounded(&mask_assignments(text));
        }
        _ => {}
    }
}
pub fn value(mut value: Value, secrets: &[String]) -> Value {
    sanitize(&mut value, secrets, 0);
    value
}
pub fn text(text: &str, secrets: &[String]) -> String {
    let mut output = text.to_owned();
    for secret in secrets {
        output = output.replace(secret, MASK);
    }
    if let Some(start) = output.find('{') {
        if let Ok(mut value) = serde_json::from_str::<Value>(&output[start..]) {
            sanitize(&mut value, secrets, 0);
            output = format!("{}{value}", &output[..start]);
        }
    }
    bounded(&mask_assignments(&output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn mcp_custom_credentials_and_resolved_header_values_are_redacted_without_labels() {
        let config = json!({"env":{"CUSTOM_ACCESS":"env-value-123"},
            "http_headers":{"X-Custom":"header-value-456","Authorization":"Bearer opaque-value-789"},
            "env_http_headers":{"X-Other":"HEADER_ENV"},"url":"https://example.com/mcp?api_key=query-value-654",
            "args":["--header","Authorization: Bearer argument-value-111","--url=https://example.com/mcp?api_key=argument-value-222"]});
        let mut secrets = Vec::new();
        collect_mcp(&config, &mut secrets, |name| (name == "HEADER_ENV").then(|| "resolved-value-321".to_owned()));
        let input = "Rejected env-value-123 header-value-456 opaque-value-789 resolved-value-321 query-value-654 argument-value-111 argument-value-222";
        let output = text(input, &secrets);
        assert_eq!(output, "Rejected [REDACTED] [REDACTED] [REDACTED] [REDACTED] [REDACTED] [REDACTED] [REDACTED]");
        assert_eq!(value(json!({"message":input}), &secrets)["message"], output);
    }
    #[test]
    fn masks_known_keys_auth_headers_json_and_url_credentials() {
        let mut secrets = Vec::new();
        collect(
            &json!({"env":{"ANTHROPIC_AUTH_TOKEN":"real-credential"},"apiKey":"other-credential"}),
            &mut secrets,
            0,
        );
        for input in ["request failed: real-credential", "Authorization: Bearer opaque-value", "url?api_key=opaque-value&status=429", "[sdk] {\"apiKey\":\"opaque-value\",\"prompt\":\"private conversation\",\"status\":429}", "failed with sk-ant-private-value"] {
            let result = text(input,&secrets); for secret in ["real-credential","opaque-value","private conversation","sk-ant-private-value"] { assert!(!result.contains(secret),"{result}"); }
        }
    }
    #[test]
    fn retains_error_messages_model_ids_and_numeric_token_counts() {
        let result = value(
            json!({"error":{"message":"context window exceeded","code":400},"model":"glm-5.3","contextTokens":120000,"token":"secret"}),
            &[],
        );
        assert_eq!(result["contextTokens"], 120000);
        assert_eq!(result["model"], "glm-5.3");
        assert_eq!(result["error"]["message"], "context window exceeded");
        assert_eq!(result["token"], MASK);
        assert!(
            text("异常：网络超时，Authorization='Bearer private-value'", &[]).contains("网络超时")
        );
    }
    #[test]
    fn redacts_before_truncation_and_bounds_the_output() {
        let secret = "secret-value-that-crosses-the-boundary".to_owned();
        let result = text(&format!("{}{}", "x".repeat(8185), secret), &[secret]);
        assert!(!result.contains("secret-value"));
        assert!(result.len() <= 8192);
        let unicode = text(&"中文错误".repeat(4096), &[]);
        assert!(unicode.len() <= 8192 && unicode.ends_with('…'));
    }
    #[test]
    fn registers_toml_and_serialized_mcp_credentials_and_redacts_embedded_payloads() {
        let mut secrets = Vec::new();
        collect(
            &json!({"config":"api_key = 'toml-private-key'", "SUPERCODE_PI_CONFIG":"{\"mcp\":{\"env\":{\"SERVICE_TOKEN\":\"mcp-private-key\"}}}"}),
            &mut secrets,
            0,
        );
        assert_eq!(secrets.len(), 2);
        let output=value(json!({"message":"sdk failed: {\"input\":\"private conversation\",\"error\":\"mcp-private-key, toml-private-key\",\"code\":429}"}),&secrets).to_string();
        for secret in [
            "private conversation",
            "mcp-private-key",
            "toml-private-key",
        ] {
            assert!(!output.contains(secret));
        }
        assert!(output.contains("429"));
    }
}
