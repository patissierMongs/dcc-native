# Enumerate every window in the session and show the ones owned by dcc.exe,
# plus whether the classic desktop windows exist at all.
$src = @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class V {
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] public static extern IntPtr GetDesktopWindow();
  [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int max);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public static string ClassOf(IntPtr h) { var sb = new StringBuilder(256); GetClassNameW(h, sb, 256); return sb.ToString(); }
  static List<IntPtr> all = new List<IntPtr>();
  static bool Visit(IntPtr h, IntPtr p) { all.Add(h); EnumChildWindows(h, Visit, IntPtr.Zero); return true; }
  public static IntPtr[] Everything() {
    all.Clear();
    EnumWindows(Visit, IntPtr.Zero);
    return all.ToArray();
  }
  public static uint PidOf(IntPtr h) { uint pid; GetWindowThreadProcessId(h, out pid); return pid; }
  public static string Describe(IntPtr h) {
    RECT r; GetWindowRect(h, out r);
    IntPtr par = GetParent(h);
    return string.Format("{0,-10} class={1,-24} vis={2,-5} {3},{4} {5}x{6} parent={7} parentClass={8}",
      h, ClassOf(h), IsWindowVisible(h), r.L, r.T, r.R-r.L, r.B-r.T, par, par == IntPtr.Zero ? "-" : ClassOf(par));
  }
}
"@
Add-Type -TypeDefinition $src

$dcc = Get-Process dcc -ErrorAction SilentlyContinue
if (-not $dcc) { Write-Output 'dcc.exe is NOT running'; exit }
Write-Output ("dcc pid = {0}" -f $dcc.Id)

$wins = [V]::Everything()
Write-Output ("windows enumerated: {0}" -f $wins.Count)

Write-Output '--- windows owned by dcc ---'
$found = 0
foreach ($h in $wins) {
    if ([V]::PidOf($h) -eq $dcc.Id) { Write-Output ("  " + [V]::Describe($h)); $found++ }
}
if ($found -eq 0) { Write-Output '  none - the window was destroyed' }

Write-Output '--- desktop classes present ---'
$classes = @{}
foreach ($h in $wins) {
    $c = [V]::ClassOf($h)
    if ($c -eq 'Progman' -or $c -eq 'WorkerW' -or $c -eq 'SHELLDLL_DefView' -or $c -like '*Wallpaper*') {
        $key = "$c"
        if (-not $classes.ContainsKey($key)) { $classes[$key] = 0 }
        $classes[$key]++
    }
}
if ($classes.Count -eq 0) { Write-Output '  no Progman / WorkerW / SHELLDLL_DefView anywhere' }
foreach ($k in $classes.Keys) { Write-Output ("  {0} x{1}" -f $k, $classes[$k]) }
