param([int]$TargetPid, [long]$TargetWindow, [string]$TargetClass, [string]$ReportName, [string]$SelectedText)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$clientExe = Join-Path $projectRoot 'desktop\target\debug\local-voice-desktop.exe'
$report = Join-Path $projectRoot "outputs\desktop\$ReportName.json"
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class VoiceClickProbe {
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(IntPtr cls,string name);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect rect);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point point);
 [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
 [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h,uint flags);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint x,uint y,uint data,UIntPtr extra);
}
'@
$root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$TargetWindow)
if ($root.Current.ProcessId -ne $TargetPid) { throw 'Target window changed.' }
$condition = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ClassNameProperty,$TargetClass)
$editor = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants,$condition)
if (!$editor) {
    $editCondition = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Edit)
    $edits = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,$editCondition)
    $matching = @($edits | Where-Object { $_.Current.ClassName.Split(' ') -contains $TargetClass })
    if ($matching.Count -eq 1) { $editor = $matching[0] }
}
if (!$editor) { throw 'Specified input control not found.' }
$null = [VoiceClickProbe]::SetForegroundWindow([IntPtr]$TargetWindow)
$editor.SetFocus()
if ($SelectedText) {
    $pattern = $editor.GetCurrentPattern([System.Windows.Automation.TextPattern]::Pattern)
    $range = $pattern.DocumentRange.FindText($SelectedText,$false,$false)
    if (!$range) { throw 'Known selection fixture not found.' }
    $range.Select()
}
# Diagnostic mode exits on Record and never starts a microphone or sends text.
$arguments = if ($SelectedText) { @('--selection-click-check',"$TargetPid",('"'+$SelectedText+'"'),$report) } else { @('--target-click-check',"$TargetPid",$report) }
$probe = Start-Process -FilePath $clientExe -ArgumentList $arguments -WindowStyle Hidden -PassThru
try {
    Start-Sleep -Seconds 2
    $window = [VoiceClickProbe]::FindWindow([IntPtr]::Zero,'Local Voice')
    $owner = [uint32]0
    $null = [VoiceClickProbe]::GetWindowThreadProcessId($window,[ref]$owner)
    if ($owner -ne $probe.Id) { throw 'Diagnostic window not found; no click performed.' }
    if ([VoiceClickProbe]::GetForegroundWindow() -ne [IntPtr]$TargetWindow) { throw 'Target lost focus; no click performed.' }
    $rect = New-Object VoiceClickProbe+Rect
    $null = [VoiceClickProbe]::GetClientRect($window,[ref]$rect)
    $point = New-Object VoiceClickProbe+Point
    $buttonCenter = if ($SelectedText) { 224 } else { 150 }
    $point.X = [int]($rect.Right * $buttonCenter / 380)
    $point.Y = [int]($rect.Right * 28 / 380)
    $null = [VoiceClickProbe]::ClientToScreen($window,[ref]$point)
    if ([VoiceClickProbe]::GetAncestor([VoiceClickProbe]::WindowFromPoint($point),2) -ne $window) { throw 'Our button is covered; no click performed.' }
    $old = New-Object VoiceClickProbe+Point
    $null = [VoiceClickProbe]::GetCursorPos([ref]$old)
    try {
        $null = [VoiceClickProbe]::SetCursorPos($point.X,$point.Y)
        [VoiceClickProbe]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
        [VoiceClickProbe]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
    } finally { $null = [VoiceClickProbe]::SetCursorPos($old.X,$old.Y) }
    if (!$probe.WaitForExit(10000)) { throw 'Button check timed out.' }
    Get-Content -LiteralPath $report
} finally {
    $probe.Refresh()
    if (!$probe.HasExited) { Stop-Process -Id $probe.Id }
}
