#include "win32_window.h"

#include <commctrl.h>
#include <dwmapi.h>
#include <flutter_windows.h>
#include <shellapi.h>
#include <windowsx.h>

#include "resource.h"

namespace {

/// Window attribute that enables dark mode window decorations.
///
/// Redefined in case the developer's machine has a Windows SDK older than
/// version 10.0.22000.0.
/// See: https://docs.microsoft.com/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute
#ifndef DWMWA_USE_IMMERSIVE_DARK_MODE
#define DWMWA_USE_IMMERSIVE_DARK_MODE 20
#endif

constexpr const wchar_t kWindowClassName[] = L"FLUTTER_RUNNER_WIN32_WINDOW";

/// Registry key for app theme preference.
///
/// A value of 0 indicates apps should use dark mode. A non-zero or missing
/// value indicates apps should use light mode.
constexpr const wchar_t kGetPreferredBrightnessRegKey[] =
  L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
constexpr const wchar_t kGetPreferredBrightnessRegValue[] = L"AppsUseLightTheme";

/// Where the window's place is kept between launches: its WINDOWPLACEMENT,
/// under the person's own key.
constexpr const wchar_t kPlacementKey[] = L"Software\\PassBundle\\Window";
constexpr const wchar_t kPlacementValue[] = L"Placement";

// The number of Win32Window objects that currently exist.
static int g_active_window_count = 0;

using EnableNonClientDpiScaling = BOOL __stdcall(HWND hwnd);

// Scale helper to convert logical scaler values to physical using passed in
// scale factor
int Scale(int source, double scale_factor) {
  return static_cast<int>(source * scale_factor);
}

// What WM_NCHITTEST answers for each caption button.
constexpr LRESULT kButtonHits[3] = {HTMINBUTTON, HTMAXBUTTON, HTCLOSE};

Win32Window::Button ButtonAt(WPARAM hit) {
  switch (hit) {
    case HTMINBUTTON:
      return Win32Window::kMinimize;
    case HTMAXBUTTON:
      return Win32Window::kMaximize;
    case HTCLOSE:
      return Win32Window::kClose;
    default:
      return Win32Window::kNoButton;
  }
}

// Dynamically loads the |EnableNonClientDpiScaling| from the User32 module.
// This API is only needed for PerMonitor V1 awareness mode.
void EnableFullDpiSupportIfAvailable(HWND hwnd) {
  HMODULE user32_module = LoadLibraryA("User32.dll");
  if (!user32_module) {
    return;
  }
  auto enable_non_client_dpi_scaling =
      reinterpret_cast<EnableNonClientDpiScaling*>(
          GetProcAddress(user32_module, "EnableNonClientDpiScaling"));
  if (enable_non_client_dpi_scaling != nullptr) {
    enable_non_client_dpi_scaling(hwnd);
  }
  FreeLibrary(user32_module);
}

}  // namespace

// Manages the Win32Window's window class registration.
class WindowClassRegistrar {
 public:
  ~WindowClassRegistrar() = default;

  // Returns the singleton registrar instance.
  static WindowClassRegistrar* GetInstance() {
    if (!instance_) {
      instance_ = new WindowClassRegistrar();
    }
    return instance_;
  }

  // Returns the name of the window class, registering the class if it hasn't
  // previously been registered.
  const wchar_t* GetWindowClass();

  // Unregisters the window class. Should only be called if there are no
  // instances of the window.
  void UnregisterWindowClass();

 private:
  WindowClassRegistrar() = default;

  static WindowClassRegistrar* instance_;

  bool class_registered_ = false;
};

WindowClassRegistrar* WindowClassRegistrar::instance_ = nullptr;

const wchar_t* WindowClassRegistrar::GetWindowClass() {
  if (!class_registered_) {
    WNDCLASS window_class{};
    window_class.hCursor = LoadCursor(nullptr, IDC_ARROW);
    window_class.lpszClassName = kWindowClassName;
    window_class.style = CS_HREDRAW | CS_VREDRAW;
    window_class.cbClsExtra = 0;
    window_class.cbWndExtra = 0;
    window_class.hInstance = GetModuleHandle(nullptr);
    window_class.hIcon =
        LoadIcon(window_class.hInstance, MAKEINTRESOURCE(IDI_APP_ICON));
    window_class.hbrBackground = 0;
    window_class.lpszMenuName = nullptr;
    window_class.lpfnWndProc = Win32Window::WndProc;
    RegisterClass(&window_class);
    class_registered_ = true;
  }
  return kWindowClassName;
}

void WindowClassRegistrar::UnregisterWindowClass() {
  UnregisterClass(kWindowClassName, nullptr);
  class_registered_ = false;
}

