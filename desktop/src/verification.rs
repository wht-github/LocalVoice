//! Explicit checks against an app-owned fixture, never arbitrary desktop inputs.
use anyhow::{Context, Result, bail, ensure};
use std::{
    sync::atomic::{AtomicIsize, Ordering},
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::w,
};
static EDIT_A: AtomicIsize = AtomicIsize::new(0);
static EDIT_B: AtomicIsize = AtomicIsize::new(0);
const NAME: windows::core::PCWSTR = w!("Local Voice controlled test fixture");
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_DESTROY => {
                PostQuitMessage(0);
                return LRESULT(0);
            }
            n if n == WM_APP + 1 => {
                let child = HWND(EDIT_A.load(Ordering::SeqCst) as *mut _);
                let _ = SetForegroundWindow(window);
                let _ = SetFocus(Some(child));
                SendMessageW(child, 0x00B1, Some(WPARAM(0)), Some(LPARAM(-1)));
                return LRESULT(0);
            }
            n if n == WM_APP + 2 => {
                let _ = SetFocus(Some(HWND(EDIT_B.load(Ordering::SeqCst) as *mut _)));
                return LRESULT(0);
            }
            n if n == WM_APP + 3 => {
                if let Ok(child) = GetDlgItem(Some(window), 103) {
                    let _ = SetFocus(Some(child));
                }
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(window, message, wparam, lparam)
    }
}
// A keyboard-driven custom editor deliberately exposes no UIA editing pattern.
unsafe extern "system" fn custom_editor(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_CHAR {
            let count = GetWindowTextLengthW(window).max(0) as usize;
            let mut buffer = vec![0u16; count + 2];
            GetWindowTextW(window, &mut buffer);
            buffer[count] = wparam.0 as u16;
            let _ = SetWindowTextW(window, windows::core::PCWSTR(buffer.as_ptr()));
            return LRESULT(0);
        }
        DefWindowProcW(window, message, wparam, lparam)
    }
}
pub fn fixture() -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: w!("LocalVoiceTestFixture"),
            ..Default::default()
        };
        ensure!(RegisterClassW(&class) != 0, "register fixture class");
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class.lpszClassName,
            NAME,
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            150,
            150,
            560,
            220,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            w!("测试选区 Hello world"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_MULTILINE as u32),
            15,
            15,
            500,
            60,
            Some(window),
            Some(HMENU(101 as *mut _)),
            Some(instance.into()),
            None,
        )?;
        let other = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            w!("保持不变"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            15,
            90,
            500,
            30,
            Some(window),
            Some(HMENU(102 as *mut _)),
            Some(instance.into()),
            None,
        )?;
        let custom_class = WNDCLASSW {
            lpfnWndProc: Some(custom_editor),
            hInstance: instance.into(),
            lpszClassName: w!("LocalVoiceCustomEditor"),
            ..Default::default()
        };
        ensure!(RegisterClassW(&custom_class) != 0, "register custom editor");
        CreateWindowExW(
            WS_EX_CLIENTEDGE,
            custom_class.lpszClassName,
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            15,
            130,
            500,
            25,
            Some(window),
            Some(HMENU(103 as *mut _)),
            Some(instance.into()),
            None,
        )?;
        EDIT_A.store(edit.0 as isize, Ordering::SeqCst);
        EDIT_B.store(other.0 as isize, Ordering::SeqCst);
        let _ = SetForegroundWindow(window);
        let _ = SetFocus(Some(edit));
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        Ok(())
    }
}
unsafe fn text(window: HWND) -> String {
    unsafe {
        let mut buffer = vec![0u16; 4096];
        let n = SendMessageW(
            window,
            WM_GETTEXT,
            Some(WPARAM(buffer.len())),
            Some(LPARAM(buffer.as_mut_ptr() as isize)),
        )
        .0;
        String::from_utf16_lossy(&buffer[..n as usize])
    }
}
pub fn native_check(expected_pid: u32, output: &str) -> Result<()> {
    unsafe {
        let deadline = Instant::now() + Duration::from_secs(8);
        let window = loop {
            if let Ok(window) = FindWindowW(w!("LocalVoiceTestFixture"), NAME) {
                let mut owner = 0;
                GetWindowThreadProcessId(window, Some(&mut owner));
                if owner == expected_pid
                    && GetDlgItem(Some(window), 101).is_ok()
                    && GetDlgItem(Some(window), 102).is_ok()
                {
                    break window;
                }
            }
            ensure!(
                Instant::now() < deadline,
                "Owned fixture controls did not finish opening"
            );
            thread::sleep(Duration::from_millis(50));
        };
        let mut pid = 0;
        GetWindowThreadProcessId(window, Some(&mut pid));
        ensure!(pid == expected_pid, "Refusing to test any other process");
        let edit = GetDlgItem(Some(window), 101).context("fixture first edit")?;
        let other = GetDlgItem(Some(window), 102).context("fixture second edit")?;
        // Activate only our verified fixture with a real click. Never bypass the
        // production foreground/target checks or click an overlapping window.
        if GetForegroundWindow() != window {
            SetWindowPos(
                window,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .context("raise owned fixture")?;
            let mut rect = RECT::default();
            GetWindowRect(window, &mut rect).context("owned fixture rectangle")?;
            let point = POINT {
                x: rect.left + 100,
                y: rect.top + 12,
            };
            ensure!(
                GetAncestor(WindowFromPoint(point), GA_ROOT) == window,
                "Fixture is covered; no click sent"
            );
            let mut old = POINT::default();
            GetCursorPos(&mut old).context("read cursor on interactive desktop")?;
            SetCursorPos(point.x, point.y).context("position cursor over owned fixture")?;
            let inputs: Vec<_> = [MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP]
                .into_iter()
                .map(|flag| INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dwFlags: flag,
                            ..Default::default()
                        },
                    },
                })
                .collect();
            let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            let _ = SetCursorPos(old.x, old.y);
            ensure!(sent == 2, "Fixture click failed");
            thread::sleep(Duration::from_millis(100));
        }
        SendMessageW(window, WM_APP + 1, None, None);
        let deadline = Instant::now() + Duration::from_secs(3);
        while GetForegroundWindow() != window && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        ensure!(
            GetForegroundWindow() == window,
            "Test fixture is not focused; no input was sent"
        );
        let automation = crate::native::Automation::new().context("automation initialization")?;
        let target = automation.target().context("capture editable target")?;
        let focus_timer = Instant::now();
        for _ in 0..100 {
            ensure!(
                automation.capture_target_matches(&target),
                "Capture focus check failed"
            );
        }
        let capture_focus_100_ms = focus_timer.elapsed().as_millis();
        ensure!(
            capture_focus_100_ms < 500,
            "Capture focus checks took too long"
        );
        automation
            .insert(&target, "你好 Rust 🙂")
            .context("Unicode insertion")?;
        thread::sleep(Duration::from_millis(150));
        let inserted = text(edit);
        ensure!(
            inserted == "你好 Rust 🙂",
            "Unicode insertion mismatch: {inserted}"
        );
        SendMessageW(window, WM_APP + 2, None, None);
        thread::sleep(Duration::from_millis(100));
        ensure!(!automation.matches(&target), "Focus change not detected");
        ensure!(
            automation.insert(&target, "不应插入").is_err(),
            "Stale target accepted"
        );
        ensure!(text(other) == "保持不变", "Other input was altered");
        // Manual dictation resolves the currently focused editor on stop, even
        // if it differs from the editor that happened to be focused on start.
        let stop_target = automation
            .target()
            .context("target selected at manual stop")?;
        automation.insert(&stop_target, "结束后一次输入 Hello")?;
        thread::sleep(Duration::from_millis(100));
        ensure!(
            text(other).contains("结束后一次输入 Hello"),
            "New stop target was not used"
        );
        ensure!(
            text(edit) == "你好 Rust 🙂",
            "Previous target changed after selecting stop target"
        );
        ensure!(
            !automation.capture_target_matches(&target),
            "Native focus change not detected"
        );
        SendMessageW(window, WM_APP + 3, None, None);
        thread::sleep(Duration::from_millis(100));
        let evidence = automation.inspect_focused_target(expected_pid)?;
        ensure!(
            evidence["previously_accepted"] == false,
            "Custom editor unexpectedly exposes a standard pattern"
        );
        let custom_target = automation.target()?;
        automation.insert(&custom_target, "自绘输入 Hello")?;
        thread::sleep(Duration::from_millis(100));
        ensure!(
            text(GetDlgItem(Some(window), 103)?) == "自绘输入 Hello",
            "Custom editor delivery failed"
        );
        crate::native::register_keys("Ctrl+Alt+Space").context("hotkey registration")?;
        let keys = [
            (VK_CONTROL, false),
            (VK_MENU, false),
            (VK_SPACE, false),
            (VK_SPACE, true),
            (VK_MENU, true),
            (VK_CONTROL, true),
        ];
        let inputs: Vec<_> = keys
            .into_iter()
            .map(|(key, up)| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: key,
                        dwFlags: if up {
                            KEYEVENTF_KEYUP
                        } else {
                            KEYBD_EVENT_FLAGS(0)
                        },
                        ..Default::default()
                    },
                },
            })
            .collect();
        ensure!(
            GetForegroundWindow() == window,
            "Fixture focus changed before hotkey test"
        );
        ensure!(
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) == inputs.len() as u32,
            "Hotkey input failed"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut seen = false;
        while Instant::now() < deadline {
            if crate::native::poll_keys().contains(&1) {
                seen = true;
                break;
            }
            thread::sleep(Duration::from_millis(15));
        }
        crate::native::unregister_keys();
        ensure!(seen, "Global hotkey was not delivered");
        std::fs::write(
            output,
            serde_json::to_vec_pretty(
                &serde_json::json!({"selection_read":true,"unicode_insert":true,"focus_change_rejected":true,"stale_target_did_not_alter_other_input":true,"manual_stop_uses_new_focus":true,"custom_editor_without_uia":true,"global_hotkey":true,"capture_focus_100_ms":capture_focus_100_ms}),
            )?,
        )?;
        Ok(())
    }
}
// Capture the actual client surface without asking Slint to render a snapshot:
// take_snapshot() itself invalidates rendering caches and would mask this bug.
fn surface(window: &slint::Window, output: &std::path::Path) -> Result<Vec<u8>> {
    capture_surface(window, output, false)
}

