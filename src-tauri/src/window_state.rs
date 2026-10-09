use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex};
use tauri::{Manager, Window, WindowEvent};
#[cfg(not(windows))]
use tauri::{PhysicalPosition, PhysicalSize};

// Outer positions and sizes are physical pixels. Keep the normal bounds separate:
// maximization/minimization must never replace them with screen-sized bounds.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Bounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct SavedWindow {
    version: u32,
    normal: Bounds,
    scale_factor: f64,
    maximized: bool,
}

#[derive(Clone, Copy)]
struct Display {
    area: Bounds,
    scale: f64,
}

struct Memory {
    path: PathBuf,
    saved: SavedWindow,
    enabled: bool,
}
pub struct WindowState(Mutex<Memory>);

impl Memory {
    fn observe(&mut self, minimized: bool, maximized: bool, normal: Option<(Bounds, f64)>) {
        if self.enabled {
            self.saved.observe(minimized, maximized, normal);
        }
    }
    fn freeze(&mut self) {
        self.enabled = false;
    }
}

impl SavedWindow {
    fn valid(self) -> bool {
        self.version == 1
            && self.normal.width >= 200
            && self.normal.height >= 150
            && self.normal.width <= 32768
            && self.normal.height <= 32768
            && self.normal.x.abs_diff(0) <= 100_000
            && self.normal.y.abs_diff(0) <= 100_000
            && self.scale_factor.is_finite()
            && (0.5..=8.0).contains(&self.scale_factor)
    }
    fn observe(&mut self, minimized: bool, maximized: bool, normal: Option<(Bounds, f64)>) {
        if minimized {
            return;
        }
        self.maximized = maximized;
        if !maximized {
            if let Some((bounds, scale)) = normal {
                let next = Self {
                    normal: bounds,
                    scale_factor: scale,
                    ..*self
                };
                if next.valid() {
                    *self = next;
                }
            }
        }
    }
}

fn intersection(a: Bounds, b: Bounds) -> i64 {
    let w = (i64::from(a.x) + i64::from(a.width)).min(i64::from(b.x) + i64::from(b.width))
        - i64::from(a.x.max(b.x));
    let h = (i64::from(a.y) + i64::from(a.height)).min(i64::from(b.y) + i64::from(b.height))
        - i64::from(a.y.max(b.y));
    w.max(0) * h.max(0)
}

fn fit(saved: Option<SavedWindow>, displays: &[Display], primary: usize) -> Option<SavedWindow> {
    if displays.is_empty() {
        return None;
    }
    let saved = saved.filter(|state| state.valid());
    let index = saved
        .and_then(|state| {
            displays
                .iter()
                .enumerate()
                .map(|(i, display)| (i, intersection(state.normal, display.area)))
                .max_by_key(|(_, area)| *area)
                .filter(|(_, area)| *area > 0)
                .map(|(i, _)| i)
        })
        .unwrap_or(primary.min(displays.len() - 1));
    let display = displays[index];
    let area = display.area;
    let scale = display.scale;
    let ratio = saved.map_or(scale, |state| scale / state.scale_factor);
    let source = saved.map_or(
        Bounds {
            x: 0,
            y: 0,
            width: 1320,
            height: 800,
        },
        |state| state.normal,
    );
    let width = ((f64::from(source.width) * ratio).round() as u32)
        .max((760.0 * scale) as u32)
        .min(area.width.saturating_sub(2).max(200));
    let height = ((f64::from(source.height) * ratio).round() as u32)
        .max((580.0 * scale) as u32)
        .min(area.height.saturating_sub(2).max(150));
    let reachable = saved.is_some_and(|state| intersection(state.normal, area) > 0);
    let x = if reachable {
        source.x
    } else {
        area.x + ((area.width - width) / 2) as i32
    };
    let y = if reachable {
        source.y
    } else {
        area.y + ((area.height - height) / 2) as i32
    };
    Some(SavedWindow {
        version: 1,
        normal: Bounds {
            x: x.clamp(area.x, area.x + area.width.saturating_sub(width + 2) as i32),
            y: y.clamp(
                area.y,
                area.y + area.height.saturating_sub(height + 2) as i32,
            ),
            width,
            height,
        },
        scale_factor: scale,
        maximized: saved.is_some_and(|state| state.maximized),
    })
}

fn read(path: &std::path::Path) -> Option<SavedWindow> {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<SavedWindow>(&text) {
            Ok(state) if state.valid() => Some(state),
            _ => {
                eprintln!("窗口状态无效，使用居中的默认窗口");
                None
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            eprintln!("无法读取 UTF-8 窗口状态：{error}");
            None
        }
    }
}

