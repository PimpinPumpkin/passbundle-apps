import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  /// Tells the app the computer is about to sleep, so it can lock first
  /// (docs/core.md, "Locking by itself").
  private var computer: FlutterMethodChannel?
  private var sleepObserver: NSObjectProtocol?
  /// Updates from PassBundle's GitHub releases (Updater.swift), for Settings
  /// (app/lib/updates.dart): calls on passbundle/platform, and how a
  /// download goes on passbundle/updates.
  private let updater = Updater.forThisApp()
  private var platform: FlutterMethodChannel?
  private var updates: FlutterMethodChannel?
  /// The window, and the app's top edge in it (app/lib/window_chrome.dart):
  /// the app's look (SystemBars in app/lib/theme.dart), and where its band
  /// is, what in it takes clicks, and where the traffic lights go; full
  /// screen, the other way.
  private var chrome: FlutterMethodChannel?

  /// The app's band along the window's top, and what in it takes clicks,
  /// in points from the window's top left: the rest of the band moves the
  /// window, as a title bar does.
  private var band = NSRect.zero
  private var holes: [NSRect] = []

  /// Where the traffic lights go, from the window's top left: the close
  /// button's left edge, their middle, and the height of the band they sit
  /// in. Until the app says, the strip it draws along the top while
  /// signing in or unlocking.
  private var lights = (x: CGFloat(12), middle: CGFloat(16), height: CGFloat(32))
  private var placingLights = false
  private var observers: [NSObjectProtocol] = []

  /// Where macOS keeps the window's frame between launches.
  private static let frameName = "PassBundle"

  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    self.contentViewController = flutterViewController
    // The app draws the window's top (app/lib/window_chrome.dart): its
    // content goes up under a clear title bar with no title, and the
    // traffic lights go where the app leaves them room.
    titleVisibility = .hidden
    titlebarAppearsTransparent = true
    titlebarSeparatorStyle = .none
    styleMask.insert(.fullSizeContentView)
    // Wide enough for the menu, the list, and an item side by side (the
    // app's wide layout starts at 900 points), within the screen; after
    // that, where and how big the person left it.
    self.contentMinSize = NSSize(width: 400, height: 560)
    if !self.setFrameUsingName(Self.frameName) {
      self.setFrame(self.openingFrame(), display: true)
    }
    _ = self.setFrameAutosaveName(Self.frameName)

    RegisterGeneratedPlugins(registry: flutterViewController)

    let computer = FlutterMethodChannel(
      name: "passbundle/computer",
      binaryMessenger: flutterViewController.engine.binaryMessenger)
    sleepObserver = NSWorkspace.shared.notificationCenter.addObserver(
      forName: NSWorkspace.willSleepNotification, object: nil, queue: .main
    ) { _ in
      computer.invokeMethod("sleeping", arguments: nil)
    }
    self.computer = computer

    let messenger = flutterViewController.engine.binaryMessenger
    let updates = FlutterMethodChannel(name: "passbundle/updates", binaryMessenger: messenger)
    updater.progress = { percent in
      DispatchQueue.main.async { updates.invokeMethod("progress", arguments: ["percent": percent]) }
    }
    let platform = FlutterMethodChannel(name: "passbundle/platform", binaryMessenger: messenger)
    platform.setMethodCallHandler { [weak self] call, result in
      self?.handle(call, result: result, updates: updates)
    }
    self.platform = platform
    self.updates = updates

    let chrome = FlutterMethodChannel(name: "passbundle/window", binaryMessenger: messenger)
    chrome.setMethodCallHandler { [weak self] call, result in
      guard let self else {
        result(FlutterMethodNotImplemented)
        return
      }
      self.handleChrome(call, result: result)
    }
    self.chrome = chrome
    watchTheTitleBar()

    super.awakeFromNib()
  }

  private func handleChrome(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    let arguments = call.arguments as? [String: Any]
    switch call.method {
    case "frame":
      result(frameForTheApp())
    case "look":
      guard let dark = arguments?["dark"] as? Bool, let ground = arguments?["ground"] as? Int else {
        result(FlutterError(code: "arguments", message: "look takes dark and ground", details: nil))
        return
      }
      show(dark: dark, ground: ground)
      result(nil)
    case "chrome":
      band = Self.rect(arguments?["band"]) ?? .zero
      holes = (arguments?["holes"] as? [Any] ?? []).compactMap { Self.rect($0) }
      if let at = arguments?["lights"] as? [NSNumber], at.count == 3 {
        lights = (CGFloat(at[0].doubleValue), CGFloat(at[1].doubleValue), CGFloat(at[2].doubleValue))
        placeLights()
      }
      result(nil)
    // The buttons are the traffic lights, which are AppKit's; as they do.
    case "minimize":
      performMiniaturize(nil)
      result(nil)
    case "maximize":
      performZoom(nil)
      result(nil)
    case "close":
      performClose(nil)
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  /// [left, top, right, bottom], from the window's top left.
  private static func rect(_ value: Any?) -> NSRect? {
    guard let edges = value as? [NSNumber], edges.count == 4 else { return nil }
    let e = edges.map { CGFloat($0.doubleValue) }
    return NSRect(x: e[0], y: e[1], width: e[2] - e[0], height: e[3] - e[1])
  }

  private func frameForTheApp(fullScreen: Bool? = nil) -> [String: Any] {
    [
      "style": "mac",
      "fullScreen": fullScreen ?? styleMask.contains(.fullScreen),
      "maximized": isZoomed,
      "active": isKeyWindow,
    ]
  }

  /// What's left of the frame as the app is, light or dark, whatever the
  /// Mac's own setting (the traffic lights' look), and the app's ground
  /// (0xRRGGBB) behind it, where the window shows before the app draws.
  private func show(dark: Bool, ground: Int) {
    appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
    backgroundColor = NSColor(
      srgbRed: CGFloat((ground >> 16) & 0xff) / 255, green: CGFloat((ground >> 8) & 0xff) / 255,
      blue: CGFloat(ground & 0xff) / 255, alpha: 1)
  }

  /// AppKit lays the title bar out again as the window changes (its size,
  /// the screen, full screen, becoming key), and puts the traffic lights
  /// back where a title bar has them: they go back where the app has room
  /// for them each time, before anything's drawn.
  private func watchTheTitleBar() {
    let center = NotificationCenter.default
    for name in [
      NSWindow.didResizeNotification, NSWindow.didEndLiveResizeNotification,
      NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification,
      NSWindow.didChangeBackingPropertiesNotification, NSWindow.didChangeScreenNotification,
    ] {
      observers.append(center.addObserver(forName: name, object: self, queue: .main) { [weak self] _ in
        self?.placeLights()
      })
    }
    // As AppKit moves them, or their title bar: at once, so the lights are
    // never drawn where AppKit had them.
    observers.append(center.addObserver(forName: NSView.frameDidChangeNotification, object: nil, queue: nil) {
      [weak self] note in
      guard let self, let view = note.object as? NSView, view.window === self,
        self.lightViews().contains(where: { $0 === view })
      else { return }
      self.placeLights()
    })
    // Full screen hides the traffic lights: the app drops their room as it
    // goes in, and they come back where they were as it comes out.
    observers.append(center.addObserver(forName: NSWindow.willEnterFullScreenNotification, object: self, queue: .main) {
      [weak self] _ in
      self?.tellTheApp(fullScreen: true)
      // And as it is once it's had time to get there, in case it didn't.
      DispatchQueue.main.asyncAfter(deadline: .now() + 3) { self?.tellTheApp() }
    })
    observers.append(center.addObserver(forName: NSWindow.didEnterFullScreenNotification, object: self, queue: .main) {
      [weak self] _ in self?.tellTheApp()
    })
    observers.append(center.addObserver(forName: NSWindow.didExitFullScreenNotification, object: self, queue: .main) {
      [weak self] _ in
      self?.placeLights()
      self?.tellTheApp(fullScreen: false)
    })
    for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification] {
      observers.append(center.addObserver(forName: name, object: self, queue: .main) { [weak self] _ in
        self?.tellTheApp()
      })
    }
    placeLights()
  }

  private func tellTheApp(fullScreen: Bool? = nil) {
    chrome?.invokeMethod("frame", arguments: frameForTheApp(fullScreen: fullScreen))
  }

  private func lightButtons() -> [NSButton] {
    [.closeButton, .miniaturizeButton, .zoomButton].compactMap { standardWindowButton($0) }
  }

  /// The traffic lights, their title bar, and its container.
  private func lightViews() -> [NSView] {
    var views: [NSView] = lightButtons()
    if let titlebar = views.first?.superview {
      views.append(titlebar)
      if let container = titlebar.superview { views.append(container) }
    }
    return views
  }

  /// The traffic lights where the app has room for them (`lights`): their
  /// container as tall as the app's band, and the lights in it, at their
  /// own spacing.
  private func placeLights() {
    let buttons = lightButtons()
    guard !placingLights, !styleMask.contains(.fullScreen), buttons.count == 3,
      let titlebar = buttons[0].superview, let container = titlebar.superview,
      let frameView = container.superview
    else { return }
    placingLights = true
    defer { placingLights = false }
    for view in [container, titlebar] + (buttons as [NSView]) where !view.postsFrameChangedNotifications {
      view.postsFrameChangedNotifications = true
    }
    var frame = container.frame
    frame.size.height = lights.height
    frame.origin.y = frameView.bounds.height - lights.height
    if container.frame != frame {
      container.frame = frame
    }
    let pitch = buttons[1].frame.minX - buttons[0].frame.minX
    let windowHeight = frameView.bounds.height
    for (i, button) in buttons.enumerated() {
      let middle = titlebar.convert(
        NSPoint(x: lights.x + CGFloat(i) * (pitch > 0 ? pitch : 20), y: windowHeight - lights.middle), from: nil)
      let origin = NSPoint(x: middle.x.rounded(), y: (middle.y - button.frame.height / 2).rounded())
      if button.frame.origin != origin {
        button.setFrameOrigin(origin)
      }
    }
    titlebar.updateTrackingAreas()
  }

  /// A press on the app's band where nothing of the app's takes it moves
  /// the window, and a double click there does what System Settings says
  /// (`AppleActionOnDoubleClick`), as a title bar would; anything else is
  /// the app's, or AppKit's (the traffic lights).
  override func sendEvent(_ event: NSEvent) {
    if event.type == .leftMouseDown, movesTheWindow(event) {
      if event.clickCount == 2 {
        doubleClicked()
      } else if event.clickCount == 1 {
        if !event.modifierFlags.contains(.command) {
          if !NSApp.isActive {
            if #available(macOS 14.0, *) {
              NSApp.activate()
            } else {
              NSApp.activate(ignoringOtherApps: true)
            }
          }
          if !isKeyWindow { makeKeyAndOrderFront(nil) }
        }
        performDrag(with: event)
      }
      return
    }
    super.sendEvent(event)
  }

  private func movesTheWindow(_ event: NSEvent) -> Bool {
    guard !band.isEmpty, !styleMask.contains(.fullScreen), let view = contentViewController?.view else {
      return false
    }
    for button in lightButtons() where !button.isHidden {
      if button.convert(button.bounds, to: nil).contains(event.locationInWindow) { return false }
    }
    let local = view.convert(event.locationInWindow, from: nil)
    let point = NSPoint(x: local.x, y: view.isFlipped ? local.y : view.bounds.height - local.y)
    return band.contains(point) && !holes.contains { $0.contains(point) }
  }

  /// As System Settings, Desktop & Dock, says a double click on a window's
  /// title bar does: zoom (or fill, which the window does as zoom),
  /// minimize, or nothing.
  private func doubleClicked() {
    let defaults = UserDefaults.standard
    switch defaults.string(forKey: "AppleActionOnDoubleClick") {
    case "Minimize":
      performMiniaturize(nil)
    case "None", "Do Nothing":
      break
    case .none:
      if defaults.bool(forKey: "AppleMiniaturizeOnDoubleClick") {
        performMiniaturize(nil)
      } else {
        performZoom(nil)
      }
    default:
      performZoom(nil)
    }
  }

  /// 1200 by 800 points, or as much of the screen as there is, centered.
  private func openingFrame() -> NSRect {
    let visible = (self.screen ?? NSScreen.main)?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
    let chrome = self.frameRect(forContentRect: NSRect(x: 0, y: 0, width: 100, height: 100)).size
    let content = NSSize(
      width: max(self.contentMinSize.width, min(1200, visible.width - 40 - (chrome.width - 100))),
      height: max(self.contentMinSize.height, min(800, visible.height - 40 - (chrome.height - 100))))
    let size = self.frameRect(forContentRect: NSRect(origin: .zero, size: content)).size
    return NSRect(
      x: visible.midX - size.width / 2, y: visible.midY - size.height / 2,
      width: size.width, height: size.height)
  }

  private func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult, updates: FlutterMethodChannel) {
    let arguments = call.arguments as? [String: Any]
    let updater = self.updater
    switch call.method {
    // A login's website, in the browser: only http and https.
    case "openUrl":
      guard let text = arguments?["url"] as? String, let url = URL(string: text),
        url.scheme == "https" || url.scheme == "http"
      else {
        result(false)
        return
      }
      result(NSWorkspace.shared.open(url))
    case "updateStatus":
      result(updater.status())
    case "updateCheck":
      let quiet = arguments?["quiet"] as? Bool ?? false
      DispatchQueue.global(qos: .userInitiated).async {
        let answer = updater.check(quiet: quiet)
        DispatchQueue.main.async { result(answer) }
      }
    case "updateInstall":
      DispatchQueue.global(qos: .userInitiated).async {
        let (answer, replaced) = updater.install()
        DispatchQueue.main.async {
          result(answer)
          if let replaced { MainFlutterWindow.restart(replaced, updates: updates) }
        }
      }
    case "updateAuto":
      updater.setAuto(arguments?["on"] as? Bool ?? true)
      result(true)
    case "updateReveal":
      result(updater.reveal())
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  /// The new version is in place: the old copy's desktop agent stops (the
  /// new PassBundle starts its own), and PassBundle quits and opens again.
  private static func restart(_ replaced: Updater.Replaced, updates: FlutterMethodChannel) {
    for pid in replaced.helpers { kill(pid, SIGTERM) }
    do {
      try Updater.relaunch(replaced.app, after: [getpid()] + replaced.helpers)
    } catch {
      updates.invokeMethod(
        "installed",
        arguments: ["ok": false, "error": "PassBundle is updated. Quit it and open it again to finish."])
      return
    }
    // A moment for Settings to say so.
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { NSApp.terminate(nil) }
  }
}