fn capture_surface(
    window: &slint::Window,
    output: &std::path::Path,
    composited: bool,
) -> Result<Vec<u8>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    use windows::Win32::Graphics::Gdi::*;
    let mut hwnd = HWND::default();
    window.with_winit_window(|w| {
        if let Ok(handle) = w.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                hwnd = HWND(h.hwnd.get() as *mut _);
            }
        }
    });
    unsafe {
        let mut owner = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut owner));
        ensure!(
            owner == windows::Win32::System::Threading::GetCurrentProcessId(),
            "Only capture our own window"
        );
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect)?;
        let (width, height) = (rect.right, rect.bottom);
        ensure!(width > 0 && height > 0, "Empty window surface");
        let source_window = if composited { None } else { Some(hwnd) };
        let mut origin = POINT::default();
        if composited {
            ensure!(
                ClientToScreen(hwnd, &mut origin).as_bool(),
                "Could not locate our own window"
            );
        }
        let source = GetDC(source_window);
        let memory = CreateCompatibleDC(Some(source));
        let bitmap = CreateCompatibleBitmap(source, width, height);
        let old = SelectObject(memory, bitmap.into());
        let copied = BitBlt(
            memory,
            0,
            0,
            width,
            height,
            Some(source),
            origin.x,
            origin.y,
            SRCCOPY,
        );
        SelectObject(memory, old);
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        let lines = GetDIBits(
            source,
            bitmap,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(source_window, source);
        copied?;
        ensure!(lines == height, "Incomplete window capture");
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        let mut encoder =
            png::Encoder::new(std::fs::File::create(output)?, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&pixels)?;
        Ok(pixels)
    }
}

pub fn restore_check(
    ui: &crate::VoiceWindow,
    timer: &slint::Timer,
    directory: String,
    unpatched: bool,
) {
    use slint::ComponentHandle;
    use std::cell::{Cell, RefCell};
    let weak = ui.as_weak();
    let started = Instant::now();
    let step = Cell::new(0usize);
    let baseline = RefCell::new(Vec::new());
    let comparisons = RefCell::new(Vec::new());
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(400), move || {
        if started.elapsed() < Duration::from_secs(2) { return; }
        let result = (|| -> Result<()> {
            let ui = weak.upgrade().context("Window closed")?;
            let path = std::path::Path::new(&directory);
            std::fs::create_dir_all(path)?;
            let index = step.get();
            if index == 0 {
                *baseline.borrow_mut() = surface(ui.window(), &path.join("before.png"))?;
                ui.hide()?;
            } else if index % 2 == 1 {
                if unpatched { ui.show()?; } else { crate::ui::show_complete(ui.window())?; }
            } else {
                let pixels = surface(ui.window(), &path.join(format!("restored-{}.png", index / 2)))?;
                let previous = baseline.borrow();
                let different = pixels.iter().zip(previous.iter()).filter(|(a,b)| a != b).count();
                comparisons.borrow_mut().push(serde_json::json!({"cycle":index/2,"different_bytes":different,"same_size":pixels.len()==previous.len()}));
                if index == 6 {
                    std::fs::write(path.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({"unpatched":unpatched,"cycles":*comparisons.borrow(),"note":"Captured own displayed surface, no mouse movement or Slint snapshot"}))?)?;
                    slint::quit_event_loop()?;
                } else { ui.hide()?; }
            }
            step.set(index + 1);
            Ok(())
        })();
        if let Err(error) = result {
            let _ = std::fs::write(std::path::Path::new(&directory).join("error.txt"), error.to_string());
            let _ = slint::quit_event_loop();
        }
    });
}

