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
pub fn collect(value: &Value, secrets: &mut Vec<String>, depth: usize) {
    if depth > 12 || secrets.len() >= 512 {
        return;
    }
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if sensitive(key) {
                    if let Some(secret) = value.as_str().filter(|s| s.len() >= 4 && s.len() <= 4096)
                    {
                        if !secrets.iter().any(|s| s == secret) {
                            secrets.push(secret.to_owned());
                        }
                    }
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
