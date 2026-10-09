use crate::engine;
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Deserialize)]
pub struct Request {
    pub path: PathBuf,
    pub version: String,
}
pub fn read_request(path: &Path, version: &str) -> Result<Request, String> {
    if !path.is_absolute() || fs::metadata(path).map_err(|_| "更新请求不存在")?.len() > 4096
    {
        return Err("更新请求无效。".into());
    }
    let mut request: Request =
        serde_json::from_slice(&fs::read(path).map_err(|_| "无法读取更新请求")?)
            .map_err(|_| "更新请求格式无效")?;
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if matches!(request.path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::VerbatimDisk(_)))
        {
            request.path =
                PathBuf::from(request.path.to_string_lossy().trim_start_matches(r"\\?\"));
        }
    }
    engine::validate_target(&request.path)?;
    if request.version != version
        || !request.path.join("supercode.exe").is_file()
        || !request.path.join("installation.json").is_file()
    {
        return Err("更新请求与已安装版本不匹配。".into());
    }
    Ok(request)
}
pub fn wait_for_release(target: &Path) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match engine::ensure_not_running(target) {
            Ok(()) => return Ok(()),
            Err(error) if started.elapsed() >= Duration::from_secs(30) => return Err(error),
            Err(_) => std::thread::sleep(Duration::from_millis(150)),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_requires_installed_target_and_matching_signed_payload_version() {
        let target =
            std::env::temp_dir().join(format!("supercode-update-request-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&target).unwrap();
        let request = target.join("request.json");
        fs::write(
            &request,
            serde_json::to_vec(&serde_json::json!({"path":target,"version":"0.2.0"})).unwrap(),
        )
        .unwrap();
        assert!(read_request(&request, "0.2.0").is_err());
        fs::write(target.join("supercode.exe"), b"old version").unwrap();
        fs::write(target.join("installation.json"), b"{}").unwrap();
        assert!(read_request(&request, "0.2.0").is_ok());
        #[cfg(windows)]
        {
            let alias = fs::canonicalize(&target).unwrap();
            fs::write(
                &request,
                serde_json::to_vec(&serde_json::json!({"path":alias,"version":"0.2.0"})).unwrap(),
            )
            .unwrap();
            assert_eq!(read_request(&request, "0.2.0").unwrap().path, target);
        }
        assert!(read_request(&request, "0.3.0").is_err());
        fs::write(&request, b"{\"path\":\"../outside\",\"version\":\"0.2.0\"}").unwrap();
        assert!(read_request(&request, "0.2.0").is_err());
        let _ = fs::remove_dir_all(target);
    }
    #[cfg(windows)]
    #[test]
    fn waits_for_old_process_file_to_be_released_before_installing() {
        use std::os::windows::fs::OpenOptionsExt;
        let target =
            std::env::temp_dir().join(format!("supercode-update-wait-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("supercode.exe"), b"old program").unwrap();
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(target.join("supercode.exe"))
            .unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(locked);
        });
        let started = Instant::now();
        wait_for_release(&target).unwrap();
        assert!(started.elapsed() >= Duration::from_millis(150));
        release.join().unwrap();
        let _ = fs::remove_dir_all(target);
    }
}
