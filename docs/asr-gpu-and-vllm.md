# STT 模式

桌面客户端设置中的“识别模式”控制专用 `LocalVoice` 实例的 ASR 服务。可选：

- `SenseVoice · CPU`：当前默认模式，使用已有 FunASR 环境和模型。
- `Qwen3-ASR 0.6B · vLLM GPU`：使用 Qwen 官方 `qwen-asr[vllm]` 后端和 vLLM，模型通过 Hugging Face 镜像下载到 LocalVoice 内部，运行在独立 `asr-gpu` Python 环境。

GPU 环境不会替换 CPU 环境。切换时客户端停止 ASR，修改专用服务配置，启动新后端并等待 `/health`；启动失败会恢复上一个后端。切换不能在录音、识别或等待输入结果时进行。

Qwen3-ASR 官方提供 vLLM 后端和 `qwen-asr[vllm]` 安装方式，0.6B 适合 6GB 显存的尝试。当前 vLLM 参数：显存预算 60%（按总显存计算，需扣除 Windows 侧桌面应用的动态占用）、`max_model_len` 3072、并发 1、`enforce_eager`（禁用 torch.compile 和 CUDA Graph，节省约 2 GB 显存并避免编译期显存波动）。vLLM 自身占用约 2.4 GB。若启动报 "No available memory for the cache blocks"，通常是 Windows 侧显存基线上涨挤压了预算，可适当下调 `max_model_len` 或上调预算。

已测基线（同一段 10 分钟中英混合真人语料，63 段能量停顿切分）：

- SenseVoice CPU：MER 14.2%，RTF 0.043，p95 单段延迟 0.61 秒，进程 PSS 峰值约 1.7 GiB。
- Qwen3-ASR vLLM：MER 10.7%，RTF 0.068，p95 单段延迟 1.03 秒，PSS 峰值约 1.5 GiB；vLLM 自身显存约 2.5 GB（整机含桌面占用实测约 5.3/6 GB）。首轮请求含 JIT 预热。

详细数据见 `outputs/long-mixed/`（WSL 内）与 `results-qwen-vllm.json`、`results-sensevoice-cpu.json`。

GPU 依赖和 Qwen 模型下载属于 LocalVoice 专用 WSL；没有修改 Windows 全局 `.wslconfig`，也没有改变其他 WSL 发行版。
