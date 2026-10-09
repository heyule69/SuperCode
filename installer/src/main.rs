#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod engine;
mod update;
#[cfg(windows)]
mod windows_install;
#[cfg(windows)]
mod windows_shutdown;
use engine::{Installed, Manifest, Progress};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

static PAYLOAD: &[u8] = include_bytes!("../generated/payload.zip");
static MANIFEST: &str = include_str!("../generated/manifest.json");
#[derive(Default)]
struct Installer {
    running: AtomicBool,
    completed: Mutex<Option<Installed>>,
    probe_report: Option<PathBuf>,
    update_request: Option<update::Request>,
}

fn default_path() -> PathBuf {
    #[cfg(windows)]
    if let Some(path) = windows_install::previous_install() {
        return path;
    }
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into())).join("SuperCode")
}
#[tauri::command]
fn installation_path(app: tauri::AppHandle) -> String {
    app.state::<Installer>()
        .update_request
        .as_ref()
        .map(|r| r.path.clone())
        .unwrap_or_else(default_path)
        .to_string_lossy()
        .into()
}

#[tauri::command]
async fn choose_directory(app: tauri::AppHandle) -> std::result::Result<Option<String>, String> {
    if app.state::<Installer>().running.load(Ordering::SeqCst)
        || app.state::<Installer>().update_request.is_some()
    {
        return Err("安装期间不能更换目录。".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择 SuperCode 安装目录")
            .set_directory(default_path())
            .blocking_pick_folder()
            .map(|path| path.to_string())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn begin_install(
    app: tauri::AppHandle,
    path: String,
) -> std::result::Result<Installed, String> {
    let state = app.state::<Installer>();
    if let Some(request) = &state.update_request {
        if request.path != PathBuf::from(&path) {
            return Err("更新期间不能更改安装目录。".into());
        }
    }
    if state.running.swap(true, Ordering::SeqCst) {
        return Err("安装正在进行。".into());
    }
    *state.completed.lock().map_err(|_| "无法读取安装状态")? = None;
    let task_app = app.clone();
    let updating = state.update_request.is_some();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let manifest: Manifest = serde_json::from_str(MANIFEST).map_err(|e| e.to_string())?;
        if updating {
            update::wait_for_release(&PathBuf::from(&path))?;
        }
        #[cfg(windows)]
        {
            windows_shutdown::prepare_upgrade(
                &PathBuf::from(&path),
                &windows_shutdown::user_data()?,
            )?;
            engine::install(
                &PathBuf::from(path),
                PAYLOAD,
                &manifest,
                &windows_install::WindowsRegistration { isolated: false },
                |event| {
                    let _ = task_app.emit("installation-progress", event);
                },
            )
        }
        #[cfg(not(windows))]
        {
            let _ = (manifest, task_app, path);
            Err("此安装器仅支持 Windows。".into())
        }
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    if let Ok(installed) = &result {
        *state.completed.lock().map_err(|_| "无法保存安装状态")? = Some(installed.clone());
    }
    state.running.store(false, Ordering::SeqCst);
    result
}

#[tauri::command]
fn launch_installed(app: tauri::AppHandle) -> std::result::Result<(), String> {
    let state = app.state::<Installer>();
    if state.running.load(Ordering::SeqCst) {
        return Err("安装尚未完成。".into());
    }
    let completed = state.completed.lock().map_err(|_| "无法读取安装状态")?;
    let installed = completed.as_ref().ok_or("安装尚未完成。")?;
    let exe = PathBuf::from(&installed.path).join("supercode.exe");
    let manifest: Manifest = serde_json::from_str(MANIFEST).map_err(|e| e.to_string())?;
    if engine::hash_file(&exe)?
        != manifest
            .entries
            .iter()
            .find(|e| e.name == "supercode.exe")
            .ok_or("安装包无效")?
            .sha256
    {
        return Err("安装文件已改变，请重新安装。".into());
    }
    #[cfg(windows)]
    {
        windows_install::hidden_command(exe)
            .current_dir(&installed.path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    app.exit(0);
    Ok(())
}
#[tauri::command]
fn show_installer(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    piece_count: u32,
) -> std::result::Result<bool, String> {
    if let Some(path) = &app.state::<Installer>().probe_report {
        engine::write_json(
            path,
            &serde_json::json!({"nativeIpcReady": true, "logoFragments": piece_count, "version": app.package_info().version.to_string()}),
        )?;
        app.exit(if piece_count == 40 { 0 } else { 1 });
        return Ok(false);
    }
    window.show().map_err(|e| e.to_string())?;
    Ok(app.state::<Installer>().update_request.is_some())
}
#[tauri::command]
fn close_installer(app: tauri::AppHandle) -> std::result::Result<(), String> {
    if app.state::<Installer>().running.load(Ordering::SeqCst) {
        return Err("安装正在进行，请等待完成。".into());
    }
    app.exit(0);
    Ok(())
}
fn main() {
    #[cfg(windows)]
    {
        let args: Vec<_> = std::env::args_os().collect();
        if args
            .get(1)
            .is_some_and(|a| a == "--verify-install" || a == "--verify-update")
        {
            let result = (|| {
                let manifest: Manifest =
                    serde_json::from_str(MANIFEST).map_err(|e| e.to_string())?;
                let automatic = args.get(1).is_some_and(|a| a == "--verify-update");
                let target = if automatic {
                    update::read_request(
                        &PathBuf::from(args.get(2).ok_or("缺少更新请求")?),
                        &manifest.version,
                    )?
                    .path
                } else {
                    PathBuf::from(args.get(2).ok_or("缺少测试目录")?)
                };
                // Verification may never target a registered production installation.
                if windows_install::previous_install()
                    .is_some_and(|p| windows_install::same_path(&p, &target))
                {
                    return Err("不能在正式安装目录运行隔离验证。".to_string());
                }
                if automatic {
                    update::wait_for_release(&target)?;
                }
                let data = args
                    .get(3)
                    .map(PathBuf::from)
                    .map(Ok)
                    .unwrap_or_else(windows_shutdown::user_data)?;
                // This is a read-only task-state check, not an installation
                // destination; the real application data directory is valid here.
                if !data.is_absolute() {
                    return Err("用户数据目录必须为完整路径。".into());
                }
                windows_shutdown::prepare_upgrade(&target, &data)?;
                let mut events: Vec<Progress> = vec![];
                let installed = engine::install(
                    &target,
                    PAYLOAD,
                    &manifest,
                    &windows_install::WindowsRegistration { isolated: true },
                    |p| events.push(p),
                )?;
                engine::write_json(
                    &target.join("verification.json"),
                    &serde_json::json!({ "installed": installed, "events": events, "isolated": true, "automaticUpdate": automatic }),
                )?;
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                if let Some(path) = args
                    .get(2)
                    .filter(|_| args.get(1).is_some_and(|a| a == "--verify-install"))
                {
                    let dest = PathBuf::from(path);
                    if engine::validate_target(&dest).is_ok() {
                        let _ = std::fs::create_dir_all(&dest);
                        let _ = engine::write_json(&dest.join("verification-error.json"), &error);
                    }
                }
                std::process::exit(1);
            }
            std::process::exit(0);
        }
    }
    let args: Vec<_> = std::env::args_os().collect();
    let update_request = if args.get(1).is_some_and(|a| a == "--update-request") {
        let manifest: Manifest =
            serde_json::from_str(MANIFEST).expect("Invalid installer manifest");
        match args.get(2).ok_or("缺少更新请求").and_then(|path| {
            update::read_request(&PathBuf::from(path), &manifest.version)
                .map_err(|_| "更新请求无效")
        }) {
            Ok(request) => Some(request),
            Err(_) => std::process::exit(2),
        }
    } else {
        None
    };
    let probe_report = if args.get(1).is_some_and(|a| a == "--verify-ui") {
        args.get(2).map(PathBuf::from).filter(|p| {
            p.is_absolute()
                && p.parent().is_some_and(|parent| {
                    engine::validate_target(parent).is_ok() && parent.exists()
                })
        })
    } else {
        None
    };
    tauri::Builder::default()
        .manage(Installer {
            probe_report,
            update_request,
            ..Installer::default()
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            installation_path,
            choose_directory,
            begin_install,
            launch_installed,
            show_installer,
            close_installer
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.state::<Installer>().running.load(Ordering::SeqCst) {
                    api.prevent_close();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Unable to start SuperCode installer");
}
