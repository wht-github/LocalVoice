use super::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::{WinitWindowAccessor, winit};
use slint::{ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::{cell::RefCell, rc::Rc};
use tray_icon::{
    Icon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};
use windows::Win32::Foundation::HWND;

#[derive(Default)]
struct PointerState {
    position: slint::LogicalPosition,
    pressed: bool,
}
impl PointerState {
    fn event(
        &mut self,
        window: &slint::Window,
        event: &winit::event::WindowEvent,
    ) -> i_slint_backend_winit::EventResult {
        use i_slint_backend_winit::EventResult::{PreventDefault, Propagate};
        use slint::platform::WindowEvent as SlintEvent;
        use winit::event::{ElementState, MouseButton, WindowEvent};
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.position = slint::LogicalPosition::new(
                    position.x as f32 / window.scale_factor(),
                    position.y as f32 / window.scale_factor(),
                );
                Propagate
            }
            WindowEvent::MouseInput { state, button, .. } => {
                // Slint 1.17's winit event loop shares cursor_pos across windows.
                // Winit can suppress an identical CursorMoved on re-entry, so
                // a click in the floating bar otherwise uses settings coordinates.
                let button = match button {
                    MouseButton::Left => slint::platform::PointerEventButton::Left,
                    MouseButton::Right => slint::platform::PointerEventButton::Right,
                    MouseButton::Middle => slint::platform::PointerEventButton::Middle,
                    _ => return Propagate,
                };
                self.pressed = *state == ElementState::Pressed;
                window.dispatch_event(SlintEvent::PointerMoved {
                    position: self.position,
                });
                window.dispatch_event(if self.pressed {
                    SlintEvent::PointerPressed {
                        position: self.position,
                        button,
                    }
                } else {
                    SlintEvent::PointerReleased {
                        position: self.position,
                        button,
                    }
                });
                PreventDefault
            }
            WindowEvent::CursorLeft { .. } if self.pressed => PreventDefault,
            _ => Propagate,
        }
    }
}

pub(crate) fn show_complete(window: &slint::Window) -> Result<()> {
    window.show()?;
    repaint_complete(window);
    Ok(())
}

fn show_settings(window: &slint::Window) -> Result<()> {
    show_complete(window)?;
    window.with_winit_window(|w| {
        w.set_minimized(false);
        if let Ok(handle) = w.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                native::activate_settings(HWND(h.hwnd.get() as *mut _));
            }
        }
        w.focus_window();
    });
    repaint_complete(window);
    Ok(())
}

fn repaint_complete(window: &slint::Window) {
    mark_complete(window);
    window.request_redraw();
}

pub(crate) fn mark_complete(window: &slint::Window) {
    // softbuffer's Windows backing store can be lost across hide/show while its
    // buffer age still says "reused". request_redraw alone only paints dirty items.
    let size = window.size().to_logical(window.scale_factor());
    let mut dirty = i_slint_core::partial_renderer::DirtyRegion::default();
    dirty.add_rect(i_slint_core::lengths::LogicalRect::from_size(
        i_slint_core::lengths::LogicalSize::new(size.width, size.height),
    ));
    i_slint_core::window::WindowInner::from_pub(window)
        .window_adapter()
        .renderer()
        .mark_dirty_region(dirty);
}

#[derive(Default)]
pub(crate) struct DisplayState {
    hwnd: isize,
    geometry: Option<(slint::PhysicalSize, f32, u64)>,
    changed: bool,
}

