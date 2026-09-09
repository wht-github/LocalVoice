# Windows 原生 PyTorch + TileLang 算子实验

这套实验已在本机 RTX 4050 Laptop GPU 上运行。环境是独立的 `.venv-tilelang`，与桌面语音应用使用的 `.venv-qwen` 分开。当前练习是 float32 向量算子，还没有替换 Qwen 模型里的计算。

## 先运行基线

在项目根目录打开 PowerShell：

```powershell
.\tilelang-lab.ps1
```

程序先检查 14 种输入长度，包括不足一个块、刚好一个块、超出一个块和百万元素，再比较 PyTorch `torch.add` 与自己编译的 TileLang 加法。负数、零和正数都会被检查；输出先填 NaN，以便发现漏写。

结果保存到 `outputs/tilelang/baseline.json`。首次执行需要编译，不计入性能测试；后续运行复用 `.runtime/tilelang-cache`。

## 你的第一个修改

打开 `experiments/tilelang/kernels.py`，找到 `add_relu_exercise` 的 TODO，把输出从：

```text
C = A + B
```

改成：

```text
C = max(A + B, 0)
```

只需要修改一行。文件里给出了 TileLang 的函数提示，但保留了这个练习让你亲手完成。`vector_add` 是已经验证的参考起点。

```powershell
# 修改前会明确报 Exercise not correct yet，这是预期行为
.\tilelang-lab.ps1 --exercise --check-only

# 修改正确后再比较：PyTorch 两次操作，对比 TileLang 一次融合操作
.\tilelang-lab.ps1 --exercise

# 然后试着改变每个块处理的元素数
.\tilelang-lab.ps1 --exercise --block 512
```

`T.Kernel` 决定启动多少块，`T.Parallel` 描述块内并行计算，`idx < N` 保护最后一个不满的块。`block` 是每块处理的元素数；当前每块线程数固定为 128，一个线程可以处理多个元素。

## 怎样读性能结果

- 两边使用预先分配的 GPU 输入和输出，排除文件读取、主机到 GPU 的传输和输出分配。
- 每种方案预热 20 次，再测 7 组、每组 100 次。每组计时前后等待 GPU 完成，报告单次平均耗时的中位数和范围。
- 这是**包含 Python 调用、后端调度和 GPU 执行的耗时**，不是 GPU 内核本身的纯执行时间。NVRTC 的 Python 包装开销可能让简单算子更慢。
- `wall_speed_ratio` 是 PyTorch 时间除以 TileLang 时间，大于 1 才表示这个测量方式下 TileLang 更快。当前加法基线没有加速，这不是安装失败。
- 本机 NVRTC 路径曾在 CUDA Graph 捕获中产生空图，因此这里不使用图捕获计时。空图不能作为加速证据。
- PyTorch 对照是 eager 模式，不代表它在 `torch.compile` 等优化后的最佳性能。不要从这个练习推断 Qwen 的整体速度。

笔记本功耗、温度和其他 GPU 应用都会影响结果。比较修改前后时使用相同输入规模和供电状态，避免同时跑识别。优先看正确性，再看耗时。

## 环境与重建

本机验证版本：Python 3.12、PyTorch 2.9.1+cu128、TileLang 0.1.14、cuda-python 13.3.1。通过环境内安装的 CUDA 13.3 编译组件和 NVRTC 编译内核；PyTorch 自身使用 CUDA 12.8 运行库。这一组合已实际运行验证，不需要 WSL。移到其他机器仍需检查驱动兼容性。

```powershell
.\scripts\setup-tilelang.ps1
# 没有现有 Qwen 环境时，指定 Python 3.12
.\scripts\setup-tilelang.ps1 -Python C:\path\to\python.exe
```

安装脚本优先复用项目缓存中的 PyTorch wheel，否则从官方 CUDA wheel 索引下载。依赖版本记录在 `.runtime/tilelang-lock.txt`，主要依赖固定在 `requirements-tilelang.txt`。

启动时可能看到 TVM 的 `Failed to JIT torch c dlpack extension` 警告。当前 NVRTC 路径使用调用者分配的 torch 张量，已验证可以正常执行；该警告涉及可选的张量分配桥接。这里没有修改第三方包来隐藏警告。

## 如何走到 Qwen 优化

完成这个练习后，下一步是对现有 Qwen 推理做性能剖析，确认实际耗时的算子、形状和数据类型。再选择一个真实热点（例如归一化或激活融合，具体以测量为准），建立 PyTorch 对照、误差检查与可切换实现，最后使用同一组录音比较识别结果和整体延迟。

入门资料：[TileLang 官方仓库](https://github.com/tile-ai/tilelang)、[安装说明](https://github.com/tile-ai/tilelang/blob/main/docs/get_started/Installation.md)、[逐元素算子示例](https://github.com/tile-ai/tilelang/blob/main/examples/elementwise/example_elementwise_add.py)。
