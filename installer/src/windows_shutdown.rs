//! Ask only the selected installation to quit; never force-kill a process.
use crate::{engine, update, windows_install::same_path};
#[path = "../../src-tauri/src/installation_exit_protocol.rs"]
mod protocol;
use std::{
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_MORE_DATA, HANDLE, HWND, LPARAM},
    System::{
        RestartManager::*,
        Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION},
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, RegisterWindowMessageW, SendMessageTimeoutW,
        SMTO_ABORTIFHUNG, SMTO_BLOCK,
    },
};

struct Session(u32);
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            RmEndSession(self.0);
        }
    }
}
impl Session {
    fn new() -> Result<Self, String> {
        let mut handle = 0;
        let mut key = [0u16; 33];
        check(unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) })?;
        Ok(Self(handle))
    }
    fn processes(&self) -> Result<Vec<RM_PROCESS_INFO>, String> {
        let mut count = 0;
        let mut needed = 0;
        let mut reasons = 0;
        for _ in 0..4 {
            let mut entries: Vec<RM_PROCESS_INFO> =
                (0..count).map(|_| unsafe { std::mem::zeroed() }).collect();
            let code = unsafe {
                RmGetList(
                    self.0,
                    &mut needed,
                    &mut count,
                    if entries.is_empty() {
                        ptr::null_mut()
                    } else {
                        entries.as_mut_ptr()
                    },
                    &mut reasons,
                )
            };
            if code == ERROR_MORE_DATA && needed <= 1024 {
                count = needed;
                continue;
            }
            check(code)?;
            entries.truncate(count as usize);
            return Ok(entries);
        }
        Err("占用程序列表发生变化，请重试。".into())
    }
}
fn check(code: u32) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!("无法请求程序退出（Windows 错误 {code}）。"))
    }
}
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
struct Process(HANDLE);
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn process_path(pid: u32) -> Result<PathBuf, String> {
    let process = Process(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) });
    if process.0.is_null() {
        return Err("无法检查占用程序，请重试。".into());
    }
    let mut path = vec![0u16; 32768];
    let mut len = path.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut len) } == 0 {
        return Err("无法检查占用程序的路径。".into());
    }
    Ok(PathBuf::from(
        String::from_utf16(&path[..len as usize]).map_err(|e| e.to_string())?,
    ))
}
struct ExitRequest {
    pid: u32,
    message: u32,
    response: isize,
    timed_out: bool,
}
unsafe extern "system" fn ask_window(hwnd: HWND, data: LPARAM) -> i32 {
    let request = &mut *(data as *mut ExitRequest);
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid != request.pid {
        return 1;
    }
    let mut reply = 0;
    if SendMessageTimeoutW(
        hwnd,
        request.message,
        0,
        0,
        SMTO_ABORTIFHUNG | SMTO_BLOCK,
        1000,
        &mut reply,
    ) == 0
    {
        request.timed_out = true;
    } else if reply as isize == protocol::ACCEPTED || reply as isize == protocol::BUSY {
        request.response = reply as isize;
        return 0;
    }
    1
}
fn ask_process(pid: u32) -> Result<isize, String> {
    let name: Vec<u16> = protocol::MESSAGE.encode_utf16().chain(Some(0)).collect();
    let message = unsafe { RegisterWindowMessageW(name.as_ptr()) };
    if message == 0 {
        return Err("无法请求 SuperCode 退出。".into());
    }
    let mut request = ExitRequest {
        pid,
        message,
        response: 0,
        timed_out: false,
    };
    unsafe {
        EnumWindows(Some(ask_window), &mut request as *mut ExitRequest as isize);
    }
    if request.response == 0 && request.timed_out {
        return Err("SuperCode 暂时没有响应，请稍后重试更新。".into());
    }
    Ok(request.response)
}
fn legacy_is_idle(data: &Path) -> Result<(), String> {
    let path = data.join("supercode.db");
    if !path.exists() {
        return Ok(());
    }
    let db =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "无法检查旧版任务状态，请先从托盘退出 SuperCode。")?;
    db.busy_timeout(std::time::Duration::from_secs(1))
        .map_err(|e| e.to_string())?;
    let active: i64 = db
        .query_row(
            "SELECT count(*) FROM sessions WHERE status IN ('starting','running','waiting')",
            [],
            |row| row.get(0),
        )
        .map_err(|_| "无法检查旧版任务状态，请先从托盘退出 SuperCode。")?;
    if active != 0 {
        return Err("还有任务正在运行或等待确认，请完成任务后再更新。".into());
    }
    Ok(())
}