#[cfg(windows)]
fn restore_bounds(window: &tauri::WebviewWindow, bounds: Bounds) -> tauri::Result<()> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};
    // A hidden undecorated window can initially report decorated client insets.
    // Restore measured outer bounds directly to avoid adding caption height on
    // every launch. This runs on Tauri's setup thread and never changes focus.
    let hwnd = window.hwnd()?.0;
    let result = unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            bounds.x,
            bounds.y,
            bounds.width as i32,
            bounds.height as i32,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn restore_bounds(window: &tauri::WebviewWindow, bounds: Bounds) -> tauri::Result<()> {
    let outer = window.outer_size()?;
    let inner = window.inner_size()?;
    window.set_size(PhysicalSize::new(
        bounds
            .width
            .saturating_sub(outer.width.saturating_sub(inner.width)),
        bounds
            .height
            .saturating_sub(outer.height.saturating_sub(inner.height)),
    ))?;
    window.set_position(PhysicalPosition::new(bounds.x, bounds.y))?;
    Ok(())
}

pub fn restore(app: &tauri::App, dir: &std::path::Path) -> tauri::Result<()> {
    let Some(webview) = app.get_webview_window("main") else {
        return Ok(());
    };
    let window = &webview;
    let path = dir.join("window-state.json");
    let monitors = window.available_monitors()?;
    let primary = window
        .primary_monitor()?
        .and_then(|primary| {
            monitors
                .iter()
                .position(|monitor| monitor.position() == primary.position())
        })
        .unwrap_or(0);
    let displays: Vec<_> = monitors
        .iter()
        .map(|monitor| {
            let area = monitor.work_area();
            Display {
                area: Bounds {
                    x: area.position.x,
                    y: area.position.y,
                    width: area.size.width,
                    height: area.size.height,
                },
                scale: monitor.scale_factor(),
            }
        })
        .collect();
    let state = fit(read(&path), &displays, primary);
    if let Some(state) = state {
        restore_bounds(window, state.normal)?;
        app.manage(WindowState(Mutex::new(Memory {
            path,
            saved: state,
            enabled: true,
        })));
        if state.maximized {
            window.maximize()?;
        }
    } else {
        // The configuration's center=true still works if monitor enumeration
        // is unavailable. Do not persist an invalid fallback geometry.
        window.center()?;
    }
    window.show()?;
    Ok(())
}

fn capture(window: &Window) {
    let Some(state) = window.try_state::<WindowState>() else {
        return;
    };
    let minimized = window.is_minimized().unwrap_or(true);
    let maximized = window.is_maximized().unwrap_or(false);
    let normal = if minimized || maximized {
        None
    } else {
        window
            .outer_position()
            .ok()
            .zip(window.outer_size().ok())
            .zip(window.scale_factor().ok())
            .map(|((position, size), scale)| {
                (
                    Bounds {
                        x: position.x,
                        y: position.y,
                        width: size.width,
                        height: size.height,
                    },
                    scale,
                )
            })
    };
    if let Ok(mut memory) = state.0.lock() {
        memory.observe(minimized, maximized, normal);
    };
}

pub fn flush(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<WindowState>() else {
        return;
    };
    if let Ok(memory) = state.0.lock() {
        if !memory.enabled {
            return;
        }
        // Write only on close or focus loss, never for every pixel of a resize.
        // A temporary UTF-8 file keeps an interrupted write from corrupting state.
        let result = serde_json::to_vec_pretty(&memory.saved)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                let temporary = memory.path.with_extension("json.tmp");
                std::fs::write(&temporary, bytes)?;
                std::fs::rename(temporary, &memory.path)
            });
        if let Err(error) = result {
            eprintln!("保存窗口状态失败：{error}");
        }
    };
}

