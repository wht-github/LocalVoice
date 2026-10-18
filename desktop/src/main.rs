#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod audio;
mod config;
mod diagnostics;
mod download;
mod llama;
mod native;
mod native_check;
mod paths;
mod runtime;
mod sensevoice;
mod service;
mod text;
mod ui;
mod vad;
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
    Cancel,
    Copy,
    CopyOriginal,
    CopyResult,
    RetryText,
    PrepareText,
    SelectOutput(String),
    TextPrepared(u64, std::result::Result<String, String>),
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
    ProcessingText(u64),
    Audio(u64, Vec<f32>, u32, Instant),
    Level(u64, f32),
    Recognized(
        u64,
        std::result::Result<service::Dictation, String>,
        Instant,
    ),
}
struct Job {
    id: u64,
    input: JobInput,
    config: Config,
    queued_at: Instant,
    preview_only: bool,
}
enum JobInput {
    Audio(Vec<f32>, u32),
    Text(String),
}
fn status(ui: &slint::Weak<VoiceWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = ui.upgrade_in_event_loop(move |ui| ui.set_status(text.into()));
}
fn state(ui: &slint::Weak<VoiceWindow>, recording: bool, pending: bool, processing: bool) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_recording(recording);
        ui.set_has_pending(pending);
        ui.set_processing(processing);
        if !recording {
            ui.set_capturing(false);
            ui.set_level(0.);
        }
    });
}

fn next_event(
    receiver: &mpsc::Receiver<Event>,
    delivery: &mut VecDeque<(u64, service::Dictation, Instant)>,
    current: u64,
    modifiers_down: bool,
) -> std::result::Result<(Event, bool), mpsc::RecvTimeoutError> {
    delivery.retain(|(id, _, _)| *id == current);
    // Honor queued Cancel/Quit before inserting a result deferred by the shortcut.
    match receiver.try_recv() {
        Ok(event) => return Ok((event, false)),
        Err(mpsc::TryRecvError::Disconnected) => return Err(mpsc::RecvTimeoutError::Disconnected),
        Err(mpsc::TryRecvError::Empty) => {}
    }
    if !modifiers_down {
        if let Some((id, transcript, at)) = delivery.pop_front() {
            return Ok((Event::Recognized(id, Ok(transcript), at), true));
        }
    }
    receiver
        .recv_timeout(Duration::from_millis(15))
        .map(|event| (event, false))
}

