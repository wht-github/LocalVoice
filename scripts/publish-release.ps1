$ErrorActionPreference = 'Stop'
$releaseDir = $PSScriptRoot
$repo = 'D:\Workspace\local_voice_app'
$baseUri = 'https://api.github.com/repos/wht-github/LocalVoice'
$tag = 'v0.4.2'
$commit = (& git -C $repo rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot read source commit' }
$previousPrompt = $env:GIT_TERMINAL_PROMPT
$previousInteractive = $env:GCM_INTERACTIVE
try {
    $env:GIT_TERMINAL_PROMPT = '0'
    $env:GCM_INTERACTIVE = 'Never'
    $credential = "protocol=https`nhost=github.com`n`n" | git credential fill 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'GitHub authentication unavailable' }
    $fields = @{}
    foreach ($line in $credential) {
        if ($line -match '^([^=]+)=(.*)$') { $fields[$matches[1]] = $matches[2] }
    }
    if (!$fields.password) { throw 'GitHub credential unavailable' }
    $headers = @{ Authorization = "Bearer $($fields.password)"; Accept = 'application/vnd.github+json'; 'X-GitHub-Api-Version' = '2022-11-28' }
    $release = $null
    try { $release = Invoke-RestMethod -Uri "$baseUri/releases/tags/$tag" -Headers $headers }
    catch { if ([int]$_.Exception.Response.StatusCode -ne 404) { throw } }
    $metadataPath = Join-Path $releaseDir 'github-release.json'
    if (!$release -and (Test-Path -LiteralPath $metadataPath)) {
        $saved = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
        $release = Invoke-RestMethod -Uri "$baseUri/releases/$($saved.id)" -Headers $headers
    }
    if ($release -and (!$release.draft -or $release.tag_name -ne $tag)) { throw 'Existing published or mismatched release; inspect before changing' }
    if (!$release) {
        $request = @{
            tag_name = $tag
            target_commitish = $commit
            name = 'LocalVoice v0.4.2 · 连续日常输入与录音预加载'
            body = Get-Content -LiteralPath (Join-Path $releaseDir 'release-notes.md') -Raw -Encoding UTF8
            draft = $true
            prerelease = $false
        }
        $release = Invoke-RestMethod -Uri "$baseUri/releases" -Headers $headers -Method Post -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes(($request | ConvertTo-Json -Depth 4)))
    }
    $release | Select-Object id,html_url,upload_url,tag_name | ConvertTo-Json | Set-Content -LiteralPath $metadataPath -Encoding UTF8
    $uploadUri = $release.upload_url -replace '\{\?name,label\}', ''
    foreach ($name in @('LocalVoice-v0.4.2-windows-x64.zip', 'SHA256SUMS')) {
        $file = Get-Item -LiteralPath (Join-Path $releaseDir $name)
        $digest = 'sha256:' + (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        $asset = @($release.assets | Where-Object name -eq $name)
        if ($asset.Count -gt 1) { throw "Duplicate release asset: $name" }
        if ($asset.Count -eq 1) { $asset = $asset[0] }
        else {
            $contentType = if ($name.EndsWith('.zip')) { 'application/zip' } else { 'text/plain' }
            $asset = Invoke-RestMethod -Uri ($uploadUri + '?name=' + [Uri]::EscapeDataString($name)) -Headers $headers -Method Post -ContentType $contentType -InFile $file.FullName -TimeoutSec 180
        }
        if ($asset.state -ne 'uploaded' -or $asset.size -ne $file.Length -or $asset.digest -ne $digest) { throw "Asset verification failed: $name" }
        Write-Output "Verified uploaded asset: $name ($($file.Length) bytes)"
    }
    $published = Invoke-RestMethod -Uri "$baseUri/releases/$($release.id)" -Headers $headers -Method Patch -ContentType 'application/json' -Body '{"draft":false,"make_latest":"true"}'
    if ($published.draft -or $published.tag_name -ne $tag) { throw 'Release did not publish' }
    $published | Select-Object id,html_url,upload_url,tag_name,published_at | ConvertTo-Json | Set-Content -LiteralPath $metadataPath -Encoding UTF8
    Write-Output $published.html_url
} finally {
    $env:GIT_TERMINAL_PROMPT = $previousPrompt
    $env:GCM_INTERACTIVE = $previousInteractive
}
