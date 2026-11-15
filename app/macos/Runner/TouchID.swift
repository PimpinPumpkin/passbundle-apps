// Unlocking with Touch ID on the Mac (the meta repo's docs/core.md, "The
// Mac without an Apple developer account"). Without an Apple developer team
// the app can't use the data protection keychain, where the iPhone keeps
// the user key behind Face ID or Touch ID (`biometryCurrentSet`). So each
// account's user key is sealed to a key of its own in the Secure Enclave,
// which needs no entitlement and does a key agreement only after the Mac's
// current fingerprints match: adding or removing one voids it. The seal, the
// enclave's wrapped key, and nothing else are kept in a file of the
// account's, readable only by the person. The user key goes between the core
// and the enclave here, never through Dart, and never as a Data: sealing
// reads it from the core's own buffer, and opening puts it in one buffer of
// its own (`TouchIDSecret`), zeroed once the core has it.
//
// Not the weaker "ask for Touch ID, then read a normal keychain item" that
// DESIGN.md rules out: without the enclave's agreement, which Touch ID
// gates, the file opens nothing.
//
// MainFlutterWindow.swift hands these to Dart (app/lib/core.dart). Nothing
// here uses Flutter, so tools/mac-touch-id-test.sh builds this file into a
// test program of its own and checks it against the meta repo's vectors
// (core/tests/vectors/mac-local-v1.json) and the real Secure Enclave.
import CryptoKit
import Foundation
import LocalAuthentication
import Security

/// A secret's bytes in one buffer of its own, zeroed when it's wiped and
/// when it's let go of. A class, so handing it on never copies them. Never a
/// Data: copies of a Data share their bytes until one is changed, so zeroing
/// one zeroes a new copy and leaves the bytes where they were.
final class TouchIDSecret {
  private let bytes: UnsafeMutableRawBufferPointer

  /// Takes `data`'s bytes and zeroes them where they were. `data` must be
  /// the only one with them, as what CryptoKit hands back is.
  init(taking data: inout Data) {
    let bytes = UnsafeMutableRawBufferPointer.allocate(byteCount: data.count, alignment: 1)
    data.withUnsafeMutableBytes { raw in
      bytes.copyMemory(from: UnsafeRawBufferPointer(raw))
      if let base = raw.baseAddress { memset_s(base, raw.count, 0, raw.count) }
    }
    self.bytes = bytes
  }

  var count: Int { bytes.count }

  func withBytes<T>(_ body: (UnsafeRawBufferPointer) throws -> T) rethrows -> T {
    try body(UnsafeRawBufferPointer(bytes))
  }

  func wipe() {
    if let base = bytes.baseAddress { memset_s(base, bytes.count, 0, bytes.count) }
  }

  deinit {
    wipe()
    bytes.deallocate()
  }
}

/// Sealing a user key to a Secure Enclave key (docs/vectors/mac-local-v1.json
/// in the meta repo, "touchId"): an ephemeral P-256 key agrees with the
/// enclave's public key; HKDF-SHA256 of the shared secret, salted with both
/// public keys (X9.63, the ephemeral's first), with `info`, gives an
/// AES-256-GCM key; the user key is sealed with it, authenticating `info`, a
/// zero byte, and the account's id, so one account's seal never opens as
/// another's. Opening needs the enclave's half of the agreement. The shared
/// secret and the AES key stay CryptoKit's own (SharedSecret, SymmetricKey),
/// which zero themselves when let go of.
///
/// `info` is what the seal is for, so one kind never opens as another: Touch
/// ID's own here, and the PIN record's in DeviceSeal.swift.
enum TouchIDSeal {
  static let info = Data("keykeeper Touch ID v1".utf8)

  static func authenticated(_ account: String, info: Data = info) -> Data {
    info + Data([0]) + Data(account.utf8)
  }

  private static func key(
    _ shared: SharedSecret, ephemeral: Data, enclave: Data, info: Data
  ) -> SymmetricKey {
    shared.hkdfDerivedSymmetricKey(
      using: SHA256.self, salt: ephemeral + enclave, sharedInfo: info, outputByteCount: 32)
  }

