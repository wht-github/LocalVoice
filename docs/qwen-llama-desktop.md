# 桌面识别模型部署

## 固定的标准部署

- llama.cpp：[官方 v0.4.0](https://github.com/ggml-org/llama.cpp/releases/tag/v0.4.0) 指向 [b10809](https://github.com/ggml-org/llama.cpp/releases/tag/b10809)，本项目选择 Windows CUDA 13.3 x64 包和同版本 cudart 包。
- Qwen：ggml-org 发布的 [1.7B GGUF](https://huggingface.co/ggml-org/Qwen3-ASR-1.7B-GGUF) 与 [0.6B GGUF](https://huggingface.co/ggml-org/Qwen3-ASR-0.6B-GGUF)，各保留 Q8_0 文本模型、BF16 音频编码器。
- SenseVoice：[SenseVoiceSmall](https://huggingface.co/FunAudioLLM/SenseVoiceSmall)，FunASR 1.4.14，CPU 版 PyTorch。

安装命令见 [首页](../README.md)。`setup-llama.ps1` 下载原始官方 ZIP、校验官方 release 给出的 SHA256，将程序与 CUDA DLL 放在同一目录；不配置全局 PATH、不安装 CUDA Toolkit、不调用 MSVC/NVCC。包内程序显示 `0.4.0-dev (build 10809, commit 5266f24da)`，这是官方包自身的版本字符串。

模型下载脚本也固定 Hugging Face revision、大小和 SHA256，已下载文件会先校验后复用，不重新下载。两个模型分别放在 `.runtime/models/Qwen3-ASR-0.6B-GGUF` 和 `.runtime/models/Qwen3-ASR-1.7B-GGUF`；各目录的 `manifest.json` 记录来源。

## 运行过程

Slint 启动 `%LOCALAPPDATA%/LocalVoice/venv/Scripts/python.exe` 运行 `asr_server.py`。SenseVoice 在此进程加载 CPU 模型；Qwen 通过 `llama_asr.py` 启动 `.runtime/llama/llama-server.exe`，Python 不加载 Qwen 或 CUDA PyTorch。

适配器保持上下文 2048、batch/ubatch 256、单并发、4 线程、F16 KV、Flash Attention 开启。文本权重与音频编码器在 CUDA0 上运行。请求不复用 prompt cache，温度 0，最多 512 输出 token；超过限制会提示缩短录音。

llama-server 使用临时回环端口与每次启动生成的 API key，密钥只在子进程环境中传入。Windows Job 负责在适配器意外退出时清理模型子进程，避免显存残留。桌面切换模型会先停止旧进程树。

## 验收

关闭日常客户端后运行首页的 `--asr-models-check`。覆盖四次加载：SenseVoice CPU → Qwen 0.6B → Qwen 1.7B → SenseVoice CPU，逐次检查设备、非空转写、重复启动和最终停止。还可运行以下异常退出检查：

```powershell
$asrPython = Join-Path $env:LOCALAPPDATA 'LocalVoice/venv/Scripts/python.exe'
& $asrPython scripts/check-llama-cleanup.py
```

测试 WAV 是两秒公开真人中文录音，仅作为链路验收；没有据此承诺识别准确率或跨版本性能。

2026-09-13 本机迁移验收：22 项桌面单元测试、release 构建通过；上述四次模型切换均返回非空转写，设备与设置一致，最终停止后 8001 端口释放。报告在 `outputs/asr-models-check.json`。两个 Qwen 模型都返回“就我们会谈到，就是。”，SenseVoice 返回“就我们会谈到就是。”。

## 排查

- 模型缺失：重新运行对应大小的 `download-qwen-gguf.py`。
- EXE 或 CUDA DLL 缺失：停止客户端后重新运行 `setup-llama.ps1`。
- CUDA 初始化失败：查看 `native-asr.log`，运行 `.runtime/llama/llama-server.exe --list-devices` 核对 NVIDIA 设备与驱动；无合适 GPU 时选择 SenseVoice CPU。
- 提示缺少 VC++ DLL：安装 Microsoft Visual C++ x64 Redistributable；使用预编译 llama.cpp 不要求安装 Visual Studio 编译器。
- 8001 端口被旧服务占用：从启动该服务的程序停止它，再启动悬浮窗。
- 新机器首次 SenseVoice 加载较慢：等待模型下载和预热；后续复用用户缓存。

本轮移除了 CUDA PyTorch、TileLang、Nsight、本地 llama 源码编译路径与量化对照。系统 Visual Studio、NVIDIA 驱动、其他项目和可选朗读部署不属于清理范围。Git 已记录的实验代码和汇总结果可从 `archive/asr-optimization-20260913` 恢复；未进 Git 的旧原始 trace、日志和评测缓存已按用户确认永久删除，标签不包含它们。