/// Send display messages only to our own fixture windows. Never changes the
/// desktop's resolution/DPI, starts models, records audio, or types into apps.
pub fn display_check(directory: &str, unpatched: bool) -> Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use windows::Win32::Graphics::Gdi::*;
    let backend = i_slint_backend_winit::Backend::builder()
        .with_renderer_name(crate::ui::RENDERER)
        .with_window_attributes_hook(|attributes| {
            use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_active(false).with_skip_taskbar(true)
        })
        .build()?;
    slint::platform::set_platform(Box::new(backend))?;
    let ui = crate::VoiceWindow::new()?;
    ui.set_status("显示变更测试".into());
    let settings = crate::SettingsWindow::new()?;
    for window in [ui.window(), settings.window()] {
        let mut state = crate::ui::DisplayState::default();
        window.on_winit_window_event(move |window, event| {
            if !unpatched {
                state.event(window, event);
            }
            i_slint_backend_winit::EventResult::Propagate
        });
    }
    ui.show()?;
    let original_dpi = (ui.window().scale_factor() * 96.).round() as u32;
    let cases = [false, true]
        .into_iter()
        .flat_map(|settings| {
            [
                (Some(96), false),
                (Some(144), false),
                (Some(192), false),
                (Some(original_dpi), false),
                (None, false),
                (None, true),
            ]
            .map(|(dpi, offscreen)| (settings, dpi, offscreen))
        })
        .collect::<Vec<_>>();
    let output = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&output)?;
    let weak_ui = ui.as_weak();
    let weak_settings = settings.as_weak();
    let step = Cell::new(0usize);
    let before = RefCell::new(Vec::new());
    let reports = RefCell::new(Vec::new());
    let failure = Rc::new(RefCell::new(None));
    let error = failure.clone();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(450), move || {
        let result = (|| -> Result<()> {
            let ui = weak_ui.upgrade().context("Window closed")?;
            let settings = weak_settings.upgrade().context("Settings closed")?;
            let index = step.get() / 4;
            let phase = step.get() % 4;
            let (is_settings, dpi, offscreen) = cases[index];
            let window = if is_settings { settings.window() } else { ui.window() };
            let mut hwnd = HWND::default();
            if phase == 0 {
                ui.hide()?;
                settings.hide()?;
                window.show()?;
            }
            window.with_winit_window(|w| {
                if let Ok(handle) = w.window_handle() {
                    if let RawWindowHandle::Win32(h) = handle.as_raw() {
                        hwnd = HWND(h.hwnd.get() as *mut _);
                    }
                }
            });
            match phase {
                0 => unsafe {
                    SetWindowPos(hwnd, Some(HWND_TOPMOST), 80, 80, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE)?;
                    if let Some(dpi) = dpi {
                        let mut rect = RECT::default();
                        GetWindowRect(hwnd, &mut rect)?;
                        SendMessageW(hwnd, WM_DPICHANGED, Some(WPARAM((dpi | dpi << 16) as usize)), Some(LPARAM(&rect as *const _ as isize)));
                    }
                },
                1 if dpi.is_none() => unsafe {
                    // Independently model surface loss and off-screen placement
                    // during a display reset, without touching global settings.
                    let mut monitor = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                    ensure!(GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut monitor).as_bool());
                    if offscreen {
                        SetWindowPos(hwnd, None, monitor.rcWork.right - 20, monitor.rcWork.bottom - 20, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)?;
                    } else {
                    let dc = GetDC(Some(hwnd));
                    let mut rect = RECT::default();
                    GetClientRect(hwnd, &mut rect)?;
                    FillRect(dc, &rect, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                    ReleaseDC(Some(hwnd), dc);
                    }
                    SendMessageW(hwnd, WM_DISPLAYCHANGE, Some(WPARAM(32)), Some(LPARAM(0)));
                },
                2 => {
                    *before.borrow_mut() = surface(window, &output.join(format!("{index}-actual.png")))?;
                    crate::ui::mark_complete(window);
                    window.request_redraw();
                }
                3 => {
                    let reference = surface(window, &output.join(format!("{index}-full.png")))?;
                    let different = before.borrow().iter().zip(&reference).filter(|(a,b)| a != b).count();
                    let mut rect = RECT::default();
                    let mut client = RECT::default();
                    let mut monitor = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                    unsafe {
                        GetWindowRect(hwnd, &mut rect)?;
                        GetClientRect(hwnd, &mut client)?;
                        ensure!(GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut monitor).as_bool());
                    }
                    let scale = window.scale_factor();
                    let expected = if is_settings { (560.,640.) } else { (380.,166.) };
                    let size_ok = client.right == (expected.0 * scale).round() as i32 && client.bottom == (expected.1 * scale).round() as i32;
                    let visible = rect.left >= monitor.rcWork.left && rect.top >= monitor.rcWork.top && rect.right <= monitor.rcWork.right && rect.bottom <= monitor.rcWork.bottom;
                    reports.borrow_mut().push(serde_json::json!({"settings":is_settings,"dpi":dpi,"offscreen":offscreen,"scale":scale,
                        "physical_size":[client.right,client.bottom],"size_ok":size_ok,"visible":visible,
                        "different_bytes":different,"same_buffer_size":before.borrow().len()==reference.len()}));
                    if index + 1 == cases.len() {
                        std::fs::write(output.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({"unpatched":unpatched,"cases":*reports.borrow(),"note":"Synthetic DPI/display messages to own windows; no global resolution changes. Actual GDI surface compared with an explicit full repaint."}))?)?;
                        if !unpatched {
                            ensure!(reports.borrow().iter().all(|row| row["different_bytes"] == 0
                                && row["same_buffer_size"] == true && row["size_ok"] == true
                                && row["visible"] == true), "Display recovery regression; see result.json");
                        }
                        slint::quit_event_loop()?;
                    }
                }
                _ => {}
            }
            step.set(step.get()+1);
            Ok(())
        })();
        if let Err(e) = result {
            *error.borrow_mut() = Some(format!("{e:#}"));
            let _ = slint::quit_event_loop();
        }
    });
    slint::run_event_loop_until_quit()?;
    if let Some(error) = failure.borrow_mut().take() {
        bail!(error);
    }
    Ok(())
}

