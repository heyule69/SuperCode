//! Recover a private linkage worker without taking another client's writer lock.
use serde_json::{json, Value};
use std::future::Future;

pub(crate) struct OpenedThread {
    pub result: Value,
    pub recovered_from: Option<String>,
}

pub(crate) async fn open<F, Fut>(
    native: Option<&str>,
    loaded: bool,
    private_worker: bool,
    options: Value,
    mut request: F,
) -> Result<OpenedThread, String>
where
    F: FnMut(&'static str, Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let Some((method, parameters)) = crate::commands::codex_thread_request(native, loaded, options)
    else {
        return Ok(OpenedThread {
            result: json!({"thread":{"id":native}}),
            recovered_from: None,
        });
    };
    let failure = match request(method, parameters.clone()).await {
        Ok(result) => {
            return Ok(OpenedThread {
                result,
                recovered_from: None,
            })
        }
        Err(error) => error,
    };
    // Only an explicit rejection before turn/start is safe to recover. Never
    // replay a timed-out request, a turn, or an ordinary user-owned chat.
    let Some(source) = native.filter(|id| {
        method == "thread/resume" && failure == format!("thread {id} already has an active writer")
    }) else {
        return Err(failure);
    };
    let occupied = || {
        format!("这条 Codex 会话正被另一客户端占用。Codex 与 SuperCode 可以同时使用，无需关闭整个程序；可在占用方结束这条会话，或使用独立会话。原始错误：{failure}")
    };
    if !private_worker {
        return Err(occupied());
    }

    // An unloaded client's read may label a still-running persisted turn as
    // interrupted. Only a positively completed latest turn allows recovery.
    let latest = request(
        "thread/turns/list",
        json!({
            "threadId":source,"limit":1,"sortDirection":"desc","itemsView":"notLoaded"
        }),
    )
    .await
    .map_err(|error| format!("{}；无法核对最近回合，未自动延续：{error}", occupied()))?;
    let completed = latest["data"]
        .as_array()
        .is_some_and(|turns| turns.len() == 1 && turns[0]["status"] == "completed");
    if !completed {
        return Err(format!(
            "{}；最近回合尚未确认完成，为避免重复执行，未自动延续。",
            occupied()
        ));
    }
    let mut fork = parameters;
    fork["deferGoalContinuation"] = json!(true);
    let result = request("thread/fork", fork)
        .await
        .map_err(|error| format!("{}；保留历史的延续恢复失败：{error}", occupied()))?;
    if result["thread"]["id"]
        .as_str()
        .is_none_or(|id| id.is_empty() || id == source)
    {
        return Err(format!(
            "{}；延续恢复未返回独立会话 ID，原会话保持不变。",
            occupied()
        ));
    }
    Ok(OpenedThread {
        result,
        recovered_from: Some(source.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};

    async fn run(
        native: Option<&str>,
        loaded: bool,
        worker: bool,
        replies: Vec<Result<Value, String>>,
    ) -> (Result<OpenedThread, String>, Vec<(&'static str, Value)>) {
        let replies = RefCell::new(VecDeque::from(replies));
        let calls = RefCell::new(Vec::new());
        let result = open(native, loaded, worker, json!({"cwd":"中文项目","model":"selected","modelProvider":"configured",
            "approvalPolicy":"on-request","sandbox":"workspace-write","developerInstructions":"existing","serviceName":"supercode"}),
            |method, params| {
                calls.borrow_mut().push((method, params));
                std::future::ready(replies.borrow_mut().pop_front().expect("unexpected native request"))
            }).await;
        assert!(replies.borrow().is_empty());
        (result, calls.into_inner())
    }
    fn conflict() -> Result<Value, String> {
        Err("thread original already has an active writer".into())
    }

    #[tokio::test]
    async fn completed_private_worker_continues_history_with_one_bounded_fork_and_current_permissions(
    ) {
        let (result, calls) = run(
            Some("original"),
            false,
            true,
            vec![
                conflict(),
                Ok(json!({"data":[{"id":"last","status":"completed","items":[]}]})),
                Ok(json!({"thread":{"id":"continuation","turns":[]}})),
            ],
        )
        .await;
        let opened = result.unwrap();
        assert_eq!(opened.result["thread"]["id"], "continuation");
        assert_eq!(opened.recovered_from.as_deref(), Some("original"));
        assert_eq!(
            calls.iter().map(|c| c.0).collect::<Vec<_>>(),
            ["thread/resume", "thread/turns/list", "thread/fork"]
        );
        assert_eq!(
            calls[1].1,
            json!({"threadId":"original","limit":1,"sortDirection":"desc","itemsView":"notLoaded"})
        );
        let fork = &calls[2].1;
        for (key, value) in [
            ("threadId", "original"),
            ("cwd", "中文项目"),
            ("model", "selected"),
            ("modelProvider", "configured"),
            ("approvalPolicy", "on-request"),
            ("sandbox", "workspace-write"),
        ] {
            assert_eq!(fork[key], value);
        }
        assert_eq!(fork["excludeTurns"], true);
        assert_eq!(fork["deferGoalContinuation"], true);
        for removed in [
            "developerInstructions",
            "serviceName",
            "history",
            "attachToSource",
        ] {
            assert!(fork.get(removed).is_none());
        }
    }
    #[tokio::test]
    async fn ordinary_chats_unrelated_errors_and_ambiguous_requests_are_never_forked() {
        let (result, calls) = run(Some("original"), false, false, vec![conflict()]).await;
        assert!(result.err().unwrap().contains("无需关闭整个程序"));
        assert_eq!(calls.len(), 1);
        for error in [
            "Codex 请求超时：thread/resume",
            "authentication failed",
            "thread other already has an active writer",
        ] {
            let (result, calls) = run(Some("original"), false, true, vec![Err(error.into())]).await;
            assert_eq!(result.err().unwrap(), error);
            assert_eq!(calls.len(), 1);
        }
        let (result, calls) = run(None, false, true, vec![conflict()]).await;
        assert!(result.is_err());
        assert_eq!(calls[0].0, "thread/start");
        let (result, calls) = run(Some("original"), true, true, vec![]).await;
        assert_eq!(result.unwrap().result["thread"]["id"], "original");
        assert!(calls.is_empty());
    }
    #[tokio::test]
    async fn running_interrupted_missing_or_unreadable_history_never_starts_parallel_work() {
        for latest in [
            json!({"data":[{"status":"inProgress"}]}),
            json!({"data":[{"status":"interrupted"}]}),
            json!({"data":[{"status":"failed"}]}),
            json!({"data":[]}),
            json!({}),
        ] {
            let (result, calls) =
                run(Some("original"), false, true, vec![conflict(), Ok(latest)]).await;
            assert!(result.err().unwrap().contains("未自动延续"));
            assert_eq!(calls.len(), 2);
        }
        let (result, calls) = run(
            Some("original"),
            false,
            true,
            vec![conflict(), Err("unsupported method".into())],
        )
        .await;
        assert!(result.err().unwrap().contains("无法核对最近回合"));
        assert_eq!(calls.len(), 2);
    }
    #[tokio::test]
    async fn failed_or_ambiguous_forks_are_not_retried_or_bound_as_a_success() {
        for fork in [
            Err("timeout".into()),
            Ok(json!({})),
            Ok(json!({"thread":{"id":"original"}})),
            Ok(json!({"thread":{"id":""}})),
        ] {
            let (result, calls) = run(
                Some("original"),
                false,
                true,
                vec![
                    conflict(),
                    Ok(json!({"data":[{"status":"completed"}]})),
                    fork,
                ],
            )
            .await;
            assert!(result.is_err());
            assert_eq!(calls.len(), 3);
        }
        let (result, calls) = run(
            Some("original"),
            false,
            true,
            vec![Ok(json!({"thread":{"id":"original"}}))],
        )
        .await;
        assert!(result.unwrap().recovered_from.is_none());
        assert_eq!(calls.len(), 1);
    }
}
