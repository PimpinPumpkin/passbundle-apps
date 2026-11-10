// Touch ID on the Mac (app/macos/Runner/TouchID.swift) and the seal for
// what unlocking with a PIN keeps (DeviceSeal.swift), on their own: built
// with this file by tools/mac-touch-id-test.sh. Both seals against the meta
// repo's vectors (a software key standing in for the Secure Enclave's), the
// opened user key's one buffer and its zeroing, the account's file and its
// modes, the core's C functions as the app finds them, and, where this Mac
// has a Secure Enclave, the real one: a key it keeps seals and opens a user
// key, one that needs Touch ID won't open it without, and a PIN's record
// sealed through the core opens only as its own account's. Never shows a
// prompt, and never reads or writes a keychain.
//
//   touch-id-test <mac-local-v1.json> <pin-unlock-v1.json> [<the core, built with its ffi feature>]
import CryptoKit
import Foundation
import LocalAuthentication

var failures = 0

func expect(_ ok: Bool, _ what: String, line: Int = #line) {
  if ok {
    print("ok    \(what)")
  } else {
    failures += 1
    print("FAIL  \(what) (main.swift:\(line))")
  }
}

func throwsError(_ body: () throws -> Void) -> Bool {
  do {
    try body()
    return false
  } catch {
    return true
  }
}

let arguments = CommandLine.arguments
func vectors(_ path: String, _ name: String) -> [String: Any]? {
  (try? JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: path))) as? [String: Any])?[name]
    as? [String: Any]
}
guard arguments.count >= 3, let v = vectors(arguments[1], "touchId"),
  let pinVectors = vectors(arguments[2], "macDeviceSeal")
else {
  print("touch-id-test <mac-local-v1.json> <pin-unlock-v1.json> [<core library>]")
  exit(2)
}
let coreLibrary = arguments.count >= 4 ? arguments[3] : nil
func bytes(_ name: String) -> Data { Data(base64Encoded: v[name] as! String)! }
func text(_ name: String) -> String { v[name] as! String }
func pinBytes(_ name: String) -> Data { Data(base64Encoded: pinVectors[name] as! String)! }
func pinText(_ name: String) -> String { pinVectors[name] as! String }

/// Sealing as the app does, from the bytes where they are.
func seal(
  _ userKey: Data, to enclave: P256.KeyAgreement.PublicKey, account: String,
  ephemeral: P256.KeyAgreement.PrivateKey = P256.KeyAgreement.PrivateKey(),
  nonce: AES.GCM.Nonce = AES.GCM.Nonce()
) throws -> (ephemeral: Data, sealed: Data) {
  try userKey.withUnsafeBytes {
    try TouchIDSeal.seal($0, to: enclave, account: account, ephemeral: ephemeral, nonce: nonce)
  }
}

/// A secret of the test's own, from bytes that aren't one.
func secret(_ bytes: Data) -> TouchIDSecret {
  var copy = bytes
  return TouchIDSecret(taking: &copy)
}

func address(_ data: Data) -> UnsafeRawPointer? { data.withUnsafeBytes { $0.baseAddress } }
func zeroed(_ secret: TouchIDSecret) -> Bool { secret.withBytes { $0.allSatisfy { $0 == 0 } } }

/// A core that keeps hold of the key it was given, to look at it after.
final class KeepingCore: TouchIDCore {
  var given: TouchIDSecret?
  var saw = Data()
  var at: UnsafeRawPointer?
  var answer = ""

  func withUserKey<T>(_ account: String, _ body: (UnsafeRawBufferPointer) throws -> T) rethrows -> T? { nil }

  func give(_ account: String, key: TouchIDSecret, forApproval: Bool) -> String {
    given = key
    saw = key.withBytes { Data($0) }
    at = key.withBytes { $0.baseAddress }
    return answer
  }
}