Win32Window::Win32Window() {
  ++g_active_window_count;
}

Win32Window::~Win32Window() {
  --g_active_window_count;
  Destroy();
}

bool Win32Window::Create(const std::wstring& title,
                         const Point& origin,
                         const Size& size) {
  Destroy();

  const wchar_t* window_class =
      WindowClassRegistrar::GetInstance()->GetWindowClass();

  const POINT target_point = {static_cast<LONG>(origin.x),
                              static_cast<LONG>(origin.y)};
  HMONITOR monitor = MonitorFromPoint(target_point, MONITOR_DEFAULTTONEAREST);
  UINT dpi = FlutterDesktopGetDpiForMonitor(monitor);
  double scale_factor = dpi / 96.0;

  HWND window = CreateWindow(
      window_class, title.c_str(), WS_OVERLAPPEDWINDOW,
      Scale(origin.x, scale_factor), Scale(origin.y, scale_factor),
      Scale(size.width, scale_factor), Scale(size.height, scale_factor),
      nullptr, nullptr, GetModuleHandle(nullptr), this);

  if (!window) {
    return false;
  }

  UpdateTheme(window);
  // The app draws the window's top (app/lib/window_chrome.dart): the frame
  // is worked out again without a caption (WM_NCCALCSIZE). The window keeps
  // its caption's styles, for Aero Snap, the window menu, and Windows'
  // animations.
  SetWindowPos(window, nullptr, 0, 0, 0, 0,
               SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER |
                   SWP_NOACTIVATE);

  return OnCreate();
}

bool Win32Window::Show() {
  return ShowWindow(window_handle_,
                    show_maximized_ ? SW_SHOWMAXIMIZED : SW_SHOWNORMAL);
}

void Win32Window::SetMinimumSize(const Size& size) {
  minimum_size_ = size;
}

bool Win32Window::RestorePlacement() {
  WINDOWPLACEMENT placement{};
  DWORD length = sizeof(placement);
  if (RegGetValue(HKEY_CURRENT_USER, kPlacementKey, kPlacementValue,
                  RRF_RT_REG_BINARY, nullptr, &placement,
                  &length) != ERROR_SUCCESS ||
      length != sizeof(placement) || placement.length != sizeof(placement)) {
    return false;
  }
  // Only onto a screen that's still there, and no bigger than it.
  RECT& rect = placement.rcNormalPosition;
  HMONITOR monitor = MonitorFromRect(&rect, MONITOR_DEFAULTTONULL);
  MONITORINFO info{};
  info.cbSize = sizeof(info);
  if (monitor == nullptr || !GetMonitorInfo(monitor, &info) ||
      rect.right <= rect.left || rect.bottom <= rect.top) {
    return false;
  }
  const LONG most_wide = info.rcWork.right - info.rcWork.left;
  const LONG most_high = info.rcWork.bottom - info.rcWork.top;
  if (rect.right - rect.left > most_wide) {
    rect.right = rect.left + most_wide;
  }
  if (rect.bottom - rect.top > most_high) {
    rect.bottom = rect.top + most_high;
  }
  show_maximized_ = placement.showCmd == SW_SHOWMAXIMIZED;
  // Still hidden: |Show| shows it once Flutter has drawn its first frame.
  placement.showCmd = SW_HIDE;
  placement.flags = 0;
  return SetWindowPlacement(window_handle_, &placement) != 0;
}

void Win32Window::SavePlacement() {
  WINDOWPLACEMENT placement{};
  placement.length = sizeof(placement);
  if (window_handle_ == nullptr ||
      !GetWindowPlacement(window_handle_, &placement)) {
    return;
  }
  // Minimized, it opens next time as it was before: maximized or not.
  const bool maximized =
      placement.showCmd == SW_SHOWMAXIMIZED ||
      (placement.showCmd == SW_SHOWMINIMIZED &&
       (placement.flags & WPF_RESTORETOMAXIMIZED) != 0);
  placement.showCmd = maximized ? SW_SHOWMAXIMIZED : SW_SHOWNORMAL;
  placement.flags = 0;
  RegSetKeyValue(HKEY_CURRENT_USER, kPlacementKey, kPlacementValue,
                 REG_BINARY, &placement, sizeof(placement));
}

