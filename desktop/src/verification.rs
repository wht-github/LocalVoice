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
pub fn selection_check(expected_pid: u32, expected: &str, output: &str) -> Result<()> {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        ensure!(
            pid == expected_pid,
            "Foreground changed; selection not read"
        );
    }
    let started = Instant::now();
    let result = crate::native::Automation::new()?.selection();
    let report = match result {
        Ok(text) => {
            serde_json::json!({"matches":text.split_whitespace().collect::<Vec<_>>().join(" ")==expected,
                "characters":text.chars().count(),"expected_characters":expected.chars().count(),
                "whitespace_codepoints":text.chars().filter(|c| c.is_whitespace()).map(|c| c as u32).collect::<Vec<_>>(),
                "elapsed_ms":started.elapsed().as_millis(),"note":"Read selected range only; no clipboard changes or playback"})
        }
        Err(error) => serde_json::json!({"matches":false,"error":error.to_string()}),
    };
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    ensure!(
        report["matches"] == true,
        "Selected text did not match the known fixture"
    );
    Ok(())
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
        let selection = automation.selection().context("selection read")?;
        ensure!(
            selection == "测试选区 Hello world",
            "Selected text mismatch: {selection}"
        );
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
        crate::native::register_keys("Ctrl+Alt+Space", "Ctrl+Alt+R")
            .context("hotkey registration")?;
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
                    let expected = if is_settings { (450.,640.) } else { (380.,102.) };
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
                    let expected = if case.compact() { (64., 64.) } else if case.pending() { (380., 138.) } else { (380., 102.) };
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
                    ensure!((size.height - 138.).abs() < 1., "Pending result must expand panel height: {size:?}");
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

/// Exercise the same manager and HTTP path as settings Save without changing
/// settings or sending recognized text to another application.
pub fn asr_models_check(audio: &str, output: &str) -> Result<()> {
    use crate::runtime::{Operation, apply};
    let client = crate::service::client()?;
    ensure!(
        client.get("http://127.0.0.1:8001/health").send().is_err(),
        "Exit the daily client and release port 8001 before this check"
    );
    let result = (|| -> Result<Vec<serde_json::Value>> {
        let mut reports = Vec::new();
        for mode in [
            "sensevoice-cpu",
            "qwen-llama-0.6b",
            "qwen-llama-1.7b",
            "sensevoice-cpu",
        ] {
            let config = crate::config::Config {
                asr_mode: mode.into(),
                tts_enabled: false,
                ..crate::config::Config::default()
            };
            let startup = apply(Operation::Start(config.clone()), || false)?;
            crate::service::ready(&config)?;
            apply(Operation::Start(config.clone()), || false)?;
            let transcript =
                crate::service::transcribe_timed(&client, &config, std::fs::read(audio)?)?;
            ensure!(
                !transcript.text.is_empty(),
                "Empty transcription from {mode}"
            );
            let health: serde_json::Value = client
                .get(format!("{}/health", config.asr_url))
                .send()?
                .json()?;
            ensure!(health["mode"] == mode, "Wrong backend after switch");
            ensure!(
                health["device"]
                    == if mode == "sensevoice-cpu" {
                        "cpu"
                    } else {
                        "cuda:0"
                    },
                "Wrong device"
            );
            reports.push(serde_json::json!({"startup":startup,"health":health,
                "text":transcript.text,"request_seconds":transcript.request_seconds,
                "inference_seconds":transcript.inference_seconds}));
        }
        apply(Operation::Stop, || false)?;
        ensure!(
            client.get("http://127.0.0.1:8001/health").send().is_err(),
            "ASR survived Stop"
        );
        Ok(reports)
    })();
    crate::runtime::native_stop();
    std::fs::write(output, serde_json::to_vec_pretty(&result?)?)?;
    Ok(())
}

