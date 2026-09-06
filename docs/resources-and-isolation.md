# SenseVoiceSmall + Kokoro：资源与隔离记录

以下为 Kokoro 历史基线。当前已换成 MeloTTS；新测量见 [Melo 与服务管理](melo-and-service-control.md)，原始数据为 `outputs/resource-profile-melo.json`。本次没有修改以下服务组额度或任何 WSL 全局配置。

2026-09-06，本机 Ryzen 7 7840H（8 物理核心 / 16 逻辑处理器）、RTX 4050 Laptop 6 GB。两个服务都使用 CPU 版 PyTorch，各为 4 个计算线程，采用系统默认同等调度权重。曾短暂将 TTS 降为 2 个线程并偏重 ASR，现按用户“不必严格限制竞争”的偏好取消。两个服务合计的资源上限保留。GPU 推理尚未安装或实测，后续结果见 [长混读测试](long-mixed-asr-test.md)。

## 实测与资源建议

同时启动 ASR 与 TTS，各连续请求 3 次，150 ms 周期采样。ASR 输入为 25 秒合成语音，TTS 输入为约百字符中英混排文本。这是资源负载测试，未覆盖 2000 字符 TTS 上限等所有极端输入。

| 项目 | 限额启用前实测 |
|---|---:|
| SenseVoice 已加载，进程 PSS | 1.59 GiB |
| Kokoro 已加载，进程 PSS | 1.65 GiB |
| 两个进程 PSS 合计，已加载 | 3.24 GiB |
| 并发请求时两个进程 PSS 合计峰值 | 3.66 GiB |
| 本轮 Linux MemTotal−MemAvailable 峰值 | 4.06 GiB |
| CPU 短时峰值 | 7.96 个逻辑处理器的满负荷等效值 |
| 测试全过程平均 CPU | 4.24 个逻辑处理器的满负荷等效值 |
| 当前两个模型的 GPU 显存 | 0 |

PSS 按比例计入共享页。Windows 中 WSL 的总占用还包含文件缓存、内核和其他实例，不能直接等同于模型 PSS。隔离后重新启动两个服务，服务组记录到约 **5.98 GiB** 内存峰值，该口径包含组内文件缓存，与 PSS 不同。因此建议给服务组 **8 GiB 上限**，不建议把整个环境卡在 4 GB。

本次采用 6 个逻辑处理器的 CPU 时间额度兼顾并发和宿主余量，不是独占 6 个物理核心。4 个逻辑处理器可作为更省资源的后续试验值，本轮没有验证该限额的体验。当前 0 显存只适用于 CPU 部署，Windows 已有的显存使用不属于这两个模型。

原始基线：Windows 项目 `outputs/resource-profile.json`。隔离后的同方法复测：LocalVoice `/opt/local-voice-app/outputs/resource-profile.json`，两个进程 PSS 合计峰值 3.64 GiB，25 秒音频识别约 1.48–1.82 秒，测试长句 TTS 约 6.14–7.87 秒。CPU quota 是周期内配额，跨周期的短采样可能略超过 6，不是 CPU 亲和性限制。

## 已应用，仅限 LocalVoice

`/etc/wsl.conf` 保留 systemd 和默认 voice 用户，添加：

```ini
[automount]
enabled=false
mountFsTab=false

[interop]
enabled=false
appendWindowsPath=false
```

配置前文件保存在 `/etc/wsl.conf.before-local-voice-isolation`。

`/etc/systemd/system/localvoice.slice` 对两个服务合计设置：

```ini
[Slice]
CPUQuota=600%
MemoryHigh=6G
MemoryMax=8G
```

MemoryHigh 是回收/限速阈值，不是预分配；MemoryMax 是组内硬上限，达到且无法回收时可能触发该组 OOM。其他 Linux 进程、安装任务、部分内核开销不在此服务组内。没有改变整个 WSL VM 的总内存或可见 CPU 数量，也未改变 swap 或预留 GPU 显存。

两个服务的 `service.d/isolation.conf` 增加：禁止获取新权限、清空 capabilities、系统及 home 只读、独立临时目录、隐藏宿主挂载入口；只允许写模型运行缓存、服务状态和测试输出目录。GPU 驱动接口保留，方便之后明确切换 GPU 部署。

关闭自动挂载不等于禁止 Linux root 手动挂载。Windows 管理端仍可使用 `wsl.exe` 及 WSL 文件访问；这是减少日常互通与服务访问范围，不能成为与宿主相互不信任的独立虚拟机。两个 HTTP 服务继续只监听回环地址；没有隔绝镜像下载。

## 全局配置与验证

`%UserProfile%\.wslconfig` 修改前后哈希相同。未调整全局 memory、processors、swap、networkingMode、localhostForwarding、guiApplications 等，也未执行 `wsl --shutdown`。只重启 LocalVoice，archlinux 和 podman-machine-default 没有配置改动。

已验证 Windows HTTP 可达、中英文合成与识别成功、Windows 盘未挂载、Windows 互操作注册不存在。中文回环文字一致，英文仍有既有的 Speech→Spee 偏差。资源复测后检查服务组的 OOM、硬上限事件，结果保存于项目 `outputs/isolation-verification.txt`。

## 运维与恢复

`voice.ps1 start/stop/status/logs/test` 改为使用实例内部路径；`voice.ps1 deploy` 通过显式 stdin 文件传输更新源码，关闭盘符挂载后仍可用。它会重新应用隔离和额度；手工调整额度时应同步 `scripts/isolate-localvoice.sh`，避免下次部署覆盖。

仅恢复原 Windows 互通配置：

```powershell
wsl -d LocalVoice -u root --cd / --exec cp /etc/wsl.conf.before-local-voice-isolation /etc/wsl.conf
.\voice.ps1 stop
.\voice.ps1 start
```

这不取消服务自身的权限与资源额度。服务额度在 slice 文件中调整，随后重新加载 systemd 并停止/启动服务。若要修改整个 WSL VM 的资源上限，应另行确认，因为会影响其他 WSL2 实例。

参考：[微软 WSL 全局与单发行版配置说明](https://learn.microsoft.com/en-us/windows/wsl/wsl-config)。
