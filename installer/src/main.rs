#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod engine;
#[cfg(windows)]
mod windows_install;
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
}

fn default_path() -> PathBuf {
    #[cfg(windows)]
    if let Some(path) = windows_install::previous_install() {
        return path;
    }
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into())).join("SuperCode")
}
#[tauri::command]
fn installation_path() -> String {
    default_path().to_string_lossy().into()
}

#[tauri::command]
async fn choose_directory(app: tauri::AppHandle) -> std::result::Result<Option<String>, String> {
    if app.state::<Installer>().running.load(Ordering::SeqCst) {
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
    if state.running.swap(true, Ordering::SeqCst) {
        return Err("安装正在进行。".into());
    }
    *state.completed.lock().map_err(|_| "无法读取安装状态")? = None;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let manifest: Manifest = serde_json::from_str(MANIFEST).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
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
) -> std::result::Result<(), String> {
    if let Some(path) = &app.state::<Installer>().probe_report {
        engine::write_json(
            path,
            &serde_json::json!({"nativeIpcReady": true, "logoFragments": piece_count, "version": app.package_info().version.to_string()}),
        )?;
        app.exit(if piece_count == 40 { 0 } else { 1 });
        return Ok(());
    }
    window.show().map_err(|e| e.to_string())
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
        if args.get(1).is_some_and(|a| a == "--verify-install") {
            let result = (|| {
                let target = PathBuf::from(args.get(2).ok_or("缺少测试目录")?);
                // Verification may never target a registered production installation.
                if windows_install::previous_install()
                    .is_some_and(|p| windows_install::same_path(&p, &target))
                {
                    return Err("不能在正式安装目录运行隔离验证。".to_string());
                }
                let manifest: Manifest =
                    serde_json::from_str(MANIFEST).map_err(|e| e.to_string())?;
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
                    &serde_json::json!({ "installed": installed, "events": events, "isolated": true }),
                )?;
                Ok::<_, String>(())
            })();
            if let Err(error) = result {
                if let Some(path) = args.get(2) {
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
