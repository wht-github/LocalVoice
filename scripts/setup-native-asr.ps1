# Uses the user's existing uv and Python; never downloads a Python interpreter.
param([string]$PyIndex = 'https://pypi.org/simple')
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$python = Join-Path $root '.venv/Scripts/python.exe'
$cache = Join-Path $root '.runtime/cache/uv'
if (!(Get-Command uv -ErrorAction SilentlyContinue)) { throw '请先自行安装 uv，并用 uv python install 3.12 准备 Python。详见 install.md。' }
if (!(Test-Path -LiteralPath $python)) {
    & uv --no-config venv --python 3.12 --no-python-downloads (Join-Path $root '.venv')
    if ($LASTEXITCODE -ne 0) { throw '创建 .venv 失败；请先用 uv python install 3.12 准备 Python。' }
}
& $python -c 'import sys; assert (3, 12) <= sys.version_info[:2] <= (3, 13), "Requires Python 3.12 or 3.13"'
if ($LASTEXITCODE -ne 0) { throw '.venv 需要 Python 3.12–3.13。' }
# Keep CPU builds pinned while resolving all dependencies together.
& uv --no-config pip install --python $python --cache-dir $cache --index-url $PyIndex `
    --index https://download.pytorch.org/whl/cpu `
    -r (Join-Path $root 'requirements-native-asr.txt')
if ($LASTEXITCODE -ne 0) { throw '安装识别服务依赖失败。' }
& uv --no-config pip check --python $python
if ($LASTEXITCODE -ne 0) { throw '依赖一致性检查失败。' }
Write-Host "原生识别环境已就绪：$python"
