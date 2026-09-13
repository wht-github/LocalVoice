# Official Windows CUDA binaries; no CUDA toolkit or local C++ build required.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$version = 'b10809' # Official v0.4.0 release points to this binary release.
$destination = Join-Path $root '.runtime/llama'
$downloads = Join-Path $root '.runtime/downloads'
$null = New-Item -ItemType Directory -Force $destination, $downloads
$packages = @(
    @{ Name = 'llama-b10809-bin-win-cuda-13.3-x64.zip'; SHA256 = '790a89e7c40049b8b1cd1a2565abe1d40cfc4e18843cdefa77ed2ce52d6a7b2d' },
    @{ Name = 'cudart-llama-bin-win-cuda-13.3-x64.zip'; SHA256 = '1462a050eb4c684921ba51dcc4cc488a036674c3e73e9945ee705b854808d03e' }
)
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
    Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force
}
@{ version = $version; release = 'https://github.com/ggml-org/llama.cpp/releases/tag/v0.4.0'; packages = $packages } |
    ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 (Join-Path $destination 'release.json')
& (Join-Path $destination 'llama-server.exe') --version
if ($LASTEXITCODE -ne 0) { throw 'llama.cpp 运行检查失败，请检查 NVIDIA 驱动和 VC++ 运行库。' }
Write-Host "官方 llama.cpp 已就绪：$destination"