/// Exercise tray restoration and compact presentation using only an owned
/// window. No services, microphone, keyboard input, or foreground activation.
pub fn floating_check(directory: &str, unpatched: bool) -> Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use windows::Win32::Graphics::Gdi::*;

    let args: Vec<_> = std::env::args().collect();
    let test_dpi = args
        .iter()
        .position(|arg| arg == "--dpi")
        .map(|index| {
            args.get(index + 1)
                .context("--dpi requires 96, 144 or 192")?
                .parse::<u32>()
                .map_err(anyhow::Error::from)
        })
        .transpose()?;
    ensure!(
        test_dpi.is_none_or(|dpi| [96, 144, 192].contains(&dpi)),
        "Use --dpi 96, 144 or 192"
    );

    #[derive(Clone, Copy)]
    enum Case {
        NativeHidden,
        Minimized,
        HiddenOffscreen,
        DisplayOffscreen,
        ResumeOffscreen,
        CompactIdle,
        CompactStarting,
        CompactRecording,
        CompactPending,
        ExpandedPending,
    }
    impl Case {
        fn name(self) -> &'static str {
            match self {
                Self::NativeHidden => "native-hidden",
                Self::Minimized => "minimized",
                Self::HiddenOffscreen => "hidden-offscreen",
                Self::DisplayOffscreen => "display-offscreen",
                Self::ResumeOffscreen => "resume-offscreen",
                Self::CompactIdle => "compact-idle",
                Self::CompactStarting => "compact-starting",
                Self::CompactRecording => "compact-recording",
                Self::CompactPending => "compact-pending",
                Self::ExpandedPending => "expanded-pending",
            }
        }
        fn compact(self) -> bool {
            matches!(
                self,
                Self::CompactIdle
                    | Self::CompactStarting
                    | Self::CompactRecording
                    | Self::CompactPending
            )
        }
        fn pending(self) -> bool {
            matches!(self, Self::CompactPending | Self::ExpandedPending)
        }
    }

    let backend = i_slint_backend_winit::Backend::builder()
        .with_renderer_name(crate::ui::RENDERER)
        .with_window_attributes_hook(|attributes| {
            use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_active(false).with_skip_taskbar(true)
        })
        .build()?;
    slint::platform::set_platform(Box::new(backend))?;
    let ui = crate::VoiceWindow::new()?;
    let weak = ui.as_weak();
    let mut floating = crate::ui::FloatingState::default();
    let drag_count = Rc::new(Cell::new(0));
    let drags = drag_count.clone();
    ui.on_drag(move || drags.set(drags.get() + 1));
    let expand = ui.as_weak();
    ui.on_expand_window(move || {
        if let Some(ui) = expand.upgrade() {
            let _ = crate::ui::show_panel(&ui);
        }
    });
    ui.window().on_winit_window_event(move |_, event| {
        if let Some(ui) = weak.upgrade() {
            floating.event(&ui, event);
        }
        i_slint_backend_winit::EventResult::Propagate
    });
    crate::ui::show_panel(&ui)?;

    let cases = [
        Case::NativeHidden,
        Case::Minimized,
        Case::HiddenOffscreen,
        Case::DisplayOffscreen,
        Case::ResumeOffscreen,
        Case::CompactIdle,
        Case::CompactStarting,
        Case::CompactRecording,
        Case::CompactPending,
        Case::ExpandedPending,
    ];
    let output = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&output)?;
    let step = Cell::new(0usize);
    let prepared = RefCell::new(serde_json::Value::Null);
    let earlier_frame = RefCell::new(Vec::<u8>::new());
    let focus_preserved = Cell::new(true);
    let reports = RefCell::new(Vec::<serde_json::Value>::new());
    let failure = Rc::new(RefCell::new(None));
    let error = failure.clone();
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(400), move || {
        let result = (|| -> Result<()> {
            let ui = weak.upgrade().context("Floating fixture closed")?;
            let index = step.get() / 4;
            let phase = step.get() % 4;
            let case = cases[index];
            let mut hwnd = HWND::default();
            ui.window().with_winit_window(|window| {
                if let Ok(handle) = window.window_handle() {
                    if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                        hwnd = HWND(handle.hwnd.get() as *mut _);
                    }
                }
            });
            unsafe {
                let mut owner = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut owner));
                ensure!(
                    owner == windows::Win32::System::Threading::GetCurrentProcessId(),
                    "Only manipulate the owned floating fixture"
                );
                ensure!(GetForegroundWindow() != hwnd, "Floating fixture took foreground focus");
            }
            // Compare around our synchronous actions, not across the whole
            // run: the user may legitimately switch between other apps.
            let foreground = unsafe { GetForegroundWindow() };
            match phase {
                0 => {
                    focus_preserved.set(true);
                    // Reset through the working path so ablation cases are independent.
                    ui.set_recording(false);
                    ui.set_capturing(false);
                    ui.set_has_pending(false);
                    ui.set_level(0.);
                    ui.set_status("准备就绪 · 点击开始说话".into());
                    crate::ui::show_panel(&ui)?;
                    if let Some(dpi) = test_dpi {
                        unsafe {
                            let mut rect = RECT::default();
                            GetWindowRect(hwnd, &mut rect)?;
                            SendMessageW(hwnd, WM_DPICHANGED, Some(WPARAM((dpi | dpi << 16) as usize)), Some(LPARAM(&rect as *const _ as isize)));
                        }
                    }
                }
                1 => unsafe {
                    match case {
                        Case::NativeHidden => {
                            let _ = ShowWindow(hwnd, SW_HIDE);
                        }
                        Case::Minimized => {
                            let _ = ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
                        }
                        Case::HiddenOffscreen | Case::DisplayOffscreen | Case::ResumeOffscreen => {
                            if matches!(case, Case::HiddenOffscreen) {
                                ui.hide()?;
                            }
                            let x = GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) + 500;
                            let y = GetSystemMetrics(SM_YVIRTUALSCREEN) + GetSystemMetrics(SM_CYVIRTUALSCREEN) + 500;
                            SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)?;
                            match case {
                                Case::DisplayOffscreen => {
                                    SendMessageW(hwnd, WM_DISPLAYCHANGE, Some(WPARAM(32)), Some(LPARAM(0)));
                                }
                                Case::ResumeOffscreen => {
                                    SendMessageW(hwnd, WM_POWERBROADCAST, Some(WPARAM(PBT_APMRESUMEAUTOMATIC as usize)), Some(LPARAM(0)));
                                }
                                _ => {}
                            }
                        }
                        _ => {
                            ui.set_recording(matches!(case, Case::CompactStarting | Case::CompactRecording));
                            ui.set_capturing(matches!(case, Case::CompactRecording));
                            ui.set_has_pending(case.pending());
                            ui.set_level(if matches!(case, Case::CompactRecording) { 0.72 } else { 0. });
                            crate::ui::set_compact(&ui, true);
                        }
                    }
                    *prepared.borrow_mut() = serde_json::json!({
                        "native_visible": IsWindowVisible(hwnd).as_bool(),
                        "slint_visible": ui.window().is_visible(),
                        "minimized": IsIconic(hwnd).as_bool(),
                    });
                },
                2 if case.compact() => {
                    if matches!(case, Case::CompactIdle) {
                        let before = surface(ui.window(), &output.join("hover-before.png"))?;
                        unsafe {
                            SendMessageW(hwnd, WM_NCPAINT, Some(WPARAM(1)), Some(LPARAM(0)));
                            SendMessageW(hwnd, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0)));
                            ensure!(SendMessageW(hwnd, WM_NCHITTEST, Some(WPARAM(0)), Some(LPARAM(0))) == LRESULT(HTCLIENT as isize), "Floating window must have no native caption hit area");
                        }
                        let after = surface(ui.window(), &output.join("hover-after.png"))?;
                        let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
                        std::fs::write(output.join("native-frame.json"), serde_json::to_vec_pretty(&serde_json::json!({"changed_bytes": changed}))?)?;
                        ensure!(changed == 0, "Native caption painting corrupted the floating ball");
                    }
                    *earlier_frame.borrow_mut() = ui.window().take_snapshot()?.as_bytes().to_vec();
                }
                2 => {
                    if unpatched && !matches!(case, Case::ExpandedPending) {
                        // Previous tray behavior: Slint show can be a no-op when
                        // Win32 visibility/minimization differs from its cache.
                        ui.show()?;
                        crate::ui::mark_complete(ui.window());
                        ui.window().request_redraw();
                    } else {
                        crate::ui::show_panel(&ui)?;
                    }
                }
                3 => unsafe {
                    let mut rect = RECT::default();
                    let mut client = RECT::default();
                    let mut monitor = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                    GetWindowRect(hwnd, &mut rect)?;
                    GetClientRect(hwnd, &mut client)?;
                    ensure!(GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut monitor).as_bool());
                    let visible = IsWindowVisible(hwnd).as_bool();
                    let minimized = IsIconic(hwnd).as_bool();
                    let in_work_area = rect.left >= monitor.rcWork.left && rect.top >= monitor.rcWork.top
                        && rect.right <= monitor.rcWork.right && rect.bottom <= monitor.rcWork.bottom;
                    let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    let no_activate = style & WS_EX_NOACTIVATE.0 as isize != 0;
                    let topmost = style & WS_EX_TOPMOST.0 as isize != 0;
                    let scale = ui.window().scale_factor();
                    let expected = if case.compact() { (64., 64.) } else if case.pending() { (380., 202.) } else { (380., 166.) };
                    let size_ok = client.right == (expected.0 * scale).round() as i32
                        && client.bottom == (expected.1 * scale).round() as i32;
                    let region = CreateRectRgn(0, 0, 0, 0);
                    let region_type = GetWindowRgn(hwnd, region);
                    let center_in_region = PtInRegion(region, client.right / 2, client.bottom / 2).as_bool();
                    let corner_in_region = PtInRegion(region, 0, 0).as_bool();
                    let mut region_bounds = RECT::default();
                    let _ = GetRgnBox(region, &mut region_bounds);
                    let shape_ok = if case.compact() {
                        region_type == COMPLEXREGION && center_in_region && !corner_in_region
                    } else {
                        // Windows may install a full rectangular region when
                        // updating a borderless window. Only clipping is wrong.
                        region_type.0 == 0 || (region_type == SIMPLEREGION
                            && region_bounds.left == 0 && region_bounds.top == 0
                            && region_bounds.right == client.right && region_bounds.bottom == client.bottom)
                    };
                    let _ = DeleteObject(region.into());
                    let foreground_unchanged = focus_preserved.get() && GetForegroundWindow() != hwnd;
                    let state_ok = ui.get_compact() == case.compact()
                        && ui.get_has_pending() == case.pending()
                        && ui.get_recording() == matches!(case, Case::CompactStarting | Case::CompactRecording)
                        && ui.get_capturing() == matches!(case, Case::CompactRecording);
                    if visible && !minimized && in_work_area {
                        surface(ui.window(), &output.join(format!("{}.png", case.name())))?;
                        if case.compact() {
                            capture_surface(ui.window(), &output.join(format!("{}-desktop.png", case.name())), true)?;
                        }
                    }
                    let mut alpha_edge_pixels = 0;
                    let mut animation_changed_bytes = 0;
                    if case.compact() {
                        let frame = ui.window().take_snapshot()?;
                        let pixels = frame.as_bytes();
                        let mut encoder = png::Encoder::new(std::fs::File::create(output.join(format!("{}-rgba.png", case.name())))?, frame.width(), frame.height());
                        encoder.set_color(png::ColorType::Rgba);
                        encoder.set_depth(png::BitDepth::Eight);
                        encoder.write_header()?.write_image_data(pixels)?;
                        alpha_edge_pixels = pixels.chunks_exact(4).filter(|pixel| pixel[3] > 0 && pixel[3] < 255).count();
                        ensure!(pixels[3] == 0 && alpha_edge_pixels > 20, "Ball needs transparent margins and antialiased alpha edges");
                        animation_changed_bytes = pixels.iter().zip(earlier_frame.borrow().iter()).filter(|(a,b)| a != b).count();
                        let animated = matches!(case, Case::CompactStarting | Case::CompactRecording);
                        ensure!((animation_changed_bytes > 0) == animated, "Active ball must animate; idle ball must settle");
                    }
                    if matches!(case, Case::CompactRecording) {
                        // Exercise the actual ball TouchArea, with a local drag
                        // callback instead of entering Windows' mouse move loop.
                        use slint::platform::{PointerEventButton, WindowEvent};
                        let start = slint::LogicalPosition::new(32., 26.);
                        let end = slint::LogicalPosition::new(44., 26.);
                        ui.window().dispatch_event(WindowEvent::PointerMoved { position: start });
                        ui.window().dispatch_event(WindowEvent::PointerPressed { position: start, button: PointerEventButton::Left });
                        ui.window().dispatch_event(WindowEvent::PointerMoved { position: end });
                        ui.window().dispatch_event(WindowEvent::PointerReleased { position: end, button: PointerEventButton::Left });
                        ensure!(ui.get_compact() && drag_count.get() == 1, "Dragging the ball must not expand it: compact={}, drags={}", ui.get_compact(), drag_count.get());
                        ui.window().dispatch_event(WindowEvent::PointerMoved { position: start });
                        ui.window().dispatch_event(WindowEvent::PointerPressed { position: start, button: PointerEventButton::Left });
                        ui.window().dispatch_event(WindowEvent::PointerReleased { position: start, button: PointerEventButton::Left });
                        ensure!(!ui.get_compact() && ui.get_recording() && ui.get_capturing(), "Click must expand without stopping capture");
                    }
                    reports.borrow_mut().push(serde_json::json!({
                        "case": case.name(), "prepared": *prepared.borrow(),
                        "visible": visible, "minimized": minimized, "in_work_area": in_work_area,
                        "no_activate": no_activate, "topmost": topmost,
                        "foreground_unchanged": foreground_unchanged, "scale": scale,
                        "physical_size": [client.right, client.bottom], "size_ok": size_ok,
                        "region_type": region_type.0, "center_in_region": center_in_region,
                        "corner_in_region": corner_in_region, "shape_ok": shape_ok,
                        "state_ok": state_ok,
                        "alpha_edge_pixels": alpha_edge_pixels, "animation_changed_bytes": animation_changed_bytes,
                        "passed": visible && !minimized && in_work_area && no_activate && topmost
                            && foreground_unchanged && size_ok && shape_ok && state_ok,
                    }));
                    if index + 1 == cases.len() {
                        let all_passed = reports.borrow().iter().all(|row| row["passed"] == true);
                        let ablation_detected = reports.borrow().iter().take(5).any(|row| row["passed"] == false);
                        std::fs::write(output.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({
                            "unpatched": unpatched, "all_passed": all_passed,
                            "ablation_detected": ablation_detected, "cases": *reports.borrow(),
                            "note": "Owned window only; synthetic display/resume messages. Capture state is simulated. No services, audio, system display changes, activation, or input to other apps."
                        }))?)?;
                        ensure!(if unpatched { ablation_detected } else { all_passed }, "Floating recovery regression; see result.json");
                        slint::quit_event_loop()?;
                    }
                }
                _ => {}
            }
            focus_preserved.set(focus_preserved.get() && unsafe { GetForegroundWindow() == foreground });
            step.set(step.get() + 1);
            Ok(())
        })();
        if let Err(e) = result {
            let message = format!("{e:#}");
            let _ = std::fs::write(output.join("error.txt"), &message);
            *error.borrow_mut() = Some(message);
            let _ = slint::quit_event_loop();
        }
    });
    slint::run_event_loop_until_quit()?;
    if let Some(error) = failure.borrow_mut().take() {
        bail!(error);
    }
    Ok(())
}

