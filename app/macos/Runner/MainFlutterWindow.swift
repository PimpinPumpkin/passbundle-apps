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

  /// Where macOS keeps the window's frame between launches.
  private static let frameName = "PassBundle"

  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    self.contentViewController = flutterViewController
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

    super.awakeFromNib()
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
