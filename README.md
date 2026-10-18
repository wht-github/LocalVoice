# LocalVoice · 本地语音输入

**按下快捷键，把口述变成文字。** 在 Windows 上用本地模型完成中英混合语音识别、日常轻整理和笔记输入。

[**下载 Windows x64 版 v0.4.1**](https://github.com/wht-github/LocalVoice/releases/download/v0.4.1/LocalVoice-v0.4.1-windows-x64.zip) · [版本说明](https://github.com/wht-github/LocalVoice/releases/latest) · [安装指南](install.md)

模型准备好后可离线使用，无需账号、API Key、Python 或开发环境。

![LocalVoice 的悬浮操作面板](docs/images/dictation-panel.png)

## 你可以用它做什么

- **随处语音输入**：将光标放进输入框，按 `Ctrl+Alt+Space` 开始说话，再按一次结束，识别完成后自动输入。
- **中英混合口述**：中文里夹带英文名称、技术术语和缩写。提供 SenseVoice CPU 与可选 Qwen ASR，无需先维护词表。
- **日常轻整理**：尝试去掉口头语和重复，让消息与工作记录更易读。
- **完整笔记正文**：说完一段想法，手动结束后整理并输入；以保留原意、形成连贯正文为目标，不使用摘要流程。
- **随手唤起**：置顶悬浮球、全局快捷键和系统托盘。最近结果可查看完整原文与正文、复制或重新整理。
- **本地处理**：录音、识别和整理都在电脑上完成。轻量 Silero VAD 判断人声，整理模型按需加载、空闲 60 秒释放。

目前是可试用版本。识别和整理仍可能改错术语、数字或口头修正，建议核对重要内容；新安装默认使用“原样转写”。整理失败、超长或输出截断时回退完整转写。

## 三步开始使用

1. **下载并完整解压**上面的 Windows ZIP，例如解压到 `D:\Apps\LocalVoice`。不要在压缩包里直接运行，也不要只取出 EXE。
2. **双击 `Start LocalVoice.cmd`**。首次会打开安装窗口，下载并校验基础组件，约 **182 MB**；完成后启动应用。后续双击直接启动。
3. **把光标放进输入框，按 `Ctrl+Alt+Space`**。说完再按一次，等待文字输入。也可以点击悬浮球里的“开始说话”。

需要 Windows 10 / 11 x64、可写的安装目录，以及首次下载时的网络连接。默认方案无需独立显卡，也无需 Git、Rust、PowerShell 7、CUDA Toolkit 或 Python。若提示缺少 VC++ 运行库，安装 [Microsoft 官方 x64 运行库](https://aka.ms/vc14/vc_redist.x64.exe) 后重试。详细步骤和常见问题见 [安装指南](install.md)。

## 按需准备模型

先完成基础安装，就能使用原样转写。其余能力按需要开启，不会一次下载所有模型。

| 想使用的能力 | 如何准备 | 首次额外下载 |
| --- | --- | --- |
| CPU 语音输入 | 首次启动自动准备 SenseVoice INT8、Silero VAD 和 CPU 推理运行库 | 合计约 **182 MB** |
| 日常输入、笔记整理 | 设置 → “下载并准备整理模型”，完成后选择相应输出模式 | AgenticASR-Refiner 约 **688 MB**，两种模式共用 |
| Qwen ASR 0.6B | 双击 `Setup Models.cmd` 选择 0.6B，再在应用设置中切换识别模型 | 权重约 **1.18 GB** + CUDA 运行库约 **540 MB** |
| Qwen ASR 1.7B | 双击 `Setup Models.cmd` 选择 1.7B，再在应用设置中切换识别模型 | 权重约 **2.81 GB** + 同一套 CUDA 运行库 |

Qwen 路线需要兼容所附 CUDA 13.3 运行库的 NVIDIA 显卡与驱动。两套 Qwen 可二选一；安装另一套时复用已准备的 CUDA 运行库。模型下载大小不等于内存占用：Agentic 在前轮本机测试中工作集约 **1.20 GiB**，使用 CPU，不占模型显存。具体表现取决于电脑与口述内容，详见 [模型实测与限制](docs/dictation-evaluation.md)。

## 录音与输入怎么工作

| 模式 | 结束方式 | 得到什么 |
| --- | --- | --- |
| 原样转写 | 默认手动结束；可开启停顿后分段提交 | 识别后的原文 |
| 日常输入 | 手动结束 | 经过轻整理的完整文字 |
| 笔记 | 手动结束 | 整理后的完整正文 |

日常输入和笔记里的停顿不会提前提交。结束后，如果等待期间输入目标发生变化，结果会留在应用里供复制。笔记默认不弹调试对照窗口；需要时可在设置中开启，也可随时打开“最近结果”。

VAD 用于判断人声，不是音频降噪器。自然轻声、键盘声和房间噪声的效果还需要结合实际麦克风试用。

## 数据与更新

模型、设置和运行日志都保存在程序旁的 `.runtime` 文件夹。录音和转写不自动保存；最近结果仅驻留内存，退出后清空。模型下载需要联网，正常识别与整理不发送音频或正文到云端。

更新时先从托盘退出旧版，将新版 ZIP 完整解压并覆盖到同一目录，**保留 `.runtime`**，再双击 `Start LocalVoice.cmd`。已有模型和设置会复用。v0.4.0 及更早的源码目录安装迁移方式见 [安装指南](install.md#从旧版源码目录迁移)。

## 开发与项目进展

应用使用 Rust + Slint，录音与 VAD 在原生客户端中执行，SenseVoice 使用 sherpa-onnx，Qwen 与 Agentic 使用由应用管理的 llama.cpp 进程。安装与使用均不依赖 Python；仓库中的历史研究代码不属于应用运行链路。

- [源码构建与调试](desktop/README.md)
- [当前结构与原生运行验证](docs/native-runtime.md)
- [产品规划与下一步需求](docs/product-roadmap.md)
- [识别模型选型](docs/asr-engine-selection.md)

```powershell
cargo test --manifest-path desktop/Cargo.toml --locked
./install.ps1 -LocalBuild -CpuOnly
./desktop.ps1 -LocalBuild
```

源码构建需要 Git、PowerShell 7、Rust MSVC 和 Visual Studio C++ 构建工具。普通用户直接下载上面的 ZIP 即可。第三方许可证随发布包提供。

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)