impl DisplayState {
    pub(crate) fn event(&mut self, window: &slint::Window, event: &winit::event::WindowEvent) {
        use winit::event::WindowEvent;
        window.with_winit_window(|w| {
            if let Ok(handle) = w.window_handle() {
                if let RawWindowHandle::Win32(h) = handle.as_raw() {
                    if self.hwnd != h.hwnd.get() {
                        self.hwnd = h.hwnd.get();
                        native::watch_display(HWND(self.hwnd as *mut _));
                        self.changed = true;
                    }
                }
            }
        });
        match event {
            WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::Resized(_)
            | WindowEvent::Occluded(false) => self.changed = true,
            WindowEvent::RedrawRequested => {
                // Resize/DPI callbacks precede Slint's own handling. Invalidate
                // here, after the new size/scale is installed, before rendering.
                let geometry = (
                    window.size(),
                    window.scale_factor(),
                    native::display_revision(),
                );
                if self.changed || self.geometry != Some(geometry) {
                    self.changed = false;
                    self.geometry = Some(geometry);
                    native::keep_in_work_area(HWND(self.hwnd as *mut _));
                    mark_complete(window);
                }
            }
            _ => {}
        }
    }
}

pub fn run() -> Result<()> {
    let config_result = Config::load();
    let config = config_result.as_ref().cloned().unwrap_or_default();
    let backend = i_slint_backend_winit::Backend::builder()
        .with_renderer_name("software")
        .with_window_attributes_hook(|attributes| {
            use winit::platform::windows::WindowAttributesExtWindows;
            // Slint supplies a temporary title here. Settings are explicitly
            // activated on open; the floating window must never steal focus.
            attributes.with_active(false).with_skip_taskbar(true)
        })
        .build()?;
    slint::platform::set_platform(Box::new(backend))?;
    let ui = VoiceWindow::new()?;
    ui.set_tts_enabled(config.tts_enabled);
    let settings = SettingsWindow::new()?;
    let mut settings_pointer = PointerState::default();
    let mut settings_display = DisplayState::default();
    settings
        .window()
        .on_winit_window_event(move |window, event| {
            settings_display.event(window, event);
            settings_pointer.event(window, event)
        });
    let shared = Arc::new(Mutex::new(config.clone()));
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        ui.on_record(move || {
            let _ = tx.send(Event::Record);
        });
    }
    {
        let tx = tx.clone();
        ui.on_read(move || {
            let _ = tx.send(Event::Read);
        });
    }
    {
        let tx = tx.clone();
        ui.on_cancel(move || {
            let _ = tx.send(Event::Cancel);
        });
    }
    {
        let tx = tx.clone();
        ui.on_copy(move || {
            let _ = tx.send(Event::Copy);
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_hide_window(move || {
            if let Some(ui) = weak.upgrade() {
                let _ = ui.hide();
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_drag(move || {
            if let Some(ui) = weak.upgrade() {
                ui.window().with_winit_window(|w| {
                    let _ = w.drag_window();
                });
            }
        });
    }
    ui.window().on_close_requested({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                let _ = ui.hide();
            }
            slint::CloseRequestResponse::KeepWindowShown
        }
    });
    settings.on_close_settings({
        let weak = settings.as_weak();
        move || {
            if let Some(s) = weak.upgrade() {
                let _ = s.hide();
            }
        }
    });
    settings.window().on_close_requested({
        let weak = settings.as_weak();
        move || {
            if let Some(s) = weak.upgrade() {
                let _ = s.hide();
            }
            slint::CloseRequestResponse::KeepWindowShown
        }
    });
    settings.on_start_services({
        let tx = tx.clone();
        move || {
            let _ = tx.send(Event::StartServices);
        }
    });
    settings.on_stop_services({
        let tx = tx.clone();
        move || {
            let _ = tx.send(Event::StopServices);
        }
    });
    let microphones = Rc::new(RefCell::new(Vec::<String>::new()));
    let voices = Rc::new(RefCell::new(Vec::<String>::new()));
    // One reusable timer avoids retaining a timer/callback cycle after each settings visit.
    let refresh = Rc::new(Timer::default());
    let open_settings: Rc<dyn Fn()> = Rc::new({
        let settings = settings.as_weak();
        let cfg = shared.clone();
        let mic = microphones.clone();
        let voice = voices.clone();
        let refresh = refresh.clone();
        move || {
            diagnostics::record("settings_requested", serde_json::json!({}));
            let Some(settings) = settings.upgrade() else {
                return;
            };
            let config = cfg.lock().unwrap().clone();
            let mut devices = vec!["系统默认麦克风".to_owned()];
            if !config.microphone.is_empty() {
                devices.push(config.microphone.clone());
            }
            settings.set_microphone_index(
                devices
                    .iter()
                    .position(|v| *v == config.microphone)
                    .unwrap_or(0) as i32,
            );
            settings.set_microphones(ModelRc::new(VecModel::from(
                devices.iter().map(SharedString::from).collect::<Vec<_>>(),
            )));
            *mic.borrow_mut() = devices;
            *voice.borrow_mut() = vec![config.voice.clone()];
            settings.set_voices(ModelRc::new(VecModel::from(vec![SharedString::from(
                &config.voice,
            )])));
            settings.set_voice_index(0);
            settings.set_record_key(config.record_key.clone().into());
            settings.set_read_key(config.read_key.clone().into());
            settings.set_asr_url(config.asr_url.clone().into());
            settings.set_asr_modes(ModelRc::new(VecModel::from(
                crate::config::ASR_MODES
                    .iter()
                    .map(|(_, label)| SharedString::from(*label))
                    .collect::<Vec<_>>(),
            )));
            settings.set_asr_mode_index(
                crate::config::ASR_MODES
                    .iter()
                    .position(|(mode, _)| *mode == config.asr_mode)
                    .unwrap_or(0) as i32,
            );
            settings.set_tts_url(config.tts_url.clone().into());
            settings.set_speed(config.speed);
            settings.set_continuous_dictation(config.continuous_dictation);
            settings.set_tts_enabled(config.tts_enabled);
            settings.set_auto_start_services(config.auto_start_services);
            settings.set_error("".into());
            if let Err(error) = show_settings(settings.window()) {
                settings.set_error(format!("设置窗口打开失败：{error}").into());
            }
            diagnostics::record(
                "settings_shown",
                serde_json::json!({"visible":settings.window().is_visible()}),
            );
            let weak = settings.as_weak();
            let (mic_tx, mic_rx) = mpsc::channel();
            thread::spawn(move || {
                let _ = mic_tx.send(audio::microphones());
            });
            let mic_list = mic.clone();
            let selected_mic = config.microphone.clone();
            let mut mic_done = false;
            let mut voice_done = false;
            let deadline = Instant::now() + Duration::from_secs(10);
            let (voice_tx, voice_rx) = mpsc::channel();
            let selected = config.voice.clone();
            thread::spawn(move || {
                let _ = voice_tx.send(service::voices(&config));
            });
            let list = voice.clone();
            let timer = Rc::downgrade(&refresh);
            refresh.start(TimerMode::Repeated, Duration::from_millis(100), move || {
                if let Ok(devices) = mic_rx.try_recv() {
                    mic_done = true;
                    if let Some(s) = weak.upgrade() {
                        let current = mic_list
                            .borrow()
                            .get(s.get_microphone_index() as usize)
                            .cloned()
                            .unwrap_or_else(|| selected_mic.clone());
                        let mut items = vec!["系统默认麦克风".to_owned()];
                        items.extend(devices);
                        s.set_microphone_index(
                            items.iter().position(|v| *v == current).unwrap_or(0) as i32,
                        );
                        s.set_microphones(ModelRc::new(VecModel::from(
                            items.iter().map(SharedString::from).collect::<Vec<_>>(),
                        )));
                        *mic_list.borrow_mut() = items;
                    }
                }
                if let Ok(result) = voice_rx.try_recv() {
                    voice_done = true;
                    if let (Ok(items), Some(settings)) = (result, weak.upgrade()) {
                        let labels: Vec<_> = items
                            .iter()
                            .map(|v| {
                                SharedString::from(if v.name.is_empty() { &v.id } else { &v.name })
                            })
                            .collect();
                        let ids: Vec<_> = items
                            .into_iter()
                            .map(|v| {
                                let _ = v.language;
                                v.id
                            })
                            .collect();
                        if !ids.is_empty() {
                            settings.set_voice_index(
                                ids.iter().position(|v| *v == selected).unwrap_or(0) as i32,
                            );
                            settings.set_voices(ModelRc::new(VecModel::from(labels)));
                            *list.borrow_mut() = ids;
                        }
                    }
                }
                if (mic_done && voice_done) || Instant::now() >= deadline {
                    if let Some(timer) = timer.upgrade() {
                        timer.stop();
                    }
                }
            });
        }
    });
    {
        let show = open_settings.clone();
        ui.on_settings(move || {
            // Finish dispatching the floating button's mouse release before a
            // second native window changes focus/capture during that dispatch.
            let show = show.clone();
            Timer::single_shot(Duration::from_millis(1), move || show());
        });
    }
    {
        let weak = settings.as_weak();
        let tx = tx.clone();
        let current = shared.clone();
        let mic = microphones.clone();
        let voice = voices.clone();
        settings.on_save(move || {
            if let Some(s) = weak.upgrade() {
                let mut cfg = current.lock().unwrap().clone();
                cfg.record_key = s.get_record_key().to_string();
                cfg.read_key = s.get_read_key().to_string();
                cfg.asr_url = s.get_asr_url().to_string();
                let Some((mode, _)) = crate::config::ASR_MODES.get(s.get_asr_mode_index() as usize)
                else {
                    s.set_error("请选择识别模型".into());
                    return;
                };
                cfg.asr_mode = (*mode).into();
                cfg.tts_url = s.get_tts_url().to_string();
                cfg.speed = s.get_speed();
                cfg.continuous_dictation = s.get_continuous_dictation();
                cfg.tts_enabled = s.get_tts_enabled();
                cfg.auto_start_services = s.get_auto_start_services();
                cfg.microphone = if s.get_microphone_index() == 0 {
                    String::new()
                } else {
                    mic.borrow()
                        .get(s.get_microphone_index() as usize)
                        .cloned()
                        .unwrap_or_default()
                };
                cfg.voice = voice
                    .borrow()
                    .get(s.get_voice_index() as usize)
                    .cloned()
                    .unwrap_or(cfg.voice);
                let _ = tx.send(Event::Configure(cfg));
            }
        });
    }
    let menu = Menu::new();
    let show = MenuItem::new("显示悬浮窗", true, None);
    let read = MenuItem::new("朗读剪贴板", true, None);
    let setup = MenuItem::new("设置", true, None);
    let quit = MenuItem::new("退出", true, None);
    menu.append_items(&[&show, &read, &setup, &quit])?;
    let mut rgba = vec![0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let i = (y * 32 + x) * 4;
            let wave = (x > 7 && x < 11 && y > 11 && y < 21)
                || (x > 14 && x < 18 && y > 6 && y < 26)
                || (x > 21 && x < 25 && y > 11 && y < 21);
            rgba[i..i + 4].copy_from_slice(if wave {
                &[162, 224, 180, 255]
            } else {
                &[23, 37, 34, 255]
            });
        }
    }
    let _tray = TrayIconBuilder::new()
        .with_tooltip("本地语音 · 说话输入 / 选区朗读")
        .with_icon(Icon::from_rgba(rgba, 32, 32)?)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()?;
    let tray_timer = Timer::default();
    let weak = ui.as_weak();
    let sender = tx.clone();
    let settings_action = open_settings.clone();
    tray_timer.start(TimerMode::Repeated, Duration::from_millis(80), move || {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *show.id() {
                if let Some(ui) = weak.upgrade() {
                    let _ = show_complete(ui.window());
                }
            } else if event.id == *read.id() {
                let _ = sender.send(Event::ClipboardRead);
            } else if event.id == *setup.id() {
                settings_action();
            } else if event.id == *quit.id() {
                let _ = sender.send(Event::Quit);
                let _ = slint::quit_event_loop();
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                if let Some(ui) = weak.upgrade() {
                    let _ = show_complete(ui.window());
                }
            }
        }
    });
    {
        let shared = shared.clone();
        let weak = ui.as_weak();
        let sw = settings.as_weak();
        let sender = tx.clone();
        thread::spawn(move || controller(config, shared, weak, sw, sender, rx));
    }
    let initialized = Rc::new(RefCell::new(None));
    let mut pointer = PointerState::default();
    let mut display = DisplayState::default();
    ui.window().on_winit_window_event(move |window, event| {
        display.event(window, event);
        window.with_winit_window(|w| {
            if let Ok(handle) = w.window_handle() {
                if let RawWindowHandle::Win32(h) = handle.as_raw() {
                    if *initialized.borrow() != Some(h.hwnd.get()) {
                        let first = initialized.borrow().is_none();
                        *initialized.borrow_mut() = Some(h.hwnd.get());
                        native::floating(HWND(h.hwnd.get() as *mut _));
                        if first {
                            if let Some(monitor) = w.current_monitor() {
                                let size = monitor.size();
                                let pos = monitor.position();
                                let window_size = w.outer_size();
                                w.set_outer_position(winit::dpi::PhysicalPosition::new(
                                    pos.x
                                        + (size.width as i32 - window_size.width as i32 - 24)
                                            .max(0),
                                    pos.y
                                        + (size.height as i32 - window_size.height as i32 - 110)
                                            .max(0),
                                ));
                            }
                        }
                    }
                }
            }
        });
        pointer.event(window, event)
    });
    show_complete(ui.window())?;
    let args: Vec<_> = std::env::args().collect();
    let snapshot_timer = Timer::default();
    let settings_snapshot = Rc::new(Timer::default());
    let restore_timer = Timer::default();
    if args.get(1).map(String::as_str) == Some("--restore-check") {
        verification::restore_check(
            &ui,
            &restore_timer,
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("report directory required"))?
                .clone(),
            args.iter().any(|arg| arg == "--unpatched"),
        );
    }
    if args.get(1).map(String::as_str) == Some("--snapshot") {
        let path = args
            .get(2)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("snapshot path required"))?;
        let weak = ui.as_weak();
        let sw = settings.as_weak();
        let settings_timer = settings_snapshot.clone();
        snapshot_timer.start(TimerMode::SingleShot, Duration::from_secs(2), move || {
            if let Some(ui) = weak.upgrade() {
                let save = |window: &slint::Window, path: &str| -> Result<()> {
                    let pixels = window.take_snapshot()?;
                    let file = std::fs::File::create(path)?;
                    let mut encoder = png::Encoder::new(
                        std::io::BufWriter::new(file),
                        pixels.width(),
                        pixels.height(),
                    );
                    encoder.set_color(png::ColorType::Rgba);
                    encoder.set_depth(png::BitDepth::Eight);
                    encoder
                        .write_header()?
                        .write_image_data(pixels.as_bytes())?;
                    Ok(())
                };
                let _ = save(ui.window(), &path);
                ui.invoke_settings();
                let sw = sw.clone();
                let path = path.clone();
                settings_timer.start(
                    TimerMode::SingleShot,
                    Duration::from_millis(700),
                    move || {
                        if let Some(settings) = sw.upgrade() {
                            let _ = save(settings.window(), &format!("{path}.settings.png"));
                        }
                        let _ = slint::quit_event_loop();
                    },
                );
            }
        });
    }
    if let Err(error) = config_result {
        ui.set_status(format!("设置读取失败，使用默认值：{error}").into());
    }
    slint::run_event_loop_until_quit()?;
    let _ = tx.send(Event::Quit);
    // The controller is a background thread. Finish process cleanup before main
    // returns, and prevent a queued service Start from racing with shutdown.
    runtime::shutdown();
    Ok(())
}
