# 本地语音服务：SenseVoiceSmall + MeloTTS

识别默认在 **Windows 本机原生运行**：桌面客户端直接启动原生 Python 进程加载 SenseVoiceSmall / FunASR（CPU），不依赖 WSL、不占 WSL 内存。
朗读（TTS）默认关闭；启用时使用专用 WSL2 实例 `LocalVoice`（Ubuntu 24.04，虚拟磁盘位于 `D:\WSL\LocalVoice`）中的 MeloTTS。旧 Kokoro 环境保留用于回退，不同时加载。
Qwen3-ASR vLLM 曾在 WSL 中验证过（数据见 [STT 模式](docs/asr-gpu-and-vllm.md)），目前从桌面端移除，脚本保留供手动实验。

悬浮窗设置现可选择 **SenseVoice CPU、llama.cpp Qwen ASR 0.6B Q8、1.7B Q8**，保存后重新加载模型，见 [桌面模型选择与安装](docs/qwen-llama-desktop.md)。原来的 PyTorch CUDA / Transformers 0.6B 入口保留作对照，见 [Qwen 原生说明](docs/qwen-native-windows.md)。

想动手修改 GPU 算子，可运行 `.\tilelang-lab.ps1`，进入独立的 [PyTorch + TileLang 实验](docs/tilelang-lab.md)：包含可运行加法基线、正确性检查、计时和一行算子融合练习。

下一项 [RMSNorm 单项实验](docs/rmsnorm-lab.md) 用 `.\rmsnorm-lab.ps1 --baseline` 运行 Qwen 原始基线；只修改 RMSNorm 后再与它对照，不叠加其他推理优化。

原生 llama.cpp 路线已完成 [CUDA 环境配置](docs/llama-native-windows.md) 和 [1.7B 量化、解码消融及真人录音评测](docs/qwen-asr-quantization-results.md)。实验固定同一模型和音频编码器，提供 BF16/Q8/Q6/Q4 对照、独立 holdout 与逐条转写；复现步骤见 [实验入口](experiments/qwen_asr/README.md)。

桌面 1.7B Q8 参数下的 [公开录音性能基线与 Nsight 剖析](docs/qwen-asr-nsight.md) 已完成，包含预热请求热点、阶段计时、显存构成及下一步单变量优化方向。

## 打开和管理

Windows 原生悬浮窗使用 Rust + Slint：运行 `./desktop.ps1` 启动客户端（按已保存设置启动识别进程）。默认 `Ctrl+Alt+Space` 开始/结束听写，`Ctrl+Alt+R` 朗读选中文字或停止播放。使用说明和兼容边界见 [桌面客户端](desktop/README.md)。

也可直接打开发布 EXE。设置中可选择随应用启动服务、单独关闭朗读，以及一键停止全部服务。关闭朗读并保存后退出 TTS 模型进程，听写不受影响；识别进程的启停完全在 Windows 本机完成。详见 [Melo 部署与资源实测](docs/melo-and-service-control.md)。

在此项目的 PowerShell 中运行：

```powershell
.\voice.ps1 start
.\voice.ps1 status
.\voice.ps1 test
.\voice.ps1 stop
```

测试页面：<http://localhost:8002>。首次启动需加载模型；服务变为 ready 后才能访问。
页面支持文字合成、音色选择、上传音频识别，以及识别刚生成的语音。

单独启动或停止一个服务：` .\voice.ps1 start tts`、`.\voice.ps1 stop asr`。
查看日志：`.\voice.ps1 logs`。`stop` 会停止两个服务并关闭专用实例以释放内存；指定单个服务时只停止该服务。
`start` 启动一个隐藏的保活进程，防止 WSL 在命令窗口退出后自动停止，重复启动不会堆积保活进程。
服务按需启动，没有设置 Windows 登录自启。

## API

| 服务 | 地址 | 用法 |
|---|---|---|
| 识别 | `http://localhost:8001/v1/audio/transcriptions` | POST multipart：`file` 和 `language=auto/zh/en` |
| 合成 | `http://localhost:8002/v1/audio/speech` | POST JSON：`input`、`voice`、`speed`，返回 WAV |
| 音色 | `http://localhost:8002/v1/audio/voices` | GET |
| 健康检查 | 两个端口的 `/health` | GET，包含模型、设备和 ready 状态 |

合成请求示例：

```json
{"input":"你好，这是本地语音测试。Hello, local voice.","voice":"melo_zh","speed":1,"response_format":"wav"}
```

