param([switch]$RuntimeOnly)
$ErrorActionPreference = 'Stop'
$llamaRoot = Split-Path -Parent $PSScriptRoot
$llamaVenv = Join-Path $llamaRoot '.venv-llama-build'
$llamaCuda = Join-Path $llamaVenv 'Lib/site-packages/nvidia/cu13'
if (!(Test-Path -LiteralPath "$llamaCuda/bin/nvcc.exe")) {
    throw 'Run scripts/setup-llama-native.ps1 first.'
}
if (!$RuntimeOnly) {
    $llamaVswhere = "${env:ProgramFiles(x86)}/Microsoft Visual Studio/Installer/vswhere.exe"
    if (!(Test-Path -LiteralPath $llamaVswhere)) { throw 'Visual Studio Installer was not found.' }
    $llamaVs = @(& $llamaVswhere -all -version '[18.0,19.0)' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json | ConvertFrom-Json) |
        Where-Object { $_.isComplete -and $_.isLaunchable } | Select-Object -First 1
    if (!$llamaVs) {
        throw 'VS2026 is incomplete or missing Desktop development with C++. Finish the VS update first.'
    }
    & (Join-Path $llamaVs.installationPath 'Common7/Tools/Launch-VsDevShell.ps1') -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
    if (!(Get-Command cl.exe -ErrorAction SilentlyContinue)) { throw 'MSVC activation failed.' }
}
$env:CUDA_PATH = $llamaCuda
$env:CUDAToolkit_ROOT = $llamaCuda
$env:CUDACXX = Join-Path $llamaCuda 'bin/nvcc.exe'
$env:PATH = "$llamaVenv/Scripts;$llamaCuda/bin;$llamaCuda/bin/x86_64;$llamaRoot/.runtime/llama-build/bin;$env:PATH"
Write-Host "llama.cpp environment: $llamaRoot"
