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

  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

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

  private func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult, updates: FlutterMethodChannel) {
    let arguments = call.arguments as? [String: Any]
    let updater = self.updater
    switch call.method {
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
