//! Local, on-demand dictation editing. No Python service or transcript files.
use crate::{download, llama::Server};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const MODEL: &str = "AgenticASR-Refiner-Q4_K_M";
const FILE: &str = "AgenticASR-Refiner-Q4_K_M.gguf";
const REPO: &str = "MigoXV/AgenticASR-Refiner-GGUF";
const REVISION: &str = "2680cd2ec627e7ae61d9288b5d6f314cb6af2e6a";
const MODEL_BYTES: u64 = 688_065_792;
const MODEL_SHA256: &str = "1dcbbbfde39742b213101c0d5fe645bfda18e5c9b693409205c481efbb487758";
const CONTEXT_TOKENS: usize = 4096;
const MAX_OUTPUT_TOKENS: usize = 3072;
const IDLE_SECONDS: u64 = 60;

// Author's native protocol, shared with the previous verified implementation.
const RULES: &str = "你是 ASR 文本纠错助手。保留原意，最小修改：去口癖/重复，修错字，补必要标点，规范数字、日期、术语和代码符号，处理自我修正。不要总结、扩写或解释。重要易错实体在末尾追加 <KEY>[词1、词2]；没有则不加。";

#[derive(Clone, Serialize)]
pub struct TextStatus {
    pub installed: bool,
    pub ready: bool,
    pub state: &'static str,
    pub model: &'static str,
    pub device: &'static str,
    pub supports_terms: bool,
    pub idle_unload_seconds: u64,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    Loading,
    Processing,
    Ready,
}

struct State {
    server: Option<Arc<Server>>,
    phase: Phase,
    idle_deadline: Option<Instant>,
    shutdown: bool,
}

struct Inner {
    model_path: PathBuf,
    operation: Mutex<()>,
    state: Mutex<State>,
    changed: Condvar,
    epoch: AtomicU64,
}

#[derive(Clone)]
pub struct TextProcessor {
    inner: Arc<Inner>,
}

impl TextProcessor {
    pub fn new(root: PathBuf) -> Self {
        let inner = Arc::new(Inner {
            model_path: root
                .join(".runtime/models/AgenticASR-Refiner-GGUF")
                .join(FILE),
            operation: Mutex::new(()),
            state: Mutex::new(State {
                server: None,
                phase: Phase::Idle,
                idle_deadline: None,
                shutdown: false,
            }),
            changed: Condvar::new(),
            epoch: AtomicU64::new(0),
        });
        let worker = inner.clone();
        std::thread::spawn(move || unload_when_idle(worker));
        Self { inner }
    }

    pub fn health(&self) -> TextStatus {
        let state = self.inner.state.lock().unwrap();
        let installed = self
            .inner
            .model_path
            .metadata()
            .map(|file| file.is_file() && file.len() == MODEL_BYTES)
            .unwrap_or(false);
        let ready = state.server.as_ref().is_some_and(|server| server.ready());
        TextStatus {
            installed,
            ready,
            state: match state.phase {
                Phase::Loading => "loading",
                Phase::Processing => "processing",
                _ if ready => "ready",
                _ if installed => "idle",
                _ => "missing",
            },
            model: MODEL,
            device: "cpu",
            supports_terms: false,
            idle_unload_seconds: IDLE_SECONDS,
        }
    }

    /// Downloading is enabled only by the explicit preparation button.
    #[cfg(test)]
    pub fn prepare(&self, download: bool) -> Result<String> {
        self.prepare_with_ticket(self.ticket(), download)
    }

    /// Capture before dispatching a worker, so Stop also cancels queued work.
    pub fn ticket(&self) -> u64 {
        self.inner.epoch.load(Ordering::SeqCst)
    }

    pub fn prepare_with_ticket(&self, ticket: u64, download: bool) -> Result<String> {
        self.operation_with_ticket(ticket, |ticket| {
            self.ensure_prepared(ticket, download)?;
            Ok("Agentic 已就绪；空闲 60 秒释放内存，下次整理自动加载".into())
        })
    }

