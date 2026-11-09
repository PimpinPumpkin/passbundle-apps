# Screenshots of the app's real window on Windows, light and dark, as
# tools/window-shots.sh takes them on a Mac and on Linux: the window as
# Windows shows it, its top edge included, on the sample vault with an
# item chosen. Needs a build with test hooks (tools/measure.sh says how).
#
#   tools/window-shots.ps1 -Exe <passbundle.exe> -Out <folder> -Name <name>
#
# writes <folder>\<name>-light.png and <folder>\<name>-dark.png.
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [Parameter(Mandatory = $true)] [string] $Out,
    [Parameter(Mandatory = $true)] [string] $Name
)
$ErrorActionPreference = 'Stop'

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Shot {
    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr window, ref POINT point);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int x, int y, uint data, UIntPtr extra);
}
"@
. (Join-Path $PSScriptRoot 'app-window.ps1')
Add-Type -AssemblyName System.Drawing
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$sample = Join-Path $PSScriptRoot '..\app\test\sample'

foreach ($theme in 'light', 'dark') {
    $data = Join-Path ([IO.Path]::GetTempPath()) ('pb-shot-' + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $data | Out-Null
    Copy-Item (Join-Path $sample 'settings.json'), (Join-Path $sample 'sync.json') $data
    $lines = [IO.Path]::GetTempFileName()
    $env:PB_DATA_DIR = $data
    $env:PB_SECRETS = 'memory'
    $env:PB_THEME = $theme
    $env:PB_MEASURE = '1'
    $env:PB_MEASURE_FILE = $lines
    $env:PB_MEASURE_PASSWORD = 'correct horse battery staple 1'
    # Windows otherwise leaves the window out of screenshots.
    $env:PB_ALLOW_SCREENSHOTS = '1'
    # The mouse off where the window opens: left over its buttons (by
    # tools/window-test.ps1's Close), Windows shows their tooltip as it opens.
    [Shot]::SetCursorPos(0, 0) | Out-Null
    $app = Start-Process -FilePath $Exe -PassThru
    for ($i = 0; $i -lt 120; $i++) {
        if ((Get-Content $lines -ErrorAction SilentlyContinue) -match '^ready') { break }
        if ($app.HasExited) { throw 'The app quit before it was ready.' }
        Start-Sleep -Milliseconds 500
    }
    if (-not ((Get-Content $lines -ErrorAction SilentlyContinue) -match '^ready')) { throw "The app wasn't ready within a minute." }
    # The app's window, as Windows shows it (tools/app-window.ps1).
    $window = Wait-AppWindow $app
    [Shot]::SetForegroundWindow($window) | Out-Null
    Start-Sleep -Seconds 1
    # An item, the first in the list, chosen: its actions show along the
    # top (a caption, where there is one, only moves the list down).
    $scale = [Shot]::GetDpiForWindow($window) / 96.0
    $point = New-Object Shot+POINT
    $point.X = [int](380 * $scale)
    $point.Y = [int](150 * $scale)
    [Shot]::ClientToScreen($window, [ref] $point) | Out-Null
    [Shot]::SetCursorPos($point.X, $point.Y) | Out-Null
    Start-Sleep -Milliseconds 300
    [Shot]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    [Shot]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    # Off the window, so nothing shows as under the mouse.
    [Shot]::SetCursorPos(0, 0) | Out-Null
    Start-Sleep -Milliseconds 1500
    $frame = [AppWindow]::Frame($window)
    $width = $frame.Right - $frame.Left
    $height = $frame.Bottom - $frame.Top
    if ($width -le 0 -or $height -le 0) {
        throw "The app's window has no size to take ($($width)x$($height)). The app's windows: $([AppWindow]::Describe($app.Id))."
    }
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    [System.Drawing.Graphics]::FromImage($bitmap).CopyFromScreen($frame.Left, $frame.Top, 0, 0, $bitmap.Size)
    $shot = Join-Path $Out "$Name-$theme.png"
    $bitmap.Save($shot, [System.Drawing.Imaging.ImageFormat]::Png)
    Stop-Process -Id $app.Id
    $app.WaitForExit(10000) | Out-Null
    Remove-Item $data -Recurse -Force -ErrorAction SilentlyContinue
    Write-Output $shot
}
Remove-Item 'HKCU:\Software\PassBundle\Window' -Recurse -ErrorAction SilentlyContinue
