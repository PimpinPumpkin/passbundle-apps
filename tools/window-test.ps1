# Checks the app's window on Windows, as tools/window-test.sh does on Linux:
#
# - the app draws the window's top: its client area goes up to the window's
#   top, with no caption over it;
# - minimize, maximize, and close are the app's, 46 by 32 at the top right,
#   and Windows hears them as a caption's buttons (WM_NCHITTEST answers
#   HTMINBUTTON, HTMAXBUTTON, which shows Windows 11's snap layouts, and
#   HTCLOSE), while Flutter's own window lets them through (HTTRANSPARENT);
#   the band where nothing is answers HTCAPTION, the search field HTCLIENT,
#   and the top edge HTTOP;
# - with the mouse, as a person would: minimize, maximize (and, maximized,
#   nothing of the app's off the screen), restore, dragging the band, a
#   double click on it, and close;
# - screen readers find the buttons by name (MSAA, as Narrator and NVDA
#   read Flutter's apps);
# - it opens no bigger than the screen, can't be made smaller than 400 by
#   560, and opens again where and how big it was left when it closed (kept
#   under HKCU\Software\PassBundle\Window, which this removes before and
#   after).
#
# It needs a build with test hooks (tools/measure.sh says how), opened on
# the sample vault so the vault shows.
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
    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)]
    public struct MONITORINFO { public int Size; public RECT Monitor, Work; public uint Flags; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr window, ref POINT point);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int x, int y, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr window, uint command);
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr window, uint flags);
    [DllImport("user32.dll")] public static extern bool GetMonitorInfo(IntPtr monitor, ref MONITORINFO info);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr window, int attribute, out RECT value, int size);
    public static RECT Frame(IntPtr window) {
        RECT r; DwmGetWindowAttribute(window, 9 /* DWMWA_EXTENDED_FRAME_BOUNDS */, out r, Marshal.SizeOf(typeof(RECT))); return r;
    }
    public static int HitTest(IntPtr window, int x, int y) {
        return (int)SendMessage(window, 0x0084 /* WM_NCHITTEST */, IntPtr.Zero, (IntPtr)((y << 16) | (x & 0xFFFF)));
    }
}
"@
# A window's accessible objects, read as a screen reader reads them (MSAA's
# IAccessible, as far as its name).
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
[ComImport, Guid("618736E0-3C3D-11CF-810C-00AA00389B71"), InterfaceType(ComInterfaceType.InterfaceIsDual)]
public interface IAcc {
    [return: MarshalAs(UnmanagedType.IDispatch)] object get_accParent();
    int get_accChildCount();
    [return: MarshalAs(UnmanagedType.IDispatch)] object get_accChild([MarshalAs(UnmanagedType.Struct)] object child);
    [return: MarshalAs(UnmanagedType.BStr)] string get_accName([MarshalAs(UnmanagedType.Struct)] object child);
}
public static class Msaa {
    [DllImport("oleacc.dll")]
    static extern int AccessibleObjectFromWindow(IntPtr window, uint id, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out IAcc accessible);
    [DllImport("oleacc.dll")]
    static extern int AccessibleChildren(IAcc container, int start, int count, [Out, MarshalAs(UnmanagedType.LPArray, ArraySubType = UnmanagedType.Struct)] object[] children, out int obtained);
    public static string[] Names(IntPtr window) {
        var names = new List<string>();
        var iid = new Guid("618736E0-3C3D-11CF-810C-00AA00389B71");
        IAcc root;
        if (AccessibleObjectFromWindow(window, 0xFFFFFFFC /* OBJID_CLIENT */, ref iid, out root) == 0 && root != null) {
            Walk(root, names, 0);
        }
        return names.ToArray();
    }
    static void Walk(IAcc node, List<string> names, int depth) {
        if (depth > 60) return;
        try { var name = node.get_accName(0); if (!string.IsNullOrEmpty(name)) names.Add(name); } catch { }
        int count = 0;
        try { count = node.get_accChildCount(); } catch { }
        if (count <= 0) return;
        var children = new object[count];
        int obtained;
        if (AccessibleChildren(node, 0, count, children, out obtained) != 0) return;
        for (int i = 0; i < obtained; i++) {
            var child = children[i] as IAcc;
            if (child != null) {
                Walk(child, names, depth + 1);
            } else if (children[i] is int) {
                try { var name = node.get_accName(children[i]); if (!string.IsNullOrEmpty(name)) names.Add(name); } catch { }
            }
        }
    }
}
"@
$noZOrder = 0x0004
$noMove = 0x0002
$key = 'HKCU:\Software\PassBundle\Window'
Remove-Item $key -ErrorAction SilentlyContinue

$sample = Join-Path $PSScriptRoot '..\app\test\sample'
$env:PB_SECRETS = 'memory'
$env:PB_MEASURE = '1'
$env:PB_MEASURE_PASSWORD = 'correct horse battery staple 1'