// static
LRESULT CALLBACK Win32Window::WndProc(HWND const window,
                                      UINT const message,
                                      WPARAM const wparam,
                                      LPARAM const lparam) noexcept {
  if (message == WM_NCCREATE) {
    auto window_struct = reinterpret_cast<CREATESTRUCT*>(lparam);
    SetWindowLongPtr(window, GWLP_USERDATA,
                     reinterpret_cast<LONG_PTR>(window_struct->lpCreateParams));

    auto that = static_cast<Win32Window*>(window_struct->lpCreateParams);
    EnableFullDpiSupportIfAvailable(window);
    that->window_handle_ = window;
  } else if (Win32Window* that = GetThisFromHandle(window)) {
    return that->MessageHandler(window, message, wparam, lparam);
  }

  return DefWindowProc(window, message, wparam, lparam);
}

LRESULT
Win32Window::MessageHandler(HWND hwnd,
                            UINT const message,
                            WPARAM const wparam,
                            LPARAM const lparam) noexcept {
  switch (message) {
    case WM_DESTROY:
      SavePlacement();
      window_handle_ = nullptr;
      Destroy();
      if (quit_on_close_) {
        PostQuitMessage(0);
      }
      return 0;

    // Where and how big it's left: kept as the person finishes moving or
    // sizing it, and when Windows ends the session (the window may not get
    // to close), as well as when it closes.
    case WM_EXITSIZEMOVE:
      SavePlacement();
      break;

    case WM_ENDSESSION:
      if (wparam) {
        SavePlacement();
      }
      break;

    case WM_GETMINMAXINFO: {
      if (minimum_size_.width == 0 || minimum_size_.height == 0) {
        break;
      }
      // The content's smallest size on this screen, with the window's
      // frame: its sides and bottom, and no caption.
      const UINT dpi = FlutterDesktopGetDpiForHWND(hwnd);
      const double scale_factor = dpi / 96.0;
      RECT frame = {0, 0, Scale(minimum_size_.width, scale_factor),
                    Scale(minimum_size_.height, scale_factor)};
      AdjustWindowRectExForDpi(&frame, WS_OVERLAPPEDWINDOW, FALSE, 0, dpi);
      auto info = reinterpret_cast<MINMAXINFO*>(lparam);
      info->ptMinTrackSize.x = frame.right - frame.left;
      info->ptMinTrackSize.y = frame.bottom;
      return 0;
    }

    // No caption: the client area goes up to the window's top, and the app
    // draws what was there (app/lib/window_chrome.dart). Windows keeps the
    // sides and bottom, where it resizes from and draws the border.
    case WM_NCCALCSIZE:
      FitClientArea(wparam ? &reinterpret_cast<NCCALCSIZE_PARAMS*>(lparam)
                                  ->rgrc[0]
                           : reinterpret_cast<RECT*>(lparam));
      return 0;

    case WM_NCHITTEST: {
      // The sides and bottom are Windows' own; the top is ours to answer.
      const LRESULT edge = DefWindowProc(hwnd, message, wparam, lparam);
      if (edge != HTCLIENT) {
        return edge;
      }
      return HitTest({GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)});
    }

    // The caption buttons are the app's to draw, and this window's to
    // answer for: Windows would draw its own old ones over them, so their
    // mouse messages stop here. Answering HTMAXBUTTON for maximize is what
    // shows Windows 11's snap layouts over it.
    case WM_NCMOUSEMOVE: {
      const Button over = ButtonAt(wparam);
      SetButtons(over, over == pressed_ ? pressed_ : kNoButton);
      if (over == kNoButton) {
        break;
      }
      if (!tracking_mouse_) {
        TRACKMOUSEEVENT track{sizeof(track), TME_LEAVE | TME_NONCLIENT, hwnd,
                              HOVER_DEFAULT};
        tracking_mouse_ = TrackMouseEvent(&track) != 0;
      }
      return 0;
    }

    case WM_NCMOUSELEAVE:
      tracking_mouse_ = false;
      SetButtons(kNoButton, kNoButton);
      break;

    case WM_NCLBUTTONDOWN:
    case WM_NCLBUTTONDBLCLK: {
      const Button down = ButtonAt(wparam);
      if (down == kNoButton) {
        // The caption moves the window, a double click maximizes it, and
        // the top edge resizes it, all as Windows does.
        break;
      }
      SetButtons(down, down);
      return 0;
    }

    case WM_NCLBUTTONUP: {
      const Button up = ButtonAt(wparam);
      if (up == kNoButton) {
        break;
      }
      const bool pressed = up == pressed_;
      SetButtons(up, kNoButton);
      if (pressed) {
        Press(up);
      }
      return 0;
    }

    case WM_NCRBUTTONDOWN:
    case WM_NCRBUTTONDBLCLK:
    case WM_NCRBUTTONUP:
      if (ButtonAt(wparam) != kNoButton) {
        return 0;
      }
      // On the caption, the window menu, as Windows shows it.
      break;

    case WM_DPICHANGED: {
      auto newRectSize = reinterpret_cast<RECT*>(lparam);
      LONG newWidth = newRectSize->right - newRectSize->left;
      LONG newHeight = newRectSize->bottom - newRectSize->top;

      SetWindowPos(hwnd, nullptr, newRectSize->left, newRectSize->top, newWidth,
                   newHeight, SWP_NOZORDER | SWP_NOACTIVATE);

      return 0;
    }
    case WM_SIZE: {
      RECT rect = GetClientArea();
      if (child_content_ != nullptr) {
        // Size and position the child window.
        MoveWindow(child_content_, rect.left, rect.top, rect.right - rect.left,
                   rect.bottom - rect.top, TRUE);
      }
      if (wparam == SIZE_MAXIMIZED || wparam == SIZE_RESTORED) {
        const bool maximized = wparam == SIZE_MAXIMIZED;
        if (maximized != maximized_) {
          maximized_ = maximized;
          OnFrameChanged();
        }
      }
      return 0;
    }

    case WM_ACTIVATE: {
      if (child_content_ != nullptr) {
        SetFocus(child_content_);
      }
      const bool active = LOWORD(wparam) != WA_INACTIVE;
      if (active != active_) {
        active_ = active;
        OnFrameChanged();
      }
      return 0;
    }

    case WM_DWMCOLORIZATIONCOLORCHANGED:
      if (has_look_) {
        ShowLook();
      } else {
        UpdateTheme(hwnd);
      }
      return 0;
  }

  return DefWindowProc(window_handle_, message, wparam, lparam);
}

