param(
    [ValidateSet('start','stop','status','logs','test','deploy')]
    [string]$Action = 'status',
    [ValidateSet('all','asr','tts')]
    [string]$Service = 'all'
)
$ErrorActionPreference = 'Stop'
$env:WSL_UTF8 = '1'
$units = if ($Service -eq 'all') { @('local-voice-asr','local-voice-tts') } else { @("local-voice-$Service") }
switch ($Action) {
    'start' {
        # A foreground WSL process keeps the dedicated instance alive after this shell exits.
        # flock ensures repeated starts never accumulate keepalive processes.
        Start-Process -FilePath 'wsl.exe' -WindowStyle Hidden -ArgumentList @(
            '-d','LocalVoice','-u','voice','--cd','/','--','flock','-n',
            '/home/voice/.local/share/local-voice-app/keepalive.lock','sleep','infinity'
        )
        & wsl -d LocalVoice -u root --cd / -- systemctl start @units
    }
    'stop' {
        & wsl -d LocalVoice -u root --cd / -- systemctl stop @units
        if ($LASTEXITCODE -eq 0 -and $Service -eq 'all') { & wsl --terminate LocalVoice }
    }
    'status' { & wsl -d LocalVoice --cd / -- systemctl status @units --no-pager -l }
    'logs' {
        $logArgs = @('-d','LocalVoice','-u','root','--cd','/','--','journalctl','--no-pager','-n','80')
        foreach ($unit in $units) { $logArgs += @('-u', $unit) }
        & wsl @logArgs
    }
    'test' {
        & wsl -d LocalVoice -u voice --cd / --exec bash /opt/local-voice-app/scripts/smoke-test.sh
    }
    'deploy' {
        # Explicit host-to-guest transfer; no Windows drive mount is needed.
        $null = New-Item -ItemType Directory -Force -Path (Join-Path $PSScriptRoot '.runtime')
        $bundle = Join-Path $PSScriptRoot '.runtime/service-bundle.tar'
        & tar -cf $bundle -C $PSScriptRoot asr_server.py tts_server.py tester.html smoke_test.py scripts requirements-asr.txt requirements-asr-gpu.txt requirements-tts.txt requirements-melo.txt uv.toml
        if ($LASTEXITCODE -ne 0) { throw 'Could not package service files.' }
        & wsl -d LocalVoice -u root --cd / --exec mkdir -p /opt/local-voice-app
        if ($LASTEXITCODE -ne 0) { throw 'Could not prepare LocalVoice deployment directory.' }
        $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = 'wsl.exe'
        $startInfo.Arguments = '-d LocalVoice -u root --cd / --exec tar -xf - -C /opt/local-voice-app'
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardInput = $true
        $process = [System.Diagnostics.Process]::Start($startInfo)
        $stream = [System.IO.File]::OpenRead($bundle)
        try { $stream.CopyTo($process.StandardInput.BaseStream) }
        finally { $stream.Dispose(); $process.StandardInput.Close() }
        $process.WaitForExit()
        $copyExit = $process.ExitCode
        $process.Dispose()
        if ($copyExit -ne 0) { throw 'Could not transfer service files into LocalVoice.' }
        & wsl -d LocalVoice -u root --cd / --exec bash /opt/local-voice-app/scripts/install-services.sh
        if ($LASTEXITCODE -ne 0) { throw 'Could not install service definitions.' }
        & wsl -d LocalVoice -u root --cd / --exec bash /opt/local-voice-app/scripts/isolate-localvoice.sh
        if ($LASTEXITCODE -ne 0) { throw 'Could not configure LocalVoice isolation.' }
        Write-Host 'Deployed to LocalVoice. Run .\voice.ps1 stop then .\voice.ps1 start to reload.'
    }
}
exit $LASTEXITCODE