pub fn runtime_check(output: &str) -> Result<()> {
    use crate::runtime::{Operation, apply};
    let config = crate::config::Config::load()?;
    ensure!(
        crate::runtime::managed(&config),
        "Test requires the dedicated LocalVoice endpoints"
    );
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let online = |url: &str| {
        client
            .get(format!("{url}/health"))
            .send()
            .and_then(|r| r.error_for_status())
            .is_ok()
    };
    let asr_pid = || -> Result<String> {
        use std::os::windows::process::CommandExt;
        let output = std::process::Command::new("wsl.exe")
            .args([
                "-d",
                "LocalVoice",
                "-u",
                "root",
                "--",
                "systemctl",
                "show",
                "local-voice-asr",
                "-p",
                "MainPID",
                "--value",
            ])
            .env("WSL_UTF8", "1")
            .creation_flags(0x08000000)
            .output()?;
        ensure!(output.status.success(), "Could not inspect ASR process");
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let mut enabled = config.clone();
    enabled.tts_enabled = true;
    apply(Operation::Start(enabled.clone()), || false)?;
    let pid = asr_pid()?;
    let mut disabled = config.clone();
    disabled.tts_enabled = false;
    let result = (|| -> Result<serde_json::Value> {
        apply(Operation::Tts(disabled.clone()), || false)?;
        ensure!(!online(&config.tts_url), "TTS still running after disable");
        crate::service::ready(&disabled)?;
        ensure!(pid == asr_pid()?, "Disabling TTS restarted ASR");
        apply(Operation::Start(disabled.clone()), || false)?;
        ensure!(
            !online(&config.tts_url),
            "Start ignored disabled TTS preference"
        );
        ensure!(pid == asr_pid()?, "Start needlessly restarted ASR");
        apply(Operation::Stop, || false)?;
        ensure!(
            !online(&config.asr_url) && !online(&config.tts_url),
            "Stop all left a service running"
        );
        Ok(
            serde_json::json!({"tts_disabled_releases_process":true,"asr_process_preserved":true,
            "start_respects_tts_disabled":true,"stop_all_closes_both_ports":true}),
        )
    })();
    // Restore enabled services even when an assertion fails; never change user settings.
    let restored = apply(Operation::Start(enabled), || false);
    let mut report = result?;
    restored?;
    report["restart_both_ready"] = true.into();
    std::fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

pub fn long_tts(output: &str) -> Result<()> {
    let config = crate::config::Config::load()?;
    let client = crate::service::client()?;
    let paragraph = "这是一段用于验证长文本自动朗读的内容，我们会保留原有的语序，在完整的句子结束时分段处理；如果一个句子比较长，就优先寻找逗号和合适的词语边界，让中文和 English 文本都能够依次交给语音服务。\nDr. Smith checks version 3.14 at example.com, and the application reads each sentence in its original order without requiring manual splitting.\n\n";
    let text = paragraph.repeat(9);
    ensure!(
        text.chars().count() > 2000,
        "Fixture must exceed a single API request limit"
    );
    let parts = crate::text::segments(&text);
    ensure!(parts.concat() == text, "Segmentation changed the input");
    let started = Instant::now();
    let mut reports = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if part.trim().is_empty() {
            continue;
        }
        ensure!(
            part.chars().count() <= crate::text::CHUNK_CHARS,
            "Chunk too long"
        );
        let response = client
            .post(format!("{}/v1/audio/speech", config.tts_url))
            .json(&serde_json::json!({"input":part,"voice":config.voice,"speed":config.speed}))
            .send()?
            .error_for_status()?;
        let bytes = response.bytes()?;
        let mut reader = hound::WavReader::new(std::io::Cursor::new(&bytes))?;
        let duration = reader.duration();
        ensure!(
            duration > 0 && reader.spec().sample_rate == 24000,
            "Invalid WAV at chunk {}",
            index + 1
        );
        ensure!(
            reader
                .samples::<i16>()
                .any(|sample| sample.is_ok_and(|value| value != 0)),
            "Silent WAV"
        );
        reports.push(serde_json::json!({"segment":index+1,"characters":part.chars().count(),"audio_seconds":duration as f64 / 24000.}));
    }
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&serde_json::json!({
            "input_characters":text.chars().count(),"lossless":true,"segments":reports,
            "elapsed_seconds":started.elapsed().as_secs_f64(),
            "note":"Every segment synthesized and WAV validated; no microphone capture or speaker playback"
        }))?,
    )?;
    Ok(())
}

