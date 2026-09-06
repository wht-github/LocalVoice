# MeloTTS 与客户端服务管理

2026-09-06：默认朗读从 Kokoro 切换到 `myshell-ai/MeloTTS-Chinese` 的 `ZH` 音色，对外 ID 为 `melo_zh`。这与用户提供的 [mrfakename 演示](https://huggingface.co/spaces/mrfakename/MeloTTS/blob/main/app.py) 使用同一官方中文模型；该页面是演示应用，并非另一套模型权重。中文模型支持中英混读，本部署也用它朗读纯英文。

本次已验证离线生成、速度和资源，未用自动化结果代替主观听感判断。试听文件：`outputs/melo-mandarin.wav`、`outputs/melo-mixed.wav`、`outputs/melo-english.wav`。中文和英文的 ASR 回环存在少量错字，链路通过不代表发音或识别完全正确。

## 资源实测

硬件：Ryzen 7 7840H、32 GB RAM、RTX 4050 Laptop 6 GB。SenseVoice 与 Melo 都使用 PyTorch `2.9.1+cpu`，`torch.version.cuda=None`，没有分配模型 GPU 显存。Windows 上其他应用的显存另计；WDDM 的 nvidia-smi 逐进程显存显示 N/A，不能拿它当作模型显存测量值。

| 项目 | 本机结果 |
|---|---:|
| SenseVoice 已加载 PSS | 1.68 GiB |
| Melo 已加载 PSS | 2.08 GiB |
| 两个进程已加载 PSS 合计 | 3.76 GiB |
| 并发请求时 PSS 合计峰值 | 3.88 GiB |
| Melo 测试峰值 PSS | 2.16 GiB |
| 模型 GPU 显存 | 0 |
| 每个模型的计算线程数 | 4 |
| 并发测试平均 CPU 满核等效数 | 4.30 个逻辑处理器 |

原始数据 `outputs/resource-profile-melo.json`：ASR 25 秒输入连续 3 次，TTS 约百字符混读连续 3 次，采样间隔 150 ms。并发时 ASR 每次约 2.0–2.1 秒，TTS 每次约 4.9–7.1 秒。短采样 CPU 峰值 6.53 个逻辑处理器，与跨 CPU quota 周期的采样有关。

保留专用 `localvoice.slice` 的 CPUQuota=600%、MemoryHigh=6G、MemoryMax=8G。它们是两个服务合计的额度，未预留物理核心或 8 GiB 内存。完成验证时组内累计内存峰值约 5.98 GiB，包含文件缓存及此前运行历史；并非 Melo 新一轮的进程峰值。组内 high/max/oom/oom_kill 事件均为 0。

日常预算可按两个模型约 4 GiB 驻留内存、另留加载与缓存余量。已有 8 GiB 服务组上限可以沿用；不建议把整个 WSL VM 卡到 4 GiB。关闭朗读后 TTS 模型进程退出，文件缓存和 WSL 内核仍可能保留，所以 Windows 的 WSL 工作集不会保证立即按进程 PSS 等量下降。

## 使用方式

直接打开发布 EXE 或运行 `desktop.ps1`。默认随应用启动识别服务，并按已保存的朗读开关决定是否加载 TTS。

- 在设置取消“启用朗读”并保存：取消播放并停止 TTS，ASR 进程保持不变。重新勾选并保存：重新加载 TTS。
- “启动已保存的服务”：使用已保存设置；未保存的勾选不生效。TTS 关闭状态不会被此按钮覆盖。
- “停止全部服务”：取消当前录音/播放并停止两个模型。保留 WSL 基础环境，不终止其他发行版。
- 隐藏或退出客户端保留后台服务；需要释放模型资源时先停止服务。也可 `voice.ps1 stop` 停服务并终止专用 LocalVoice。

Rust 在独立工作线程以隐藏进程调用固定 `wsl.exe -d LocalVoice ... systemctl` 参数，无新增 HTTP 管理端口、WebView 或常驻管理服务器。服务加载最多等待两分钟，超时后可重试。自定义服务地址不由这套固定实例管理逻辑接管。

## 模型与环境

- Melo 源码固定提交：`209145371cff8fc3bd60d7be902ea69cbdb7965a`；MIT 许可。
- 独立环境：`/home/voice/.local/share/local-voice-app/tts-melo`，Python 3.12.3、Transformers 4.44.2；完整版本清单 `outputs/melo-environment.lock.txt`。原 ASR 和 Kokoro 的环境没有升级或混装。
- `scripts/prepare-melo.py` 仅移除中文服务不使用的语言模块的立即导入，把英文所需的纯辅助函数原样提取，避免下载日语、法语、西语资源；合成模型和中文/英文前处理算法沿用上游。
- 模型权重与分词文件通过 `hf-mirror.com` 下载，大文件分块续传后验证镜像元数据提供的 SHA256。Melo checkpoint SHA256：`a74e9eadffff065c75eb6dfa040efa72cad23e72cfea70d39190bc174fb97093`；多语言 BERT safetensors：`b33adb2b700b7029a64a4a14ddec6bda8555d2ca879e80a75789fd9542a6290e`。
- uv 普通包使用清华源，复用已有 CPU torch wheel；匹配的 CPU torchaudio 从官方 CPU wheel 地址取得。NLTK 英文词典来自官方数据仓库，WSL 连接该仓库超时时通过宿主下载后显式传入。
- 生产服务启用 HF/Transformers 离线模式。模型生成原生 44.1 kHz 音频，接口保持 24 kHz PCM WAV；重采样初始化包含在启动预热内。
- API 保留单次 2000 字符上限，Melo 内部也限制每片最多 180 字符。桌面端自动按句、段和词边界处理最多 20000 字符文章，按顺序生成与播放，不要求手工分段。

重装 Melo（基础 LocalVoice 环境已存在）：先 `voice.ps1 deploy`，再在 LocalVoice 中以 voice 用户运行 `scripts/install-melo.sh`。启用：

```powershell
wsl -d LocalVoice -u root -- bash /opt/local-voice-app/scripts/select-tts.sh melo
```

回退把末尾改成 `kokoro`，然后在客户端设置中重新选择该后端的音色。旧环境仅留在磁盘，不与 Melo 同时加载。`service.env.before-melo` 与 Windows `settings.before-melo.json` 保留切换前设置。

## 验证记录

- 13 项 Rust 单元测试通过，发布构建通过。
- `outputs/desktop/runtime-check.json`：关闭 TTS、ASR 进程不变、启动遵守 TTS 关闭状态、停止全部以及重新启动就绪均通过。
- 专用输入框的 Unicode 输入、选区、快捷键及焦点变化检查通过；仅操作本程序的测试窗口。
- `outputs/desktop/restore-regression/result.json`：连续隐藏/显示三次，实际窗口像素与初始画面一致。
- `outputs/desktop/service-check.json`：Melo → SenseVoice 真实回环和麦克风设备枚举通过。
- `outputs/desktop/long-tts-melo.json`：2187 字符，18 段，237.81 秒音频，串行合成 69.54 秒，所有 WAV 非静音且格式正确，分段拼接保留全部文本。
- 测试未录制真实麦克风或播放扬声器；音色舒适度由试听判断。

## Qwen Serena 的取舍

[Qwen3-TTS 0.6B CustomVoice 官方模型卡](https://huggingface.co/Qwen/Qwen3-TTS-12Hz-0.6B-CustomVoice) 确认 Serena 为温柔的中文女声。页面完整权重统计约 0.9B 参数、BF16，按每参数 2 字节仅权重就是约 1.8 GB；运行还需要其他内存，不能把“0.6B”当成全部显存开销。本轮优先部署用户已偏好的 Melo CPU 方案，没有安装或实测 Qwen，因此不提供本机 Qwen 显存保证值。若试听仍不满意，可以再用 Serena 做对比；当前服务开关已支持用完停止模型。

本次未修改 Windows 全局 `.wslconfig`，未执行全体 WSL 关闭，也未修改其他发行版。