    pub fn process_with_ticket(&self, ticket: u64, text: &str, mode: &str) -> Result<String> {
        let timeout = match mode {
            "clean" => Duration::from_secs(25),
            "notes" => Duration::from_secs(90),
            _ => bail!("未知文字整理模式"),
        };
        self.operation_with_ticket(ticket, |ticket| {
            let server = self.ensure_prepared(ticket, false)?;
            {
                let mut state = self.inner.state.lock().unwrap();
                self.check_cancelled(ticket)?;
                state.phase = Phase::Processing;
            }
            let mut body = json!({
                "messages": [{"role": "system", "content": RULES}, {"role": "user", "content": text}],
                "stream": false,
                "chat_template_kwargs": {"enable_thinking": false},
                "temperature": 0, "seed": 42, "cache_prompt": false, "stop": ["<|im_end|>"]
            });
            let tokens = (|| -> Result<(usize, usize)> {
                let template = server.request("/apply-template", &body, Duration::from_secs(5))?;
                let prompt = template["prompt"].as_str().context("整理模型缺少提示模板")?;
                let prompt_tokens = token_count(&server, prompt, true)?;
                let source_tokens = token_count(&server, text, false)?;
                Ok((prompt_tokens, source_tokens))
            })();
            let (prompt_tokens, source_tokens) = match tokens {
                Ok(tokens) => tokens,
                Err(error) => { self.drop_server(); return Err(error); }
            };
            self.check_cancelled(ticket)?;
            body["max_tokens"] = json!(output_budget(prompt_tokens, source_tokens)?);
            let result = match server.request("/v1/chat/completions", &body, timeout) {
                Ok(result) => result,
                Err(error) => {
                    // A timed-out generation must not keep occupying the model.
                    self.drop_server();
                    return Err(error);
                }
            };
            self.check_cancelled(ticket)?;
            completed_body(&result)
        })
    }

    /// Cancels preparation and generation, and releases only this owned model.
    /// A later explicit use may load it again.
    pub fn stop(&self) {
        self.inner.epoch.fetch_add(1, Ordering::SeqCst);
        self.drop_server();
    }

    pub fn shutdown(&self) {
        self.inner.epoch.fetch_add(1, Ordering::SeqCst);
        let mut state = self.inner.state.lock().unwrap();
        state.shutdown = true;
        close_server(&mut state);
        self.inner.changed.notify_all();
    }

    fn operation_with_ticket(
        &self,
        ticket: u64,
        action: impl FnOnce(u64) -> Result<String>,
    ) -> Result<String> {
        let _operation = self.inner.operation.lock().unwrap();
        self.check_cancelled(ticket)?;
        {
            let mut state = self.inner.state.lock().unwrap();
            if state.shutdown {
                bail!("整理服务已关闭");
            }
            state.idle_deadline = None;
            self.inner.changed.notify_all();
        }
        let result = action(ticket);
        let mut state = self.inner.state.lock().unwrap();
        if self.inner.epoch.load(Ordering::SeqCst) == ticket && !state.shutdown {
            if state.server.as_ref().is_some_and(|server| server.ready()) {
                state.phase = Phase::Ready;
                state.idle_deadline = Some(Instant::now() + Duration::from_secs(IDLE_SECONDS));
            } else {
                close_server(&mut state);
            }
            self.inner.changed.notify_all();
        }
        drop(state);
        self.check_cancelled(ticket)?;
        result
    }

    fn check_cancelled(&self, ticket: u64) -> Result<()> {
        if self.inner.epoch.load(Ordering::SeqCst) != ticket {
            bail!("已取消文字整理");
        }
        Ok(())
    }

