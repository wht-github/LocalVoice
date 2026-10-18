# Native Silero VAD + SenseVoice assets. No Python or pip environment.
param([switch]$VadOnly)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$models = Join-Path $projectRoot '.runtime/models'
$cache = Join-Path $projectRoot '.runtime/downloads'
New-Item -ItemType Directory -Force $cache, (Join-Path $models 'silero-vad') | Out-Null

function Get-VerifiedAsset([string]$Url, [string]$Destination, [long]$Bytes, [string]$Sha256) {
    if ((Test-Path -LiteralPath $Destination) -and
        (Get-Item -LiteralPath $Destination).Length -eq $Bytes -and
        (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash -eq $Sha256) { return }
    $partial = "$Destination.partial"
    & curl.exe --fail --location --retry 3 --connect-timeout 20 --output $partial $Url
    if ($LASTEXITCODE -ne 0) { throw "Download failed: $Url" }
    if ((Get-Item -LiteralPath $partial).Length -ne $Bytes -or
        (Get-FileHash -LiteralPath $partial -Algorithm SHA256).Hash -ne $Sha256) {
        throw "Checksum mismatch: $partial"
    }
    Move-Item -LiteralPath $partial -Destination $Destination -Force
}

Get-VerifiedAsset 'https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx' `
    (Join-Path $models 'silero-vad/silero_vad.onnx') 643854 `
    '9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6'
Write-Host 'Silero VAD verified (629 KiB).'
if ($VadOnly) { return }

$name = 'sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17'
$archive = Join-Path $cache "$name.tar.bz2"
Get-VerifiedAsset "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/$name.tar.bz2" `
    $archive 163002883 '7d1efa2138a65b0b488df37f8b89e3d91a60676e416f515b952358d83dfd347e'
# Extract only known files from the checksum-verified official archive.
$destination = Join-Path $models 'sensevoice-int8'
New-Item -ItemType Directory -Force $destination | Out-Null
& tar.exe -xf $archive -C $destination --strip-components 1 "$name/model.int8.onnx" "$name/tokens.txt"
if ($LASTEXITCODE -ne 0) { throw 'Cannot extract native SenseVoice model.' }
Write-Host 'Native SenseVoice INT8 ready.'