void Win32Window::Destroy() {
  OnDestroy();

  if (window_handle_) {
    DestroyWindow(window_handle_);
    window_handle_ = nullptr;
  }
  if (g_active_window_count == 0) {
    WindowClassRegistrar::GetInstance()->UnregisterWindowClass();
  }
}

Win32Window* Win32Window::GetThisFromHandle(HWND const window) noexcept {
  return reinterpret_cast<Win32Window*>(
      GetWindowLongPtr(window, GWLP_USERDATA));
}

void Win32Window::SetChildContent(HWND content) {
  child_content_ = content;
  SetParent(content, window_handle_);
  SetWindowSubclass(content, ContentProc, 1,
                    reinterpret_cast<DWORD_PTR>(this));
  RECT frame = GetClientArea();

  MoveWindow(content, frame.left, frame.top, frame.right - frame.left,
             frame.bottom - frame.top, true);

  SetFocus(child_content_);
}

RECT Win32Window::GetClientArea() {
  RECT frame;
  GetClientRect(window_handle_, &frame);
  return frame;
}

HWND Win32Window::GetHandle() {
  return window_handle_;
}

void Win32Window::SetQuitOnClose(bool quit_on_close) {
  quit_on_close_ = quit_on_close;
}

bool Win32Window::OnCreate() {
  // No-op; provided for subclasses.
  return true;
}

void Win32Window::OnDestroy() {
  // No-op; provided for subclasses.
}

void Win32Window::SetLook(bool dark, unsigned int ground) {
  has_look_ = true;
  look_dark_ = dark;
  look_ground_ = ground;
  ShowLook();
}

void Win32Window::ShowLook() {
  if (!window_handle_) {
    return;
  }
  BOOL dark = look_dark_ ? TRUE : FALSE;
  DwmSetWindowAttribute(window_handle_, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark,
                        sizeof(dark));
}

void Win32Window::SetChrome(const Box& band, std::vector<Box> holes,
                            const Box buttons[3], const bool has_buttons[3]) {
  band_ = band;
  holes_ = std::move(holes);
  for (int i = 0; i < 3; i++) {
    buttons_[i] = buttons[i];
    has_buttons_[i] = has_buttons[i];
  }
}

void Win32Window::Press(Button button) {
  if (!window_handle_) {
    return;
  }
  switch (button) {
    case kMinimize:
      PostMessage(window_handle_, WM_SYSCOMMAND, SC_MINIMIZE, 0);
      break;
    case kMaximize:
      PostMessage(window_handle_, WM_SYSCOMMAND,
                  Maximized() ? SC_RESTORE : SC_MAXIMIZE, 0);
      break;
    case kClose:
      PostMessage(window_handle_, WM_SYSCOMMAND, SC_CLOSE, 0);
      break;
    default:
      break;
  }
}

bool Win32Window::Maximized() const {
  return window_handle_ != nullptr && IsZoomed(window_handle_);
}

int Win32Window::ResizeEdge() const {
  const UINT dpi = FlutterDesktopGetDpiForHWND(window_handle_);
  return GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) +
         GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
}