function Open-App {
    $data = Join-Path ([IO.Path]::GetTempPath()) ('pb-window-' + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $data | Out-Null
    Copy-Item (Join-Path $sample 'settings.json'), (Join-Path $sample 'sync.json') $data
    $env:PB_DATA_DIR = $data
    $lines = [IO.Path]::GetTempFileName()
    $env:PB_MEASURE_FILE = $lines
    $app = Start-Process -FilePath $Exe -PassThru
    for ($i = 0; $i -lt 120; $i++) {
        if ((Get-Content $lines -ErrorAction SilentlyContinue) -match '^ready') { break }
        if ($app.HasExited) { throw 'The app quit before it was ready.' }
        Start-Sleep -Milliseconds 500
    }
    if (-not ((Get-Content $lines -ErrorAction SilentlyContinue) -match '^ready')) { throw "The app wasn't ready within a minute." }
    $app.Refresh()
    if ($app.MainWindowHandle -eq [IntPtr]::Zero) { throw "The app's window didn't show." }
    [Win]::SetForegroundWindow($app.MainWindowHandle) | Out-Null
    Start-Sleep -Seconds 1
    $app
}

function Get-Place([IntPtr] $window) {
    $rect = New-Object Win+RECT
    [Win]::GetWindowRect($window, [ref] $rect) | Out-Null
    "$($rect.Right - $rect.Left)x$($rect.Bottom - $rect.Top)+$($rect.Left)+$($rect.Top)"
}

# A point of the app's content, in logical pixels, on the screen.
function To-Screen([IntPtr] $window, [double] $x, [double] $y) {
    $scale = [Win]::GetDpiForWindow($window) / 96.0
    $point = New-Object Win+POINT
    $point.X = [int]($x * $scale)
    $point.Y = [int]($y * $scale)
    [Win]::ClientToScreen($window, [ref] $point) | Out-Null
    $point
}

function Client-Width([IntPtr] $window) {
    $client = New-Object Win+RECT
    [Win]::GetClientRect($window, [ref] $client) | Out-Null
    ($client.Right - $client.Left) / ([Win]::GetDpiForWindow($window) / 96.0)
}

function Click([IntPtr] $window, [double] $x, [double] $y, [int] $times = 1) {
    $p = To-Screen $window $x $y
    [Win]::SetCursorPos($p.X, $p.Y) | Out-Null
    Start-Sleep -Milliseconds 300
    for ($i = 0; $i -lt $times; $i++) {
        [Win]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero) # left down
        Start-Sleep -Milliseconds 40
        [Win]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) # left up
        Start-Sleep -Milliseconds 60
    }
    Start-Sleep -Milliseconds 800
}

