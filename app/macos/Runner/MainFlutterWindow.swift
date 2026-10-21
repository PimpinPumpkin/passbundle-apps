import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  /// Tells the app the computer is about to sleep, so it can lock first
  /// (docs/core.md, "Locking by itself").
  private var computer: FlutterMethodChannel?
  private var sleepObserver: NSObjectProtocol?

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

    super.awakeFromNib()
  }
}
