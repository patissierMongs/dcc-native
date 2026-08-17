# Start the release build detached, then report what it costs.
Set-Location $PSScriptRoot
Stop-Process -Name dcc -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

Start-Process -FilePath '.\target\release\dcc.exe' -RedirectStandardOutput 'run.log' -RedirectStandardError 'run.err' -WindowStyle Hidden
Start-Sleep -Seconds 25

Write-Output '--- stdout ---'
Get-Content run.log -ErrorAction SilentlyContinue
Write-Output '--- stderr ---'
Get-Content run.err -ErrorAction SilentlyContinue

Write-Output '--- cost of dcc ---'
$p = Get-Process dcc -ErrorAction SilentlyContinue
if ($p) {
    $c1 = $p.CPU
    Start-Sleep -Seconds 10
    $p.Refresh()
    $c2 = $p.CPU
    '{0} MB rss  ·  {1}% of one core over 10s  ·  {2} threads' -f `
        [math]::Round($p.WorkingSet64 / 1MB, 1), [math]::Round(($c2 - $c1) * 10, 2), $p.Threads.Count
} else {
    'dcc is not running'
}

Write-Output '--- child processes of dcc (should be none) ---'
$dccPid = (Get-Process dcc -ErrorAction SilentlyContinue).Id
if ($dccPid) {
    $kids = Get-CimInstance Win32_Process -Filter "ParentProcessId=$dccPid"
    if ($kids) { $kids | Select-Object ProcessId, Name | Format-Table -AutoSize } else { 'none' }
}