function Close-App($app) {
    # As the close button does: the window closes, and keeps its place.
    [Win]::PostMessage($app.MainWindowHandle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (-not $app.WaitForExit(20000)) { Stop-Process -Id $app.Id; throw "The app didn't close." }
}

function Expect-Hit([IntPtr] $window, [string] $what, [double] $x, [double] $y, [int] $expected, [string] $name) {
    $p = To-Screen $window $x $y
    $hit = [Win]::HitTest($window, $p.X, $p.Y)
    if ($hit -ne $expected) { throw "$what answers $hit, not $name ($expected)." }
    Write-Output "$what answers $name."
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

    # No caption: the client area starts at the window's visible top.
    $frame = [Win]::Frame($window)
    $origin = To-Screen $window 0 0
    if ([math]::Abs($origin.Y - $frame.Top) -gt 1) {
        throw "The client area starts $($origin.Y - $frame.Top) pixels under the window's top: there's a caption."
    }
    Write-Output 'No caption: the app goes up to the window''s top.'

    # Windows hears the app's buttons as a caption's, and Flutter's window
    # lets it.
    $width = Client-Width $window
    Expect-Hit $window 'Minimize' ($width - 46 * 2.5) 16 8 'HTMINBUTTON'
    Expect-Hit $window 'Maximize' ($width - 46 * 1.5) 16 9 'HTMAXBUTTON'
    Expect-Hit $window 'Close' ($width - 23) 16 20 'HTCLOSE'
    Expect-Hit $window 'The band where nothing is' ($width - 250) 20 2 'HTCAPTION'
    Expect-Hit $window 'The search field' 320 36 1 'HTCLIENT'
    Expect-Hit $window 'The top edge' ($width / 2) 1 12 'HTTOP'
    # Flutter's view: the window's one child.
    $view = [Win]::GetWindow($window, 5) # GW_CHILD
    if ($view -eq [IntPtr]::Zero) { throw "No Flutter view in the window." }
    $p = To-Screen $window ($width - 46 * 1.5) 16
    $hit = [Win]::HitTest($view, $p.X, $p.Y)
    if ($hit -ne -1) { throw "Flutter's window answers $hit over maximize, not HTTRANSPARENT (-1)." }
    Write-Output 'Flutter''s window lets the caption through (HTTRANSPARENT).'

    # Screen readers find the buttons by name: through Flutter's window's
    # accessibility (MSAA, which Narrator and NVDA read Flutter's apps
    # through; asking turns it on, and it takes a moment to build).
    $names = @()
    for ($i = 0; $i -lt 40; $i++) {
        $names = [Msaa]::Names($view)
        if ($names -contains 'Minimize' -and $names -contains 'Maximize' -and $names -contains 'Close') { break }
        Start-Sleep -Milliseconds 250
    }
    foreach ($name in 'Minimize', 'Maximize', 'Close') {
        if ($names -notcontains $name) {
            throw "A screen reader finds no button named $name, among: $($names -join ', ')."
        }
    }
    Write-Output 'A screen reader finds Minimize, Maximize, and Close.'

    # With the mouse: dragging the band moves the window.
    $before = [Win]::Frame($window)
    $p = To-Screen $window ($width - 250) 20
    [Win]::SetCursorPos($p.X, $p.Y) | Out-Null
    Start-Sleep -Milliseconds 300
    [Win]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    for ($i = 1; $i -le 10; $i++) {
        [Win]::SetCursorPos($p.X + 8 * $i, $p.Y + 6 * $i) | Out-Null
        Start-Sleep -Milliseconds 40
    }
    [Win]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 800
    $after = [Win]::Frame($window)
    if ($after.Left - $before.Left -ne 80 -or $after.Top - $before.Top -ne 60) {
        throw "Dragging the band moved the window by $($after.Left - $before.Left), $($after.Top - $before.Top), not 80, 60."
    }
    Write-Output 'Dragging the band moved the window.'
    # Back where it was, all of it on CI's small screen.
    [Win]::SetWindowPos($window, [IntPtr]::Zero, $rect.Left, $rect.Top, 0, 0, $noZOrder -bor 0x0001) | Out-Null
    Start-Sleep -Milliseconds 500

    # A double click on the band maximizes, and again restores.
    Click $window ($width - 250) 20 2
    if (-not [Win]::IsZoomed($window)) { throw "A double click on the band didn't maximize." }
    Click $window ((Client-Width $window) - 250) 20 2
    if ([Win]::IsZoomed($window)) { throw "A second double click didn't restore." }
    Write-Output 'A double click on the band maximizes, and restores.'

    # The buttons: minimize, and back.
    Click $window ((Client-Width $window) - 46 * 2.5) 16
    if (-not [Win]::IsIconic($window)) { throw "Minimize didn't." }
    [Win]::ShowWindow($window, 9) | Out-Null # SW_RESTORE
    Start-Sleep -Seconds 1
    [Win]::SetForegroundWindow($window) | Out-Null
    Write-Output 'Minimize minimized it.'

    # Maximize: the app's content exactly on the screen's work area, none
    # of it off the screen; then restore.
    Click $window ((Client-Width $window) - 46 * 1.5) 16
    if (-not [Win]::IsZoomed($window)) { throw "Maximize didn't." }
    $info = New-Object Win+MONITORINFO
    $info.Size = [Runtime.InteropServices.Marshal]::SizeOf($info)
    [Win]::GetMonitorInfo([Win]::MonitorFromWindow($window, 2), [ref] $info) | Out-Null
    $client = New-Object Win+RECT
    [Win]::GetClientRect($window, [ref] $client) | Out-Null
    $origin = To-Screen $window 0 0
    if ($origin.X -ne $info.Work.Left -or $origin.Y -ne $info.Work.Top -or
        $client.Right -ne $info.Work.Right - $info.Work.Left -or $client.Bottom -ne $info.Work.Bottom - $info.Work.Top) {
        throw "Maximized, the content is $($client.Right)x$($client.Bottom) at $($origin.X),$($origin.Y), not the work area $($info.Work.Left),$($info.Work.Top) to $($info.Work.Right),$($info.Work.Bottom)."
    }
    Write-Output 'Maximized, the app fills the work area, none of it off the screen.'
    Expect-Hit $window 'Restore, maximized' ((Client-Width $window) - 46 * 1.5) 16 9 'HTMAXBUTTON'
    Expect-Hit $window 'The very top, maximized' ((Client-Width $window) / 2) 0 2 'HTCAPTION'
    Click $window ((Client-Width $window) - 46 * 1.5) 16
    if ([Win]::IsZoomed($window)) { throw "Restore didn't." }
    Write-Output 'Maximize and restore.'

    # Close.
    Click $window ((Client-Width $window) - 23) 16
    if (-not $app.WaitForExit(20000)) { Stop-Process -Id $app.Id; throw "Close didn't close the window." }
    Write-Output 'Close closed it.'

    # Smaller than it may be: it stops at 400 by 560.
    Remove-Item $key -Recurse -ErrorAction SilentlyContinue
    $app = Open-App
    $window = $app.MainWindowHandle
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
}
