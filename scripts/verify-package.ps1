$ErrorActionPreference = 'Stop'
$repo = 'D:\Workspace\local_voice_app'
$releaseDir = Join-Path $repo 'outputs\release\v0.4.2'
$archivePath = Join-Path $releaseDir 'LocalVoice-v0.4.2-windows-x64.zip'
$testRoot = (Get-Content -LiteralPath (Join-Path $repo 'outputs\v0.4.1-test-path.txt') -Raw).Trim()
if (!(Test-Path -LiteralPath $testRoot -PathType Container)) { throw 'Previous portable test installation missing' }
$modelPath = Join-Path $testRoot '.runtime\models\sensevoice-int8\model.int8.onnx'
$modelHashBefore = (Get-FileHash -LiteralPath $modelPath -Algorithm SHA256).Hash
$checksum = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
$expected = (Get-Content -LiteralPath (Join-Path $releaseDir 'SHA256SUMS') -Raw).Split(' ')[0]
if ($checksum -cne $expected) { throw 'Archive checksum mismatch' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($archivePath)
try {
    $entries = @($archive.Entries.FullName)
    foreach ($name in @('local-voice-desktop.exe','localvoice.portable','Start LocalVoice.cmd','Setup Models.cmd','build-info.json','install.md','scripts/setup-portable.ps1')) {
        if ($entries -notcontains $name) { throw "Missing release entry: $name" }
    }
    if (@($entries | Where-Object { $_ -match '^\.runtime/|settings\.json$|\.wav$|\.gguf$|\.onnx$|^\.git/' }).Count) { throw 'Release contains private/runtime data' }
    $reader = [IO.StreamReader]::new($archive.GetEntry('build-info.json').Open())
    try { $build = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
    $commit = (& git -C $repo rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $build.version -cne '0.4.2' -or $build.commit -cne $commit) { throw 'Build provenance mismatch' }
} finally { $archive.Dispose() }
Expand-Archive -LiteralPath $archivePath -DestinationPath $testRoot -Force
if ((Get-FileHash -LiteralPath $modelPath -Algorithm SHA256).Hash -cne $modelHashBefore) { throw 'Upgrade changed existing model' }
$exe = Join-Path $testRoot 'local-voice-desktop.exe'
foreach ($check in @('installation','service','vad')) {
    $report = Join-Path $releaseDir "$check-check.json"
    $process = Start-Process -FilePath $exe -ArgumentList @("--$check-check", $report) -WorkingDirectory $env:SystemRoot -WindowStyle Hidden -PassThru -Wait
    if ($process.ExitCode -ne 0 -or !(Test-Path -LiteralPath $report)) { throw "$check check failed: $($process.ExitCode)" }
}
$installation = Get-Content -LiteralPath (Join-Path $releaseDir 'installation-check.json') -Raw | ConvertFrom-Json
if (!$installation.portable -or !$installation.basic_ready -or !$installation.files_present.llama_cpu -or $installation.root -ne $testRoot) { throw 'Portable discovery failed' }
$service = Get-Content -LiteralPath (Join-Path $releaseDir 'service-check.json') -Raw | ConvertFrom-Json
if (!$service.asr_ready -or !$service.asr_text.Trim()) { throw 'Recognition check failed' }
$result = [ordered]@{
    version = $build.version
    commit = $commit
    archive_bytes = (Get-Item -LiteralPath $archivePath).Length
    archive_sha256 = $checksum
    zip_entries = $entries.Count
    test_installation = $testRoot
    launched_from = $env:SystemRoot
    reused_model_unchanged = $true
    installation_passed = $true
    native_recognition_passed = $true
    vad_passed = $true
}
$result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $releaseDir 'package-verification.json') -Encoding UTF8
$result | ConvertTo-Json
