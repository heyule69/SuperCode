// Native diagnostics reference WebView/dialog code during unit tests. Reuse
// Tauri's Common Controls v6 resource, which its build script only links to bins.
#[cfg(all(test, target_os = "windows", target_env = "msvc"))]
#[link(name = "resource", kind = "static", modifiers = "-bundle")]
unsafe extern "C" {}

mod accounts;
mod agent_commands;
mod agent_smoke;
mod agent_versions;
mod agent_recovery;
mod agents;
mod app_updates;
mod automation;
mod backup;
mod bridge;
mod browser;
mod browser_bridge;
mod browser_native;
mod ccswitch;
mod chat_connection;
mod claude;
mod claude_connection;
mod claude_tasks;
mod client_features;
mod commands;
mod computer;
mod computer_bridge;
mod computer_gate;
mod computer_native;
mod computer_smoke;
mod connection_order;
mod connection_usage;
mod credentials;
mod desktop_control;
mod desktop_control_smoke;
#[cfg(windows)]
mod desktop_banner;
mod desktop_glow;
mod desktop_lifecycle;
mod desktop_smoke;
mod extensions;
mod followup_smoke;
mod file_changes;
mod file_attachments;
mod handoff;
mod image_attachments;
mod input_limits;
mod installation_exit;
mod installation_exit_protocol;
mod links;
mod log_files;
mod log_redaction;
mod maintenance;
mod media;
mod model_context;
mod codex_context;
mod native_agents;
mod notifications;
mod official_accounts;
mod official_models;
mod outbox;
mod platform_usage;
mod process;
mod protocol;
mod subagents;
mod providers;
mod magpie;
mod runtime;
mod runtime_diagnostics;
mod session_config;
mod session_runtimes;
mod sidebar;
mod skills;
mod smoke;
mod storage;
mod startup_data;
mod tray_popup;
mod ui_recovery;
mod usage_tokens;
mod window_state;
mod window_theme;
mod window_viewport;

use tauri::Manager;

pub struct AppState {
    pub store: storage::Store,
    pub data_dir: std::path::PathBuf,
    pub runtime: runtime::Runtime,
    pub claude: claude::Runtime,
    pub native: native_agents::Runtime,
    pub activity: session_runtimes::Sessions<std::sync::Mutex<protocol::ActivityBuffer>>,
    pub change_snapshots: std::sync::Mutex<file_changes::Tracker>,
}

