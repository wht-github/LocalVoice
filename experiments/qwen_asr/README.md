# 1.7B 量化与解码实验

先读 `docs/qwen-asr-experiment-plan.md`。这是一组固定条件下的小实验，不是新的服务框架。

1. 安装 `requirements-qwen-eval.txt` 到已有 `.venv-qwen`。
2. 运行 `scripts/prepare-qwen-real-eval.py`，校验并恢复固定 ASCEND 音频。`ascend-manifest.json` 是样本与人工转写清单，不要按结果改样本。
3. 运行 `scripts/download-qwen-gguf.py --with-bf16` 和 `scripts/quantize-qwen.py`。后者在 CPU 上从 BF16 生成三个量化文件，并将来源、哈希、大小、耗时写入 `quantization.json`。
4. 运行 `scripts/test_qwen_asr_metrics.py` 检查计分规则。
5. 每次只启动一项 `scripts/evaluate-qwen-real.py`，等它结束再启动下一项。脚本会拒绝覆盖已有同名结果，重新测量请换 tag。

示例（项目根目录）：

```powershell
.venv-qwen\Scripts\python.exe -X utf8 scripts\evaluate-qwen-real.py --model .runtime\models\Qwen3-ASR-1.7B-GGUF\Qwen3-ASR-1.7B-local-Q8_0.gguf --tag q8

# 两项独立消融，均以 Q8 参考为对照。
.venv-qwen\Scripts\python.exe -X utf8 scripts\evaluate-qwen-real.py --model .runtime\models\Qwen3-ASR-1.7B-GGUF\Qwen3-ASR-1.7B-local-Q8_0.gguf --tag q8-no-graphs --no-cuda-graphs
.venv-qwen\Scripts\python.exe -X utf8 scripts\evaluate-qwen-real.py --model .runtime\models\Qwen3-ASR-1.7B-GGUF\Qwen3-ASR-1.7B-local-Q8_0.gguf --tag q8-no-fa --flash off
```

`--split holdout` 只在 pilot 选择候选之后使用。每个结果保存在 `results/<tag>-<split>.json`；音频、模型和详细服务日志分别在 `.runtime/` 与 `outputs/qwen-real/`。每个 JSON 包含原始转写、计分分子/分母、每次性能数据与运行条件。

完整实验结束后运行 `scripts/summarize-qwen-real.py`，核对配置与 GPU 放置并重建 CSV/Markdown 简表。最终解释见 `docs/qwen-asr-quantization-results.md`。原始记录保留 pilot 退化的样本，不因推荐 Q4 而删除。

`ascend-manifest.json` 中的转写来自 [CAiRE/ASCEND](https://huggingface.co/datasets/CAiRE/ASCEND)，按 CC-BY-SA-4.0 提供；署名与版本见清单。本次选出的 60 条录音只有两位说话人，总计约 250 秒。pilot/holdout 分离不能解决说话人覆盖不足。