// The seal, as the vectors have it.
do {
  let enclave = try P256.KeyAgreement.PrivateKey(rawRepresentation: bytes("enclavePrivateKey"))
  let ephemeral = try P256.KeyAgreement.PrivateKey(rawRepresentation: bytes("ephemeralPrivateKey"))
  expect(enclave.publicKey.x963Representation == bytes("enclavePublicKey"), "the enclave stand-in's public key")
  expect(TouchIDSeal.info == Data(text("info").utf8), "the seal's info")
  let account = text("account")
  let (ephemeralPublic, sealed) = try seal(
    bytes("userKey"), to: enclave.publicKey, account: account, ephemeral: ephemeral,
    nonce: AES.GCM.Nonce(data: bytes("nonce")))
  expect(ephemeralPublic == bytes("ephemeralPublicKey"), "the ephemeral public key, as X9.63")
  expect(sealed == bytes("sealed"), "the user key sealed exactly as the vectors say")
  let agree = { (other: P256.KeyAgreement.PublicKey) in try enclave.sharedSecretFromKeyAgreement(with: other) }
  let opened = try TouchIDSeal.open(
    bytes("sealed"), ephemeral: bytes("ephemeralPublicKey"), enclave: enclave.publicKey, account: account,
    agree: agree)
  expect(opened.withBytes { Data($0) } == bytes("userKey"), "the vectors' seal opens to their user key")
  expect(
    throwsError {
      _ = try TouchIDSeal.open(
        bytes("sealed"), ephemeral: bytes("ephemeralPublicKey"), enclave: enclave.publicKey,
        account: text("otherAccount"), agree: agree)
    }, "another account's id doesn't open it")
  var changed = bytes("sealed")
  changed[changed.count / 2] ^= 1
  expect(
    throwsError {
      _ = try TouchIDSeal.open(
        changed, ephemeral: bytes("ephemeralPublicKey"), enclave: enclave.publicKey, account: account,
        agree: agree)
    }, "a changed seal doesn't open")
  let someoneElse = P256.KeyAgreement.PrivateKey()
  expect(
    throwsError {
      _ = try TouchIDSeal.open(
        bytes("sealed"), ephemeral: bytes("ephemeralPublicKey"), enclave: enclave.publicKey, account: account
      ) { try someoneElse.sharedSecretFromKeyAgreement(with: $0) }
    }, "another enclave key's agreement doesn't open it")
  let fresh = try seal(bytes("userKey"), to: enclave.publicKey, account: account)
  expect(fresh.sealed != bytes("sealed") && fresh.ephemeral != bytes("ephemeralPublicKey"), "a new ephemeral key and nonce each time")
} catch {
  expect(false, "the vectors' seal: \(error)")
}

