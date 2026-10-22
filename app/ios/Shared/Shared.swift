import Foundation
import LocalAuthentication
import Security

/// What PassBundle's app and its AutoFill extension share (docs/phones.md,
/// "Autofill"): the folder the vault is kept in, and the Keychain item that
/// unlocks it with Face ID or Touch ID. Both are in groups both can reach, so
/// the extension, its own process, reads the same kept vault and unlocks it
/// on its own.
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

  /// The Keychain item holding the user key for Face ID or Touch ID.
  static var biometricItem: [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: "com.passbundle.app.biometric",
      kSecAttrAccount as String: "user-key",
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

  /// Whether the user key is kept for Face ID or Touch ID, without asking.
  static func biometricKept() -> Bool {
    let context = LAContext()
    context.interactionNotAllowed = true
    var query = biometricItem
    query[kSecUseAuthenticationContext as String] = context
    query[kSecReturnAttributes as String] = true
    let status = SecItemCopyMatching(query as CFDictionary, nil)
    return status == errSecSuccess || status == errSecInteractionNotAllowed
  }

  /// Turns it off: the Keychain item goes.
  static func removeBiometric() {
    SecItemDelete(biometricItem as CFDictionary)
  }

  /// Reads the user key, which asks for Face ID or Touch ID, and opens the
  /// kept vault with it. `done(unlocked, error)` on the main queue: unlocked
  /// is false, with no words, if the person cancelled.
  static func unlockWithBiometric(_ done: @escaping (Bool, String?) -> Void) {
    DispatchQueue.global(qos: .userInitiated).async {
      let context = LAContext()
      context.localizedReason = "Unlock PassBundle"
      var query = biometricItem
      query[kSecUseAuthenticationContext as String] = context
      query[kSecReturnData as String] = true
      var found: CFTypeRef?
      let status = SecItemCopyMatching(query as CFDictionary, &found)
      DispatchQueue.main.async {
        let method = biometricMethod()
        switch status {
        case errSecSuccess:
          guard var key = found as? Data else { return done(false, "Couldn't unlock with \(method).") }
          let why = key.withUnsafeBytes { raw -> String in
            guard let answer = pb_unlock_with_key(raw.bindMemory(to: UInt8.self).baseAddress, raw.count) else {
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
          removeBiometric()
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

  /// An item's value from the core ("username", "password", ...), wiped from
  /// the core's buffer once it's a string.
  static func secret(_ id: String, _ what: String) -> String? {
    var length = 0
    guard let bytes = pb_secret(id, what, &length) else { return nil }
    defer { pb_free_secret(bytes, length) }
    return String(bytes: UnsafeBufferPointer(start: bytes, count: length), encoding: .utf8)
  }
}
