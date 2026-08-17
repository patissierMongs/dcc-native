# Which linkers are available on this machine?
Write-Output '--- gcc / mingw (for the gnu toolchain) ---'
foreach ($c in 'gcc','cc','clang','lld-link') {
    $p = Get-Command $c -ErrorAction SilentlyContinue
    if ($p) { '{0} -> {1}' -f $c, $p.Source }
}

Write-Output '--- Visual Studio / Build Tools ---'
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vswhere) {
    & $vswhere -products * -property installationPath
} else {
    'vswhere not found'
}

Write-Output '--- any link.exe under Program Files ---'
Get-ChildItem 'C:\Program Files\Microsoft Visual Studio','C:\Program Files (x86)\Microsoft Visual Studio' -Filter 'link.exe' -Recurse -ErrorAction SilentlyContinue |
    Select-Object -First 3 -ExpandProperty FullName

Write-Output '--- Windows SDK ---'
Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\Lib' -Directory -ErrorAction SilentlyContinue |
    Select-Object -ExpandProperty Name

Write-Output '--- scoop / winget available? ---'
foreach ($c in 'scoop','winget') {
    $p = Get-Command $c -ErrorAction SilentlyContinue
    if ($p) { '{0} -> {1}' -f $c, $p.Source }
}
