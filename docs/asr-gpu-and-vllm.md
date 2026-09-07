# STT 模式

**当前状态**：桌面端仅保留 `SenseVoice · CPU`，且以 Windows 原生 Python 进程运行（`scripts/setup-native-asr.ps1` 安装环境），默认完全不启动 WSL。Qwen3-ASR vLLM 已从桌面端界面移除，以下 WSL 流程保留供手动实验。

## Qwen3-ASR vLLM（WSL，手动实验）

在专用 `LocalVoice` 实例内运行，使用 Qwen 官方 `qwen-asr[vllm]` 后端，模型通过 Hugging Face 镜像下载到实例内部，运行在独立 `asr-gpu` Python 环境。

手动使用：先停止桌面端的原生识别（设置中“停止全部服务”），再在 WSL 内启动：

```powershell
wsl -d LocalVoice -u root -- bash /opt/local-voice-app/scripts/select-asr.sh qwen-vllm
```

服务就绪后监听 `127.0.0.1:8001`，与桌面端同端口；实验结束后切回 `sensevoice-cpu` 并停止该服务，避免与原生识别抢占端口。

安装（如尚未装好 GPU 环境）：

```powershell
wsl -d LocalVoice -u voice -- bash /opt/local-voice-app/scripts/install-asr-gpu.sh
wsl -d LocalVoice -u voice -- bash -c "source /opt/local-voice-app/scripts/env.sh && \"$VOICE_RUNTIME\"/asr-gpu/bin/python /opt/local-voice-app/scripts/download-qwen-asr.py"
```

Qwen3-ASR 官方提供 vLLM 后端和 `qwen-asr[vllm]` 安装方式，0.6B 适合 6GB 显存的尝试。当前 vLLM 参数：显存预算 60%（按总显存计算，需扣除 Windows 侧桌面应用的动态占用）、`max_model_len` 3072、并发 1、`enforce_eager`（禁用 torch.compile 和 CUDA Graph，节省约 2 GB 显存并避免编译期显存波动）。vLLM 自身占用约 2.4 GB。若启动报 "No available memory for the cache blocks"，通常是 Windows 侧显存基线上涨挤压了预算，可适当下调 `max_model_len` 或上调预算。

已测基线（同一段 10 分钟中英混合真人语料，63 段能量停顿切分）：

- SenseVoice CPU：MER 14.2%，RTF 0.043，p95 单段延迟 0.61 秒，进程 PSS 峰值约 1.7 GiB。
- Qwen3-ASR vLLM：MER 10.7%，RTF 0.068，p95 单段延迟 1.03 秒，PSS 峰值约 1.5 GiB；vLLM 自身显存约 2.5 GB（整机含桌面占用实测约 5.3/6 GB）。首轮请求含 JIT 预热。

详细数据见 `outputs/long-mixed/`（WSL 内）与 `results-qwen-vllm.json`、`results-sensevoice-cpu.json`。

GPU 依赖和 Qwen 模型下载属于 LocalVoice 专用 WSL；没有修改 Windows 全局 `.wslconfig`，也没有改变其他 WSL 发行版。
