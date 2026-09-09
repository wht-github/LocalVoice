# Run the independent Windows/NVIDIA transcription experiment.
param(
    [Parameter(Mandatory=$true, Position=0)][string]$Audio,
    [int]$Repeat = 2,
    [string]$Language,
    [string]$Prompt,
    [string]$Output
)
$ErrorActionPreference = 'Stop'
$python = Join-Path $PSScriptRoot '.venv-qwen\Scripts\python.exe'
if (!(Test-Path -LiteralPath $python)) {
    throw 'Run scripts/setup-qwen-native.ps1 first.'
}
$arguments = @('-X', 'utf8', (Join-Path $PSScriptRoot 'scripts\qwen-native.py'), $Audio, '--repeat', "$Repeat")
if ($Language) { $arguments += @('--language', $Language) }
if ($Prompt) { $arguments += @('--prompt', $Prompt) }
if ($Output) { $arguments += @('--output', $Output) }
& $python @arguments
if ($LASTEXITCODE -ne 0) { throw "Transcription failed (exit $LASTEXITCODE)." }
