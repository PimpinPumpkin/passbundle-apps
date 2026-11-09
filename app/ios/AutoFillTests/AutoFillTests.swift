import AuthenticationServices
import SwiftUI
import XCTest

/// The AutoFill extension's own code (CredentialProviderViewController.swift
/// and Shared.swift, compiled in here with the core linked in, as the
/// extension has them), on an iPhone simulator. iOS hands a request to the
/// extension only in a build an Apple Developer account signs
/// (docs/phones.md, "Order and tests"), so these drive the extension's
/// Provider as its view controller does for iOS's requests, on two accounts
/// kept as the app keeps them: each unlocks on its own, the website's logins
/// are labeled with their account, a login fills only from its own account,
/// nothing in the app's folder changes, and every account locks as the
/// request ends; and a passkey signs from its own account, its answer the
/// core's test vectors' byte for byte. The screens are drawn in the host app's window, attached to
/// the results, and saved where PB_SHOTS says (tools/ios-autofill-screens.sh).
final class AutoFillTests: XCTestCase {
  /// The sample vault's master password, and its Backup service login
  /// (app/test/sample; the core's test vectors).
  static let password = "correct horse battery staple 1"
  static let backup = "5f607182-93a4-4b52-cde3-f40516273849"
  /// Two accounts, as the app names them on this device.
  static let work = "0123456789abcdef0123456789abcdef"
  static let personal = "fedcba9876543210fedcba9876543210"

  private var folder = ""
  private var index = ""

  override func setUpWithError() throws {
    continueAfterFailure = false
    folder = try XCTUnwrap(Shared.filesDir())
    let files = FileManager.default
    for name in (try? files.contentsOfDirectory(atPath: folder)) ?? [] {
      try files.removeItem(atPath: "\(folder)/\(name)")
    }
    // The sample vault, kept twice, as two accounts the app signed in to.
    let sample = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
      .appendingPathComponent("../../test/sample").standardizedFileURL
    let settings = try String(contentsOf: sample.appendingPathComponent("settings.json"), encoding: .utf8)
    let sync = try String(contentsOf: sample.appendingPathComponent("sync.json"), encoding: .utf8)
    for id in [Self.work, Self.personal] {
      let account = "\(folder)/accounts/\(id)"
      try files.createDirectory(atPath: account, withIntermediateDirectories: true)
      try settings.write(toFile: "\(account)/settings.json", atomically: true, encoding: .utf8)
      try sync.write(toFile: "\(account)/sync.json", atomically: true, encoding: .utf8)
      Shared.removeBiometric(id)
    }
    index = """
      {"device": "00000000-0000-4000-8000-000000000000", "active": "\(Self.work)", "accounts": [
        {"id": "\(Self.work)", "name": "Work", "tone": "emerald"},
        {"id": "\(Self.personal)", "name": "Personal", "tone": "amethyst"}]}
      """
    try index.write(toFile: "\(folder)/accounts.json", atomically: true, encoding: .utf8)
  }

  override func tearDown() {
    _ = Shared.answer(pb_autofill_close())
    for id in [Self.work, Self.personal] {
      Shared.removeBiometric(id)
    }
  }