- 当前音色：`melo_zh`，支持中文、中英混读和英文。`default` 使用当前后端首个音色；可查询音色接口获取有效 ID。
- 第一版限制识别文件为 30 秒以内、20 MB 以内的 WAV/FLAC/OGG；合成文字最多 2000 字符。
- API 返回完整识别结果和完整 WAV；Windows 客户端手动开始/结束录音，结束后识别、一次输入，朗读支持长文本连续播放（有界缓存与预取），没有接入 LLM 或模型原生流式推理。
- 每个服务一次执行一个推理请求，忙碌时返回 429。
- 仅监听本机 `127.0.0.1`，不使用云端语音 API。

## 独立环境与版本

服务以普通用户 `voice` 运行，安装目录：

| 内容 | LocalVoice 中的路径 |
|---|---|
| 部署后的运行文件 | `/opt/local-voice-app` |
| Python 环境与模型 | `/home/voice/.local/share/local-voice-app` |
| ASR 独立环境 | 上述目录下 `asr` |
| TTS 独立环境 | 上述目录下 `tts-melo`；旧 Kokoro 在 `tts` |
| 模型缓存 | 上述目录下 `modelscope` 和 `huggingface` |
| 服务定义 | `/etc/systemd/system/local-voice-{asr,tts}.service` |

Python 3.12；FunASR 1.4.14；Melo 源码固定提交 `2091453`，Transformers 4.44.2；PyTorch CPU 2.9.1。旧 Kokoro/Misaki 0.9.4 单独保留。
安装完成后完整依赖记录在项目 `.runtime/asr-lock.txt`、`.runtime/tts-lock.txt`。
软件源和模型下载配置：

- Ubuntu APT（含安全更新）：`https://mirrors.tuna.tsinghua.edu.cn/ubuntu/`。
- uv 默认 PyPI：`https://pypi.tuna.tsinghua.edu.cn/simple`，同时写入实例内 `/etc/uv/uv.toml` 和项目 `uv.toml`。pip 默认源也已设置。
- 模型及音色：`HF_ENDPOINT=https://hf-mirror.com`，SenseVoice 后续新下载、Kokoro 和 spaCy 英文模型均使用该入口。
- 当前已有的 SenseVoice 本地缓存直接复用。运行服务启用离线模式，不重新下载权重。
- PyTorch/torchaudio 的 `+cpu` 专用 wheel 是 PyPI 之外的独立发行包，安装脚本仍使用官方 wheel 或已验证的本地缓存；普通 Python 包均使用清华源。

这些默认值在实例内持久化到 `/etc/environment`、`/etc/profile.d/local-voice-mirrors.sh` 和 `/etc/uv/uv.toml`。
原 APT 与环境配置已备份。只需重新应用镜像设置时可运行：

```powershell
wsl -d LocalVoice -u root --cd / --exec bash /opt/local-voice-app/scripts/configure-mirrors.sh
```
模型和语言前处理资源首次下载后缓存在本机。
验证完成后 `scripts/finalize.sh` 会把 SenseVoice 配置成直接读取本地目录，并启用 Hugging Face 离线模式。

## 在新 Windows 机器上部署

默认部署只需 Windows 本机环境：识别（SenseVoice CPU）以原生 Python 进程运行，不启动 WSL。WSL 仅在需要朗读（TTS）或手动实验 Qwen GPU 时安装。

前置要求：Windows 10 22H2 / 11、Git、Rust 工具链（编译桌面端）、Python 3.10–3.13。

```powershell
# 1. 安装原生识别环境（CPU torch + funasr，venv 建在 %LOCALAPPDATA%\LocalVoice\venv）。
pwsh -NoProfile -File scripts/setup-native-asr.ps1

# 2. 编译并启动桌面客户端。首次识别启动时 SenseVoice 模型经 HF 镜像自动下载。
.\desktop.ps1 build
.\desktop.ps1 start
```

听写即开即用。需要朗读或想实验 Qwen 时再装 WSL 部分：

```powershell
# 创建专用发行版。名字必须是 LocalVoice，脚本有实例名检查。
wsl --install Ubuntu-24.04 --name LocalVoice

# 传入项目文件并注册 systemd 服务（主动传输，不依赖 /mnt）。
.\voice.ps1 deploy

# 实例内基础初始化（root）：换镜像源、装系统包、建 voice 用户、启用 systemd。
wsl -d LocalVoice -u root -- bash /opt/local-voice-app/scripts/provision.sh

# TTS 环境与模型（voice 用户）；首次启动 TTS 后可运行 finalize.sh 固化离线路径。
wsl -d LocalVoice -u voice -- bash /opt/local-voice-app/scripts/setup.sh
wsl -d LocalVoice -u voice -- bash /opt/local-voice-app/scripts/finalize.sh
```

