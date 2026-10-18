//! Installation checks inspect files only; they never start models or the microphone.
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::{os::windows::process::CommandExt, path::Path, process::Command};

#[derive(Serialize)]
pub struct Installation {
    pub root: std::path::PathBuf,
    pub runtime: std::path::PathBuf,
    pub portable: bool,
    pub setup_available: bool,
    pub basic_ready: bool,
    pub files_present: ModelFiles,
}

#[derive(Serialize)]
pub struct ModelFiles {
    pub sensevoice: bool,
    pub silero_vad: bool,
    pub llama_cpu: bool,
    pub llama_cuda: bool,
    pub agentic: bool,
    pub qwen_0_6b: bool,
    pub qwen_1_7b: bool,
}

fn present(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|entry| entry.is_file() && entry.len() > 0)
}

pub fn inspect() -> Result<Installation> {
    let root = crate::paths::root()?;
    let runtime = root.join(".runtime");
    let models = runtime.join("models");
    let qwen = |size| {
        let directory = models.join(format!("Qwen3-ASR-{size}-GGUF"));
        present(&directory.join(format!("Qwen3-ASR-{size}-Q8_0.gguf")))
            && present(&directory.join(format!("mmproj-Qwen3-ASR-{size}-bf16.gguf")))
    };
    let files_present = ModelFiles {
        sensevoice: present(&models.join("sensevoice-int8/model.int8.onnx"))
            && present(&models.join("sensevoice-int8/tokens.txt")),
        silero_vad: present(&models.join("silero-vad/silero_vad.onnx")),
        llama_cpu: present(&runtime.join("llama-cpu/llama-server.exe")),
        llama_cuda: present(&runtime.join("llama/llama-server.exe")),
        agentic: present(&models.join("AgenticASR-Refiner-GGUF/AgenticASR-Refiner-Q4_K_M.gguf")),
        qwen_0_6b: qwen("0.6B"),
        qwen_1_7b: qwen("1.7B"),
    };
    Ok(Installation {
        portable: crate::paths::is_portable(&root),
        setup_available: root.join("Setup Models.cmd").is_file(),
        basic_ready: files_present.sensevoice && files_present.silero_vad,
        root,
        runtime,
        files_present,
    })
}

pub fn check(output: &str) -> Result<()> {
    let installation = inspect()?;
    std::fs::write(output, serde_json::to_vec_pretty(&installation)?)?;
    Ok(())
}

pub fn setup_hint(source_hint: &str) -> String {
    if crate::paths::root().is_ok_and(|root| crate::paths::is_portable(&root)) {
        "请在设置中点击「安装 / 管理模型」，或双击程序目录的 Setup Models.cmd".into()
    } else {
        source_hint.into()
    }
}

pub fn open_setup() -> Result<()> {
    let root = crate::paths::root()?;
    if !crate::paths::is_portable(&root) {
        bail!(
            "源码版请运行 install.ps1 -CpuOnly；需要 Qwen 时运行 scripts/setup-llama.ps1 和 scripts/download-qwen-gguf.ps1 -Size 1.7B"
        );
    }
    if !root.join("Setup Models.cmd").is_file() {
        bail!("缺少 Setup Models.cmd；请重新完整解压发布包，保留已有 .runtime 文件夹");
    }
    // The fixed command name avoids interpreting the installation directory as
    // shell text. A new console lets the user choose and watch model downloads.
    Command::new("cmd.exe")
        .args(["/D", "/C"])
        .raw_arg("\"Setup Models.cmd\"")
        .current_dir(root)
        .creation_flags(0x00000010)
        .spawn()
        .context("无法打开模型安装窗口，请手动双击 Setup Models.cmd")?;
    Ok(())
}
