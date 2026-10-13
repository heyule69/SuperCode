//! Windows implementation. No shell, clipboard, Python, or downloaded runtime.
use serde_json::Value;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    #[cfg(windows)]
    unsafe {
        native::execute(name, args)
    }
    #[cfg(not(windows))]
    {
        let _ = (name, args);
        Err("Desktop control currently requires Windows".into())
    }
}

pub(crate) fn image_point(
    origin: i32,
    extent: i32,
    image_extent: i32,
    value: i32,
) -> Result<i32, String> {
    if extent <= 0 || image_extent <= 0 || value < 0 || value >= image_extent {
        return Err("Invalid screenshot coordinate".into());
    }
    Ok(origin + ((value as i64 * extent as i64) / image_extent as i64) as i32)
}
fn key_codes(chord: &str) -> Result<Vec<u16>, String> {
    let tokens: Vec<_> = chord
        .split('+')
        .map(str::trim)
        .map(str::to_ascii_uppercase)
        .collect();
    if tokens.is_empty() || tokens.len() > 5 {
        return Err("Invalid key chord".into());
    }
    let mut keys = vec![];
    for (i, token) in tokens.iter().enumerate() {
        let modifier = match token.as_str() {
            "CTRL" | "CONTROL" => Some(0x11),
            "ALT" => Some(0x12),
            "SHIFT" => Some(0x10),
            "WIN" | "META" => Some(0x5b),
            _ => None,
        };
        let key = if i + 1 < tokens.len() {
            modifier.ok_or("Only modifiers can precede the final key")?
        } else {
            match token.as_str() {
                "CTRL" | "CONTROL" => 0x11,
                "ALT" => 0x12,
                "SHIFT" => 0x10,
                "WIN" | "META" => 0x5b,
                "ENTER" | "RETURN" => 0x0d,
                "TAB" => 9,
                "ESC" | "ESCAPE" => 0x1b,
                "SPACE" => 0x20,
                "BACKSPACE" => 8,
                "DELETE" => 0x2e,
                "LEFT" => 0x25,
                "UP" => 0x26,
                "RIGHT" => 0x27,
                "DOWN" => 0x28,
                "HOME" => 0x24,
                "END" => 0x23,
                "PAGEUP" => 0x21,
                "PAGEDOWN" => 0x22,
                s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphanumeric() => {
                    s.as_bytes()[0] as u16
                }
                s if s.starts_with('F') => s[1..]
                    .parse::<u16>()
                    .ok()
                    .filter(|n| (1..=12).contains(n))
                    .map(|n| 0x70 + n - 1)
                    .ok_or("Unsupported function key")?,
                _ => return Err("Unsupported key; use desktop_type for literal text".into()),
            }
        };
        if keys.contains(&key) {
            return Err("Duplicate key in chord".into());
        }
        keys.push(key);
    }
    Ok(keys)
}

#[cfg(windows)]
mod native {
    use super::*;
    use base64::Engine;
    use serde_json::json;
    use std::{
        ptr::{null, null_mut},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::Threading::*,
        UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    };

