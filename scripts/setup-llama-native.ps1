param(
    [string]$Python,
    [string]$PyIndex = 'https://pypi.tuna.tsinghua.edu.cn/simple'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root '.venv-llama-build/Scripts/python.exe'
$env:PIP_CACHE_DIR = Join-Path $root '.runtime/pip-cache'
if (!$Python) {
    $Python = Join-Path $root '.venv-qwen/Scripts/python.exe'
    if (!(Test-Path -LiteralPath $Python)) { $Python = 'python' }
}
if (!(Test-Path -LiteralPath $exe)) {
    & $Python -m venv (Join-Path $root '.venv-llama-build')
    if ($LASTEXITCODE -ne 0) { throw 'Environment creation failed.' }
}
& $exe -m pip install -r (Join-Path $root 'requirements-llama-build.txt') -i $PyIndex --disable-pip-version-check
if ($LASTEXITCODE -ne 0) { throw 'Build dependencies installation failed.' }
& $exe -m pip check
if ($LASTEXITCODE -ne 0) { throw 'Dependency check failed.' }
& $exe (Join-Path $PSScriptRoot 'prepare-llama-cublas.py')
if ($LASTEXITCODE -ne 0) { throw 'cuBLAS import libraries setup failed.' }
$source = Join-Path $root 'external/llama.cpp'
if (!(Test-Path -LiteralPath $source)) {
    & git clone --depth 1 https://github.com/ggml-org/llama.cpp $source
    if ($LASTEXITCODE -ne 0) { throw 'llama.cpp clone failed.' }
}
& git -C $source rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Source directory is not a Git checkout.' }
Write-Host 'Build dependencies ready. After VS completes, run scripts/build-llama-native.ps1.'