    fn ensure_prepared(&self, ticket: u64, allow_download: bool) -> Result<Arc<Server>> {
        self.check_cancelled(ticket)?;
        {
            let mut state = self.inner.state.lock().unwrap();
            self.check_cancelled(ticket)?;
            if let Some(server) = &state.server {
                if server.ready() {
                    return Ok(server.clone());
                }
            }
            close_server(&mut state);
            state.phase = Phase::Loading;
        }
        let endpoint =
            std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://huggingface.co".into());
        let url = format!(
            "{}/{REPO}/resolve/{REVISION}/{FILE}?download=true",
            endpoint.trim_end_matches('/')
        );
        download::ensure_file(
            &self.inner.model_path,
            &url,
            MODEL_BYTES,
            MODEL_SHA256,
            allow_download,
            || self.inner.epoch.load(Ordering::SeqCst) != ticket,
        )?;
        self.check_cancelled(ticket)?;
        let mut arguments = vec![
            "-m".into(),
            self.inner.model_path.to_string_lossy().into_owned(),
        ];
        arguments.extend(
            [
                "--device",
                "none",
                "-ngl",
                "0",
                "-c",
                "4096",
                "-np",
                "1",
                "-b",
                "256",
                "-ub",
                "256",
                "-t",
                "4",
                "-tb",
                "4",
                "-fa",
                "on",
                "--fit",
                "off",
                "--cache-ram",
                "0",
                "--no-cache-idle-slots",
                "--no-context-shift",
                "--jinja",
                "--reasoning-format",
                "auto",
                "--log-disable",
            ]
            .map(str::to_owned),
        );
        let server = Arc::new(Server::start(
            &arguments,
            "text",
            Duration::from_secs(60),
            || self.inner.epoch.load(Ordering::SeqCst) != ticket,
        )?);
        let mut state = self.inner.state.lock().unwrap();
        // Stop may arrive after loading succeeds but before the server is published.
        if self.inner.epoch.load(Ordering::SeqCst) != ticket || state.shutdown {
            let _ = server.close();
            bail!("已取消文字整理");
        }
        state.server = Some(server.clone());
        state.phase = Phase::Ready;
        Ok(server)
    }

    fn drop_server(&self) {
        let mut state = self.inner.state.lock().unwrap();
        close_server(&mut state);
        self.inner.changed.notify_all();
    }
}

fn close_server(state: &mut State) {
    if let Some(server) = state.server.take() {
        let _ = server.close();
    }
    // Publish idle only after the process has actually exited.
    state.phase = Phase::Idle;
    state.idle_deadline = None;
}

fn unload_when_idle(inner: Arc<Inner>) {
    let mut state = inner.state.lock().unwrap();
    loop {
        if state.shutdown {
            return;
        }
        if let Some(deadline) = state.idle_deadline {
            if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                state = inner.changed.wait_timeout(state, wait).unwrap().0;
            } else {
                close_server(&mut state);
            }
        } else {
            state = inner.changed.wait(state).unwrap();
        }
    }
}

fn token_count(server: &Server, content: &str, add_special: bool) -> Result<usize> {
    let tokens = server.request(
        "/tokenize",
        &json!({"content":content,"add_special":add_special}),
        Duration::from_secs(5),
    )?;
    Ok(tokens["tokens"]
        .as_array()
        .context("整理模型缺少词元计数")?
        .len())
}

fn output_budget(prompt_tokens: usize, source_tokens: usize) -> Result<usize> {
    let output = 256.max(source_tokens * 3 / 2 + 128);
    if output > MAX_OUTPUT_TOKENS || prompt_tokens + output + 32 > CONTEXT_TOKENS {
        bail!("文字超过本次整理的上下文或完整输出预算；完整原文已保留，请分段整理。");
    }
    Ok(output)
}

fn completed_body(result: &Value) -> Result<String> {
    let choice = &result["choices"][0];
    if choice["finish_reason"] != "stop" {
        bail!("整理输出未完整生成；完整原文已保留，请缩短内容后重试。");
    }
    edited_body(choice["message"]["content"].as_str().unwrap_or_default())
}