/// Run the production presentation controller in an isolated window. Pointer
/// events are local to this fixture; the user's cursor and audio are untouched.
pub fn interaction_check(directory: &str) -> Result<()> {
    use slint::{
        ComponentHandle,
        winit_030::{WinitWindowAccessor, winit},
    };
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    let backend = i_slint_backend_winit::Backend::builder()
        .with_renderer_name(crate::ui::RENDERER)
        .with_window_attributes_hook(|attributes| {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_active(false).with_skip_taskbar(true)
        })
        .build()?;
    slint::platform::set_platform(Box::new(backend))?;
    let ui = crate::VoiceWindow::new()?;
    let presentation = crate::ui::Presentation::new(&ui);
    let expand = presentation.clone();
    ui.on_expand_window(move || {
        let _ = expand.show_panel();
    });
    let collapse = presentation.clone();
    ui.on_collapse_window(move || {
        let _ = collapse.show_ball();
    });
    let weak = ui.as_weak();
    let mut floating = crate::ui::FloatingState::default();
    ui.window().on_winit_window_event(move |_, event| {
        if let Some(ui) = weak.upgrade() {
            floating.event(&ui, event);
        }
        // Use only synthetic enter/leave below, so the real cursor cannot
        // invalidate the timing assertions while the user works elsewhere.
        i_slint_backend_winit::EventResult::Propagate
    });
    presentation.show_ball()?;
    let output = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&output)?;
    let phase = Cell::new(0);
    let anchor = Cell::new(slint::PhysicalPosition::default());
    let failure = Rc::new(RefCell::new(None));
    let error = failure.clone();
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(1600), move || {
        let result = (|| -> Result<()> {
            let ui = weak.upgrade().context("Fixture closed")?;
            let left = winit::event::WindowEvent::CursorLeft { device_id: winit::event::DeviceId::dummy() };
            let entered = winit::event::WindowEvent::CursorEntered { device_id: winit::event::DeviceId::dummy() };
            let foreground = unsafe { GetForegroundWindow() };
            match phase.get() {
                0 => {
                    ensure!(ui.get_compact() && ui.window().size().to_logical(ui.window().scale_factor()).width == 64., "Startup must show ball");
                    // Left edge expansion is clamped. Collapsing must still
                    // recover the ball's original location without drift.
                    ui.window().set_position(slint::PhysicalPosition::new(12, 160));
                }
                1 => {
                    anchor.set(ui.window().position());
                    ui.set_recording(true);
                    ui.set_capturing(true);
                    ui.invoke_expand_window();
                    presentation.event(&left);
                    presentation.event(&entered);
                }
                2 => {
                    ensure!(!ui.get_compact(), "Returning pointer must cancel collapse");
                    presentation.event(&left);
                }
                3 => {
                    ensure!(ui.get_compact() && ui.window().position() == anchor.get(), "Auto collapse must return to ball anchor");
                    ensure!(ui.get_capturing() && ui.get_recording(), "Presentation must preserve capture");
                    ui.invoke_expand_window();
                    ui.set_has_pending(true);
                    presentation.event(&left);
                }
                4 => {
                    ensure!(!ui.get_compact(), "Pending result must keep panel open");
                    let size = ui.window().size().to_logical(ui.window().scale_factor());
                    ensure!((size.height - 202.).abs() < 1., "Pending result must expand panel height: {size:?}");
                    ui.set_has_pending(false);
                }
                5 => {
                    ensure!(ui.get_compact(), "Panel must collapse after pending result is handled");
                    ui.invoke_expand_window();
                    presentation.hide()?;
                    ensure!(!ui.window().is_visible() && !ui.get_animation_enabled(), "Hide must stop drawing");
                    presentation.show_ball()?;
                    presentation.show_ball()?;
                }
                6 => {
                    ensure!(ui.get_compact() && ui.window().is_visible() && ui.window().position() == anchor.get(), "Repeated tray reveal must restore ball without drift");
                    ui.invoke_expand_window();
                    ui.invoke_collapse_window();
                }
                7 => {
                    ensure!(ui.get_compact() && ui.window().position() == anchor.get(), "Explicit collapse must restore anchor");
                    std::fs::write(output.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({
                        "passed": true,
                        "checks": ["startup ball", "reentry cancels collapse", "leave collapses", "edge anchor restored", "capture preserved", "pending keeps panel open", "pending resizes panel", "cleared pending collapses", "hidden animation stops", "repeated tray reveal", "explicit collapse"],
                        "note": "Production presentation controller; synthetic local pointer events and capture state; no services or audio."
                    }))?)?;
                    slint::quit_event_loop()?;
                }
                _ => unreachable!(),
            }
            ensure!(unsafe { GetForegroundWindow() } == foreground, "Presentation stole foreground focus");
            phase.set(phase.get() + 1);
            Ok(())
        })();
        if let Err(e) = result {
            let message = format!("{e:#}");
            let _ = std::fs::write(output.join("error.txt"), &message);
            *error.borrow_mut() = Some(message);
            let _ = slint::quit_event_loop();
        }
    });
    slint::run_event_loop_until_quit()?;
    if let Some(error) = failure.borrow_mut().take() {
        bail!(error);
    }
    Ok(())
}

