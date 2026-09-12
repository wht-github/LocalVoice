# Slint 中使用 llama.cpp Qwen ASR

2026-09-12。设置 → 识别模型 → 选择后保存；正在听写时先结束当前录音。以下模式均在 Windows 本机运行：

| 选项 | 实际模型与设备 | 用途 |
| --- | --- | --- |
| SenseVoice · CPU | SenseVoiceSmall / FunASR，CPU | 留出 NVIDIA 显存 |
| llama · Qwen ASR 0.6B Q8 | Q8_0 文本模型 + BF16 音频编码器，CUDA | 较小的 Qwen 模型 |
| llama · Qwen ASR 1.7B Q8 | Q8_0 文本模型 + BF16 音频编码器，CUDA | 本次指定的日常试用配置 |
| PyTorch · Qwen ASR 0.6B | 原 Transformers / PyTorch CUDA 路径 | 保留已有设置与实验对照 |

原来的默认 CPU 和已保存的选择不变。本次没有将 1.7B 换成研究中的 Q4；两个 Qwen llama 选项明确使用 Q8。新选项不会自动下载权重；缺少文件时启动失败，日志给出安装文档位置。

## 安装与启动

本机已有 `.venv-qwen`、llama.cpp CUDA 构建与两个模型。换电脑时，先按 [CUDA 环境说明](llama-native-windows.md) 安装并构建 llama.cpp，按 [原生 Python 环境说明](qwen-native-windows.md) 准备 `.venv-qwen`。llama 模式仅复用其中的 HTTP、音频处理依赖，不加载 PyTorch 模型。

在仓库根目录执行：

```powershell
.venv-qwen/Scripts/python.exe scripts/download-qwen-gguf.py --size 0.6B
.venv-qwen/Scripts/python.exe scripts/download-qwen-gguf.py --size 1.7B
./desktop.ps1 build
./desktop.ps1
```

下载脚本固定上游版本并检查 SHA256。0.6B 来自 `ggml-org/Qwen3-ASR-0.6B-GGUF` 的 `928ab958557df9aa2ef1c93e0e83c7ad0933fae2`，两个文件约 1.18 GB；1.7B 来自 `36a678687ba7d07a74ca70ccb0e36902e005fb80`，两个文件约 2.81 GB。这是磁盘大小，不是峰值显存。

无需另开终端或先进入 CUDA 编译环境。桌面启动 Python 适配器，适配器设置项目内 CUDA DLL 路径，再启动本地 llama-server。识别接口仍是 `127.0.0.1:8001/v1/audio/transcriptions`，内部 llama-server 使用临时回环端口和每次启动生成的密钥。

保存另一模型、停止服务或正常退出时，桌面停止自己启动的识别进程树，覆盖 Windows venv 启动器的额外 Python 子进程。适配器用 Windows Job Object 管理 llama-server，即使适配器被强制终止，也会清理它的 llama 子进程。不会停止其他程序启动的识别服务。

日志在 `%LOCALAPPDATA%/LocalVoice/native-asr.log`。识别无须 WSL；朗读仍遵循原有开关。

SenseVoice 优先使用已有的 Hugging Face 本地快照；仅首次安装无缓存时才走下载。退出窗口事件循环前同步清理识别进程，并阻止尚在排队的启动操作重新加载模型。

## 本次推理配置

单并发、上下文 2048、batch/ubatch 256、4 个 CPU 线程，全部可卸载文本层及音频编码器使用 CUDA，Flash Attention 和 CUDA Graph 开启；KV 为 F16。每次音频独立请求，关闭 prompt cache；最多输出 512 token，超限报告错误，不静默截断。数字零音频直接返回空文本。

上下文比量化实验的 1024 大，用于桌面最多 30 秒的请求；因此不能把那份实验的耗时和显存数字直接当作桌面版本实测。客户端仍在停顿或手动结束后提交整段音频，这次没有加入模型流式识别。

桌面使用自动语言检测。API 的 `zh/en` 通过模型的语言前缀提示实现，并不保证翻译或强制改变录音语言；普通听写保持 `auto`。

## 验证入口

先退出日常客户端，让出 8001 端口。已下载公开 ASCEND 录音时执行：

```powershell
desktop/target/release/local-voice-desktop.exe --asr-models-check .runtime/asr-eval/audio/01115.wav outputs/desktop/asr-models-check.json
```

检查通过与设置保存相同的服务管理路径，连续切换 CPU → llama 0.6B → llama 1.7B → CPU；每次检查就绪、重复启动和真实 HTTP 转写，最后验证停止。不会修改设置、录音或把文字输入其他应用。它是集成检查，不是模型准确率或性能排名。

`scripts/check-llama-cleanup.py` 使用真实 0.6B 模型验证适配器被强制终止后，llama-server 在 10 秒内退出。使用 `.venv-qwen/Scripts/python.exe` 运行；检查输出进程 ID 和通过标记。

本机已完成上述切换及强制清理检查。两个 llama 模型还分别检查了自动/指定语言、数字静音，以及将公开录音重复至 30 秒的输入上限；结果文本无协议标记泄漏。重复音频只用于长度和资源边界检查，不纳入质量评测。22 项 Rust 测试通过。

## 1.7B 下一步值得做什么

**优先做术语和中英混说的上下文提示消融。** 现有真人录音中，连 BF16 都把 `homework` 听错；仅继续压低量化位数或改一个小算子，不会解决这种问题。Qwen 官方实现将 context 放在 system 消息中，见 [官方 `_build_messages` / `_build_text_prompt`](https://github.com/QwenLM/Qwen3-ASR/blob/main/qwen_asr/inference/qwen3_asr.py)。这提供了一条值得验证的路径，但并不证明在当前 GGUF 后端上一定有收益。

1. 先固定 1.7B Q8、编码器、推理参数和录音；为技术名词、英文缩写、中英切换分别记录错误，之后再补个人麦克风录音。
2. 只对比“空上下文”与“一份事先固定的术语表”。术语来自实际使用领域，不能从测试转写答案中提取；校验 llama 模板是否忠实保留 system 消息。
3. 同时统计术语正确率、总体 CER/MER、错误插入术语的次数和请求延迟。普通无术语语音也必须测，防止模型把词表硬塞进结果。选定候选后再看独立留出的录音。
4. 有稳定收益再接到设置中；没有收益就记录负结果，保持默认无上下文。

如果更想深入推理系统，第二优先是**对 1.7B Q8 做完整时间线剖析**：分开音频处理/编码、文本 prefill、逐 token decode 和桌面等待，取得 Nsight 证据后再决定要改哪段 CUDA/ggml 路径。已有消融支持保留 CUDA Graph 和 Flash Attention，尚未证明某个具体 kernel 是主要瓶颈。

这里学到的是模型提示协议、解码行为、带负例的评测和性能归因，经验不局限于 Windows。当前量化实验的结果与边界见 [真人录音报告](qwen-asr-quantization-results.md)。
