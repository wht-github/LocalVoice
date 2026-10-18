# Package an already-built native Windows release and its dependency notices.
#Requires -Version 7.0
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $status = & git status --porcelain
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect the source checkout.' }
    if ($status) { throw 'Commit source changes before packaging a release.' }
    $manifest = Get-Content -Raw -LiteralPath 'desktop/Cargo.toml'
    $version = [regex]::Match($manifest, '(?m)^version = "([^"]+)"').Groups[1].Value
    if (!$version) { throw 'Cargo package version is missing.' }
    $metadataJson = & cargo metadata --manifest-path desktop/Cargo.toml --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Cargo dependency metadata failed.' }
    $metadata = ($metadataJson -join "`n") | ConvertFrom-Json
    $commit = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read the build commit.' }
    $rustc = & rustc --version
    if ($LASTEXITCODE -ne 0) { throw 'Cannot identify the Rust compiler.' }
    $release = Join-Path $metadata.target_directory 'release'
    $required = @('local-voice-desktop.exe', 'sherpa-onnx-c-api.dll', 'sherpa-onnx-cxx-api.dll', 'onnxruntime.dll', 'onnxruntime_providers_shared.dll')
    foreach ($name in $required) {
        if (!(Test-Path -LiteralPath (Join-Path $release $name) -PathType Leaf)) {
            throw "Release is incomplete: $name. Run cargo build --release first."
        }
    }
    $output = Join-Path $root "outputs/release/v$version"
    $null = New-Item -ItemType Directory -Force $output
    $archivePath = Join-Path $output "LocalVoice-v$version-windows-x64.zip"
    $partial = "$archivePath.partial"
    $archiveStream = [IO.File]::Open($partial, [IO.FileMode]::Create, [IO.FileAccess]::ReadWrite)
    $archive = [IO.Compression.ZipArchive]::new($archiveStream, [IO.Compression.ZipArchiveMode]::Create)
    function Add-Text([string]$Name, [string]$Text) {
        $entry = $archive.CreateEntry($Name, [IO.Compression.CompressionLevel]::Optimal)
        $writer = [IO.StreamWriter]::new($entry.Open(), [Text.UTF8Encoding]::new($false))
        try { $writer.Write($Text) } finally { $writer.Dispose() }
    }
    function Add-File([string]$Path, [string]$Name) {
        # Avoid ZIP's 1980 minimum timestamp for old Cargo license files.
        $entry = $archive.CreateEntry($Name, [IO.Compression.CompressionLevel]::Optimal)
        $source = [IO.File]::OpenRead($Path)
        $destination = $entry.Open()
        try { $source.CopyTo($destination) } finally { $source.Dispose(); $destination.Dispose() }
    }
    try {
        Add-File (Join-Path $release 'local-voice-desktop.exe') 'local-voice-desktop.exe'
        $dlls = @(Get-ChildItem -LiteralPath $release -Filter '*.dll' -File | Sort-Object Name)
        foreach ($dll in $dlls) { Add-File $dll.FullName $dll.Name }
        Add-Text 'build-info.json' ((@{
            version = $version; commit = "$commit"; target = 'x86_64-pc-windows-msvc';
            rustc = "$rustc"; runtime = 'native'; dlls = @($dlls.Name)
        } | ConvertTo-Json -Depth 4) + "`n")
        Add-File (Join-Path $root 'install.md') 'install.md'
        foreach ($name in @('localvoice.portable', 'Start LocalVoice.cmd', 'Setup Models.cmd')) {
            Add-File (Join-Path $root "distribution/$name") $name
        }
        foreach ($name in @('setup-portable.ps1', 'setup-speech.ps1', 'setup-llama.ps1', 'download-qwen-gguf.ps1')) {
            Add-File (Join-Path $root "scripts/$name") "scripts/$name"
        }
        $notices = [Collections.Generic.List[string]]::new()
        $notices.Add("# Third-party components`n")
        $notices.Add('Slint is used under the Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0.')
        $notices.Add('The download page displays the Made with Slint attribution badge.')
        $notices.Add("License files cover the Cargo dependencies and bundled native speech libraries.`n")
        foreach ($package in ($metadata.packages | Sort-Object name, version)) {
            if (!$package.source) { continue }
            $name = "$($package.name)-$($package.version)"
            $directory = Split-Path -Parent $package.manifest_path
            $entries = @(Get-ChildItem -LiteralPath $directory | Where-Object { $_.Name -match '^(?i:license|licence|copying|notice|copyright)' })
            if (!$entries.Count) {
                $directory = Join-Path $root "desktop/third-party/$name"
                $entries = @(Get-ChildItem -LiteralPath $directory)
                if (!$entries.Count) { throw "Missing license notices: $name" }
            }
            foreach ($entry in $entries) {
                $files = if ($entry.PSIsContainer) { @(Get-ChildItem -LiteralPath $entry.FullName -File -Recurse) } else { @($entry) }
                foreach ($file in $files) {
                    $relative = [IO.Path]::GetRelativePath($directory, $file.FullName).Replace('\', '/')
                    Add-File $file.FullName "licenses/$name/$relative"
                }
            }
            $sourceUrl = if ($package.repository) { $package.repository } elseif ($package.homepage) { $package.homepage } else { 'see crate metadata' }
            $notices.Add("- ${name}: $($package.license); $sourceUrl")
        }
        $nativeNotices = Join-Path $root 'desktop/third-party/native-speech'
        foreach ($name in @('sherpa-onnx-LICENSE', 'onnxruntime-LICENSE', 'onnxruntime-ThirdPartyNotices.txt', 'SOURCE.txt')) {
            Add-File (Join-Path $nativeNotices $name) "licenses/native-speech/$name"
        }
        $notices.Add('- Native speech runtime: sherpa-onnx (Apache-2.0), ONNX Runtime (MIT). See licenses/native-speech for their notices.')
        Add-Text 'THIRD-PARTY-NOTICES.md' (($notices -join "`n") + "`n")
    } finally { $archive.Dispose() }
    Move-Item -LiteralPath $partial -Destination $archivePath -Force
    $digest = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$digest  $([IO.Path]::GetFileName($archivePath))" | Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS') -Encoding ascii
    Write-Host $archivePath
    Write-Host $digest
} finally { Pop-Location }