/// Check the floating window against an overlapping ordinary window owned by
/// this process. Neither fixture activates or sends input to other applications.
pub fn topmost_check(directory: &str) -> Result<()> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use windows::Win32::UI::Shell::RemoveWindowSubclass;

    struct Cover(HWND);
    impl Drop for Cover {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    struct Order {
        floating: HWND,
        cover: HWND,
        first: HWND,
    }
    unsafe extern "system" fn first_fixture(window: HWND, data: LPARAM) -> windows::core::BOOL {
        let order = unsafe { &mut *(data.0 as *mut Order) };
        if window == order.floating || window == order.cover {
            order.first = window;
            return windows::core::BOOL(0);
        }
        windows::core::BOOL(1)
    }

    let backend = i_slint_backend_winit::Backend::builder()
        .with_renderer_name(crate::ui::RENDERER)
        .with_window_attributes_hook(|attributes| {
            use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_active(false).with_skip_taskbar(true)
        })
        .build()?;
    slint::platform::set_platform(Box::new(backend))?;
    let ui = crate::VoiceWindow::new()?;
    let presentation = crate::ui::Presentation::new(&ui);
    let weak = ui.as_weak();
    let mut floating = crate::ui::FloatingState::default();
    ui.window().on_winit_window_event(move |_, event| {
        if let Some(ui) = weak.upgrade() {
            floating.event(&ui, event);
        }
        i_slint_backend_winit::EventResult::Propagate
    });
    let foreground = unsafe { GetForegroundWindow() };
    presentation.show_ball()?;
    // No owner: an owned popup would inherit its topmost owner's Z-order band.
    let cover = Cover(unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            w!("STATIC"),
            w!("Local Voice ordinary-window fixture"),
            WS_POPUP,
            0,
            0,
            64,
            64,
            None,
            None,
            Some(GetModuleHandleW(None)?.into()),
            None,
        )?
    });
    ensure!(
        unsafe { GetForegroundWindow() } == foreground,
        "Fixture startup stole focus"
    );
    let output = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&output)?;
    let cases = [
        "startup-ball",
        "panel",
        "collapse",
        "hide-show",
        "display",
        "resume",
        "demotion-blocked",
        "guard-removed-ablation",
    ];
    let step = Cell::new(0usize);
    let reports = RefCell::new(Vec::new());
    let failure = Rc::new(RefCell::new(None));
    let error = failure.clone();
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(350), move || {
        let checked = (|| -> Result<()> {
            let ui = weak.upgrade().context("Topmost fixture closed")?;
            let mut hwnd = HWND::default();
            ui.window().with_winit_window(|window| {
                if let Ok(handle) = window.window_handle() {
                    if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                        hwnd = HWND(handle.hwnd.get() as *mut _);
                    }
                }
            });
            unsafe {
                for window in [hwnd, cover.0] {
                    let mut pid = 0;
                    GetWindowThreadProcessId(window, Some(&mut pid));
                    ensure!(pid == windows::Win32::System::Threading::GetCurrentProcessId(), "Only manipulate owned fixtures");
                }
            }
            let index = step.get() / 2;
            let foreground = unsafe { GetForegroundWindow() };
            ensure!(foreground != hwnd && foreground != cover.0, "Fixture took foreground focus");
            if step.get() % 2 == 0 {
                match index {
                    0 => ensure!(ui.get_compact(), "Startup must show the ball"),
                    1 => presentation.show_panel()?,
                    2 => presentation.show_ball()?,
                    3 => { presentation.hide()?; presentation.show_ball()?; }
                    4 => unsafe { SendMessageW(hwnd, WM_DISPLAYCHANGE, Some(WPARAM(32)), Some(LPARAM(0))); },
                    5 => unsafe { SendMessageW(hwnd, WM_POWERBROADCAST, Some(WPARAM(PBT_APMRESUMEAUTOMATIC as usize)), Some(LPARAM(0))); },
                    6 | 7 => unsafe {
                        if index == 7 {
                            ensure!(RemoveWindowSubclass(hwnd, Some(crate::native::no_activate), 1).as_bool(), "Remove the guard only on the owned fixture");
                        }
                        SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)?;
                    },
                    _ => unreachable!(),
                }
            } else {
                let row = unsafe {
                    let mut rect = RECT::default();
                    GetWindowRect(hwnd, &mut rect)?;
                    // Move the ordinary window directly over the floating fixture,
                    // then raise it without activating either window.
                    SetWindowPos(cover.0, Some(HWND_TOP), rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, SWP_SHOWWINDOW | SWP_NOACTIVATE)?;
                    let mut order = Order { floating: hwnd, cover: cover.0, first: HWND::default() };
                    let _ = EnumWindows(Some(first_fixture), LPARAM(&mut order as *mut _ as isize));
                    ensure!(!order.first.is_invalid(), "Fixture absent from desktop Z order");
                    let topmost = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
                    let cover_topmost = GetWindowLongPtrW(cover.0, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
                    let above_cover = order.first == hwnd;
                    let visible = IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool();
                    let foreground_unchanged = GetForegroundWindow() == foreground;
                    let ablation = index == 7;
                    let passed = visible && !cover_topmost && foreground_unchanged
                        && if ablation { !topmost && !above_cover } else { topmost && above_cover };
                    if ablation {
                        crate::native::floating(hwnd);
                    }
                    serde_json::json!({
                        "case": cases[index], "passed": passed, "visible": visible,
                        "topmost": topmost, "above_ordinary_cover": above_cover,
                        "cover_topmost": cover_topmost, "foreground_unchanged": foreground_unchanged,
                        "ablation": ablation,
                    })
                };
                reports.borrow_mut().push(row);
                let all_passed = reports.borrow().iter().all(|row| row["passed"] == true);
                std::fs::write(output.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({
                    "all_passed": all_passed, "complete": index + 1 == cases.len(), "cases": *reports.borrow(),
                    "note": "Owned fixtures only; ordinary overlapping window raised without activation. Display/resume messages are synthetic. Final ablation removes the floating guard before demotion, then restores it. No services, audio, mouse, keyboard, or user-window changes."
                }))?)?;
                ensure!(all_passed, "Topmost regression; see result.json");
                if index + 1 == cases.len() {
                    slint::quit_event_loop()?;
                }
            }
            ensure!(unsafe { GetForegroundWindow() } == foreground, "Topmost check stole foreground focus");
            step.set(step.get() + 1);
            Ok(())
        })();
        if let Err(failure) = checked {
            let message = format!("{failure:#}");
            let _ = std::fs::write(output.join("error.txt"), &message);
            *error.borrow_mut() = Some(message);
            let _ = slint::quit_event_loop();
        }
    });
    slint::run_event_loop_until_quit()?;
    if let Some(error) = failure.borrow_mut().take() {
        bail!(error);
    }
    Ok(())
}