void Win32Window::FitClientArea(RECT* rect) {
  const LONG top = rect->top;
  // Windows' own sides and bottom, as with a caption.
  DefWindowProc(window_handle_, WM_NCCALCSIZE, FALSE,
                reinterpret_cast<LPARAM>(rect));
  rect->top = top;
  if (!Maximized()) {
    return;
  }
  // Maximized, a window hangs its frame off the screen's edges: the top
  // comes in by as much, so nothing of the app's is off the screen.
  rect->top += ResizeEdge();
  // A taskbar that hides itself comes back when the mouse reaches its edge
  // of the screen: a maximized window leaves that edge a pixel.
  APPBARDATA state{sizeof(state)};
  if ((SHAppBarMessage(ABM_GETSTATE, &state) & ABS_AUTOHIDE) == 0) {
    return;
  }
  MONITORINFO info{sizeof(info)};
  if (!GetMonitorInfo(MonitorFromWindow(window_handle_,
                                        MONITOR_DEFAULTTONEAREST),
                      &info)) {
    return;
  }
  const auto hides_at = [&info](UINT edge) {
    APPBARDATA bar{sizeof(bar)};
    bar.uEdge = edge;
    bar.rc = info.rcMonitor;
    return SHAppBarMessage(ABM_GETAUTOHIDEBAREX, &bar) != 0;
  };
  if (hides_at(ABE_TOP)) {
    rect->top += 1;
  } else if (hides_at(ABE_BOTTOM)) {
    rect->bottom -= 1;
  } else if (hides_at(ABE_LEFT)) {
    rect->left += 1;
  } else if (hides_at(ABE_RIGHT)) {
    rect->right -= 1;
  }
}

LRESULT Win32Window::HitTest(POINT screen) const {
  POINT point = screen;
  ScreenToClient(window_handle_, &point);
  RECT client;
  GetClientRect(window_handle_, &client);
  if (point.x < client.left || point.x >= client.right ||
      point.y < client.top || point.y >= client.bottom) {
    return HTNOWHERE;
  }
  // The top edge resizes the window, as a caption's top does, but not
  // while it's maximized; its ends resize from the corners.
  const int edge = ResizeEdge();
  if (!Maximized() && point.y < edge) {
    if (point.x < edge * 2) {
      return HTTOPLEFT;
    }
    if (point.x >= client.right - edge * 2) {
      return HTTOPRIGHT;
    }
    return HTTOP;
  }
  const double scale = FlutterDesktopGetDpiForHWND(window_handle_) / 96.0;
  const double x = point.x / scale;
  const double y = point.y / scale;
  for (int i = 0; i < 3; i++) {
    if (has_buttons_[i] && buttons_[i].Contains(x, y)) {
      return kButtonHits[i];
    }
  }
  if (!band_.Contains(x, y)) {
    return HTCLIENT;
  }
  for (const Box& hole : holes_) {
    if (hole.Contains(x, y)) {
      return HTCLIENT;
    }
  }
  return HTCAPTION;
}

void Win32Window::SetButtons(Button hovered, Button pressed) {
  if (hovered == hovered_ && pressed == pressed_) {
    return;
  }
  hovered_ = hovered;
  pressed_ = pressed;
  OnButtonsChanged(hovered, pressed);
}

// static
LRESULT CALLBACK Win32Window::ContentProc(HWND window, UINT message,
                                          WPARAM wparam, LPARAM lparam,
                                          UINT_PTR id,
                                          DWORD_PTR self) noexcept {
  if (message == WM_NCHITTEST) {
    auto that = reinterpret_cast<Win32Window*>(self);
    if (that->HitTest({GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)}) !=
        HTCLIENT) {
      // The caption's, a button's, or the top edge's: the window under
      // this one, which is the app's own, answers.
      return HTTRANSPARENT;
    }
  } else if (message == WM_NCDESTROY) {
    RemoveWindowSubclass(window, ContentProc, id);
  }
  return DefSubclassProc(window, message, wparam, lparam);
}

void Win32Window::UpdateTheme(HWND const window) {
  DWORD light_mode;
  DWORD light_mode_size = sizeof(light_mode);
  LSTATUS result = RegGetValue(HKEY_CURRENT_USER, kGetPreferredBrightnessRegKey,
                               kGetPreferredBrightnessRegValue,
                               RRF_RT_REG_DWORD, nullptr, &light_mode,
                               &light_mode_size);

  if (result == ERROR_SUCCESS) {
    BOOL enable_dark_mode = light_mode == 0;
    DwmSetWindowAttribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE,
                          &enable_dark_mode, sizeof(enable_dark_mode));
  }
}
