#ifndef RUNNER_WIN32_WINDOW_H_
#define RUNNER_WIN32_WINDOW_H_

#include <windows.h>

#include <functional>
#include <memory>
#include <string>
#include <vector>

// A class abstraction for a high DPI-aware Win32 Window. Intended to be
// inherited from by classes that wish to specialize with custom
// rendering and input handling
class Win32Window {
 public:
  struct Point {
    unsigned int x;
    unsigned int y;
    Point(unsigned int x, unsigned int y) : x(x), y(y) {}
  };

  struct Size {
    unsigned int width;
    unsigned int height;
    Size(unsigned int width, unsigned int height)
        : width(width), height(height) {}
  };

  Win32Window();
  virtual ~Win32Window();

  // Creates a win32 window with |title| that is positioned and sized using
  // |origin| and |size|. New windows are created on the default monitor. Window
  // sizes are specified to the OS in physical pixels, hence to ensure a
  // consistent size this function will scale the inputted width and height as
  // as appropriate for the default monitor. The window is invisible until
  // |Show| is called. Returns true if the window was created successfully.
  bool Create(const std::wstring& title, const Point& origin, const Size& size);

  // Show the current window. Returns true if the window was successfully shown.
  bool Show();

  // Release OS resources associated with window.
  void Destroy();

  // Inserts |content| into the window tree.
  void SetChildContent(HWND content);

  // Returns the backing Window handle to enable clients to set icon and other
  // window properties. Returns nullptr if the window has been destroyed.
  HWND GetHandle();

  // If true, closing this window will quit the application.
  void SetQuitOnClose(bool quit_on_close);

  // Return a RECT representing the bounds of the current client area.
  RECT GetClientArea();

  // The smallest the window's content may be, in logical pixels (scaled for
  // the screen it's on).
  void SetMinimumSize(const Size& size);

  // Puts the window back where and how big the person left it, if that's
  // still on a screen (the screens may have changed since). Before |Show|.
  // Returns whether there was a place to go back to.
  bool RestorePlacement();

  // What's left of the window's frame as the app is (SystemBars in
  // app/lib/theme.dart), whatever Windows' own setting: its border and
  // window menu dark or light. |ground| is the app's ground, 0xRRGGBB.
  void SetLook(bool dark, unsigned int ground);

  // A box in the app's logical pixels, from the window's top left.
  struct Box {
    double left = 0, top = 0, right = 0, bottom = 0;
    bool Contains(double x, double y) const {
      return x >= left && x < right && y >= top && y < bottom;
    }
  };

  // The window's caption buttons, as the app draws them.
  enum Button { kNoButton = -1, kMinimize = 0, kMaximize = 1, kClose = 2 };

  // Where the app's top band is, what in it takes clicks (|holes|), and
  // where it draws minimize, maximize, and close (app/lib/window_chrome.dart):
  // the rest of the band is the window's caption.
  void SetChrome(const Box& band, std::vector<Box> holes,
                 const Box buttons[3], const bool has_buttons[3]);

  // As the caption button would: minimizes, maximizes or restores, closes.
  void Press(Button button);

  bool Maximized() const;
  bool Active() const { return active_; }

 protected:
  // Processes and route salient window messages for mouse handling,
  // size change and DPI. Delegates handling of these to member overloads that
  // inheriting classes can handle.
  virtual LRESULT MessageHandler(HWND window,
                                 UINT const message,
                                 WPARAM const wparam,
                                 LPARAM const lparam) noexcept;

  // Called when CreateAndShow is called, allowing subclass window-related
  // setup. Subclasses should return false if setup fails.
  virtual bool OnCreate();

  // Called when Destroy is called.
  virtual void OnDestroy();

  // The window was maximized or restored, or gained or lost the focus.
  virtual void OnFrameChanged() {}

  // The mouse moved over or off a caption button, or pressed or let go of
  // one: Windows answers for the mouse there, so the app only draws them.
  virtual void OnButtonsChanged(Button hovered, Button pressed) {}

 private:
  friend class WindowClassRegistrar;

  // OS callback called by message pump. Handles the WM_NCCREATE message which
  // is passed when the non-client area is being created and enables automatic
  // non-client DPI scaling so that the non-client area automatically
  // responds to changes in DPI. All other messages are handled by
  // MessageHandler.
  static LRESULT CALLBACK WndProc(HWND const window,
                                  UINT const message,
                                  WPARAM const wparam,
                                  LPARAM const lparam) noexcept;

  // Retrieves a class instance pointer for |window|
  static Win32Window* GetThisFromHandle(HWND const window) noexcept;

  // The hosted content's (Flutter's) own window, asked what's under the
  // mouse: what's the caption's it leaves to this window (HTTRANSPARENT).
  static LRESULT CALLBACK ContentProc(HWND window, UINT message, WPARAM wparam,
                                      LPARAM lparam, UINT_PTR id,
                                      DWORD_PTR self) noexcept;

  // What's under |screen| (a point on the screen), as WM_NCHITTEST answers:
  // the top edge (to resize), a caption button, the caption, or the client.
  LRESULT HitTest(POINT screen) const;

  // The top resize edge's height, inside the window as Windows 10 and 11
  // have it; and how far a maximized window hangs off the screen's top.
  int ResizeEdge() const;

  // The client area for a window rect |rect|: Windows' own sides and
  // bottom, and no caption, but on screen when maximized.
  void FitClientArea(RECT* rect);

  // The caption buttons' state changed: tells the app, if it did.
  void SetButtons(Button hovered, Button pressed);

  // Update the window frame's theme to match the system theme.
  static void UpdateTheme(HWND const window);

  // Puts the app's look (|SetLook|) on the frame.
  void ShowLook();

  // Remembers where and how big the window is, for |RestorePlacement| next time.
  void SavePlacement();

  bool quit_on_close_ = false;

  // The smallest content size, in logical pixels; none when zero.
  Size minimum_size_ = Size(0, 0);

  // It was maximized when the person left it: |Show| maximizes it again.
  bool show_maximized_ = false;

  // The app's look, once it has said (|SetLook|): then Windows' own theme
  // no longer decides the frame's.
  bool has_look_ = false;
  bool look_dark_ = false;
  unsigned int look_ground_ = 0;

  // The app's top band (|SetChrome|), in its logical pixels.
  Box band_;
  std::vector<Box> holes_;
  Box buttons_[3];
  bool has_buttons_[3] = {false, false, false};

  // The caption button the mouse is over, and the one it pressed.
  Button hovered_ = kNoButton;
  Button pressed_ = kNoButton;
  bool tracking_mouse_ = false;

  // What the app was last told about the window.
  bool maximized_ = false;
  bool active_ = false;

  // window handle for top level window.
  HWND window_handle_ = nullptr;

  // window handle for hosted content.
  HWND child_content_ = nullptr;
};

#endif  // RUNNER_WIN32_WINDOW_H_
