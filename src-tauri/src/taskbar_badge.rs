//! The taskbar badge follows persisted, unread results across all workspaces.
use crate::{storage::Store, AppState};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(windows)]
use std::sync::Mutex;
use tauri::{AppHandle, Listener, Manager};

pub struct TaskbarBadge {
    pending: AtomicBool,
    applied: AtomicUsize,
    #[cfg(windows)]
    icon: Mutex<Option<(usize, tauri::image::Image<'static>)>>,
}
impl Default for TaskbarBadge {
    fn default() -> Self {
        Self {
            pending: AtomicBool::new(false),
            applied: AtomicUsize::new(usize::MAX),
            #[cfg(windows)]
            icon: Mutex::new(None),
        }
    }
}

impl Store {
    pub(crate) fn unread_result_count(&self) -> Result<usize, String> {
        self.0.lock().map_err(|error| error.to_string())?.query_row(
            "SELECT COUNT(*) FROM sessions s
             JOIN sidebar_items i ON i.kind='session' AND i.id=s.id
             WHERE i.unread=1 AND i.removed=0 AND s.archived=0
             AND s.status NOT IN ('starting','running','waiting')
             AND NOT EXISTS (SELECT 1 FROM sidebar_items p WHERE p.kind='project' AND p.id=s.project_id AND p.removed=1)",
            [], |row| row.get(0),
        ).map_err(|error| error.to_string())
    }
}

pub fn install(app: &AppHandle) {
    let handle = app.clone();
    app.listen("workspace-updated", move |_| refresh(&handle));
    refresh(app);
}

pub fn refresh(app: &AppHandle) {
    let Some(state) = app.try_state::<TaskbarBadge>() else {
        return;
    };
    if state.pending.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    // Post from a worker so an emitter never re-enters storage or the Windows
    // taskbar while a synchronous IPC/window callback is still running.
    tauri::async_runtime::spawn(async move {
        let handle = app.clone();
        if let Err(error) = app.run_on_main_thread(move || {
            handle
                .state::<TaskbarBadge>()
                .pending
                .store(false, Ordering::Release);
            if let Err(error) = apply(&handle) {
                report(&handle, &error);
            }
        }) {
            app.state::<TaskbarBadge>()
                .pending
                .store(false, Ordering::Release);
            report(&app, &error.to_string());
        }
    });
}

fn report(app: &AppHandle, error: &str) {
    crate::runtime_diagnostics::message(
        app,
        None,
        "taskbar",
        "warn",
        "taskbar_badge_update_error",
        error,
    );
}

fn apply(app: &AppHandle) -> Result<(), String> {
    let count = app.state::<AppState>().store.unread_result_count()?;
    let state = app.state::<TaskbarBadge>();
    #[cfg(windows)]
    let icon = if count == 0 {
        None
    } else {
        let key = count.min(100);
        let mut cached = state.icon.lock().map_err(|error| error.to_string())?;
        if cached.as_ref().is_none_or(|(previous, _)| *previous != key) {
            *cached = Some((key, badge_icon(count)?));
        }
        cached.as_ref().map(|(_, icon)| icon.clone())
    };
    for window in app
        .windows()
        .into_values()
        .filter(|window| window.label() == "main" || window.label().starts_with("chat-"))
    {
        #[cfg(windows)]
        window
            .set_overlay_icon(icon.clone())
            .map_err(|error| error.to_string())?;
        #[cfg(not(windows))]
        window
            .set_badge_count((count > 0).then_some(count as i64))
            .map_err(|error| error.to_string())?;
    }
    if state.applied.swap(count, Ordering::AcqRel) != count {
        crate::runtime_diagnostics::record(
            app,
            None,
            "taskbar_badge_changed",
            serde_json::json!({"count":count}),
        );
    }
    Ok(())
}

pub(crate) fn applied_count(app: &AppHandle) -> Option<usize> {
    let count = app.state::<TaskbarBadge>().applied.load(Ordering::Acquire);
    (count != usize::MAX).then_some(count)
}

#[cfg(windows)]
pub(crate) fn badge_icon(count: usize) -> Result<tauri::image::Image<'static>, String> {
    drawing::icon(count)
}

#[cfg(windows)]
mod drawing {
    use std::ptr::null_mut;
    use windows_sys::Win32::{Foundation::RECT, Graphics::Gdi::*};

    const SIZE: u32 = 32;
    const SAMPLE: u32 = 128;

