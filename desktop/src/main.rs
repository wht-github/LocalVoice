#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod audio;
mod config;
mod diagnostics;
mod native;
mod runtime;
mod service;
mod text;
mod ui;
mod verification;
use anyhow::Result;
use config::Config;
use slint::ComponentHandle;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant},
};
slint::include_modules!();

enum Event {
    Record,
    Read,
    ClipboardRead,
    Cancel,
    Copy,
    Quit,
    Configure(Config),
    StartServices,
    StopServices,
    Runtime(bool, String),
    Health(std::result::Result<(), String>),
    CaptureStarted(u64),
    CaptureStopped(u64),
    CaptureError(u64, String),
    RecognitionProgress(u64, usize, usize),
    Audio(u64, Vec<f32>, u32, Instant),
    Level(u64, f32),
    Recognized(
        u64,
        std::result::Result<service::Transcript, String>,
        Instant,
    ),
    TtsStatus(u64, String),
    TtsDone(u64, std::result::Result<(), String>),
}
struct Job {
    id: u64,
    samples: Vec<f32>,
    rate: u32,
    config: Config,
    queued_at: Instant,
}
struct SpeechJob {
    id: u64,
    text: String,
    config: Config,
}
fn status(ui: &slint::Weak<VoiceWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = ui.upgrade_in_event_loop(move |ui| ui.set_status(text.into()));
}
fn state(ui: &slint::Weak<VoiceWindow>, recording: bool, speaking: bool, pending: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_recording(recording);
        ui.set_speaking(speaking);
        ui.set_has_pending(pending);
        if !recording {
            ui.set_level(0.);
        }
    });
}