  /// The user key sealed to the enclave's public key: the ephemeral public
  /// key (X9.63) and the sealed box (nonce, ciphertext, tag). Only the
  /// enclave's public key is used, so sealing asks for nothing. The user
  /// key is read where it is, not copied.
  static func seal(
    _ userKey: UnsafeRawBufferPointer, to enclave: P256.KeyAgreement.PublicKey, account: String,
    info: Data = info,
    ephemeral: P256.KeyAgreement.PrivateKey = P256.KeyAgreement.PrivateKey(),
    nonce: AES.GCM.Nonce = AES.GCM.Nonce()
  ) throws -> (ephemeral: Data, sealed: Data) {
    let shared = try ephemeral.sharedSecretFromKeyAgreement(with: enclave)
    let ephemeralPublic = ephemeral.publicKey.x963Representation
    let box = try AES.GCM.seal(
      userKey,
      using: key(shared, ephemeral: ephemeralPublic, enclave: enclave.x963Representation, info: info),
      nonce: nonce, authenticating: authenticated(account, info: info))
    guard let combined = box.combined else { throw TouchIDError.damaged }
    return (ephemeralPublic, combined)
  }

  /// The user key, opened with the enclave's half of the agreement
  /// (`agree`), which is what asks for Touch ID. The caller wipes it.
  static func open(
    _ sealed: Data, ephemeral: Data, enclave: P256.KeyAgreement.PublicKey, account: String,
    info: Data = info,
    agree: (P256.KeyAgreement.PublicKey) throws -> SharedSecret
  ) throws -> TouchIDSecret {
    let shared = try agree(P256.KeyAgreement.PublicKey(x963Representation: ephemeral))
    // CryptoKit opens into a Data, held only here: taken and zeroed at once.
    var opened = try AES.GCM.open(
      AES.GCM.SealedBox(combined: sealed),
      using: key(shared, ephemeral: ephemeral, enclave: enclave.x963Representation, info: info),
      authenticating: authenticated(account, info: info))
    return TouchIDSecret(taking: &opened)
  }
}

enum TouchIDError: Error {
  case damaged
}

/// What's kept for one account: the enclave's key (wrapped by the enclave,
/// usable only on this Mac and only by the enclave), the ephemeral public
/// key and the seal, and the Mac's fingerprints as they were when it was
/// turned on (LocalAuthentication's domain state, a hash, not a
/// fingerprint), so a change is noticed before asking.
struct TouchIDRecord: Codable, Equatable {
  var version = 1
  var enclaveKey: Data
  var ephemeralPublicKey: Data
  var sealed: Data
  var fingerprints: Data
}

/// Each account's record, in a folder only the person can open (0700), in
/// files only they can read (0600, from the moment each is made): in
/// PassBundle's folder in Application Support, `touch-id/<account id>.json`.
struct TouchIDFiles {
  let folder: URL

  static var app: TouchIDFiles {
    let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    return TouchIDFiles(folder: support.appendingPathComponent("PassBundle/touch-id", isDirectory: true))
  }

  /// An account's id on this device, as the core makes them (accounts.rs):
  /// 32 lowercase hex digits, so it's always a plain file name.
  static func isAccount(_ account: String) -> Bool {
    account.count == 32 && account.allSatisfy { $0.isASCII && $0.isHexDigit && !$0.isUppercase }
  }

  func file(_ account: String) -> URL {
    folder.appendingPathComponent("\(account).json")
  }

  func read(_ account: String) -> TouchIDRecord? {
    guard Self.isAccount(account), let data = try? Data(contentsOf: file(account)) else { return nil }
    return try? JSONDecoder().decode(TouchIDRecord.self, from: data)
  }

