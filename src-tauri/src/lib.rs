mod accounts;
mod official_accounts;
mod agent_commands;
mod agent_smoke;
mod agent_versions;
mod agents;
mod app_updates;
mod automation;
mod bridge;
mod ccswitch;
mod chat_connection;
mod claude;
mod client_features;
mod commands;
mod connection_order;
mod credentials;
mod desktop_lifecycle;
mod desktop_smoke;
mod extensions;
mod followup_smoke;
mod handoff;
mod image_attachments;
mod installation_exit;
mod installation_exit_protocol;
mod links;
mod media;
mod native_agents;
mod notifications;
mod outbox;
mod platform_usage;
mod process;
mod protocol;
mod providers;
mod runtime;
mod session_config;
mod sidebar;
mod skills;
mod smoke;
mod storage;
mod tray_popup;
mod ui_recovery;
mod window_state;
mod window_theme;

use tauri::Manager;

pub struct AppState {
    pub store: storage::Store,
    pub data_dir: std::path::PathBuf,
    pub runtime: runtime::Runtime,
    pub claude: claude::Runtime,
    pub native: native_agents::Runtime,
    pub activity: std::sync::Mutex<protocol::ActivityBuffer>,
    pub change_snapshots:
        std::sync::Mutex<std::collections::HashMap<String, (String, Option<String>)>>,
}

