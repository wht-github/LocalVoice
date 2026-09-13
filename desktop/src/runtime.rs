//! Host-side control: native Windows ASR child process plus the optional WSL TTS unit.
//! No management HTTP server. The dictation path never touches WSL.
use crate::config::Config;
use anyhow::{Context, Result, bail};
use std::{
    fs::OpenOptions,
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub enum Operation {
    Start(Config),
    Stop,
    Tts(Config),
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

// ---- native Windows ASR child process ----

struct NativeAsr {
    child: Option<Child>,
    mode: String,
}
static NATIVE: OnceLock<Mutex<NativeAsr>> = OnceLock::new();
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
// Whether this process has started the WSL TTS unit, so Stop avoids booting a
// dormant distro just to run `systemctl stop`.
static TTS_ACTIVE: AtomicBool = AtomicBool::new(false);

fn native_health(config: &Config) -> Option<serde_json::Value> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    client
        .get(format!("{}/health", config.asr_url.trim_end_matches('/')))
        .send()
        .ok()?
        .json::<serde_json::Value>()
        .ok()
}
fn stop_native_child(child: &mut Child) -> Result<()> {
    if child.try_wait()?.is_none() {
        // Windows venv python.exe can be a launcher with a second Python child.
        // Terminating only the launcher leaves the model holding the port/GPU.
        let result = Command::new("taskkill.exe")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x08000000)
            .output()
            .context("无法停止识别进程树")?;
        if !result.status.success() && child.try_wait()?.is_none() {
            bail!(
                "无法停止识别进程树：{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
    child.wait()?;
    Ok(())
}
fn native_start(config: &Config) -> Result<()> {
    let root = crate::paths::root()?;
    let script = root.join("asr_server.py");
    let python = root.join(".venv/Scripts/python.exe");
    if !python.is_file() {
        bail!(
            "识别环境未安装（{}）；请运行对应的 setup 安装脚本",
            python.display()
        );
    }
    let mut guard = NATIVE
        .get_or_init(|| {
            Mutex::new(NativeAsr {
                child: None,
                mode: String::new(),
            })
        })
        .lock()
        .unwrap();
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        bail!("应用正在退出");
    }
    let same_mode = guard.mode == config.asr_mode;
    if let Some(child) = guard.child.as_mut() {
        if child.try_wait()?.is_none() {
            if same_mode {
                return Ok(());
            }
            stop_native_child(child).context("无法停止旧识别模型")?;
        }
    }
    guard.child = None;
    // Another server may already own the port (e.g. WSL services from an old setup).
    if let Some(health) = native_health(config) {
        if health["ready"] == true {
            if health["mode"] == config.asr_mode {
                return Ok(());
            }
            bail!(
                "识别端口被其他进程占用（{} 模式）；请先从启动它的程序停止服务",
                health["mode"]
            );
        }
    }
    let log_dir = crate::paths::runtime()?;
    let _ = std::fs::create_dir_all(&log_dir);
    let open_log = || {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("native-asr.log"))
            .map(Stdio::from)
            .unwrap_or(Stdio::null())
    };
    let log_out = open_log();
    let log_err = open_log();
    let mut command = Command::new(&python);
    command
        .arg("-u")
        .args(["-X", "utf8"])
        .arg(&script)
        .env("ASR_BACKEND", &config.asr_mode)
        .env("ASR_THREADS", "4")
        .env(
            "HF_ENDPOINT",
            std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://hf-mirror.com".into()),
        )
        .env("PYTHONUNBUFFERED", "1")
        .current_dir(script.parent().unwrap_or(std::path::Path::new(".")))
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(log_out)
        .stderr(log_err);
    let child = command
        .spawn()
        .with_context(|| format!("无法启动原生识别进程（{}）", python.display()))?;
    guard.child = Some(child);
    guard.mode = config.asr_mode.clone();
    Ok(())
}
pub fn native_stop() {
    if let Some(lock) = NATIVE.get() {
        if let Ok(mut guard) = lock.lock() {
            if let Some(mut child) = guard.child.take() {
                if let Err(error) = stop_native_child(&mut child) {
                    eprintln!("{error:#}");
                    guard.child = Some(child);
                }
            }
        }
    }
}

pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    native_stop();
}

// ---- optional WSL TTS ----

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
        .context("无法连接 LocalVoice WSL（朗读功能需要）；请确认专用 WSL 已安装")?;
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
fn tts_start() -> Result<()> {
    keep_alive()?;
    unit("start", &["local-voice-tts"])?;
    TTS_ACTIVE.store(true, Ordering::SeqCst);
    Ok(())
}
fn tts_stop() {
    if TTS_ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = unit("stop", &["local-voice-tts"]);
    }
}

pub fn managed(config: &Config) -> bool {
    config.asr_url.trim_end_matches('/') == "http://127.0.0.1:8001"
        && config.tts_url.trim_end_matches('/') == "http://127.0.0.1:8002"
}
pub fn apply(operation: Operation, cancelled: impl Fn() -> bool) -> Result<String> {
    let (config, tts_only) = match operation {
        Operation::Stop => {
            native_stop();
            tts_stop();
            return Ok("识别、朗读均已停止，模型内存已释放".into());
        }
        Operation::Start(c) => (c, false),
        Operation::Tts(c) => (c, true),
    };
    if !managed(&config) {
        bail!("自定义地址请自行管理；一键启停仅支持默认本机地址");
    }
    if tts_only {
        if config.tts_enabled {
            tts_start()?;
            return Ok("朗读已就绪；识别不受影响".into());
        }
        tts_stop();
        return Ok("朗读已关闭，模型内存已释放；识别不受影响".into());
    }
    native_start(&config)?;
    if config.tts_enabled {
        tts_start()?;
    } else {
        tts_stop();
    }
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let mut urls = vec![config.asr_url.trim_end_matches('/')];
    if config.tts_enabled {
        urls.push(config.tts_url.trim_end_matches('/'));
    }
    // The first native start downloads the SenseVoice model, which can take minutes.
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        if cancelled() {
            return Ok("正在应用新的服务设置…".into());
        }
        if let Some(lock) = NATIVE.get() {
            let mut guard = lock.lock().unwrap();
            if let Some(child) = guard.child.as_mut() {
                if let Some(status) = child.try_wait()? {
                    bail!("识别进程启动失败（{status}）；请查看 .runtime/native-asr.log");
                }
            }
        }
        let ready = urls.iter().all(|url| {
            client
                .get(format!("{url}/health"))
                .send()
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.json::<serde_json::Value>())
                .map(|v| {
                    v["ready"] == true
                        && (*url != config.asr_url.trim_end_matches('/')
                            || v["mode"] == config.asr_mode)
                })
                .unwrap_or(false)
        });
        if ready {
            let name = crate::config::ASR_MODES
                .iter()
                .find(|(mode, _)| *mode == config.asr_mode)
                .map(|(_, label)| *label)
                .unwrap_or("未知模型");
            return Ok(format!(
                "识别已就绪 · {name}；朗读{}",
                if config.tts_enabled {
                    "已开启"
                } else {
                    "已关闭"
                }
            ));
        }
        if Instant::now() >= deadline {
            bail!("服务在十分钟内未就绪；首次启动需下载模型，请查看 native-asr.log");
        }
        thread::sleep(Duration::from_millis(500));
    }
}