（可选）MeloTTS 中文音色：`wsl -d LocalVoice -u voice -- bash /opt/local-voice-app/scripts/install-melo.sh`，再在桌面设置中切换。镜像端点等环境值见 `scripts/env.sh` 与 `scripts/configure-mirrors.sh`。

Qwen3-ASR vLLM 目前已从桌面端移除（界面不再提供切换），WSL 侧脚本保留供手动实验，见 [STT 模式](docs/asr-gpu-and-vllm.md)。

## 重新部署

仅对 `LocalVoice` 操作，脚本对基础环境配置与服务安装做了实例名检查。

```powershell
.\voice.ps1 deploy
.\voice.ps1 stop
.\voice.ps1 start
```

`deploy` 通过 Windows 主动传输文件到实例内，不依赖 `/mnt/c` 或 `/mnt/d`。它更新服务定义并应用 LocalVoice 隔离设置，不重新安装模型或 Python 依赖。修改项目源码后需要重新部署并重启服务。

## 单实例隔离与资源额度

2026-09-06 已仅对 LocalVoice 关闭 Windows 盘自动挂载、fstab 启动挂载、Windows 程序互操作和 Windows PATH 导入。两个服务使用普通用户、只读系统目录、独立临时目录和受限写入路径，不能访问 `/mnt` 等宿主挂载入口。

两个语音服务共同归入 `localvoice.slice`：CPUQuota=600%（合计约 6 个逻辑处理器的 CPU 时间）、MemoryHigh=6G、MemoryMax=8G。这不是整台 WSL VM 的内存上限，其他 Linux 进程和部分系统开销不在此服务组内。没有设置 CPU 亲和性，`nproc` 仍可显示 16。

Windows 的 `%UserProfile%\.wslconfig` 未改动；没有调整全局 memory、processors、swap、网络模式或 WSLg，也没有执行全体 WSL 关闭。本机 HTTP 访问和镜像下载配置保留。

资源实测、作用范围及恢复说明见 [资源与隔离记录](docs/resources-and-isolation.md)。

以上额度描述历史 WSL 服务，不限制当前 Windows 原生识别进程。桌面当前的 CPU、llama.cpp 和 PyTorch 选择见 [模型设置](docs/qwen-llama-desktop.md)。历史 vLLM 测试曾对 11 分 40 秒真人混读拼接音频进行分段，自动语言模式处理约 30 秒，混合错误率 14.22%；方法和限制见 [长中英混用测试](docs/long-mixed-asr-test.md)，不能把它作为当前桌面后端的性能数据。

## 验证

`.\voice.ps1 test` 会真实调用两个模型，生成中英文音频并交给 SenseVoice 识别。
测试输出保存在 LocalVoice 的 `/opt/local-voice-app/outputs/`，包括 `tts-zh.wav`、`tts-en.wav` 和 `smoke-results.json`；终端同时返回识别结果。Windows 项目 `outputs/melo-*.wav` 为新模型试听。
同时检查非静音输出及无效请求处理。合成音频回环只用于验证链路；真实麦克风、口音、噪声准确率需要另行测试。

2026-09-06 首轮 CPU 实测（短句、已预热）：

| 样例 | 音频长度 | Kokoro 合成 | SenseVoice 识别 |
|---|---:|---:|---:|
| 中文合成回环 | 6.50 秒 | 2.05 秒 | 0.46 秒 |
| 英文合成回环 | 5.38 秒 | 1.46 秒 | 0.31 秒 |
| 官方中文真人样例 | 5.62 秒 | — | 0.31 秒 |
| 官方英文真人样例 | 7.18 秒 | — | 0.36 秒 |

中文回环逐字一致；英文回环把 `Speech` 识别成了 `Sp`，因此链路通过不代表识别完全准确。
首次识别曾有约 11.7 秒的初始化开销，服务已增加启动预热。

官方参考：[SenseVoice](https://github.com/FunAudioLLM/SenseVoice)、[FunASR](https://github.com/modelscope/FunASR)、[Kokoro](https://github.com/hexgrad/kokoro)。
