# Capture the composited desktop without going through the MCP screenshot
# server, and downscale so it is cheap to look at.
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.X, $b.Y, 0, 0, $bmp.Size)
$g.Dispose()

$scale = 0.30
$sw = [int]($b.Width * $scale)
$sh = [int]($b.Height * $scale)
$small = New-Object System.Drawing.Bitmap($sw, $sh)
$g2 = [System.Drawing.Graphics]::FromImage($small)
$g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g2.DrawImage($bmp, 0, 0, $sw, $sh)
$g2.Dispose()

$out = Join-Path $PSScriptRoot 'screen.png'
$small.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$small.Dispose(); $bmp.Dispose()

'captured {0}x{1} (virtual {2}x{3} at {4},{5}) -> {6}' -f $sw, $sh, $b.Width, $b.Height, $b.X, $b.Y, $out
