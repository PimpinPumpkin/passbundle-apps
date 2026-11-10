// Sealing to this iPhone what unlocking with a PIN keeps (DESIGN.md in the
// meta repo, "Unlock with a PIN"). The core wraps the account's user key
// under a key made from the PIN and hands that record here to be sealed to
// the phone before it's written to the account's folder. A PIN is short, so
// the record alone could be guessed at leisure; sealed, a copy of the files
// or a backup is nothing to guess against.
//
// The seal is AES-256-GCM under one random key the app keeps in the
// Keychain for this device only, readable only while the phone is unlocked
// (`kSecAttrAccessibleWhenUnlockedThisDeviceOnly`), so it's in no backup
// another phone can restore. It authenticates the label, a zero byte, and
// the account's id, as the core, the Mac's Swift and Android's Kotlin do
// (core/src/secrets.rs, `DEVICE_SEAL_LABEL`), so one account's record never
// opens as another's. What the core keeps is the nonce, the ciphertext, and
// the tag (CryptoKit's combined form). The key asks for nothing to be read,
// so neither call ever shows a prompt.
//
// AppDelegate.swift gives the core these calls at start
// (`pb_set_device_seal`). The AutoFill extension doesn't, so it offers no
// PIN.
import CryptoKit
import Foundation
import Security

enum DeviceSeal {
  static let info = Data("keykeeper PIN device seal v1".utf8)

  /// The Keychain item with the key: one for the app, not one per account,
  /// since each account's record authenticates its own id.
  private static let item: [String: Any] = [
    kSecClass as String: kSecClassGenericPassword,
    kSecAttrService as String: "com.passbundle.app.pin",
    kSecAttrAccount as String: "device-seal-key",
  ]

  static func authenticated(_ account: String) -> Data {
    info + Data([0]) + Data(account.utf8)
  }

  /// The key, read from the Keychain, or made and kept there the first time
  /// a record is sealed. Nil when the Keychain can't be read now (the phone
  /// is locked): an existing key is never replaced, which would leave every
  /// account's record unopenable.
  private static func key(making: Bool) -> SymmetricKey? {
    var query = item
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var found: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &found)
    if status == errSecSuccess, var bytes = found as? Data {
      defer { bytes.resetBytes(in: 0..<bytes.count) }
      return bytes.count == 32 ? SymmetricKey(data: bytes) : nil
    }
    guard making, status == errSecItemNotFound else { return nil }
    let key = SymmetricKey(size: .bits256)
    var add = item
    add[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
    add[kSecValueData as String] = key.withUnsafeBytes { Data($0) }
    let added = SecItemAdd(add as CFDictionary, nil)
    // Another call made one first: that's the key.
    if added == errSecDuplicateItem { return Self.key(making: false) }
    return added == errSecSuccess ? key : nil
  }

  /// `plain` sealed for the account: the nonce, the ciphertext, and the tag.
  static func seal(_ plain: UnsafeRawBufferPointer, account: String) throws -> Data {
    guard Shared.isAccount(account), let key = key(making: true) else { throw DeviceSealError.unavailable }
    guard let combined = try AES.GCM.seal(plain, using: key, authenticating: authenticated(account)).combined
    else { throw DeviceSealError.unavailable }
    return combined
  }

  /// What a record sealed for the account on this phone opens to, written
  /// to `output`, and how many bytes that is. Another phone's, another
  /// account's, or a changed one doesn't open. What CryptoKit opened into
  /// is zeroed before this returns.
  static func open(_ sealed: Data, account: String, into output: UnsafeMutableRawBufferPointer) throws -> Int {
    guard Shared.isAccount(account), let key = key(making: false) else { throw DeviceSealError.unavailable }
    var opened = try AES.GCM.open(
      AES.GCM.SealedBox(combined: sealed), using: key, authenticating: authenticated(account))
    defer { opened.resetBytes(in: 0..<opened.count) }
    guard opened.count <= output.count else { throw DeviceSealError.unavailable }
    opened.withUnsafeBytes { output.copyMemory(from: $0) }
    return opened.count
  }

  /// How the core calls either (core/src/secrets.rs, `DeviceSealCall`): the
  /// account's id, `length` bytes at `input`, and `capacity` bytes at
  /// `output` for the answer. Each returns how many bytes it wrote, or a
  /// negative number when it can't, or when the answer doesn't fit.
  typealias Call = @convention(c) (
    UnsafePointer<CChar>?, UnsafePointer<UInt8>?, Int, UnsafeMutablePointer<UInt8>?, Int
  ) -> Int

  static let sealCall: Call = { account, input, length, output, capacity in
    guard let account, let input, let output, length > 0, capacity > 0 else { return -1 }
    let plain = UnsafeRawBufferPointer(start: input, count: length)
    guard let sealed = try? seal(plain, account: String(cString: account)), sealed.count <= capacity
    else { return -1 }
    sealed.copyBytes(to: output, count: sealed.count)
    return sealed.count
  }

  static let openCall: Call = { account, input, length, output, capacity in
    guard let account, let input, let output, length > 0, capacity > 0 else { return -1 }
    // The record isn't secret: what it opens to is.
    let sealed = Data(bytes: input, count: length)
    let room = UnsafeMutableRawBufferPointer(start: output, count: capacity)
    return (try? open(sealed, account: String(cString: account), into: room)) ?? -1
  }
}

enum DeviceSealError: Error {
  case unavailable
}
