# Qwen ASR 1.7B Q8：公开录音基线与 Nsight

2026-09-13。本轮完成测量和定位，没有修改模型、精度或 CUDA 内核。

**速度优先看 Q8 矩阵向量乘；显存优先审视常驻权重与音频编码器工作区。** 在预热后的请求中，`mul_mat_vec_q` 占已采集 kernel 时间的 81.10%，RMSNorm 为 2.06%；没有观察到新的 CUDA device 内存申请。继续优化 RMSNorm 或重复做分配器改造，都不是这份证据支持的首选。

## 1. 条件和样本

- RTX 4050 Laptop 6 GB，驱动 610.88，Windows 原生 CUDA；llama.cpp 固定 `eafe15a5e3d87dd68ae33acf6a7cbd9415a0ac5e`，没有修改上游源码。
- 官方 Q8_0 文本模型、BF16 音频编码器；上下文 **2048**，batch/ubatch 256，单并发，4 线程，F16 KV，CUDA Graph / Flash Attention 开启，关闭 prompt cache。模型与录音 SHA256 进入原始结果。
- 沿用 [ASCEND 固定清单](../experiments/qwen_asr/ascend-manifest.json)：60 条公开真人录音、两位说话人、约 250 秒。原 pilot/holdout 命名保留，但这些数据以前已经查看过，本轮只做性能与质量回归，不声称新的盲测。
- 普通运行分别测两个 30 条子集；每个子集按语言选择最短/最长，共六条性能录音，每条预热一次、计时五次。
- Nsight 另起进程测 pilot，六条性能录音每条计时三次。其计时不用于替代普通基线。

计时从已准备好的 WAV/base64 发起本地 llama HTTP 请求开始，到完整响应结束。**不包括录音、静音等待、桌面适配器重采样和输入文字。** 参数对齐桌面，但为了拆分阶段，本轮使用更详细的服务日志（`-lv 4`）；此前量化实验的上下文为 1024，不直接拼成同条件排名。

## 2. 不挂分析工具的性能与质量

下表每个子集的“平均请求耗时”是六条录音各自五次中位数的等权平均，不是所有录音的 p95。

| 子集 | 平均请求耗时 | 六条录音中位数范围 | 识别错误数 / 参考单位 | MER |
| --- | ---: | ---: | ---: | ---: |
| pilot | 297 ms | 178–508 ms | 39 / 433 | 9.01% |
| holdout | 307 ms | 184–497 ms | 44 / 448 | 9.82% |

最终基线使用 `q8-desktop-clean-{pilot,holdout}.json`。首次普通运行与 NVTX 工具验证有少量时间重叠，因此保留原始文件供追溯，补跑以上两组作为正式结果；没有同时执行其他自有分析任务。后台桌面负载和 GPU 频率仍未锁定。

全部 60 条输出正常结束；性能重复的文本稳定。错误计数与之前 Q8 结果一致，但小样本不能证明普遍无退化。

几个具体例子（毫秒，分别对各阶段取中位数，因此各列不要求严格相加）：

| 音频 | 时长 | 请求 | 编码调用墙钟 | 其余 prompt 阶段 | 自回归 decode |
| --- | ---: | ---: | ---: | ---: | ---: |
| `01219` 中文 | 2.00 s | 202 | 13.5 | 42.1 | 120.8 |
| `00047` 英文 | 7.04 s | 397 | 19.7 | 45.4 | 311.0 |
| `01055` 中英混说 | 11.34 s | 508 | 47.3 | 50.5 | 376.6 |

**服务端 `prompt_ms` 包含音频编码，不能直接叫“纯文本 prefill”。** 编码调用墙钟由日志中的 `encoding mtmd batch` 到 `decoding audio batch` 计算，对应 `mtmd_batch_encode` 及其批处理/传输；其余 prompt 时间仍含调度与同步，不是 GPU 纯计算。来源见 `tools/server/server-context.cpp` 的 `process_mtmd_chunk`、`tools/mtmd/mtmd-helper.cpp`，计时格式见 `common/log.cpp`。脚本检查了每份日志的请求数量，按实际顺序关联录音。

完整 12 条性能样本见 [baseline-cases.csv](../experiments/qwen_asr/performance/baseline-cases.csv)。

## 3. Nsight：热点与采集边界

使用 Nsight Systems **2026.5.1.161**，采集 CUDA、NVTX、CUDA Graph 节点和内存活动。关闭 CPU 采样与上下文切换采集，因此不需要管理员权限，也没有 CPU 调用栈或调度归因。

NVTX 标记加在 Python 客户端的 HTTP 请求范围，CUDA 活动来自其 llama-server 子进程。共核对 54 个请求标记，以下只汇总其中 **18 个预热后的性能请求**，排除了加载、质量检查及预热请求。

