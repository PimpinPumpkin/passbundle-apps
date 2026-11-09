// Touch ID on the Mac (app/macos/Runner/TouchID.swift) on its own: built
// with this file by tools/mac-touch-id-test.sh. The seal against the meta
// repo's vectors (a software key standing in for the Secure Enclave's), the
// opened user key's one buffer and its zeroing, the account's file and its
// modes, the core's C functions as the app finds them, and, where this Mac
// has a Secure Enclave, the real one: a key it keeps seals and opens a user
// key, and one that needs Touch ID won't open it without. Never shows a
// prompt.
//
//   touch-id-test <mac-local-v1.json> [<the core, built with its ffi feature>]
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
guard arguments.count >= 2,
  let json = try? JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: arguments[1])))
    as? [String: Any],
  let v = json["touchId"] as? [String: Any]
else {
  print("touch-id-test <mac-local-v1.json> [<core library>]")
  exit(2)
}
func bytes(_ name: String) -> Data { Data(base64Encoded: v[name] as! String)! }
func text(_ name: String) -> String { v[name] as! String }

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
if arguments.count >= 3 {
  if dlopen(arguments[2], RTLD_NOW | RTLD_GLOBAL) == nil {
    expect(false, "the core loads: \(String(cString: dlerror()))")
  } else if let core = LoadedCore.find() {
    expect(true, "the app finds the core's functions for Touch ID")
    let account = text("account")
    expect(core.withUserKey(account, { _ in true }) == nil, "no user key for an account that isn't open")
    expect(!core.give(account, key: secret(bytes("userKey")), forApproval: false).isEmpty, "a key for an account that isn't here isn't taken")
    expect(!core.give(account, key: secret(bytes("userKey")), forApproval: true).isEmpty, "nor is one to approve with")
  } else {
    expect(false, "the app finds the core's functions for Touch ID")
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
} else {
  print("skip  no Secure Enclave here (a virtual machine): the real enclave")
}

if failures > 0 {
  print("\(failures) failed")
  exit(1)
}
print("all passed")
