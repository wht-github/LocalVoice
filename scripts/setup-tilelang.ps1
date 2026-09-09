param(
    [string]$Python,
    [string]$TorchWheel,
    [string]$PyIndex = 'https://pypi.tuna.tsinghua.edu.cn/simple'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root '.venv-tilelang\Scripts\python.exe'
$env:PIP_CACHE_DIR = Join-Path $root '.runtime\pip-cache'
$env:TVM_FFI_DISABLE_TORCH_C_DLPACK = '1'
if (!$Python) {
    $Python = Join-Path $root '.venv-qwen\Scripts\python.exe'
    if (!(Test-Path -LiteralPath $Python)) { $Python = 'python' }
}
if (!(Test-Path -LiteralPath $exe)) {
    & $Python -c 'import sys; assert sys.version_info[:2] == (3,12), "Please supply Python 3.12 with -Python"'
    if ($LASTEXITCODE -ne 0) { throw 'Python 3.12 required.' }
    & $Python -m venv (Join-Path $root '.venv-tilelang')
    if ($LASTEXITCODE -ne 0) { throw 'Environment creation failed.' }
}
if (!$TorchWheel) {
    $cached = Join-Path $root '.runtime\qwen-wheels\torch-2.9.1+cu128-cp312-cp312-win_amd64.whl'
    if (Test-Path -LiteralPath $cached) { $TorchWheel = $cached }
}
if ($TorchWheel) {
    & $exe -m pip install $TorchWheel -i $PyIndex --disable-pip-version-check
} else {
    & $exe -m pip install torch==2.9.1 --index-url https://download.pytorch.org/whl/cu128 --disable-pip-version-check
}
if ($LASTEXITCODE -ne 0) { throw 'PyTorch installation failed.' }
& $exe -m pip install -r (Join-Path $root 'requirements-tilelang.txt') -i $PyIndex --disable-pip-version-check
if ($LASTEXITCODE -ne 0) { throw 'TileLang installation failed.' }
& $exe -m pip check
if ($LASTEXITCODE -ne 0) { throw 'Dependency check failed.' }
& $exe -m pip list --format=freeze | Set-Content (Join-Path $root '.runtime\tilelang-lock.txt') -Encoding utf8
if ($LASTEXITCODE -ne 0) { throw 'Could not save dependency versions.' }
& (Join-Path $root 'tilelang-lab.ps1') --check-only
if ($LASTEXITCODE -ne 0) { throw 'GPU kernel verification failed.' }
