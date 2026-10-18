# Windows 安装指南

v0.4.0 的录音、Silero VAD、SenseVoice 识别、Qwen ASR 和 Agentic 整理均通过 Rust 与原生运行库完成。安装和运行不使用 Python、uv、pip、PyTorch 或 `.venv`。

**默认安装 [v0.4.0 预编译 Release](https://github.com/wht-github/LocalVoice/releases/tag/v0.4.0)，无需 Rust 或 C++ 开发环境。** 只有显式传入 `-LocalBuild` 时才构建源码。

## 1. 前置环境

- Windows x64、Git、PowerShell 7，系统自带的 `curl.exe` 与 `tar.exe`。
- 仅源码构建需要 Rust MSVC 工具链与 Visual Studio C++ Build Tools。预编译包已包含 sherpa-onnx 与 ONNX Runtime DLL。
- 使用 Qwen GPU 识别需要支持所固定 CUDA 13.3 运行库的 NVIDIA 驱动。只用 CPU 可选择下方的 `-CpuOnly`，无需 NVIDIA 显卡。
- 需要时安装 [Microsoft VC++ x64 运行库](https://aka.ms/vc14/vc_redist.x64.exe)。无需 CUDA Toolkit、WSL 或 Python。

## 2. 安装 v0.4.0

```powershell
git clone --branch v0.4.0 https://github.com/wht-github/LocalVoice.git
cd LocalVoice
./install.ps1
./desktop.ps1
```

安装脚本按仓库版本下载客户端 ZIP 和 SHA256 校验文件，校验后安装到 `.runtime/desktop`，并准备 Silero VAD、SenseVoice INT8、llama.cpp、Qwen ASR 1.7B 与 0.6B 及其音频编码器。已有完整模型与已验证下载包会复用，不重复下载。Qwen 两套模型与编码器合计约 4 GB。

只用 CPU：

```powershell
./install.ps1 -CpuOnly
./desktop.ps1
```

CPU 安装包含 VAD、SenseVoice INT8 与独立的 llama.cpp CPU 包，跳过 Qwen 下载。CPU 包保存在 `.runtime/llama-cpu`，不覆盖已安装的 CUDA 包；Agentic 整理会优先使用它。

已经有 Qwen 和 Agentic 的用户，在原仓库目录中升级；先从托盘退出旧客户端：

```powershell
git fetch --tags
git switch --detach v0.4.0
./install.ps1
./desktop.ps1
```

已有 Qwen、Agentic 权重和 `.runtime/settings.json` 会保留，旧模型与研究文件不会被删除。只用 CPU 的用户继续传 `-CpuOnly`，并在设置中选择 SenseVoice CPU；安装不会改写已保存的模型选择。旧版 SenseVoice 的 Python 权重不能替代本版原生 INT8 权重，因此首次升级需要补充原生 SenseVoice 和 Silero 文件。

需要从源码构建时，在项目根目录执行：

```powershell
./install.ps1 -LocalBuild
./desktop.ps1 -LocalBuild
```

也可组合 `-LocalBuild -CpuOnly`。构建时 Cargo 会下载固定版本的原生运行库，并将 DLL 放到 EXE 同目录；首次构建需要联网和额外的构建缓存空间。

运行中的客户端必须先从托盘退出；安装脚本会在更新前检查本项目的下载版与本地构建版，检测到仍在运行便停止安装。`desktop.ps1 -LocalBuild` 启动 `desktop/target/release` 中的构建版；不带参数启动 `.runtime/desktop` 中的 Release。`install.ps1 -Prebuilt` 保留为默认安装方式的兼容写法，不能与 `-LocalBuild` 同时使用。

## 3. 首次使用

启动后可在设置中选择 SenseVoice CPU、Qwen ASR 0.6B 或 1.7B。三者一次只加载一种。单击悬浮球展开操作面板，按 **Ctrl+Alt+Space** 开始说话，再次按下结束；也可点击面板上的开始、结束按钮。

Silero VAD 只判断音频是否有人声，用来减少纯噪声触发并保留轻声；约 629 KiB 的模型使用 CPU。日常输入和笔记仍由你手动结束，内部停顿不会提交。可选连续听写仅用于原样转写，停顿后提交分段结果。

日常输入和笔记都会在手动结束后识别、整理完整正文，再自动输入结束时的目标。等待期间目标变更时保留文字供复制。笔记对照窗口默认关闭，可在设置中打开；最近原文与正文仍可从面板手动查看。

Agentic 整理模型不随 `install.ps1` 下载。在设置中点击“准备整理模型”，额外下载约 688 MB 的 AgenticASR-Refiner Q4_K_M，固定版本并校验 SHA256。已有权重复用，CPU 4 线程、4096 上下文、0 GPU 层；按需加载，空闲 60 秒释放。整理失败或超过完整输出预算时保留全部原文。无需手动维护术语表即可使用。

## 4. 文件布局

| 路径 | 内容 |
| --- | --- |
| `desktop/target/release/` | 本地构建的 EXE、sherpa 与 ONNX Runtime DLL |
| `.runtime/desktop/` | 下载的预编译 Release 与第三方许可 |
| `.runtime/llama/` | CUDA 版 llama.cpp 与 DLL，供 Qwen 使用 |
| `.runtime/llama-cpu/` | 可选 CPU 版 llama.cpp，供 Agentic 使用 |
| `.runtime/models/silero-vad/` | 轻量人声检测模型 |
| `.runtime/models/sensevoice-int8/` | 原生 SenseVoice INT8 与词元表 |
| `.runtime/models/` | Qwen 与 Agentic GGUF 权重 |
| `.runtime/downloads/` | 已下载的原始安装包 |
| `.runtime/settings.json` | 麦克风、输出模式、快捷键等设置 |
| `.runtime/native-asr.log`、`.runtime/native-text.log` | 原生模型进程日志 |
| `.runtime/timings.jsonl` | 有大小上限的耗时记录，不含转写正文 |

保留 EXE 与同目录 DLL，并保留整个仓库布局。程序按 EXE 位置寻找资源，不依赖当前终端目录。旧 `.venv`、Python 服务和研究脚本不再属于应用运行链路；无需卸载系统 Python，也不自动删除历史实验文件。

## 5. 更新与排查

退出托盘客户端后，切换到目标版本的 Git 标签并重新运行 `install.ps1`，然后使用 `desktop.ps1`。本地源码构建才在安装和启动时都加 `-LocalBuild`；仅 CPU 安装继续传 `-CpuOnly`。模型和保存的设置会保留。

- 缺少 `cargo` 或 C++ 链接器：不带 `-LocalBuild` 安装预编译版，或安装源码构建工具；不需要补 Python。
- 缺少 `sherpa-onnx-c-api.dll`、`onnxruntime.dll`：保留 EXE 同目录全部 DLL；重新构建或重新安装完整原生 Release，不单独复制 EXE。
- VAD / SenseVoice 资源缺失：运行 `./scripts/setup-speech.ps1`。仅补 VAD 可加 `-VadOnly`。
- Qwen 模型缺失：运行 `./scripts/download-qwen-gguf.ps1 -Size 1.7B` 或 `-Size 0.6B`。
- Agentic 缺少 llama.cpp：CPU 机器运行 `./scripts/setup-llama.ps1 -CpuOnly`；使用 Qwen 的机器运行不带参数的版本。
- Hugging Face 连接失败：可设置 `$env:HF_ENDPOINT` 为可访问的镜像，再下载 Qwen 或准备 Agentic。VAD 与 SenseVoice 安装包来自 sherpa-onnx 官方 GitHub Release。
- GPU 加载失败：查看原生日志，运行 `.runtime/llama/llama-server.exe --list-devices` 检查驱动；也可切换 SenseVoice CPU。
- 新旧版本混淆：先退出旧客户端，按上面的升级流程安装 v0.4.0，再运行 `./desktop.ps1`。使用本地构建时才加 `-LocalBuild`；旧 v0.3.0 预编译包不包含本轮原生改造。

开发验证与窗口诊断见 [desktop/README.md](desktop/README.md)。维护者提交源码后用 `./scripts/package-desktop.ps1` 打包，产物包含 EXE、全部原生 DLL、构建信息、第三方许可和 SHA256 校验文件，整个过程不需要 Python。
