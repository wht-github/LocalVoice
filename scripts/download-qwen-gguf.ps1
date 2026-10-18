# Pinned Qwen ASR GGUF assets; no Python or Hugging Face client required.
param([ValidateSet('0.6B','1.7B')][string]$Size = '1.7B')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$destination = Join-Path $projectRoot ".runtime/models/Qwen3-ASR-$Size-GGUF"
$repo = "ggml-org/Qwen3-ASR-$Size-GGUF"
$revision = '36a678687ba7d07a74ca70ccb0e36902e005fb80'
$files = @{
    'Qwen3-ASR-1.7B-Q8_0.gguf' = @(2165034944L, '58e22d0532d4eacaf034cfac17a6fed159f37c41390c710186783be439d1fc57')
    'mmproj-Qwen3-ASR-1.7B-bf16.gguf' = @(641773984L, '8882e9ddab3186f9aa71b1417c847177913e1466655ac944cf86e9b846735d62')
}
if ($Size -eq '0.6B') {
    $revision = '928ab958557df9aa2ef1c93e0e83c7ad0933fae2'
    $files = @{
        'Qwen3-ASR-0.6B-Q8_0.gguf' = @(804749248L, 'bca259818b50ca7c4c05e9bdb35a5dc04fa039653a6d6f3f0f331f96f6aa1971')
        'mmproj-Qwen3-ASR-0.6B-bf16.gguf' = @(378575520L, 'dae36c855f9a82a8916bea2238b24bda69a39d8da8b2f46dee7c103775656039')
    }
}
$endpoint = if ($env:HF_ENDPOINT) { $env:HF_ENDPOINT.TrimEnd('/') } else { 'https://huggingface.co' }
New-Item -ItemType Directory -Force $destination | Out-Null
$manifest = @{}
foreach ($name in $files.Keys) {
    $bytes, $sha256 = $files[$name]
    $path = Join-Path $destination $name
    $valid = (Test-Path -LiteralPath $path) -and (Get-Item -LiteralPath $path).Length -eq $bytes
    if ($valid) { $valid = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -eq $sha256 }
    if (!$valid) {
        $partial = "$path.partial"
        & curl.exe --fail --location --retry 3 --connect-timeout 20 --output $partial "$endpoint/$repo/resolve/$revision/${name}?download=true"
        if ($LASTEXITCODE -ne 0) { throw "Download failed: $name" }
        if ((Get-Item -LiteralPath $partial).Length -ne $bytes -or
            (Get-FileHash -LiteralPath $partial -Algorithm SHA256).Hash -ne $sha256) {
            throw "Checksum mismatch: $name"
        }
        Move-Item -LiteralPath $partial -Destination $path -Force
    }
    $manifest[$name] = @{ bytes=$bytes; sha256=$sha256 }
    Write-Host "Verified $name"
}
@{ repo=$repo; revision=$revision; files=$manifest } | ConvertTo-Json -Depth 4 |
    Set-Content -LiteralPath (Join-Path $destination 'manifest.json') -Encoding utf8