pub fn run() {
    let args: Vec<_> = std::env::args().collect();
    let diagnostic_dir = startup_data::directory(&args).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    if args.get(1).is_some_and(|arg| arg == "--browser-mcp") {
        if let Err(error) = browser_bridge::run(&args[2..]) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    // These headless modes must run before Tauri/WebView initialization.
    if args.get(1).is_some_and(|arg| arg == "--computer-mcp") {
        if let Err(error) = computer::run() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.get(1).is_some_and(|arg| arg == "--computer-worker") {
        computer::worker();
        return;
    }
    if args
        .get(1)
        .is_some_and(|arg| arg == "--computer-mcp-bridge")
    {
        if let Err(error) = computer_bridge::run(&args[2..]) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let desktop_test = std::env::args()
        .any(|arg| matches!(arg.as_str(), "--desktop-smoke-test" | "--tray-hover-test"));
    let desktop_control_test = std::env::args().any(|arg| arg == "--desktop-control-test");
    let browser_test = std::env::args().any(|arg| arg == "--browser-smoke-test");
    let browser_viewport_test = std::env::args().any(|arg| arg == "--browser-viewport-test");
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
    }) || cfg!(debug_assertions) && args.iter().any(|arg|arg=="--activity-details-test");
    let smoke_dir = (smoke_test
        || agents_test
        || desktop_test
        || desktop_control_test
        || browser_test
        || browser_viewport_test
        || followup_test
        || update_test
        || installation_test)
        .then(|| {
            std::env::current_dir()
                .unwrap_or_default()
                .join(if browser_viewport_test { ".supercode/browser-viewport-audit" } else { ".supercode/smoke" })
                .join(if browser_viewport_test {
                    "runtime".into()
                } else if agents_test {
                    format!("Agent test 测试 {}", uuid::Uuid::new_v4())
                } else {
                    uuid::Uuid::new_v4().to_string()
                })
        });
    let mut context = tauri::generate_context!();
    if let Some(dir) = diagnostic_dir.as_ref().or(smoke_dir.as_ref()) {
        for window in &mut context.config_mut().app.windows {
            window.data_directory = Some(dir.join("webview-data"));
            if smoke_dir.is_some() { window.visible = false; }
        }
    }
    if automation_test {
        for window in &mut context.config_mut().app.windows {
            window.visible = false;
        }
    }
    let builder = tauri::Builder::default();
    let builder = if smoke_test
        || diagnostic_dir.is_some()
        || automation_test
        || agents_test
        || desktop_test
        || desktop_control_test
        || browser_test
        || browser_viewport_test
        || followup_test
        || installation_test
    {
        builder
    } else {
        builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_window("main") {
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
                let webview = context.webview_label().to_owned();
                tauri::async_runtime::spawn_blocking(move || {
                    responder.respond(media::response(&app.state::<media::Media>(), &webview, request));
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
        .manage(runtime_diagnostics::Diagnostics::default())
        .manage(desktop_lifecycle::DesktopLifecycle::default())
        .manage(desktop_control::DesktopControl::default())
        .manage(browser::Browser::default())
        .manage(tray_popup::TrayPopup::default())
        .on_page_load(|webview, payload| {
            if !webview.label().starts_with("browser-")
                && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
            {
                runtime_diagnostics::record(webview.app_handle(), None, "ui_page_loaded",
                    serde_json::json!({"window":webview.label()}));
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
            let dir = if let Some(dir) = diagnostic_dir.as_ref().or(smoke_dir.as_ref()) {
                dir.clone()
            } else {
                app.path().app_data_dir()?
            };
            std::fs::create_dir_all(&dir)?;
            runtime_diagnostics::initialize(app.handle(), &dir).map_err(std::io::Error::other)?;
            let store = storage::Store::open(&dir.join("supercode.db")).inspect_err(|error| {
                runtime_diagnostics::message(app.handle(), None, "storage", "error", "storage_open_error", &error.to_string());
            })?;
            app.manage(AppState {
                store,
                data_dir: dir.clone(),
                runtime: runtime::Runtime::default(),
                claude: claude::Runtime::default(),
                native: native_agents::Runtime::default(),
                activity: session_runtimes::Sessions::default(),
                change_snapshots: std::sync::Mutex::new(file_changes::Tracker::default()),
            });
            runtime_diagnostics::record(app.handle(), None, "app_started", serde_json::json!({}));
            if let Some(window) = app.get_webview_window("main") {
                window_viewport::install(&window)?;
            }
            media::install(app.handle());
            if let Err(error) = desktop_control::install(app.handle()) {
                runtime_diagnostics::message(app.handle(),None,"desktop_control","warn","desktop_control_install_error",&error.to_string());
            }
            browser::install(app.handle())?;
            if !std::env::args().any(|arg| arg == "--legacy-installation-exit-test") {
                installation_exit::attach(app.handle())?;
            }
            let outbox_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move { outbox::watch(outbox_handle).await });
            let handle = app.handle().clone();
            if browser_viewport_test {
                tauri::async_runtime::spawn(async move { browser::viewport_smoke(handle).await; });
            } else if browser_test {
                tauri::async_runtime::spawn(async move {
                    browser::smoke(handle).await;
                });
            } else if desktop_control_test {
                tauri::async_runtime::spawn(async move {
                    desktop_control_smoke::run(handle).await;
                });
            } else if installation_test {
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
                    runtime_diagnostics::message(app.handle(),None,"tray","warn","tray_install_error",&error.to_string());
                }
                let update_handle = handle.clone();
                tauri::async_runtime::spawn(async move { app_updates::watch(update_handle).await });
                tauri::async_runtime::spawn(async move { runtime::idle_watch(handle).await });
            }
            Ok(())
        })
        .invoke_handler(browser::guard(tauri::generate_handler![
            browser::browser_open,
            browser::browser_layout,
            browser::browser_manual,
            browser::browser_close,
            browser::browser_takeover,
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
            agent_recovery::list_agent_recovery,
            agent_recovery::rollback_agent,
            agents::cancel_agent_install,
            agents::configure_agent,
            backup::export_data_backup,
            backup::prepare_data_restore,
            backup::restored_frontend,
            backup::acknowledge_frontend_restore,
            maintenance::storage_summary,
            maintenance::clean_retired_storage,
            automation::list_automation,
            automation::install_automation,
            automation::test_automation,
            automation::cancel_automation,
            automation::set_automation_enabled,
            window_theme::set_window_colors,
            links::open_external_link,
            media::prepare_media,
            media::open_media,
            media::reveal_media,
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
            runtime_diagnostics::save_runtime_diagnostics,
            runtime_diagnostics::open_runtime_log_folder,
            runtime_diagnostics::report_frontend_error,
            commands::configure_codex,
            commands::add_project,
            commands::create_session,
            chat_connection::check_chat_connection,
            commands::list_messages,
            subagents::get_subagent_messages,
            commands::send_message,
            commands::send_chat_message,
            outbox::enqueue_followup,
            outbox::list_followups,
            outbox::followup_capabilities,
            outbox::change_followup,
            outbox::begin_followup_edit,
            outbox::finish_followup_edit,
            outbox::steer_followup,
            image_attachments::import_image_attachment,
            image_attachments::read_image_attachment,
            commands::interrupt_turn,
            commands::respond_request,
            commands::list_models,
            providers::get_model_source,
            commands::workspace_changes,
            commands::read_project_file,
            commands::list_workspace_directory,
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
            providers::delete_provider_profiles,
            magpie::open_magpie,
            magpie::use_magpie,
            magpie::remove_magpie,
            connection_order::reorder_provider_connections,
            providers::fetch_provider_models,
            providers::fetch_provider_models_for_settings,
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
        ]))
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
                runtime_diagnostics::record(app, None, "app_exited", serde_json::json!({}));
                desktop_control::shutdown(app);
                browser::shutdown(app);
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
