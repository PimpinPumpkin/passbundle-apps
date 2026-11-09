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
/// request ends; a passkey signs from its own account, its answer the core's
/// test vectors' byte for byte; a login search finds for a site that isn't
/// its own asks before it fills; and nothing begun for one request lands in
/// the next. The screens are drawn in the host app's window, attached to the
/// results, and saved where PB_SHOTS says (tools/ios-autofill-screens.sh).
final class AutoFillTests: XCTestCase {
  /// The sample vault's master password, and its Backup service login
  /// (app/test/sample; the core's test vectors).
  static let password = "correct horse battery staple 1"
  static let backup = "5f607182-93a4-4b52-cde3-f40516273849"
  /// Two accounts, as the app names them on this device.
  static let work = "0123456789abcdef0123456789abcdef"
  static let personal = "fedcba9876543210fedcba9876543210"
  /// How each of the browser extension's "Site doesn't match" questions ends
  /// (its `loginSingleSiteDesc` and the ones beside it).
  static let verify = "Verify you're on a trusted site before autofilling."

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

  /// The host app's window. Tests run on the main thread, so on the main
  /// actor, where UIKit is.
  private func window() -> UIWindow? {
    MainActor.assumeIsolated {
      UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow }.first
    }
  }

  /// The extension's screens, in the host app's window, as iOS shows them.
  private func show(_ provider: Provider) throws {
    let window = try XCTUnwrap(window(), "The host app has no window")
    MainActor.assumeIsolated {
      window.rootViewController = UIHostingController(rootView: ProviderView(provider: provider))
    }
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
    guard let window = window() else { return }
    let image = MainActor.assumeIsolated {
      UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
        window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
      }
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

    // Asked to unlock the other account, the person can go back to the
    // logins there are.
    provider.askToUnlock(provider.locked[0])
    wait("asked to unlock the personal account") { isLocked(provider) }
    provider.backToLogins()
    wait("went back to the logins") { isLogins(provider) }
    XCTAssertEqual(provider.logins.count, 2)

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

  /// A login search finds, for a site that isn't its own: it asks first, in
  /// the browser extension's words, with the login's website and this one,
  /// and fills only once the person says so. The site's own logins fill as
  /// they're picked (the first test).
  func testALoginFoundBySearchAsksBeforeItFillsOnASiteThatIsNotItsOwn() throws {
    let provider = Provider()
    var filled: ASPasswordCredential?
    var ended = false
    provider.onFinish = { answer in
      if case .password(let credential) = answer { filled = credential }
      ended = true
    }
    try show(provider)
    // Another site altogether: none of the sample's logins is its own.
    provider.start(domains: ["shop.example.org"], picked: nil)
    wait("asked to unlock") { isLocked(provider) }
    provider.unlock(password: Self.password)
    wait("listed the site's logins") { isLogins(provider) }
    XCTAssertTrue(provider.logins.isEmpty)

    provider.query = "backup"
    provider.search()
    let login = try XCTUnwrap(provider.logins.first { $0.name == "Backup service" })
    provider.pick(login)
    guard case .elsewhere(let asked) = provider.screen else {
      return XCTFail("Didn't ask: \(provider.screen)")
    }
    XCTAssertFalse(ended)
    XCTAssertEqual(asked.login, login)
    XCTAssertEqual(asked.title, "Site doesn't match")
    XCTAssertEqual(
      asked.words, "This site doesn't match the one you have saved for this login. \(Self.verify)")
    XCTAssertEqual(asked.savedHeading, "Saved website")
    XCTAssertEqual(asked.saved, ["backup.example.test"])
    XCTAssertEqual(asked.current, "shop.example.org")
    snapshot("6-site-doesnt-match")

    // Back to the logins: nothing filled, and still what was searched for.
    provider.backToLogins()
    XCTAssertTrue(isLogins(provider))
    XCTAssertFalse(ended)
    XCTAssertEqual(provider.logins.map(\.name), ["Backup service"])

    // A login with no website saved says that instead.
    provider.query = "wi-fi"
    provider.search()
    let wifi = try XCTUnwrap(provider.logins.first { $0.name == "Office Wi-Fi" })
    provider.pick(wifi)
    guard case .elsewhere(let unsaved) = provider.screen else {
      return XCTFail("Didn't ask: \(provider.screen)")
    }
    XCTAssertEqual(unsaved.title, "Login has no site saved")
    XCTAssertEqual(unsaved.words, "This login has no website saved yet. \(Self.verify)")
    XCTAssertTrue(unsaved.saved.isEmpty)
    XCTAssertEqual(unsaved.current, "shop.example.org")
    XCTAssertFalse(ended)
    provider.backToLogins()

    // "Autofill" on the question fills it, from its own account.
    provider.pick(login, anyway: true)
    wait("filled") { ended }
    XCTAssertEqual(filled?.user, "backup@example.test")
    XCTAssertEqual(filled?.password, "not-a-real-password-3")
  }

  /// The question's other wordings, the browser extension's too: several
  /// websites saved, and a website that is this very site under a rule that
  /// doesn't fill here. And a saved website's host, however it was written.
  func testTheSiteQuestionSaysWhatTheBrowserExtensionSays() {
    let login = Provider.Login(
      account: Self.work, item: Self.backup, name: "Backup service", username: "backup@example.test")
    let several = Provider.Elsewhere(
      login: login, saved: ["backup.example.test", "files.example.test"], current: "shop.example.org",
      sameSite: false)
    XCTAssertEqual(several.title, "Site doesn't match")
    XCTAssertEqual(
      several.words, "This site doesn't match the ones you have saved for this login. \(Self.verify)")
    XCTAssertEqual(several.savedHeading, "Saved websites ( 2 )")

    let never = Provider.Elsewhere(
      login: login, saved: ["shop.example.org"], current: "shop.example.org", sameSite: true)
    XCTAssertEqual(never.title, "Login doesn't match")
    XCTAssertEqual(never.words, "This site doesn't match your saved login details. \(Self.verify)")

    XCTAssertEqual(Provider.host("https://mail.example.test/login"), "mail.example.test")
    XCTAssertEqual(Provider.host("backup.example.test"), "backup.example.test")
    XCTAssertEqual(Provider.host("Shop.Example.org:8443/cart"), "shop.example.org")
  }

  /// A request's number is good from when it begins until it ends, and never
  /// again: what was begun under it isn't run afterwards, and closing it
  /// twice, or after the next request began, closes nothing.
  func testWhatAnEndedRequestBeganIsNotRunAndClosesNothing() {
    let requests = Requests()
    var opened = 0
    var closed = 0
    var ran = 0
    let first = requests.begin { opened += 1 }
    XCTAssertEqual(opened, 1)
    XCTAssertTrue(requests.has(first))
    XCTAssertEqual(requests.run(for: first) { ran += 1 }.map { "ran" }, "ran")
    XCTAssertEqual(ran, 1)

    requests.end(first) { closed += 1 }
    XCTAssertEqual(closed, 1)
    XCTAssertFalse(requests.has(first))
    XCTAssertNil(requests.run(for: first) { ran += 1 }.map { "ran" })
    XCTAssertEqual(ran, 1)
    requests.end(first) { closed += 1 }
    XCTAssertEqual(closed, 1)

    // A request that never ended, with the next one begun: the next one's
    // from then on, whatever the first still does.
    let second = requests.begin()
    let third = requests.begin()
    XCTAssertFalse(second == third)
    XCTAssertFalse(requests.has(second))
    XCTAssertNil(requests.run(for: second) { ran += 1 }.map { "ran" })
    requests.end(second) { closed += 1 }
    XCTAssertEqual(ran, 1)
    XCTAssertEqual(closed, 1)
    XCTAssertTrue(requests.has(third))
    XCTAssertEqual(requests.run(for: third) { ran += 1 }.map { "ran" }, "ran")
    requests.end(third) { closed += 1 }
    XCTAssertEqual(closed, 2)
  }

  /// A core call that's running as its request ends finishes first: the
  /// request's close (and so the next request's open) waits for it.
  func testEndingARequestWaitsForItsRunningCall() {
    final class Order: @unchecked Sendable {
      var seen: [String] = []
    }
    let requests = Requests()
    let order = Order()
    let request = requests.begin()
    let running = DispatchSemaphore(value: 0)
    let letGo = DispatchSemaphore(value: 0)
    let both = DispatchGroup()
    DispatchQueue.global().async(group: both) {
      _ = requests.run(for: request) {
        running.signal()
        letGo.wait()
        order.seen.append("call")
      }
    }
    running.wait()
    DispatchQueue.global().async(group: both) {
      requests.end(request) { order.seen.append("close") }
    }
    // Time for the close to get as far as it can with the call running.
    Thread.sleep(forTimeInterval: 0.3)
    XCTAssertTrue(order.seen.isEmpty)
    letGo.signal()
    XCTAssertTrue(both.wait(timeout: .now() + 30) == .success)
    XCTAssertEqual(order.seen, ["call", "close"])
    XCTAssertFalse(requests.has(request))
  }

  /// A master password typed for a request that ended before the core got
  /// to it: the next request, in the same process, starts with every account
  /// locked and stays so, and nothing answers iOS a second time.
  func testAnUnlockLeftFromAnEndedRequestDoesNothingInTheNextOne() throws {
    let provider = Provider()
    // The core's slow calls wait here until the test lets them go.
    let held = DispatchQueue(label: "com.passbundle.tests.held")
    held.suspend()
    var resumed = false
    defer { if !resumed { held.resume() } }
    provider.queue = held
    var answers = 0
    provider.onFinish = { _ in answers += 1 }
    try show(provider)
    provider.start(domains: ["backup.example.test"], picked: nil)
    wait("asked to unlock") { isLocked(provider) }
    provider.unlock(password: Self.password)
    XCTAssertTrue(provider.working)
    // Cancel, and the next request begins, before the unlock has run.
    provider.finish(.refused(.userCanceled))
    XCTAssertEqual(answers, 1)
    provider.start(domains: ["backup.example.test"], picked: nil)
    wait("asked to unlock again") { isLocked(provider) }

    resumed = true
    held.resume()
    held.sync {}
    RunLoop.main.run(until: Date().addingTimeInterval(0.5))
    // The unlock was never made: the new request's accounts are all locked.
    XCTAssertTrue(isLocked(provider))
    XCTAssertTrue(provider.accounts.allSatisfy { !$0.unlocked })
    let now = Shared.answer(pb_fillable("backup.example.test")) as? [String: Any]
    XCTAssertEqual(now?["locked"] as? Bool, true)
    XCTAssertEqual(now?["signedOut"] as? Bool, false)
    XCTAssertEqual(answers, 1)

    // The new request goes on as any other.
    provider.unlock(password: Self.password)
    wait("listed the logins") { isLogins(provider) }
    XCTAssertEqual(provider.logins.count, 2)
  }

  /// A sheet that goes away late, after the next request has begun (iOS
  /// tells it so when it likes): its close leaves the next request's
  /// accounts as they are.
  func testAnEndedRequestsLateCloseLeavesTheNextRequestOpen() throws {
    let earlier = Provider()
    earlier.start(domains: ["backup.example.test"], picked: nil)
    wait("asked to unlock") { isLocked(earlier) }

    let provider = Provider()
    try show(provider)
    provider.start(domains: ["backup.example.test"], picked: nil)
    wait("asked to unlock") { isLocked(provider) }
    provider.unlock(password: Self.password)
    wait("listed the logins") { isLogins(provider) }

    earlier.close()
    let now = Shared.answer(pb_fillable("backup.example.test")) as? [String: Any]
    XCTAssertEqual(now?["locked"] as? Bool, false)

    // Its own close locks them.
    provider.close()
    let after = Shared.answer(pb_fillable("backup.example.test")) as? [String: Any]
    XCTAssertEqual(after?["locked"] as? Bool, true)
  }
}