  /// The extension's screens, in the host app's window, as iOS shows them.
  private func show(_ provider: Provider) throws {
    let window = try XCTUnwrap(
      UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow }.first)
    window.rootViewController = UIHostingController(rootView: ProviderView(provider: provider))
  }

  /// Waits for the screens and the core's answers (they come on the main queue).
  private func wait(_ what: String, timeout: TimeInterval = 30, until done: () -> Bool) {
    let end = Date().addingTimeInterval(timeout)
    while !done() && Date() < end {
      RunLoop.main.run(until: Date().addingTimeInterval(0.05))
    }
    XCTAssertTrue(done(), "Never \(what)")
  }

  private func isLocked(_ provider: Provider) -> Bool {
    if case .locked = provider.screen { return true }
    return false
  }

  private func isLogins(_ provider: Provider) -> Bool {
    if case .logins = provider.screen { return true }
    return false
  }

  /// The window as it is, attached to the results, and saved for CI's
  /// artifacts where PB_SHOTS says.
  private func snapshot(_ name: String) {
    RunLoop.main.run(until: Date().addingTimeInterval(0.8))
    guard
      let window = UIApplication.shared.connectedScenes.compactMap({ ($0 as? UIWindowScene)?.keyWindow }).first
    else { return }
    let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
      window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
    }
    let attachment = XCTAttachment(image: image)
    attachment.name = name
    attachment.lifetime = .keepAlways
    add(attachment)
    if let shots = ProcessInfo.processInfo.environment["PB_SHOTS"], let png = image.pngData() {
      try? FileManager.default.createDirectory(atPath: shots, withIntermediateDirectories: true)
      try? png.write(to: URL(fileURLWithPath: shots).appendingPathComponent("autofill-\(name).png"))
    }
  }

  func testUnlocksEachAccountOnItsOwnLabelsItsLoginsAndFillsFromItsOwnAccount() throws {
    let provider = Provider()
    var filled: ASPasswordCredential?
    var ended = false
    provider.onFinish = { answer in
      if case .password(let credential) = answer { filled = credential }
      ended = true
    }
    try show(provider)
    provider.start(domains: ["backup.example.test"], picked: nil)

    // All locked: it asks for the account the app shows, by name, with no
    // Face ID kept for it.
    wait("asked to unlock") { isLocked(provider) }
    XCTAssertEqual(provider.accounts.map(\.label), ["Work", "Personal"])
    XCTAssertEqual(provider.unlocking?.id, Self.work)
    XCTAssertNil(provider.biometric)
    snapshot("1-unlock")

    // A wrong master password says so, and stays.
    provider.unlock(password: "not the master password")
    wait("said the password was wrong") { isLocked(provider) && provider.note != nil }

    provider.unlock(password: Self.password)
    wait("listed the work account's logins") { isLogins(provider) }
    // The website's logins (by its registrable domain), from that account
    // only, with the personal account still locked.
    XCTAssertEqual(Set(provider.logins.map(\.name)), ["Backup service", "Example Mail"])
    XCTAssertEqual(Set(provider.logins.map(\.account)), [Self.work])
    XCTAssertEqual(provider.locked.map(\.id), [Self.personal])
    XCTAssertEqual(provider.label(Self.work), "Work")
    snapshot("2-logins")

    // The other account, unlocked on its own: both accounts' logins now,
    // each labeled with its own.
    provider.askToUnlock(provider.locked[0])
    wait("asked to unlock the personal account") { isLocked(provider) }
    XCTAssertEqual(provider.unlocking?.id, Self.personal)
    provider.unlock(password: Self.password)
    wait("listed both accounts' logins") { isLogins(provider) && provider.logins.count == 4 }
    XCTAssertEqual(Set(provider.logins.map(\.account)), [Self.work, Self.personal])
    XCTAssertTrue(provider.locked.isEmpty)
    snapshot("3-two-accounts")

    // Searching looks through every account unlocked here.
    provider.query = "mail"
    provider.search()
    XCTAssertEqual(
      Set(provider.logins.filter { $0.name == "Example Mail" }.map(\.account)), [Self.work, Self.personal])
    provider.query = ""
    provider.search()

    // Picking the personal account's login fills it from that account.
    let login = try XCTUnwrap(
      provider.logins.first { $0.account == Self.personal && $0.name == "Backup service" })
    provider.pick(login)
    wait("filled") { ended }
    XCTAssertEqual(filled?.user, "backup@example.test")
    XCTAssertEqual(filled?.password, "not-a-real-password-3")

    // The request is over: every account locked and let go, and the app's
    // folder as the app left it.
    let after = Shared.answer(pb_fillable("backup.example.test")) as? [String: Any]
    XCTAssertEqual(after?["locked"] as? Bool, true)
    XCTAssertEqual(try String(contentsOfFile: "\(folder)/accounts.json", encoding: .utf8), index)
  }

  /// A suggestion picked above the keyboard (`<account>/<item>`): that
  /// account's own Face ID item unlocks it, with nothing to type, and that
  /// login fills.
  func testAPickedSuggestionUnlocksItsAccountWithItsFaceIDItemAndFills() throws {
    // The personal account's key kept as the app keeps it for Face ID, with
    // debug builds' stand-in for the prompt: no access control, so no prompt.
    _ = Shared.answer(pb_autofill_open(folder))
    let unlocked = Shared.answer(pb_unlock(Self.personal, Self.password)) as? [String: Any]
    XCTAssertEqual(unlocked?["ok"] as? Bool, true)
    var length = 0
    let key = try XCTUnwrap(pb_user_key(Self.personal, &length))
    var item = Shared.biometricItem(Self.personal)
    item[kSecValueData as String] = Data(bytes: key, count: length)
    item[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
    pb_free_secret(key, length)
    XCTAssertEqual(SecItemAdd(item as CFDictionary, nil), errSecSuccess)
    _ = Shared.answer(pb_autofill_close())
    XCTAssertTrue(Shared.biometricKept(Self.personal))
    XCTAssertFalse(Shared.biometricKept(Self.work))

    let provider = Provider()
    var filled: ASPasswordCredential?
    provider.onFinish = { answer in
      if case .password(let credential) = answer { filled = credential }
    }
    try show(provider)
    provider.start(domains: ["backup.example.test"], picked: "\(Self.personal)/\(Self.backup)")
    wait("filled the picked login") { filled != nil }
    XCTAssertEqual(filled?.user, "backup@example.test")
    XCTAssertEqual(filled?.password, "not-a-real-password-3")
    // Only the picked suggestion's account was unlocked.
    XCTAssertEqual(provider.accounts.filter(\.unlocked).map(\.id), [Self.personal])
  }

  /// A site asking for a passkey (as iOS's passkey sheet hands it over):
  /// the site's passkeys from each account unlocked here, and picking one
  /// signs the site's hash with it, from its own account. The sample's
  /// passkey and its answer are the core's vectors (core/tests/vectors,
  /// "passkeyAssertions"); unlocking here is the proof the site asked for.
  func testAPasskeySignsFromItsOwnAccountAsTheVectorsSay() throws {
    let provider = Provider()
    var signed: ASPasskeyAssertionCredential?
    provider.onFinish = { answer in
      if case .signedIn(let credential) = answer { signed = credential }
    }
    try show(provider)
    let hash = try XCTUnwrap(Data(base64Encoded: "XKejityl+6pXSQZTJkFlB9itgiV5zYYy9GZ/SvZZWsw="))
    provider.signIn = Provider.PasskeySignIn(rpId: "backup.example.test", clientDataHash: hash, allowed: [])
    provider.start(domains: ["backup.example.test"], picked: nil)
    wait("asked to unlock") { isLocked(provider) }
    provider.unlock(password: Self.password)
    wait("listed the site's passkey") { isLogins(provider) && !provider.passkeys.isEmpty }
    XCTAssertEqual(provider.passkeys.map(\.name), ["Backup service"])
    XCTAssertEqual(provider.passkeys.map(\.account), [Self.work])
    XCTAssertEqual(provider.passkeys.map(\.username), ["backup@example.test"])
    snapshot("4-passkey")

    provider.pickPasskey(provider.passkeys[0])
    wait("signed") { signed != nil }
    XCTAssertEqual(signed?.relyingParty, "backup.example.test")
    XCTAssertEqual(signed?.clientDataHash, hash)
    XCTAssertEqual(signed?.credentialID.base64EncodedString(), "DC9qjk0bTzqefFuNKh9uQA==")
    XCTAssertEqual(signed?.userHandle.base64EncodedString(), "oKGio6SlpqeoqaqrrK2urw==")
    XCTAssertEqual(
      signed?.authenticatorData.base64EncodedString(), "nh7vJ4KxrtUibcKd8eJn5gsIbkWJD4xOEHkA/LT0nzkdAAAAAA==")
    XCTAssertEqual(
      signed?.signature.base64EncodedString(),
      "MEYCIQDlKsQiHkf2H+IojpwLSgfI1KvTjEW6TN/cQqv9gJz83gIhAOxmswPGHHoBLgawOa7+hzh2ieh91G1wdcE/C5jEcbR+")
  }

  /// A site making a passkey, with two accounts unlocked: it names the site
  /// and the account on it, and asks which PassBundle account it goes in.
  /// (Saving it goes to the server, which this test has none of.)
  func testMakingAPasskeyAsksWhichAccountItGoesIn() throws {
    let provider = Provider()
    try show(provider)
    provider.making = Provider.PasskeyMaking(
      rpId: "new.example.test", userName: "ana@new.example.test", userHandle: Data([1, 2, 3, 4]),
      clientDataHash: Data(repeating: 7, count: 32), excluded: [])
    provider.start(domains: [], picked: nil)
    wait("asked to unlock") { isLocked(provider) }
    provider.unlock(password: Self.password)
    wait("asked to save it") {
      if case .saving = provider.screen { return true }
      return false
    }
    XCTAssertEqual(provider.saveIn, Self.work)
    provider.askToUnlock(provider.locked[0])
    wait("asked to unlock the personal account") { isLocked(provider) }
    provider.unlock(password: Self.password)
    wait("asked to save it again") {
      if case .saving = provider.screen { return true }
      return false
    }
    // Still where it was going, with the other account to choose now.
    XCTAssertEqual(provider.saveIn, Self.work)
    XCTAssertTrue(provider.locked.isEmpty)
    snapshot("5-save-passkey")
  }

  /// With no account signed in to in the app, it says where to start.
  func testWithNoAccountItSaysToSignInInTheApp() throws {
    try FileManager.default.removeItem(atPath: "\(folder)/accounts.json")
    try FileManager.default.removeItem(atPath: "\(folder)/accounts")
    let provider = Provider()
    try show(provider)
    provider.start(domains: ["backup.example.test"], picked: nil)
    guard case .unavailable(let words) = provider.screen else {
      return XCTFail("Not unavailable: \(provider.screen)")
    }
    XCTAssertEqual(words, "Sign in to PassBundle in its app first.")
  }
}
