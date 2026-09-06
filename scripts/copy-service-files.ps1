param([Parameter(Mandatory)][string[]]$Files)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$bundle = Join-Path $projectRoot '.runtime/update-files.tar'
foreach ($file in $Files) {
    $full = [IO.Path]::GetFullPath((Join-Path $projectRoot $file))
    if (!$full.StartsWith($projectRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'File must be inside this workspace' }
}
& tar -cf $bundle -C $projectRoot @Files
if ($LASTEXITCODE -ne 0) { throw 'Packaging failed' }
$start = [Diagnostics.ProcessStartInfo]::new('wsl.exe')
$start.Arguments = '-d LocalVoice -u root --cd / --exec tar -xf - -C /opt/local-voice-app'
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardInput = $true
$process = [Diagnostics.Process]::Start($start)
$stream = [IO.File]::OpenRead($bundle)
try { $stream.CopyTo($process.StandardInput.BaseStream) }
finally { $stream.Dispose(); $process.StandardInput.Close() }
$process.WaitForExit()
if ($process.ExitCode -ne 0) { throw 'Transfer failed' }
$process.Dispose()
