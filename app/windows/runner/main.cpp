#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <flutter_windows.h>
#include <windows.h>

#include <algorithm>

#include "flutter_window.h"
#include "utils.h"

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // Initialize COM, so that it is available for use in the library and/or
  // plugins.
  ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

  flutter::DartProject project(L"data");

  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();

  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));

  // At least 400 by 560, as on the Mac. The first time, 1200 by 800 (wide
  // enough for the menu, the list, and an item side by side; the wide layout
  // starts at 900), or as much of the main screen as there is, centered;
  // after that, where and how big the person left it.
  const Win32Window::Size minimum(400, 560);
  HMONITOR monitor = MonitorFromPoint({0, 0}, MONITOR_DEFAULTTOPRIMARY);
  MONITORINFO info{};
  info.cbSize = sizeof(info);
  GetMonitorInfo(monitor, &info);
  const double scale = FlutterDesktopGetDpiForMonitor(monitor) / 96.0;
  const double work_left = info.rcWork.left / scale;
  const double work_top = info.rcWork.top / scale;
  const double work_width = (info.rcWork.right - info.rcWork.left) / scale;
  const double work_height = (info.rcWork.bottom - info.rcWork.top) / scale;
  const unsigned int width = static_cast<unsigned int>(
      std::max<double>(minimum.width, std::min(1200.0, work_width - 40)));
  const unsigned int height = static_cast<unsigned int>(
      std::max<double>(minimum.height, std::min(800.0, work_height - 40)));
  const Win32Window::Point origin(
      static_cast<unsigned int>(
          std::max(0.0, work_left + (work_width - width) / 2)),
      static_cast<unsigned int>(
          std::max(0.0, work_top + (work_height - height) / 2)));

  FlutterWindow window(project);
  window.SetMinimumSize(minimum);
  if (!window.Create(L"PassBundle", origin, Win32Window::Size(width, height))) {
    return EXIT_FAILURE;
  }
  window.RestorePlacement();
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();
  return EXIT_SUCCESS;
}
