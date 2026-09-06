param([int]$AppPid, [string]$Report = 'settings-click.json', [switch]$RequireForeground)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class SettingsClick {
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(IntPtr cls,string title);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
 [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h,uint flags);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint x,uint y,uint data,UIntPtr extra);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint msg,IntPtr w,IntPtr l);
}
'@
$app = Get-Process -Id $AppPid
if ($app.ProcessName -ne 'local-voice-desktop') { throw 'Only the Local Voice app can be tested.' }
$results = @()
for ($cycle=1; $cycle -le 3; $cycle++) {
    $window=[SettingsClick]::FindWindow([IntPtr]::Zero,'Local Voice')
    $owner=[uint32]0
    $null=[SettingsClick]::GetWindowThreadProcessId($window,[ref]$owner)
    if($owner -ne $AppPid){throw 'Floating window belongs to another process.'}
    $rect=New-Object SettingsClick+Rect
    $null=[SettingsClick]::GetClientRect($window,[ref]$rect)
    $point=New-Object SettingsClick+Point
    $point.X=[int]($rect.Right*282/380); $point.Y=[int]($rect.Right*28/380)
    $null=[SettingsClick]::ClientToScreen($window,[ref]$point)
    if([SettingsClick]::GetAncestor([SettingsClick]::WindowFromPoint($point),2) -ne $window){throw 'Settings button is covered; no click sent.'}
    $old=New-Object SettingsClick+Point
    $null=[SettingsClick]::GetCursorPos([ref]$old)
    try {
        $null=[SettingsClick]::SetCursorPos($point.X,$point.Y)
        Start-Sleep -Milliseconds 100
        [SettingsClick]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
        Start-Sleep -Milliseconds 70
        [SettingsClick]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
        Start-Sleep -Milliseconds 100
    } finally {$null=[SettingsClick]::SetCursorPos($old.X,$old.Y)}
    Start-Sleep -Milliseconds 900
    $settings=[SettingsClick]::FindWindow([IntPtr]::Zero,'Local Voice settings')
    $owner=[uint32]0
    $null=[SettingsClick]::GetWindowThreadProcessId($settings,[ref]$owner)
    $visible=$owner -eq $AppPid -and [SettingsClick]::IsWindowVisible($settings)
    $foreground=$visible -and [SettingsClick]::GetForegroundWindow() -eq $settings
    $results+=@{cycle=$cycle;visible=$visible;foreground=$foreground}
    if($owner -eq $AppPid){$null=[SettingsClick]::PostMessage($settings,0x10,[IntPtr]::Zero,[IntPtr]::Zero)}
    Start-Sleep -Milliseconds 300
}
$reportPath=Join-Path (Split-Path $PSScriptRoot -Parent) "outputs/desktop/$Report"
$results | ConvertTo-Json | Set-Content -LiteralPath $reportPath
Get-Content -LiteralPath $reportPath
if($RequireForeground -and @($results | Where-Object { !$_.visible -or !$_.foreground }).Count){throw 'Settings did not become visible and active on every click.'}
