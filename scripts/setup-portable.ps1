# Portable Windows setup. Uses Windows PowerShell 5.1, curl and tar; no developer tools.
#Requires -Version 5.1
[CmdletBinding()]
param([ValidateSet('Menu', 'Basic', 'Qwen06', 'Qwen17', 'Start')][string]$Mode = 'Menu')

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$marker = Join-Path $root 'localvoice.portable'
if (!(Test-Path -LiteralPath $marker -PathType Leaf) -or
    (Get-Content -LiteralPath $marker -Raw -Encoding UTF8).Trim() -cne 'LocalVoice portable v1') {
    throw '未找到有效的便携版标记。请完整解压 LocalVoice ZIP，再运行根目录中的 CMD 文件。'
}
if (![Environment]::Is64BitOperatingSystem) { throw '此版本需要 64 位 Windows 10 / 11。' }
$client = Join-Path $root 'local-voice-desktop.exe'
foreach ($file in @('local-voice-desktop.exe', 'sherpa-onnx-c-api.dll', 'sherpa-onnx-cxx-api.dll', 'onnxruntime.dll', 'onnxruntime_providers_shared.dll')) {
    if (!(Test-Path -LiteralPath (Join-Path $root $file) -PathType Leaf)) {
        throw "便携包缺少 $file。请将整个 ZIP 解压到同一文件夹，不要单独移动 EXE。"
    }
}

function Test-Files([string[]]$Paths) {
    foreach ($path in $Paths) {
        if (!(Test-Path -LiteralPath (Join-Path $root $path) -PathType Leaf)) { return $false }
    }
    return $true
}
function Test-SpeechReady {
    return Test-Files @('.runtime/models/silero-vad/silero_vad.onnx', '.runtime/models/sensevoice-int8/model.int8.onnx', '.runtime/models/sensevoice-int8/tokens.txt')
}
function Test-EditorRuntimeReady {
    return Test-Files @('.runtime/llama-cpu/llama-server.exe', '.runtime/llama-cpu/llama-server-impl.dll', '.runtime/llama-cpu/llama.dll', '.runtime/llama-cpu/ggml.dll', '.runtime/llama-cpu/ggml-base.dll', '.runtime/llama-cpu/ggml-cpu-x64.dll', '.runtime/llama-cpu/release.json')
}
function Test-VisualCppRuntime {
    $system = [Environment]::GetFolderPath('System')
    foreach ($file in @('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll')) {
        if (!(Test-Path -LiteralPath (Join-Path $system $file)) -and !(Test-Path -LiteralPath (Join-Path $root $file))) {
            throw "Windows 缺少 Microsoft Visual C++ x64 运行库（$file）。请从微软安装后重试：https://aka.ms/vc14/vc_redist.x64.exe"
        }
    }
}
function Install-Basic {
    Write-Host '准备基础功能：SenseVoice 识别、Silero 人声检测、CPU 文字整理运行库。'
    Write-Host '首次约下载 182 MB；已安装的文件会复用。'
    if (Test-SpeechReady) { Write-Host 'SenseVoice 和 Silero 已安装，复用现有模型。' }
    else { & (Join-Path $PSScriptRoot 'setup-speech.ps1') }
    # This script reuses the pinned runtime when already installed, even while it is in use.
    & (Join-Path $PSScriptRoot 'setup-llama.ps1') -CpuOnly
    Write-Host ''
    Write-Host '基础功能已就绪。日常轻整理 / 笔记需要在 App 设置中再下载 Agentic（约 688 MB）。'
    Write-Host '模型下载后可离线使用。模型和运行库保存在当前目录的 .runtime 中。'
}

if ($Mode -eq 'Start') {
    $running = @(Get-Process -Name 'local-voice-desktop*' -ErrorAction SilentlyContinue)
    if ($running.Count) {
        Write-Host 'LocalVoice 已在运行，请从系统托盘打开；切换版本前请先从托盘退出。'
        return
    }
}
Test-VisualCppRuntime

if ($Mode -eq 'Menu') {
    Write-Host 'LocalVoice 模型安装'
    Write-Host ''
    Write-Host '1. 基础 CPU 版（推荐）：约 182 MB，无需独立显卡'
    Write-Host '2. Qwen3-ASR 0.6B：NVIDIA 显卡，额外约 1.18 GB 模型 + 540 MB 运行库'
    Write-Host '3. Qwen3-ASR 1.7B：NVIDIA 显卡，额外约 2.81 GB 模型 + 540 MB 运行库'
    Write-Host '0. 退出'
    Write-Host ''
    Write-Host '日常轻整理和笔记共用 Agentic（约 688 MB），请在 App 设置中下载。'
    Write-Host '选项 2 / 3 只安装所选 Qwen，不会自动下载另一个大小。'
    $choice = Read-Host '输入选项，直接回车选择 1'
    switch ($choice.Trim()) {
        '0' { return }
        '' { $Mode = 'Basic' }
        '1' { $Mode = 'Basic' }
        '2' { $Mode = 'Qwen06' }
        '3' { $Mode = 'Qwen17' }
        default { throw '选项无效。请重新运行 Setup Models.cmd，输入 0、1、2 或 3。' }
    }
}

if ($Mode -eq 'Start') {
    if (!(Test-SpeechReady) -or !(Test-EditorRuntimeReady)) { Install-Basic }
    # This is the interactive application the user chose to open; model helpers remain hidden.
    Start-Process -FilePath $client -WorkingDirectory $root
    return
}

Install-Basic
if ($Mode -in @('Qwen06', 'Qwen17')) {
    $size = if ($Mode -eq 'Qwen06') { '0.6B' } else { '1.7B' }
    Write-Host "准备可选 Qwen3-ASR $size。请确保 NVIDIA 驱动支持此 CUDA 运行库。"
    & (Join-Path $PSScriptRoot 'setup-llama.ps1')
    & (Join-Path $PSScriptRoot 'download-qwen-gguf.ps1') -Size $size
    Write-Host "Qwen3-ASR $size 已安装。在 App 设置中选择对应识别模型并启动即可。"
}
Write-Host '安装完成。双击 Start LocalVoice.cmd 开始使用。'
