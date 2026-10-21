//! A bounded summary can take longer than two minutes while still making progress.
use std::{future::Future, time::Duration};
use tokio::{sync::watch, time::Instant};

const IDLE: Duration = Duration::from_secs(300);
pub(crate) const MAX_SECONDS: u64 = 900;

#[derive(Clone)]
pub(crate) struct Progress(watch::Sender<Instant>);

impl Progress {
    pub(crate) fn touch(&self) {
        // Keep only the latest timestamp, never a queue of text or progress events.
        self.0.send_replace(Instant::now());
    }
}

pub(crate) struct SummaryWait {
    progress: Progress,
    changed: watch::Receiver<Instant>,
    deadline: Instant,
}

impl SummaryWait {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        let (sender, changed) = watch::channel(now);
        Self {
            progress: Progress(sender),
            changed,
            deadline: now + Duration::from_secs(MAX_SECONDS),
        }
    }

    pub(crate) fn progress(&self) -> Progress {
        self.progress.clone()
    }

    pub(crate) async fn run<T>(
        mut self,
        operation: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        tokio::pin!(operation);
        loop {
            let idle = *self.changed.borrow_and_update() + IDLE;
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(self.deadline) => return Err("压缩等待已达 15 分钟，已停止；原模型和聊天记录未变，可以重试".into()),
                result = &mut operation => return result,
                // Observe an already delivered update before an old idle timer.
                _ = self.changed.changed() => {},
                _ = tokio::time::sleep_until(idle) => return Err("压缩连续 5 分钟未收到模型进展，已停止；请检查连接后重试".into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn slow_first_response_and_progress_beyond_two_minutes_can_finish() {
        let wait = SummaryWait::new();
        let progress = wait.progress();
        let started = Instant::now();
        let result = wait
            .run(async {
                tokio::time::sleep(Duration::from_secs(180)).await;
                progress.touch();
                tokio::time::sleep(Duration::from_secs(240)).await;
                progress.touch();
                tokio::time::sleep(Duration::from_secs(240)).await;
                Ok("摘要")
            })
            .await;
        assert_eq!(result.unwrap(), "摘要");
        assert_eq!(started.elapsed(), Duration::from_secs(660));
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_operation_is_dropped_after_idle_limit() {
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct Guard(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let guard = Guard(dropped.clone());
        let started = Instant::now();
        let result: Result<(), _> = SummaryWait::new()
            .run(async {
                let _guard = guard;
                std::future::pending().await
            })
            .await;
        assert!(result.unwrap_err().contains("5 分钟"));
        assert_eq!(started.elapsed(), IDLE);
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn endless_progress_cannot_remove_total_limit() {
        let wait = SummaryWait::new();
        let progress = wait.progress();
        let started = Instant::now();
        let result: Result<(), _> = wait
            .run(async {
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    progress.touch();
                }
            })
            .await;
        assert!(result.unwrap_err().contains("15 分钟"));
        assert_eq!(started.elapsed(), Duration::from_secs(MAX_SECONDS));
    }
}