fn controller(
    initial: Config,
    shared: Arc<Mutex<Config>>,
    ui: slint::Weak<VoiceWindow>,
    settings: slint::Weak<SettingsWindow>,
    events: Sender<Event>,
    receiver: mpsc::Receiver<Event>,
) {
    let automation = match native::Automation::new() {
        Ok(value) => value,
        Err(error) => {
            status(&ui, format!("Windows 辅助功能初始化失败：{error}"));
            return;
        }
    };
    let mut config = initial;
    let mut keys_ready = match native::register_keys(&config.record_key, &config.read_key) {
        Ok(()) => true,
        Err(error) => {
            status(&ui, error.to_string());
            false
        }
    };
    let generation = Arc::new(AtomicU64::new(1));
    let (jobs, job_rx) = mpsc::sync_channel::<Job>(2);
    let (speech_jobs, speech_rx) = mpsc::sync_channel::<SpeechJob>(1);
    {
        let out = events.clone();
        let shared_gen = generation.clone();
        thread::spawn(move || {
            while let Ok(job) = speech_rx.recv() {
                if job.id != shared_gen.load(Ordering::SeqCst) {
                    continue;
                }
                let result = service::speak(
                    job.config,
                    job.text,
                    job.id,
                    shared_gen.clone(),
                    |message| {
                        let _ = out.send(Event::TtsStatus(job.id, message.into()));
                    },
                );
                let _ = out.send(Event::TtsDone(job.id, result.map_err(|e| e.to_string())));
            }
        });
    }
    let replies = events.clone();
    let worker_gen = generation.clone();
    thread::spawn(move || {
        let client = service::client();
        while let Ok(job) = job_rx.recv() {
            if job.id != worker_gen.load(Ordering::SeqCst) {
                continue;
            }
            let result = (|| {
                let client = client
                    .as_ref()
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                let encoding = Instant::now();
                diagnostics::record(
                    "asr_request",
                    serde_json::json!({"generation":job.id,"audio_seconds":job.samples.len() as f64/job.rate as f64,"sample_rate":job.rate,"queue_ms":encoding.duration_since(job.queued_at).as_millis()}),
                );
                service::transcribe_recording(
                    client,
                    &job.config,
                    &job.samples,
                    job.rate,
                    || job.id != worker_gen.load(Ordering::SeqCst),
                    |index, total| {
                        let _ = replies.send(Event::RecognitionProgress(job.id, index, total));
                    },
                )
            })();
            let _ = replies.send(Event::Recognized(
                job.id,
                result.map_err(|e| e.to_string()),
                Instant::now(),
            ));
        }
    });
    let manager = runtime::Manager::new({
        let tx = events.clone();
        move |busy, message| {
            let _ = tx.send(Event::Runtime(busy, message));
        }
    });
    let diagnostic = std::env::args().nth(1).is_some();
    if config.auto_start_services && runtime::managed(&config) && !diagnostic {
        manager.send(runtime::Operation::Start(config.clone()));
    } else {
        let cfg = config.clone();
        let tx = events.clone();
        thread::spawn(move || {
            let _ = tx.send(Event::Health(
                service::ready(&cfg).map_err(|e| e.to_string()),
            ));
        });
    }
    let mut capture: Option<audio::Capture> = None;
    let mut target: Option<native::Target> = None;
    let mut recording = false;
    let mut continuous_session = false;
    let mut speaking = false;
    let mut pending = String::new();
    let mut inflight = 0usize;
    let mut stopping = false;
    let mut delivery: VecDeque<(u64, service::Transcript, Instant)> = VecDeque::new();
    loop {
        for key in native::poll_keys() {
            let _ = events.send(if key == 1 { Event::Record } else { Event::Read });
        }
        // Fast ASR can finish before the stop shortcut is released. Keep pumping
        // events so Cancel/Quit still work; never synthesize modifier key releases.
        delivery.retain(|(id, _, _)| *id == generation.load(Ordering::SeqCst));
        let ready = !delivery.is_empty() && !native::modifiers_down();
        let next = if ready {
            let (id, transcript, at) = delivery.pop_front().unwrap();
            Ok(Event::Recognized(id, Ok(transcript), at))
        } else {
            receiver.recv_timeout(Duration::from_millis(15))
        };
        let event = match next {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        let current = generation.load(Ordering::SeqCst);
        match event {
            Event::Quit => {
                if let Some(c) = capture.take() {
                    c.stop(false);
                }
                generation.fetch_add(1, Ordering::SeqCst);
                runtime::native_stop();
                break;
            }
            Event::Health(result) => {
                let message = if result.is_ok() {
                    "服务已就绪"
                } else {
                    "服务未就绪，可点击启动服务"
                };
                let _ =
                    settings.upgrade_in_event_loop(move |s| s.set_service_status(message.into()));
                if keys_ready && !recording && !speaking && inflight == 0 {
                    match result {
                        Ok(()) => status(&ui, format!("就绪 · {} 说话", config.record_key)),
                        Err(_) => status(&ui, "语音服务未就绪，请先启动 LocalVoice"),
                    }
                }
            }
            Event::Runtime(busy, message) => {
                if !recording && !speaking && inflight == 0 {
                    status(&ui, message.clone());
                }
                let _ = settings.upgrade_in_event_loop(move |s| {
                    s.set_services_busy(busy);
                    s.set_service_status(message.into());
                });
            }
            Event::StartServices => {
                manager.send(runtime::Operation::Start(config.clone()));
            }
            Event::StopServices => {
                stopping = false;
                if !runtime::managed(&config) {
                    status(&ui, "自定义地址请自行管理服务");
                    continue;
                }
                if let Some(c) = capture.take() {
                    c.stop(false);
                }
                generation.fetch_add(1, Ordering::SeqCst);
                recording = false;
                speaking = false;
                inflight = 0;
                manager.send(runtime::Operation::Stop);
            }
            Event::Record => {
                let args: Vec<_> = std::env::args().collect();
                if args.get(1).map(String::as_str) == Some("--target-click-check") {
                    // Explicit diagnostic mode: exercise the real button path but
                    // never start capture, write into another app, or play audio.
                    if let (Some(pid), Some(path)) =
                        (args.get(2).and_then(|s| s.parse::<u32>().ok()), args.get(3))
                    {
                        let result = automation
                            .inspect_focused_target(pid)
                            .unwrap_or_else(|e| serde_json::json!({"error":e.to_string()}));
                        let _ = std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap());
                    }
                    let _ = slint::quit_event_loop();
                    break;
                }
                if recording {
                    recording = false;
                    stopping = true;
                    // Snapshot immediately on manual stop, before capture teardown.
                    let destination = automation.target();
                    diagnostics::record(
                        "dictation_stop_target",
                        serde_json::json!({
                            "captured":destination.is_ok(),
                            "error":destination.as_ref().err().map(|e| e.to_string())
                        }),
                    );
                    target = if continuous_session {
                        None
                    } else {
                        destination.ok()
                    };
                    if let Some(c) = capture.take() {
                        c.stop(true);
                    }
                    status(&ui, "录音已结束，正在识别整段内容…");
                } else if inflight > 0 || stopping {
                    status(&ui, "请等待本次识别完成，或点击取消");
                } else {
                    let id = generation.fetch_add(1, Ordering::SeqCst) + 1;
                    speaking = false;
                    continuous_session = config.continuous_dictation;
                    target = None;
                    capture = Some(audio::Capture::start(config.clone(), id, events.clone()));
                    recording = true;
                    status(&ui, "正在打开麦克风…");
                }
            }
            Event::Read | Event::ClipboardRead => {
                let args: Vec<_> = std::env::args().collect();
                if args.get(1).map(String::as_str) == Some("--selection-click-check") {
                    if let (Some(pid), Some(expected), Some(path)) = (
                        args.get(2).and_then(|s| s.parse::<u32>().ok()),
                        args.get(3),
                        args.get(4),
                    ) {
                        let _ = verification::selection_check(pid, expected, path);
                    }
                    let _ = slint::quit_event_loop();
                    break;
                }
                if speaking {
                    generation.fetch_add(1, Ordering::SeqCst);
                    speaking = false;
                    status(&ui, "已停止朗读");
                } else if !config.tts_enabled {
                    status(&ui, "朗读已关闭，请在设置中启用并保存");
                } else {
                    let text = if matches!(event, Event::ClipboardRead) {
                        arboard::Clipboard::new()
                            .and_then(|mut c| c.get_text())
                            .map_err(anyhow::Error::from)
                    } else {
                        automation.selection()
                    };
                    match text {
                        Ok(text) if !text.trim().is_empty() && text.chars().count() <= 20000 => {
                            stopping = false;
                            if let Some(c) = capture.take() {
                                c.stop(false);
                            }
                            recording = false;
                            inflight = 0;
                            let id = generation.fetch_add(1, Ordering::SeqCst) + 1;
                            speaking = speech_jobs
                                .try_send(SpeechJob {
                                    id,
                                    text,
                                    config: config.clone(),
                                })
                                .is_ok();
                            status(
                                &ui,
                                if speaking {
                                    "正在合成语音…"
                                } else {
                                    "上一项朗读尚未退出，请稍后再试"
                                },
                            );
                        }
                        Ok(_) => status(&ui, "请选择 1–20000 字的文本"),
                        Err(error) => status(&ui, error.to_string()),
                    }
                }
            }
            Event::Cancel => {
                stopping = false;
                generation.fetch_add(1, Ordering::SeqCst);
                if let Some(c) = capture.take() {
                    c.stop(false);
                }
                recording = false;
                speaking = false;
                inflight = 0;
                status(&ui, "已取消未完成任务；已输入文字不回删");
            }
            Event::Copy => {
                if !pending.is_empty() {
                    match arboard::Clipboard::new().and_then(|mut c| c.set_text(pending.clone())) {
                        Ok(()) => {
                            pending.clear();
                            status(&ui, "结果已复制，可粘贴到目标输入框");
                        }
                        Err(error) => status(&ui, error.to_string()),
                    }
                }
            }
            Event::CaptureStarted(id) if id == current && recording => {
                status(
                    &ui,
                    if continuous_session {
                        "连续听写中 · 停顿后输入 · 再按结束"
                    } else {
                        "正在录音 · 选好输入框后，手动结束说话"
                    },
                );
            }
            Event::CaptureStopped(id) if id == current => {
                stopping = false;
                if inflight == 0 && !recording {
                    status(
                        &ui,
                        if pending.is_empty() {
                            "录音已结束"
                        } else {
                            "结果已保留，请复制后粘贴"
                        },
                    );
                }
            }
            Event::CaptureError(id, error) if id == current => {
                stopping = false;
                generation.fetch_add(1, Ordering::SeqCst);
                inflight = 0;
                if let Some(c) = capture.take() {
                    c.stop(false);
                }
                recording = false;
                status(&ui, format!("麦克风不可用：{error}"));
            }
            Event::Level(id, level) if id == current && recording => {
                let _ = ui.upgrade_in_event_loop(move |ui| ui.set_level(level));
            }
            Event::RecognitionProgress(id, index, total) if id == current => {
                status(
                    &ui,
                    if continuous_session && recording {
                        "连续听写中 · 正在识别…".into()
                    } else {
                        format!("正在识别整段录音 · {index}/{total} · 完成后一次输入")
                    },
                );
            }
            Event::Audio(id, samples, rate, queued_at) if id == current => {
                // Include deferred results in the bound, not just HTTP jobs.
                if inflight < 3
                    && jobs
                        .try_send(Job {
                            id,
                            samples,
                            rate,
                            config: config.clone(),
                            queued_at,
                        })
                        .is_ok()
                {
                    inflight += 1;
                    status(
                        &ui,
                        if recording {
                            "正在听 · 识别上一段…"
                        } else {
                            "正在识别…"
                        },
                    );
                } else {
                    if let Some(c) = capture.take() {
                        c.stop(false);
                    }
                    recording = false;
                    status(&ui, "识别积压，已暂停录音；最新片段未提交，请重说");
                }
            }
            Event::Recognized(id, result, returned_at) if id == current => {
                if result.is_ok() && (native::modifiers_down() || (!ready && !delivery.is_empty()))
                {
                    let item = (id, result.unwrap(), returned_at);
                    if ready {
                        delivery.push_front(item);
                    } else {
                        delivery.push_back(item);
                    }
                    status(&ui, "识别完成，松开快捷键后输入…");
                    continue;
                }
                inflight = inflight.saturating_sub(1);
                match result {
                    Ok(transcript) if !transcript.text.trim().is_empty() => {
                        let text = transcript.text;
                        let insert_started = Instant::now();
                        // Continuous input follows the active keyboard destination
                        // when each result is ready, including after shortcut release.
                        let inserted = if continuous_session {
                            Some(
                                automation
                                    .target()
                                    .and_then(|t| automation.insert(&t, &text)),
                            )
                        } else {
                            target.as_ref().map(|t| automation.insert(t, &text))
                        };
                        diagnostics::record(
                            "asr_result",
                            serde_json::json!({"generation":id,"http_seconds":transcript.request_seconds,"server_seconds":transcript.inference_seconds,"result_dispatch_ms":insert_started.duration_since(returned_at).as_millis(),"target_check_and_insert_ms":insert_started.elapsed().as_millis(),"inserted":matches!(&inserted,Some(Ok(()))),"delivery_error":inserted.as_ref().and_then(|r| r.as_ref().err()).map(|e| e.to_string())}),
                        );
                        match inserted {
                            Some(Ok(())) => status(
                                &ui,
                                if recording && continuous_session {
                                    "已输入 · 连续听写中"
                                } else if recording {
                                    "已输入 · 继续说话"
                                } else {
                                    "已输入到目标应用"
                                },
                            ),
                            failure => {
                                if !pending.is_empty() {
                                    pending.push('\n');
                                }
                                pending.push_str(&text);
                                target = None;
                                // A temporarily unavailable input must not end
                                // continuous listening. Keep failed text for Copy.
                                if !continuous_session || pending.len() > 256 * 1024 {
                                    if let Some(c) = capture.take() {
                                        c.stop(false);
                                    }
                                    recording = false;
                                }
                                status(
                                    &ui,
                                    match failure {
                                        _ if continuous_session && pending.len() > 256 * 1024 => {
                                            "待复制文字积累过多，已暂停听写，请先复制保存".into()
                                        }
                                        _ if continuous_session && recording => {
                                            "此处未能输入，结果已保留 · 连续听写中".into()
                                        }
                                        Some(Err(e)) => e.to_string(),
                                        _ => "结果已保留，请复制后粘贴".into(),
                                    },
                                );
                            }
                        }
                    }
                    Ok(_) => status(
                        &ui,
                        if continuous_session && recording {
                            "连续听写中 · 等待声音…"
                        } else {
                            "未识别到清晰语音"
                        },
                    ),
                    Err(error) => {
                        if !continuous_session {
                            if let Some(c) = capture.take() {
                                c.stop(false);
                            }
                            recording = false;
                        }
                        status(
                            &ui,
                            if continuous_session && recording {
                                format!("本段识别失败，请重说 · 连续听写中：{error}")
                            } else {
                                format!("识别失败，请重说：{error}")
                            },
                        );
                    }
                }
            }
            Event::TtsStatus(id, message) if id == current => status(&ui, message),
            Event::TtsDone(id, result) if id == current => {
                speaking = false;
                status(
                    &ui,
                    match result {
                        Ok(()) => "朗读结束".into(),
                        Err(e) => format!("朗读失败：{e}"),
                    },
                );
            }
            Event::Configure(next) => {
                let result = (|| -> Result<()> {
                    next.validate()?;
                    native::hotkey(&next.record_key)?;
                    native::hotkey(&next.read_key)?;
                    native::unregister_keys();
                    if let Err(error) = native::register_keys(&next.record_key, &next.read_key) {
                        let _ = native::register_keys(&config.record_key, &config.read_key);
                        return Err(error);
                    }
                    if let Err(error) = next.save() {
                        native::unregister_keys();
                        let _ = native::register_keys(&config.record_key, &config.read_key);
                        return Err(error);
                    }
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        keys_ready = true;
                        let tts_changed = config.tts_enabled != next.tts_enabled;
                        config = next;
                        *shared.lock().unwrap() = config.clone();
                        let enabled = config.tts_enabled;
                        let _ = ui.upgrade_in_event_loop(move |u| u.set_tts_enabled(enabled));
                        if tts_changed {
                            if speaking && !config.tts_enabled {
                                generation.fetch_add(1, Ordering::SeqCst);
                                speaking = false;
                            }
                            manager.send(runtime::Operation::Tts(config.clone()));
                        }
                        status(&ui, "设置已保存；新任务使用新设置");
                        let _ = settings.upgrade_in_event_loop(|s| {
                            s.set_error("".into());
                            let _ = s.hide();
                        });
                    }
                    Err(error) => {
                        let _ = settings
                            .upgrade_in_event_loop(move |s| s.set_error(error.to_string().into()));
                    }
                }
            }
            _ => {}
        }
        state(&ui, recording, speaking, !pending.is_empty());
    }
    native::unregister_keys();
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--manual-asr-check") => verification::manual_asr(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("input WAV required"))?,
            args.get(3)
                .ok_or_else(|| anyhow::anyhow!("report path required"))?,
        ),
        Some("--runtime-check") => verification::runtime_check(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("report path required"))?,
        ),
        Some("--console-fixture") => {
            println!("LocalVoice selection fixture");
            println!("Waiting for test input. This fixture does not execute commands.");
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            Ok(())
        }
        Some("--test-fixture") => verification::fixture(),
        Some("--native-check") => verification::native_check(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("fixture pid required"))?
                .parse()?,
            args.get(3)
                .ok_or_else(|| anyhow::anyhow!("output path required"))?,
        ),
        Some("--focus-check") => {
            let pid = args
                .get(2)
                .ok_or_else(|| anyhow::anyhow!("foreground pid required"))?
                .parse()?;
            let output = args
                .get(3)
                .ok_or_else(|| anyhow::anyhow!("report path required"))?;
            let result = native::Automation::new()?.inspect_focused_target(pid)?;
            std::fs::write(output, serde_json::to_vec_pretty(&result)?)?;
            Ok(())
        }
        Some("--selection-check") => verification::selection_check(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("foreground pid required"))?
                .parse()?,
            args.get(3)
                .ok_or_else(|| anyhow::anyhow!("expected selected text required"))?,
            args.get(4)
                .ok_or_else(|| anyhow::anyhow!("report path required"))?,
        ),
        Some("--service-check") => verification::services(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("output path required"))?,
        ),
        Some("--long-tts-check") => verification::long_tts(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("output path required"))?,
        ),
        Some("--asr-latency-check") => verification::asr_latency(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("input WAV required"))?,
            args.get(3)
                .ok_or_else(|| anyhow::anyhow!("report path required"))?,
        ),
        _ => ui::run(),
    }
}