// The seal for a PIN's record, as the vectors have it: Touch ID's, with a
// label of its own, so neither kind opens as the other.
do {
  let enclave = try P256.KeyAgreement.PrivateKey(rawRepresentation: pinBytes("enclavePrivateKey"))
  let ephemeral = try P256.KeyAgreement.PrivateKey(rawRepresentation: pinBytes("ephemeralPrivateKey"))
  let account = pinText("account")
  let record = Data(pinText("record").utf8)
  expect(enclave.publicKey.x963Representation == pinBytes("enclavePublicKey"), "a PIN's seal: the enclave stand-in's public key")
  expect(ephemeral.publicKey.x963Representation == pinBytes("ephemeralPublicKey"), "a PIN's seal: the ephemeral public key, as X9.63")
  expect(DeviceSeal.info == Data(pinText("info").utf8), "a PIN's seal: its label")
  let shared = try ephemeral.sharedSecretFromKeyAgreement(with: enclave.publicKey)
  expect(shared.withUnsafeBytes { Data($0) } == pinBytes("sharedSecret"), "a PIN's seal: the shared secret")
  let key = shared.hkdfDerivedSymmetricKey(
    using: SHA256.self, salt: pinBytes("ephemeralPublicKey") + pinBytes("enclavePublicKey"),
    sharedInfo: DeviceSeal.info, outputByteCount: 32)
  expect(key.withUnsafeBytes { Data($0) } == pinBytes("key"), "a PIN's seal: the AES key from HKDF")
  let (ephemeralPublic, sealed) = try record.withUnsafeBytes {
    try TouchIDSeal.seal(
      $0, to: enclave.publicKey, account: account, info: DeviceSeal.info, ephemeral: ephemeral,
      nonce: AES.GCM.Nonce(data: pinBytes("nonce")))
  }
  expect(ephemeralPublic == pinBytes("ephemeralPublicKey") && sealed == pinBytes("sealed"), "a PIN's record sealed exactly as the vectors say")
  let agree = { (other: P256.KeyAgreement.PublicKey) in try enclave.sharedSecretFromKeyAgreement(with: other) }
  let opened = try TouchIDSeal.open(
    pinBytes("sealed"), ephemeral: pinBytes("ephemeralPublicKey"), enclave: enclave.publicKey, account: account,
    info: DeviceSeal.info, agree: agree)
  expect(opened.withBytes { Data($0) } == record, "the vectors' seal opens to their record")
  for other in [pinText("otherAccount"), String(account.dropLast()), account + "0", ""] {
    expect(
      throwsError {
        _ = try TouchIDSeal.open(
          pinBytes("sealed"), ephemeral: pinBytes("ephemeralPublicKey"), enclave: enclave.publicKey,
          account: other, info: DeviceSeal.info, agree: agree)
      }, "a PIN's seal doesn't open as another account's (\"\(other)\")")
  }
  expect(
    throwsError {
      _ = try TouchIDSeal.open(
        pinBytes("sealed"), ephemeral: pinBytes("ephemeralPublicKey"), enclave: enclave.publicKey,
        account: account, agree: agree)
    }, "a PIN's seal doesn't open as Touch ID's")
  let touchID = try record.withUnsafeBytes {
    try TouchIDSeal.seal($0, to: enclave.publicKey, account: account)
  }
  expect(
    throwsError {
      _ = try TouchIDSeal.open(
        touchID.sealed, ephemeral: touchID.ephemeral, enclave: enclave.publicKey, account: account,
        info: DeviceSeal.info, agree: agree)
    }, "nor Touch ID's as a PIN's")
  // What the core keeps, as JSON, and only this version of it.
  let kept = DeviceSealRecord(enclaveKey: Data([1, 2, 3]), ephemeralPublicKey: ephemeralPublic, sealed: sealed)
  let json = try JSONEncoder().encode(kept)
  expect(try JSONDecoder().decode(DeviceSealRecord.self, from: json) == kept, "what the core keeps reads back as written")
  var later = kept
  later.version = 2
  expect(
    throwsError { _ = try DeviceSeal.open(JSONEncoder().encode(later), account: account) },
    "a record of a version this app doesn't know doesn't open")
  expect(throwsError { _ = try DeviceSeal.open(Data("{}".utf8), account: account) }, "nor does something that isn't one")
  expect(throwsError { _ = try DeviceSeal.open(json, account: "../\(account.dropFirst(3))") }, "nor anything for what isn't an account's id")
} catch {
  expect(false, "a PIN's seal against the vectors: \(error)")
}

// The core's calls into the PIN's seal take nothing they can't use.
do {
  let account = pinText("account")
  var output = [UInt8](repeating: 0, count: 64)
  let none = account.withCString { id in
    output.withUnsafeMutableBufferPointer { out in
      [
        DeviceSeal.sealCall(nil, [1], 1, out.baseAddress, out.count),
        DeviceSeal.sealCall(id, nil, 1, out.baseAddress, out.count),
        DeviceSeal.sealCall(id, [1], 0, out.baseAddress, out.count),
        DeviceSeal.sealCall(id, [1], 1, nil, out.count),
        DeviceSeal.sealCall(id, [1], 1, out.baseAddress, 0),
        DeviceSeal.openCall(nil, [1], 1, out.baseAddress, out.count),
        DeviceSeal.openCall(id, nil, 1, out.baseAddress, out.count),
        DeviceSeal.openCall(id, [1], 1, out.baseAddress, out.count),
      ]
    }
  }
  expect(none.allSatisfy { $0 < 0 }, "the seal's calls refuse what's missing, empty, or not a record")
  let notAnAccount = "not an account".withCString { id in
    output.withUnsafeMutableBufferPointer { DeviceSeal.sealCall(id, [1, 2, 3], 3, $0.baseAddress, $0.count) }
  }
  expect(notAnAccount < 0 && output.allSatisfy { $0 == 0 }, "and seal nothing for what isn't an account's id")
}

