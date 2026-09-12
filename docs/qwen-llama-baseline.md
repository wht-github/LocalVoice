# Qwen3-ASR 原生 Windows 部署基线

2026-09-12：Qwen3-ASR 1.7B 的 Q8_0 文本模型 + BF16 音频编码器已在 RTX 4050 Laptop 6 GB 上完成音频转写。当前组合比本机现有 PyTorch 0.6B FP16 方案响应更快，值得进入真实录音评测。模型大小、精度和运行方式同时改变，本轮结果不能单独归因于推理框架。

## 配置与复现

- llama.cpp 提交：`eafe15a5e3d87dd68ae33acf6a7cbd9415a0ac5e`，MSVC 19.51 + NVCC 13.4，Release，SM 8.9。
- 模型：`ggml-org/Qwen3-ASR-1.7B-GGUF`，修订 `36a678687ba7d07a74ca70ccb0e36902e005fb80`。Q8_0 主模型 2,165,034,944 字节，BF16 mmproj 641,773,984 字节；下载脚本固定修订并校验 SHA-256。
- 常驻 llama-server：GPU layers=99，context=2048，parallel=1，CPU threads=4，greedy，max tokens=1024。关闭请求间 prompt cache 和 RAM cache。
- PyTorch：现有 Qwen3-ASR 0.6B HF 权重，FP16、SDPA、原始 greedy generate，无 TileLang 替换，无手动 CUDA Graph。
- 输入：两段已有 Melo 合成语音，经同一流程重采样到 16 kHz。短音频截取中文前 3 秒；长音频拼接中文、0.5 秒静音与中英混合。两条路线的输入 float32 哈希完全相同。
- 每个样本单独预热一次，再测三次。llama 路线包含本地 HTTP、WAV 解码、音频处理和生成，不包含事先 base64 编码；PyTorch 包含音频处理、传输、生成与文本解码，不包含文件读取。两者都排除模型加载。

从项目根目录运行，两个后端按顺序执行，避免同时占用显卡：

```powershell
.venv-qwen\Scripts\python.exe scripts\download-qwen-gguf.py
.venv-qwen\Scripts\python.exe scripts\benchmark-qwen-llama.py --backend llama --output outputs\qwen-llama\my-run
.venv-qwen\Scripts\python.exe scripts\benchmark-qwen-llama.py --backend pytorch --output outputs\qwen-llama\my-run
```

脚本自行启动仅监听 127.0.0.1 的临时服务，测试结束或异常时结束该进程，不接管桌面服务。

## 实测结果

单位为秒，括号为三次运行的最小值至最大值。

| 样本 | 音频时长 | llama.cpp 1.7B Q8 中位数 | PyTorch 0.6B FP16 中位数 |
| --- | ---: | ---: | ---: |
| 短句 | 3.000 | 0.273（0.271–0.290） | 0.749（0.722–0.752） |
| 中文 | 10.417 | 0.652（0.648–0.654） | 1.709（1.650–1.764） |
| 中英混合 | 12.588 | 0.673（0.671–0.675） | 1.804（1.747–1.815） |
| 长音频 | 23.505 | 1.240（1.222–1.265） | 3.312（3.285–3.327） |

llama-server 从启动到健康检查通过约 3.10 秒；第一次短句请求约 0.53 秒，预热后的结果见表。加载时间受系统文件缓存影响，不是机器重启后的磁盘冷启动结果。

NVML 每 20 ms 采样：llama 路线整卡显存峰值 5,043 MiB（约 4.92 GiB），模型加载前整卡占用 1,714 MiB，加载后 4,989 MiB。PyTorch 参考峰值约 3,881 MiB。以上包含桌面和其他程序，不能当作模型进程的精确峰值；采样也可能漏掉极短峰值。

12 次 llama 正式请求都返回正常停止，cached tokens=0。各后端在每个样本上的三次输出完全一致；两条路线的生成 token 数分别均为 17、41、43、80。模型之间存在用词、标点差异，本轮没有人工标注，也没有计算 CER/WER，不能得出准确率提升结论。

首次 PyTorch 参考在模型下载期间出现较大波动，原始记录保存在 `pytorch-during-download.json`。表中使用下载结束后的完整复测结果。尚未锁定 GPU 频率、温度或后台负载，因此不是严格受控的性能论文实验。

## 下一步依据

长音频的 llama 服务端计时中位数：prompt 阶段约 131 ms，逐 token 生成约 1,019 ms；客户端请求总耗时约 1,240 ms。生成阶段约占总时间 82%，约 77.5 token/s。prompt 计时不能直接当作纯音频编码器耗时，其余开销也不能未经追踪就归因于某个算子。

实际部署的可行性已得到验证。下一轮应先加入真实麦克风录音及人工转写，固定 **同一个 1.7B 模型**，建立高精度参考，再用 profiler 判断文本解码时间花在哪里。之后比较 Q8、Q6/Q4 或按张量保留精度的方案，检查延迟、显存和识别错误率的取舍。现有证据支持优先研究解码阶段，还不足以认定是显存带宽、内核效率或调度中的哪一个瓶颈。

## 原始记录

目录：`outputs/qwen-llama/20260912-1p7b-baseline/`。

- `llama.json`、`pytorch.json`：逐次计时、转写、显存、输入哈希；llama 记录保留完整 HTTP 响应、服务器计时、模型修订和启动参数。
- `server.log`：加载与请求日志。
- `transcriptions.md`：四组样本的并列转写。
- 四个 WAV 文件：相同输入的浮点 WAV 副本。

环境准备说明见 `docs/llama-native-windows.md`。模型来源：[ggml-org 模型仓库](https://huggingface.co/ggml-org/Qwen3-ASR-1.7B-GGUF/tree/36a678687ba7d07a74ca70ccb0e36902e005fb80)。
