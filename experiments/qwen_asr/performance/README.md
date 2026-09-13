# 1.7B Q8 性能剖析

先读 [结果与限制](../../../docs/qwen-asr-nsight.md)。普通基准与 profiler 分开运行；每次用新 tag，脚本拒绝覆盖已有原始 JSON。不要同时运行其他模型任务。

## 工具准备

本机已将 Nsight Systems 解压到 `.runtime/nsight-systems`，未安装系统组件、未更换驱动。官方 MSI 的常规静默安装因缺少管理员权限返回 1603，行政解包也失败，日志在 `outputs/nsight-install.log` / `nsight-extract.log`；最终使用只读 MSI 表和 7-Zip 提取程序文件。

换机器时可直接安装 [NVIDIA 官方 Windows 包](https://developer.nvidia.com/nsight-systems/get-started)。若采用本项目的解包方式，先将 [固定 2026.5.1 MSI](https://developer.nvidia.com/downloads/assets/tools/secure/nsight-systems/2026_5/nsightsystems-2026.5.1.161-3889610.msi) 保存为 `.runtime/llama-downloads/NsightSystems-2026.5.1.161-3889610.msi`，确认签名发布者为 NVIDIA Corporation（本机检查为 Valid），再运行：

```powershell
# Python 3.12（msilib）及 C:/Program Files/7-Zip/7z.exe
.venv-qwen/Scripts/python.exe -W ignore scripts/prepare-nsight.py
.venv-qwen/Scripts/python.exe -m pip install -r requirements-qwen-eval.txt
.runtime/nsight-systems/target-windows-x64/nsys.exe --version
```

MSI SHA256：`379c0a15a9cf7b8028081073fdd1d9798b9aaa618fb46c6a01785b69ed01d5f2`。解包脚本再次校验哈希，只复制 INSTALLDIR 内文件，不执行安装动作。解包不是 NVIDIA 标准安装流程，未来版本需重新验证。

## 普通基准

在项目根目录执行；`--split holdout` 另跑一次。退出日常悬浮窗释放其模型后再测，完成后用 `./desktop.ps1` 恢复。

```powershell
.venv-qwen/Scripts/python.exe -X utf8 scripts/evaluate-qwen-real.py --model .runtime/models/Qwen3-ASR-1.7B-GGUF/Qwen3-ASR-1.7B-Q8_0.gguf --tag q8-desktop-new --context 2048 --repetitions 5 --results-dir experiments/qwen_asr/performance
```

原来的量化 runner 默认仍是 1024 上下文、三次重复；旧结果和汇总不变。当前目录隔离了这次桌面参数的测量。

## Nsight 采集

```powershell
.runtime/nsight-systems/target-windows-x64/nsys.exe profile --session-new=qwen-asr-new --kill=false --trace=cuda,nvtx --sample=none --cpuctxsw=none --cuda-graph-trace=node --cuda-memory-usage=true --output=outputs/qwen-nsight/q8-new .venv-qwen/Scripts/python.exe -X utf8 scripts/evaluate-qwen-real.py --model .runtime/models/Qwen3-ASR-1.7B-GGUF/Qwen3-ASR-1.7B-Q8_0.gguf --tag q8-nsys-new --context 2048 --repetitions 3 --nvtx --nsys-session=qwen-asr-new --results-dir experiments/qwen_asr/performance
```

`--nsys-session` 必须与外层 `--session-new` 一致。runner 在 GPU 进程仍存活时停止采集，之后才清理模型；否则 Windows 强制停止会跳过 CUPTI 的正常退出路径。

导出及复核本机已保存的最终结果：

```powershell
.runtime/nsight-systems/target-windows-x64/nsys.exe export --type=sqlite --output=outputs/qwen-nsight/q8-desktop-final.sqlite outputs/qwen-nsight/q8-desktop-final.nsys-rep
.venv-qwen/Scripts/python.exe scripts/analyze-qwen-nsight.py --trace outputs/qwen-nsight/q8-desktop-final.sqlite --profile experiments/qwen_asr/performance/q8-nsys-final-pilot.json --baseline experiments/qwen_asr/performance/q8-desktop-clean-pilot.json experiments/qwen_asr/performance/q8-desktop-clean-holdout.json --output experiments/qwen_asr/performance
.venv-qwen/Scripts/python.exe scripts/test_qwen_nsight_analysis.py
.venv-qwen/Scripts/python.exe scripts/test_qwen_asr_metrics.py
```

如果 SQLite 已存在无需再次导出。分析脚本只读 trace，核对配置、权重/清单、结束原因、缓存、请求标记和日志请求数量，输出 CSV 与含来源哈希的 JSON。三个 GPU 区间并集测试和五个识别计分测试通过。

## 查看与保存

打开原始时间线：

```powershell
.runtime/nsight-systems/host-windows-x64/nsys-ui.exe outputs/qwen-nsight/q8-desktop-final.nsys-rep
```

在 Python 进程的 `qwen-asr` NVTX 域找到 `performance/<录音ID>/<重复次数>`，再查看同一时间范围的 llama-server CUDA 行。不要把 Python 标记当作 CUDA 内核内部阶段。

最终 `.nsys-rep` 为 32,291,264 字节，SHA256：`1b38f058d096eef31d17a103fe437d0064943fd0ceac0c3e992b7f6bc84f1b35`。大型 trace、SQLite、服务日志保存在 `outputs/`；Git 保存原始 benchmark JSON、派生 CSV/JSON、代码及文档。`q8-nsys-20260913-pilot.json` 是改进停止流程之前的首次采集，不进入最终热点汇总。

普通基线以 `q8-desktop-clean-pilot.json` / `q8-desktop-clean-holdout.json` 为准。首次 `q8-desktop-20260913-*` 运行与 NVTX 工具验证有少量重叠，仅保留供追溯，不进入最终延迟汇总。
