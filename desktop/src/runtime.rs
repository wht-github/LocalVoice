//! Own the selected native ASR model. Python and public service ports are unused.
use crate::{config::Config, llama::Server, sensevoice::Recognizer};
use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub enum Operation {
    Start(Config),
    Stop,
}
pub struct Manager {
    sender: mpsc::Sender<(u64, Operation)>,
    revision: Arc<AtomicU64>,
}
impl Manager {
    pub fn new(mut notify: impl FnMut(bool, String) + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::channel::<(u64, Operation)>();
        let revision = Arc::new(AtomicU64::new(0));
        let current = revision.clone();
        thread::spawn(move || {
            while let Ok((id, operation)) = receiver.recv() {
                if current.load(Ordering::SeqCst) != id {
                    continue;
                }
                notify(true, "正在调整识别模型…".into());
                let result = match operation {
                    // Stop is performed when it is sent, before a later Start can
                    // supersede its notification or start another model.
                    Operation::Stop => Ok("识别与整理模型已停止".into()),
                    Operation::Start(config) => apply(Operation::Start(config), || {
                        current.load(Ordering::SeqCst) != id
                    }),
                };
                if current.load(Ordering::SeqCst) == id {
                    notify(
                        false,
                        result.unwrap_or_else(|e| format!("模型操作失败：{e:#}；可重试启动")),
                    );
                }
            }
        });
        Self { sender, revision }
    }
    pub fn send(&self, operation: Operation) {
        let id = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        if matches!(operation, Operation::Stop) {
            native_stop();
        }
        let _ = self.sender.send((id, operation));
    }
}

enum Model {
    Qwen(Server),
    SenseVoice(Recognizer),
}
impl Model {
    fn ready(&self) -> bool {
        match self {
            Self::Qwen(server) => server.ready(),
            Self::SenseVoice(_) => true,
        }
    }
    fn close(&self) {
        if let Self::Qwen(server) = self {
            if let Err(error) = server.close() {
                eprintln!("{error:#}");
            }
        }
    }
}

#[derive(Default)]
struct State {
    mode: String,
    model: Option<Arc<Model>>,
}
static NATIVE: OnceLock<Mutex<State>> = OnceLock::new();
static START_LOCK: Mutex<()> = Mutex::new(());
static REVISION: AtomicU64 = AtomicU64::new(0);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

fn state() -> &'static Mutex<State> {
    NATIVE.get_or_init(|| Mutex::new(State::default()))
}

fn native_start(config: &Config, cancelled: impl Fn() -> bool) -> Result<()> {
    // Start operations are serialized; stop never waits for model loading.
    let _starting = START_LOCK.lock().unwrap();
    if SHUTTING_DOWN.load(Ordering::SeqCst) || cancelled() {
        bail!("模型加载已取消");
    }
    let (previous, revision) = {
        let mut state = state().lock().unwrap();
        if state.mode == config.asr_mode && state.model.as_ref().is_some_and(|m| m.ready()) {
            return Ok(());
        }
        let revision = REVISION.fetch_add(1, Ordering::SeqCst) + 1;
        state.mode = config.asr_mode.clone();
        (state.model.take(), revision)
    };
    if let Some(previous) = previous {
        previous.close();
    }
    let cancelled = || {
        cancelled()
            || SHUTTING_DOWN.load(Ordering::SeqCst)
            || REVISION.load(Ordering::SeqCst) != revision
    };
    let model = match config.asr_mode.as_str() {
        "sensevoice-cpu" => Model::SenseVoice(Recognizer::load()?),
        "qwen-llama-0.6b" | "qwen-llama-1.7b" => Model::Qwen(Server::start(
            &qwen_arguments(&config.asr_mode)?,
            "asr",
            Duration::from_secs(180),
            &cancelled,
        )?),
        _ => bail!("识别模式无效"),
    };
    let mut state = state().lock().unwrap();
    if cancelled() {
        bail!("模型加载已取消");
    }
    state.model = Some(Arc::new(model));
    Ok(())
}

