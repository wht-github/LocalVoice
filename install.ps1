# Install the matching native Release, or explicitly build from source. No Python.
#Requires -Version 7.0
param(
    [switch]$CpuOnly,
    [switch]$Prebuilt,
    [switch]$LocalBuild
)
$ErrorActionPreference = 'Stop'
if ($Prebuilt -and $LocalBuild) {
    throw '-Prebuilt 与 -LocalBuild 不能同时使用；默认安装预编译 Release。'
}
$clientDirectories = @(
    (Join-Path $PSScriptRoot '.runtime/desktop'),
    (Join-Path $PSScriptRoot 'desktop/target/release')
)
$running = Get-Process -Name 'local-voice-desktop*' -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and ([IO.Path]::GetDirectoryName($_.Path) -in $clientDirectories) }
if ($running) {
    throw '请先从托盘退出此项目的 Local Voice 客户端，再更新 EXE、DLL 和模型运行库。'
}
Push-Location $PSScriptRoot
try {
    if (!$LocalBuild) {
        & "$PSScriptRoot/scripts/setup-desktop.ps1"
    } else {
        if (!(Get-Command cargo -ErrorAction SilentlyContinue)) {
            throw '本地构建需要 Rust MSVC 与 Visual Studio C++ Build Tools；不带 -LocalBuild 可直接安装预编译 Release。详见 install.md。'
        }
        & cargo build --release --locked --manifest-path desktop/Cargo.toml
        if ($LASTEXITCODE -ne 0) { throw '原生客户端构建失败。' }
    }
    & "$PSScriptRoot/scripts/setup-speech.ps1"
    # The same llama.cpp runtime also runs the CPU-only Agentic editor.
    & "$PSScriptRoot/scripts/setup-llama.ps1" -CpuOnly:$CpuOnly
    if (!$CpuOnly) {
        foreach ($size in @('1.7B', '0.6B')) {
            & "$PSScriptRoot/scripts/download-qwen-gguf.ps1" -Size $size
        }
    }
} finally { Pop-Location }
if ($LocalBuild) {
    Write-Host '安装完成。运行 ./desktop.ps1 -LocalBuild 启动本地构建版。'
} else {
    Write-Host '安装完成。运行 ./desktop.ps1 启动 Release；设置中选择已安装的模型。'
}