/// Render product states with fixed text only. This entry point bypasses the
/// controller, services, microphone, hotkeys, clipboard, and external input.
pub fn dictation_ui_check(directory: &str) -> Result<()> {
    use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    fn snapshot(window: &slint::Window, path: &std::path::Path) -> Result<serde_json::Value> {
        let frame = window.take_snapshot()?;
        let mut encoder =
            png::Encoder::new(std::fs::File::create(path)?, frame.width(), frame.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(frame.as_bytes())?;
        let logical = window.size().to_logical(window.scale_factor());
        Ok(serde_json::json!({
            "file": path.file_name().and_then(|name| name.to_str()),
            "physical_size": [frame.width(), frame.height()],
            "logical_size": [logical.width, logical.height],
            "scale": window.scale_factor(),
        }))
    }

    let backend = i_slint_backend_winit::Backend::builder()
        .with_renderer_name(crate::ui::RENDERER)
        .with_window_attributes_hook(|attributes| {
            use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_active(false).with_skip_taskbar(true)
        })
        .build()?;
    slint::platform::set_platform(Box::new(backend))?;
    let ui = crate::VoiceWindow::new()?;
    let settings = crate::SettingsWindow::new()?;
    let result = crate::ResultWindow::new()?;
    ui.set_animation_enabled(false);
    settings.set_asr_modes(ModelRc::new(VecModel::from(
        crate::config::ASR_MODES
            .iter()
            .map(|(_, label)| SharedString::from(*label))
            .collect::<Vec<_>>(),
    )));
    settings.set_asr_mode_index(2);
    settings.set_microphones(ModelRc::new(VecModel::from(vec![SharedString::from(
        "系统默认麦克风",
    )])));
    settings.set_record_key("Ctrl+Alt+Space".into());
    settings.set_continuous_dictation(true);
    settings.set_terms_text("".into());
    settings.set_app_version("本地试用".into());
    settings.set_text_installed(true);
    settings.set_text_ready(false);
    settings
        .set_text_status("模型已安装，当前未占用模型内存；下次整理自动加载，也可提前预加载".into());
    settings.set_service_status("Qwen ASR 1.7B · 服务已就绪（固定界面示例）".into());
    let original = "嗯，我觉得这个 attention 算子，主要是那个 memory bandwidth 的问题。不是计算量不够，是访存。然后可以先试一下 fusion，不过这个还得测。\n\n版本先用 2.1，不，还是 2.2。KV cache 保留 BF16，不要改成 FP16。这里没有确定结论，需要比较峰值显存和 p95 延迟。".repeat(8);
    let notes = "我认为这个 attention 算子主要存在 memory bandwidth 问题。不是计算量不足，而是访存问题。可以先尝试 fusion，但还需要测试。\n\n版本从 2.1 改为 2.2。KV cache 保留 BF16，不要改成 FP16。目前没有确定结论，需要比较峰值显存和 p95 延迟。\n\n".repeat(8);
    result.set_original(original.clone().into());
    result.set_result(notes.clone().into());
    result.set_output_mode("notes".into());
    result.set_retryable(true);
    result.set_notice("笔记正文已整理，原始转写仍可查看和复制".into());
    let output = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&output)?;
    let weak_ui = ui.as_weak();
    let weak_settings = settings.as_weak();
    let weak_result = result.as_weak();
    let cases = [
        "panel-raw",
        "panel-clean",
        "panel-notes-busy",
        "panel-pending",
        "settings",
        "settings-bottom",
        "settings-small",
        "settings-small-bottom",
        "result",
        "result-small",
        "result-small-bottom",
        "result-retry",
    ];
    let step = Cell::new(0usize);
    let reports = RefCell::new(Vec::new());
    let failure = Rc::new(RefCell::new(None));
    let error = failure.clone();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(350), move || {
        let checked = (|| -> Result<()> {
            let ui = weak_ui.upgrade().context("Panel fixture closed")?;
            let settings = weak_settings.upgrade().context("Settings fixture closed")?;
            let result = weak_result.upgrade().context("Result fixture closed")?;
            let index = step.get() / 2;
            let name = cases[index];
            let window = if index < 4 { ui.window() } else if index < 8 { settings.window() } else { result.window() };
            if step.get() % 2 == 0 {
                let foreground = unsafe { GetForegroundWindow() };
                ui.hide()?;
                settings.hide()?;
                result.hide()?;
                if index < 4 {
                    ui.set_compact(false);
                    ui.set_output_mode(if index == 0 { "raw" } else if index == 1 { "clean" } else { "notes" }.into());
                    ui.set_processing(index == 2);
                    ui.set_has_pending(index == 3);
                    ui.set_status(if index == 2 { "正在整理完整笔记…" } else if index == 3 { "输入目标已变化，结果保留在应用内" } else { "准备就绪 · 点击开始说话" }.into());
                    window.set_size(slint::LogicalSize::new(380., if index == 3 { 202. } else { 166. }));
                } else if index < 8 {
                    window.set_size(slint::LogicalSize::new(if index < 6 { 560. } else { 420. }, if index < 6 { 640. } else { 400. }));
                    settings.set_show_note_preview(index >= 6);
                    settings.set_content_scroll_y(if index % 2 == 1 { -settings.get_content_scroll_range() } else { 0. });
                } else {
                    window.set_size(slint::LogicalSize::new(if index == 8 { 820. } else { 420. }, if index == 8 { 780. } else { 400. }));
                    result.set_content_scroll_y(if index == 10 { -result.get_content_scroll_range() } else { 0. });
                    if index == 11 {
                        result.set_result(original.clone().into());
                        result.set_retryable(true);
                        result.set_notice("笔记整理未完成，本次使用原始转写。准备模型后可重新整理。".into());
                    }
                }
                if index >= 8 {
                    crate::ui::show_result(window, false)?;
                } else {
                    window.show()?;
                }
                crate::ui::mark_complete(window);
                window.request_redraw();
                ensure!(unsafe { GetForegroundWindow() } == foreground, "UI snapshot fixture changed foreground focus");
            } else {
                ensure!(result.get_original().as_str() == original, "Original text was truncated");
                if index < 11 {
                    ensure!(result.get_result().as_str() == notes, "Processed text was truncated");
                }
                let mut report = snapshot(window, &output.join(format!("{name}.png")))?;
                report["case"] = name.into();
                if index >= 4 && index < 8 {
                    report["show_note_preview"] = settings.get_show_note_preview().into();
                    report["scroll_range"] = settings.get_content_scroll_range().into();
                    report["scroll_y"] = settings.get_content_scroll_y().into();
                    ensure!(settings.get_content_scroll_range() > 0., "Settings content must be scrollable");
                } else if index >= 8 {
                    report["scroll_range"] = result.get_content_scroll_range().into();
                    report["scroll_y"] = result.get_content_scroll_y().into();
                }
                reports.borrow_mut().push(report);
                if index + 1 == cases.len() {
                    std::fs::write(output.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({
                        "passed": true,
                        "cases": *reports.borrow(),
                        "original_characters": original.chars().count(),
                        "result_characters": notes.chars().count(),
                        "note": "Fixed fixture text only. No controller, audio, HTTP, hotkeys, clipboard changes, user input, or foreground activation. Screenshots require visual review.",
                    }))?)?;
                    slint::quit_event_loop()?;
                }
            }
            step.set(step.get() + 1);
            Ok(())
        })();
        if let Err(failure) = checked {
            let message = format!("{failure:#}");
            let _ = std::fs::write(output.join("error.txt"), &message);
            *error.borrow_mut() = Some(message);
            let _ = slint::quit_event_loop();
        }
    });
    slint::run_event_loop_until_quit()?;
    if let Some(error) = failure.borrow_mut().take() {
        bail!(error);
    }
    Ok(())
}

/// Each model diagnostic owns its native models and always closes them. It never
/// attaches to, stops, or sends fixture content to a daily client's model server.
struct NativeCheck;
impl NativeCheck {
    fn start(mode: &str) -> Result<crate::config::Config> {
        let config = crate::config::Config {
            asr_mode: mode.into(),
            ..crate::config::Config::default()
        };
        crate::runtime::apply(crate::runtime::Operation::Start(config.clone()), || false)?;
        crate::runtime::ready(&config)?;
        Ok(config)
    }
}
impl Drop for NativeCheck {
    fn drop(&mut self) {
        crate::runtime::native_stop();
    }
}

/// Exercise model switching and repeated Start without reading saved settings.
pub fn asr_models_check(audio: &str, output: &str) -> Result<()> {
    native_models_check(audio, output, false)
}

/// Native ASR plus Agentic editing, using only a prerecorded fixture. No Python,
/// microphone, clipboard, keyboard registration, or user app interaction.
pub fn native_smoke(audio: &str, output: &str) -> Result<()> {
    native_models_check(audio, output, true)
}

