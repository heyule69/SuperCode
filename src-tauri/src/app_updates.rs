//! Signed GitHub updates. Downloads stream to disk; active tasks are never interrupted.
use crate::{desktop_lifecycle, AppState};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::StreamExt;
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex;

const ENDPOINT: &str = "https://github.com/heyule69/SuperCode/releases/latest/download/latest.json";
const RELEASES: &str = "https://github.com/heyule69/SuperCode/releases";
const MAX_PACKAGE: u64 = 256 * 1024 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    current_version: String,
    latest_version: Option<String>,
    notes: String,
    phase: String,
    progress: u8,
    automatic: bool,
    checked_at: Option<u64>,
    error: Option<String>,
    release_url: String,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION").into(),
            latest_version: None,
            notes: String::new(),
            phase: "idle".into(),
            progress: 0,
            automatic: true,
            checked_at: None,
            error: None,
            release_url: RELEASES.into(),
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
struct Package {
    url: String,
    signature: String,
    sha256: String,
    size: u64,
}
#[derive(Clone, Deserialize, Serialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    platforms: BTreeMap<String, Package>,
}
#[derive(Default)]
struct Cache {
    status: Status,
    candidate: Option<Manifest>,
    ready: Option<PathBuf>,
}
#[derive(Default)]
pub struct AppUpdates {
    cache: Mutex<Cache>,
    operation: Mutex<()>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn supported() -> bool {
    cfg!(all(windows, target_arch = "x86_64"))
}
fn decode_text(value: &str) -> Result<String, String> {
    String::from_utf8(
        STANDARD
            .decode(value.trim())
            .map_err(|_| "更新签名编码无效")?,
    )
    .map_err(|_| "更新签名不是 UTF-8".into())
}
fn public_key() -> Result<PublicKey, String> {
    PublicKey::decode(&decode_text(include_str!("../update-public-key.txt"))?)
        .map_err(|_| "更新公钥无效".into())
}
fn signature(package: &Package, version: &str) -> Result<Signature, String> {
    let sig = Signature::decode(&decode_text(&package.signature)?).map_err(|_| "更新签名无效")?;
    // The trusted comment is authenticated by Minisign's global signature at finalize().
    if sig
        .trusted_comment()
        .split('\t')
        .find_map(|s| s.strip_prefix("version:"))
        != Some(version)
    {
        return Err("更新版本与签名不一致，请稍后重试。".into());
    }
    Ok(sig)
}
fn validate_manifest(manifest: &Manifest, current: &str) -> Result<bool, String> {
    let version = Version::parse(&manifest.version).map_err(|_| "更新版本无效")?;
    let current = Version::parse(current).map_err(|_| "当前版本无效")?;
    if !version.pre.is_empty() {
        return Err("自动更新仅使用正式版本。".into());
    }
    if version <= current {
        return Ok(false);
    }
    let package = manifest
        .platforms
        .get("windows-x86_64")
        .ok_or("暂无适用于此电脑的更新。")?;
    let expected = format!("{RELEASES}/download/v{version}/SuperCode_{version}_x64-setup.exe");
    if package.url != expected
        || package.size == 0
        || package.size > MAX_PACKAGE
        || package.sha256.len() != 64
        || !package.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || manifest.notes.len() > 16 * 1024
        || package.signature.len() > 4096
    {
        return Err("更新文件信息无效。".into());
    }
    signature(package, &manifest.version)?;
    Ok(true)
}
fn redirect_allowed(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
fn http(timeout: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("SuperCode/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(timeout))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 6 || !redirect_allowed(attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}
async fn fetch_manifest() -> Result<Option<Manifest>, String> {
    let response = http(25)?
        .get(ENDPOINT)
        .send()
        .await
        .map_err(|_| "无法连接 GitHub，请检查网络后重试。")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .map_err(|_| "GitHub 更新服务暂时不可用。")?;
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "读取更新信息失败。")?;
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err("更新信息过大。".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "更新信息格式无效。".into())
}
async fn emit(app: &AppHandle) -> Status {
    let status = app.state::<AppUpdates>().cache.lock().await.status.clone();
    let _ = app.emit("app-update", &status);
    status
}
#[tauri::command]
pub async fn app_update_status(app: AppHandle) -> Status {
    app.state::<AppUpdates>().cache.lock().await.status.clone()
}

#[tauri::command]
pub async fn check_app_update(app: AppHandle, force: Option<bool>) -> Result<Status, String> {
    let state = app.state::<AppUpdates>();
    let Ok(_operation) = state.operation.try_lock() else {
        return Ok(state.cache.lock().await.status.clone());
    };
    {
        let mut cache = state.cache.lock().await;
        if cache.status.phase == "ready"
            || cache.status.phase == "installing"
            || (!force.unwrap_or(false)
                && cache
                    .status
                    .checked_at
                    .is_some_and(|t| now().saturating_sub(t) < CHECK_INTERVAL.as_secs()))
        {
            return Ok(cache.status.clone());
        }
        if !supported() {
            cache.status.phase = "unsupported".into();
            return Ok(cache.status.clone());
        }
        cache.status.phase = "checking".into();
        cache.status.error = None;
    }
    emit(&app).await;
    let result = fetch_manifest().await.and_then(|manifest| {
        if let Some(m) = &manifest {
            validate_manifest(m, env!("CARGO_PKG_VERSION"))?;
        }
        Ok(manifest)
    });
    {
        let mut cache = state.cache.lock().await;
        cache.status.checked_at = Some(now());
        match result {
            Ok(Some(manifest)) => {
                let newer = validate_manifest(&manifest, env!("CARGO_PKG_VERSION"))?;
                cache.status.latest_version = Some(manifest.version.clone());
                cache.status.notes = manifest.notes.clone();
                cache.status.phase = if newer { "available" } else { "current" }.into();
                cache.candidate = newer.then_some(manifest);
            }
            Ok(None) => {
                cache.status.phase = "unpublished".into();
                cache.status.latest_version = None;
                cache.candidate = None;
            }
            Err(error) => {
                cache.status.phase = "error".into();
                cache.status.error = Some(error);
            }
        }
    }
    Ok(emit(&app).await)
}

struct PartialFile(PathBuf);
impl Drop for PartialFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn verify_file(path: &Path, package: &Package, version: &str) -> Result<(), String> {
    let key = public_key()?;
    let sig = signature(package, version)?;
    let mut verifier = key.verify_stream(&sig).map_err(|_| "更新签名不可验证")?;
    let mut file = fs::File::open(path).map_err(|_| "更新文件不存在，请重新下载。")?;
    let mut hash = Sha256::new();
    let mut count = 0;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|_| "无法读取更新文件")?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > package.size {
            return Err("更新文件大小不正确".into());
        }
        verifier.update(&buffer[..n]);
        hash.update(&buffer[..n]);
    }
    if count != package.size || format!("{:x}", hash.finalize()) != package.sha256.to_lowercase() {
        return Err("更新文件校验失败，请重新下载。".into());
    }
    verifier
        .finalize()
        .map_err(|_| "更新签名校验失败，已拒绝安装。".into())
}
async fn download(app: &AppHandle, manifest: &Manifest) -> Result<PathBuf, String> {
    let package = &manifest.platforms["windows-x86_64"];
    let dir = app.state::<AppState>().data_dir.join("updates");
    fs::create_dir_all(&dir).map_err(|_| "无法创建更新目录")?;
    let path = dir.join(format!("SuperCode_{}_x64-setup.exe", manifest.version));
    if path.exists() && verify_file(&path, package, &manifest.version).is_ok() {
        return Ok(path);
    }
    let temp = PartialFile(dir.join(format!("{}.part", uuid::Uuid::new_v4())));
    let response = http(300)?
        .get(&package.url)
        .send()
        .await
        .map_err(|_| "下载失败，请检查网络后重试。")?
        .error_for_status()
        .map_err(|_| "GitHub 安装包暂时不可下载。")?;
    if response.content_length().is_some_and(|n| n != package.size) {
        return Err("下载文件大小与发布信息不一致。".into());
    }
    let key = public_key()?;
    let sig = signature(package, &manifest.version)?;
    let mut verifier = key.verify_stream(&sig).map_err(|_| "更新签名不可验证")?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp.0)
        .map_err(|_| "无法保存更新文件")?;
    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64;
    let mut hash = Sha256::new();
    let mut percent = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "下载中断，原版本仍可使用，请重试。")?;
        downloaded += chunk.len() as u64;
        if downloaded > package.size {
            return Err("下载文件大小不正确。".into());
        }
        file.write_all(&chunk).map_err(|_| "无法写入更新文件")?;
        verifier.update(&chunk);
        hash.update(&chunk);
        let next = ((downloaded * 100 / package.size).min(99)) as u8;
        if next != percent {
            percent = next;
            app.state::<AppUpdates>().cache.lock().await.status.progress = percent;
            emit(app).await;
        }
    }
    if downloaded != package.size
        || format!("{:x}", hash.finalize()) != package.sha256.to_lowercase()
    {
        return Err("更新文件校验失败。".into());
    }
    verifier
        .finalize()
        .map_err(|_| "更新签名校验失败，已拒绝安装。")?;
    file.sync_all().map_err(|_| "无法保存更新文件")?;
    drop(file);
    fs::rename(&temp.0, &path).map_err(|_| "无法完成更新下载")?;
    Ok(path)
}
#[tauri::command]
pub async fn download_app_update(app: AppHandle) -> Result<Status, String> {
    let state = app.state::<AppUpdates>();
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "更新正在进行，请稍后。")?;
    let manifest = {
        let mut cache = state.cache.lock().await;
        let manifest = cache.candidate.clone().ok_or("请先检查更新。")?;
        if !validate_manifest(&manifest, env!("CARGO_PKG_VERSION"))? {
            return Err("无需更新。".into());
        }
        cache.status.phase = "downloading".into();
        cache.status.error = None;
        cache.status.progress = 0;
        manifest
    };
    emit(&app).await;
    let result = download(&app, &manifest).await;
    {
        let mut cache = state.cache.lock().await;
        match result {
            Ok(path) => {
                cache.ready = Some(path);
                cache.status.phase = "ready".into();
                cache.status.progress = 100;
            }
            Err(error) => {
                cache.status.phase = "available".into();
                cache.status.error = Some(error);
            }
        }
    }
    Ok(emit(&app).await)
}

