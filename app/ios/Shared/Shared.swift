import Foundation
import LocalAuthentication
import Security

/// What PassBundle's app and its AutoFill extension share (docs/phones.md,
/// "Autofill"): the folder the vault is kept in, and each account's Keychain
/// item that unlocks it with Face ID or Touch ID. Both are in groups both can
/// reach (the App Group, and the Keychain group the entitlements name first,
/// where items go by default), so the extension, its own process, reads the
/// same kept vault and unlocks it on its own.
enum Shared {
  static let appGroup = "group.com.passbundle.app"

  /// The App Group's folder; the app's own Application Support folder in a
  /// build without the group.
  static func filesDir() -> String? {
    let base =
      FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup)
      ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
    guard let dir = base?.appendingPathComponent("PassBundle", isDirectory: true) else { return nil }
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return dir.path
  }

  static let biometricService = "com.passbundle.app.biometric"

  /// An account's id on this device, as the core makes them (accounts.rs).
  static func isAccount(_ account: String) -> Bool {
    account.count == 32 && account.allSatisfy { $0.isASCII && $0.isHexDigit && !$0.isUppercase }
  }

  /// The Keychain item holding an account's user key for Face ID or Touch
  /// ID: each account its own (DESIGN.md in the meta repo, "Several accounts
  /// in the apps").
  static func biometricItem(_ account: String) -> [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: biometricService,
      kSecAttrAccount as String: "user-key-" + account,
    ]
  }

  /// "Face ID" or "Touch ID": what the screens call it.
  static func biometricMethod() -> String {
    let context = LAContext()
    _ = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
    switch context.biometryType {
    case .touchID: return "Touch ID"
    case .opticID: return "Optic ID"
    default: return "Face ID"
    }
  }

  /// Whether an account's user key is kept for Face ID or Touch ID, without asking.
  static func biometricKept(_ account: String) -> Bool {
    guard isAccount(account) else { return false }
    let context = LAContext()
    context.interactionNotAllowed = true
    var query = biometricItem(account)
    query[kSecUseAuthenticationContext as String] = context
    query[kSecReturnAttributes as String] = true
    let status = SecItemCopyMatching(query as CFDictionary, nil)
    return status == errSecSuccess || status == errSecInteractionNotAllowed
  }

  /// Turns it off for one account: its Keychain item goes, and the others' stay.
  static func removeBiometric(_ account: String) {
    guard isAccount(account) else { return }
    SecItemDelete(biometricItem(account) as CFDictionary)
  }

  /// Unlocks an account: reading its key asks for Face ID or Touch ID, then
  /// the core opens that account's kept vault with it. `done(unlocked,
  /// error)` on the main queue: unlocked is false, with no words, if the
  /// person cancelled. With `forApproval`, it's the person confirming it's
  /// them to approve a sign-in from another device, and nothing else: the
  /// core only compares the key with that account's open vault's
  /// (`pb_confirm_approval_with_key`), and the prompt says `reason`.
  ///
  /// `wanted` is asked on the main queue once the Keychain has answered,
  /// before the key is used: false lets the key go unused, and nothing is
  /// answered (the AutoFill extension's request ended while Face ID was up).
  ///
  /// The key is in memory here only from the Keychain's answer until the
  /// core's, on the main queue's next turn (`KeptKey`, below, says what is
  /// wiped and what can't be).
  static func unlockWithBiometric(
    account: String, reason: String = "Unlock PassBundle", forApproval: Bool = false,
    wanted: @escaping () -> Bool = { true },
    _ done: @escaping (Bool, String?) -> Void
  ) {
    let method = biometricMethod()
    guard isAccount(account) else { return done(false, "Unlocking with \(method) is off. Use your master password.") }
    DispatchQueue.global(qos: .userInitiated).async {
      let context = LAContext()
      context.localizedReason = reason
      var query = biometricItem(account)
      query[kSecUseAuthenticationContext as String] = context
      query[kSecReturnData as String] = true
      var found: CFTypeRef?
      let status = SecItemCopyMatching(query as CFDictionary, &found)
      let key = KeptKey(found)
      found = nil
      DispatchQueue.main.async {
        guard wanted() else { return key.letGo() }
        switch status {
        case errSecSuccess:
          let given = key.use { bytes, count in
            forApproval
              ? pb_confirm_approval_with_key(account, bytes, count) : pb_unlock_with_key(account, bytes, count)
          }
          guard let answer = given ?? nil else { return done(false, "Couldn't unlock with \(method).") }
          let why = String(cString: answer)
          pb_free(answer)
          done(why.isEmpty, why.isEmpty ? nil : why)
        case errSecUserCanceled:
          done(false, nil)
        case errSecItemNotFound:
          // Off, or voided by a face or finger added since.
          removeBiometric(account)
          done(false, "Unlocking with \(method) is off. Use your master password.")
        default:
          done(false, "Couldn't unlock with \(method).")
        }
      }
    }
  }

  /// base64url without padding, as WebAuthn and the core write IDs.
  static func base64url(_ data: Data) -> String {
    data.base64EncodedString()
      .replacingOccurrences(of: "+", with: "-")
      .replacingOccurrences(of: "/", with: "_")
      .replacingOccurrences(of: "=", with: "")
  }

  /// base64url, padded or not, read; nil when it isn't.
  static func fromBase64url(_ text: String) -> Data? {
    var plain = text.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
    plain = plain.trimmingCharacters(in: CharacterSet(charactersIn: "="))
    plain += String(repeating: "=", count: (4 - plain.count % 4) % 4)
    return Data(base64Encoded: plain)
  }

  /// One of the core's calls that answers JSON, read and freed.
  static func answer(_ text: UnsafeMutablePointer<CChar>?) -> Any? {
    guard let text else { return nil }
    defer { pb_free(text) }
    return try? JSONSerialization.jsonObject(with: Data(bytes: text, count: strlen(text)))
  }

  /// A value the core hands over as UTF-8 bytes (`pb_secret`, `pb_fill_secret`),
  /// wiped from the core's buffer once it's a string. The string is Swift's,
  /// which can't be wiped: it's made only for what takes it at once (the
  /// pasteboard, or iOS's AutoFill, which keep their own copies), and kept
  /// nowhere after.
  static func string(_ bytes: UnsafeMutablePointer<UInt8>?, _ length: Int) -> String? {
    guard let bytes else { return nil }
    defer { pb_free_secret(bytes, length) }
    return String(bytes: UnsafeBufferPointer(start: bytes, count: length), encoding: .utf8)
  }
}