fn qwen_arguments(mode: &str) -> Result<Vec<String>> {
    let size = qwen_size(mode)?;
    let directory = crate::paths::runtime()?.join(format!("models/Qwen3-ASR-{size}-GGUF"));
    let decoder = directory.join(format!("Qwen3-ASR-{size}-Q8_0.gguf"));
    let encoder = directory.join(format!("mmproj-Qwen3-ASR-{size}-bf16.gguf"));
    for path in [&decoder, &encoder] {
        if !path.is_file() {
            bail!(
                "缺少 {}；{}",
                path.display(),
                crate::installation::setup_hint(&format!(
                    "请运行 scripts/setup-llama.ps1 和 scripts/download-qwen-gguf.ps1 -Size {size}"
                ))
            );
        }
    }
    let mut args = vec![
        "-m".into(),
        decoder.to_string_lossy().into_owned(),
        "--mmproj".into(),
        encoder.to_string_lossy().into_owned(),
    ];
    args.extend(
        [
            "--device",
            "CUDA0",
            "-ngl",
            "99",
            "-c",
            "2048",
            "-b",
            "256",
            "-ub",
            "256",
            "-np",
            "1",
            "-t",
            "4",
            "-tb",
            "4",
            "-fa",
            "on",
            "-ctk",
            "f16",
            "-ctv",
            "f16",
            "--fit",
            "off",
            "--cache-ram",
            "0",
            "--no-cache-idle-slots",
            "--no-context-shift",
        ]
        .map(str::to_owned),
    );
    Ok(args)
}

fn qwen_size(mode: &str) -> Result<&'static str> {
    match mode {
        "qwen-llama-0.6b" => Ok("0.6B"),
        "qwen-llama-1.7b" => Ok("1.7B"),
        _ => bail!("识别模式无效"),
    }
}

fn selected(config: &Config) -> Result<Arc<Model>> {
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        bail!("应用正在退出");
    }
    let state = state().lock().unwrap();
    if state.mode != config.asr_mode {
        bail!("所选识别模型尚未启动，请在设置中启动模型");
    }
    state
        .model
        .as_ref()
        .filter(|model| model.ready())
        .cloned()
        .context("识别模型尚未就绪，请在设置中启动模型")
}

pub fn ready(config: &Config) -> Result<()> {
    selected(config).map(|_| ())
}

pub fn transcribe(config: &Config, samples: &[f32], rate: u32) -> Result<String> {
    let revision = REVISION.load(Ordering::SeqCst);
    let model = selected(config)?;
    if samples.is_empty() || samples.iter().all(|&value| value == 0.) {
        return Ok(String::new());
    }
    let samples = crate::vad::resample_mono(samples, rate, 16000)?;
    let result = match model.as_ref() {
        Model::SenseVoice(recognizer) => recognizer.transcribe(&samples),
        Model::Qwen(server) => {
            let context = config
                .terms
                .iter()
                .map(|term| term.text.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            if !context.is_empty() {
                let tokens = server.request(
                    "/tokenize",
                    &json!({"content": context}),
                    Duration::from_secs(5),
                )?;
                if tokens["tokens"]
                    .as_array()
                    .context("模型未返回术语分词结果")?
                    .len()
                    > 512
                {
                    bail!("术语上下文超过 512 tokens；请减少本次术语");
                }
            }
            let wav = crate::audio::encode_wav(&samples, 16000)?;
            let body = json!({
                "messages": [
                    {"role": "system", "content": context},
                    {"role": "user", "content": [{"type": "input_audio", "input_audio": {
                        "data": STANDARD.encode(wav), "format": "wav"
                    }}]}
                ],
                "temperature": 0, "seed": 42, "max_tokens": 512,
                "cache_prompt": false, "stream": false
            });
            parse_qwen_response(&server.request(
                "/v1/chat/completions",
                &body,
                Duration::from_secs(90),
            )?)
        }
    }?;
    if SHUTTING_DOWN.load(Ordering::SeqCst) || REVISION.load(Ordering::SeqCst) != revision {
        bail!("识别已停止");
    }
    Ok(result)
}

fn parse_qwen_response(value: &Value) -> Result<String> {
    let choice = &value["choices"][0];
    if choice["finish_reason"] != "stop" {
        bail!("识别未完整结束，请缩短录音后重试");
    }
    let raw = choice["message"]["content"]
        .as_str()
        .context("模型未返回识别正文")?;
    let (_, text) = raw
        .split_once("<asr_text>")
        .context("Qwen 返回了无法解析的识别结果")?;
    Ok(text.trim().to_owned())
}

pub fn native_stop() {
    REVISION.fetch_add(1, Ordering::SeqCst);
    crate::service::stop_text();
    let model = state().lock().unwrap().model.take();
    if let Some(model) = model {
        model.close();
    }
}

pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    crate::service::shutdown_text();
    native_stop();
}

