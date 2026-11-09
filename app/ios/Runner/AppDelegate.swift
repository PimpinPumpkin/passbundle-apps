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
    // The core turns an account's biometric unlock off when signing out of
    // it: Swift deletes that account's Keychain item.
    pb_set_remove_biometric { account in
      guard let account else { return }
      Biometric.remove(account: String(cString: account))
    }
    // The one item a build from before several accounts kept, for its one
    // account (now carried over as an account of its own): it goes, and the
    // app offers Face ID again after the next unlock with the master password.
    Biometric.forgetEarlier()
    guard let registrar = engineBridge.pluginRegistry.registrar(forPlugin: "PassBundlePlatform") else { return }
    // What only the phone's own code does (docs/phones.md, "Talking to the OS").
    let channel = FlutterMethodChannel(name: "passbundle/platform", binaryMessenger: registrar.messenger())
    channel.setMethodCallHandler { call, result in
      switch call.method {
      // Where the core keeps the vault: the App Group's folder, which the
      // AutoFill extension reads too (Shared.swift).
      case "filesDir":
        result(Shared.filesDir())
      // The server an organization's MDM set (managed app configuration's
      // `server_url`, as on Android), if any.
      case "managedServer":
        let managed = UserDefaults.standard.dictionary(forKey: "com.apple.configuration.managed")
        let server = (managed?["server_url"] as? String)?.trimmingCharacters(in: .whitespaces)
        result(server?.isEmpty == false ? server : nil)
      // Copying straight from the core to the pasteboard: the value never passes through Dart.
      // Each call about an account names it (core.dart, Core.account).
      case "copy":
        let arguments = call.arguments as? [String: Any]
        guard let id = arguments?["id"] as? String, let what = arguments?["what"] as? String else {
          result(false)
          return
        }
        result(Platform.copy(account: arguments?["account"] as? String ?? "", id: id, what: what))
      // A login's website, in the browser: only http and https.
      case "openUrl":
        guard let text = (call.arguments as? [String: Any])?["url"] as? String, let url = URL(string: text),
          url.scheme == "https" || url.scheme == "http"
        else {
          result(false)
          return
        }
        UIApplication.shared.open(url) { opened in result(opened) }
      // Duo's Universal Prompt at sign-in (app/lib/two_step.dart): the link
      // it comes back with, or nil if the person gave up.
      case "duoSignIn":
        DuoSession.shared.start((call.arguments as? [String: Any])?["url"] as? String ?? "") { link in
          result(link)
        }
      case "duoCancel":
        DuoSession.shared.cancel()
        result(nil)
      // Unlocking with Face ID or Touch ID (Biometric, below), each account its own.
      case "biometricStatus":
        result(["status": Biometric.status(account: Platform.account(call)), "method": Shared.biometricMethod()])
      case "biometricEnroll":
        Biometric.enroll(account: Platform.account(call)) { on, error in
          result(["ok": error == nil, "on": on, "error": error as Any])
        }
      case "biometricUnlock":
        Shared.unlockWithBiometric(account: Platform.account(call)) { unlocked, error in
          result(["ok": error == nil, "unlocked": unlocked, "error": error as Any])
        }
      // Confirming it's the person, to approve a sign-in from another device.
      case "biometricConfirmApproval":
        Shared.unlockWithBiometric(account: Platform.account(call), reason: "Approve sign-in", forApproval: true) {
          confirmed, error in
          result(["ok": error == nil, "confirmed": confirmed, "error": error as Any])
        }
      // Tests only: debug builds' stand-in for the Face ID prompt (false in a release build).
      case "biometricStandIn":
        let on = (call.arguments as? [String: Any])?["on"] as? Bool ?? false
        result(Biometric.useStandIn(on))
      case "biometricRemove":
        Biometric.remove(account: Platform.account(call))
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
        let account = Platform.account(call)
        DispatchQueue.global(qos: .userInitiated).async {
          var answer: [String: Any] = ["ok": false, "error": "Couldn't save that."]
          if let text = pb_save_attachment_in(account, id, attachment, documents.path) {
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
      // What iOS may suggest above the keyboard, after each sync (Suggestions, below).
      case "suggest":
        Suggestions.update { count, stored in result(["count": count, "stored": stored]) }
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

/// Duo's Universal Prompt at sign-in (app/lib/two_step.dart), in the
/// system's sign-in sheet (ASWebAuthenticationSession), which hands back the
/// link the server's Duo page opens, `passbundle://duo-callback?...`. So the
/// app needn't take passbundle:// links from anywhere else, and the core
/// checks the link belongs to the sign-in waiting (core/src/two_step.rs).
final class DuoSession: NSObject, ASWebAuthenticationPresentationContextProviding {
  static let shared = DuoSession()
  private var session: ASWebAuthenticationSession?

  func start(_ text: String, done: @escaping (String?) -> Void) {
    guard let url = URL(string: text), url.scheme == "https" else {
      done(nil)
      return
    }
    session?.cancel()
    let session = ASWebAuthenticationSession(url: url, callbackURLScheme: "passbundle") { [weak self] link, _ in
      DispatchQueue.main.async {
        self?.session = nil
        done(link?.absoluteString)
      }
    }
    session.presentationContextProvider = self
    // Nothing kept from Safari, nor shared with it, and no "wants to use" alert.
    session.prefersEphemeralWebBrowserSession = true
    self.session = session
    if !session.start() {
      self.session = nil
      done(nil)
    }
  }

  func cancel() {
    session?.cancel()
  }

  func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
    let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
    if let key = scenes.flatMap(\.windows).first(where: \.isKeyWindow) { return key }
    if let scene = scenes.first { return UIWindow(windowScene: scene) }
    return ASPresentationAnchor()
  }
}

/// The iOS app's own piece (docs/phones.md): the pasteboard. The data
/// folder is the App Group's (Shared.swift).
enum Platform {
  /// The account a call is about: its id on this device (core.dart, Core.account).
  static func account(_ call: FlutterMethodCall) -> String {
    (call.arguments as? [String: Any])?["account"] as? String ?? ""
  }

  /// Puts an item's value on the pasteboard for this device only (not
  /// Universal Clipboard). Secrets expire after 30 seconds, which iOS
  /// enforces even if the app is suspended; a username stays.
  static func copy(account: String, id: String, what: String) -> Bool {
    var length = 0
    let bytes = pb_secret(account, id, what, &length)
    guard let value = Shared.string(bytes, length) else { return false }
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
/// Swift, never through Dart; the AutoFill extension reads it too
/// (Shared.swift, which also unlocks with it).
///
/// Each account on the phone has its own Keychain item, named by the
/// account's id on this device (DESIGN.md in the meta repo, "Several accounts
/// in the apps"): turning one off, or iOS voiding it, leaves the others'.
///
/// For tests, debug builds have a stand-in for the prompt that says yes,
/// keeping the key without biometrics, as on Android; release builds refuse
/// it. Real prompts are checked by hand on real phones.
enum Biometric {
  /// The one item a build from before several accounts kept.
  private static let earlier = "user-key"

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
      removeAll()
      UserDefaults.standard.set(on, forKey: standInKey)
      return true
    #else
      return false
    #endif
  }

  /// For an account: "setUp", "notSetUp", or "unavailable" (no Face ID or Touch ID set up on this phone).
  static func status(account: String) -> String {
    if !standIn && !LAContext().canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil) {
      return "unavailable"
    }
    return Shared.biometricKept(account) ? "setUp" : "notSetUp"
  }

  /// Turns it off for one account: its Keychain item goes, and the others'
  /// stay. The core calls this when signing out of it.
  static func remove(account: String) {
    Shared.removeBiometric(account)
  }

  /// Every account's, for the tests' stand-in.
  private static func removeAll() {
    SecItemDelete(
      [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: Shared.biometricService]
        as CFDictionary)
  }

  /// The item a build from before several accounts kept goes (at start).
  static func forgetEarlier() {
    SecItemDelete(
      [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: Shared.biometricService,
        kSecAttrAccount as String: earlier,
      ] as CFDictionary)
  }

  /// Turns it on for an account, while it's unlocked: asks for Face ID or
  /// Touch ID, then keeps its user key in its own item. `done(on, error)`:
  /// on is false if the person cancelled.
  static func enroll(account: String, _ done: @escaping (Bool, String?) -> Void) {
    let method = Shared.biometricMethod()
    guard Shared.isAccount(account) else { return done(false, "Couldn't set up unlocking with \(method).") }
    remove(account: account)
    let keep = {
      var length = 0
      guard let bytes = pb_user_key(account, &length) else { return done(false, "Unlock PassBundle first.") }
      defer { pb_free_secret(bytes, length) }
      var query = Shared.biometricItem(account)
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
/// fills on, from every unlocked account, never a password. Each names its
/// account and item (`<account>/<item>`), so picking one opens the AutoFill
/// extension on that account's login. A locked account's go once another
/// account's change replaces the list, and come back at its next unlock.
enum Suggestions {
  /// Gives iOS the unlocked accounts' suggestions: `done(count, stored)`, on
  /// the main queue once iOS has them (stored is false while PassBundle is
  /// off for AutoFill in Settings).
  static func update(_ done: @escaping (Int, Bool) -> Void) {
    let found = (Shared.answer(pb_suggestions()) as? [[String: Any]]) ?? []
    let identities: [any ASCredentialIdentity] = found.compactMap { s -> ASPasswordCredentialIdentity? in
      guard let account = s["account"] as? String, let id = s["id"] as? String,
        let user = s["username"] as? String, let domain = s["domain"] as? String
      else { return nil }
      return ASPasswordCredentialIdentity(
        serviceIdentifier: ASCredentialServiceIdentifier(identifier: domain, type: .domain),
        user: user, recordIdentifier: "\(account)/\(id)")
    }
    let store = ASCredentialIdentityStore.shared
    let answer = { (stored: Bool) in DispatchQueue.main.async { done(identities.count, stored) } }
    // Only while PassBundle is turned on for AutoFill in Settings.
    store.getState { state in
      guard state.isEnabled else { return answer(false) }
      store.replaceCredentialIdentities(identities) { stored, _ in answer(stored) }
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
