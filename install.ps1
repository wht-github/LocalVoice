# Run from a clone with uv and Python already installed by the user.
#Requires -Version 7.0
param(
    [switch]$CpuOnly,
    [string]$PyIndex = 'https://pypi.org/simple'
)
$ErrorActionPreference = 'Stop'
& "$PSScriptRoot/scripts/setup-native-asr.ps1" -PyIndex $PyIndex
& "$PSScriptRoot/scripts/setup-desktop.ps1"
$python = Join-Path $PSScriptRoot '.venv/Scripts/python.exe'
Push-Location $PSScriptRoot
try {
    # Import config before huggingface_hub so the cache remains inside the clone.
    & $python -X utf8 -c 'import asr_server as s; from huggingface_hub import snapshot_download; print(snapshot_download("FunAudioLLM/SenseVoiceSmall", revision=s.SENSEVOICE_REVISION))'
    if ($LASTEXITCODE -ne 0) { throw 'SenseVoice 模型下载失败。' }
    if (!$CpuOnly) {
        & "$PSScriptRoot/scripts/setup-llama.ps1"
        foreach ($size in @('1.7B', '0.6B')) {
            & $python -X utf8 scripts/download-qwen-gguf.py --size $size
            if ($LASTEXITCODE -ne 0) { throw "Qwen $size 下载失败。" }
        }
    }
} finally { Pop-Location }
Write-Host '安装完成。运行 ./desktop.ps1 启动悬浮窗；设置中选择已安装的模型。'