  /// Written whole, as a new file made readable only by the person, then
  /// renamed into place.
  func write(_ record: TouchIDRecord, _ account: String) throws {
    guard Self.isAccount(account) else { throw TouchIDError.damaged }
    try FileManager.default.createDirectory(
      at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    guard chmod(folder.path, 0o700) == 0 else { throw TouchIDError.damaged }
    let data = try JSONEncoder().encode(record)
    let temporary = folder.appendingPathComponent(".\(account).\(UUID().uuidString).new")
    let fd = open(temporary.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
    guard fd >= 0 else { throw TouchIDError.damaged }
    let written = data.withUnsafeBytes { raw in Darwin.write(fd, raw.baseAddress, raw.count) }
    let synced = fsync(fd) == 0
    close(fd)
    guard written == data.count, synced, rename(temporary.path, file(account).path) == 0 else {
      unlink(temporary.path)
      throw TouchIDError.damaged
    }
  }

  func remove(_ account: String) {
    guard Self.isAccount(account) else { return }
    unlink(file(account).path)
  }
}

/// What the core does for Touch ID, without Dart: hand over an unlocked
/// account's user key to seal, and take one back to unlock with or to
/// confirm an approval with.
protocol TouchIDCore {
  /// `body` with the account's user key where the core has it for the call,
  /// wiped after; nil while the account is locked or the core is busy.
  func withUserKey<T>(_ account: String, _ body: (UnsafeRawBufferPointer) throws -> T) rethrows -> T?
  /// The user key given back: "" if the core took it, else the words to
  /// show. The core reads it where it is; the caller wipes it after.
  func give(_ account: String, key: TouchIDSecret, forApproval: Bool) -> String
}

/// The core the app loaded (Dart opens it: app/lib/core.dart), found by its
/// C functions. Never a second copy of it: that would be a core of its own,
/// with no account unlocked.
final class LoadedCore: TouchIDCore {
  private typealias UserKey = @convention(c) (UnsafePointer<CChar>, UnsafeMutablePointer<Int>) -> UnsafeMutablePointer<UInt8>?
  private typealias FreeSecret = @convention(c) (UnsafeMutablePointer<UInt8>?, Int) -> Void
  private typealias WithKey = @convention(c) (UnsafePointer<CChar>, UnsafePointer<UInt8>?, Int) -> UnsafeMutablePointer<CChar>?
  private typealias Free = @convention(c) (UnsafeMutablePointer<CChar>?) -> Void
  typealias Remove = @convention(c) (UnsafePointer<CChar>?) -> Void
  private typealias SetRemove = @convention(c) (Remove) -> Void
  private typealias SetDeviceSeal = @convention(c) (DeviceSeal.Call, DeviceSeal.Call) -> Void

  private let userKey: UserKey
  private let freeSecret: FreeSecret
  private let unlockWithKey: WithKey
  private let confirmWithKey: WithKey
  private let free: Free
  private let setRemove: SetRemove
  private let setDeviceSeal: SetDeviceSeal

  /// Where Dart opened it, as Dart names it (the program's folder, then
  /// ../Frameworks), only if it's loaded; else wherever the process has it.
  static func find() -> LoadedCore? {
    var handle: UnsafeMutableRawPointer?
    if let program = Bundle.main.executableURL?.resolvingSymlinksInPath() {
      let path = program.deletingLastPathComponent().path + "/../Frameworks/libpassbundle_core.dylib"
      handle = dlopen(path, RTLD_NOW | RTLD_NOLOAD)
    }
    return LoadedCore(handle ?? UnsafeMutableRawPointer(bitPattern: -2))  // RTLD_DEFAULT
  }

  private init?(_ handle: UnsafeMutableRawPointer?) {
    func symbol<T>(_ name: String, _: T.Type) -> T? {
      dlsym(handle, name).map { unsafeBitCast($0, to: T.self) }
    }
    guard let userKey = symbol("pb_user_key", UserKey.self),
      let freeSecret = symbol("pb_free_secret", FreeSecret.self),
      let unlockWithKey = symbol("pb_unlock_with_key", WithKey.self),
      let confirmWithKey = symbol("pb_confirm_approval_with_key", WithKey.self),
      let free = symbol("pb_free", Free.self),
      let setRemove = symbol("pb_set_remove_biometric", SetRemove.self),
      let setDeviceSeal = symbol("pb_set_device_seal", SetDeviceSeal.self)
    else { return nil }
    self.userKey = userKey
    self.freeSecret = freeSecret
    self.unlockWithKey = unlockWithKey
    self.confirmWithKey = confirmWithKey
    self.free = free
    self.setRemove = setRemove
    self.setDeviceSeal = setDeviceSeal
  }

  /// What the core does when it turns an account's Touch ID off (signing
  /// out, or a key that's out of date): set once; the core keeps the first.
  func onRemove(_ remove: Remove) {
    setRemove(remove)
  }

  /// Gives the core this Mac's seal for what unlocking with a PIN keeps
  /// (DeviceSeal.swift), where the Mac has a Secure Enclave: without one the
  /// core is given nothing, and offers no PIN. True if it was given. The
  /// core keeps the first.
  @discardableResult
  func giveDeviceSeal() -> Bool {
    guard DeviceSeal.available else { return false }
    setDeviceSeal(DeviceSeal.sealCall, DeviceSeal.openCall)
    return true
  }

  func withUserKey<T>(_ account: String, _ body: (UnsafeRawBufferPointer) throws -> T) rethrows -> T? {
    var length = 0
    guard let bytes = account.withCString({ userKey($0, &length) }) else { return nil }
    defer { freeSecret(bytes, length) }
    // Not copied: the core's buffer, which it wipes as it frees it.
    return try body(UnsafeRawBufferPointer(start: bytes, count: length))
  }

  func give(_ account: String, key: TouchIDSecret, forApproval: Bool) -> String {
    let call = forApproval ? confirmWithKey : unlockWithKey
    let answer = key.withBytes { raw in
      account.withCString { call($0, raw.bindMemory(to: UInt8.self).baseAddress, raw.count) }
    }
    guard let answer else { return "Couldn't unlock with Touch ID." }
    defer { free(answer) }
    return String(cString: answer)
  }
}

/// Touch ID for each account on this Mac: whether it's on, turning it on
/// and off, and unlocking (or confirming an approval) with it.
enum TouchID {
  static let method = "Touch ID"
  static let off = "Unlocking with Touch ID is off. Use your master password, then turn it on again."

  /// The enclave key's access: usable only on this Mac while it's unlocked,
  /// and only after the fingerprints enrolled now match.
  static func access() -> SecAccessControl? {
    SecAccessControlCreateWithFlags(
      nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, [.privateKeyUsage, .biometryCurrentSet], nil)
  }

  /// Whether this Mac can ask for Touch ID now, and its fingerprints' domain
  /// state if so.
  static func fingerprints() -> (can: Bool, state: Data?, error: LAError.Code?) {
    let context = LAContext()
    var error: NSError?
    let can = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
    return (can, can ? context.evaluatedPolicyDomainState : nil, error.flatMap { LAError.Code(rawValue: $0.code) })
  }

  /// For an account: "setUp", "notSetUp", "unavailable" (this Mac has Touch
  /// ID but no fingerprint), or "unsupported" (no Touch ID here at all, or
  /// none to reach now, as with the lid closed), which the screens don't
  /// offer. An account whose fingerprints changed since is turned off.
  static func status(account: String, files: TouchIDFiles = .app) -> String {
    let now = fingerprints()
    if let record = files.read(account) {
      if now.can, now.state != record.fingerprints {
        files.remove(account)
        return "notSetUp"
      }
      if now.error == .biometryNotEnrolled {
        files.remove(account)
        return "unavailable"
      }
      return "setUp"
    }
    if now.can { return "notSetUp" }
    return now.error == .biometryNotEnrolled || now.error == .biometryLockout ? "unavailable" : "unsupported"
  }

  static func remove(account: String, files: TouchIDFiles = .app) {
    files.remove(account)
  }

  /// Turns it on for an unlocked account: asks for Touch ID, then makes the
  /// account's enclave key and seals its user key to it. `done(on, error)`:
  /// on is false if the person cancelled.
  static func enroll(
    account: String, core: TouchIDCore, files: TouchIDFiles = .app,
    _ done: @escaping (Bool, String?) -> Void
  ) {
    let failed = "Couldn't set up unlocking with Touch ID."
    guard TouchIDFiles.isAccount(account) else { return done(false, failed) }
    let context = LAContext()
    context.localizedFallbackTitle = ""
    context.evaluatePolicy(
      .deviceOwnerAuthenticationWithBiometrics, localizedReason: "turn on unlocking with Touch ID"
    ) { agreed, error in
      DispatchQueue.main.async {
        guard agreed else { return done(false, cancelled(error) ? nil : failed) }
        do {
          guard let access = access() else { return done(false, failed) }
          let enclave = try SecureEnclave.P256.KeyAgreement.PrivateKey(accessControl: access)
          guard
            let made = try core.withUserKey(account, {
              try TouchIDSeal.seal($0, to: enclave.publicKey, account: account)
            })
          else { return done(false, "Unlock PassBundle first.") }
          let (ephemeral, sealed) = made
          let record = TouchIDRecord(
            enclaveKey: enclave.dataRepresentation, ephemeralPublicKey: ephemeral, sealed: sealed,
            fingerprints: fingerprints().state ?? Data())
          try files.write(record, account)
          done(true, nil)
        } catch {
          done(false, failed)
        }
      }
    }
  }

  /// Unlocks an account: the enclave asks for Touch ID (the system's sheet,
  /// "PassBundle is trying to <reason>"), and the core opens the account's
  /// kept vault with the user key. With `forApproval`, it's the person
  /// confirming it's them to approve a sign-in, and the core only compares
  /// the key with that account's open vault's. `done(given, error)`: given
  /// is false if the person cancelled.
  static func unlock(
    account: String, core: TouchIDCore, reason: String = "unlock your vault", forApproval: Bool = false,
    files: TouchIDFiles = .app, _ done: @escaping (Bool, String?) -> Void
  ) {
    guard let record = files.read(account) else { return done(false, off) }
    // Fingerprints added or removed since: the enclave won't use the key.
    let now = fingerprints()
    if (now.can && now.state != record.fingerprints) || now.error == .biometryNotEnrolled {
      files.remove(account)
      return done(false, off)
    }
    DispatchQueue.global(qos: .userInitiated).async {
      let context = LAContext()
      context.localizedReason = reason
      let opened = Result<TouchIDSecret, Error> {
        let enclave = try SecureEnclave.P256.KeyAgreement.PrivateKey(
          dataRepresentation: record.enclaveKey, authenticationContext: context)
        return try TouchIDSeal.open(
          record.sealed, ephemeral: record.ephemeralPublicKey, enclave: enclave.publicKey, account: account
        ) { try enclave.sharedSecretFromKeyAgreement(with: $0) }
      }
      DispatchQueue.main.async {
        finish(opened, account: account, core: core, forApproval: forApproval, files: files, done)
      }
    }
  }

  /// What `unlock` does with what the enclave opened: gives the core the
  /// user key and zeroes it, or says why not.
  static func finish(
    _ opened: Result<TouchIDSecret, Error>, account: String, core: TouchIDCore, forApproval: Bool,
    files: TouchIDFiles, _ done: (Bool, String?) -> Void
  ) {
    switch opened {
    case .success(let key):
      let why = core.give(account, key: key, forApproval: forApproval)
      key.wipe()
      done(why.isEmpty, why.isEmpty ? nil : why)
    case .failure(let failure):
      if cancelled(failure) { return done(false, nil) }
      // Touch ID said no for now (too many tries, say): kept, for later.
      if (failure as NSError).domain == LAErrorDomain {
        return done(false, "Couldn't unlock with Touch ID. Use your master password.")
      }
      // The enclave's key or the seal no longer opens: off.
      files.remove(account)
      done(false, off)
    }
  }

  /// The person cancelled, chose another way, or the system did.
  static func cancelled(_ error: Error?) -> Bool {
    guard let error = error as NSError?, error.domain == LAErrorDomain else { return false }
    let codes: [LAError.Code] = [.userCancel, .userFallback, .systemCancel, .appCancel]
    return codes.contains { $0.rawValue == error.code }
  }
}
