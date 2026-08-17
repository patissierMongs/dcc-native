# Walk the desktop window family with FindWindowEx (same calls the Rust code
# makes) and report where our window ended up in the z-order.
$src = @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int max);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public static string ClassOf(IntPtr h) { var sb = new StringBuilder(256); GetClassNameW(h, sb, 256); return sb.ToString(); }
  public static string Describe(IntPtr h) {
    RECT r; GetWindowRect(h, out r);
    uint pid; GetWindowThreadProcessId(h, out pid);
    string pname = "?";
    try { pname = System.Diagnostics.Process.GetProcessById((int)pid).ProcessName; } catch {}
    return string.Format("{0,-10} class={1,-22} vis={2,-5} {3},{4} {5}x{6}  pid={7} ({8})",
      h, ClassOf(h), IsWindowVisible(h), r.L, r.T, r.R-r.L, r.B-r.T, pid, pname);
  }
}
"@
Add-Type -TypeDefinition $src

function Show-Children([IntPtr]$parent, [string]$label) {
    Write-Output "  children of ${label}:"
    $GW_CHILD = 5; $GW_HWNDNEXT = 2
    $c = [W]::GetWindow($parent, $GW_CHILD)
    $i = 0
    while ($c -ne [IntPtr]::Zero -and $i -lt 20) {
        Write-Output ("    [z{0}] {1}" -f $i, [W]::Describe($c))
        $c = [W]::GetWindow($c, $GW_HWNDNEXT)
        $i++
    }
    if ($i -eq 0) { Write-Output "    (none)" }
}

$progman = [W]::FindWindowW("Progman", $null)
Write-Output ("Progman : " + [W]::Describe($progman))
Show-Children $progman "Progman"

Write-Output ''
$prev = [IntPtr]::Zero
$n = 0
while ($true) {
    $w = [W]::FindWindowExW([IntPtr]::Zero, $prev, "WorkerW", $null)
    if ($w -eq [IntPtr]::Zero) { break }
    Write-Output ("WorkerW #{0} : {1}" -f $n, [W]::Describe($w))
    Show-Children $w ("WorkerW #" + $n)
    $prev = $w; $n++
    if ($n -gt 8) { break }
}
if ($n -eq 0) { Write-Output "no top-level WorkerW at all" }
