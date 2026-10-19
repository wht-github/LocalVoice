# LocalVoice 安装指南

v0.4.2 提供独立的 Windows x64 便携包：下载、完整解压、双击启动即可准备模型并使用。安装与运行均无需 Python、Git、Rust 或 PowerShell 7。

## 下载与首次启动

1. 在 [GitHub Releases](https://github.com/wht-github/LocalVoice/releases/latest) 的 **Assets** 下载 `LocalVoice-v0.4.2-windows-x64.zip`。不要下载 GitHub 自动生成的 `Source code` 压缩包。
2. 右键 ZIP → “全部解压缩”，放进你有写入权限的目录，例如 `D:\Apps\LocalVoice`。不要放进 `Program Files`，不要直接在 ZIP 内打开程序。
3. 双击 **`Start LocalVoice.cmd`**。首次下载并校验基础文件，约 182 MB：SenseVoice INT8、Silero VAD 和供文字整理使用的 CPU 运行库。窗口会显示进度；完成后自动启动客户端。
4. 把光标放进需要输入的地方，按 **Ctrl+Alt+Space** 开始说话，再按一次结束。新安装默认“原样转写”和 SenseVoice CPU。

系统要求：Windows 10 / 11 x64，系统自带的 Windows PowerShell 5.1、`curl.exe` 和 `tar.exe`，首次下载可访问 GitHub。基本语音输入无需 NVIDIA 显卡。目录需要为模型、下载缓存和临时解压文件预留空间；基础安装建议至少留 1 GB，使用整理模型再预留 2 GB，使用 Qwen 则建议至少留 10 GB。

若提示缺少 `VCRUNTIME140.dll`、`MSVCP140.dll` 或 VC++ 运行库，先安装 [Microsoft 官方 Visual C++ x64 运行库](https://aka.ms/vc14/vc_redist.x64.exe)，再启动。安装 VC++ 可能需要管理员权限；LocalVoice 自身不需要以管理员身份运行。当前发布包尚未签名，Windows 可能显示发布者或 SmartScreen 提示，请核对下载来源；由组织策略阻止时联系设备管理员。

后续直接双击 `Start LocalVoice.cmd`，已准备的文件会复用。模型齐全后识别和整理可离线运行。

## 开启日常整理和笔记

1. 打开悬浮球 → **设置**。
2. 在“口语整理与笔记”中点击 **“下载并准备整理模型”**，额外下载约 688 MB 的 AgenticASR-Refiner Q4_K_M。下载完成前可继续使用原样转写。
3. 选择 **“日常输入”** 或 **“笔记”**。

两种模式共用一份模型，使用 CPU；前轮本机实测工作集约 1.20 GiB。下载文件大小不等于运行内存。v0.4.2 在开始录音时后台准备已安装的整理模型，让加载与说话重叠；录音期间保持可用，结束后恢复空闲 60 秒释放。模型、提示词和完整输出预算保持不变；短录音仍可能等待加载，正文生成也需要时间。

默认说完后手动结束录音，应用整理完整文字并自动输入，内部停顿不提交。笔记不使用摘要流程。整理失败、过长或输出不完整时保留全部转写。术语、数字和改口仍可能被模型处理错误，“最近结果”可查看原文与正文、复制或重新整理。笔记对照窗口默认不弹出，可在设置中开启调试选项。

v0.4.2 支持日常连续输入：设置中开启“连续输入”，开始前选好输入框；停顿至少 1.2 秒后，整段识别和整理完再输入。切换输入目标会停止录音并保留后续结果。前段已经输入的文字不会自动重写，需要跨段改口或统一整理长内容时关闭此项。笔记仍由你手动结束。

## 可选：安装 Qwen 识别

基础 SenseVoice 可以先用起来。有兼容 NVIDIA 显卡与驱动时，可尝试 Qwen ASR：

1. 双击 **`Setup Models.cmd`**，或在应用设置中点击 **“安装 / 管理模型”**。
2. 选择 **Qwen ASR 0.6B** 或 **Qwen ASR 1.7B**。只下载选中的一套；首次还会准备 CUDA 运行库。
3. 完成后在应用设置中选择对应识别模型，点击底部 **“保存”**；保存后会加载所选模型。若选择未变、只是补齐缺少的文件，可点击“加载已选模型”。

| 组件 | 下载大小 |
| --- | --- |
| Qwen ASR 0.6B Q8 与音频编码器 | 约 1.18 GB |
| Qwen ASR 1.7B Q8 与音频编码器 | 约 2.81 GB |
| 两者共用的 llama.cpp CUDA 运行库 | 约 540 MB |

附带的运行库使用 CUDA 13.3，需要兼容的 NVIDIA 驱动，无需安装 CUDA Toolkit。下载成功不代表显卡和驱动兼容；若加载失败，先切回 SenseVoice CPU，再查看 `.runtime/native-asr.log`。不要同时运行多个客户端。

模型来自固定版本的发布地址并校验 SHA256。VAD、SenseVoice 与运行库来自 GitHub；Qwen 和 Agentic 来自 Hugging Face。网络中断后重新运行安装入口，已校验的完整下载会复用；未完成的文件可能需要重新下载。

## 更新与保留模型

从托盘退出 LocalVoice，将新版 ZIP 完整解压并覆盖到原目录，保留 **`.runtime` 整个文件夹**，再双击 `Start LocalVoice.cmd`。发布 ZIP 不含用户设置或模型，因此不会通过解压覆盖这些数据。正在运行时不要覆盖程序或 DLL。

**v0.4.2 的行为变化：** 已有“连续输入”偏好会保留；原先开启的用户切到日常输入时，也会在停顿后分段整理并输入。希望手动结束后统一整理时，请在设置中关闭“连续输入”。笔记仍手动结束，不修改该偏好。此次模型不变，已有模型无需重新下载。

也可解压到新目录，再把旧目录中的 `.runtime` 复制过去。确认新版本可用前保留旧目录。不要把 EXE 单独移走，程序需要同目录 DLL、脚本和 `localvoice.portable` 标记。

### 从旧版源码目录迁移

v0.4.0 的发布 ZIP 仍依赖仓库目录；v0.4.1 起已提供独立便携包。升级到 v0.4.2 时：

1. 退出旧版客户端，完整解压新版到一个新目录。
2. 把旧仓库的 `.runtime` 复制到新目录，与 `local-voice-desktop.exe` 同级。已有模型和设置可继续使用；旧的 `.runtime/desktop` 可保留，新版入口不使用它。
3. 双击新目录的 `Start LocalVoice.cmd`。缺少的基础组件会补齐；已有模型和模式设置不改写。

不要把旧 `.venv`、Python 环境或整个源码仓库搬进新安装目录。保留源码工作流的开发者仍可使用下方的仓库安装方式。

## 文件放在哪里

| 路径 | 内容 |
| --- | --- |
| `local-voice-desktop.exe` 与同目录 DLL | 客户端和原生语音运行库 |
| `Start LocalVoice.cmd` / `Setup Models.cmd` | 启动与模型安装入口 |
| `localvoice.portable` / `scripts/` | 便携目录标记与模型准备脚本 |
| `.runtime/settings.json` | 模型、麦克风、快捷键和输出模式设置 |
| `.runtime/models/` | Silero、SenseVoice、可选 Qwen 和 Agentic 权重 |
| `.runtime/llama-cpu/` / `.runtime/llama/` | CPU / CUDA 推理运行库 |
| `.runtime/downloads/` | 下载缓存与临时文件 |
| `.runtime/native-asr.log` / `.runtime/native-text.log` | 模型运行日志 |
| `.runtime/timings.jsonl` | 有大小上限的耗时记录，不含转写正文 |

录音和转写不自动保存，最近结果退出后清空。卸载时先退出应用，再删除便携目录即可；这也会删除目录内的模型与设置。

## 常见问题

- **双击后打不开或安装失败**：从 `Start LocalVoice.cmd` 启动并查看窗口提示；确认已完整解压且目录可写。缺少 VC++ 时使用上方微软链接安装。
- **缺少 sherpa 或 ONNX Runtime DLL**：重新完整解压发布 ZIP，保留 EXE 同目录全部 DLL。
- **提示没有模型**：运行 `Setup Models.cmd` 安装基础组件；Qwen 需要另外选择对应大小。整理模型在应用设置中下载。
- **下载无法连接**：检查是否能访问 GitHub / Hugging Face。高级用户可设置 `HF_ENDPOINT` 为自行信任的兼容镜像，然后从同一终端运行安装脚本或启动应用；仍会验证固定文件哈希。
- **没有输入到目标应用**：确认录音结束时光标在输入框，等待期间不要切换目标；从“最近结果”复制文字。部分以管理员权限运行的程序可能限制普通进程输入。
- **没有识别到轻声**：检查 Windows 麦克风权限和应用中的麦克风选择。Silero VAD 判断人声，但不提供音频降噪，也不保证所有环境下都不漏检。
- **需要停顿后自动输入**：在设置中开启“连续输入”，适用于原样转写和日常输入，笔记仍手动结束。连续日常输入至少等待 1.2 秒停顿，再完整识别、整理一段后交付，不会改写已输入文字。

## 开发者：从源码目录安装

以下流程仅供开发者或已有源码工作目录的用户。需要 Git 和 PowerShell 7；源码编译另需 Rust MSVC 与 Visual Studio C++ 构建工具。

```powershell
git clone --branch v0.4.2 https://github.com/wht-github/LocalVoice.git
cd LocalVoice
./install.ps1 -CpuOnly
./desktop.ps1
```

仓库安装把预编译客户端放进 `.runtime/desktop`，继续使用仓库根的 `.runtime`。不带 `-CpuOnly` 的旧安装流程会准备两套 Qwen，普通用户应使用便携包的按需菜单。

源码构建使用 `./install.ps1 -LocalBuild -CpuOnly`，然后 `./desktop.ps1 -LocalBuild`。维护者提交后运行 `./scripts/package-desktop.ps1`，生成 ZIP 与 SHA256SUMS。开发检查见 [GitHub 桌面开发说明](https://github.com/wht-github/LocalVoice/blob/master/desktop/README.md)。