fn edited_body(output: &str) -> Result<String> {
    if output.trim().is_empty() {
        bail!("整理模型返回空内容；完整原文已保留。");
    }
    if ["<think>", "</think>", "<|im_start|>", "<|im_end|>"]
        .iter()
        .any(|marker| output.contains(marker))
    {
        bail!("整理模型返回了非正文内容；完整原文已保留。");
    }
    let body = if let Some((body, metadata)) = output.split_once("<KEY>") {
        let valid = metadata
            .trim_end()
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .is_some_and(|value| {
                !value.contains(['[', ']', '\r', '\n']) && !value.contains("<KEY>")
            });
        if !valid {
            bail!("整理模型返回了不完整的正文标记；完整原文已保留。");
        }
        body
    } else {
        output
    };
    if body.trim().is_empty() {
        bail!("整理模型返回空正文；完整原文已保留。");
    }
    Ok(body.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_removal_never_repairs_or_rewrites_body() {
        assert_eq!(
            edited_body("保留 12 ms 和 shader。 <KEY>[shader、12 ms]\n").unwrap(),
            "保留 12 ms 和 shader。"
        );
        assert_eq!(edited_body("原文没有元数据。").unwrap(), "原文没有元数据。");
        for output in [
            "正文<KEY>[未闭合",
            "正文<KEY>[x]尾随正文",
            "正文<KEY>[x]<KEY>[y]",
            "<KEY>[x]",
            "正文<KEY>[x\ny]",
            "<think>推理</think>正文",
        ] {
            assert!(edited_body(output).is_err(), "{output}");
        }
    }

    #[test]
    fn output_budget_preserves_room_for_complete_long_notes() {
        assert_eq!(output_budget(200, 40).unwrap(), 256);
        assert_eq!(output_budget(1300, 1000).unwrap(), 1628);
        assert!(output_budget(2600, 2000).is_err());
        assert!(output_budget(1000, 2000).is_err());
        assert_eq!(output_budget(1000, 1536).unwrap(), 2432);
    }

    #[test]
    fn unfinished_generation_is_never_delivered_as_full_text() {
        for reason in ["length", "error", ""] {
            assert!(
                completed_body(
                    &json!({"choices":[{"finish_reason":reason,"message":{"content":"尚未写完"}}]})
                )
                .is_err()
            );
        }
        assert!(
            completed_body(
                &json!({"choices":[{"finish_reason":"stop","message":{"content":" "}}]})
            )
            .is_err()
        );
        assert_eq!(
            completed_body(
                &json!({"choices":[{"finish_reason":"stop","message":{"content":"完整正文。"}}]})
            )
            .unwrap(),
            "完整正文。"
        );
    }

    #[test]
    fn stopping_invalidates_pending_operations_without_loading_a_model() {
        let processor = TextProcessor::new(std::env::temp_dir().join("local-voice-no-text-model"));
        let ticket = processor.inner.epoch.load(Ordering::SeqCst);
        processor.stop();
        assert!(processor.check_cancelled(ticket).is_err());
        let status = processor.health();
        assert!(!status.ready);
        assert!(!status.installed);
        assert_eq!(status.state, "missing");
        processor.shutdown();
        assert!(
            processor
                .prepare(false)
                .unwrap_err()
                .to_string()
                .contains("关闭")
        );
    }

    #[test]
    fn preparation_reserved_before_stop_cannot_load_when_worker_starts_later() {
        let processor = TextProcessor::new(std::env::temp_dir().join("local-voice-no-text-model"));
        let ticket = processor.ticket();
        let worker = processor.clone();
        let (release, wait) = std::sync::mpsc::channel();
        let queued = std::thread::spawn(move || {
            wait.recv().unwrap();
            worker.prepare_with_ticket(ticket, false)
        });
        processor.stop();
        release.send(()).unwrap();
        let error = queued.join().unwrap().unwrap_err().to_string();
        assert!(
            error.contains("取消"),
            "Queued preparation reached model loading: {error}"
        );
        assert_eq!(processor.health().state, "missing");
        assert!(!processor.health().ready);
        processor.shutdown();
    }

    #[test]
    fn stop_discards_in_flight_result_and_does_not_publish_ready() {
        let processor = TextProcessor::new(std::env::temp_dir().join("local-voice-no-text-model"));
        let (started, waiting) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let worker = processor.clone();
        let operation = std::thread::spawn(move || {
            worker.operation_with_ticket(worker.ticket(), |_| {
                worker.inner.state.lock().unwrap().phase = Phase::Processing;
                started.send(()).unwrap();
                resume.recv().unwrap();
                Ok("应当丢弃".into())
            })
        });
        waiting.recv_timeout(Duration::from_secs(3)).unwrap();
        processor.stop();
        release.send(()).unwrap();
        assert!(
            operation
                .join()
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("取消")
        );
        assert!(!processor.health().ready);
        assert!(
            processor
                .inner
                .state
                .lock()
                .unwrap()
                .idle_deadline
                .is_none()
        );
        processor.shutdown();
    }
}
