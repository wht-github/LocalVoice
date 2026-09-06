param([Parameter(Mandatory=$true)][string[]]$Files)
$ErrorActionPreference = 'Stop'
$env:WSL_UTF8 = '1'
foreach ($file in $Files) {
    if ($file -notmatch '^[a-zA-Z0-9][a-zA-Z0-9_.-]*$') { throw 'Only output filenames are accepted.' }
}
$project = Split-Path $PSScriptRoot -Parent
$archive = Join-Path $project '.runtime/output-bundle.tar'
$info = [System.Diagnostics.ProcessStartInfo]::new()
$info.FileName = 'wsl.exe'
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$info.RedirectStandardOutput = $true
foreach ($arg in (@('-d','LocalVoice','-u','voice','--cd','/','--','tar','-cf','-','-C','/opt/local-voice-app/outputs') + $Files)) {
    $info.ArgumentList.Add($arg)
}
$child = [System.Diagnostics.Process]::Start($info)
$stream = [System.IO.File]::Create($archive)
try { $child.StandardOutput.BaseStream.CopyTo($stream) }
finally { $stream.Dispose() }
$child.WaitForExit()
if ($child.ExitCode -ne 0) { throw 'Could not export voice outputs.' }
& tar -xf $archive -C (Join-Path $project 'outputs')
if ($LASTEXITCODE -ne 0) { throw 'Could not unpack voice outputs.' }
