import AuthenticationServices
import Flutter
import LocalAuthentication
import Security
import UIKit
import UniformTypeIdentifiers

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    Snapshot.cover()
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    // The core turns biometric unlock off when signing out: Swift deletes the Keychain item.
    pb_set_remove_biometric { Biometric.remove() }
    guard let registrar = engineBridge.pluginRegistry.registrar(forPlugin: "PassBundlePlatform") else { return }
    // What only the phone's own code does (docs/phones.md, "Talking to the OS").
    let channel = FlutterMethodChannel(name: "passbundle/platform", binaryMessenger: registrar.messenger())
    channel.setMethodCallHandler { call, result in
      switch call.method {
      // Where the core keeps the vault: the App Group's folder, which the AutoFill extension reads too.
      case "filesDir":
        result(Shared.filesDir())
      // The server an organization's MDM set (managed app configuration's
      // `server_url`, as on Android), if any.
      case "managedServer":
        let managed = UserDefaults.standard.dictionary(forKey: "com.apple.configuration.managed")
        let server = (managed?["server_url"] as? String)?.trimmingCharacters(in: .whitespaces)
        result(server?.isEmpty == false ? server : nil)
      // Copying straight from the core to the pasteboard: the value never passes through Dart.
      case "copy":
        let arguments = call.arguments as? [String: Any]
        guard let id = arguments?["id"] as? String, let what = arguments?["what"] as? String else {
          result(false)
          return
        }
        result(Platform.copy(id: id, what: what))
      // Unlocking with Face ID or Touch ID (Biometric, below).
      case "biometricStatus":
        result(["status": Biometric.status(), "method": Shared.biometricMethod()])
      case "biometricEnroll":
        Biometric.enroll { on, error in
          result(["ok": error == nil, "on": on, "error": error as Any])
        }
      case "biometricUnlock":
        Shared.unlockWithBiometric { unlocked, error in
          result(["ok": error == nil, "unlocked": unlocked, "error": error as Any])
        }
      // Tests only: debug builds' stand-in for the Face ID prompt (false in a release build).
      case "biometricStandIn":
        let on = (call.arguments as? [String: Any])?["on"] as? Bool ?? false
        result(Biometric.useStandIn(on))
      case "biometricRemove":
        Biometric.remove()
        result(["ok": true])
      // What iOS may suggest above the keyboard, after each sync (Suggestions, below).
      case "suggest":
        Suggestions.update { count, stored in result(["count": count, "stored": stored]) }
      case "forgetSuggestions":
        Suggestions.forget()
        result(true)
      default:
        result(FlutterMethodNotImplemented)
      }
    }
    // The phone locking: iOS's notice that protected data is about to become
    // unavailable. The app locks, if the person chose that (main.dart).
    let computer = FlutterMethodChannel(name: "passbundle/computer", binaryMessenger: registrar.messenger())
    phoneLocks = NotificationCenter.default.addObserver(
      forName: UIApplication.protectedDataWillBecomeUnavailableNotification, object: nil, queue: .main
    ) { _ in
      computer.invokeMethod("sleeping", arguments: nil)
    }
  }

  private var phoneLocks: NSObjectProtocol?
}

/// The iOS app's own piece (docs/phones.md): the pasteboard.
enum Platform {
  /// Puts an item's value on the pasteboard for this device only (not
  /// Universal Clipboard). Secrets expire after 30 seconds, which iOS
  /// enforces even if the app is suspended; a username stays.
  static func copy(id: String, what: String) -> Bool {
    guard let value = Shared.secret(id, what) else { return false }
    var options: [UIPasteboard.OptionsKey: Any] = [.localOnly: true]
    if what != "username" {
      options[.expirationDate] = Date().addingTimeInterval(30)
    }
    UIPasteboard.general.setItems([[UTType.utf8PlainText.identifier: value]], options: options)
    return true
  }
}

/// Unlocking with Face ID or Touch ID (docs/phones.md, "Unlocking with
/// biometrics"). The user key is kept in the Keychain for this device only,
/// readable only after the person's current Face ID or Touch ID: adding a
/// face or a finger voids it. It goes between the core and the Keychain in
/// Swift, never through Dart; the AutoFill extension reads it too (Shared).
///
/// For tests, debug builds have a stand-in for the prompt that says yes,
/// keeping the key without biometrics, as on Android; release builds refuse
/// it. Real prompts are checked by hand on real phones.
enum Biometric {
  /// Kept in the app's own defaults, not the vault's folder, so a test that
  /// clears the folder keeps it.
  private static let standInKey = "biometricStandIn"

  private static var standIn: Bool {
    #if DEBUG
      return UserDefaults.standard.bool(forKey: standInKey)
    #else
      return false
    #endif
  }