// The opened user key: one buffer, zeroed where it is once the core has it.
do {
  let userKey = bytes("userKey")
  let aes = SymmetricKey(size: .bits256)
  // What CryptoKit opens into is held by no one else, so taking it zeroes
  // those bytes where they were, not a copy of them.
  var opened = try AES.GCM.open(AES.GCM.seal(userKey, using: aes), using: aes)
  let was = address(opened)
  let taken = TouchIDSecret(taking: &opened)
  expect(taken.withBytes { Data($0) } == userKey, "an opened key is taken into a buffer of its own")
  expect(address(opened) == was && opened == Data(count: userKey.count), "and zeroed where CryptoKit left it")
  // Why it's never kept as a Data: one that shares its bytes is copied to
  // be zeroed, and the bytes stay where they were.
  var shared = userKey + Data()
  let other = shared
  let sharedWas = address(shared)
  _ = TouchIDSecret(taking: &shared)
  expect(address(shared) != sharedWas && other == userKey, "a Data sharing its bytes would be left with them")
  // Wiping zeroes that one buffer, in place.
  let held = taken.withBytes { $0.baseAddress }
  taken.wipe()
  expect(zeroed(taken) && taken.count == userKey.count && taken.withBytes { $0.baseAddress } == held, "wiping zeroes its one buffer where it is")

  // Unlocking, after the enclave has opened the key: the core reads it, and
  // it's zeroed before anything else hears, whatever the core answered.
  let folder = URL(fileURLWithPath: NSTemporaryDirectory())
    .appendingPathComponent("pb-touch-id-\(UUID().uuidString)/touch-id", isDirectory: true)
  defer { try? FileManager.default.removeItem(at: folder.deletingLastPathComponent()) }
  let files = TouchIDFiles(folder: folder)
  let account = text("account")
  for (answer, forApproval) in [("", false), ("Couldn't unlock with Touch ID.", false), ("", true)] {
    let core = KeepingCore()
    core.answer = answer
    let key = secret(userKey)
    let keyAt = key.withBytes { $0.baseAddress }
    var heard: (Bool, String?)?
    var zeroedByThen = false
    TouchID.finish(.success(key), account: account, core: core, forApproval: forApproval, files: files) {
      zeroedByThen = zeroed(key)
      heard = ($0, $1)
    }
    let what = answer.isEmpty ? (forApproval ? "approving" : "unlocking") : "a key the core doesn't take"
    expect(core.saw == userKey && core.at == keyAt && core.given === key, "\(what): the core reads the key where it is")
    expect(zeroedByThen && zeroed(core.given!), "\(what): and it's zeroed there once the core has it")
    expect(heard?.0 == answer.isEmpty && heard?.1 == (answer.isEmpty ? nil : answer), "\(what): the answer is the core's")
  }
} catch {
  expect(false, "the opened user key: \(error)")
}

// The account's file: in a folder only the person opens, readable only by them.
do {
  let folder = URL(fileURLWithPath: NSTemporaryDirectory())
    .appendingPathComponent("pb-touch-id-\(UUID().uuidString)/touch-id", isDirectory: true)
  defer { try? FileManager.default.removeItem(at: folder.deletingLastPathComponent()) }
  let files = TouchIDFiles(folder: folder)
  let account = text("account")
  let record = TouchIDRecord(
    enclaveKey: Data([1, 2, 3]), ephemeralPublicKey: bytes("ephemeralPublicKey"), sealed: bytes("sealed"),
    fingerprints: Data([4, 5]))
  try files.write(record, account)
  let mode = { (path: String) in ((try? FileManager.default.attributesOfItem(atPath: path))?[.posixPermissions] as? Int) ?? -1 }
  expect(mode(folder.path) == 0o700, "the folder is the person's alone (0700)")
  expect(mode(files.file(account).path) == 0o600, "the file is readable only by them (0600)")
  expect(files.read(account) == record, "it reads back as written")
  try files.write(record, account)
  expect(
    (try? FileManager.default.contentsOfDirectory(atPath: folder.path)) == ["\(account).json"],
    "written again, it's still one file, with nothing left beside it")
  expect(!TouchIDFiles.isAccount("../\(account.dropFirst(3))") && !TouchIDFiles.isAccount(account.uppercased()), "only an account's id names a file")
  expect(throwsError { try files.write(record, "../elsewhere") }, "nothing else is written")
  files.remove(account)
  expect(files.read(account) == nil && !FileManager.default.fileExists(atPath: files.file(account).path), "removing it leaves nothing")
} catch {
  expect(false, "the account's file: \(error)")
}

