param([ValidateSet('debug','release')][string]$Profile = 'release')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$clientExe = Join-Path $projectRoot "desktop\target\$Profile\local-voice-desktop.exe"
$reportRoot = Join-Path $projectRoot 'outputs\desktop'
$null = New-Item -ItemType Directory -Force -Path $reportRoot
function Invoke-ClientCheck([string[]]$CheckArguments, [string]$Name, [int]$Timeout = 30000) {
    $errorPath = Join-Path $reportRoot "$Name.stderr.txt"
    $check = Start-Process -FilePath $clientExe -WindowStyle Hidden -ArgumentList $CheckArguments -RedirectStandardError $errorPath -PassThru
    if (!$check.WaitForExit($Timeout)) { Stop-Process -Id $check.Id; throw "$Name timed out" }
    if ($check.ExitCode -ne 0) { Get-Content $errorPath; throw "$Name failed: $($check.ExitCode)" }
}
# Visible fixture is the only input target used by the native check.
$fixture = Start-Process -FilePath $clientExe -ArgumentList '--test-fixture' -PassThru
try {
    $deadline = (Get-Date).AddSeconds(5)
    do { Start-Sleep -Milliseconds 100; $fixture.Refresh() } while (!$fixture.MainWindowHandle -and !$fixture.HasExited -and (Get-Date) -lt $deadline)
    if (!$fixture.MainWindowHandle) { throw 'Test fixture did not open.' }
    Invoke-ClientCheck @('--native-check', "$($fixture.Id)", (Join-Path $reportRoot 'native-check.json')) 'native'
} finally {
    $fixture.Refresh()
    if (!$fixture.HasExited) { $null = $fixture.CloseMainWindow(); if (!$fixture.WaitForExit(3000)) { Stop-Process -Id $fixture.Id } }
}
Invoke-ClientCheck @('--service-check', (Join-Path $reportRoot 'service-check.json')) 'service' 45000
Invoke-ClientCheck @('--snapshot', (Join-Path $reportRoot 'window.png')) 'snapshot' 15000
$restoreRoot = Join-Path $reportRoot 'restore-regression'
Invoke-ClientCheck @('--restore-check', $restoreRoot) 'restore' 15000
$restore = Get-Content -LiteralPath (Join-Path $restoreRoot 'result.json') -Raw | ConvertFrom-Json
if ($restore.cycles.Count -ne 3 -or @($restore.cycles | Where-Object { $_.different_bytes -ne 0 -or !$_.same_size }).Count) {
    throw 'Hidden/restored window surface did not match the initial frame.'
}
Get-Content (Join-Path $reportRoot 'native-check.json')
Get-Content (Join-Path $reportRoot 'service-check.json')
