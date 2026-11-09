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
  static func unlockWithBiometric(
    account: String, reason: String = "Unlock PassBundle", forApproval: Bool = false,
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
      DispatchQueue.main.async {
        switch status {
        case errSecSuccess:
          guard var key = found as? Data else { return done(false, "Couldn't unlock with \(method).") }
          let why = key.withUnsafeBytes { raw -> String in
            let bytes = raw.bindMemory(to: UInt8.self).baseAddress
            let given =
              forApproval
              ? pb_confirm_approval_with_key(account, bytes, raw.count) : pb_unlock_with_key(account, bytes, raw.count)
            guard let answer = given else {
              return "Couldn't unlock with \(method)."
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
          removeBiometric(account)
          done(false, "Unlocking with \(method) is off. Use your master password.")
        default:
          done(false, "Couldn't unlock with \(method).")
        }
      }
    }
  }

  /// One of the core's calls that answers JSON, read and freed.
  static func answer(_ text: UnsafeMutablePointer<CChar>?) -> Any? {
    guard let text else { return nil }
    defer { pb_free(text) }
    return try? JSONSerialization.jsonObject(with: Data(bytes: text, count: strlen(text)))
  }

  /// A value the core hands over as UTF-8 bytes (`pb_secret`, `pb_fill_secret`),
  /// wiped from the core's buffer once it's a string.
  static func string(_ bytes: UnsafeMutablePointer<UInt8>?, _ length: Int) -> String? {
    guard let bytes else { return nil }
    defer { pb_free_secret(bytes, length) }
    return String(bytes: UnsafeBufferPointer(start: bytes, count: length), encoding: .utf8)
  }
}
