//! Host-side control of this app's dedicated WSL units. No management HTTP server.
use crate::config::Config;
use anyhow::{Context, Result, bail};
use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub enum Operation {
    Start(Config),
    Stop,
    Tts(Config),
    Asr(Config),
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
                notify(true, "正在调整语音服务…".into());
                let result = apply(operation, || current.load(Ordering::SeqCst) != id);
                if current.load(Ordering::SeqCst) == id {
                    notify(
                        false,
                        result.unwrap_or_else(|e| format!("服务操作失败：{e:#}；可重试启动")),
                    );
                }
            }
        });
        Self { sender, revision }
    }
    pub fn send(&self, operation: Operation) {
        let id = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.sender.send((id, operation));
    }
}
fn wsl(user: &str) -> Command {
    let mut command = Command::new("wsl.exe");
    command
        .args(["-d", "LocalVoice", "-u", user, "--cd", "/", "--"])
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}
fn unit(action: &str, units: &[&str]) -> Result<()> {
    let mut child = wsl("root")
        .arg("systemctl")
        .arg(action)
        .args(units)
        .spawn()
        .context("无法启动 LocalVoice，请确认专用 WSL 已安装")?;
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("LocalVoice 服务 {action} 失败，请查看 voice.ps1 logs");
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("LocalVoice 服务操作超时");
        }
        thread::sleep(Duration::from_millis(100));
    }
}
fn select_asr(config: &Config) -> Result<()> {
    config.validate()?;
    let mut child = wsl("root")
        .args([
            "bash",
            "/opt/local-voice-app/scripts/select-asr.sh",
            &config.asr_mode,
        ])
        .spawn()
        .context("无法切换识别模式")?;
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("识别模式切换失败，已尝试恢复原模式；请查看语音服务日志");
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("识别模式切换超时，请检查服务日志");
        }
        thread::sleep(Duration::from_millis(100));
    }
}
pub fn managed(config: &Config) -> bool {
    config.asr_url.trim_end_matches('/') == "http://127.0.0.1:8001"
        && config.tts_url.trim_end_matches('/') == "http://127.0.0.1:8002"
}
fn keep_alive() -> Result<()> {
    let mut child = wsl("voice")
        .args([
            "flock",
            "-n",
            "/home/voice/.local/share/local-voice-app/keepalive.lock",
            "sleep",
            "infinity",
        ])
        .spawn()?;
    // Reap duplicate flock attempts; the sole successful holder keeps this distro alive.
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
pub fn apply(operation: Operation, cancelled: impl Fn() -> bool) -> Result<String> {
    let (config, tts_only, asr_only) = match operation {
        Operation::Stop => {
            unit("stop", &["local-voice-asr", "local-voice-tts"])?;
            return Ok("识别、朗读均已停止，模型内存已释放".into());
        }
        Operation::Start(c) => (c, false, false),
        Operation::Tts(c) => (c, true, false),
        Operation::Asr(c) => (c, false, true),
    };
    if !managed(&config) {
        bail!("自定义地址请自行管理；一键启停仅支持默认 LocalVoice 地址");
    }
    if !asr_only && !config.tts_enabled {
        unit("stop", &["local-voice-tts"])?;
    }
    if !tts_only || config.tts_enabled {
        keep_alive()?;
    }
    if !tts_only {
        select_asr(&config)?;
    }
    if !asr_only && config.tts_enabled {
        unit("start", &["local-voice-tts"])?;
    }
    if asr_only {
        return Ok(format!("识别已就绪 · {}", config.asr_mode));
    }
    if tts_only && !config.tts_enabled {
        return Ok("朗读已关闭，模型内存已释放；识别不受影响".into());
    }
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let mut urls = Vec::new();
    if !tts_only {
        urls.push(&config.asr_url);
    }
    if config.tts_enabled {
        urls.push(&config.tts_url);
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if cancelled() {
            return Ok("正在应用新的服务设置…".into());
        }
        let ready = urls.iter().all(|url| {
            client
                .get(format!("{url}/health"))
                .send()
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.json::<serde_json::Value>())
                .map(|v| v["ready"] == true)
                .unwrap_or(false)
        });
        if ready {
            return Ok(if config.tts_enabled {
                "朗读已就绪"
            } else {
                "识别已就绪 · 朗读已关闭"
            }
            .into());
        }
        if Instant::now() >= deadline {
            bail!("模型在两分钟内未就绪");
        }
        thread::sleep(Duration::from_millis(500));
    }
}
