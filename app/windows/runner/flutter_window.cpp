#include "flutter_window.h"

#include <flutter/standard_method_codec.h>

#include <optional>
#include <string>
#include <variant>
#include <vector>

#include "flutter/generated_plugin_registrant.h"

namespace {

using flutter::EncodableList;
using flutter::EncodableMap;
using flutter::EncodableValue;

// [left, top, right, bottom], in the app's logical pixels.
Win32Window::Box BoxOf(const EncodableValue& value) {
  Win32Window::Box box;
  const auto* edges = std::get_if<EncodableList>(&value);
  if (!edges || edges->size() != 4) {
    return box;
  }
  double numbers[4] = {0, 0, 0, 0};
  for (size_t i = 0; i < 4; i++) {
    const EncodableValue& edge = (*edges)[i];
    if (const auto* d = std::get_if<double>(&edge)) {
      numbers[i] = *d;
    } else if (std::holds_alternative<int32_t>(edge) ||
               std::holds_alternative<int64_t>(edge)) {
      numbers[i] = static_cast<double>(edge.LongValue());
    }
  }
  box.left = numbers[0];
  box.top = numbers[1];
  box.right = numbers[2];
  box.bottom = numbers[3];
  return box;
}

Win32Window::Box BoxAt(const EncodableMap& map, const char* key) {
  const auto found = map.find(EncodableValue(key));
  return found == map.end() ? Win32Window::Box() : BoxOf(found->second);
}

std::vector<Win32Window::Box> BoxesAt(const EncodableMap& map,
                                      const char* key) {
  std::vector<Win32Window::Box> boxes;
  const auto found = map.find(EncodableValue(key));
  if (found == map.end()) {
    return boxes;
  }
  if (const auto* list = std::get_if<EncodableList>(&found->second)) {
    for (const EncodableValue& value : *list) {
      boxes.push_back(BoxOf(value));
    }
  }
  return boxes;
}

// Where the app draws minimize, maximize, and close, if it does.
struct Buttons {
  Win32Window::Box boxes[3];
  bool has[3] = {false, false, false};
};

Buttons ButtonsAt(const EncodableMap& map) {
  Buttons buttons;
  const auto found = map.find(EncodableValue("buttons"));
  const auto* named =
      found == map.end() ? nullptr : std::get_if<EncodableMap>(&found->second);
  if (!named) {
    return buttons;
  }
  const char* names[3] = {"minimize", "maximize", "close"};
  for (int i = 0; i < 3; i++) {
    const auto box = named->find(EncodableValue(names[i]));
    if (box != named->end()) {
      buttons.boxes[i] = BoxOf(box->second);
      buttons.has[i] = true;
    }
  }
  return buttons;
}

EncodableValue ButtonName(Win32Window::Button button) {
  switch (button) {
    case Win32Window::kMinimize:
      return EncodableValue("minimize");
    case Win32Window::kMaximize:
      return EncodableValue("maximize");
    case Win32Window::kClose:
      return EncodableValue("close");
    default:
      return EncodableValue();
  }
}

}  // namespace

FlutterWindow::FlutterWindow(const flutter::DartProject& project)
    : project_(project) {}

FlutterWindow::~FlutterWindow() {}

bool FlutterWindow::OnCreate() {
  if (!Win32Window::OnCreate()) {
    return false;
  }

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());
  computer_ = std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
      flutter_controller_->engine()->messenger(), "passbundle/computer",
      &flutter::StandardMethodCodec::GetInstance());
  window_ = std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
      flutter_controller_->engine()->messenger(), "passbundle/window",
      &flutter::StandardMethodCodec::GetInstance());
  window_->SetMethodCallHandler(
      [this](const flutter::MethodCall<flutter::EncodableValue>& call,
             std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>>
                 result) {
        const std::string& method = call.method_name();
        const auto* arguments =
            std::get_if<flutter::EncodableMap>(call.arguments());
        if (method == "frame") {
          result->Success(Frame());
        } else if (method == "minimize" || method == "maximize" ||
                   method == "close") {
          Press(method == "minimize"   ? kMinimize
                : method == "maximize" ? kMaximize
                                       : kClose);
          result->Success();
        } else if (method == "chrome" && arguments) {
          const Buttons buttons = ButtonsAt(*arguments);
          SetChrome(BoxAt(*arguments, "band"), BoxesAt(*arguments, "holes"),
                    buttons.boxes, buttons.has);
          result->Success();
        } else if (method == "look" && arguments) {
          const auto dark = arguments->find(flutter::EncodableValue("dark"));
          const auto ground =
              arguments->find(flutter::EncodableValue("ground"));
          if (dark == arguments->end() || ground == arguments->end() ||
              !std::holds_alternative<bool>(dark->second) ||
              !(std::holds_alternative<int32_t>(ground->second) ||
                std::holds_alternative<int64_t>(ground->second))) {
            result->Error("arguments", "look takes dark and ground");
            return;
          }
          SetLook(std::get<bool>(dark->second),
                  static_cast<unsigned int>(ground->second.LongValue()));
          result->Success();
        } else {
          result->NotImplemented();
        }
      });
  SetChildContent(flutter_controller_->view()->GetNativeWindow());

  flutter_controller_->engine()->SetNextFrameCallback([&]() {
    this->Show();
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  return true;
}

void FlutterWindow::OnDestroy() {
  computer_ = nullptr;
  window_ = nullptr;
  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

flutter::EncodableValue FlutterWindow::Frame() const {
  return EncodableValue(EncodableMap{
      {EncodableValue("style"), EncodableValue("windows")},
      {EncodableValue("maximized"), EncodableValue(Maximized())},
      {EncodableValue("active"), EncodableValue(Active())},
  });
}

void FlutterWindow::OnFrameChanged() {
  if (window_) {
    window_->InvokeMethod("frame",
                          std::make_unique<EncodableValue>(Frame()));
  }
}

void FlutterWindow::OnButtonsChanged(Button hovered, Button pressed) {
  if (window_) {
    window_->InvokeMethod(
        "buttons", std::make_unique<EncodableValue>(EncodableMap{
                       {EncodableValue("hovered"), ButtonName(hovered)},
                       {EncodableValue("pressed"), ButtonName(pressed)},
                   }));
  }
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  switch (message) {
    case WM_FONTCHANGE:
      flutter_controller_->engine()->ReloadSystemFonts();
      break;
    case WM_POWERBROADCAST:
      if (wparam == PBT_APMSUSPEND && computer_) {
        computer_->InvokeMethod("sleeping", nullptr);
      }
      break;
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}