  /// Tests only: the stand-in, kept across the app's launches. False in a release build.
  static func useStandIn(_ on: Bool) -> Bool {
    #if DEBUG
      remove()
      UserDefaults.standard.set(on, forKey: standInKey)
      return true
    #else
      return false
    #endif
  }

  /// "setUp", "notSetUp", or "unavailable" (no Face ID or Touch ID set up on this phone).
  static func status() -> String {
    if !standIn && !LAContext().canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil) {
      return "unavailable"
    }
    return Shared.biometricKept() ? "setUp" : "notSetUp"
  }

  /// Turns it off: the Keychain item goes. The core calls this when signing out.
  static func remove() {
    Shared.removeBiometric()
  }

  /// Turns it on, while unlocked: asks for Face ID or Touch ID, then keeps
  /// the user key. `done(on, error)`: on is false if the person cancelled.
  static func enroll(_ done: @escaping (Bool, String?) -> Void) {
    remove()
    let method = Shared.biometricMethod()
    let keep = {
      var length = 0
      guard let bytes = pb_user_key(&length) else { return done(false, "Unlock PassBundle first.") }
      defer { pb_free_secret(bytes, length) }
      var query = Shared.biometricItem
      // Not copied: the Keychain takes it from the core's buffer, which is wiped after.
      query[kSecValueData as String] = Data(bytesNoCopy: bytes, count: length, deallocator: .none)
      if standIn {
        query[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
      } else {
        guard
          let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, .biometryCurrentSet, nil)
        else { return done(false, "Couldn't set up unlocking with \(method).") }
        query[kSecAttrAccessControl as String] = access
      }
      guard SecItemAdd(query as CFDictionary, nil) == errSecSuccess else {
        return done(false, "Couldn't set up unlocking with \(method).")
      }
      done(true, nil)
    }
    if standIn { return keep() }
    LAContext().evaluatePolicy(
      .deviceOwnerAuthenticationWithBiometrics, localizedReason: "Turn on unlocking PassBundle with \(method)"
    ) { agreed, error in
      DispatchQueue.main.async {
        if agreed { return keep() }
        let cancelling: [LAError.Code] = [.userCancel, .appCancel, .systemCancel]
        let cancelled = (error as? LAError).map { cancelling.contains($0.code) } ?? false
        done(false, cancelled ? nil : "Couldn't set up unlocking with \(method).")
      }
    }
  }
}

/// What iOS suggests above the keyboard (docs/phones.md, "Autofill"): after
/// each sync the app gives iOS each login's username and the domains it
/// fills on, never a password. Picking one opens the AutoFill extension.
enum Suggestions {
  /// Gives iOS the unlocked vault's suggestions: `done(count, stored)`, on
  /// the main queue once iOS has them (stored is false while PassBundle is
  /// off for AutoFill in Settings).
  static func update(_ done: @escaping (Int, Bool) -> Void) {
    let found = (Shared.answer(pb_suggestions()) as? [[String: Any]]) ?? []
    let identities = found.compactMap { s -> ASPasswordCredentialIdentity? in
      guard let id = s["id"] as? String, let user = s["username"] as? String,
        let domain = s["domain"] as? String
      else { return nil }
      return ASPasswordCredentialIdentity(
        serviceIdentifier: ASCredentialServiceIdentifier(identifier: domain, type: .domain),
        user: user, recordIdentifier: id)
    }
    let store = ASCredentialIdentityStore.shared
    let answer = { (stored: Bool) in DispatchQueue.main.async { done(identities.count, stored) } }
    // Only while PassBundle is turned on for AutoFill in Settings.
    store.getState { state in
      guard state.isEnabled else { return answer(false) }
      store.replaceCredentialIdentities(with: identities) { stored, _ in answer(stored) }
    }
  }

  /// Signing out: iOS forgets them.
  static func forget() {
    ASCredentialIdentityStore.shared.removeAllCredentialIdentities { _, _ in }
  }
}

/// The vault stays out of the app switcher's snapshot (docs/phones.md,
/// "Locking"): a plain cover over the window while the app isn't active.
/// Scene notifications, since Flutter's scene delegate handles the scene's
/// own calls.
enum Snapshot {
  private static var covers: [ObjectIdentifier: UIView] = [:]

  static func cover() {
    let center = NotificationCenter.default
    center.addObserver(forName: UIScene.willDeactivateNotification, object: nil, queue: .main) { note in
      guard let scene = note.object as? UIWindowScene, let window = scene.windows.first,
        covers[ObjectIdentifier(scene)] == nil
      else { return }
      let view = UIView(frame: window.bounds)
      view.backgroundColor = .systemBackground
      view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
      window.addSubview(view)
      covers[ObjectIdentifier(scene)] = view
    }
    center.addObserver(forName: UIScene.didActivateNotification, object: nil, queue: .main) { note in
      guard let scene = note.object as? UIScene else { return }
      covers.removeValue(forKey: ObjectIdentifier(scene))?.removeFromSuperview()
    }
  }
}
