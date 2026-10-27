// Reads and drives a running app's window on a Mac, for
// tools/mac-window-test.sh and tools/window-shots.sh: through the
// Accessibility API (the window, its traffic lights, pressing them, moving
// and sizing it) and the mouse (CGEvent), as a person would. Coordinates
// are points from the top left of the main screen. Needs the Accessibility
// permission, which CI's Mac runners give the processes they run.
//
//   swiftc -O tools/mac-window.swift -o mac-window
//   mac-window window <pid>                  the window: id, frame, minimized, full screen, its lights
//   mac-window press <pid> close|minimize|zoom|fullscreen
//   mac-window set <pid> frame <x> <y> <width> <height>
//   mac-window set <pid> minimized|fullscreen true|false
//   mac-window click <x> <y> [clicks]        a left click, or a double click (2)
//   mac-window drag <x> <y> <to x> <to y>    pressed, moved, and let go
//   mac-window screen                        the main screen's visible frame
import AppKit
import ApplicationServices
import Foundation

func fail(_ message: String) -> Never {
  FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
  exit(1)
}

func attribute<T>(_ element: AXUIElement, _ name: String) -> T? {
  var value: CFTypeRef?
  guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
  return value as? T
}

func point(_ element: AXUIElement, _ name: String) -> CGPoint? {
  guard let value: AXValue = attribute(element, name) else { return nil }
  var p = CGPoint.zero
  return AXValueGetValue(value, .cgPoint, &p) ? p : nil
}

func size(_ element: AXUIElement, _ name: String) -> CGSize? {
  guard let value: AXValue = attribute(element, name) else { return nil }
  var s = CGSize.zero
  return AXValueGetValue(value, .cgSize, &s) ? s : nil
}

func frame(_ element: AXUIElement) -> [String: Double]? {
  guard let p = point(element, kAXPositionAttribute), let s = size(element, kAXSizeAttribute) else { return nil }
  return ["x": p.x, "y": p.y, "width": s.width, "height": s.height]
}

/// The app's main window (its first standard one).
func window(_ pid: pid_t) -> AXUIElement {
  guard AXIsProcessTrusted() else {
    fail("This process can't use the Accessibility API (System Settings, Privacy & Security, Accessibility).")
  }
  let app = AXUIElementCreateApplication(pid)
  for _ in 0..<40 {
    if let windows: [AXUIElement] = attribute(app, kAXWindowsAttribute),
      let first = windows.first(where: {
        (attribute($0, kAXSubroleAttribute) as String?) == kAXStandardWindowSubrole
      }) ?? windows.first
    {
      return first
    }
    usleep(250_000)
  }
  fail("No window for \(pid).")
}

/// The window's id, for screencapture -l.
func windowNumber(_ pid: pid_t, _ bounds: [String: Double]) -> Int? {
  guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] else {
    return nil
  }
  let mine = list.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }
  return (mine.first?[kCGWindowNumber as String] as? Int)
}

func button(_ window: AXUIElement, _ name: String) -> AXUIElement {
  let attributeName: String
  switch name {
  case "close": attributeName = kAXCloseButtonAttribute
  case "minimize": attributeName = kAXMinimizeButtonAttribute
  case "zoom", "fullscreen": attributeName = kAXZoomButtonAttribute
  default: fail("No such button: \(name)")
  }
  guard let b: AXUIElement = attribute(window, attributeName) else { fail("The window has no \(name) button.") }
  return b
}

func json(_ value: Any) -> String {
  let data = try! JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
  return String(data: data, encoding: .utf8)!
}

func post(_ type: CGEventType, _ at: CGPoint, clicks: Int64 = 1, flags: CGEventFlags = []) {
  guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: at, mouseButton: .left) else {
    fail("Couldn't make a mouse event.")
  }
  event.setIntegerValueField(.mouseEventClickState, value: clicks)
  event.flags = flags
  event.post(tap: .cghidEventTap)
  usleep(30_000)
}

let args = CommandLine.arguments
guard args.count >= 2 else { fail("usage: see the top of mac-window.swift") }
func number(_ i: Int) -> Double {
  guard args.count > i, let n = Double(args[i]) else { fail("Missing a number at \(i).") }
  return n
}

switch args[1] {
case "window":
  let pid = pid_t(number(2))
  let w = window(pid)
  guard let bounds = frame(w) else { fail("The window has no frame.") }
  var lights: [String: Any] = [:]
  for name in ["close", "minimize", "zoom"] {
    let b = button(w, name)
    lights[name] = frame(b) ?? [:]
  }
  print(
    json([
      "id": windowNumber(pid, bounds) ?? 0,
      "frame": bounds,
      "minimized": (attribute(w, kAXMinimizedAttribute) as Bool?) ?? false,
      "fullScreen": (attribute(w, "AXFullScreen") as Bool?) ?? false,
      "lights": lights,
    ]))
case "press":
  let w = window(pid_t(number(2)))
  let name = args.count > 3 ? args[3] : ""
  let b = button(w, name)
  if name == "zoom" {
    // A zoom, not full screen: Option and the green button.
    guard let at = frame(b) else { fail("The zoom button has no frame.") }
    let middle = CGPoint(x: at["x"]! + at["width"]! / 2, y: at["y"]! + at["height"]! / 2)
    post(.mouseMoved, middle)
    post(.leftMouseDown, middle, flags: .maskAlternate)
    post(.leftMouseUp, middle, flags: .maskAlternate)
  } else {
    let result = AXUIElementPerformAction(b, kAXPressAction as CFString)
    guard result == .success else { fail("Pressing \(name) failed: \(result.rawValue)") }
  }
case "set":
  let w = window(pid_t(number(2)))
  let what = args.count > 3 ? args[3] : ""
  switch what {
  case "frame":
    var p = CGPoint(x: number(4), y: number(5))
    var s = CGSize(width: number(6), height: number(7))
    AXUIElementSetAttributeValue(w, kAXPositionAttribute as CFString, AXValueCreate(.cgPoint, &p)!)
    AXUIElementSetAttributeValue(w, kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &s)!)
  case "minimized", "fullscreen":
    let on = (args.count > 4 ? args[4] : "") == "true"
    let name = what == "minimized" ? kAXMinimizedAttribute : "AXFullScreen"
    let result = AXUIElementSetAttributeValue(w, name as CFString, (on ? kCFBooleanTrue : kCFBooleanFalse)!)
    guard result == .success else { fail("Setting \(what) failed: \(result.rawValue)") }
  default:
    fail("Can't set \(what).")
  }
case "click":
  let at = CGPoint(x: number(2), y: number(3))
  let clicks = args.count > 4 ? Int64(number(4)) : 1
  post(.mouseMoved, at)
  for click in 1...clicks {
    post(.leftMouseDown, at, clicks: click)
    post(.leftMouseUp, at, clicks: click)
  }
case "drag":
  let from = CGPoint(x: number(2), y: number(3))
  let to = CGPoint(x: number(4), y: number(5))
  post(.mouseMoved, from)
  post(.leftMouseDown, from)
  for step in 1...10 {
    let f = Double(step) / 10
    post(.leftMouseDragged, CGPoint(x: from.x + (to.x - from.x) * f, y: from.y + (to.y - from.y) * f))
  }
  post(.leftMouseUp, to)
case "screen":
  guard let screen = NSScreen.main else { fail("No screen.") }
  let full = screen.frame
  let visible = screen.visibleFrame
  // In points from the top left, as the rest.
  print(
    json([
      "x": visible.minX, "y": full.maxY - visible.maxY, "width": visible.width, "height": visible.height,
    ]))
default:
  fail("No such command: \(args[1])")
}
