#ifndef RUNNER_FLUTTER_WINDOW_H_
#define RUNNER_FLUTTER_WINDOW_H_

#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <flutter/method_channel.h>

#include <memory>

#include "win32_window.h"

// A window that does nothing but host a Flutter view.
class FlutterWindow : public Win32Window {
 public:
  // Creates a new FlutterWindow hosting a Flutter view running |project|.
  explicit FlutterWindow(const flutter::DartProject& project);
  virtual ~FlutterWindow();

 protected:
  // Win32Window:
  bool OnCreate() override;
  void OnDestroy() override;
  LRESULT MessageHandler(HWND window, UINT const message, WPARAM const wparam,
                         LPARAM const lparam) noexcept override;
  void OnFrameChanged() override;
  void OnButtonsChanged(Button hovered, Button pressed) override;

 private:
  // The project to run.
  flutter::DartProject project_;

  // The Flutter instance hosted by this window.
  std::unique_ptr<flutter::FlutterViewController> flutter_controller_;

  // Tells the app the computer is about to sleep, so it can lock first
  // (docs/core.md, "Locking by itself").
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> computer_;

  // The window, and the app's top edge in it (app/lib/window_chrome.dart):
  // the app's look for the frame (SystemBars in app/lib/theme.dart), where
  // its band and buttons are, and the buttons pressed from the keyboard;
  // the window maximized or not, active or not, and the mouse over the
  // buttons, the other way.
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> window_;

  // The window's state, as the app hears it (|window_|'s "frame").
  flutter::EncodableValue Frame() const;
};

#endif  // RUNNER_FLUTTER_WINDOW_H_