fn controller(
    initial: Config,
    shared: Arc<Mutex<Config>>,
    ui: slint::Weak<VoiceWindow>,
    settings: slint::Weak<SettingsWindow>,
    result_window: slint::Weak<ResultWindow>,
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
    let mut keys_ready = match native::register_keys(&config.record_key) {
        Ok(()) => true,
        Err(error) => {
            status(&ui, error.to_string());
            false
        }
    };
    let generation = Arc::new(AtomicU64::new(1));
    let (jobs, job_rx) = mpsc::sync_channel::<Job>(2);
    let replies = events.clone();
    let worker_gen = generation.clone();
    thread::spawn(move || {
        while let Ok(job) = job_rx.recv() {
            if job.id != worker_gen.load(Ordering::SeqCst) {
                continue;
            }
            let result = (|| {
                let encoding = Instant::now();
                let transcript = match job.input {
                    JobInput::Audio(samples, rate) => {
                        diagnostics::record(
                            "asr_request",
                            serde_json::json!({"generation":job.id,"asr_mode":job.config.asr_mode,"output_mode":job.config.output_mode,"term_count":job.config.terms.len(),"audio_seconds":samples.len() as f64/rate as f64,"sample_rate":rate,"queue_ms":encoding.duration_since(job.queued_at).as_millis()}),
                        );
                        service::transcribe_recording(
                            &job.config,
                            &samples,
                            rate,
                            || job.id != worker_gen.load(Ordering::SeqCst),
                            |index, total| {
                                let _ =
                                    replies.send(Event::RecognitionProgress(job.id, index, total));
                            },
                        )?
                    }
                    JobInput::Text(text) => service::Transcript {
                        text,
                        request_seconds: 0.,
                        inference_seconds: None,
                    },
                };
                if job.config.output_mode != "raw" && !transcript.text.trim().is_empty() {
                    let _ = replies.send(Event::ProcessingText(job.id));
                }
                service::process_transcript(&job.config, transcript, job.preview_only, || {
                    job.id != worker_gen.load(Ordering::SeqCst)
                })
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
    let mut session_config = config.clone();
    let mut pending = String::new();
    let mut latest: Option<service::Dictation> = None;
    let mut preparing_text = false;
    let mut preparation_id = 0u64;
    let mut inflight = 0usize;
    let mut stopping = false;
    let mut delivery: VecDeque<(u64, service::Dictation, Instant)> = VecDeque::new();
    loop {
        for key in native::poll_keys() {
            if key == 1 {
                let _ = events.send(Event::Record);
            }
        }
        // Fast ASR can finish before the stop shortcut is released. Keep pumping
        // events so Cancel/Quit still work; never synthesize modifier key releases.
        let next = next_event(
            &receiver,
            &mut delivery,
            generation.load(Ordering::SeqCst),
            native::modifiers_down(),
        );
        let (event, from_delivery) = match next {
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
            Event::SelectOutput(mode) => {
                if recording || inflight > 0 || stopping || preparing_text {
                    ui::set_output(&ui, &config.output_mode);
                    ui::set_result_output(&result_window, &config.output_mode);
                    status(&ui, "请结束当前任务后再切换输出模式");
                    continue;
                }
                let mut next = config.clone();
                next.output_mode = mode;
                match next.save() {
                    Ok(()) => {
                        config = next;
                        *shared.lock().unwrap() = config.clone();
                        status(
                            &ui,
                            if config.output_mode == "raw" {
                                "原样转写 · 保留口述表达"
                            } else if config.output_mode == "notes" {
                                "笔记 · 手动结束后整理正文并输入"
                            } else {
                                "日常输入 · 手动结束后轻整理并输入"
                            },
                        );
                    }
                    Err(error) => status(&ui, error.to_string()),
                }
                ui::set_output(&ui, &config.output_mode);
                ui::set_result_output(&result_window, &config.output_mode);
            }
            Event::PrepareText => {
                if preparing_text || recording || inflight > 0 || stopping {
                    status(&ui, "请先完成当前任务，再准备整理模型");
                    continue;
                }
                let prepare = match service::prepare_text_task() {
                    Ok(task) => task,
                    Err(error) => {
                        status(&ui, format!("无法准备整理模型：{error}"));
                        continue;
                    }
                };
                preparing_text = true;
                preparation_id += 1;
                let id = preparation_id;
                let tx = events.clone();
                let _ = settings.upgrade_in_event_loop(|s| {
                    s.set_text_busy(true);
                    s.set_text_status("正在下载、校验并加载整理模型，首次可能需要几分钟…".into());
                });
                thread::spawn(move || {
                    let prepared = prepare().map_err(|e| e.to_string());
                    let _ = tx.send(Event::TextPrepared(id, prepared));
                });
            }
            Event::TextPrepared(id, result) if id == preparation_id => {
                preparing_text = false;
                let prepared = result.is_ok();
                let message = result.unwrap_or_else(|e| format!("准备失败：{e}"));
                status(&ui, message.clone());
                let _ = settings.upgrade_in_event_loop(move |s| {
                    s.set_text_busy(false);
                    if prepared {
                        s.set_text_installed(true);
                        s.set_text_ready(true);
                    }
                    s.set_text_status(message.into());
                });
            }
            Event::CopyOriginal | Event::CopyResult => {
                if let Some(value) = &latest {
                    let text = if matches!(event, Event::CopyOriginal) {
                        &value.original
                    } else {
                        &value.text
                    };
                    match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.clone())) {
                        Ok(()) => status(&ui, "最近结果已复制"),
                        Err(error) => status(&ui, error.to_string()),
                    }
                }
            }
            Event::RetryText => {
                if recording || inflight > 0 || stopping || preparing_text {
                    status(&ui, "请先完成当前任务，再重新整理");
                    continue;
                }
                if let Some(previous) = &latest {
                    if config.output_mode == "raw" || previous.original.is_empty() {
                        continue;
                    }
                    let id = generation.fetch_add(1, Ordering::SeqCst) + 1;
                    let cfg = config.clone();
                    session_config = cfg.clone();
                    continuous_session = false;
                    target = None;
                    if jobs
                        .try_send(Job {
                            id,
                            input: JobInput::Text(previous.original.clone()),
                            config: cfg,
                            queued_at: Instant::now(),
                            preview_only: true,
                        })
                        .is_ok()
                    {
                        inflight = 1;
                        status(&ui, "正在重新整理，完成后在结果窗口查看…");
                    } else {
                        status(&ui, "上一任务仍在退出，请稍后重试");
                    }
                }
            }
            Event::Health(result) => {
                let message = if result.is_ok() {
                    "识别模型已就绪"
                } else {
                    "模型未就绪，可在设置中加载模型"
                };
                let _ =
                    settings.upgrade_in_event_loop(move |s| s.set_service_status(message.into()));
                if keys_ready && !recording && inflight == 0 {
                    match result {
                        Ok(()) => status(&ui, format!("就绪 · {} 说话", config.record_key)),
                        Err(_) => status(&ui, "识别模型未就绪，请在设置中加载模型"),
                    }
                }
            }
            Event::Runtime(busy, message) => {
                if !recording && inflight == 0 {
                    status(&ui, message.clone());
                }
                let _ = settings.upgrade_in_event_loop(move |s| {
                    s.set_services_busy(busy);
                    s.set_service_status(message.into());
                });
            }
            Event::StartServices => {
                if recording || inflight > 0 || stopping || preparing_text {
                    status(&ui, "请先完成当前任务，再加载模型；停止全部模型可中断任务");
                    continue;
                }
                manager.send(runtime::Operation::Start(config.clone()));
            }
            Event::StopServices => {
                stopping = false;
                if !runtime::managed(&config) {
                    status(&ui, "识别模型类型无效");
                    continue;
                }
                if let Some(c) = capture.take() {
                    c.stop(false);
                }
                generation.fetch_add(1, Ordering::SeqCst);
                recording = false;
                inflight = 0;
                preparing_text = false;
                preparation_id += 1;
                let _ = settings.upgrade_in_event_loop(|s| {
                    s.set_text_busy(false);
                    s.set_text_ready(false);
                    s.set_text_status("模型已停止；下次整理会自动加载".into());
                });
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
                    target = if continuous_session {
                        None
                    } else {
                        let destination = automation.target();
                        diagnostics::record(
                            "dictation_stop_target",
                            serde_json::json!({"captured":destination.is_ok(),
                                "error":destination.as_ref().err().map(|e| e.to_string())}),
                        );
                        destination.ok()
                    };
                    if let Some(c) = capture.take() {
                        c.stop(true);
                    }
                    status(&ui, "录音已结束，正在识别整段内容…");
                } else if preparing_text {
                    status(&ui, "正在准备整理模型，请稍候；停止全部模型可中断准备");
                } else if inflight > 0 || stopping {
                    status(&ui, "请等待本次识别完成，或点击取消");
                } else {
                    let id = generation.fetch_add(1, Ordering::SeqCst) + 1;
                    session_config = config.clone();
                    continuous_session = config.continuous_dictation && config.output_mode == "raw";
                    session_config.continuous_dictation = continuous_session;
                    target = None;
                    capture = Some(audio::Capture::start(
                        session_config.clone(),
                        id,
                        events.clone(),
                    ));
                    recording = true;
                    status(&ui, "正在打开麦克风…");
                }
            }
            Event::Cancel => {
                if preparing_text {
                    status(&ui, "整理模型仍在准备；请在设置中停止全部模型以中断");
                    continue;
                }
                stopping = false;
                generation.fetch_add(1, Ordering::SeqCst);
                if let Some(c) = capture.take() {
                    c.stop(false);
                }
                service::stop_text();
                recording = false;
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
                let _ = ui.upgrade_in_event_loop(|ui| ui.set_capturing(true));
                status(
                    &ui,
                    if continuous_session {
                        "连续听写中 · 停顿后输入 · 再按结束"
                    } else if session_config.output_mode == "notes" {
                        "笔记录音中 · 选好输入框后，手动结束说话"
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
                        format!("正在识别整段录音 · {index}/{total}")
                    },
                );
            }
            Event::ProcessingText(id) if id == current => {
                status(
                    &ui,
                    if session_config.output_mode == "notes" {
                        "正在整理笔记正文，保留术语与原意…"
                    } else {
                        "正在轻整理，保留术语与原意…"
                    },
                );
            }
            Event::Audio(id, samples, rate, queued_at) if id == current => {
                // Include deferred results in the bound, not just HTTP jobs.
                if inflight < 3
                    && jobs
                        .try_send(Job {
                            id,
                            input: JobInput::Audio(samples, rate),
                            config: session_config.clone(),
                            queued_at,
                            preview_only: false,
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
                if result.as_ref().is_ok_and(|value| !value.preview_only)
                    && (native::modifiers_down() || (!from_delivery && !delivery.is_empty()))
                {
                    let item = (id, result.unwrap(), returned_at);
                    if from_delivery {
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
                        let notice = transcript.warning.clone().unwrap_or_else(|| {
                            match transcript.mode.as_str() {
                                "notes" => "笔记正文已整理，原始转写仍可查看和复制",
                                "clean" => "轻整理完成，原始转写仍可复制",
                                _ => "原样转写",
                            }
                            .into()
                        });
                        ui::set_latest(
                            &result_window,
                            transcript.original.clone(),
                            transcript.text.clone(),
                            notice.clone(),
                            !transcript.original.is_empty(),
                        );
                        latest = Some(transcript.clone());
                        let text = transcript.text;
                        if transcript.preview_only {
                            status(&ui, notice);
                            ui::show_latest(&result_window, true);
                        } else {
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
                                serde_json::json!({"generation":id,"output_mode":transcript.mode,"http_seconds":transcript.request_seconds,"server_seconds":transcript.inference_seconds,"processing_seconds":transcript.processing_seconds,"result_dispatch_ms":insert_started.duration_since(returned_at).as_millis(),"target_check_and_insert_ms":insert_started.elapsed().as_millis(),"inserted":matches!(&inserted,Some(Ok(()))),"delivery_error":inserted.as_ref().and_then(|r| r.as_ref().err()).map(|e| e.to_string())}),
                            );
                            match inserted {
                                Some(Ok(())) => {
                                    status(
                                        &ui,
                                        transcript
                                            .warning
                                            .as_ref()
                                            .map(|warning| format!("已输入原文 · {warning}"))
                                            .unwrap_or_else(|| {
                                                if recording && continuous_session {
                                                    "已输入 · 连续听写中"
                                                } else if recording {
                                                    "已输入 · 继续说话"
                                                } else {
                                                    "已输入到目标应用"
                                                }
                                                .into()
                                            }),
                                    );
                                }
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
                                            _ if continuous_session
                                                && pending.len() > 256 * 1024 =>
                                            {
                                                "待复制文字积累过多，已暂停听写，请先复制保存"
                                                    .into()
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
                            if transcript.mode == "notes" && session_config.show_note_preview {
                                // The optional comparison must never take focus
                                // from the destination receiving keyboard input.
                                ui::show_latest(&result_window, false);
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
            Event::Configure(next) => {
                let result = (|| -> Result<()> {
                    next.validate()?;
                    if (next.asr_mode != config.asr_mode || next.output_mode != config.output_mode)
                        && (recording || inflight > 0 || stopping || preparing_text)
                    {
                        anyhow::bail!("请先完成当前任务，再更换模型或输出模式");
                    }
                    native::hotkey(&next.record_key)?;
                    native::unregister_keys();
                    if let Err(error) = native::register_keys(&next.record_key) {
                        let _ = native::register_keys(&config.record_key);
                        return Err(error);
                    }
                    if let Err(error) = next.save() {
                        native::unregister_keys();
                        let _ = native::register_keys(&config.record_key);
                        return Err(error);
                    }
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        keys_ready = true;
                        let asr_changed = config.asr_mode != next.asr_mode;
                        config = next;
                        *shared.lock().unwrap() = config.clone();
                        ui::set_output(&ui, &config.output_mode);
                        ui::set_result_output(&result_window, &config.output_mode);
                        if asr_changed {
                            manager.send(runtime::Operation::Start(config.clone()));
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
        let processing = inflight > 0 || stopping || preparing_text;
        state(&ui, recording, !pending.is_empty(), processing);
        ui::set_result_busy(&result_window, recording || processing);
    }
    native::unregister_keys();
}
#[cfg(test)]
mod delivery_tests {
    use super::*;

    #[test]
    fn notes_wait_for_shortcut_release_and_deliver_once() {
        let (_sender, receiver) = mpsc::channel();
        let note = service::Dictation {
            original: "嗯，先检查 shader，再检查 transition".into(),
            text: "先检查 shader，再检查 transition。".into(),
            mode: "notes".into(),
            warning: None,
            request_seconds: 0.,
            inference_seconds: None,
            processing_seconds: 0.,
            preview_only: false,
        };
        let mut delivery = VecDeque::from([(1, note, Instant::now())]);
        assert!(matches!(
            next_event(&receiver, &mut delivery, 1, true),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(matches!(
            next_event(&receiver, &mut delivery, 1, false),
            Ok((Event::Recognized(1, Ok(note), _), true))
                if note.mode == "notes" && !note.preview_only
        ));
        assert!(matches!(
            next_event(&receiver, &mut delivery, 1, false),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
    }

    #[test]
    fn queued_cancel_precedes_deferred_input_and_stale_results_are_discarded() {
        let (sender, receiver) = mpsc::channel();
        let result = service::Dictation {
            original: "不要修改 shader".into(),
            text: "不要修改 shader。".into(),
            mode: "clean".into(),
            warning: None,
            request_seconds: 0.,
            inference_seconds: None,
            processing_seconds: 0.,
            preview_only: false,
        };
        let mut delivery = VecDeque::from([(1, result, Instant::now())]);
        sender.send(Event::Cancel).unwrap();
        assert!(matches!(
            next_event(&receiver, &mut delivery, 1, false),
            Ok((Event::Cancel, false))
        ));
        assert_eq!(delivery.len(), 1);
        assert!(matches!(
            next_event(&receiver, &mut delivery, 2, false),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(delivery.is_empty());
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--vad-check") => native_check::run_vad(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("report path required"))?,
        ),
        Some("--native-smoke") => verification::native_smoke(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("input WAV required"))?,
            args.get(3)
                .ok_or_else(|| anyhow::anyhow!("report path required"))?,
        ),
        Some("--dictation-ui-check") => verification::dictation_ui_check(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("report directory required"))?,
        ),
        Some("--interaction-check") => verification::interaction_check(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("report directory required"))?,
        ),
        Some("--topmost-check") => verification::topmost_check(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("report directory required"))?,
        ),
        Some("--floating-check") => verification::floating_check(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("report directory required"))?,
            args.iter().any(|arg| arg == "--unpatched"),
        ),
        Some("--display-check") => verification::display_check(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("report directory required"))?,
            args.iter().any(|arg| arg == "--unpatched"),
        ),
        Some("--asr-models-check") => verification::asr_models_check(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("input WAV required"))?,
            args.get(3)
                .ok_or_else(|| anyhow::anyhow!("report path required"))?,
        ),
        Some("--manual-asr-check") => verification::manual_asr(
            args.get(2)
                .ok_or_else(|| anyhow::anyhow!("input WAV required"))?,
            args.get(3)
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
        Some("--service-check") => verification::service_check(
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