fn native_models_check(audio: &str, output: &str, edit: bool) -> Result<()> {
    let _cleanup = NativeCheck;
    let (samples, rate) = crate::service::decode_wav(&std::fs::read(audio)?)?;
    ensure!(!samples.is_empty(), "Empty test fixture");
    let mut reports = Vec::new();
    for (index, mode) in ["sensevoice-cpu", "qwen-llama-0.6b", "sensevoice-cpu"]
        .into_iter()
        .enumerate()
    {
        let startup = Instant::now();
        let mut config = NativeCheck::start(mode)?;
        let startup_seconds = startup.elapsed().as_secs_f64();
        // An unchanged model remains usable when Start is requested twice.
        crate::runtime::apply(crate::runtime::Operation::Start(config.clone()), || false)?;
        let transcript =
            crate::service::transcribe_recording(&config, &samples, rate, || false, |_, _| {})?;
        ensure!(
            !transcript.text.trim().is_empty(),
            "Empty transcription from {mode}"
        );
        let mut report = serde_json::json!({
            "mode":mode, "startup_seconds":startup_seconds,
            "original":transcript.text, "recognition_seconds":transcript.request_seconds,
            "sample_rate":rate, "audio_seconds":samples.len() as f64 / rate as f64,
        });
        if edit && index < 2 {
            ensure!(
                crate::service::text_status(&config)?.installed,
                "Prepare the Agentic model in Settings before the native smoke check"
            );
            config.output_mode = "clean".into();
            let result = crate::service::process_transcript(&config, transcript, false, || false)?;
            ensure!(
                result.warning.is_none(),
                "Editing fell back: {:?}",
                result.warning
            );
            ensure!(!result.text.trim().is_empty(), "Empty edited body");
            ensure!(
                !result.preview_only,
                "Daily input unexpectedly requires a preview"
            );
            report["edited"] = result.text.into();
            report["editing_seconds"] = result.processing_seconds.into();
        }
        reports.push(report);
    }
    crate::runtime::apply(crate::runtime::Operation::Stop, || false)?;
    ensure!(
        crate::runtime::ready(&crate::config::Config::default()).is_err(),
        "ASR stayed available after Stop"
    );
    ensure!(
        !crate::service::text_status(&crate::config::Config::default())?.ready,
        "Editor stayed ready after Stop"
    );
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&serde_json::json!({
            "passed":true, "runtime":"native", "process_id":std::process::id(),
            "cases":reports, "stopped":true,
            "note":"Owned SenseVoice/Qwen 0.6B models only. The daily Qwen 1.7B process, saved settings, microphone, clipboard, and foreground apps are untouched. Text quality requires reading the complete bodies."
        }))?,
    )?;
    Ok(())
}

pub fn manual_asr(input: &str, output: &str) -> Result<()> {
    let _cleanup = NativeCheck;
    let (original, source_rate) = crate::service::decode_wav(&std::fs::read(input)?)?;
    ensure!(!original.is_empty(), "Empty fixture");
    let original = crate::vad::resample_mono(&original, source_rate, 48000)?;
    let samples: Vec<f32> = original.iter().copied().cycle().take(48000 * 65).collect();
    let config = NativeCheck::start("sensevoice-cpu")?;
    let start = Instant::now();
    let mut requests = Vec::new();
    let result = crate::service::transcribe_recording(
        &config,
        &samples,
        48000,
        || false,
        |index, total| requests.push(serde_json::json!({"index":index,"total":total})),
    )?;
    ensure!(
        requests.len() >= 3 && !result.text.trim().is_empty(),
        "Long recording not recognized"
    );
    let mut cancelled_progress = false;
    ensure!(
        crate::service::transcribe_recording(
            &config,
            &samples,
            48000,
            || true,
            |_, _| cancelled_progress = true,
        )
        .is_err()
            && !cancelled_progress,
        "Cancelled recording was submitted"
    );
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&serde_json::json!({
            "runtime":"native", "mode":"sensevoice-cpu",
            "audio_seconds":65, "sample_rate":48000, "requests":requests,
            "single_combined_result":result.text, "cancel_before_recognition":true,
            "elapsed_seconds":start.elapsed().as_secs_f64(),
            "note":"Prerecorded fixture; no microphone capture or insertion into user apps."
        }))?,
    )?;
    Ok(())
}

pub fn asr_latency(input: &str, output: &str) -> Result<()> {
    let _cleanup = NativeCheck;
    let (samples, source_rate) = crate::service::decode_wav(&std::fs::read(input)?)?;
    ensure!(!samples.is_empty(), "Empty fixture");
    let config = NativeCheck::start("sensevoice-cpu")?;
    let mut reports = Vec::new();
    for copies in [1, 4] {
        for rate in [24000, 48000] {
            let wave = crate::vad::resample_mono(&samples.repeat(copies), source_rate, rate)?;
            let result =
                crate::service::transcribe_recording(&config, &wave, rate, || false, |_, _| {})?;
            ensure!(!result.text.trim().is_empty(), "Empty ASR result");
            reports.push(serde_json::json!({
                "sample_rate":rate, "audio_seconds":wave.len() as f64/rate as f64,
                "recognition_seconds":result.request_seconds, "text":result.text,
            }));
        }
    }
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&serde_json::json!({
            "runtime":"native", "mode":"sensevoice-cpu", "cases":reports,
            "note":"Known test WAV through native resampling and recognition; no microphone capture."
        }))?,
    )?;
    Ok(())
}

/// One fixed developer utterance, identical generation settings. This isolates
/// preparation from editing; it is neither an ASR benchmark nor a quality score.
pub fn text_latency(output: &str) -> Result<()> {
    let processor = crate::text::TextProcessor::new(crate::paths::root()?);
    let original = "嗯，先把 Unreal 里的 shader cache 清一下，然后重新跑 blueprint 的测试。AGV 的速度不是每秒十米，是每秒一点二米。这个 transition 现在只是初步验证，还没有确认稳定，明天再看 profiler 的结果。";
    let run = || -> Result<(String, f64)> {
        let started = Instant::now();
        let text = processor.process_with_ticket(processor.ticket(), original, "clean")?;
        Ok((text, started.elapsed().as_secs_f64()))
    };
    let result = (|| -> Result<serde_json::Value> {
        let (cold_body, cold_seconds) = run()?;
        processor.stop();
        let recording = processor.keep_warm();
        let preparation = Instant::now();
        processor.prepare_with_ticket(processor.ticket(), false)?;
        let preparation_seconds = preparation.elapsed().as_secs_f64();
        let (prepared_body, prepared_seconds) = run()?;
        drop(recording);
        processor.stop();
        ensure!(
            !processor.health().ready,
            "Editor was not released after Stop"
        );
        Ok(serde_json::json!({
            "original":original,
            "without_prewarm":{"body":cold_body,"seconds":cold_seconds},
            "prepared_during_recording":{"body":prepared_body,"preparation_seconds":preparation_seconds,"after_recording_seconds":prepared_seconds},
            "same_body":cold_body == prepared_body,
            "stopped":true,
            "note":"Fixed developer text; identical model/prompt/CPU settings. Preparation is complete before the second request, modelling a sufficiently long recording. Preparation cost is moved, not eliminated. No microphone, user text, ASR, clipboard, or running app is used."
        }))
    })();
    processor.shutdown();
    std::fs::write(output, serde_json::to_vec_pretty(&result?)?)?;
    Ok(())
}

pub fn service_check(output: &str) -> Result<()> {
    let _cleanup = NativeCheck;
    let config = NativeCheck::start("sensevoice-cpu")?;
    let fixture = crate::paths::root()?.join("tests/fixtures/ascend-01219.wav");
    let bytes = std::fs::read(&fixture)
        .with_context(|| format!("fixture missing: {}", fixture.display()))?;
    let transcript = crate::service::transcribe_timed(&config, bytes)?;
    ensure!(!transcript.text.trim().is_empty(), "Empty recognition");
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&serde_json::json!({
            "runtime":"native", "mode":"sensevoice-cpu", "asr_ready":true,
            "asr_text":transcript.text, "recognition_seconds":transcript.request_seconds,
            "note":"Owned native SenseVoice with prerecorded fixture. No microphone recording or external application interaction."
        }))?,
    )?;
    Ok(())
}