pub fn run() {
    let desktop_test = std::env::args()
        .any(|arg| matches!(arg.as_str(), "--desktop-smoke-test" | "--tray-hover-test"));
    let installation_test = std::env::args().any(|arg| {
        matches!(
            arg.as_str(),
            "--installation-exit-test" | "--legacy-installation-exit-test"
        )
    });
    let followup_test = std::env::args().any(|arg| arg == "--followups-smoke-test");
    let smoke_test = std::env::args().any(|arg| arg == "--smoke-test");
    let automation_test = std::env::args().any(|arg| arg == "--automation-smoke-test");
    let update_test = std::env::args().any(|arg| arg == "--app-updates-test");
    let agents_test = std::env::args().any(|arg| {
        matches!(
            arg.as_str(),
            "--agents-smoke-test"
                | "--agents-mcp-test"
                | "--agent-updates-test"
                | "--provider-accounts-test"
        )
    });
    let smoke_dir = (smoke_test
        || agents_test
        || desktop_test
        || followup_test
        || update_test
        || installation_test)
        .then(|| {
            std::env::current_dir()
                .unwrap_or_default()
                .join(".supercode/smoke")
                .join(if agents_test {
                    format!("Agent test 测试 {}", uuid::Uuid::new_v4())
                } else {
                    uuid::Uuid::new_v4().to_string()
                })
        });
    let mut context = tauri::generate_context!();
    if let Some(dir) = &smoke_dir {
        for window in &mut context.config_mut().app.windows {
            window.data_directory = Some(dir.join("webview-data"));
            window.visible = false;
        }
    }
    if automation_test {
        for window in &mut context.config_mut().app.windows {
            window.visible = false;
        }
    }
    let builder = tauri::Builder::default();
    let builder = if smoke_test
        || automation_test
        || agents_test
        || desktop_test
        || followup_test
        || installation_test
    {
        builder
    } else {
        builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
    };
    builder
        .manage(outbox::Outbox::default())
        .manage(media::Media::default())
        .register_asynchronous_uri_scheme_protocol(
            "supercode-media",
            |context, request, responder| {
                let app = context.app_handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    responder.respond(media::response(&app.state::<media::Media>(), request));
                });
            },
        )
        .manage(automation::Automation::default())
        .manage(agents::Agents::default())
        .manage(agent_versions::Versions::default())
        .manage(app_updates::AppUpdates::default())
        .manage(accounts::Accounts::default())
        .manage(official_accounts::Logins::default())
        .manage(notifications::Notifications::default())
        .manage(platform_usage::UsageCache::default())
        .manage(ui_recovery::UiRecovery::default())
        .manage(desktop_lifecycle::DesktopLifecycle::default())
        .manage(tray_popup::TrayPopup::default())
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                ui_recovery::attach(webview);
            }
        })
        .manage(window_theme::WindowTheme::default())
        .on_window_event(|window, event| {
            window_theme::on_window_event(window, event);
            if !desktop_lifecycle::on_window_event(window, event) {
                window_state::on_window_event(window, event);
            }
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let dir = if let Some(dir) = &smoke_dir {
                dir.clone()
            } else {
                app.path().app_data_dir()?
            };
            std::fs::create_dir_all(&dir)?;
            app.manage(AppState {
                store: storage::Store::open(&dir.join("supercode.db"))?,
                data_dir: dir.clone(),
                runtime: runtime::Runtime::default(),
                claude: claude::Runtime::default(),
                native: native_agents::Runtime::default(),
                activity: std::sync::Mutex::new(protocol::ActivityBuffer::default()),
                change_snapshots: std::sync::Mutex::new(std::collections::HashMap::new()),
            });
            if !std::env::args().any(|arg| arg == "--legacy-installation-exit-test") {
                installation_exit::attach(app.handle())?;
            }
            let outbox_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move { outbox::watch(outbox_handle).await });
            let handle = app.handle().clone();
            if installation_test {
                installation_exit::smoke(app.handle().clone())?;
            } else if update_test {
                if let Some(window) = app.get_webview_window("main") {
                    window.hide()?;
                }
                tauri::async_runtime::spawn(async move { app_updates::smoke(handle).await });
            } else if followup_test {
                tauri::async_runtime::spawn(async move { followup_smoke::run(handle).await });
            } else if desktop_test {
                window_state::restore(app, &dir)?;
                desktop_lifecycle::install(app.handle())?;
                tauri::async_runtime::spawn(async move { desktop_smoke::run(handle).await });
            } else if agents_test {
                tauri::async_runtime::spawn(async move {
                    agent_smoke::run(handle).await;
                });
            } else if automation_test {
                tauri::async_runtime::spawn(async move {
                    automation::smoke(handle).await;
                });
            } else if smoke_test {
                if let Some(window) = app.get_webview_window("main") {
                    window.hide()?;
                }
                tauri::async_runtime::spawn(async move {
                    smoke::run(handle).await;
                });
            } else {
                window_state::restore(app, &dir)?;
                if let Err(error) = desktop_lifecycle::install(app.handle()) {
                    // Keep ordinary close available if the OS has no tray.
                    eprintln!("系统托盘不可用：{error}");
                }
                let update_handle = handle.clone();
                tauri::async_runtime::spawn(async move { app_updates::watch(update_handle).await });
                tauri::async_runtime::spawn(async move { runtime::idle_watch(handle).await });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_updates::app_update_status,
            app_updates::check_app_update,
            app_updates::download_app_update,
            app_updates::install_app_update,
            app_updates::set_automatic_app_updates,
            notifications::update_notification_preferences,
            notifications::update_notification_context,
            notifications::notification_status,
            notifications::test_system_notification,
            notifications::open_system_notification_settings,
            agents::list_agents,
            agents::test_agent,
            agents::install_agent,
            agents::update_agent,
            agent_versions::check_agent_updates,
            agents::cancel_agent_install,
            agents::configure_agent,
            automation::list_automation,
            automation::install_automation,
            automation::test_automation,
            automation::cancel_automation,
            automation::set_automation_enabled,
            window_theme::set_window_colors,
            links::open_external_link,
            media::prepare_media,
            media::open_media,
            commands::bootstrap,
            ui_recovery::get_ui_recovery,
            commands::pending_requests,
            session_config::switch_session_model,
            session_config::compact_session_context,
            sidebar::get_sidebar_state,
            sidebar::update_sidebar_item,
            sidebar::mark_session_read,
            sidebar::save_sidebar_section,
            sidebar::delete_sidebar_section,
            sidebar::edit_sidebar_project,
            sidebar::sidebar_project_action,
            sidebar::move_sidebar_session,
            sidebar::fork_sidebar_session,
            sidebar::list_archived_sessions,
            sidebar::restore_sidebar_session,
            sidebar::delete_sidebar_session,
            sidebar::open_sidebar_project,
            sidebar::open_chat_window,
            desktop_lifecycle::new_workspace_window,
            desktop_lifecycle::request_app_exit,
            tray_popup::tray_menu_snapshot,
            tray_popup::present_tray_menu,
            tray_popup::tray_menu_action,
            sidebar::copy_chat_transcript,
            sidebar::save_chat_export,
            commands::configure_codex,
            commands::add_project,
            commands::create_session,
            chat_connection::check_chat_connection,
            commands::list_messages,
            commands::send_message,
            commands::send_chat_message,
            outbox::enqueue_followup,
            outbox::list_followups,
            outbox::followup_capabilities,
            outbox::change_followup,
            outbox::steer_followup,
            image_attachments::import_image_attachment,
            image_attachments::read_image_attachment,
            commands::interrupt_turn,
            commands::respond_request,
            commands::list_models,
            providers::get_model_source,
            commands::workspace_changes,
            commands::read_project_file,
            commands::open_project_path,
            commands::rename_session,
            commands::archive_session,
            commands::runtime_info,
            commands::release_runtime,
            ccswitch::scan_ccswitch,
            ccswitch::import_ccswitch,
            ccswitch::select_agent_profile,
            providers::provider_catalog,
            providers::get_provider_profile,
            providers::save_provider_profile,
            providers::delete_provider_profile,
            connection_order::reorder_provider_connections,
            providers::fetch_provider_models,
            providers::test_provider_connection,
            accounts::official_account_status,
            accounts::list_provider_accounts,
            accounts::use_official_account,
            official_accounts::list_official_accounts,
            official_accounts::start_official_account_login,
            official_accounts::finish_official_account_login,
            official_accounts::cancel_official_account_login,
            official_accounts::rename_official_account,
            official_accounts::remove_official_account,
            agent_commands::execute_agent_command,
            client_features::get_client_preferences,
            client_features::save_client_preferences,
            client_features::get_usage,
            client_features::get_account_limits,
            platform_usage::get_platform_usage,
            platform_usage::get_quota_connections,
            client_features::list_tool_servers,
            client_features::save_tool_servers,
            client_features::tool_server_status,
            client_features::list_local_skills,
            client_features::read_skill,
            client_features::create_local_skill,
            client_features::agent_extensions,
            extensions::list_extensions,
            extensions::set_extension_enabled,
            extensions::import_extension,
            extensions::open_extension_folder,
        ])
        .build(context)
        .expect("无法启动 SuperCode")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { code, ref api, .. } = event {
                desktop_lifecycle::on_exit_requested(app, code, api);
            }
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                window_state::flush(app);
            }
            if matches!(event, tauri::RunEvent::Exit) {
                let state = app.state::<AppState>();
                if let Ok(mut client) = state.runtime.client.try_lock() {
                    client.take();
                };
                if let Ok(mut client) = state.claude.client.try_lock() {
                    client.take();
                };
                if let Ok(mut client) = state.native.client.try_lock() {
                    client.take();
                };
            }
        });
}