pub fn manual_asr(input: &str, output: &str) -> Result<()> {
    let mut reader = hound::WavReader::open(input)?;
    ensure!(
        reader.spec().channels == 1 && reader.spec().sample_rate == 24000,
        "Use 24 kHz mono fixture"
    );
    let original: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as f32 / 32768.))
        .collect::<std::result::Result<_, _>>()?;
    ensure!(!original.is_empty(), "Empty fixture");
    let samples: Vec<f32> = original
        .iter()
        .cycle()
        .take(24000 * 65)
        .flat_map(|v| [*v, *v])
        .collect();
    let client = crate::service::client()?;
    let config = crate::config::Config::load()?;
    let start = Instant::now();
    let mut requests = Vec::new();
    let result = crate::service::transcribe_recording(
        &client,
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
            &client,
            &config,
            &samples,
            48000,
            || true,
            |_, _| cancelled_progress = true
        )
        .is_err()
            && !cancelled_progress,
        "Cancelled recording was submitted"
    );
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&serde_json::json!({"audio_seconds":65,
        "sample_rate":48000,"requests":requests,"single_combined_result":result.text,
        "cancel_before_upload":true,"elapsed_seconds":start.elapsed().as_secs_f64(),
        "note":"Prerecorded fixture; no microphone capture and no insertion into user apps"}))?,
    )?;
    Ok(())
}

pub fn asr_latency(input: &str, output: &str) -> Result<()> {
    let config = crate::config::Config::load()?;
    let client = crate::service::client()?;
    let mut reader = hound::WavReader::open(input)?;
    ensure!(
        reader.spec().channels == 1 && reader.spec().sample_rate == 24000,
        "Use a 24 kHz mono test WAV"
    );
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as f32 / 32768.))
        .collect::<std::result::Result<_, _>>()?;
    let mut reports = Vec::new();
    for copies in [1, 4] {
        for rate in [24000, 48000] {
            // Repeated samples are a timing fixture for the microphone's 48 kHz
            // upload path, not an audio-quality resampler used by the application.
            let wave: Vec<f32> = samples
                .repeat(copies)
                .into_iter()
                .flat_map(|s| std::iter::repeat_n(s, rate as usize / 24000))
                .collect();
            ensure!(
                wave.len() < rate as usize * 30,
                "Fixture exceeds API duration limit"
            );
            let encoding = Instant::now();
            let wav = crate::audio::encode_wav(&wave, rate)?;
            let encode_ms = encoding.elapsed().as_millis();
            let result = crate::service::transcribe_timed(&client, &config, wav)?;
            ensure!(!result.text.trim().is_empty(), "Empty ASR result");
            reports.push(serde_json::json!({"sample_rate":rate,"audio_seconds":wave.len() as f64 / rate as f64,"encode_ms":encode_ms,"http_seconds":result.request_seconds,"server_seconds":result.inference_seconds}));
        }
    }
    std::fs::write(
        output,
        serde_json::to_vec_pretty(
            &serde_json::json!({"cases":reports,"note":"Known test WAV replay through client encoding and HTTP, no microphone capture"}),
        )?,
    )?;
    Ok(())
}

pub fn services(output: &str) -> Result<()> {
    let config = crate::config::Config::load()?;
    crate::service::ready(&config)?;
    let client = crate::service::client()?;
    let response=client.post(format!("{}/v1/audio/speech",config.tts_url)).json(&serde_json::json!({"input":"你好，这是 Windows 客户端测试。Hello, local voice.","voice":config.voice})).send()?.error_for_status()?;
    let bytes = response.bytes()?.to_vec();
    let reader = hound::WavReader::new(std::io::Cursor::new(&bytes))?;
    ensure!(
        reader.duration() > 0 && reader.spec().sample_rate == 24000,
        "Invalid TTS WAV"
    );
    let seconds = reader.duration() as f64 / 24000.;
    let text = crate::service::transcribe(&client, &config, bytes)?;
    if text.trim().is_empty() {
        bail!("Empty recognition");
    }
    std::fs::write(
        output,
        serde_json::to_vec_pretty(
            &serde_json::json!({"asr_ready":true,"tts_ready":true,"tts_audio_seconds":seconds,"asr_text":text,"microphones":crate::audio::microphones(),"note":"No microphone recording and no speaker playback performed by this check"}),
        )?,
    )?;
    Ok(())
}
