//! An owned llama.cpp process with a private, authenticated loopback connection.
use anyhow::{Context, Result, bail};
use reqwest::blocking::{Client, Response};
use serde_json::Value;
use std::{
    fs::OpenOptions,
    net::TcpListener,
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::HANDLE,
    System::{
        Com::CoCreateGuid,
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
    },
};

struct Process {
    child: Child,
    job: OwnedHandle,
}

pub struct Server {
    process: Mutex<Option<Process>>,
    client: Client,
    url: String,
    key: String,
}

impl Server {
    pub fn start(
        arguments: &[String],
        label: &str,
        load_timeout: Duration,
        cancelled: impl Fn() -> bool,
    ) -> Result<Self> {
        if cancelled() {
            bail!("模型加载已取消");
        }
        let runtime = crate::paths::runtime()?;
        let cpu = runtime.join("llama-cpu/llama-server.exe");
        let executable = if cpu.is_file()
            && arguments
                .windows(2)
                .any(|pair| pair[0] == "--device" && pair[1] == "none")
        {
            cpu
        } else {
            runtime.join("llama/llama-server.exe")
        };
        if !executable.is_file() {
            bail!("缺少 llama.cpp；请运行 scripts/setup-llama.ps1");
        }
        // Bind only to choose a free port. A race fails the authenticated handshake;
        // it never attaches to an already-running public model service.
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let key = unsafe { format!("{:?}{:?}", CoCreateGuid()?, CoCreateGuid()?) };
        let client = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .build()?;
        let job = create_job()?;
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(runtime.join(format!("native-{label}.log")))?;
        let mut command = Command::new(&executable);
        command
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .args(arguments)
            .env("LLAMA_API_KEY", &key)
            .env_remove("GGML_CUDA_DISABLE_GRAPHS")
            .env_remove("GGML_CUDA_GRAPH_OPT")
            .env_remove("GGML_CUDA_PDL")
            .current_dir(executable.parent().unwrap())
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        drop(listener);
        let mut child = command.spawn().context("无法启动 llama.cpp")?;
        if let Err(error) = unsafe {
            AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(child.as_raw_handle()))
        } {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error).context("无法为模型进程建立退出清理");
        }
        let server = Self {
            process: Mutex::new(Some(Process { child, job })),
            client,
            url: format!("http://127.0.0.1:{port}"),
            key,
        };
        let deadline = Instant::now() + load_timeout;
        loop {
            if cancelled() {
                bail!("模型加载已取消");
            }
            if !server.ready() {
                bail!("模型启动失败；请查看 .runtime/native-{label}.log");
            }
            if server
                .get("/health", Duration::from_millis(500))
                .is_ok_and(|value| value["status"] == "ok")
            {
                server.handshake()?;
                if cancelled() {
                    bail!("模型加载已取消");
                }
                return Ok(server);
            }
            if Instant::now() >= deadline {
                bail!("模型加载超时；请查看 .runtime/native-{label}.log");
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    fn handshake(&self) -> Result<()> {
        // /health is public in llama.cpp. Check an authenticated route, including
        // rejection without our key, before sending any recording or note.
        let anonymous = self
            .client
            .get(format!("{}/props", self.url))
            .timeout(Duration::from_secs(2))
            .send()?;
        if anonymous.status() != reqwest::StatusCode::UNAUTHORIZED {
            bail!("模型私有连接校验失败，请重试启动");
        }
        self.get("/props", Duration::from_secs(2))?;
        if !self.ready() {
            bail!("模型在连接校验期间退出");
        }
        Ok(())
    }

    pub fn ready(&self) -> bool {
        let mut process = self.process.lock().unwrap();
        process
            .as_mut()
            .is_some_and(|process| matches!(process.child.try_wait(), Ok(None)))
    }

    pub fn get(&self, path: &str, timeout: Duration) -> Result<Value> {
        if !self.ready() {
            bail!("模型已停止，请重新启动");
        }
        response_json(
            self.client
                .get(format!("{}{path}", self.url))
                .bearer_auth(&self.key)
                .timeout(timeout)
                .send()?,
        )
    }

    pub fn request(&self, path: &str, body: &Value, timeout: Duration) -> Result<Value> {
        if !self.ready() {
            bail!("模型已停止，请重新启动");
        }
        response_json(
            self.client
                .post(format!("{}{path}", self.url))
                .bearer_auth(&self.key)
                .timeout(timeout)
                .json(body)
                .send()?,
        )
    }

    /// May run concurrently with a request; stopping the owned job interrupts it.
    pub fn close(&self) -> Result<()> {
        let Some(mut process) = self.process.lock().unwrap().take() else {
            return Ok(());
        };
        if process.child.try_wait()?.is_some() {
            return Ok(());
        }
        unsafe { TerminateJobObject(HANDLE(process.job.as_raw_handle()), 0) }
            .context("无法停止模型进程树")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while process.child.try_wait()?.is_none() {
            if Instant::now() >= deadline {
                bail!("模型进程尚未退出");
            }
            thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("{error:#}");
        }
    }
}

fn create_job() -> Result<OwnedHandle> {
    let handle = unsafe { CreateJobObjectW(None, None)? };
    let job = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of_val(&limits) as u32,
        )?;
    }
    Ok(job)
}

fn response_json(response: Response) -> Result<Value> {
    let status = response.status();
    let value: Value = response.json().context("模型返回了无法解析的结果")?;
    if !status.is_success() {
        let detail = value["error"]["message"]
            .as_str()
            .unwrap_or("本地模型请求失败");
        bail!("{detail}（{status}）");
    }
    Ok(value)
}