    fn window_id(window: HWND) -> String {
        unsafe {
            let mut pid = 0;
            GetWindowThreadProcessId(window, &mut pid);
            format!("{pid}:{:x}", window as usize)
        }
    }
    struct TypingPacer(HANDLE, crate::computer_gate::Gate);
    impl TypingPacer {
        unsafe fn new() -> Result<Self, String> {
            let gate = crate::computer_gate::input_allowed()?;
            let timer = CreateWaitableTimerExW(
                null(),
                null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS,
            );
            if timer.is_null() {
                return Err("Cannot pace interruptible keyboard input".into());
            }
            Ok(Self(timer, gate))
        }
        unsafe fn wait(&self) -> Result<(), String> {
            // SendInput can enqueue faster than an app consumes text. Keep the
            // backlog small, with a high-resolution timer that Esc can cancel.
            let due = -10_000i64;
            if SetWaitableTimer(self.0, &due, 0, None, null(), 0) == 0 {
                return Err("Cannot pace keyboard input".into());
            }
            let handles = [self.1.raw() as HANDLE, self.0];
            if WaitForMultipleObjects(2, handles.as_ptr(), 0, 200) != WAIT_OBJECT_0 + 1 {
                return Err(
                    "Keyboard input stopped; continue conversation or permitted non-control work. Do not retry input without renewed user authorization".into(),
                );
            }
            Ok(())
        }
    }
    impl Drop for TypingPacer {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    unsafe fn resolve(id: &str) -> Result<HWND, String> {
        let (pid, raw) = id
            .split_once(':')
            .ok_or("Invalid windowId; list windows first")?;
        let pid = pid.parse::<u32>().map_err(|_| "Invalid window process")?;
        let raw = usize::from_str_radix(raw, 16).map_err(|_| "Invalid window handle")?;
        let window = raw as HWND;
        let mut actual = 0;
        if IsWindow(window) == 0
            || GetWindowThreadProcessId(window, &mut actual) == 0
            || actual != pid
            || GetAncestor(window, GA_ROOT) != window
        {
            return Err("Window closed or changed; list windows again".into());
        }
        Ok(window)
    }
    unsafe fn title(window: HWND) -> String {
        let mut buf = [0u16; 512];
        let len = GetWindowTextW(window, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..len.max(0) as usize])
    }
    unsafe fn overlay(window: HWND) -> bool {
        let mut buf = [0u16; 96];
        let n = GetClassNameW(window, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..n.max(0) as usize]) == "SuperCodeDesktopGlow"
    }
    unsafe extern "system" fn enumerate(window: HWND, data: LPARAM) -> BOOL {
        if IsWindowVisible(window) != 0 && !overlay(window) {
            let name = title(window);
            if !name.is_empty() && GetWindow(window, GW_OWNER).is_null()
                || (!name.is_empty() && GetForegroundWindow() == window)
            {
                (*(data as *mut Vec<Value>)).push(json!({"windowId":window_id(window),"title":name,"minimized":IsIconic(window)!=0,"foreground":GetForegroundWindow()==window}));
            }
        }
        1
    }
    unsafe fn foreground(window: HWND) -> Result<(), String> {
        crate::computer_gate::verify_foreground(
            IsIconic(window) == 0 && IsWindowVisible(window) != 0 && GetForegroundWindow() == window,
            || {
                let name = std::env::var(crate::computer_gate::ENVIRONMENT)
                    .map_err(|_| "电脑操作停止机制未连接".to_owned())?;
                crate::computer_gate::Gate::revoke(&name)
            },
        )
    }
    unsafe fn geometry(window: HWND) -> Result<Value, String> {
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(window, &mut r) == 0 {
            return Err("Cannot read window bounds".into());
        }
        let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        r.left = r.left.max(left);
        r.top = r.top.max(top);
        r.right = r.right.min(left + GetSystemMetrics(SM_CXVIRTUALSCREEN));
        r.bottom = r.bottom.min(top + GetSystemMetrics(SM_CYVIRTUALSCREEN));
        if r.right <= r.left || r.bottom <= r.top {
            return Err("Window is outside the visible desktop".into());
        }
        Ok(json!({"left":r.left,"top":r.top,"width":r.right-r.left,"height":r.bottom-r.top}))
    }
    fn num(v: &Value, key: &str) -> Result<i32, String> {
        v[key]
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .ok_or_else(|| format!("Invalid {key}"))
    }
    fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
        v[key].as_str().ok_or_else(|| format!("Missing {key}"))
    }

    struct Capture {
        screen: HDC,
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
    }
    impl Drop for Capture {
        fn drop(&mut self) {
            unsafe {
                if !self.old.is_null() {
                    SelectObject(self.dc, self.old);
                }
                if !self.bitmap.is_null() {
                    DeleteObject(self.bitmap);
                }
                if !self.dc.is_null() {
                    DeleteDC(self.dc);
                }
                if !self.screen.is_null() {
                    ReleaseDC(null_mut(), self.screen);
                }
            }
        }
    }
    unsafe fn screenshot(rect: &Value) -> Result<(String, u32, u32), String> {
        let w = num(rect, "width")?;
        let h = num(rect, "height")?;
        if w <= 0 || h <= 0 || w as i64 * h as i64 > 16_777_216 {
            return Err("Screenshot exceeds memory limit".into());
        }
        // Scale before allocating pixels, keeping 4K capture memory bounded.
        let longest = w.max(h).max(1280);
        let iw = (w as i64 * 1280 / longest as i64).max(1) as i32;
        let ih = (h as i64 * 1280 / longest as i64).max(1) as i32;
        let screen = GetDC(null_mut());
        let dc = CreateCompatibleDC(screen);
        let mut capture = Capture {
            screen,
            dc,
            bitmap: null_mut(),
            old: null_mut(),
        };
        if screen.is_null() || dc.is_null() {
            return Err("Screen capture unavailable".into());
        }
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = iw;
        info.bmiHeader.biHeight = -ih;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut pixels = null_mut();
        capture.bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
        if capture.bitmap.is_null() || pixels.is_null() {
            return Err("Cannot allocate screenshot".into());
        }
        capture.old = SelectObject(dc, capture.bitmap);
        SetStretchBltMode(dc, HALFTONE);
        if StretchBlt(
            dc,
            0,
            0,
            iw,
            ih,
            screen,
            num(rect, "left")?,
            num(rect, "top")?,
            w,
            h,
            SRCCOPY | CAPTUREBLT,
        ) == 0
        {
            return Err("Screen capture failed".into());
        }
        GdiFlush();
        let bgra = std::slice::from_raw_parts(pixels as *const u8, iw as usize * ih as usize * 4);
        let mut rgb = Vec::with_capacity(iw as usize * ih as usize * 3);
        for p in bgra.chunks_exact(4) {
            rgb.extend_from_slice(&[p[2], p[1], p[0]]);
        }
        let image = image::RgbImage::from_raw(iw as u32, ih as u32, rgb)
            .ok_or("Invalid screenshot image")?;
        drop(capture);
        let (iw, ih) = image.dimensions();
        let mut jpeg = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
            .encode(image.as_raw(), iw, ih, image::ExtendedColorType::Rgb8)
            .map_err(|e| e.to_string())?;
        if jpeg.len() > 4 * 1024 * 1024 {
            return Err("Encoded screenshot exceeds limit".into());
        }
        Ok((
            base64::engine::general_purpose::STANDARD.encode(jpeg),
            iw,
            ih,
        ))
    }
    unsafe fn input(events: &[INPUT]) -> Result<(), String> {
        let _gate = crate::computer_gate::input_allowed()?;
        if SendInput(
            events.len() as u32,
            events.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        ) != events.len() as u32
        {
            return Err("Windows rejected input; check focus and application privilege level. Do not retry blindly.".into());
        }
        Ok(())
    }
    fn mouse(flags: u32, x: i32, y: i32, data: u32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: x,
                    dy: y,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: crate::computer_gate::INPUT_TAG,
                },
            },
        }
    }
    fn key(vk: u16, scan: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: crate::computer_gate::INPUT_TAG,
                },
            },
        }
    }
    unsafe fn movement(x: i32, y: i32) -> INPUT {
        let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        mouse(
            MOUSEEVENTF_MOVE
                | MOUSEEVENTF_MOVE_NOCOALESCE
                | MOUSEEVENTF_ABSOLUTE
                | MOUSEEVENTF_VIRTUALDESK,
            (((x - left) as i64 * 65535) / (w - 1).max(1) as i64) as i32,
            (((y - top) as i64 * 65535) / (h - 1).max(1) as i64) as i32,
            0,
        )
    }
    unsafe fn observed(args: &Value, x: &str, y: &str) -> Result<(HWND, i32, i32), String> {
        let state = &args["observed"];
        let window = resolve(string(state, "windowId")?)?;
        foreground(window)?;
        if geometry(window)? != state["bounds"] {
            return Err("Window moved or resized; take a new snapshot".into());
        }
        let r = &state["bounds"];
        let x = image_point(
            num(r, "left")?,
            num(r, "width")?,
            num(state, "imageWidth")?,
            num(args, x)?,
        )?;
        let y = image_point(
            num(r, "top")?,
            num(r, "height")?,
            num(state, "imageHeight")?,
            num(args, y)?,
        )?;
        let hit = WindowFromPoint(POINT { x, y });
        if GetAncestor(hit, GA_ROOT) != window {
            return Err("Target point is covered by another window; take a new snapshot".into());
        }
        Ok((window, x, y))
    }
    unsafe fn move_pointer(window: HWND, x: i32, y: i32) -> Result<(), String> {
        let mut start: POINT = std::mem::zeroed();
        GetCursorPos(&mut start);
        for step in 1..=12 {
            foreground(window)?;
            let t = step as f32 / 12.;
            let t = t * t * (3. - 2. * t);
            input(&[movement(
                start.x + ((x - start.x) as f32 * t) as i32,
                start.y + ((y - start.y) as f32 * t) as i32,
            )])?;
            std::thread::sleep(Duration::from_millis(12));
        }
        Ok(())
    }
    fn pointer_cue(x: i32, y: i32, click: bool) -> Result<(), String> {
        use std::{
            io::{BufRead, BufReader, Read, Write},
            net::{SocketAddr, TcpStream},
        };
        let _gate = crate::computer_gate::input_allowed()?;
        let address = std::env::var("SUPERCODE_COMPUTER_CUE_ADDRESS")
            .map_err(|_| "AI pointer cue is not connected")?;
        let address: SocketAddr = address.parse().map_err(|_| "Invalid pointer cue address")?;
        if !address.ip().is_loopback() {
            return Err("Pointer cue must be local".into());
        }
        let name = std::env::var(crate::computer_gate::ENVIRONMENT).map_err(|e| e.to_string())?;
        let token = name
            .strip_prefix("Local\\SuperCodeComputer-")
            .ok_or("Invalid input lease")?;
        let agent = std::env::var("SUPERCODE_COMPUTER_AGENT").map_err(|e| e.to_string())?;
        let mut socket = TcpStream::connect_timeout(&address, Duration::from_millis(500))
            .map_err(|e| e.to_string())?;
        socket
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(Duration::from_millis(500)))
            .map_err(|e| e.to_string())?;
        writeln!(
            socket,
            "{}",
            json!({"token":token,"agent":agent,"cursor":{"x":x,"y":y,"click":click}})
        )
        .map_err(|e| e.to_string())?;
        let mut reply = String::new();
        BufReader::new(socket)
            .take(16)
            .read_line(&mut reply)
            .map_err(|e| e.to_string())?;
        if reply != "ok\n" {
            return Err("AI pointer cue was revoked; cancel desktop input and continue conversation or permitted non-control work. Do not retry control without renewed user authorization".into());
        }
        Ok(())
    }
    /// Invoke an accessible button without moving the user's system pointer.
    /// Unsupported patterns fall back before side effects; failed invokes never retry.
    unsafe fn accessible_click(window: HWND, x: i32, y: i32) -> Result<bool, String> {
        use windows::{
            core::Interface,
            Win32::{System::Com::*, UI::Accessibility::*},
        };
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            return Ok(false);
        }
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _com = Com;
        let automation: IUIAutomation =
            match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                Ok(value) => value,
                Err(_) => return Ok(false),
            };
        let element = match automation.ElementFromPoint(windows::Win32::Foundation::POINT { x, y })
        {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        let mut pid = 0;
        GetWindowThreadProcessId(window, &mut pid);
        if element.CurrentProcessId().ok() != Some(pid as i32)
            || !element.CurrentIsEnabled().is_ok_and(|v| v.as_bool())
        {
            return Ok(false);
        }
        let invoke = match element
            .GetCurrentPattern(UIA_InvokePatternId)
            .and_then(|pattern| pattern.cast::<IUIAutomationInvokePattern>())
        {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        foreground(window)?;
        let _gate = crate::computer_gate::input_allowed()?;
        invoke.Invoke().map_err(|e| format!("Accessibility invoke outcome is unknown: {e}. Observe again; do not repeat the action."))?;
        Ok(true)
    }
    unsafe fn tree(window: HWND) -> Result<Value, String> {
        use windows::Win32::{Foundation::HWND as WWindow, System::Com::*, UI::Accessibility::*};
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _com = Com;
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| e.to_string())?;
        let root = automation
            .ElementFromHandle(WWindow(window))
            .map_err(|e| e.to_string())?;
        let condition = automation
            .CreateTrueCondition()
            .map_err(|e| e.to_string())?;
        let mut pending = vec![(root, 0)];
        let mut out = vec![];
        let started = Instant::now();
        while let Some((element, depth)) = pending.pop() {
            if out.len() >= 256 || started.elapsed() > Duration::from_millis(2000) {
                break;
            }
            let password = element
                .CurrentIsPassword()
                .map(|v| v.as_bool())
                .unwrap_or(false);
            let name = if password {
                "[password]".into()
            } else {
                element
                    .CurrentName()
                    .map(|v| v.to_string().chars().take(240).collect::<String>())
                    .unwrap_or_default()
            };
            let bounds=element.CurrentBoundingRectangle().ok().map(|r|json!({"left":r.left,"top":r.top,"width":r.right-r.left,"height":r.bottom-r.top}));
            out.push(json!({"name":name,"controlType":element.CurrentControlType().map(|v|v.0).unwrap_or(0),"bounds":bounds,"enabled":element.CurrentIsEnabled().map(|v|v.as_bool()).unwrap_or(false),"depth":depth}));
            if depth < 24 && !password {
                if let Ok(children) = element.FindAll(TreeScope_Children, &condition) {
                    let n = children
                        .Length()
                        .unwrap_or(0)
                        .min((256 - out.len()).min(256 - pending.len()) as i32);
                    for i in (0..n).rev() {
                        if let Ok(child) = children.GetElement(i) {
                            pending.push((child, depth + 1));
                        }
                    }
                }
            }
        }
        Ok(
            json!({"items":out,"truncated":!pending.is_empty(),"coordinateSystem":"physical desktop pixels; map using snapshot bounds and image dimensions"}),
        )
    }
    pub unsafe fn execute(name: &str, args: &Value) -> Result<Value, String> {
        // Both capture and input use physical virtual-desktop pixels, including negative monitors.
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        match name {
            "desktop_list_windows" => {
                let mut windows = vec![];
                EnumWindows(Some(enumerate), (&mut windows as *mut Vec<Value>) as isize);
                Ok(json!({"windows":windows,"foreground":window_id(GetForegroundWindow())}))
            }
            "desktop_focus_window" => {
                let _gate = crate::computer_gate::input_allowed()?;
                let window = resolve(string(args, "windowId")?)?;
                if IsIconic(window) != 0 {
                    ShowWindow(window, SW_RESTORE);
                }
                SetForegroundWindow(window);
                std::thread::sleep(Duration::from_millis(180));
                foreground(window)?;
                Ok(json!({"windowId":window_id(window),"focused":true}))
            }
            "desktop_snapshot" => {
                let window = if let Some(id) = args["windowId"].as_str() {
                    resolve(id)?
                } else {
                    GetForegroundWindow()
                };
                foreground(window)?;
                let bounds = geometry(window)?;
                let (image, w, h) = screenshot(&bounds)?;
                // Recheck after capture; don't associate another window's pixels with this identity.
                foreground(window)?;
                if geometry(window)? != bounds {
                    return Err("Window changed during screenshot; observe again".into());
                }
                let mut state = json!({"windowId":window_id(window),"title":title(window),"bounds":bounds,"imageWidth":w,"imageHeight":h,"capture":"visible foreground window","coordinates":"screenshot pixels"});
                if args["includeImage"] != false {
                    state["image"] = json!(image);
                }
                Ok(state)
            }
            "tree" => {
                let window = resolve(string(args, "windowId")?)?;
                foreground(window)?;
                tree(window)
            }
            "desktop_move" | "desktop_click" | "desktop_scroll" => {
                let (window, x, y) = observed(args, "x", "y")?;
                pointer_cue(x, y, name == "desktop_click")?;
                if name == "desktop_click"
                    && args["button"].as_str().unwrap_or("left") == "left"
                    && args["count"].as_u64().unwrap_or(1) == 1
                    && accessible_click(window, x, y)?
                {
                    return Ok(
                        json!({"completed":true,"input":"uia_invoke","systemPointerMoved":false,
                        "verify":"Inspect the new screenshot; Invoke delivery alone does not prove the intended outcome"}),
                    );
                }
                move_pointer(window, x, y)?;
                observed(args, "x", "y")?;
                if name == "desktop_click" {
                    let (down, up) = match args["button"].as_str().unwrap_or("left") {
                        "left" => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
                        "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                        "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
                        _ => return Err("Unknown mouse button".into()),
                    };
                    let mut events = vec![];
                    for _ in 0..args["count"].as_u64().unwrap_or(1) {
                        events.push(mouse(down, 0, 0, 0));
                        events.push(mouse(up, 0, 0, 0));
                    }
                    input(&events)?;
                } else if name == "desktop_scroll" {
                    let mut events = vec![];
                    for (key, flags) in [
                        ("vertical", MOUSEEVENTF_WHEEL),
                        ("horizontal", MOUSEEVENTF_HWHEEL),
                    ] {
                        if let Some(n) = args[key].as_i64().filter(|n| *n != 0) {
                            events.push(mouse(flags, 0, 0, (n as i32 * 120) as u32));
                        }
                    }
                    if !events.is_empty() {
                        input(&events)?;
                    }
                }
                Ok(
                    json!({"completed":true,"input":"foreground_send_input","systemPointerMoved":true,"verify":"Read a fresh snapshot to verify the result"}),
                )
            }
            "desktop_drag" => {
                let (window, x, y) = observed(args, "fromX", "fromY")?;
                let (_, tx, ty) = observed(args, "toX", "toY")?;
                pointer_cue(x, y, true)?;
                move_pointer(window, x, y)?;
                observed(args, "fromX", "fromY")?;
                // A single atomic input batch always includes release; killing a worker
                // cannot leave a held button in a later, unrelated application.
                let mut events = vec![mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, 0)];
                for i in 1..=32 {
                    events.push(movement(x + (tx - x) * i / 32, y + (ty - y) * i / 32));
                }
                events.push(mouse(MOUSEEVENTF_LEFTUP, 0, 0, 0));
                input(&events)?;
                Ok(json!({"completed":true,"verify":"Read a fresh snapshot to verify the drag"}))
            }
            "desktop_type" | "desktop_key" => {
                let window = resolve(string(args, "windowId")?)?;
                foreground(window)?;
                // Human-held modifiers can redirect input; wait for their release.
                for vk in [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN] {
                    if GetAsyncKeyState(vk as i32) < 0 {
                        return Err("A modifier key is held; release it before typing".into());
                    }
                }
                let mut events = vec![];
                if name == "desktop_type" {
                    let pacer = TypingPacer::new()?;
                    // One Unicode character per batch: stop cannot leave thousands of
                    // keystrokes queued. Each batch includes all key releases/surrogates.
                    for character in string(args, "text")?.chars() {
                        events.clear();
                        for unit in character.encode_utf16(&mut [0u16; 2]) {
                            events.push(key(0, *unit, KEYEVENTF_UNICODE));
                            events.push(key(0, *unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                        }
                        foreground(window)?;
                        input(&events)?;
                        pacer.wait()?;
                    }
                    events.clear();
                } else {
                    let codes = key_codes(string(args, "keys")?)?;
                    if codes == [VK_ESCAPE] {
                        // Global Esc belongs to the user. Dismiss the app's UI locally.
                        let _gate = crate::computer_gate::input_allowed()?;
                        let mut info: GUITHREADINFO = std::mem::zeroed();
                        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
                        let thread = GetWindowThreadProcessId(window, std::ptr::null_mut());
                        if GetGUIThreadInfo(thread, &mut info) == 0 {
                            return Err("Cannot identify the focused control".into());
                        }
                        let target = if info.hwndFocus.is_null() {
                            window
                        } else {
                            info.hwndFocus
                        };
                        if GetAncestor(target, GA_ROOT) != window {
                            return Err(
                                "Focused control changed; cancel desktop input and continue conversation or permitted non-control work. Do not retry control without renewed user authorization".into()
                            );
                        }
                        let scan = MapVirtualKeyW(VK_ESCAPE as u32, MAPVK_VK_TO_VSC) as isize;
                        if PostMessageW(target, WM_KEYDOWN, VK_ESCAPE as usize, 1 | (scan << 16))
                            == 0
                        {
                            return Err("Window rejected Escape".into());
                        }
                        PostMessageW(
                            target,
                            WM_KEYUP,
                            VK_ESCAPE as usize,
                            1 | (scan << 16) | (3isize << 30),
                        );
                        return Ok(json!({"completed":true,"input":"focused-control Escape"}));
                    }
                    for &vk in &codes {
                        events.push(key(vk, 0, 0));
                    }
                    for &vk in codes.iter().rev() {
                        events.push(key(vk, 0, KEYEVENTF_KEYUP));
                    }
                }
                foreground(window)?;
                if !events.is_empty() {
                    input(&events)?;
                }
                Ok(json!({"completed":true,"verify":"Read a fresh snapshot to verify the input"}))
            }
            _ => Err("Unknown native desktop operation".into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_scaled_images_and_negative_monitors() {
        assert_eq!(image_point(-1920, 1920, 960, 480).unwrap(), -960);
        assert_eq!(image_point(0, 2560, 1280, 1279).unwrap(), 2558);
        assert!(image_point(0, 1920, 960, 960).is_err());
    }
    #[test]
    fn key_chords_never_execute_shell_text() {
        assert_eq!(key_codes("Ctrl+Shift+A").unwrap(), [0x11, 0x10, 65]);
        assert_eq!(key_codes("F12").unwrap(), [0x7b]);
        assert_eq!(key_codes("SHIFT").unwrap(), [0x10]);
        assert_eq!(key_codes("CONTROL").unwrap(), [0x11]);
        for bad in [
            "Ctrl+Ctrl+A",
            "A+B",
            "powershell.exe",
            "F13",
            "CTRL+",
            "Alt+Enter; rm",
        ] {
            assert!(key_codes(bad).is_err(), "{bad}");
        }
    }
}
