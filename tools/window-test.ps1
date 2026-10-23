# Checks the app's window on Windows, as tools/window-test.sh does on Linux:
# it opens no bigger than the screen, can't be made smaller than 400 by 560,
# and opens again where and how big it was left when it closed (kept under
# HKCU\Software\PassBundle\Window, which this removes before and after). It
# needs a build with test hooks (tools/measure.sh says how), so it opens on
# an empty folder of its own.
#
#   tools/window-test.ps1 -Exe <passbundle.exe>
param(
    [Parameter(Mandatory = $true)] [string] $Exe
)
$ErrorActionPreference = 'Stop'

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
}
"@
$noZOrder = 0x0004
$noMove = 0x0002
$key = 'HKCU:\Software\PassBundle\Window'
Remove-Item $key -ErrorAction SilentlyContinue

$data = Join-Path ([IO.Path]::GetTempPath()) ('pb-window-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $data | Out-Null
$env:PB_DATA_DIR = $data
$env:PB_SECRETS = 'memory'

function Open-App {
    $app = Start-Process -FilePath $Exe -PassThru
    for ($i = 0; $i -lt 60; $i++) {
        $app.Refresh()
        if ($app.MainWindowHandle -ne [IntPtr]::Zero) { break }
        if ($app.HasExited) { throw 'The app quit before its window showed.' }
        Start-Sleep -Milliseconds 500
    }
    if ($app.MainWindowHandle -eq [IntPtr]::Zero) { throw "The app's window didn't show within 30 seconds." }
    # Its first frame.
    Start-Sleep -Seconds 2
    $app
}

function Get-Place([IntPtr] $window) {
    $rect = New-Object Win+RECT
    [Win]::GetWindowRect($window, [ref] $rect) | Out-Null
    "$($rect.Right - $rect.Left)x$($rect.Bottom - $rect.Top)+$($rect.Left)+$($rect.Top)"
}

function Close-App($app) {
    # As the close button does: the window closes, and keeps its place.
    [Win]::PostMessage($app.MainWindowHandle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (-not $app.WaitForExit(20000)) { Stop-Process -Id $app.Id; throw "The app didn't close." }
}

try {
    Add-Type -AssemblyName System.Windows.Forms
    $work = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea

    $app = Open-App
    $window = $app.MainWindowHandle
    $scale = [Win]::GetDpiForWindow($window) / 96.0
    $first = Get-Place $window
    Write-Output "Opened at $first on a $($work.Width)x$($work.Height) work area, at $($scale * 100)%."
    $rect = New-Object Win+RECT
    [Win]::GetWindowRect($window, [ref] $rect) | Out-Null
    if (($rect.Right - $rect.Left) -gt $work.Width -or ($rect.Bottom - $rect.Top) -gt $work.Height) {
        throw 'It opened bigger than the screen.'
    }

    # Smaller than it may be: it stops at 400 by 560.
    [Win]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 100, 100, $noZOrder -bor $noMove) | Out-Null
    $client = New-Object Win+RECT
    [Win]::GetClientRect($window, [ref] $client) | Out-Null
    $width = ($client.Right - $client.Left) / $scale
    $height = ($client.Bottom - $client.Top) / $scale
    Write-Output "Made as small as it goes: ${width}x${height}."
    if ($width -lt 399 -or $height -lt 559) { throw "It went smaller than 400 by 560 (${width}x${height})." }

    # Moved and sized as someone would, then closed.
    [Win]::SetWindowPos($window, [IntPtr]::Zero, 60, 50, [int](900 * $scale), [int](700 * $scale), $noZOrder) | Out-Null
    Start-Sleep -Milliseconds 500
    $left = Get-Place $window
    Write-Output "Left at $left."
    Close-App $app
    if (-not (Get-ItemProperty $key -Name Placement -ErrorAction SilentlyContinue)) { throw "Its place wasn't kept." }

    $app = Open-App
    $again = Get-Place $app.MainWindowHandle
    Write-Output "Opened again at $again."
    Close-App $app
    if ($again -ne $left) { throw "It opened at $again, not where it was left ($left)." }
    Write-Output 'The window opens again where and how big it was left.'
} finally {
    Remove-Item $key -Recurse -ErrorAction SilentlyContinue
    Remove-Item $data -Recurse -Force -ErrorAction SilentlyContinue
}
