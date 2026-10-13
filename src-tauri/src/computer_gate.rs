//! A kernel event closes input immediately, independently of the WebView/UI thread.
pub const INPUT_TAG: usize = 0x53434f44;
pub const ENVIRONMENT: &str = "SUPERCODE_COMPUTER_STOP_EVENT";
pub(crate) const FOREGROUND_CHANGED: &str = "Target is no longer the foreground window.";

pub(crate) fn verify_foreground(
    matches: bool,
    revoke: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if matches { return Ok(()); }
    // Focus loss closes the same authoritative input gate as user takeover;
    // a model-facing instruction alone cannot prevent a subsequent refocus.
    let failure = revoke().err().map(|error| format!(" ({error})")).unwrap_or_default();
    Err(format!("{FOREGROUND_CHANGED} Desktop control revoked{failure}. Continue conversation or permitted non-control work; do not refocus without renewed user authorization"))
}
pub fn event_name(token: &str) -> String {
    format!("Local\\SuperCodeComputer-{token}")
}

#[cfg(windows)]
mod native {
    use windows_sys::Win32::{Foundation::*, System::Threading::*};
    pub struct Gate(HANDLE);
    // Windows event handles support concurrent SetEvent/WaitForSingleObject.
    unsafe impl Send for Gate {}
    unsafe impl Sync for Gate {}
    fn wide(name: &str) -> Vec<u16> {
        name.encode_utf16().chain(Some(0)).collect()
    }
    impl Gate {
        pub fn create(name: &str) -> Result<Self, String> {
            let handle = unsafe { CreateEventW(std::ptr::null(), 1, 1, wide(name).as_ptr()) };
            if handle.is_null() {
                Err("无法创建电脑操作停止机制".into())
            } else {
                Ok(Self(handle))
            }
        }
        pub fn open(name: &str) -> Result<Self, String> {
            let handle = unsafe { OpenEventW(0x0010_0000, 0, wide(name).as_ptr()) };
            if handle.is_null() {
                Err("电脑操作控制连接已关闭，请回到 SuperCode 重新开始".into())
            } else {
                Ok(Self(handle))
            }
        }
        pub fn revoke(name: &str) -> Result<(), String> {
            let handle=unsafe { OpenEventW(EVENT_MODIFY_STATE,0,wide(name).as_ptr()) };
            if handle.is_null() { return Err("电脑操作停止机制已关闭".into()); }
            let stopped=unsafe { SetEvent(handle) } != 0;
            unsafe { CloseHandle(handle); }
            if stopped { Ok(()) } else { Err("无法停止电脑输入".into()) }
        }
        pub fn stop(&self) {
            unsafe {
                SetEvent(self.0);
            }
        }
        pub fn arm(&self) -> Result<(), String> {
            if unsafe { ResetEvent(self.0) } == 0 {
                Err("无法启用电脑操作停止机制".into())
            } else {
                Ok(())
            }
        }
        pub fn stopped(&self) -> bool {
            (unsafe { WaitForSingleObject(self.0, 0) }) != WAIT_TIMEOUT
        }
        pub fn raw(&self) -> usize {
            self.0 as usize
        }
    }
    impl Drop for Gate {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
#[cfg(windows)]
pub use native::Gate;
#[cfg(not(windows))]
pub struct Gate(std::sync::atomic::AtomicBool);
#[cfg(not(windows))]
impl Gate {
    pub fn create(_: &str) -> Result<Self, String> {
        Ok(Self(std::sync::atomic::AtomicBool::new(true)))
    }
    pub fn stop(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
    pub fn arm(&self) -> Result<(), String> {
        self.0.store(false, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    pub fn stopped(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}
#[cfg(windows)]
pub fn input_allowed() -> Result<Gate, String> {
    let name =
        std::env::var(ENVIRONMENT).map_err(|_| "电脑操作必须由 SuperCode 管理，停止机制未连接")?;
    let gate = Gate::open(&name)?;
    if gate.stopped() {
        return Err(
            "用户已接管电脑，电脑输入已停止。继续对话或其他已授权工作；再次操作电脑前需用户重新授权，不要重试或重新抢占焦点".into(),
        );
    }
    Ok(gate)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gate_starts_closed_and_can_stop_without_ui_work() {
        let name = event_name(&uuid::Uuid::new_v4().to_string());
        let gate = Gate::create(&name).unwrap();
        assert!(gate.stopped());
        gate.arm().unwrap();
        assert!(!gate.stopped());
        #[cfg(windows)]
        {
            assert!(!Gate::open(&name).unwrap().stopped());
            Gate::revoke(&name).unwrap();
            assert!(gate.stopped());
            gate.arm().unwrap();
        }
        gate.stop();
        assert!(gate.stopped());
        #[cfg(windows)]
        {
            assert!(Gate::open(&name).unwrap().stopped());
        }
    }
    #[test]
    fn an_old_worker_gate_stays_revoked_when_a_new_turn_is_armed() {
        let old = Gate::create(&event_name(&uuid::Uuid::new_v4().to_string())).unwrap();
        old.arm().unwrap();
        old.stop();
        let new = Gate::create(&event_name(&uuid::Uuid::new_v4().to_string())).unwrap();
        new.arm().unwrap();
        assert!(old.stopped());
        assert!(!new.stopped());
        new.stop();
    }
    #[test]
    fn foreground_loss_revokes_input_without_operating_the_desktop() {
        let name = event_name(&uuid::Uuid::new_v4().to_string());
        let gate = Gate::create(&name).unwrap();
        gate.arm().unwrap();
        verify_foreground(true, || panic!("safe foreground must retain its grant")).unwrap();
        assert!(!gate.stopped());
        let error = verify_foreground(false, || {
            #[cfg(windows)]
            { Gate::revoke(&name) }
            #[cfg(not(windows))]
            { gate.stop(); Ok(()) }
        }).unwrap_err();
        assert!(error.starts_with(FOREGROUND_CHANGED));
        assert!(gate.stopped());
        #[cfg(windows)]
        assert!(Gate::open(&name).unwrap().stopped());
    }
}
