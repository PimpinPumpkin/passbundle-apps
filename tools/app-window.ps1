# The app's own window on Windows, for the window tests (tools/window-test.ps1,
# tools/window-shots.ps1): dot-source it, then `Wait-AppWindow $app`.
#
# Process.MainWindowHandle isn't it. That's the first visible top-level
# window of the process that nothing owns, in z-order, so it's no window at
# all until the runner shows its window (after Flutter's first frame,
# app/windows/runner/flutter_window.cpp), and it can be another of the
# process's windows: the tooltip Windows shows for the window's Minimize,
# Maximize, or Close when the mouse rests on one, which is in front, shown,
# and owned by nothing. Then a screenshot took the tooltip's frame, gone a
# moment later (none: "Parameter is not valid"), and the smallest-size check
# sized the tooltip (98 by 98). This finds the
# runner's window by its class (FLUTTER_RUNNER_WIN32_WINDOW,
# app/windows/runner/win32_window.cpp), and waits until Windows shows it, not
# minimized, with a frame.

Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class AppWindow {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    delegate bool EnumProc(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr data);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr window, StringBuilder name, int size);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr window, uint command);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr window, int attribute, out RECT value, int size);

    static List<IntPtr> Windows(int process) {
        var found = new List<IntPtr>();
        EnumWindows((window, data) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner == (uint)process) found.Add(window);
            return true;
        }, IntPtr.Zero);
        return found;
    }

    static string ClassOf(IntPtr window) {
        var name = new StringBuilder(256);
        GetClassName(window, name, name.Capacity);
        return name.ToString();
    }

    // The window as DWM draws it, without the invisible resize borders.
    public static RECT Frame(IntPtr window) {
        RECT r;
        if (DwmGetWindowAttribute(window, 9 /* DWMWA_EXTENDED_FRAME_BOUNDS */, out r, Marshal.SizeOf(typeof(RECT))) != 0) {
            r = new RECT();
        }
        return r;
    }

    // The runner's window once it's shown, not minimized, with a frame; else zero.
    public static IntPtr Shown(int process) {
        foreach (var window in Windows(process)) {
            if (ClassOf(window) != "FLUTTER_RUNNER_WIN32_WINDOW") continue;
            var frame = Frame(window);
            if (IsWindowVisible(window) && !IsIconic(window) && frame.Right > frame.Left && frame.Bottom > frame.Top) {
                return window;
            }
        }
        return IntPtr.Zero;
    }

    // Every top-level window of the process, for a failure's message.
    public static string Describe(int process) {
        var lines = new List<string>();
        foreach (var window in Windows(process)) {
            RECT r;
            GetWindowRect(window, out r);
            var f = Frame(window);
            lines.Add(String.Format("0x{0:X} {1}, {2}{3}{4}, at {5},{6} to {7},{8} (frame {9},{10} to {11},{12})",
                window.ToInt64(), ClassOf(window), IsWindowVisible(window) ? "shown" : "hidden",
                IsIconic(window) ? ", minimized" : "", GetWindow(window, 4 /* GW_OWNER */) != IntPtr.Zero ? ", owned" : "",
                r.Left, r.Top, r.Right, r.Bottom, f.Left, f.Top, f.Right, f.Bottom));
        }
        return lines.Count == 0 ? "none" : String.Join("; ", lines);
    }
}
"@

# The app's window, once Windows shows it. Says so when it came later than
# the app said it was ready, or when Process.MainWindowHandle would have
# answered something else.
function Wait-AppWindow([System.Diagnostics.Process] $App, [int] $Seconds = 30) {
    $started = Get-Date
    $first = $true
    while ($true) {
        $window = [AppWindow]::Shown($App.Id)
        if ($window -ne [IntPtr]::Zero) { break }
        if ($first) {
            Write-Host "The app's window isn't showing yet. The app's windows: $([AppWindow]::Describe($App.Id))."
            $first = $false
        }
        if ($App.HasExited) { throw 'The app quit before its window showed.' }
        if (((Get-Date) - $started).TotalSeconds -gt $Seconds) {
            throw "The app's window didn't show within $Seconds seconds. The app's windows: $([AppWindow]::Describe($App.Id))."
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $first) {
        Write-Host "The app's window showed $([int]((Get-Date) - $started).TotalMilliseconds) ms after the app was ready."
    }
    $App.Refresh()
    if ($App.MainWindowHandle -ne $window) {
        Write-Host ("Process.MainWindowHandle answers 0x{0:X}, not the app's window 0x{1:X}. The app's windows: {2}." -f $App.MainWindowHandle.ToInt64(), $window.ToInt64(), [AppWindow]::Describe($App.Id))
    }
    $window
}
