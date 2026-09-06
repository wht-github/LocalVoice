param([ValidateSet('start','build')][string]$Action = 'start')
$ErrorActionPreference = 'Stop'
$clientRoot = Join-Path $PSScriptRoot 'desktop'
$clientExe = Join-Path $clientRoot 'target\release\local-voice-desktop.exe'
if ($Action -eq 'build') {
    Push-Location $clientRoot
    try { & cargo build --release --locked; if ($LASTEXITCODE -ne 0) { throw '客户端编译失败。' } }
    finally { Pop-Location }
    exit 0
}
if (!(Test-Path -LiteralPath $clientExe)) { throw '尚未编译客户端，请先执行 .\desktop.ps1 build。' }
$running = Get-Process -Name 'local-voice-desktop' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $clientExe }
if ($running) { Write-Host '客户端已在运行，可从系统托盘显示悬浮窗。'; exit 0 }
# The Rust application starts services according to its saved settings.
# This is the user-facing interactive application, not a background helper.
Start-Process -FilePath $clientExe -WorkingDirectory $clientRoot
