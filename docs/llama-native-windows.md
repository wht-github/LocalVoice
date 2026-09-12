# llama.cpp 原生 Windows 开发环境

这套环境用于在 RTX 4050 Laptop（6 GB，SM 8.9）上编译、运行和修改 llama.cpp / GGML CUDA 后端。现有 PyTorch 和 TileLang 环境继续作为对照。

## 组成

| 部分 | 配置 |
| --- | --- |
| C/C++ 编译器 | VS2026 18.10.0，x64 MSVC 19.51.36257.0 |
| CUDA 编译器 | NVCC 13.4.59（CUDA 13.4） |
| CUDA 库 | Runtime 13.4.49、cuBLAS 13.7.0.27 |
| 构建工具 | CMake 4.4.3、Ninja 1.13.2 |
| 工具环境 | `.venv-llama-build/` |
| 源码 | `external/llama.cpp/`，独立 Git 仓库 |
| 构建输出 | `.runtime/llama-build/bin/` |

初次下载的上游提交为 `eafe15a5e3d87dd68ae33acf6a7cbd9415a0ac5e`。安装脚本首次克隆上游 HEAD，已有源码不自动更新；每次成功构建会在构建目录记录 `source-commit.txt`，比较实验时应记录并固定源码版本。

这里的 Python 虚拟环境仅用于分发开发工具，不是 llama.cpp 的推理依赖。NVCC 编译 CUDA 设备代码，MSVC 编译宿主 C++ 代码，Ninja 负责组织构建任务。使用 Ninja 不需要安装 CUDA 的 Visual Studio MSBuild 集成组件。

## 安装和构建

VS Installer 中需要“使用 C++ 的桌面开发”，包含 x64 MSVC 和 Windows SDK。等待 VS2026 更新完成，再运行构建；脚本会拒绝使用安装尚未完成的 VS 实例。

从项目根目录运行：

```powershell
# 工具准备，可在 VS 更新期间运行；当前机器已经执行完成。
.\scripts\setup-llama-native.ps1

# VS 更新完成后构建；默认最多 4 个编译任务。
.\scripts\build-llama-native.ps1

# 构建并运行上游 CUDA 矩阵乘法正确性测试。
.\scripts\build-llama-native.ps1 -VerifyCuda
```

安装脚本优先使用现有 `.venv-qwen` 的 Python，也可传入 `-Python C:\path\to\python.exe`；可通过 `-PyIndex https://pypi.org/simple` 改用官方 PyPI。依赖版本保存在根目录 `requirements-llama-build.txt`。

Windows cuBLAS wheel 缺少 `cublas.lib` 和 `cublasLt.lib`。辅助脚本从 NVIDIA 官方归档获取对应版本，核对 NVIDIA 发布的 SHA-256 后，仅提取这两个导入库到工具环境。下载缓存保存在 `.runtime/llama-downloads/`。

构建目标包含 `llama-cli`、`llama-server`、`llama-quantize`、`llama-bench` 和支持音频输入的 `llama-mtmd-cli`。首次构建需要编译较多 CUDA 源码，之后会增量编译。显存容量不决定编译内存需求；遇到系统内存紧张可使用 `-Jobs 2`。

`-VerifyCuda` 额外构建上游 `test-backend-ops`，筛选 FP16/Q8_0 权重与 FP32 输入的矩阵乘法用例，在 CUDA0 上计算并与 CPU 对照。验证日志保存在 `.runtime/llama-build/cuda-verification.txt`，设备信息保存在同目录的 `devices.txt`。脚本也会检查实际存在 CUDA 设备，以及至少一个矩阵乘法用例确实执行成功。

## 日常使用

在当前 PowerShell 中载入 DLL 路径：

```powershell
. .\scripts\enter-llama-env.ps1 -RuntimeOnly
llama-cli --list-devices
llama-quantize --help
llama-mtmd-cli --help
```

要手动编译 CUDA/C++，省略 `-RuntimeOnly`，脚本会额外载入 MSVC 开发环境。上述环境变量只影响当前 PowerShell 及其子进程。单独打开另一个终端需要重新载入。

默认编译架构为 `89`，适合当前 RTX 4050。换用其他 NVIDIA 显卡时通过 `-CudaArchitectures` 设置相应架构；跨平台迁移时保留源码和 CMake 构建思路，重新安装对应平台的编译器并构建。

为避免额外引入 OpenSSL，本次构建关闭了 HTTPS 支持。使用本地模型文件和本机 HTTP 服务；不要依赖可执行文件内的 HTTPS 模型下载功能。

## 验证范围与后续实验

2026-09-12 已完成：安装脚本全流程、Python 包依赖检查、CMake/Ninja/NVCC 版本检查、官方 cuBLAS 归档 SHA-256 校验、CUDA 枚举到 1 个设备、CUDA 上下文初始化以及 cuBLAS 句柄创建/销毁。CUDA 调用均返回成功。当前驱动与运行库 API 版本查询均返回 `13030`，NVCC 报告 `13.4.59`；保留这些原始版本信息供后续编译排查。

VS2026 更新完成后，已使用 MSVC 19.51.36257.0 + NVCC 13.4.59 完成 Release 构建，CUDA 架构为 89。上述五个可执行文件全部生成，`llama-cli --list-devices` 正确列出 RTX 4050 Laptop，显存 6140 MiB。CLI 版本输出确认使用 MSVC 编译；各工具帮助页面均能正常加载（上游 `llama-quantize --help` 按程序约定返回退出码 1）。

上游 CUDA 矩阵乘法测试 **55/55 通过**，覆盖 FP16、Q8_0、不同列数与批次/布局，没有不支持或失败的用例。日志中的 `2/2 backends passed` 包含被筛选跳过的 CPU 测试入口；实际验证对象是 CUDA0，其计算结果与 CPU 参考实现比较。

编译存在上游警告，包括中文 Windows 代码页引发的 C4819、数值转换提示及单卡环境缺少 NCCL 的提示；本次没有修改上游源码。工具链和 GPU 算子验证已完成。

随后已下载并校验 Qwen3-ASR 1.7B 的 Q8_0 主模型和 BF16 音频编码器，完成四组音频的原生转写基线；结果见 `docs/qwen-llama-baseline.md`。桌面悬浮窗后端尚未切换。合成音频的稳定输出和延迟测试不能替代真实录音准确率评测。

参考：[NVIDIA Windows 安装与编译器支持](https://docs.nvidia.com/cuda/cuda-installation-guide-microsoft-windows/index.html)、[CUDA 13.4.1 组件清单与校验值](https://developer.download.nvidia.com/compute/cuda/redist/redistrib_13.4.1.json)、[llama.cpp 构建说明](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md)。
