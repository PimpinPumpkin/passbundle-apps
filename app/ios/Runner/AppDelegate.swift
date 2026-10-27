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
      // Where the core keeps the vault: the app's own Application Support folder.
      case "filesDir":
        result(Platform.filesDir())
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
      // A login's website, in the browser: only http and https.
      case "openUrl":
        guard let text = (call.arguments as? [String: Any])?["url"] as? String, let url = URL(string: text),
          url.scheme == "https" || url.scheme == "http"
        else {
          result(false)
          return
        }
        UIApplication.shared.open(url) { opened in result(opened) }
      // Unlocking with Face ID or Touch ID (Biometric, below).
      case "biometricStatus":
        result(["status": Biometric.status(), "method": Biometric.method()])
      case "biometricEnroll":
        Biometric.enroll { on, error in
          result(["ok": error == nil, "on": on, "error": error as Any])
        }
      case "biometricUnlock":
        Biometric.unlock { unlocked, error in
          result(["ok": error == nil, "unlocked": unlocked, "error": error as Any])
        }
      // Confirming it's the person, to approve a sign-in from another device.
      case "biometricConfirmApproval":
        Biometric.unlock(reason: "Approve sign-in", forApproval: true) { confirmed, error in
          result(["ok": error == nil, "confirmed": confirmed, "error": error as Any])
        }
      // Tests only: debug builds' stand-in for the Face ID prompt (false in a release build).
      case "biometricStandIn":
        let on = (call.arguments as? [String: Any])?["on"] as? Bool ?? false
        result(Biometric.useStandIn(on))
      case "biometricRemove":
        Biometric.remove()
        result(["ok": true])
      // An attachment, into the app's Documents folder, which Files shows
      // (On My iPhone, PassBundle). The core writes it there itself.
      case "saveAttachment":
        let arguments = call.arguments as? [String: Any]
        guard let id = arguments?["id"] as? String, let attachment = arguments?["attachmentId"] as? String,
          let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first
        else {
          result(["ok": false, "error": "Couldn't save that."])
          return
        }
        DispatchQueue.global(qos: .userInitiated).async {
          var answer: [String: Any] = ["ok": false, "error": "Couldn't save that."]
          if let text = pb_save_attachment_in(id, attachment, documents.path) {
            defer { pb_free(text) }
            if let parsed = try? JSONSerialization.jsonObject(with: Data(bytes: text, count: strlen(text)))
              as? [String: Any]
            {
              answer = parsed
              if let path = parsed["path"] as? String {
                answer["path"] = "Files, On My iPhone, PassBundle, " + (path as NSString).lastPathComponent
              }
            }
          }
          DispatchQueue.main.async { result(answer) }
        }
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

/// The iOS app's own piece (docs/phones.md): the data folder and the pasteboard.
enum Platform {
  static func filesDir() -> String? {
    guard let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else {
      return nil
    }
    let dir = support.appendingPathComponent("PassBundle", isDirectory: true)
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return dir.path
  }

  /// Puts an item's value on the pasteboard for this device only (not
  /// Universal Clipboard). Secrets expire after 30 seconds, which iOS
  /// enforces even if the app is suspended; a username stays.
  static func copy(id: String, what: String) -> Bool {
    var length = 0
    guard let bytes = pb_secret(id, what, &length) else { return false }
    defer { pb_free_secret(bytes, length) }
    guard let value = String(bytes: UnsafeBufferPointer(start: bytes, count: length), encoding: .utf8) else {
      return false
    }
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
/// face or a finger voids it. It goes between the core and the Keychain here,
/// never through Dart.
///
/// For tests, debug builds have a stand-in for the prompt that says yes,
/// keeping the key without biometrics, as on Android; release builds refuse
/// it. Real prompts are checked by hand on real phones.
enum Biometric {
  private static let service = "com.passbundle.app.biometric"
  private static let account = "user-key"

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

  /// "Face ID" or "Touch ID": what the screens call it.
  static func method() -> String {
    let context = LAContext()
    _ = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
    switch context.biometryType {
    case .touchID: return "Touch ID"
    case .opticID: return "Optic ID"
    default: return "Face ID"
    }
  }

  private static var item: [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
    ]
  }

  /// Whether the key is kept, without asking for Face ID.
  private static func kept() -> Bool {
    let context = LAContext()
    context.interactionNotAllowed = true
    var query = item
    query[kSecUseAuthenticationContext as String] = context
    query[kSecReturnAttributes as String] = true
    let status = SecItemCopyMatching(query as CFDictionary, nil)
    return status == errSecSuccess || status == errSecInteractionNotAllowed
  }

  /// "setUp", "notSetUp", or "unavailable" (no Face ID or Touch ID set up on this phone).
  static func status() -> String {
    if !standIn && !LAContext().canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil) {
      return "unavailable"
    }
    return kept() ? "setUp" : "notSetUp"
  }

