# Official Windows binaries; no CUDA toolkit or local C++ build required.
param([switch]$CpuOnly, [switch]$Repair)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$version = 'b10809' # Official v0.4.0 release points to this binary release.
$destination = Join-Path $root $(if ($CpuOnly) { '.runtime/llama-cpu' } else { '.runtime/llama' })
$downloads = Join-Path $root '.runtime/downloads'
$server = Join-Path $destination 'llama-server.exe'
$null = New-Item -ItemType Directory -Force $destination, $downloads
$packages = if ($CpuOnly) { @(
    @{ Name = 'llama-b10809-bin-win-cpu-x64.zip'; SHA256 = '9df3158ed228a641a4b127942d7f459f24c9e13f04682659d05c00c80099b6b5' }
) } else { @(
    @{ Name = 'llama-b10809-bin-win-cuda-13.3-x64.zip'; SHA256 = '790a89e7c40049b8b1cd1a2565abe1d40cfc4e18843cdefa77ed2ce52d6a7b2d' },
    @{ Name = 'cudart-llama-bin-win-cuda-13.3-x64.zip'; SHA256 = '1462a050eb4c684921ba51dcc4cc488a036674c3e73e9945ee705b854808d03e' }
) }
# Reuse an intact pinned runtime, so installing an optional ASR model does not
# overwrite DLLs used by an already running editor. -Repair forces re-extraction.
$releasePath = Join-Path $destination 'release.json'
$installed = $null
if (Test-Path -LiteralPath $releasePath) {
    try { $installed = Get-Content -LiteralPath $releasePath -Raw -Encoding UTF8 | ConvertFrom-Json }
    catch { $installed = $null }
}
$ready = $installed -and $installed.version -eq $version -and [bool]$installed.cpu_only -eq [bool]$CpuOnly
$required = @('llama-server.exe', 'llama-server-impl.dll', 'llama.dll', 'llama-common.dll', 'mtmd.dll', 'ggml.dll', 'ggml-base.dll')
$required += if ($CpuOnly) { 'ggml-cpu-x64.dll' } else { 'ggml-cuda.dll' }
foreach ($name in $required) {
    if (!(Test-Path -LiteralPath (Join-Path $destination $name) -PathType Leaf)) { $ready = $false }
}
foreach ($package in $packages) {
    if (!($installed.packages | Where-Object { $_.Name -eq $package.Name -and $_.SHA256 -eq $package.SHA256 })) { $ready = $false }
}
if ($ready -and !$Repair) {
    Write-Host "已安装官方 llama.cpp $version，复用：$destination"
    return
}
if (Get-Process -Name 'llama-server' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $server }) {
    throw '该 llama.cpp 模型进程仍在运行。请先在客户端停止模型，再更新运行库。'
}
foreach ($package in $packages) {
    $archive = Join-Path $downloads $package.Name
    $url = "https://github.com/ggml-org/llama.cpp/releases/download/$version/$($package.Name)"
    if (!(Test-Path -LiteralPath $archive)) {
        & curl.exe --fail --location --retry 3 --output "$archive.partial" $url
        if ($LASTEXITCODE -ne 0) { throw "下载失败：$url" }
        if ((Get-FileHash -LiteralPath "$archive.partial" -Algorithm SHA256).Hash -ne $package.SHA256) {
            throw "下载校验失败：$($package.Name)"
        }
        Move-Item -LiteralPath "$archive.partial" -Destination $archive
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $package.SHA256) {
        throw "文件校验失败：$archive"
    }
    Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force -ErrorAction Stop
}
@{ version = $version; release = 'https://github.com/ggml-org/llama.cpp/releases/tag/v0.4.0'; cpu_only = [bool]$CpuOnly; packages = $packages } |
    ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 (Join-Path $destination 'release.json')
& $server --version
if ($LASTEXITCODE -ne 0) { throw 'llama.cpp 运行检查失败，请检查 VC++ 运行库；CUDA 版本还需兼容的 NVIDIA 驱动。' }
Write-Host "官方 llama.cpp 已就绪：$destination"
