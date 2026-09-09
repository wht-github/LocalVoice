# Qwen ASR Windows 原生实验

使用独立 `.venv-qwen` 环境运行 Qwen3-ASR-0.6B-hf，通过 Transformers / PyTorch CUDA 调用 NVIDIA GPU。实验脚本不启动 WSL，也不改变桌面端当前的 SenseVoice 设置。

## 悬浮窗

在“设置 → 识别模型”选择“Qwen · NVIDIA GPU”并保存。客户端启动项目 `.venv-qwen/Scripts/python.exe` 运行 `asr_server.py`，复用现有模型缓存，使用与原识别服务相同的本机接口。首次预热完成后显示就绪。手动听写、连续听写和长录音分段沿用原客户端流程。

更换识别模型前需结束当前听写并等待识别完成。保存会停止客户端持有的旧识别进程并加载新模型；停止全部服务或退出客户端会释放该进程和显存。若端口被其他程序启动的模型占用，客户端提示冲突，不终止不属于自己的服务。启动失败见 `%LOCALAPPDATA%/LocalVoice/native-asr.log`。

2026-09-08 桌面接入验证：21 项 Rust 测试通过；真实 Qwen API 的中文、混读、静音、错误输入与时长限制检查通过；发布版 `--qwen-runtime-check` 验证子进程启动、重复启动、HTTP 转写、停止服务通过。设置窗口渲染检查通过。报告位于 `outputs/qwen-native/api-check.json`、`desktop-runtime.json`；没有自动采集真实麦克风或向用户应用输入文字。

## 安装

在项目目录的 PowerShell 中运行（需要可用的 Python 3.12）：

```powershell
.\scripts\setup-qwen-native.ps1 -Python 'C:\path\to\python.exe'
```

PyTorch 固定为 2.9.1 / CUDA 12.8，Transformers 固定为 5.16.1。安装包自带 CUDA 运行库；正常运行无需单独安装 CUDA Toolkit，但需要兼容的 NVIDIA 驱动。普通 Python 依赖使用清华源，PyTorch 使用官方 CUDA 源。

## 转写自己的录音

```powershell
.\qwen-native.ps1 'C:\path\to\recording.wav' -Repeat 3 -Output .\outputs\qwen-native\recording.json
```

也可以直接调用 Python 脚本：`.\.venv-qwen\Scripts\python.exe -X utf8 .\scripts\qwen-native.py --help`。下面的 `--language` / `--prompt` 等为 Python 参数；PowerShell 入口对应 `-Language` / `-Prompt`。

首次运行下载模型，缓存位于 `.runtime/qwen-hf`，默认使用 HF 镜像。若要使用官方源，可先设置 `$env:HF_ENDPOINT = 'https://huggingface.co'`。后续复用缓存；需要严格离线时设置 `$env:HF_HUB_OFFLINE = '1'`。

可选 `--language Chinese` 指定中文，或 `--prompt '词汇：PyTorch、千问、语音识别。'` 提供词汇提示。默认自动判断语言，适合中英混读。

默认单请求、FP16、SDPA 注意力、普通执行模式，不启用 torch.compile 或外部 FlashAttention。模型固定放在 CUDA，GPU 不可用时直接报错，不会悄悄退回 CPU。

## 看懂输出

- `result`：语言与识别文字。
- `seconds`：本次音频特征处理、GPU 数据传输、文字生成和解码耗时；不含模型加载与音频文件读取/重采样。
- `first_request`：第一轮可能包含首次初始化开销；第二轮起用于观察预热后的性能。
- `rtf`：推理时间除以录音长度，小于 1 表示比录音时长快。
- `peak_allocated_mib`：PyTorch 张量峰值显存。
- `peak_reserved_mib`：PyTorch 内存池峰值预留量。两者均不包含驱动与其他程序的全部显存占用。
- `token_limit_reached`：输出达到限制，文字可能不完整。可以增大 `--max-new-tokens`；长录音还需要单独设计分段。

这是一份基础推理实验，不等于完整长录音/流式识别服务。同一音频重复测试只能证明运行情况与预热性能，不能替代独立录音集的准确率评估。与历史 vLLM 数据比较时，需要统一音频、模型、精度和计时范围。

## 本机首次验证（2026-09-08）

Windows 原生、RTX 4050 Laptop 6GB、Python 3.12.14、PyTorch 2.9.1+cu128、Transformers 5.16.1、FP16/SDPA。依赖一致性检查与 CUDA 矩阵计算通过；模型缓存完整后，在 `HF_HUB_OFFLINE=1` 下转写成功。

| 项目内的合成录音 | 长度 | 首次请求 | 第二/三次请求 | 峰值预留显存 |
|---|---:|---:|---:|---:|
| `outputs/melo-mandarin.wav` | 10.42 秒 | 3.087 秒 | 1.614 / 1.612 秒 | 1688 MiB |
| `outputs/melo-mixed.wav` | 12.59 秒 | 2.677 秒 | 3.037 / 2.428 秒 | 1718 MiB |

两段文本人工对照与原始合成文本一致，标点存在差异；未计算正式 CER/MER。混读耗时有波动，不能只取最快一次，也不能与历史 vLLM 的另一套测试集直接算速度倍数。原始报告保存在 `outputs/qwen-native/mandarin.json` 和 `mixed.json`，依赖版本记录保存在 `.runtime/qwen-native-lock.txt`。

模型 revision：`7f1569a48a89f3e3f4dc3a5c9d28bddd903bc76c`；模型权重 SHA-256：`d3f212dd20abecd315d830bc54ae3865e56ebfc3276484e57b771288ba27fd35`，已校验。PyTorch 官方 wheel 校验通过后保留在 `.runtime/qwen-wheels`，可用安装脚本的 `-TorchWheel` 参数复用，避免重新下载。

本机环境使用 Codex 随附的 Python 3.12 创建，基础解释器位于 `C:\Users\VICO\.cache\codex-runtimes\codex-primary-runtime\dependencies\python\python.exe`。虚拟环境依赖该解释器，不能直接拷贝到其他电脑；若基础解释器被移除，需要使用新的 Python 3.12 重建环境。
