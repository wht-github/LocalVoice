param([switch]$LocalBuild)

$ErrorActionPreference = 'Stop'
$clientExe = Join-Path $PSScriptRoot $(if ($LocalBuild) { 'desktop/target/release/local-voice-desktop.exe' } else { '.runtime/desktop/local-voice-desktop.exe' })
if (!(Test-Path -LiteralPath $clientExe)) {
    if ($LocalBuild) { throw '尚未构建本地开发版，请先执行 cargo build --release --manifest-path desktop/Cargo.toml。' }
    throw '尚未安装 Release，请先执行 ./install.ps1。需要本地构建时使用 ./install.ps1 -LocalBuild。详见 install.md。'
}
$running = Get-Process -Name 'local-voice-desktop*' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $clientExe }
if ($running) { Write-Host '客户端已在运行，可从系统托盘显示悬浮窗。'; exit 0 }
$otherExe = Join-Path $PSScriptRoot $(if ($LocalBuild) { '.runtime/desktop/local-voice-desktop.exe' } else { 'desktop/target/release/local-voice-desktop.exe' })
$otherRunning = Get-Process -Name 'local-voice-desktop*' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $otherExe }
if ($otherRunning) { throw '此项目的另一个版本正在运行。请先从系统托盘退出 Local Voice，再启动所选版本。' }
if ($LocalBuild) { Write-Host '正在启动本地开发版（desktop/target/release）；已下载的 Release 客户端不受影响。' }
# User-facing interactive application; child model services stay hidden.
Start-Process -FilePath $clientExe -WorkingDirectory $PSScriptRoot
