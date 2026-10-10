//! A native fixture owned by this test, never a user's application.
#[cfg(windows)]
mod native {
    use std::{
        ptr::{null, null_mut},
        sync::{
            atomic::{AtomicIsize, AtomicUsize, Ordering},
            mpsc,
        },
        time::Duration,
    };
    use windows_sys::Win32::{
        Foundation::*, Graphics::Gdi::COLOR_WINDOW, System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    };
    static CLICKS: AtomicUsize = AtomicUsize::new(0);
    static WHEELS: AtomicUsize = AtomicUsize::new(0);
    static DRAGS: AtomicUsize = AtomicUsize::new(0);
    static ESCAPES: AtomicUsize = AtomicUsize::new(0);
    static EDIT_PROC: AtomicIsize = AtomicIsize::new(0);
    unsafe extern "system" fn edit_proc(
        window: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
    ) -> LRESULT {
        if matches!(message, WM_MOUSEWHEEL | WM_MOUSEHWHEEL) {
            WHEELS.fetch_add(1, Ordering::SeqCst);
        }
        if message == WM_KEYDOWN && w == 0x1b {
            ESCAPES.fetch_add(1, Ordering::SeqCst);
        }
        if message == WM_MOUSEMOVE && w & 1 != 0 {
            DRAGS.fetch_add(1, Ordering::SeqCst);
        }
        let original: WNDPROC = std::mem::transmute(EDIT_PROC.load(Ordering::SeqCst));
        CallWindowProcW(original, window, message, w, l)
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    unsafe extern "system" fn proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if message == WM_COMMAND && w & 0xffff == 1 && (w >> 16) & 0xffff == BN_CLICKED as usize {
            CLICKS.fetch_add(1, Ordering::SeqCst);
        }
        if message == WM_CLOSE {
            DestroyWindow(window);
            return 0;
        }
        if message == WM_DESTROY {
            PostQuitMessage(0);
            return 0;
        }
        DefWindowProcW(window, message, w, l)
    }
    pub struct Fixture {
        handle: usize,
        pub button: usize,
        pub edit: usize,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Fixture {
        pub fn start() -> Result<Self, String> {
            CLICKS.store(0, Ordering::SeqCst);
            WHEELS.store(0, Ordering::SeqCst);
            DRAGS.store(0, Ordering::SeqCst);
            ESCAPES.store(0, Ordering::SeqCst);
            let (tx, rx) = mpsc::sync_channel(1);
            let thread = std::thread::spawn(move || unsafe {
                let class = wide("SuperCodeNativeTestFixture");
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(proc),
                    hInstance: GetModuleHandleW(null()),
                    lpszClassName: class.as_ptr(),
                    hbrBackground: (COLOR_WINDOW + 1) as _,
                    ..std::mem::zeroed()
                };
                RegisterClassW(&wc);
                let window = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    wide("SuperCode Native Computer Test").as_ptr(),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    160,
                    140,
                    620,
                    380,
                    null_mut(),
                    null_mut(),
                    wc.hInstance,
                    null(),
                );
                let button = CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    wide("SC Native Button").as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    35,
                    40,
                    200,
                    45,
                    window,
                    1usize as _,
                    wc.hInstance,
                    null(),
                );
                let edit = CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    wide("EDIT").as_ptr(),
                    wide("").as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                    35,
                    120,
                    500,
                    40,
                    window,
                    2usize as _,
                    wc.hInstance,
                    null(),
                );
                EDIT_PROC.store(
                    SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize),
                    Ordering::SeqCst,
                );
                ShowWindow(window, SW_SHOW);
                SetForegroundWindow(window);
                let _ = tx.send((window as usize, button as usize, edit as usize));
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                UnregisterClassW(class.as_ptr(), wc.hInstance);
            });
            let (handle, button, edit) = rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
            if handle == 0 || button == 0 || edit == 0 {
                return Err("Native fixture creation failed".into());
            }
            Ok(Self {
                handle,
                button,
                edit,
                thread: Some(thread),
            })
        }
        pub fn id(&self) -> String {
            format!("{}:{:x}", std::process::id(), self.handle)
        }
        pub fn focus(&self) {
            unsafe {
                ShowWindow(self.handle as _, SW_RESTORE);
                SetForegroundWindow(self.handle as _);
            }
        }
        /// Exercise the real OS hooks with a foreign marker, like keyboard/remapping
        /// software. This is only used while this fixture owns the foreground.
        pub fn user_input(&self, mouse: bool, escape: bool) -> Result<(), String> {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
            unsafe {
                if GetForegroundWindow() != self.handle as _ {
                    return Err("Input fixture is not foreground".into());
                }
                let events = if mouse {
                    vec![INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: 1,
                                dy: 0,
                                mouseData: 0,
                                dwFlags: MOUSEEVENTF_MOVE,
                                time: 0,
                                dwExtraInfo: 0x55534552,
                            },
                        },
                    }]
                } else {
                    let vk = if escape { VK_ESCAPE } else { VK_F12 };
                    [0, KEYEVENTF_KEYUP]
                        .into_iter()
                        .map(|flags| INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: vk,
                                    wScan: 0,
                                    dwFlags: flags,
                                    time: 0,
                                    dwExtraInfo: 0x55534552,
                                },
                            },
                        })
                        .collect()
                };
                if SendInput(
                    events.len() as u32,
                    events.as_ptr(),
                    std::mem::size_of::<INPUT>() as i32,
                ) != events.len() as u32
                {
                    return Err("Native stop input injection failed".into());
                }
            }
            Ok(())
        }
        pub fn clicks(&self) -> usize {
            CLICKS.load(Ordering::SeqCst)
        }
        pub fn pointer(&self) -> (i32, i32) {
            unsafe {
                let mut p: POINT = std::mem::zeroed();
                GetCursorPos(&mut p);
                (p.x, p.y)
            }
        }
        pub fn wheels(&self) -> usize {
            WHEELS.load(Ordering::SeqCst)
        }
        pub fn drags(&self) -> usize {
            DRAGS.load(Ordering::SeqCst)
        }
        pub fn text(&self) -> String {
            unsafe {
                let mut b = [0u16; 256];
                let mut result = 0;
                if SendMessageTimeoutW(
                    self.edit as _,
                    WM_GETTEXT,
                    b.len(),
                    b.as_mut_ptr() as _,
                    SMTO_ABORTIFHUNG,
                    1000,
                    &mut result,
                ) == 0
                {
                    return String::new();
                }
                String::from_utf16_lossy(&b[..result.min(b.len() - 1)])
            }
        }
        pub fn text_length(&self) -> usize {
            unsafe { GetWindowTextLengthW(self.edit as _) as usize }
        }
        pub fn escapes(&self) -> usize {
            ESCAPES.load(Ordering::SeqCst)
        }
        pub fn point(
            &self,
            control: usize,
            snapshot: &serde_json::Value,
        ) -> Result<(i32, i32), String> {
            unsafe {
                let mut r: RECT = std::mem::zeroed();
                if GetWindowRect(control as _, &mut r) == 0 {
                    return Err("Fixture control closed".into());
                }
                let bounds = &snapshot["bounds"];
                let x = (r.left + r.right) / 2;
                let y = (r.top + r.bottom) / 2;
                Ok((
                    ((x as i64 - bounds["left"].as_i64().unwrap())
                        * snapshot["imageWidth"].as_i64().unwrap()
                        / bounds["width"].as_i64().unwrap()) as i32,
                    ((y as i64 - bounds["top"].as_i64().unwrap())
                        * snapshot["imageHeight"].as_i64().unwrap()
                        / bounds["height"].as_i64().unwrap()) as i32,
                ))
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe {
                PostMessageW(self.handle as _, WM_CLOSE, 0, 0);
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}
#[cfg(windows)]
pub use native::Fixture;
