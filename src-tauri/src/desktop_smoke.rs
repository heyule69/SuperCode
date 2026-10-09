//! Opt-in native window/tray lifecycle checks in an isolated data directory.
//! Session status below is a fixture; this test never starts or approves an agent.
use crate::{desktop_lifecycle, tray_popup, AppState};
use serde_json::json;
use std::time::Duration;
use tauri::{AppHandle, Manager};

pub async fn run(app: AppHandle) {
    if std::env::args().any(|arg| arg == "--tray-hover-test") {
        let result = hover_fixture(&app).await;
        if result.is_err() {
            app.exit(1);
            return;
        }
        // Keep the real native popup available for bounded Windows input tests.
        tokio::time::sleep(Duration::from_secs(180)).await;
        app.exit(0);
        return;
    }
    let result = verify(&app).await;
    let report = match result {
        Ok(value) => json!({"ok":true,"result":value}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    let root = std::env::current_dir()
        .unwrap_or_default()
        .join(".supercode");
    let _ = std::fs::create_dir_all(&root);
    let _ = std::fs::write(
        root.join("desktop-smoke-report.json"),
        serde_json::to_vec_pretty(&report).unwrap_or_default(),
    );
    println!("{report}");
    if report["ok"] == true {
        // Exercise the same explicit quit handler as both File and tray menus.
        if desktop_lifecycle::request_app_exit(app.clone()).is_err() {
            app.exit(1);
        }
    } else {
        app.exit(1);
    }
}

async fn hover_fixture(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let root = state.data_dir.join("tray-hover-project");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let p = state.store.add_project(&root)?;
    let s = state.store.create_session(&p.id, None)?;
    state.store.rename(&s.id, "托盘悬停测试会话")?;
    desktop_lifecycle::show_window(app, "main")?;
    desktop_lifecycle::new_workspace_window(app.clone()).await?;
    tray_popup::open(app.clone(), tauri::PhysicalPosition::new(1400., 800.)).await?;
    Ok(())
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(400)).await;
}
async fn verify(app: &AppHandle) -> Result<serde_json::Value, String> {
    let main = app.get_webview_window("main").ok_or("主窗口不存在")?;
    if app.tray_by_id("supercode-tray").is_none() {
        return Err("未创建系统托盘".into());
    }
    let state = app.state::<AppState>();
    let project = state.store.add_project(&state.data_dir)?;
    let session = state.store.create_session(&project.id, None)?;
    state
        .store
        .set_status(&session.id, "running", Some("fixture-turn"))?;
    state.store.save_message(
        "fixture-reply",
        &session.id,
        "assistant",
        "保留对话",
        "agentMessage",
        &json!({"status":"completed"}),
    )?;
    main.maximize().map_err(|e| e.to_string())?;
    settle().await;
    main.close().map_err(|e| e.to_string())?;
    settle().await;
    if main.is_visible().map_err(|e| e.to_string())? || app.get_webview_window("main").is_none() {
        return Err("关闭主窗口未保留隐藏的 WebView".into());
    }
    if state.store.session(&session.id)?.status != "running"
        || state.store.messages(&session.id, None)?.len() != 1
    {
        return Err("关闭窗口改变了任务或对话".into());
    }
    desktop_lifecycle::show_window(app, "main")?;
    settle().await;
    if !main.is_visible().map_err(|e| e.to_string())?
        || !main.is_maximized().map_err(|e| e.to_string())?
    {
        return Err("恢复主窗口未保留最大化状态".into());
    }
    main.unmaximize().map_err(|e| e.to_string())?;
    main.set_size(tauri::LogicalSize::new(1000.0, 700.0))
        .map_err(|e| e.to_string())?;
    settle().await;
    main.close().map_err(|e| e.to_string())?;
    settle().await;
    let saved: serde_json::Value = serde_json::from_slice(
        &std::fs::read(state.data_dir.join("window-state.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if saved["maximized"] != false {
        return Err("恢复后窗口状态被冻结，无法记录还原".into());
    }
    let label = desktop_lifecycle::new_workspace_window(app.clone()).await?;
    let second = app.get_webview_window(&label).ok_or("新窗口不存在")?;
    settle().await;
    second.close().map_err(|e| e.to_string())?;
    settle().await;
    if second.is_visible().map_err(|e| e.to_string())? || app.webview_windows().len() != 2 {
        return Err("关闭新窗口未保留两个独立工作区".into());
    }
    desktop_lifecycle::show_window(app, &label)?;
    settle().await;
    if !second.is_visible().map_err(|e| e.to_string())? {
        return Err("无法单独恢复新窗口".into());
    }
    if app.get_webview_window(tray_popup::LABEL).is_some() {
        return Err("托盘菜单未按需创建".into());
    }
    tray_popup::open(app.clone(), tauri::PhysicalPosition::new(1200.0, 800.0)).await?;
    settle().await;
    let popup = app
        .get_webview_window(tray_popup::LABEL)
        .ok_or("未创建自定义托盘菜单")?;
    let snapshot = tray_popup::snapshot(app)?;
    if snapshot.token == 0 || snapshot.windows.len() != 2 {
        return Err("托盘菜单缺少窗口上下文".into());
    }
    tray_popup::present_tray_menu(popup.clone(), snapshot.token, 440.0, app.clone())?;
    settle().await;
    if !popup.is_visible().map_err(|e| e.to_string())? {
        return Err("托盘菜单未显示".into());
    }
    // This WebView has an isolated profile; exercise the actual storage/theme listener.
    for (theme, expected) in [("dark", tauri::Theme::Dark), ("light", tauri::Theme::Light)] {
        popup.eval(&format!("localStorage.setItem('supercode.preferences', JSON.stringify({{theme:'{theme}'}})); window.dispatchEvent(new StorageEvent('storage',{{key:'supercode.preferences'}}));")).map_err(|e| e.to_string())?;
        settle().await;
        if popup.theme().map_err(|e| e.to_string())? != expected {
            return Err(format!("托盘菜单未跟随 {theme} 主题"));
        }
    }
    tray_popup::dismiss(app);
    if popup.is_visible().map_err(|e| e.to_string())? {
        return Err("托盘菜单未收起".into());
    }
    // A stale renderer callback must never reopen a dismissed menu.
    tray_popup::present_tray_menu(popup.clone(), snapshot.token, 440.0, app.clone())?;
    if popup.is_visible().map_err(|e| e.to_string())? {
        return Err("过期托盘回调重新打开了菜单".into());
    }
    tokio::time::sleep(Duration::from_secs(31)).await;
    if app.get_webview_window(tray_popup::LABEL).is_some() {
        return Err("闲置托盘菜单未释放 WebView".into());
    }
    state.store.set_status(&session.id, "idle", None)?;
    Ok(
        json!({"nativeTray":true,"trayPopupOnDemand":true,"trayThemeFollowsPreferences":true,"trayIdleWebviewReleased":true,"staleTrayCallbackIgnored":true,"closePreservesWebviewsAndFixtureTask":true,"restoreKeepsMaximized":true,"geometryUpdatesAfterHide":true,"newWindowRestoresIndependently":true,"windows":app.webview_windows().len(),"dataDirectory":state.data_dir,"fixtureOnly":true}),
    )
}