| Kernel 家族 | 已采集 kernel 时间占比 |
| --- | ---: |
| `mul_mat_vec_q` | 81.10% |
| `mul_mat_q` | 5.54% |
| `rms_norm_f32` | 2.06% |
| `flash_attn_ext_vec` | 1.60% |
| `quantize_q8_1` | 1.39% |

这些比例的分母是 kernel 时长总和，不是请求耗时。18 次请求共 5495 ms，GPU kernel/复制/memset 区间取并集约 4349 ms；并集避免把重叠 stream 重复计时。复制事件总时长约 20.5 ms。CUDA API 中 `cudaStreamSynchronize` 的长耗时主要表示等待，不能当作同等时长的 CPU 算法开销。

细分形状后，`has_fusion=true` 的 6144、2048 行单列 MMVQ 变体，以及 151936 行非融合变体最值得进一步检查。GGUF 中 FFN gate/up 为 `[2048, 6144]`，down 为 `[6144, 2048]`，`output.weight` 为 `[2048, 151936]`；形状能帮助定位，但最终算子归属仍应通过调用关联确认。完整名称、grid/block、次数和耗时见 [kernel-shapes.csv](../experiments/qwen_asr/performance/kernel-shapes.csv)。

采集存在必须保留的边界：

- 首次采集直接强制结束 Windows CUDA 子进程，可能来不及刷新 CUPTI 缓冲，因此不用于结论；原始 `q8-nsys-20260913-pilot.json` 保留。
- 最终采集先执行 `nsys stop`，待报告生成后再终止模型。但工具仍输出“可能未收集全部 CUDA/NVTX 事件”的警告；警告全文保存在 [nsight-summary.json](../experiments/qwen_asr/performance/nsight-summary.json)。
- 最终 54 个请求标记齐全，六条录音各三次的 kernel 数逐次相同，最后一个 kernel 到响应结束约 1.0–1.6 ms；这些检查支持使用预热区间做热点定位，**不等于证明 trace 完全无损**。
- profiler 与普通 pilot 的 30 条转写逐条相同。其平均请求耗时约 304 ms，普通基线约 297 ms；运行顺序、频率及后台负载未锁定，不能据此精确计算 profiler 开销。
- 尚未采集 Nsight Compute 的带宽、缓存命中、occupancy 等硬件计数器，不能仅凭 MMVQ 占比就断言已经证明显存带宽瓶颈。

## 4. 显存花在哪里

Nsight 捕获的主要 device 申请与服务启动日志对应：

| 用途 | MiB |
| --- | ---: |
| GPU 文本权重 | 1743.77 |
| BF16 音频编码器权重 | 612.02 |
| 音频编码器计算工作区 | 553.89 |
| 2048 上下文 F16 KV | 224.00 |
| 文本计算工作区 | 25.01 |

另有少量临时/缓存申请，捕获到的 device 申请总量约 3197 MiB。采集在销毁模型之前结束，所以没有对应的模型释放事件；这不是内存泄漏证据。18 次预热性能请求内没有新的 device 申请，重复分配不是当前首要方向。

NVML 的**整卡**峰值为 pilot 5469 MiB、holdout 5479 MiB；加载前也已有约 2145–2202 MiB。后台图形程序没有被关闭，且 Windows WDDM 下申请量与驻留量不同，因此不能把整卡峰值全部归给模型，更不能拿它和昨天不同桌面负载下的数值宣称显存回归。

编码器初始化按 3000 音频帧预留工作区（`clip-model.h` / `clip.cpp`），对应当前 30 秒路径。这使其成为值得研究的目标，但 554 MiB **不代表都能省掉**：必须检查实际张量生存期、短长音频交替时的重新分配和尾延迟。

## 5. 下一步按单变量开展

**速度方向：**先选一种实际命中的 Q8 MMVQ 形状，用 Nsight Compute 检查带宽、缓存及执行效率，再决定线程/warp 布局或融合方式。固定权重和激活格式，先做该形状的数值检查，再跑这批公开录音的质量与耗时回归。不要同时修改量化精度和 kernel 布局。

**显存方向：**先检查编码器工作区是否能按输入长度更合理地保留/复用。保持 BF16 编码器与 Q8 文本模型不变，对照当前固定预留；必须加入短→长→短音频序列，避免省显存却增加重复分配和卡顿。若仅试 KV 量化，当前 KV 总量只有 224 MiB，要先估算收益上限，不能期待它省出数 GB。

这次仅提交基线、采集入口、分析与文档；桌面模型配置未变，悬浮窗已恢复。

复现、工具准备与报告打开方式见 [性能实验入口](../experiments/qwen_asr/performance/README.md)。Nsight 的采集选项和平台限制以 [NVIDIA User Guide](https://docs.nvidia.com/nsight-systems/UserGuide/) 为准。