  /// Turns it off: the Keychain item goes. The core calls this when signing out.
  static func remove() {
    SecItemDelete(item as CFDictionary)
  }

  /// Turns it on, while unlocked: asks for Face ID or Touch ID, then keeps
  /// the user key. `done(on, error)`: on is false if the person cancelled.
  static func enroll(_ done: @escaping (Bool, String?) -> Void) {
    remove()
    let keep = {
      var length = 0
      guard let bytes = pb_user_key(&length) else { return done(false, "Unlock PassBundle first.") }
      defer { pb_free_secret(bytes, length) }
      var query = item
      // Not copied: the Keychain takes it from the core's buffer, which is wiped after.
      query[kSecValueData as String] = Data(bytesNoCopy: bytes, count: length, deallocator: .none)
      if standIn {
        query[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
      } else {
        guard
          let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, .biometryCurrentSet, nil)
        else { return done(false, "Couldn't set up unlocking with \(method()).") }
        query[kSecAttrAccessControl as String] = access
      }
      guard SecItemAdd(query as CFDictionary, nil) == errSecSuccess else {
        return done(false, "Couldn't set up unlocking with \(method()).")
      }
      done(true, nil)
    }
    if standIn { return keep() }
    LAContext().evaluatePolicy(
      .deviceOwnerAuthenticationWithBiometrics, localizedReason: "Turn on unlocking PassBundle with \(method())"
    ) { agreed, error in
      DispatchQueue.main.async {
        if agreed { return keep() }
        let cancelling: [LAError.Code] = [.userCancel, .appCancel, .systemCancel]
        let cancelled = (error as? LAError).map { cancelling.contains($0.code) } ?? false
        done(false, cancelled ? nil : "Couldn't set up unlocking with \(method()).")
      }
    }
  }

  /// Unlocks: reading the key asks for Face ID or Touch ID, then the core
  /// opens the kept vault with it. `done(unlocked, error)`: unlocked is false
  /// if the person cancelled. With `forApproval`, it's the person confirming
  /// it's them to approve a sign-in from another device, and nothing else:
  /// the core only compares the key with the open vault's
  /// (`pb_confirm_approval_with_key`), and the prompt says `reason`.
  static func unlock(
    reason: String = "Unlock PassBundle", forApproval: Bool = false, _ done: @escaping (Bool, String?) -> Void
  ) {
    DispatchQueue.global(qos: .userInitiated).async {
      let context = LAContext()
      context.localizedReason = reason
      var query = item
      query[kSecUseAuthenticationContext as String] = context
      query[kSecReturnData as String] = true
      var found: CFTypeRef?
      let status = SecItemCopyMatching(query as CFDictionary, &found)
      DispatchQueue.main.async {
        switch status {
        case errSecSuccess:
          guard var key = found as? Data else { return done(false, "Couldn't unlock with \(method()).") }
          let why = key.withUnsafeBytes { raw -> String in
            let bytes = raw.bindMemory(to: UInt8.self).baseAddress
            let given = forApproval ? pb_confirm_approval_with_key(bytes, raw.count) : pb_unlock_with_key(bytes, raw.count)
            guard let answer = given else {
              return "Couldn't unlock with \(method())."
            }
            defer { pb_free(answer) }
            return String(cString: answer)
          }
          key.resetBytes(in: 0..<key.count)
          done(why.isEmpty, why.isEmpty ? nil : why)
        case errSecUserCanceled:
          done(false, nil)
        case errSecItemNotFound:
          // Off, or voided by a face or finger added since.
          remove()
          done(false, "Unlocking with \(method()) is off. Use your master password.")
        default:
          done(false, "Couldn't unlock with \(method()).")
        }
      }
    }
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