#[tauri::command]
pub async fn install_app_update(app: AppHandle) -> Result<(), String> {
    if !supported() {
        return Err("当前平台不支持自动安装。".into());
    }
    let state = app.state::<AppUpdates>();
    let _operation = state.operation.try_lock().map_err(|_| "更新正在进行。")?;
    let (manifest, path) = {
        let cache = state.cache.lock().await;
        (
            cache.candidate.clone().ok_or("没有可用更新。")?,
            cache.ready.clone().ok_or("请先下载更新。")?,
        )
    };
    let package = &manifest.platforms["windows-x86_64"];
    verify_file(&path, package, &manifest.version)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let target = exe.parent().ok_or("无法确定安装目录")?;
    if exe
        .file_name()
        .is_none_or(|s| !s.eq_ignore_ascii_case("supercode.exe"))
        || !target.join("installation.json").is_file()
    {
        return Err("请先使用 Windows 安装包安装 SuperCode，再使用软件内更新。".into());
    }
    let request = path
        .parent()
        .unwrap()
        .join(format!("update-{}.json", uuid::Uuid::new_v4()));
    fs::write(
        &request,
        serde_json::to_vec(&serde_json::json!({"path": target, "version": manifest.version}))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|_| "无法准备更新")?;
    desktop_lifecycle::reserve_update_exit(&app)?;
    let result = (|| {
        let mut command = std::process::Command::new(&path);
        command.arg(format!("/UPDATE_REQUEST={}", request.display()));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command
            .spawn()
            .map_err(|_| "无法启动更新安装器，原版本仍可使用。")?;
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        desktop_lifecycle::cancel_update_exit(&app);
        let _ = fs::remove_file(&request);
        return Err(error);
    }
    state.cache.lock().await.status.phase = "installing".into();
    emit(&app).await;
    desktop_lifecycle::finish_update_exit(app.clone());
    Ok(())
}

#[tauri::command]
pub async fn set_automatic_app_updates(app: AppHandle, enabled: bool) -> Result<Status, String> {
    let state = app.state::<AppUpdates>();
    let mut cache = state.cache.lock().await;
    let path = app.state::<AppState>().data_dir.join("app-updates.json");
    let temp = path.with_extension("json.tmp");
    fs::write(
        &temp,
        serde_json::to_vec(&serde_json::json!({"automatic": enabled})).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    fs::rename(temp, path).map_err(|e| e.to_string())?;
    cache.status.automatic = enabled;
    drop(cache);
    Ok(emit(&app).await)
}
pub async fn watch(app: AppHandle) {
    let preference = app.state::<AppState>().data_dir.join("app-updates.json");
    if let Ok(text) = fs::read_to_string(preference) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(enabled) = value["automatic"].as_bool() {
                app.state::<AppUpdates>()
                    .cache
                    .lock()
                    .await
                    .status
                    .automatic = enabled;
            }
        }
    }
    tokio::time::sleep(Duration::from_secs(15)).await;
    loop {
        if app
            .state::<AppUpdates>()
            .cache
            .lock()
            .await
            .status
            .automatic
        {
            let _ = check_app_update(app.clone(), Some(false)).await;
        }
        tokio::time::sleep(CHECK_INTERVAL).await;
    }
}