    struct Canvas {
        dc: HDC,
        bitmap: HBITMAP,
        font: HFONT,
        old_bitmap: HGDIOBJ,
        old_font: HGDIOBJ,
    }
    fn valid(object: HGDIOBJ) -> bool {
        !object.is_null() && object as isize != -1
    }
    impl Drop for Canvas {
        fn drop(&mut self) {
            unsafe {
                if valid(self.old_font) {
                    SelectObject(self.dc, self.old_font);
                }
                if valid(self.old_bitmap) {
                    SelectObject(self.dc, self.old_bitmap);
                }
                if !self.font.is_null() {
                    DeleteObject(self.font);
                }
                if !self.bitmap.is_null() {
                    DeleteObject(self.bitmap);
                }
                if !self.dc.is_null() {
                    DeleteDC(self.dc);
                }
            }
        }
    }

    pub(super) fn icon(count: usize) -> Result<tauri::image::Image<'static>, String> {
        let label = if count > 99 {
            "99+".into()
        } else {
            count.to_string()
        };
        unsafe {
            let mut canvas = Canvas {
                dc: CreateCompatibleDC(null_mut()),
                bitmap: null_mut(),
                font: null_mut(),
                old_bitmap: null_mut(),
                old_font: null_mut(),
            };
            if canvas.dc.is_null() {
                return Err("无法创建任务栏角标画布".into());
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = SAMPLE as i32;
            info.bmiHeader.biHeight = -(SAMPLE as i32);
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut pixels = null_mut();
            canvas.bitmap =
                CreateDIBSection(canvas.dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
            if canvas.bitmap.is_null() || pixels.is_null() {
                return Err("无法创建任务栏角标位图".into());
            }
            canvas.old_bitmap = SelectObject(canvas.dc, canvas.bitmap);
            if !valid(canvas.old_bitmap) {
                return Err("无法选择任务栏角标位图".into());
            }
            let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
            let height = match label.len() {
                1 => 96,
                2 => 84,
                _ => 64,
            };
            canvas.font = CreateFontW(
                -height,
                0,
                0,
                0,
                FW_BOLD as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                ANTIALIASED_QUALITY as u32,
                (DEFAULT_PITCH | FF_SWISS) as u32,
                face.as_ptr(),
            );
            if canvas.font.is_null() {
                return Err("无法创建任务栏角标字体".into());
            }
            canvas.old_font = SelectObject(canvas.dc, canvas.font);
            if !valid(canvas.old_font) {
                return Err("无法选择任务栏角标字体".into());
            }
            std::ptr::write_bytes(pixels, 0, (SAMPLE * SAMPLE * 4) as usize);
            if SetBkMode(canvas.dc, TRANSPARENT as i32) == 0
                || SetTextColor(canvas.dc, 0x00ff_ffff) == u32::MAX
            {
                return Err("无法设置任务栏角标文字颜色".into());
            }
            let text: Vec<u16> = label.encode_utf16().collect();
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: SAMPLE as i32,
                bottom: SAMPLE as i32,
            };
            if DrawTextW(
                canvas.dc,
                text.as_ptr(),
                text.len() as i32,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            ) == 0
                || GdiFlush() == 0
            {
                return Err("无法绘制任务栏角标文字".into());
            }
            let mask =
                std::slice::from_raw_parts(pixels.cast::<u8>(), (SAMPLE * SAMPLE * 4) as usize);
            let image = image::RgbaImage::from_fn(SAMPLE, SAMPLE, |x, y| {
                let distance =
                    ((x as f32 + 0.5 - 64.).powi(2) + (y as f32 + 0.5 - 64.).powi(2)).sqrt();
                if distance > 62. {
                    return image::Rgba([36, 36, 39, 0]);
                }
                if distance > 59. {
                    return image::Rgba([245, 245, 245, 255]);
                }
                let ink = mask[((y * SAMPLE + x) * 4) as usize] as u32;
                let blend = |background: u32| (background + (255 - background) * ink / 255) as u8;
                image::Rgba([blend(36), blend(36), blend(39), 255])
            });
            let image =
                image::imageops::resize(&image, SIZE, SIZE, image::imageops::FilterType::Lanczos3);
            Ok(tauri::image::Image::new_owned(image.into_raw(), SIZE, SIZE))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;
    use std::path::Path;

    fn store() -> Store {
        Store::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }
    fn finish(store: &Store, project: &str, status: &str) -> String {
        let chat = if project.is_empty() {
            store
                .create_projectless_session(
                    &std::env::temp_dir().join(format!(
                        "supercode-badge-workspaces-{}",
                        uuid::Uuid::new_v4()
                    )),
                    None,
                    "codex",
                    None,
                )
                .unwrap()
        } else {
            store.create_session(project, None).unwrap()
        };
        store.set_status(&chat.id, "running", Some("turn")).unwrap();
        store.complete_turn(&chat.id, "turn", status).unwrap();
        chat.id
    }

    #[test]
    fn unread_results_increment_once_and_clear_on_reading_archiving_and_deletion() {
        let store = store();
        let project = store.add_project(Path::new("D:/badge-count")).unwrap();
        assert_eq!(store.unread_result_count().unwrap(), 0);
        let first = finish(&store, &project.id, "completed");
        assert_eq!(store.unread_result_count().unwrap(), 1);
        let second = finish(&store, &project.id, "completed");
        let third = finish(&store, &project.id, "completed");
        assert_eq!(store.unread_result_count().unwrap(), 3);
        store.complete_turn(&first, "turn", "completed").unwrap();
        assert_eq!(store.unread_result_count().unwrap(), 3);
        assert!(store.mark_session_read(&first, 0).unwrap());
        assert_eq!(store.unread_result_count().unwrap(), 2);
        store.archive(&second).unwrap();
        assert_eq!(store.unread_result_count().unwrap(), 1);
        store.delete_session(&third).unwrap();
        assert_eq!(store.unread_result_count().unwrap(), 0);
        store.restore_session(&second).unwrap();
        assert_eq!(store.unread_result_count().unwrap(), 1);
    }

    #[test]
    fn active_and_removed_tasks_are_excluded_and_failed_results_follow_sidebar_unread_state() {
        let store = store();
        let project = store.add_project(Path::new("D:/badge-hidden")).unwrap();
        let chat = finish(&store, &project.id, "failed");
        assert_eq!(store.unread_result_count().unwrap(), 1);
        for status in ["starting", "running", "waiting"] {
            store.set_status(&chat, status, Some("next")).unwrap();
            assert_eq!(store.unread_result_count().unwrap(), 0);
        }
        store.complete_turn(&chat, "next", "completed").unwrap();
        assert_eq!(store.unread_result_count().unwrap(), 1);
        store.project_action(&project.id, "remove").unwrap();
        assert_eq!(store.unread_result_count().unwrap(), 0);
        let direct = finish(&store, "", "completed");
        assert_eq!(store.unread_result_count().unwrap(), 1);
        store
            .update_sidebar("session", &direct, "unread", json!(false))
            .unwrap();
        assert_eq!(store.unread_result_count().unwrap(), 0);
        finish(&store, "", "interrupted");
        assert_eq!(store.unread_result_count().unwrap(), 0);
    }

    #[test]
    fn restored_counts_include_the_whole_directory_and_ignore_stale_read_receipts() {
        let path =
            std::env::temp_dir().join(format!("supercode-badge-{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&path).unwrap();
        let chat = finish(&store, "", "completed");
        store
            .save_message(
                "reply",
                &chat,
                "assistant",
                "新的结果",
                "agentMessage",
                &json!({}),
            )
            .unwrap();
        for _ in 0..35 {
            finish(&store, "", "completed");
        }
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.unread_result_count().unwrap(), 36);
        assert!(!store.mark_session_read(&chat, 0).unwrap());
        assert_eq!(store.unread_result_count().unwrap(), 36);
        let seq = store.messages(&chat, None).unwrap().last().unwrap().seq;
        assert!(store.mark_session_read(&chat, seq).unwrap());
        assert_eq!(store.unread_result_count().unwrap(), 35);
    }

    #[cfg(windows)]
    #[test]
    fn native_badge_images_have_transparent_corners_readable_ink_and_bounded_labels() {
        let mut images = Vec::new();
        for count in [1, 2, 3, 12, 99, 100, usize::MAX] {
            let icon = badge_icon(count).unwrap();
            assert_eq!((icon.width(), icon.height()), (32, 32));
            assert_eq!(icon.rgba()[3], 0);
            let pixels = icon.rgba().chunks_exact(4).collect::<Vec<_>>();
            assert!(
                pixels
                    .iter()
                    .filter(|pixel| pixel[3] > 200 && pixel[0] > 220)
                    .count()
                    > 20
            );
            assert!(
                pixels
                    .iter()
                    .filter(|pixel| pixel[3] > 200 && pixel[0] < 60)
                    .count()
                    > 100
            );
            images.push(icon.rgba().to_vec());
        }
        assert_ne!(images[0], images[1]);
        assert_ne!(images[1], images[2]);
        assert_eq!(images[5], images[6]);
    }
}