pub fn managed(config: &Config) -> bool {
    crate::config::ASR_MODES
        .iter()
        .any(|(mode, _)| *mode == config.asr_mode)
}

pub fn apply(operation: Operation, cancelled: impl Fn() -> bool) -> Result<String> {
    let config = match operation {
        Operation::Stop => {
            native_stop();
            return Ok("识别与整理模型已停止".into());
        }
        Operation::Start(config) => config,
    };
    config.validate()?;
    native_start(&config, &cancelled)?;
    let name = crate::config::ASR_MODES
        .iter()
        .find(|(mode, _)| *mode == config.asr_mode)
        .map(|(_, label)| *label)
        .unwrap_or("未知模型");
    Ok(format!("识别已就绪 · {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_invalidates_now_even_if_a_later_start_replaces_its_queued_notification() {
        let (entered, wait_entered) = mpsc::channel();
        let (release, wait_release) = mpsc::channel();
        let (completed, wait_completed) = mpsc::channel();
        let mut first = true;
        let manager = Manager::new(move |busy, message| {
            if busy && first {
                first = false;
                entered.send(()).unwrap();
                let _ = wait_release.recv();
            } else if !busy {
                let _ = completed.send(message);
            }
        });
        let invalid = Config {
            asr_mode: "invalid-test-mode".into(),
            ..Config::default()
        };
        manager.send(Operation::Start(invalid.clone()));
        wait_entered.recv_timeout(Duration::from_secs(3)).unwrap();
        let revision = REVISION.load(Ordering::SeqCst);
        manager.send(Operation::Stop);
        assert!(
            REVISION.load(Ordering::SeqCst) > revision,
            "Stop waited for the blocked manager queue before invalidating work"
        );
        manager.send(Operation::Start(invalid));
        release.send(()).unwrap();
        let message = wait_completed.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(message.contains("识别模式无效"), "{message}");
    }

    #[test]
    fn native_model_selection_supports_only_installed_backend_types() {
        for (mode, _) in crate::config::ASR_MODES {
            assert!(managed(&Config {
                asr_mode: (*mode).into(),
                ..Config::default()
            }));
        }
        assert!(!managed(&Config {
            asr_mode: "unknown".into(),
            ..Config::default()
        }));
    }

    #[test]
    fn qwen_response_preserves_mixed_terms_and_numeric_corrections() {
        let text = "把 AGV 的 Blueprint 导入 UE，坐标 12，不对，改成 21。";
        let value = json!({"choices": [{"finish_reason": "stop", "message": {
            "content": format!("language Chinese<asr_text>{text}")
        }}]});
        assert_eq!(parse_qwen_response(&value).unwrap(), text);
    }

    #[test]
    fn qwen_rejects_truncation_or_an_unparseable_response() {
        for value in [
            json!({"choices": [{"finish_reason": "length", "message": {"content": "<asr_text>部分"}}]}),
            json!({"choices": [{"finish_reason": "stop", "message": {"content": "unframed"}}]}),
            json!({"choices": []}),
        ] {
            assert!(parse_qwen_response(&value).is_err());
        }
    }
}
