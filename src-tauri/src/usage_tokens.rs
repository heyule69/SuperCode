//! Protocol token counts. Missing counters stay unknown; cache counters are
//! subsets in OpenAI responses and additional input in Anthropic/Pi responses.
use serde_json::{json, Value};

pub(crate) fn count(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

pub(crate) fn chat_usage(native: &Value) -> Value {
    let cached = count(&native["prompt_tokens_details"]["cached_tokens"])
        .or_else(|| count(&native["input_tokens_details"]["cached_tokens"]))
        .or_else(|| count(&native["prompt_cache_hit_tokens"]));
    let input = count(&native["prompt_tokens"])
        .or_else(|| count(&native["input_tokens"]))
        .or_else(|| Some(count(&native["prompt_cache_miss_tokens"])?.saturating_add(cached?)));
    let output = count(&native["completion_tokens"]).or_else(|| count(&native["output_tokens"]));
    let mut usage = json!({});
    if let Some(input) = input {
        let cached = cached.unwrap_or(0).min(input);
        let write = count(&native["prompt_tokens_details"]["cache_write_tokens"])
            .unwrap_or(0)
            .min(input - cached);
        usage["input_tokens"] = json!(input - cached - write);
        usage["cache_read_input_tokens"] = json!(cached);
        usage["cache_creation_input_tokens"] = json!(write);
    }
    if let Some(output) = output {
        usage["output_tokens"] = json!(output);
        if let Some(reasoning) = count(&native["completion_tokens_details"]["reasoning_tokens"])
            .or_else(|| count(&native["output_tokens_details"]["reasoning_tokens"]))
        {
            usage["output_tokens_details"] = json!({"thinking_tokens":reasoning.min(output)});
        }
    }
    usage
}

pub(crate) fn pi_usage(native: &Value) -> Value {
    let input = count(&native["input"]);
    let cached = count(&native["cacheRead"]);
    let write = count(&native["cacheWrite"]);
    let output = count(&native["output"]);
    let mut last = json!({});
    if input.is_some() || cached.is_some() || write.is_some() {
        last["inputTokens"] = json!(input
            .unwrap_or(0)
            .saturating_add(cached.unwrap_or(0))
            .saturating_add(write.unwrap_or(0)));
    }
    for (key, value) in [
        ("cachedInputTokens", cached),
        ("cacheWriteInputTokens", write),
        ("outputTokens", output),
    ] {
        if let Some(value) = value {
            last[key] = json!(value);
        }
    }
    if let Some(total) = count(&native["totalTokens"])
        .filter(|n| *n > 0)
        .or_else(|| Some(count(&last["inputTokens"])?.saturating_add(output?)))
        .or_else(|| count(&native["totalTokens"]))
    {
        last["totalTokens"] = json!(total);
    }
    last
}

pub(crate) fn pi_context(last: &Value) -> Option<u64> {
    match count(&last["inputTokens"]) {
        Some(0) => None,
        Some(input) => Some(input.saturating_add(count(&last["outputTokens"]).unwrap_or(0))),
        None => count(&last["totalTokens"]).filter(|n| *n > 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_protocol_variants_preserve_cache_without_double_counting() {
        for native in [
            json!({"prompt_tokens":100,"completion_tokens":10,"prompt_tokens_details":{"cached_tokens":80},"completion_tokens_details":{"reasoning_tokens":4}}),
            json!({"input_tokens":"100","output_tokens":"10","input_tokens_details":{"cached_tokens":"80"},"output_tokens_details":{"reasoning_tokens":"4"}}),
            json!({"prompt_cache_miss_tokens":20,"prompt_cache_hit_tokens":80,"completion_tokens":10,"completion_tokens_details":{"reasoning_tokens":4}}),
        ] {
            let usage = chat_usage(&native);
            assert_eq!(usage["input_tokens"], 20);
            assert_eq!(usage["cache_read_input_tokens"], 80);
            assert_eq!(usage["output_tokens"], 10);
            assert_eq!(usage["output_tokens_details"]["thinking_tokens"], 4);
        }
        let usage = chat_usage(
            &json!({"prompt_tokens":100,"prompt_tokens_details":{"cached_tokens":60,"cache_write_tokens":30}}),
        );
        assert_eq!(usage["input_tokens"], 10);
        assert_eq!(usage["cache_creation_input_tokens"], 30);
        assert!(usage.get("output_tokens").is_none());
    }

    #[test]
    fn missing_and_invalid_usage_does_not_become_zero() {
        for native in [
            Value::Null,
            json!({}),
            json!({"prompt_tokens":-1,"completion_tokens":"unknown"}),
        ] {
            assert_eq!(chat_usage(&native), json!({}));
            assert_eq!(pi_usage(&native), json!({}));
        }
        assert!(count(&json!(1.5)).is_none());
        assert!(count(&json!(true)).is_none());
        assert_eq!(count(&json!(0)), Some(0));
    }

    #[test]
    fn pi_counts_include_cache_and_reported_total_without_guessing_missing_fields() {
        let usage = pi_usage(&json!({"input":10,"cacheRead":5,"cacheWrite":2,"output":4}));
        assert_eq!(usage["inputTokens"], 17);
        assert_eq!(usage["totalTokens"], 21);
        let only_total = pi_usage(&json!({"totalTokens":21}));
        assert_eq!(only_total["totalTokens"], 21);
        assert!(only_total.get("inputTokens").is_none());
        assert_eq!(pi_context(&only_total), Some(21));
        assert!(pi_context(&pi_usage(&json!({"input":0,"output":4,"totalTokens":4}))).is_none());
    }
}
