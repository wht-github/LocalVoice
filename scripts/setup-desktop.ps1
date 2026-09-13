# Install the prebuilt desktop matching this checkout's package version.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content -Raw (Join-Path $root 'desktop/Cargo.toml')
$version = [regex]::Match($manifest, '(?m)^version = "([^"]+)"').Groups[1].Value
$name = "LocalVoice-v$version-windows-x64.zip"
$base = "https://github.com/wht-github/LocalVoice/releases/download/v$version"
$downloads = Join-Path $root '.runtime/downloads'
$destination = Join-Path $root '.runtime/desktop'
$exe = Join-Path $destination 'local-voice-desktop.exe'
if (Get-Process -Name 'local-voice-desktop' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }) {
    throw '请先从托盘退出当前客户端，再更新 Release。'
}
$null = New-Item -ItemType Directory -Force $downloads, $destination
$checksums = Join-Path $downloads "v$version-SHA256SUMS"
& curl.exe --fail --location --retry 3 --output $checksums "$base/SHA256SUMS"
if ($LASTEXITCODE -ne 0) { throw "无法下载 v$version 的校验文件，请检查网络和 GitHub Release。" }
$line = Get-Content $checksums | Where-Object { $_ -match "^[a-fA-F0-9]{64}  $([regex]::Escape($name))$" }
if (@($line).Count -ne 1) { throw "校验文件中缺少唯一条目：$name" }
$expected = $line.Substring(0, 64)
$archive = Join-Path $downloads $name
if (!(Test-Path -LiteralPath $archive)) {
    & curl.exe --fail --location --retry 3 --output "$archive.partial" "$base/$name"
    if ($LASTEXITCODE -ne 0) { throw "下载失败：$name" }
    if ((Get-FileHash -LiteralPath "$archive.partial" -Algorithm SHA256).Hash -ne $expected) { throw 'Release 下载校验失败。' }
    Move-Item -LiteralPath "$archive.partial" -Destination $archive
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expected) { throw "Release 校验失败，请删除后重试：$archive" }
Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force
if (!(Test-Path -LiteralPath $exe)) { throw 'Release 包内缺少客户端。' }
Write-Host "Slint v$version 已就绪：$exe"