pub async fn smoke(app: AppHandle) {
    let result = async {
        let disabled = set_automatic_app_updates(app.clone(), false).await?;
        if disabled.automatic {
            return Err("自动检查设置未保存".to_string());
        }
        set_automatic_app_updates(app.clone(), true).await?;
        let status = check_app_update(app.clone(), Some(true)).await?;
        if status.phase != "current"
            || status.latest_version.as_deref() != Some(env!("CARGO_PKG_VERSION"))
        {
            return Err(format!(
                "线上版本检查失败：{} {:?}",
                status.phase, status.error
            ));
        }
        let manifest = fetch_manifest().await?.ok_or("缺少已发布更新清单")?;
        let package = manifest
            .platforms
            .get("windows-x86_64")
            .ok_or("缺少 Windows 安装包")?;
        signature(package, &manifest.version)?;
        let downloaded = download(&app, &manifest).await?;
        verify_file(&downloaded, package, &manifest.version)?;
        Ok::<_, String>(serde_json::json!({"nativeRuntime": true, "isolated": true,
            "version": status.current_version, "githubCheck": true, "preferencePersistence": true,
            "downloadSignatureVerified": true, "bytes": package.size, "sha256": package.sha256}))
    }
    .await;
    let report = app
        .state::<AppState>()
        .data_dir
        .join("app-update-verification.json");
    let passed = result.is_ok();
    let value = match result {
        Ok(value) => value,
        Err(error) => serde_json::json!({"passed":false,"error":error}),
    };
    let _ = fs::write(report, serde_json::to_vec_pretty(&value).unwrap());
    app.exit(if passed { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Manifest, Vec<u8>) {
        let bytes = include_bytes!("../testdata/update-fixture.txt").to_vec();
        let version = "0.2.0";
        (
            Manifest {
                version: version.into(),
                notes: "测试".into(),
                platforms: BTreeMap::from([(
                    "windows-x86_64".into(),
                    Package {
                        url: format!(
                            "{RELEASES}/download/v{version}/SuperCode_{version}_x64-setup.exe"
                        ),
                        signature: include_str!("../testdata/update-fixture.txt.sig")
                            .trim()
                            .into(),
                        sha256: format!("{:x}", Sha256::digest(&bytes)),
                        size: bytes.len() as u64,
                    },
                )]),
            },
            bytes,
        )
    }
    #[test]
    fn versions_reject_downgrades_prereleases_and_unrelated_downloads() {
        let (mut m, _) = fixture();
        assert!(validate_manifest(&m, "0.1.0").unwrap());
        assert!(!validate_manifest(&m, "0.2.0").unwrap());
        assert!(!validate_manifest(&m, "0.3.0").unwrap());
        m.version = "0.2.0-beta.1".into();
        assert!(validate_manifest(&m, "0.1.0").is_err());
        let (mut m, _) = fixture();
        m.platforms.get_mut("windows-x86_64").unwrap().url =
            "https://github.com/attacker/app/a.exe".into();
        assert!(validate_manifest(&m, "0.1.0").is_err());
    }
    #[test]
    fn signed_version_cannot_be_changed_by_editing_manifest() {
        let (mut m, _) = fixture();
        m.version = "0.3.0".into();
        m.platforms.get_mut("windows-x86_64").unwrap().url =
            format!("{RELEASES}/download/v0.3.0/SuperCode_0.3.0_x64-setup.exe");
        assert!(validate_manifest(&m, "0.1.0").is_err());
    }
    #[test]
    fn verifies_signature_in_chunks_and_rejects_tampered_content_and_signature() {
        let (m, bytes) = fixture();
        let package = &m.platforms["windows-x86_64"];
        let path =
            std::env::temp_dir().join(format!("supercode-signature-{}", uuid::Uuid::new_v4()));
        fs::write(&path, &bytes).unwrap();
        assert!(verify_file(&path, package, &m.version).is_ok());
        let mut forged = bytes;
        forged[0] ^= 1;
        fs::write(&path, &forged).unwrap();
        let mut forged_package = package.clone();
        forged_package.sha256 = format!("{:x}", Sha256::digest(&forged));
        assert!(verify_file(&path, &forged_package, &m.version).is_err());
        let _ = fs::remove_file(path);
    }
    #[test]
    fn rejects_oversized_packages_and_insecure_redirects() {
        let (mut m, _) = fixture();
        m.platforms.get_mut("windows-x86_64").unwrap().size = MAX_PACKAGE + 1;
        assert!(validate_manifest(&m, "0.1.0").is_err());
        for url in [
            "http://github.com/x",
            "https://github.com.evil.test/x",
            "https://user:pass@github.com/x",
        ] {
            assert!(!redirect_allowed(&reqwest::Url::parse(url).unwrap()));
        }
        assert!(redirect_allowed(
            &reqwest::Url::parse("https://release-assets.githubusercontent.com/x").unwrap()
        ));
    }
}