// The core's C functions, as the app finds them in the core it loaded.
var loadedCore: LoadedCore?
if let coreLibrary {
  if dlopen(coreLibrary, RTLD_NOW | RTLD_GLOBAL) == nil {
    expect(false, "the core loads: \(String(cString: dlerror()))")
  } else if let core = LoadedCore.find() {
    loadedCore = core
    expect(true, "the app finds the core's functions for Touch ID and the PIN's seal")
    let account = text("account")
    expect(core.withUserKey(account, { _ in true }) == nil, "no user key for an account that isn't open")
    expect(!core.give(account, key: secret(bytes("userKey")), forApproval: false).isEmpty, "a key for an account that isn't here isn't taken")
    expect(!core.give(account, key: secret(bytes("userKey")), forApproval: true).isEmpty, "nor is one to approve with")
  } else {
    expect(false, "the app finds the core's functions for Touch ID and the PIN's seal")
  }
}

// The real Secure Enclave, where there is one (not in a virtual machine),
// while the Mac is unlocked: its keys here are for an unlocked Mac only.
let plain = SecAccessControlCreateWithFlags(nil, kSecAttrAccessibleWhenUnlockedThisDeviceOnly, .privateKeyUsage, nil)!
var macLocked = false
if SecureEnclave.isAvailable {
  do {
    _ = try SecureEnclave.P256.KeyAgreement.PrivateKey(accessControl: plain)
  } catch let error as NSError where error.domain == NSOSStatusErrorDomain && error.code == Int(errSecInteractionNotAllowed) {
    macLocked = true
  } catch {}
}
if macLocked {
  print("skip  the Mac is locked, so the Secure Enclave makes no keys for an unlocked Mac: the real enclave")
} else if SecureEnclave.isAvailable {
  do {
    let made = try SecureEnclave.P256.KeyAgreement.PrivateKey(accessControl: plain)
    let enclave = try SecureEnclave.P256.KeyAgreement.PrivateKey(dataRepresentation: made.dataRepresentation)
    let account = text("account")
    let (ephemeral, sealed) = try seal(bytes("userKey"), to: enclave.publicKey, account: account)
    let opened = try TouchIDSeal.open(sealed, ephemeral: ephemeral, enclave: enclave.publicKey, account: account) {
      try enclave.sharedSecretFromKeyAgreement(with: $0)
    }
    expect(opened.withBytes { Data($0) } == bytes("userKey"), "the Secure Enclave opens what was sealed to its key, kept as the app keeps it")
  } catch {
    expect(false, "the Secure Enclave seals and opens: \(error)")
  }
  if TouchID.fingerprints().can {
    do {
      let enclave = try SecureEnclave.P256.KeyAgreement.PrivateKey(accessControl: TouchID.access()!)
      let account = text("account")
      let (ephemeral, sealed) = try seal(bytes("userKey"), to: enclave.publicKey, account: account)
      let context = LAContext()
      context.interactionNotAllowed = true
      let loaded = try SecureEnclave.P256.KeyAgreement.PrivateKey(
        dataRepresentation: enclave.dataRepresentation, authenticationContext: context)
      var refused: Error?
      do {
        _ = try TouchIDSeal.open(sealed, ephemeral: ephemeral, enclave: loaded.publicKey, account: account) {
          try loaded.sharedSecretFromKeyAgreement(with: $0)
        }
      } catch {
        refused = error
      }
      expect(refused != nil, "a key that needs Touch ID opens nothing without it")
      expect(refused.map { (($0 as NSError).domain == LAErrorDomain) && !TouchID.cancelled($0) } ?? false, "and says so as Touch ID, not as a cancel")
    } catch {
      expect(false, "a key that needs Touch ID: \(error)")
    }
  } else {
    print("skip  no fingerprints on this Mac: a key that needs Touch ID")
  }
  // A PIN's record, sealed to a new key in the enclave and opened again,
  // through the calls the core makes.
  let account = pinText("account")
  let record = Data(pinText("record").utf8)
  var room = [UInt8](repeating: 0, count: 16 * 1024)
  let written = account.withCString { id in
    record.withUnsafeBytes { plain in
      room.withUnsafeMutableBufferPointer {
        DeviceSeal.sealCall(id, plain.bindMemory(to: UInt8.self).baseAddress, plain.count, $0.baseAddress, $0.count)
      }
    }
  }
  expect(written > 0, "the Secure Enclave seals a PIN's record")
  if written > 0 {
    let sealed = Data(room[0..<written])
    let kept = try? JSONDecoder().decode(DeviceSealRecord.self, from: sealed)
    expect(kept != nil && sealed.range(of: record) == nil, "what's kept is the enclave's record, not the PIN's")
    func open(_ sealed: Data, as account: String, room: Int = 1024) -> Data? {
      var out = [UInt8](repeating: 0, count: room)
      let count = account.withCString { id in
        sealed.withUnsafeBytes { input in
          out.withUnsafeMutableBufferPointer {
            DeviceSeal.openCall(id, input.bindMemory(to: UInt8.self).baseAddress, input.count, $0.baseAddress, $0.count)
          }
        }
      }
      return count < 0 ? nil : Data(out[0..<count])
    }
    expect(open(sealed, as: account) == record, "and opens it without a prompt")
    expect(open(sealed, as: pinText("otherAccount")) == nil, "but not as another account's")
    expect(open(sealed, as: account, room: record.count - 1) == nil, "nor into less room than it needs")
    if var changed = kept {
      changed.sealed[changed.sealed.count / 2] ^= 1
      expect(open((try? JSONEncoder().encode(changed)) ?? Data(), as: account) == nil, "nor once it's changed")
    }
    let again = account.withCString { id in
      record.withUnsafeBytes { plain in
        room.withUnsafeMutableBufferPointer {
          DeviceSeal.sealCall(id, plain.bindMemory(to: UInt8.self).baseAddress, plain.count, $0.baseAddress, $0.count)
        }
      }
    }
    let other = again > 0 ? try? JSONDecoder().decode(DeviceSealRecord.self, from: Data(room[0..<again])) : nil
    expect(other != nil && other?.enclaveKey != kept?.enclaveKey, "each seal has a new enclave key")
  }
} else {
  print("skip  no Secure Enclave here (a virtual machine): the real enclave")
}

