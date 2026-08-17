# Rebuild the desktop window family: stop every explorer.exe, make sure the
# shell comes back, then report the Progman/WorkerW layout.
Stop-Process -Name dcc -Force -ErrorAction SilentlyContinue
Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 4

if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) {
    Start-Process explorer.exe
    Start-Sleep -Seconds 4
}

Write-Output '--- explorer processes ---'
Get-Process explorer | Select-Object Id, StartTime | Format-Table -AutoSize

$src = @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class P {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string c, string t);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr p, IntPtr a, string c, string t);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public static string Rect(IntPtr h) { RECT r; GetWindowRect(h, out r); return string.Format("{0}x{1} at {2},{3} vis={4}", r.R-r.L, r.B-r.T, r.L, r.T, IsWindowVisible(h)); }
}
"@
Add-Type -TypeDefinition $src

$pm = [P]::FindWindowW("Progman", $null)
Write-Output ("Progman {0} : {1}" -f $pm, [P]::Rect($pm))

Write-Output '--- WorkerW children of Progman ---'
$prev = [IntPtr]::Zero
for ($i = 0; $i -lt 8; $i++) {
    $w = [P]::FindWindowExW($pm, $prev, "WorkerW", $null)
    if ($w -eq [IntPtr]::Zero) { break }
    Write-Output ("  {0} : {1}" -f $w, [P]::Rect($w))
    $prev = $w
}

Write-Output '--- top-level WorkerW ---'
$prev = [IntPtr]::Zero
for ($i = 0; $i -lt 8; $i++) {
    $w = [P]::FindWindowExW([IntPtr]::Zero, $prev, "WorkerW", $null)
    if ($w -eq [IntPtr]::Zero) { break }
    Write-Output ("  {0} : {1}" -f $w, [P]::Rect($w))
    $prev = $w
}