pub fn prepare_upgrade(target: &Path, data: &Path) -> Result<(), String> {
    engine::validate_target(target)?;
    if engine::ensure_not_running(target).is_ok() {
        return Ok(());
    }
    let exe = target.join("supercode.exe");
    let session = Session::new()?;
    let resource = wide(&exe);
    let resources = [resource.as_ptr()];
    check(unsafe {
        RmRegisterResources(
            session.0,
            1,
            resources.as_ptr(),
            0,
            ptr::null(),
            0,
            ptr::null(),
        )
    })?;
    let entries = session.processes()?;
    // Restart Manager's file list can also contain antivirus/indexer processes.
    // Do not request shutdown for anything other than this exact executable.
    for entry in &entries {
        let path = match process_path(entry.Process.dwProcessId) {
            Ok(path) => path,
            Err(_) if engine::ensure_not_running(target).is_ok() => return Ok(()),
            Err(error) => return Err(error),
        };
        if !same_path(&path, &exe) {
            return Err("安装文件被其他程序占用，请稍后重试。".into());
        }
    }
    let mut legacy = vec![];
    for entry in entries {
        match ask_process(entry.Process.dwProcessId)? {
            protocol::BUSY => return Err("还有任务正在运行或等待确认，请完成任务后再更新。".into()),
            protocol::ACCEPTED => {}
            _ => legacy.push(entry.Process),
        }
    }
    if !legacy.is_empty() {
        legacy_is_idle(data)?;
        // Register PID + creation time, rather than files, so newly appearing
        // unrelated resource users can never be closed by this fallback.
        let shutdown = Session::new()?;
        check(unsafe {
            RmRegisterResources(
                shutdown.0,
                0,
                ptr::null(),
                legacy.len() as u32,
                legacy.as_ptr(),
                0,
                ptr::null(),
            )
        })?;
        check(unsafe { RmShutdown(shutdown.0, 0, None) })?; // No RmForceShutdown.
    }
    update::wait_for_release(target)
}

pub fn user_data() -> Result<PathBuf, String> {
    std::env::var_os("APPDATA")
        .map(|p| PathBuf::from(p).join("dev.supercode.desktop"))
        .ok_or_else(|| "无法定位 SuperCode 用户数据目录。".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_tasks_are_checked_read_only_and_fail_closed() {
        let root =
            std::env::temp_dir().join(format!("supercode-shutdown-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let db = rusqlite::Connection::open(root.join("supercode.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE sessions(status TEXT); INSERT INTO sessions VALUES('completed');",
        )
        .unwrap();
        assert!(legacy_is_idle(&root).is_ok());
        for status in ["starting", "running", "waiting"] {
            db.execute("INSERT INTO sessions VALUES(?1)", [status])
                .unwrap();
            assert!(legacy_is_idle(&root).is_err());
            db.execute("DELETE FROM sessions WHERE status=?1", [status])
                .unwrap();
        }
        db.execute_batch("DROP TABLE sessions").unwrap();
        assert!(legacy_is_idle(&root).is_err());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn idle_or_missing_installations_need_no_shutdown() {
        let root =
            std::env::temp_dir().join(format!("supercode-shutdown-idle-{}", uuid::Uuid::new_v4()));
        assert!(prepare_upgrade(&root, &root).is_ok());
        assert!(prepare_upgrade(Path::new("../outside"), &root).is_err());
    }
}
