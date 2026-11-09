import UIKit

/// A bare app that hosts the AutoFill extension's tests (AutoFillTests.swift),
/// so the extension's screens have a window to draw in on the simulator. It
/// has none of PassBundle in it, and is never shipped.
@main
final class TestHost: UIResponder, UIApplicationDelegate {
  var window: UIWindow?

  func application(
    _ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    let window = UIWindow(frame: UIScreen.main.bounds)
    window.rootViewController = UIViewController()
    window.makeKeyAndVisible()
    self.window = window
    return true
  }
}
