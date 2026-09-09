# Native Windows GPU operator lab; arguments are passed to run.py.
$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot '.venv-tilelang\Scripts\python.exe'
if (!(Test-Path -LiteralPath $exe)) { throw 'Run scripts/setup-tilelang.ps1 first.' }
$env:TILELANG_CACHE_DIR = Join-Path $PSScriptRoot '.runtime\tilelang-cache'
$env:TVM_FFI_CACHE_DIR = Join-Path $PSScriptRoot '.runtime\tvm-ffi-cache'
$env:TVM_FFI_DISABLE_TORCH_C_DLPACK = '1'
& $exe (Join-Path $PSScriptRoot 'experiments\tilelang\run.py') @args
exit $LASTEXITCODE
