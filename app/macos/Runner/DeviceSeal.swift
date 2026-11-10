// Sealing to this Mac what unlocking with a PIN keeps (DESIGN.md in the meta
// repo, "Unlock with a PIN"). The core wraps the account's user key under a
// key made from the PIN, and hands that record here to be sealed to the
// device before it's written to the account's folder. A PIN is short, so the
// record alone could be guessed at leisure; sealed, a copy of the disk is
// nothing to guess against without this Mac's Secure Enclave.
//
// The seal is Touch ID's (TouchID.swift, `TouchIDSeal`) with a label of its
// own: each record gets a new key in the Secure Enclave, an ephemeral P-256
// key agrees with it, and HKDF-SHA256 of the shared secret gives the
// AES-256-GCM key. The enclave's key asks for no fingerprint: the PIN is
// what the person gives. It's usable only on this Mac, only while the Mac is
// unlocked, and only by the enclave. Nothing goes in a keychain.
//
// What the core gets back and keeps is `DeviceSealRecord` as JSON: the
// enclave's key (wrapped by the enclave), the ephemeral public key, and the
// sealed box. The core calls `sealCall` and `openCall` once it's been given
// them (`pb_set_device_seal`, by `LoadedCore.giveDeviceSeal`). Neither ever
// shows a prompt.
//
// Nothing here uses Flutter, so tools/mac-touch-id-test.sh builds this file
// with TouchID.swift and checks it against the meta repo's vectors
// (core/tests/vectors/pin-unlock-v1.json, "macDeviceSeal") and the real
// Secure Enclave.
import CryptoKit
import Foundation
import LocalAuthentication
import Security

/// What the core keeps for one sealed record.
struct DeviceSealRecord: Codable, Equatable {
  var version = 1
  var enclaveKey: Data
  var ephemeralPublicKey: Data
  var sealed: Data
}

enum DeviceSeal {
  /// What the seal is for. With a zero byte and the account's id, it's what
  /// the core, Android's Kotlin and the iPhone's Swift authenticate too
  /// (core/src/secrets.rs, `DEVICE_SEAL_LABEL`).
  static let info = Data("keykeeper PIN device seal v1".utf8)

  /// Only a Mac with a Secure Enclave seals. Others are offered no PIN.
  static var available: Bool { SecureEnclave.isAvailable }

  /// The enclave key's access: usable only on this Mac while it's unlocked.
  /// No fingerprint, so using it never asks for anything.
  static func access() -> SecAccessControl? {
    SecAccessControlCreateWithFlags(
      nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, .privateKeyUsage, nil)
  }

  /// `plain` sealed for the account to a new key in this Mac's Secure
  /// Enclave: the record to keep. `plain` is read where it is, not copied.
  static func seal(_ plain: UnsafeRawBufferPointer, account: String) throws -> Data {
    guard TouchIDFiles.isAccount(account), let access = access() else { throw TouchIDError.damaged }
    let enclave = try SecureEnclave.P256.KeyAgreement.PrivateKey(accessControl: access)
    let (ephemeral, sealed) = try TouchIDSeal.seal(
      plain, to: enclave.publicKey, account: account, info: info)
    return try JSONEncoder().encode(
      DeviceSealRecord(
        enclaveKey: enclave.dataRepresentation, ephemeralPublicKey: ephemeral, sealed: sealed))
  }

  /// What a record sealed for the account on this Mac holds. Another Mac's,
  /// another account's, or a changed one doesn't open. The caller wipes it.
  static func open(_ record: Data, account: String) throws -> TouchIDSecret {
    guard TouchIDFiles.isAccount(account) else { throw TouchIDError.damaged }
    let record = try JSONDecoder().decode(DeviceSealRecord.self, from: record)
    guard record.version == 1 else { throw TouchIDError.damaged }
    // The key asks for nothing. If the system would ask anyway, it fails.
    let context = LAContext()
    context.interactionNotAllowed = true
    let enclave = try SecureEnclave.P256.KeyAgreement.PrivateKey(
      dataRepresentation: record.enclaveKey, authenticationContext: context)
    return try TouchIDSeal.open(
      record.sealed, ephemeral: record.ephemeralPublicKey, enclave: enclave.publicKey,
      account: account, info: info
    ) { try enclave.sharedSecretFromKeyAgreement(with: $0) }
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
    guard let record = try? seal(plain, account: String(cString: account)),
      record.count <= capacity
    else { return -1 }
    record.copyBytes(to: output, count: record.count)
    return record.count
  }

  static let openCall: Call = { account, input, length, output, capacity in
    guard let account, let input, let output, length > 0, capacity > 0 else { return -1 }
    // The record isn't secret: what it opens to is.
    let record = Data(bytes: input, count: length)
    guard let opened = try? open(record, account: String(cString: account)) else { return -1 }
    defer { opened.wipe() }
    guard opened.count <= capacity else { return -1 }
    opened.withBytes { raw in
      UnsafeMutableRawBufferPointer(start: output, count: capacity).copyMemory(from: raw)
    }
    return opened.count
  }
}
