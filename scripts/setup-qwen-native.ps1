# Independent Windows CUDA environment for Qwen3-ASR experiments.
param(
    [string]$Python = 'python',
    [string]$TorchWheel,
    [string]$PyIndex = 'https://pypi.tuna.tsinghua.edu.cn/simple'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$venv = Join-Path $root '.venv-qwen'
$exe = Join-Path $venv 'Scripts\python.exe'
$env:PIP_CACHE_DIR = Join-Path $root '.runtime\pip-cache'
if (!(Test-Path -LiteralPath $exe)) {
    & $Python -m venv $venv
    if ($LASTEXITCODE -ne 0) { throw 'Cannot create Python environment; pass -Python with a Python 3.12 executable.' }
}
if ($TorchWheel) {
    & $exe -m pip install $TorchWheel -i $PyIndex --disable-pip-version-check
} else {
    & $exe -m pip install torch==2.9.1 --index-url https://download.pytorch.org/whl/cu128 --disable-pip-version-check
}
if ($LASTEXITCODE -ne 0) { throw 'CUDA PyTorch installation failed.' }
& $exe -m pip install -r (Join-Path $root 'requirements-qwen-native.txt') -i $PyIndex --disable-pip-version-check
if ($LASTEXITCODE -ne 0) { throw 'Qwen dependencies installation failed.' }
& $exe -m pip check
if ($LASTEXITCODE -ne 0) { throw 'Dependency check failed.' }
& $exe -c 'import torch; assert torch.cuda.is_available(), "CUDA unavailable"; x=torch.ones((128,128),device="cuda"); y=x@x; torch.cuda.synchronize(); assert y[0,0].item()==128; print(torch.__version__, torch.cuda.get_device_name(0))'
if ($LASTEXITCODE -ne 0) { throw 'GPU execution check failed.' }
& $exe -m pip list --format=freeze | Set-Content (Join-Path $root '.runtime\qwen-native-lock.txt') -Encoding utf8
if ($LASTEXITCODE -ne 0) { throw 'Could not save dependency versions.' }
Write-Host "Ready: $exe"