/// An account's user key as the Keychain hands it over (`Shared.unlockWithBiometric`),
/// for as short a time as it's needed.
///
/// What is wiped, and what isn't: the bytes are in a CFData the Keychain
/// made, which can't be changed, so it can't be wiped. A `Data` made from it
/// would only be one more copy, and wiping that would wipe the copy. So no
/// copy is made in Swift: the core reads the bytes where they are, and the
/// CFData is let go as soon as the core has answered, or at once when the
/// key isn't wanted any more. Its memory goes back to the system as it was.
/// The copy the core makes for itself is the core's to wipe, and it does
/// (`pb_unlock_with_key`).
///
/// Made on the queue that read the Keychain and used on the main queue,
/// never by both at once.
final class KeptKey: @unchecked Sendable {
  private var data: CFData?

  init(_ found: CFTypeRef?) {
    if let found, CFGetTypeID(found) == CFDataGetTypeID() {
      data = unsafeDowncast(found, to: CFData.self)
    }
  }

  /// The key's bytes for `call`, once, then let go: nil if there's no key.
  func use<T>(_ call: (UnsafePointer<UInt8>, Int) -> T) -> T? {
    defer { letGo() }
    guard let data, let bytes = CFDataGetBytePtr(data) else { return nil }
    return call(bytes, CFDataGetLength(data))
  }

  func letGo() {
    data = nil
  }
}
