use serde::Deserialize;
use std::{collections::HashMap, sync::Mutex};
use tauri::{webview::Color, Manager, State, Webview, Window, WindowEvent};

#[derive(Deserialize)]
pub struct WindowColors {
    background: String,
    #[serde(default)]
    canvas: Option<String>,
    text: String,
    border: String,
}

#[derive(Clone, Copy)]
struct Palette {
    background: u32,
    canvas: u32,
    text: u32,
    border: u32,
}

#[derive(Default)]
pub struct WindowTheme(Mutex<HashMap<String, Palette>>);

fn color_ref(value: &str) -> Result<u32, String> {
    let hex = value
        .trim()
        .strip_prefix('#')
        .ok_or("窗口颜色必须为十六进制颜色")?;
    let rgb = match hex.len() {
        3 if hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
            let short = u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?;
            let red = ((short >> 8) & 0xf) * 17;
            let green = ((short >> 4) & 0xf) * 17;
            let blue = (short & 0xf) * 17;
            (red << 16) | (green << 8) | blue
        }
        6 if hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
            u32::from_str_radix(hex, 16).map_err(|e| e.to_string())?
        }
        _ => return Err("窗口颜色必须为 #RGB 或 #RRGGBB".into()),
    };
    // DWM expects COLORREF (0x00BBGGRR), not CSS's RGB byte order.
    Ok(((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 0xff))
}

impl TryFrom<WindowColors> for Palette {
    type Error = String;
    fn try_from(colors: WindowColors) -> Result<Self, Self::Error> {
        Ok(Self {
            background: color_ref(&colors.background)?,
            canvas: color_ref(colors.canvas.as_deref().unwrap_or(&colors.background))?,
            text: color_ref(&colors.text)?,
            border: color_ref(&colors.border)?,
        })
    }
}

fn canvas_color(palette: Palette) -> Color {
    // Palette fields are Windows COLORREF; WebView2 expects ordinary RGBA.
    Color(
        (palette.canvas & 0xff) as u8,
        ((palette.canvas >> 8) & 0xff) as u8,
        ((palette.canvas >> 16) & 0xff) as u8,
        255,
    )
}

pub fn canvas_for_window(app: &tauri::AppHandle, label: &str) -> Option<Color> {
    let state = app.try_state::<WindowTheme>()?;
    let palettes = state.0.lock().ok()?;
    palettes.get(label).copied().map(canvas_color)
}

#[cfg(windows)]
fn apply(window: &Window, palette: Palette) -> Result<bool, String> {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
    };
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0;
    for (attribute, color) in [
        (DWMWA_CAPTION_COLOR, palette.background),
        (DWMWA_TEXT_COLOR, palette.text),
        (DWMWA_BORDER_COLOR, palette.border),
    ] {
        // The handle belongs to this live Tauri window, and DWM copies the u32
        // during this call. No custom painting or replacement window controls.
        let result = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attribute as u32,
                (&color as *const u32).cast(),
                std::mem::size_of::<u32>() as u32,
            )
        };
        if result < 0 {
            // Custom caption colors require Windows 11 build 22000+. Older
            // systems retain Tauri's native light/dark title bar.
            if result as u32 == 0x80070057 || result as u32 == 0x80004001 {
                return Ok(false);
            }
            return Err(format!("同步窗口颜色失败：0x{:08X}", result as u32));
        }
    }
    Ok(true)
}

#[cfg(not(windows))]
fn apply(_window: &Window, _palette: Palette) -> Result<bool, String> {
    Ok(false)
}

#[tauri::command]
pub fn set_window_colors(
    window: Window,
    webview: Webview,
    state: State<'_, WindowTheme>,
    colors: WindowColors,
) -> Result<bool, String> {
    let palette = Palette::try_from(colors)?;
    let canvas = Some(canvas_color(palette));
    // Caption colors alone do not change WebView2's white backing surface.
    // Keep both layers opaque and themed when a renderer is repainting/reloading.
    window
        .set_background_color(canvas)
        .map_err(|e| e.to_string())?;
    webview
        .set_background_color(canvas)
        .map_err(|e| e.to_string())?;
    state
        .inner()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .insert(window.label().to_owned(), palette);
    apply(&window, palette)
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let Some(state) = window.try_state::<WindowTheme>() else {
        return;
    };
    if matches!(event, WindowEvent::Destroyed) {
        crate::ui_recovery::forget(window);
        if let Ok(mut palettes) = state.inner().0.lock() {
            palettes.remove(window.label());
        }
    } else if matches!(
        event,
        WindowEvent::Focused(_) | WindowEvent::ThemeChanged(_)
    ) {
        // Windows accent settings must not reintroduce a colored frame when
        // the window gains or loses focus. Never hold the lock across DWM calls.
        let palette = state
            .inner()
            .0
            .lock()
            .ok()
            .and_then(|p| p.get(window.label()).copied());
        if let Some(palette) = palette {
            if let Err(error) = apply(window, palette) {
                eprintln!("{error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_colors_use_windows_byte_order() {
        assert_eq!(color_ref("#20262f").unwrap(), 0x002f2620);
        assert_eq!(color_ref("#fcfaf5").unwrap(), 0x00f5fafc);
        assert_eq!(color_ref(" #E9EAE7 ").unwrap(), 0x00e7eae9);
        assert_eq!(color_ref("#fff").unwrap(), 0x00ffffff);
        assert_eq!(color_ref("#a2f").unwrap(), 0x00ff22aa);
    }

    #[test]
    fn invalid_colors_cannot_be_dwm_sentinels_or_partial_palettes() {
        for value in [
            "#ffffffff",
            "#fffffffe",
            "#12",
            "#gggggg",
            "red",
            "rgb(0,0,0)",
            "#中",
        ] {
            assert!(color_ref(value).is_err(), "{value}");
        }
        assert!(Palette::try_from(WindowColors {
            background: "#202120".into(),
            canvas: None,
            text: "invalid".into(),
            border: "#393b38".into(),
        })
        .is_err());
    }

    #[test]
    fn webview_canvas_matches_content_instead_of_caption_and_rejects_invalid_colors() {
        let palette = Palette::try_from(WindowColors {
            background: "#1a2129".into(),
            canvas: Some("#20262f".into()),
            text: "#e3e3e3".into(),
            border: "#333639".into(),
        })
        .unwrap();
        assert_eq!(canvas_color(palette), Color(32, 38, 47, 255));
        let legacy: WindowColors =
            serde_json::from_str(r##"{"background":"#181818","text":"#eee","border":"#333"}"##)
                .unwrap();
        assert_eq!(
            canvas_color(Palette::try_from(legacy).unwrap()),
            Color(24, 24, 24, 255)
        );
        assert!(Palette::try_from(WindowColors {
            background: "#181818".into(),
            canvas: Some("transparent".into()),
            text: "#eee".into(),
            border: "#333".into(),
        })
        .is_err());
    }
}
