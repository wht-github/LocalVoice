# Windows 安装指南

默认下载 GitHub Release 中已编译的 Slint 悬浮窗和 llama.cpp 官方程序。用户无需编译，不需要 Rust、Visual Studio、CUDA Toolkit 或 WSL。

## 1. 自行准备前置环境

- Windows x64、Git、PowerShell 7，以及可用的 `curl.exe`。
- 自行安装 [uv](https://docs.astral.sh/uv/getting-started/installation/) 和 Python 3.12。已经有 Python 3.13 的用户也可以自行创建 `.venv`，见下一节。
- 使用 Qwen 时需要支持当前 CUDA 13.3 官方包的 NVIDIA 显卡与驱动；没有合适显卡可只安装 SenseVoice CPU。
- 完整安装建议预留 15 GB 磁盘空间，包含模型、Python 依赖和下载缓存。两个 Qwen 模型及其音频编码器合计约 4 GB。

在 PowerShell 7 中确认：

```powershell
git --version
uv --version
uv python install 3.12
uv python find 3.12 --no-python-downloads
```

`uv python install` 是用户准备环境的步骤。项目安装脚本不会安装 uv、下载 Python 或修改全局 PATH；它只使用已有解释器。

## 2. Clone 并安装

```powershell
git clone https://github.com/wht-github/LocalVoice.git
cd LocalVoice
./install.ps1
./desktop.ps1
```

安装脚本依次完成：

1. 用已有 Python 3.12 创建项目 `.venv`，通过 uv 安装 CPU 版 PyTorch 和识别服务依赖。
2. 根据 `desktop/Cargo.toml` 的版本下载配套 Slint Release，并校验 SHA256。
3. 下载固定版本的 SenseVoiceSmall 模型到仓库缓存。
4. 下载并校验官方 llama.cpp、CUDA DLL、Qwen ASR 1.7B Q8 和 0.6B Q8，以及各自的 BF16 音频编码器。

如果希望使用已有 Python 3.13，请在执行 `install.ps1` 前运行：

```powershell
uv venv --python 3.13 --no-python-downloads .venv
```

无需激活 `.venv`。项目安装和启动均使用它的绝对路径，不会安装到当前终端激活的其他项目环境中。

仅安装 CPU 模式：

```powershell
./install.ps1 -CpuOnly
./desktop.ps1
```

这种安装请在设置中保持 SenseVoice CPU；之后运行不带 `-CpuOnly` 的安装命令即可补齐两个 Qwen 模型。三种模式一次只加载一种。

若 PowerShell 阻止脚本运行，可仅对本次进程放行：

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File ./install.ps1
pwsh -NoProfile -ExecutionPolicy Bypass -File ./desktop.ps1
```

## 3. 首次使用

新配置默认使用 SenseVoice CPU，朗读关闭。等待窗口显示识别已就绪，在设置中选择需要的模型并保存。Qwen 模式会停止旧服务后加载 GPU 模型。

按 **Ctrl+Alt+Space** 开始说话，再次按下结束，识别结果输入当前目标窗口。也可以在设置中开启连续听写。窗口收起后仍在托盘运行；从托盘退出会停止它启动的识别进程。

## 4. 本地文件布局

| 路径（相对 clone 根目录） | 内容 |
| --- | --- |
| `.venv/` | 项目 Python 环境，使用用户已有 uv Python |
| `.runtime/desktop/` | 下载的 Slint EXE、构建信息、第三方许可 |
| `.runtime/llama/` | 官方 llama.cpp 程序与 CUDA DLL |
| `.runtime/models/` | 两套 Qwen GGUF 权重 |
| `.runtime/cache/` | SenseVoice、uv 包、音频处理缓存 |
| `.runtime/downloads/` | 原始 ZIP 与校验文件 |
| `.runtime/settings.json` | 模型、麦克风、快捷键等设置 |
| `.runtime/native-asr.log` | 识别服务日志 |
| `.runtime/timings.jsonl` | 有大小上限的耗时记录，不含转写正文 |

项目安装内容保留在 clone 目录中。uv 和基础 Python 由用户管理，项目 `.venv` 仍依赖该基础 Python；不要删除基础解释器。移动整个仓库后，应重建 `.venv`，其他模型与设置可以保留。不要单独把 EXE 移出仓库，程序通过 EXE 的位置寻找服务脚本和环境。

## 5. 更新与排查

先从托盘退出，再更新：

```powershell
git pull --ff-only
./install.ps1
./desktop.ps1
```

使用过 `-CpuOnly` 且仍只需要 CPU 的用户，更新时继续加该参数。已存在的模型会校验并复用，设置不会被覆盖。只需重新安装客户端时可执行 `./scripts/setup-desktop.ps1`。

- 缺少 uv 或 Python：完成第一节前置步骤后重试。安装脚本不会替你下载解释器。
- Python 包下载慢：运行 `./install.ps1 -PyIndex https://pypi.tuna.tsinghua.edu.cn/simple`；CPU PyTorch 仍来自官方 wheel 源。
- 模型连接失败：可先设置 `$env:HF_ENDPOINT = 'https://huggingface.co'` 或可用的 Hugging Face 镜像地址，再执行安装。SenseVoice 默认使用 `https://hf-mirror.com`，Qwen 默认使用官方站点；该变量对两者都生效。
- Release 下载失败：检查 [项目 Releases](https://github.com/wht-github/LocalVoice/releases) 与网络。每个源码版本固定配套客户端版本，不从源码自动编译。
- 提示缺少 VC++ DLL：安装 [Microsoft VC++ x64 运行库](https://aka.ms/vc14/vc_redist.x64.exe)，无需安装 Visual Studio。
- GPU 加载失败：查看 `.runtime/native-asr.log`，执行 `.runtime/llama/llama-server.exe --list-devices`；检查 NVIDIA 驱动或先使用 SenseVoice CPU。
- 端口 8001 被占用：从启动旧服务的程序停止它，避免同时运行多个 clone 的客户端。

部署验收（先退出日常客户端）：

```powershell
./.runtime/desktop/local-voice-desktop.exe --asr-models-check tests/fixtures/ascend-01219.wav outputs/asr-models-check.json
```

此检查需要完整安装，会切换 CPU → 0.6B → 1.7B → CPU 并转写两秒公开真人录音，不采集麦克风、不输入文字。它验证部署链路，不代表准确率评测。

朗读是独立的可选 WSL MeloTTS 功能，本安装流程不安装，默认关闭。需要时参阅 [朗读部署记录](docs/melo-and-service-control.md)。开发者修改客户端源码的构建方式见 [desktop/README.md](desktop/README.md)，普通安装无需此步骤。