// The core given the app's seal, as the Mac app gives it at start: a PIN's
// record through the core's own store, which only a test build lets this
// program ask for (pb_test_device_seal_round_trip). Nothing is kept.
if let core = loadedCore {
  let gave = core.giveDeviceSeal()
  expect(gave == SecureEnclave.isAvailable, "the core is given the seal only where there's a Secure Enclave")
  typealias RoundTrip = @convention(c) (UnsafePointer<CChar>, UnsafePointer<CChar>, UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?
  typealias Free = @convention(c) (UnsafeMutablePointer<CChar>?) -> Void
  if let tripAt = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "pb_test_device_seal_round_trip"),
    let freeAt = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "pb_free")
  {
    let trip = unsafeBitCast(tripAt, to: RoundTrip.self)
    let free = unsafeBitCast(freeAt, to: Free.self)
    let record = pinText("record")
    let answer = pinText("account").withCString { a in
      pinText("otherAccount").withCString { o in record.withCString { trip(a, o, $0) } }
    }
    let json = answer.flatMap { try? JSONSerialization.jsonObject(with: Data(String(cString: $0).utf8)) as? [String: Any] }
    free(answer)
    expect(json?["seals"] as? Bool == gave, "the core's store seals to the device once it's given the seal")
    if gave && !macLocked {
      let sealed = (json?["sealed"] as? String).flatMap { Data(base64Encoded: $0) }
      expect(sealed.flatMap { try? JSONDecoder().decode(DeviceSealRecord.self, from: $0) } != nil, "what the core would keep is the enclave's record")
      expect(json?["opened"] as? String == record, "and the core opens it again through the app's seal")
      expect(json?["asOther"] as? Bool == false, "but never as another account's")
    } else if !gave {
      expect(json?["sealed"] is NSNull, "with no seal the core seals nothing")
    }
  } else {
    print("skip  the core isn't a test build (its test-hooks feature): a PIN's record through the core")
  }
}

if failures > 0 {
  print("\(failures) failed")
  exit(1)
}
print("all passed")
