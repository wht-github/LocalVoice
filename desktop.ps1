$ErrorActionPreference = 'Stop'
$clientExe = Join-Path $PSScriptRoot '.runtime/desktop/local-voice-desktop.exe'
if (!(Test-Path -LiteralPath $clientExe)) { throw '尚未安装 Release 客户端，请先执行 ./install.ps1。详见 install.md。' }
$running = Get-Process -Name 'local-voice-desktop' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $clientExe }
if ($running) { Write-Host '客户端已在运行，可从系统托盘显示悬浮窗。'; exit 0 }
# User-facing interactive application; child model services stay hidden.
Start-Process -FilePath $clientExe -WorkingDirectory $PSScriptRoot