pub fn prepare_hide(window: &Window) {
    if window.label() == "main" {
        capture(window);
        flush(window.app_handle());
    }
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }
    if matches!(event, WindowEvent::CloseRequested { .. }) {
        capture(window);
        flush(window.app_handle());
        // Destruction can report a non-maximized resize with the old maximized
        // rectangle. Freeze the final snapshot before those trailing events.
        if let Some(state) = window.try_state::<WindowState>() {
            if let Ok(mut memory) = state.0.lock() {
                memory.freeze();
            };
        }
        return;
    }
    if matches!(
        event,
        WindowEvent::Moved(_)
            | WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::Focused(_)
    ) {
        capture(window);
    }
    if matches!(event, WindowEvent::Focused(false)) {
        flush(window.app_handle());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn display(x: i32, scale: f64) -> Display {
        Display {
            area: Bounds {
                x,
                y: 0,
                width: 1920,
                height: 1040,
            },
            scale,
        }
    }
    fn saved() -> SavedWindow {
        SavedWindow {
            version: 1,
            normal: Bounds {
                x: 120,
                y: 80,
                width: 1000,
                height: 700,
            },
            scale_factor: 1.0,
            maximized: false,
        }
    }
    #[test]
    fn first_launch_centers_in_work_area() {
        let state = fit(None, &[display(0, 1.0)], 0).unwrap();
        assert_eq!(
            state.normal,
            Bounds {
                x: 300,
                y: 120,
                width: 1320,
                height: 800
            }
        );
        assert!(!state.maximized);
    }
    #[test]
    fn normal_position_and_size_survive_reopen() {
        assert_eq!(fit(Some(saved()), &[display(0, 1.0)], 0), Some(saved()));
    }
    #[test]
    fn maximizing_preserves_restore_bounds_and_minimizing_preserves_state() {
        let mut state = saved();
        state.observe(false, true, Some((display(0, 1.0).area, 1.0)));
        assert_eq!(state.normal, saved().normal);
        assert!(state.maximized);
        state.observe(true, false, None);
        assert!(state.maximized);
        let encoded = serde_json::to_vec(&state).unwrap();
        let decoded: SavedWindow = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(fit(Some(decoded), &[display(0, 1.0)], 0), Some(state));
        state.observe(false, false, Some((saved().normal, 1.0)));
        assert_eq!(state, saved());
    }
    #[test]
    fn closing_freezes_normal_bounds_before_late_resize_and_focus_events() {
        let mut memory = Memory {
            path: PathBuf::new(),
            saved: saved(),
            enabled: true,
        };
        memory.observe(false, true, None);
        let final_state = memory.saved;
        memory.freeze();
        memory.observe(false, false, Some((display(0, 1.0).area, 1.0)));
        memory.observe(false, true, None);
        assert_eq!(memory.saved, final_state);
        assert_eq!(memory.saved.normal, saved().normal);
        assert!(memory.saved.maximized);
    }
    #[test]
    fn hidden_window_can_restore_and_record_new_geometry() {
        let mut memory = Memory {
            path: PathBuf::new(),
            saved: saved(),
            enabled: true,
        };
        memory.observe(false, true, None);
        // Tray hiding does not freeze persistence or replace restore bounds.
        memory.observe(true, false, None);
        assert!(memory.saved.maximized);
        assert_eq!(memory.saved.normal, saved().normal);
        let resized = Bounds {
            x: 200,
            y: 160,
            width: 1200,
            height: 800,
        };
        memory.observe(false, false, Some((resized, 1.0)));
        assert_eq!(memory.saved.normal, resized);
        assert!(!memory.saved.maximized);
    }
    #[test]
    fn removed_monitor_recenters_and_keeps_maximization() {
        let old = SavedWindow {
            normal: Bounds {
                x: -1800,
                ..saved().normal
            },
            maximized: true,
            ..saved()
        };
        let next = fit(Some(old), &[display(0, 1.0)], 0).unwrap();
        assert_eq!((next.normal.x, next.normal.y), (460, 170));
        assert!(next.maximized);
    }
    #[test]
    fn negative_coordinates_are_valid_on_connected_monitors() {
        let old = SavedWindow {
            normal: Bounds {
                x: -1800,
                ..saved().normal
            },
            ..saved()
        };
        assert_eq!(
            fit(Some(old), &[display(-1920, 1.0), display(0, 1.0)], 1),
            Some(old)
        );
    }
    #[test]
    fn scale_change_keeps_logical_size_and_clamps_to_available_area() {
        let state = fit(Some(saved()), &[display(0, 1.5)], 0).unwrap();
        assert_eq!((state.normal.width, state.normal.height), (1500, 1038));
        assert_eq!(state.scale_factor, 1.5);
        assert_eq!(state.normal.y, 0);
    }
    #[test]
    fn malformed_bounds_fall_back_and_oversized_windows_remain_reachable() {
        let bad = SavedWindow {
            scale_factor: f64::NAN,
            ..saved()
        };
        assert_eq!(
            fit(Some(bad), &[display(0, 1.0)], 0),
            fit(None, &[display(0, 1.0)], 0)
        );
        let huge = SavedWindow {
            normal: Bounds {
                width: 9000,
                height: 9000,
                ..saved().normal
            },
            ..saved()
        };
        let state = fit(Some(huge), &[display(0, 1.0)], 0).unwrap();
        assert_eq!(
            state.normal,
            Bounds {
                x: 0,
                y: 0,
                width: 1918,
                height: 1038
            }
        );
        assert_eq!(fit(None, &[], 0), None);
    }
    #[test]
    fn saved_file_is_utf8_and_can_replace_previous_state() {
        let dir = std::env::temp_dir().join(format!("supercode-window-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("window-state.json");
        std::fs::write(&path, serde_json::to_vec(&saved()).unwrap()).unwrap();
        let next = SavedWindow {
            maximized: true,
            ..saved()
        };
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&next).unwrap()).unwrap();
        std::fs::rename(tmp, &path).unwrap();
        assert_eq!(read(&path), Some(next));
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
