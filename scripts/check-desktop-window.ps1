$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$clientExe = Join-Path $projectRoot 'desktop\target\release\local-voice-desktop.exe'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class VoiceWindowProbe {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct MonitorInfo { public int Size; public Rect Monitor,Work; public uint Flags; }
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls,string title);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr hwnd,int index);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd,out Rect rect);
  [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr hwnd,uint flags);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool GetMonitorInfo(IntPtr monitor,ref MonitorInfo info);
  [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeout(IntPtr hwnd,uint msg,IntPtr wp,IntPtr lp,uint flags,uint timeout,out UIntPtr result);
}
'@
if (Get-Process -Name 'local-voice-desktop' -ErrorAction SilentlyContinue) { throw 'Exit the existing client before measuring a new launch.' }
# This launches the user-facing application and deliberately leaves it running.
$before = [VoiceWindowProbe]::GetForegroundWindow()
$client = Start-Process -FilePath $clientExe -WorkingDirectory (Split-Path $clientExe) -PassThru
$deadline = (Get-Date).AddSeconds(10)
do {
    Start-Sleep -Milliseconds 100
    $client.Refresh()
    $window = $client.MainWindowHandle
    $owner = [uint32]0
    if ($window -ne [IntPtr]::Zero) { $null = [VoiceWindowProbe]::GetWindowThreadProcessId($window,[ref]$owner) }
} while ($owner -ne $client.Id -and (Get-Date) -lt $deadline)
if ($owner -ne $client.Id) { throw 'Client window did not open.' }
Start-Sleep -Seconds 2
$after = [VoiceWindowProbe]::GetForegroundWindow()
$style = [VoiceWindowProbe]::GetWindowLong($window,-20)
$rect = New-Object VoiceWindowProbe+Rect
$null = [VoiceWindowProbe]::GetWindowRect($window,[ref]$rect)
$monitor = New-Object VoiceWindowProbe+MonitorInfo
$monitor.Size = [Runtime.InteropServices.Marshal]::SizeOf($monitor)
$null = [VoiceWindowProbe]::GetMonitorInfo([VoiceWindowProbe]::MonitorFromWindow($window,2),[ref]$monitor)
$mouseResult = [UIntPtr]::Zero
$reply = [VoiceWindowProbe]::SendMessageTimeout($window,0x21,[IntPtr]::Zero,[IntPtr]::Zero,2,1000,[ref]$mouseResult)
$client.Refresh()
$cpuBefore = $client.TotalProcessorTime.TotalSeconds
$timer = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Seconds 10
$client.Refresh()
$elapsed = $timer.Elapsed.TotalSeconds
$cpuDelta = $client.TotalProcessorTime.TotalSeconds - $cpuBefore
$report = [ordered]@{
    pid = $client.Id
    exe_bytes = (Get-Item -LiteralPath $clientExe).Length
    working_set_mib = [Math]::Round($client.WorkingSet64 / 1MB,2)
    private_commit_mib = [Math]::Round($client.PrivateMemorySize64 / 1MB,2)
    idle_sample_seconds = [Math]::Round($elapsed,2)
    idle_cpu_one_core_percent = [Math]::Round(100*$cpuDelta/$elapsed,3)
    idle_cpu_machine_percent = [Math]::Round(100*$cpuDelta/$elapsed/[Environment]::ProcessorCount,3)
    foreground_unchanged_during_launch = ($before -eq $after)
    no_activate_style = (($style -band 0x08000000) -ne 0)
    tool_window_style = (($style -band 0x80) -ne 0)
    topmost_style = (($style -band 8) -ne 0)
    mouse_activation_rejected = ($reply -ne [IntPtr]::Zero -and $mouseResult.ToUInt64() -eq 3)
    within_monitor = ($rect.Left -ge $monitor.Monitor.Left -and $rect.Top -ge $monitor.Monitor.Top -and $rect.Right -le $monitor.Monitor.Right -and $rect.Bottom -le $monitor.Monitor.Bottom)
    window = $rect
    monitor = $monitor.Monitor
}
$report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $projectRoot 'outputs\desktop\window-check.json') -Encoding utf8
$report | ConvertTo-Json -Depth 4
if (!$report.no_activate_style -or !$report.mouse_activation_rejected -or !$report.within_monitor) { throw 'Window behavior check failed.' }
