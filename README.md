# 本地语音输入

Windows 原生 Rust + Slint 悬浮球，用本地模型完成识别、日常轻整理和笔记正文输入。**v0.4.0 的运行与安装流程不需要 Python、pip、uv、PyTorch 或 WSL。** [下载与版本说明](https://github.com/wht-github/LocalVoice/releases/tag/v0.4.0)。

| 功能 | 当前实现 | 资源策略 |
| --- | --- | --- |
| 人声检测 | Silero VAD，经 sherpa-onnx / ONNX Runtime | 629 KiB 模型，CPU 单线程，录音时加载 |
| 中英混合识别 | Qwen ASR 1.7B / 0.6B Q8，llama.cpp CUDA | 一次加载一个识别模型 |
| CPU 识别 | SenseVoice INT8，经 sherpa-onnx | 无需独立显卡，无 Python |
| 轻整理、笔记 | AgenticASR-Refiner Q4_K_M，llama.cpp CPU | 688 MB 权重，4 线程，4096 上下文，空闲 60 秒卸载 |
| 界面与输入 | Rust + Slint、CPAL、Windows UI Automation | 原生窗口，无 WebView |

Qwen 和 Agentic 由 Rust 直接启动独立 llama.cpp 进程，使用私有随机回环端口；切换、停止和退出仅清理本应用拥有的模型。SenseVoice 与 Silero 直接在客户端内运行。具体结构及验证见 [原生运行说明](docs/native-runtime.md)。

## 使用方式

- **Ctrl+Alt+Space** 开始／结束录音。日常输入和笔记始终由你手动结束，完成识别和整理后自动输入。等待期间目标发生变化时保留结果供复制。
- **原样转写**保留口述；**日常输入**做轻整理；**笔记**整理连贯完整正文，不以摘要为目标。模型仍可能改错数字或术语，最近结果保留完整原文供核对、复制或重新整理。
- 默认不开启停顿提交。可在设置勾选“连续听写（仅原样转写，停顿后输入）”；日常输入和笔记继续手动结束。
- Silero 用人声模型代替固定音量门限，不再先按音量丢掉轻声。手动录音保留内部停顿，首音前留 400 ms 缓冲，末尾保留约 250 ms。VAD 不是降噪器，真实轻声、键盘声和房间噪声仍需实际试用。
- 笔记默认不弹对照窗口；设置中可开关“笔记完成后显示对照窗口（调试）”。这不影响自动输入，也可随时打开“最近结果”。
- 悬浮球与操作面板置顶、不抢输入焦点；托盘可找回窗口、打开设置或退出。

无需维护词表即可使用。可选专名只供 Qwen 识别参考，Agentic 不用词表替换正文。文字整理使用原作者协议，不为固定例句追加纠错规则；失败、超长或输出截断时回退完整转写。

## 安装与启动

在 Windows x64 的 PowerShell 7 中执行：

```powershell
git clone https://github.com/wht-github/LocalVoice.git
cd LocalVoice
./install.ps1
./desktop.ps1
```

默认安装校验过的预编译 Release，已有 Qwen、Agentic 权重会复用。无 NVIDIA 显卡可用 `./install.ps1 -CpuOnly`。源码构建使用 `./install.ps1 -LocalBuild`，需要 Rust MSVC 和 Visual Studio C++ 构建工具；随后用 `./desktop.ps1 -LocalBuild` 启动。**普通安装不需要这些开发工具或 Python**。详细升级步骤见 [install.md](install.md)。

打开设置选择识别模型。Agentic 缺失时，点击“下载并准备整理模型”才会下载；后续可离线使用。日常与笔记共用权重，按需加载，空闲释放。前轮本机整理模型工作集约 1.20 GiB，模型文件大小不等于运行内存；新的原生回环结果见验证报告。

停止全部模型或退出客户端会释放本版本的模型。旧版客户端及其 Python 服务需要从旧版托盘退出；新版不会接管或结束其他进程。不要同时运行两个客户端，以免快捷键冲突或同时占用识别模型显存。

## 文件与数据

| 位置 | 内容 |
| --- | --- |
| `desktop/src` | 录音、VAD、识别、整理、输入和原生模型生命周期 |
| `.runtime/settings.json` | 模型、模式、快捷键、麦克风及界面偏好 |
| `.runtime/models/silero-vad` | 轻量人声检测权重 |
| `.runtime/models/sensevoice-int8` | 原生 CPU 识别权重 |
| `.runtime/models/Qwen3-ASR-*-GGUF` | Qwen Q8 权重、BF16 音频编码器 |
| `.runtime/models/AgenticASR-Refiner-GGUF` | 文字整理权重 |
| `.runtime/llama`、`.runtime/llama-cpu` | 官方 llama.cpp 及配套 DLL |
| `desktop/target/release` | 本地构建的 EXE、sherpa 和 ONNX Runtime DLL |
| `.runtime/native-asr.log`、`.runtime/native-text.log` | 模型进程诊断日志 |

录音和转写不自动保存，最近结果仅保留在内存中。历史 Python 服务、下载脚本和评测代码保留用于研究复现，**不参与新版启动、录音、识别或整理**。旧的 8001 HTTP API 不是当前应用接口。

## 验证

```powershell
cargo test --manifest-path desktop/Cargo.toml --locked
./desktop/target/release/local-voice-desktop.exe --vad-check outputs/native-vad-check.json
./desktop/target/release/local-voice-desktop.exe --native-smoke tests/fixtures/ascend-01219.wav outputs/native-smoke.json
```

VAD 检查比较真实公开语音、合成衰减、静音和合成噪声，并与旧 RMS 门限对照。原生回环检查使用公开录音，验证 SenseVoice、Qwen 0.6B、Agentic 和进程退出；不采集麦克风、不输入外部应用。这些是回归与部署检查，不是准确率排名；自然轻声和实际应用输入需结合试用验证。

窗口及其他开发检查见 [桌面说明](desktop/README.md)。产品方向见 [产品规划](docs/product-roadmap.md)，历史模型比较见 [口述评测](docs/dictation-evaluation.md) 与 [识别引擎选型](docs/asr-engine-selection.md)。

原 PyTorch / TileLang / Nsight 实验位于 Git 标签 `archive/asr-optimization-20260913`。可选 TTS 已移除，相关记录保留在历史提交。

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)
