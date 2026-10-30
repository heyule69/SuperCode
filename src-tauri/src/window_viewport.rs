//! Keep workspace WebViews at their real client size while Windows minimizes them.
//! Tauri's `unstable` ratio autoresize also forwards the minimized 144x19 client
//! rectangle, which reflows the page before it is restored from the taskbar.
#[cfg(windows)]
use tauri::{Manager, PhysicalSize, Webview, WebviewWindow, Window, WindowEvent};
#[cfg(windows)]
fn resize(window: &Window, webview: &Webview, size: PhysicalSize<u32>) -> tauri::Result<()> {
    if !window.is_minimized()? && size.width > 0 && size.height > 0 {
        // Use the Webview API: WebviewWindow::set_size would resize the HWND.
        webview.set_size(size)?;
    }
    Ok(())
}

#[cfg(windows)]
pub fn install(workspace: &WebviewWindow) -> tauri::Result<()> {
    let webview: &Webview = workspace.as_ref();
    webview.set_auto_resize(false)?;
    let window = webview.window();
    resize(&window, webview, window.inner_size()?)?;

    let event_window = window.clone();
    let event_webview = webview.clone();
    window.on_window_event(move |event| {
        let size = match event {
            WindowEvent::Resized(size)
            | WindowEvent::ScaleFactorChanged {
                new_inner_size: size,
                ..
            } => *size,
            _ => return,
        };
        if let Err(error) = resize(&event_window, &event_webview, size) {
            crate::runtime_diagnostics::message(
                event_window.app_handle(),
                None,
                "window",
                "error",
                "ui_viewport_resize_error",
                &format!("{}: {error}", event_window.label()),
            );
        }
    });
    Ok(())
}

#[cfg(not(windows))]
pub fn install(_workspace: &tauri::WebviewWindow) -> tauri::Result<()> {
    Ok(())
}

#[cfg(windows)]
pub fn sync(workspace: &WebviewWindow) -> tauri::Result<()> {
    // Tauri queues listener registration even on the main thread. Setup's
    // synchronous restore/show therefore finishes before our listener is active.
    // Commit the final client size once after that initial geometry is applied.
    let webview: &Webview = workspace.as_ref();
    let window = webview.window();
    resize(&window, webview, window.inner_size()?)
}

#[cfg(not(windows))]
pub fn sync(_workspace: &tauri::WebviewWindow) -> tauri::Result<()> {
    Ok(())
}
