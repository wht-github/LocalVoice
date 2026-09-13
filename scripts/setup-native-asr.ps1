# Shared native environment: SenseVoice CPU and the HTTP adapter for llama.cpp.
# Qwen runs in llama-server.exe; this environment does not need CUDA PyTorch.
param(
    [string]$PyIndex = 'https://pypi.tuna.tsinghua.edu.cn/simple'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$venvDir = Join-Path $env:LOCALAPPDATA 'LocalVoice\venv'
$python = Join-Path $venvDir 'Scripts\python.exe'

# Locate a system Python 3.12-3.13 (versioned names cover uv-managed installs).
$base = $null
foreach ($candidate in @('py', 'python', 'python3.13', 'python3.12')) {
    try {
        $version = & $candidate -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")' 2>$null
        if ($LASTEXITCODE -eq 0 -and $version -match '^3\.1[2-3]$') { $base = $candidate; break }
    } catch { }
}
if (-not $base) { throw '未找到 Python 3.12-3.13，请先安装 Python（勾选 Add to PATH）。' }

if (!(Test-Path -LiteralPath $python)) {
    & $base -m venv $venvDir
    if ($LASTEXITCODE -ne 0) { throw '创建 venv 失败。' }
}
& $python -m pip install --upgrade pip -i $PyIndex
if ($LASTEXITCODE -ne 0) { throw '升级 pip 失败。' }

# CPU-only torch must come from the PyTorch wheel index before funasr pulls deps,
# otherwise pip resolves the multi-gigabyte CUDA build from PyPI.
& $python -m pip install torch torchaudio --index-url https://download.pytorch.org/whl/cpu
if ($LASTEXITCODE -ne 0) { throw '安装 CPU 版 torch 失败。' }
& $python -m pip install -r (Join-Path $root 'requirements-native-asr.txt') -i $PyIndex
if ($LASTEXITCODE -ne 0) { throw '安装识别服务依赖失败。' }
& $python -m pip check
if ($LASTEXITCODE -ne 0) { throw '依赖一致性检查失败。' }
Write-Host "原生识别环境已就绪：$python"
Write-Host '三个识别后端共用此环境；SenseVoiceSmall 首次启动时自动下载，Qwen GGUF 由 download-qwen-gguf.py 准备。'
