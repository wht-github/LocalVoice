# 本地语音：Qwen ASR + SenseVoice

Windows 原生 Rust + Slint 悬浮窗，提供三种识别模型：

| 设置项 | 推理方式 | 用途 |
| --- | --- | --- |
| Qwen ASR 1.7B Q8 | 官方 llama.cpp，NVIDIA CUDA | 日常听写 |
| Qwen ASR 0.6B Q8 | 官方 llama.cpp，NVIDIA CUDA | 较小的模型和显存占用 |
| SenseVoice CPU | FunASR，CPU 版 PyTorch | 无需独立显卡 |

三种模式共用一个 Python 服务环境，一次加载一种模型。Qwen 使用官方 GGUF Q8 文本权重与 BF16 音频编码器，llama.cpp 使用官方 v0.4.0 对应的 **b10809 Windows CUDA 13.3 x64 预编译包**。不需要编译 llama.cpp，也不需要 CUDA Toolkit、CUDA PyTorch 或 WSL 来做识别。

## 安装与启动

前置：Windows x64、Python 3.12–3.13；Qwen 需要受支持的 NVIDIA 显卡和驱动。自己编译桌面客户端另需 Rust MSVC 工具链与 Visual Studio C++ 桌面开发组件。

在项目根目录的 PowerShell 7 中执行：

```powershell
./scripts/setup-native-asr.ps1
./scripts/setup-llama.ps1
$asrPython = Join-Path $env:LOCALAPPDATA 'LocalVoice/venv/Scripts/python.exe'
& $asrPython scripts/download-qwen-gguf.py --size 1.7B
& $asrPython scripts/download-qwen-gguf.py --size 0.6B
./desktop.ps1 build
./desktop.ps1
```

已有安装只需 `./desktop.ps1`。在设置中选择模型并保存，客户端负责停止旧模型、加载新模型。默认新配置使用 SenseVoice CPU，朗读关闭；保存过的模型选择和其他偏好继续生效。

- **Ctrl+Alt+Space**：开始／结束听写；可在设置中开启连续听写。
- 退出客户端会停止它启动的识别进程，隐藏到托盘则继续运行。
- 日志：`%LOCALAPPDATA%/LocalVoice/native-asr.log`；设置：同目录 `settings.json`。

安装细节与故障排查见 [模型部署](docs/qwen-llama-desktop.md)，窗口操作与验证见 [桌面客户端](desktop/README.md)。

## 文件与环境

| 位置 | 内容 |
| --- | --- |
| `%LOCALAPPDATA%/LocalVoice/venv` | 共用服务环境，SenseVoice CPU PyTorch + llama HTTP 适配器 |
| `.runtime/llama` | 官方 llama.cpp EXE、配套 CUDA DLL、版本记录 |
| `.runtime/models/Qwen3-ASR-{大小}-GGUF` | 两个 Qwen 模型各自的 Q8 权重和 BF16 音频编码器 |
| 用户 Hugging Face 缓存 | SenseVoiceSmall 模型，首次启动下载 |
| `desktop/target/release` | 编译后的悬浮窗 |

下载脚本固定版本并校验 SHA256。升级 llama.cpp 时明确更新版本与校验值，再运行三模型验收，避免启动时自动更新。

## 本机 API

- `GET http://127.0.0.1:8001/health`：模型、设备、ready 状态。
- `POST http://127.0.0.1:8001/v1/audio/transcriptions`：multipart `file`，`language=auto/zh/en`。
- 单次不超过 30 秒、20 MiB，支持 WAV/FLAC/OGG；忙碌时返回 429。

客户端处理麦克风、分段与输入文字；服务返回完整转写。录音和转写不会自动保存。

## 可选朗读

现有 MeloTTS 朗读仍可在设置中开启，使用专用 LocalVoice WSL 环境；它与上述识别部署独立，默认关闭。保留 `voice.ps1` 和相关 TTS 安装文件以维护现有功能，详见 [朗读部署记录](docs/melo-and-service-control.md)。`voice.ps1` 管理 WSL 服务，不是 Windows 识别的启动入口。

## 验证与历史

```powershell
# 先退出日常悬浮窗，释放 8001 端口
./desktop/target/release/local-voice-desktop.exe --asr-models-check tests/fixtures/ascend-01219.wav outputs/asr-models-check.json
```

该检查通过实际桌面管理路径切换 CPU → 0.6B → 1.7B → CPU，验证转写和停止，不录音、不输入文字。公开样例出处见 [样例说明](tests/fixtures/README.md)。它是部署检查，不是准确率 benchmark。

原 PyTorch／TileLang、量化及 Nsight 实验已从当前工作目录移除，保存在 Git 标签 `archive/asr-optimization-20260913`（提交 `a17b0a46`）。例如 `git show archive/asr-optimization-20260913:docs/qwen-asr-nsight.md` 可查看当时报告；旧报告中的耗时不代表当前官方版本。
