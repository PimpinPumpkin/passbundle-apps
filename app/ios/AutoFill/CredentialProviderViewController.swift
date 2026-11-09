import AuthenticationServices
import SwiftUI

/// PassBundle in iOS's AutoFill (docs/phones.md, "Autofill" and "Passkeys"):
/// a separate process with a tight memory limit, so no Flutter. Its few screens are our
/// own, in SwiftUI, around the same core, linked in. It reads the vault the
/// app keeps in the App Group, beside the app and without changing anything
/// there, and unlocks on its own (Face ID or Touch ID, or the master
/// password), since it can't share the app's memory. With several accounts,
/// it lists the logins of each account unlocked here, labeled, and fills a
/// login only from its own account. A password is taken from the core only
/// once someone picks its login, a passkey signs only once someone picks it,
/// and every account locks as the request ends, however it ends. Nothing
/// begun for one request reaches the next (`Requests`), and a login found by
/// search fills on a site that isn't its own only once the person says so
/// (`Provider.Elsewhere`).
class CredentialProviderViewController: ASCredentialProviderViewController {
  private let provider = Provider()

  override func viewDidLoad() {
    super.viewDidLoad()
    // A kept key that no longer opens its account's vault: the core turns
    // that account's Face ID off, as the app does.
    pb_set_remove_biometric { account in
      guard let account else { return }
      Shared.removeBiometric(String(cString: account))
    }
    provider.onFinish = { [weak self] answer in
      guard let context = self?.extensionContext else { return }
      switch answer {
      case .password(let credential):
        context.completeRequest(withSelectedCredential: credential, completionHandler: nil)
      case .signedIn(let credential):
        context.completeAssertionRequest(using: credential, completionHandler: nil)
      case .made(let credential):
        context.completeRegistrationRequest(using: credential, completionHandler: nil)
      case .refused(let code):
        context.cancelRequest(withError: NSError(domain: ASExtensionErrorDomain, code: code.rawValue))
      }
    }
    let screens = UIHostingController(rootView: ProviderView(provider: provider))
    addChild(screens)
    screens.view.frame = view.bounds
    screens.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    view.addSubview(screens.view)
    screens.didMove(toParent: self)
  }

  /// However the sheet goes away (Cancel, a fill, or swiped down), nothing
  /// decrypted stays in this process, which iOS may keep for the next request.
  override func viewDidDisappear(_ animated: Bool) {
    super.viewDidDisappear(animated)
    provider.close()
  }

  /// The logins for the website someone is signing in to.
  override func prepareCredentialList(for serviceIdentifiers: [ASCredentialServiceIdentifier]) {
    provider.start(domains: serviceIdentifiers.compactMap(Provider.domain), picked: nil)
  }

  /// A site asking for a passkey (and maybe a password): its passkeys from
  /// each account unlocked here, then its logins. iOS has checked the site's
  /// ID against the page or the app.
  override func prepareCredentialList(
    for serviceIdentifiers: [ASCredentialServiceIdentifier],
    requestParameters: ASPasskeyCredentialRequestParameters
  ) {
    provider.signIn = Provider.PasskeySignIn(
      rpId: requestParameters.relyingPartyIdentifier,
      clientDataHash: requestParameters.clientDataHash,
      allowed: requestParameters.allowedCredentials)
    provider.start(domains: serviceIdentifiers.compactMap(Provider.domain), picked: nil)
  }

  /// A suggestion picked from iOS's own list: never filled or signed without
  /// the person, since every account is locked in this process.
  override func provideCredentialWithoutUserInteraction(for credentialRequest: ASCredentialRequest) {
    extensionContext.cancelRequest(
      withError: NSError(domain: ASExtensionErrorDomain, code: ASExtensionError.userInteractionRequired.rawValue))
  }

  /// A suggestion picked from iOS's own list, once its account is unlocked:
  /// that login, or that passkey.
  override func prepareInterfaceToProvideCredential(for credentialRequest: ASCredentialRequest) {
    if let request = credentialRequest as? ASPasskeyCredentialRequest,
      let identity = request.credentialIdentity as? ASPasskeyCredentialIdentity
    {
      provider.signIn = Provider.PasskeySignIn(
        rpId: identity.relyingPartyIdentifier, clientDataHash: request.clientDataHash,
        allowed: [identity.credentialID])
      provider.start(domains: [], picked: identity.recordIdentifier)
      return
    }
    let identity = credentialRequest.credentialIdentity
    provider.start(
      domains: [Provider.domain(identity.serviceIdentifier)].compactMap { $0 },
      picked: identity.recordIdentifier)
  }

  /// A site making a passkey: saved in a new login for the site, in the
  /// account the person picks among those unlocked here, once they say so.
  override func prepareInterface(forPasskeyRegistration registrationRequest: ASCredentialRequest) {
    guard let request = registrationRequest as? ASPasskeyCredentialRequest,
      let identity = request.credentialIdentity as? ASPasskeyCredentialIdentity
    else {
      return provider.finish(.refused(.failed))
    }
    // ES256 is the only kind the core makes: a site that doesn't take it is
    // told so before anyone unlocks.
    guard request.supportedAlgorithms.contains(ASCOSEAlgorithmIdentifier.ES256) else {
      return provider.finish(.refused(.failed))
    }
    // iOS 17 doesn't give the site's own list of the credentials it has.
    var excluded: [Data] = []
    if #available(iOS 18.0, *) {
      excluded = request.excludedCredentials?.map(\.credentialID) ?? []
    }
    provider.making = Provider.PasskeyMaking(
      rpId: identity.relyingPartyIdentifier, userName: identity.userName, userHandle: identity.userHandle,
      clientDataHash: request.clientDataHash, excluded: excluded)
    provider.start(domains: [], picked: nil)
  }
}

/// Which AutoFill request the core is open for. The core's accounts are the
/// whole process's: opened as a request starts, let go as it ends, and iOS
/// may keep the process for the next request. So nothing begun for one
/// request may land in the next: each request has a number, and whatever
/// comes back later (from another queue, from Face ID, from iOS) carries the
/// number it started under and does nothing once that request has ended.
final class Requests: @unchecked Sendable {
  /// The process's, as the core's accounts are.
  static let core = Requests()

  private let lock = NSLock()
  /// The request the core is open for. Once it ends, its number is never
  /// anyone's again.
  private var front = 0

  /// Starts a request, ending the one before it: `open` opens the core for
  /// it, with none of an earlier request's core calls running. Its number.
  func begin(_ open: () -> Void = {}) -> Int {
    lock.lock()
    defer { lock.unlock() }
    front += 1
    open()
    return front
  }

  /// Ends `request` if it's still the one the core is open for: `close`
  /// closes the core, with none of the request's core calls running. One
  /// that has ended already, or that another has taken the place of, closes
  /// nothing.
  func end(_ request: Int, _ close: () -> Void = {}) {
    lock.lock()
    defer { lock.unlock() }
    guard request == front else { return }
    front += 1
    close()
  }

  /// Whether `request` is still the one the core is open for.
  func has(_ request: Int) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return request == front
  }

  /// A core call for `request`, on another queue: not made (nil) once the
  /// request has ended. Starting and ending a request wait for one that's
  /// running, so it never lands in the next request's accounts. `call`
  /// mustn't start or end a request itself.
  func run<T>(for request: Int, _ call: () -> T) -> T? {
    lock.lock()
    defer { lock.unlock() }
    return request == front ? call() : nil
  }
}

/// What the extension's screens show, and what they do with the core.
final class Provider: ObservableObject {
  enum Screen {
    case opening
    /// The core unlocking or checking a master password (the KDF), or
    /// saving a passkey or signing, over the network maybe.
    case working
    /// An account to unlock (`unlocking`).
    case locked
    case logins
    /// A login that asks for the master password again, before it fills or
    /// signs (docs/core.md, "Asking for the master password again").
    case reprompt(Login)
    /// A login found by search, picked for a site that isn't its own: it
    /// fills once the person says so.
    case elsewhere(Elsewhere)
    /// Saving a new passkey, once the person says so.
    case saving
    case unavailable(String)
  }

  /// How the extension ends: with what iOS asked for, or not.
  enum Answer {
    case password(ASPasswordCredential)
    case signedIn(ASPasskeyAssertionCredential)
    case made(ASPasskeyRegistrationCredential)
    case refused(ASExtensionError.Code)
  }

  /// A site's passkey sign-in: its ID, the hash of what it signs, and the
  /// credentials it names (none: any it can find by itself).
  struct PasskeySignIn {
    let rpId: String
    let clientDataHash: Data
    let allowed: [Data]
  }

  /// A site making a passkey: its ID, the account, and the credentials the
  /// vault mustn't already have (iOS 18 and later give them).
  struct PasskeyMaking {
    let rpId: String
    let userName: String
    let userHandle: Data
    let clientDataHash: Data
    let excluded: [Data]
  }

  /// An account signed in to in the app.
  struct Account: Identifiable, Equatable {
    let id: String
    /// Its name here, or its email.
    let label: String
    /// Its color's name (core/src/accounts.rs, `TONES`).
    let tone: String
    var unlocked = false
  }

  struct Login: Identifiable, Equatable {
    /// The account's id and the item's, together: two accounts can hold the
    /// same item (an organization's) under one id.
    var id: String { "\(account)/\(item)" }
    let account: String
    let item: String
    let name: String
    let username: String
    /// Asks for the master password again before it fills.
    var asks = false
    /// A passkey, rather than a password to fill.
    var passkey = false
  }

  /// A login picked for a site that isn't one of its own, which only search
  /// finds here: what the person is shown before it fills, in the words the
  /// browser extension asks with there. The extension can also save the site
  /// in the login; AutoFill changes nothing in a login, so it has the one
  /// button the extension shows someone who can't edit the login.
  struct Elsewhere: Equatable {
    let login: Login
    /// The login's websites, by host (as saved, where one has no host).
    let saved: [String]
    /// The site iOS asked for, if it named one.
    let current: String?
    /// One of the login's websites is this very site, under a rule that
    /// doesn't fill here: never, or one that needs the page's whole address
    /// (core/src/matching.rs).
    let sameSite: Bool

    var title: String {
      if saved.isEmpty { return "Login has no site saved" }
      return sameSite ? "Login doesn't match" : "Site doesn't match"
    }

    var words: String {
      let verify = "Verify you're on a trusted site before autofilling."
      if saved.isEmpty { return "This login has no website saved yet. \(verify)" }
      if sameSite { return "This site doesn't match your saved login details. \(verify)" }
      let saved = saved.count == 1 ? "the one" : "the ones"
      return "This site doesn't match \(saved) you have saved for this login. \(verify)"
    }

    var savedHeading: String {
      saved.count == 1 ? "Saved website" : "Saved websites ( \(saved.count) )"
    }
  }

  @Published var screen = Screen.opening
  @Published var accounts: [Account] = []
  /// The account the lock screen unlocks.
  @Published var unlocking: Account?
  @Published var logins: [Login] = []
  /// What the person searches for, among all their logins, when the
  /// website's aren't what they want.
  @Published var query = ""
  @Published var note: String?
  /// "Face ID" or "Touch ID", when the account being unlocked has it on.
  @Published var biometric: String?
  /// The site's passkeys, when it asks for one.
  @Published var passkeys: [Login] = []
  /// The account a new passkey goes in: one unlocked here.
  @Published var saveIn: String?
  var signIn: PasskeySignIn?
  var making: PasskeyMaking?
  /// What ends the extension (the view controller's).
  var onFinish: (Answer) -> Void = { _ in }
  /// Ended: nothing answers iOS twice, or after Cancel.
  private(set) var finished = false
  /// Where the core's slow calls run (the KDF, or one that waits on the
  /// network). The tests hold it back, to end a request with one waiting.
  var queue = DispatchQueue.global(qos: .userInitiated)

  private let requests = Requests.core
  /// This request's number, from `start` until it ends (`Requests`).
  private var request: Int?
  private var domains: [String] = []
  /// A suggestion picked above the keyboard: `<account>/<item>`.
  private var picked: (account: String, item: String)?
  private var matching: [Login] = []
  private var confirmed: Set<String> = []
  /// The accounts the person proved who they are for in this request: each
  /// unlocks here only with its Face ID or master password, a passkey's
  /// proof that it's them.
  private var verified: Set<String> = []

  /// Whether the core is saving or signing: Cancel waits for it.
  var working: Bool {
    if case .working = screen { return true }
    return false
  }

  /// The domain a service identifier names (a website's, or its address's host).
  static func domain(_ service: ASCredentialServiceIdentifier) -> String? {
    switch service.type {
    case .domain: return service.identifier
    case .URL: return URL(string: service.identifier)?.host
    @unknown default: return nil
    }
  }

  /// With more than one account: what an account's logins are labeled with.
  func label(_ account: String) -> String? {
    guard accounts.count > 1 else { return nil }
    return accounts.first { $0.id == account }?.label
  }

  func tone(_ account: String) -> String {
    accounts.first { $0.id == account }?.tone ?? "sapphire"
  }

  /// Opens the kept accounts, all locked, and asks to unlock the one the
  /// app shows (or a picked suggestion's): Face ID or Touch ID at once where
  /// it's on, else its master password.
  func start(domains: [String], picked: String?) {
    finished = false
    confirmed = []
    verified = []
    self.domains = domains
    self.picked = picked.flatMap { record in
      let parts = record.split(separator: "/", maxSplits: 1).map(String.init)
      return parts.count == 2 ? (parts[0], parts[1]) : nil
    }
    guard let dir = Shared.filesDir() else {
      screen = .unavailable("PassBundle's folder isn't there.")
      return
    }
    var opened: [String: Any]?
    request = requests.begin { opened = Shared.answer(pb_autofill_open(dir)) as? [String: Any] }
    accounts = ((opened?["accounts"] as? [[String: Any]]) ?? []).compactMap { a in
      guard let id = a["id"] as? String else { return nil }
      let name = (a["name"] as? String).flatMap { $0.isEmpty ? nil : $0 }
      return Account(
        id: id, label: name ?? a["email"] as? String ?? "", tone: a["tone"] as? String ?? "sapphire")
    }
    guard !accounts.isEmpty else {
      screen = .unavailable("Sign in to PassBundle in its app first.")
      return
    }
    let shown = opened?["active"] as? String
    let first =
      accounts.first { $0.id == self.picked?.account } ?? accounts.first { $0.id == shown } ?? accounts[0]
    askToUnlock(first, automatically: true)
  }

  /// The lock screen for an account; Face ID or Touch ID at once, where it's
  /// on and `automatically`.
  func askToUnlock(_ account: Account, automatically: Bool = false) {
    unlocking = account
    note = nil
    biometric = Shared.biometricKept(account.id) ? Shared.biometricMethod() : nil
    screen = .locked
    if automatically && biometric != nil { unlockWithBiometric() }
  }

  /// A slow core call for this request, off the main queue, with the
  /// screens waiting: `then` gets its answer on the main queue. Once the
  /// request has ended (`finish` or `close` has run, or another request has
  /// started), a call still waiting isn't made, and one that comes back
  /// does nothing more.
  private func work<T>(_ call: @escaping () -> T, then: @escaping (T) -> Void) {
    guard let request else { return }
    screen = .working
    queue.async { [requests] in
      let answer = requests.run(for: request, call)
      DispatchQueue.main.async {
        guard let answer, requests.has(request) else { return }
        then(answer)
      }
    }
  }

  /// Face ID can be up for a while: if the request ends under it, the key
  /// it hands over goes unused, and nothing more happens (`wanted`).
  func unlockWithBiometric() {
    guard let account = unlocking?.id, let request else { return }
    let requests = self.requests
    Shared.unlockWithBiometric(account: account, wanted: { requests.has(request) }) { [weak self] unlocked, error in
      guard let self, requests.has(request) else { return }
      if unlocked { return self.unlocked(account) }
      self.note = error
      if !Shared.biometricKept(account) { self.biometric = nil }
    }
  }

  /// The master password comes as a Swift string, which can't be wiped:
  /// the screens clear their field as they hand it over, it's kept here
  /// only until the core has answered, and the copy the core makes is the
  /// core's to wipe, which it does (`pb_unlock`).
  func unlock(password: String) {
    guard let account = unlocking?.id else { return }
    work({ Shared.answer(pb_unlock(account, password)) as? [String: Any] }) { answer in
      if answer?["ok"] as? Bool == true { return self.unlocked(account) }
      self.screen = .locked
      self.note = answer?["error"] as? String ?? "That's not the master password."
    }
  }

  private func unlocked(_ account: String) {
    if let at = accounts.firstIndex(where: { $0.id == account }) {
      accounts[at].unlocked = true
    }
    verified.insert(account)
    show()
  }

  /// The accounts still locked here, to unlock for their logins too.
  var locked: [Account] { accounts.filter { !$0.unlocked } }

  /// Back to the logins (or to saving a new passkey), from unlocking another
  /// account or a login's master password, once an account is unlocked here.
  func backToLogins() {
    guard accounts.contains(where: \.unlocked) else { return }
    show()
  }

  /// The website's logins from each unlocked account, the one the app shows
  /// first, by name, and its passkeys when it asks for one; a suggestion
  /// picked fills or signs at once. Making a passkey asks to save it.
  private func show() {
    note = nil
    if making != nil {
      let unlocked = accounts.filter(\.unlocked).map(\.id)
      if saveIn == nil || !unlocked.contains(saveIn!) { saveIn = unlocked.first }
      screen = .saving
      return
    }
    passkeys = signIn.map(passkeysFor) ?? []
    var found: [Login] = []
    for domain in domains {
      let answer = Shared.answer(pb_fillable(domain)) as? [String: Any]
      for item in (answer?["items"] as? [[String: Any]]) ?? [] {
        guard let login = Self.login(item, username: "username"), !found.contains(login) else { continue }
        found.append(login)
      }
    }
    matching = found
    if let picked {
      self.picked = nil
      let isPicked = { (login: Login) in login.account == picked.account && login.item == picked.item }
      if let passkey = passkeys.first(where: isPicked) { return pickPasskey(passkey) }
      if let login = found.first(where: isPicked) { return pick(login) }
    }
    if query.isEmpty {
      logins = found
    } else {
      search()
    }
    screen = .logins
  }

  /// The site's passkeys from each account unlocked here: the ones it names,
  /// or, naming none, the ones it can find by itself (discoverable), as
  /// WebAuthn has it.
  private func passkeysFor(_ signIn: PasskeySignIn) -> [Login] {
    let answer = Shared.answer(pb_passkeys(signIn.rpId)) as? [String: Any]
    let allowed = Set(signIn.allowed.map(Shared.base64url))
    return ((answer?["items"] as? [[String: Any]]) ?? []).compactMap { item in
      guard let account = item["account"] as? String, let id = item["id"] as? String,
        let credential = item["credentialId"] as? String
      else { return nil }
      if allowed.isEmpty ? item["discoverable"] as? Bool == false : !allowed.contains(credential) { return nil }
      let name = item["name"] as? String ?? ""
      let user = item["userName"] as? String ?? ""
      return Login(
        account: account, item: id, name: name, username: user.isEmpty ? name : user,
        asks: item["reprompt"] as? Bool ?? false, passkey: true)
    }
  }

  private static func login(_ item: [String: Any], username: String) -> Login? {
    guard let account = item["account"] as? String, let id = item["id"] as? String else { return nil }
    return Login(
      account: account, item: id, name: item["name"] as? String ?? "",
      username: item[username] as? String ?? "", asks: item["reprompt"] as? Bool ?? false)
  }

  /// The website's logins, or, while searching, every unlocked account's
  /// logins that have it.
  func search() {
    guard !query.isEmpty else {
      logins = matching
      return
    }
    let items = (Shared.answer(pb_list_all(query)) as? [[String: Any]]) ?? []
    logins = items.compactMap { item in
      guard item["kind"] as? String == "login" else { return nil }
      return Self.login(item, username: "subtitle")
    }
  }

  /// The master password, for a login that asks for it again; then it fills.
  /// A Swift string, as in `unlock`: not wiped here, and kept only until the
  /// core has answered.
  func confirm(_ login: Login, password: String) {
    work({ Shared.answer(pb_confirm_master_password(login.account, login.item, password)) as? [String: Any] }) {
      answer in
      guard answer?["ok"] as? Bool == true else {
        self.screen = .reprompt(login)
        self.note = answer?["error"] as? String ?? "That master password isn't right."
        return
      }
      self.confirmed.insert(login.id)
      login.passkey ? self.pickPasskey(login) : self.pick(login)
    }
  }

  /// Fills a login: its username and password, from its own account's core
  /// only now. One that asks for the master password again asks first. Then
  /// one that isn't the site's own (search found it) asks whether to fill it
  /// here, as the browser extension does, unless the person just said so
  /// (`anyway`).
  func pick(_ login: Login, anyway: Bool = false) {
    if login.asks && !confirmed.contains(login.id) {
      note = nil
      screen = .reprompt(login)
      return
    }
    if !anyway && !matching.contains(where: { $0.id == login.id }) {
      note = nil
      screen = .elsewhere(elsewhere(login))
      return
    }
    guard let password = value(login, "password") else {
      note = "That login has no password."
      screen = .logins
      return
    }
    finish(.password(ASPasswordCredential(user: value(login, "username") ?? "", password: password)))
  }

  /// What to show before a login fills on a site that isn't its own: the
  /// websites saved in it, from its own account, and the site iOS asked for.
  private func elsewhere(_ login: Login) -> Elsewhere {
    let item = Shared.answer(pb_item(login.account, login.item)) as? [String: Any]
    let websites = item?["uris"] as? [String] ?? (item?["website"] as? String).map { [$0] } ?? []
    let saved = websites.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
      .map(Self.host)
    let sites = domains.map { $0.lowercased() }
    return Elsewhere(
      login: login, saved: saved, current: sites.first, sameSite: saved.contains(where: sites.contains))
  }

  /// A saved website's host, however it was written (with or without
  /// `https://`, a path, or a port); the website as written if it has none.
  static func host(_ website: String) -> String {
    let address = website.contains("://") ? website : "https://\(website)"
    guard let host = URLComponents(string: address)?.host, !host.isEmpty else { return website }
    return host.lowercased()
  }

  /// Signs in to the site with a passkey, from its own account: the core
  /// signs the site's hash only now. One that asks for the master password
  /// again asks first.
  func pickPasskey(_ login: Login) {
    guard let signIn else { return }
    if login.asks && !confirmed.contains(login.id) {
      note = nil
      screen = .reprompt(login)
      return
    }
    let hash = Shared.base64url(signIn.clientDataHash)
    let proved = verified.contains(login.account) ? "true" : "false"
    // The core may save the passkey's raised counter first: off the main
    // thread, with Cancel hidden until it's back.
    work({
      Shared.answer(pb_passkey_assert(login.account, login.item, signIn.rpId, hash, proved)) as? [String: Any]
    }) { answer in self.signedIn(answer, signIn) }
  }

  private func signedIn(_ answer: [String: Any]?, _ signIn: PasskeySignIn) {
    guard answer?["ok"] as? Bool == true,
      let data = (answer?["authenticatorData"] as? String).flatMap(Shared.fromBase64url),
      let signature = (answer?["signature"] as? String).flatMap(Shared.fromBase64url),
      let credential = (answer?["credentialId"] as? String).flatMap(Shared.fromBase64url)
    else {
      note = answer?["error"] as? String ?? "Couldn't sign in with that passkey."
      screen = .logins
      return
    }
    let handle = (answer?["userHandle"] as? String).flatMap(Shared.fromBase64url) ?? Data()
    finish(
      .signedIn(
        ASPasskeyAssertionCredential(
          userHandle: handle, relyingParty: signIn.rpId, signature: signature,
          clientDataHash: signIn.clientDataHash, authenticatorData: data, credentialID: credential)))
  }

  /// Saves the new passkey in a new login for the site, in the account
  /// chosen: the core makes it and saves it to the server before iOS hears
  /// of it.
  func save() {
    guard let making, let account = saveIn, !finished else { return }
    let request: [String: Any] = [
      "account": account,
      "rpId": making.rpId,
      "userHandle": Shared.base64url(making.userHandle),
      "userName": making.userName,
      "userVerified": verified.contains(account),
      "discoverable": true,
      "exclude": making.excluded.map(Shared.base64url),
    ]
    guard let body = try? JSONSerialization.data(withJSONObject: request),
      let json = String(data: body, encoding: .utf8)
    else { return }
    // Saved over the network: off the main thread, with Cancel hidden
    // until it's back, so the site never misses a passkey the vault kept.
    work({ Shared.answer(pb_passkey_create(json)) as? [String: Any] }) { answer in
      self.made(answer, making, account)
    }
  }

  private func made(_ answer: [String: Any]?, _ making: PasskeyMaking, _ account: String) {
    guard let request else { return }
    guard answer?["ok"] as? Bool == true,
      let item = answer?["itemId"] as? String,
      let credential = (answer?["credentialId"] as? String).flatMap(Shared.fromBase64url),
      let attestation = (answer?["attestationObject"] as? String).flatMap(Shared.fromBase64url)
    else {
      // The vault has one of the site's own: it says so, as WebAuthn has it.
      if answer?["excluded"] as? Bool == true {
        if #available(iOS 18.0, *) { return finish(.refused(.matchedExcludedCredential)) }
      }
      note = answer?["error"] as? String ?? "Couldn't save the passkey."
      screen = .saving
      return
    }
    let made = Answer.made(
      ASPasskeyRegistrationCredential(
        relyingParty: making.rpId, clientDataHash: making.clientDataHash, credentialID: credential,
        attestationObject: attestation))
    // iOS lists it from now on, as it would after the app's next sync:
    // answered once iOS has it, before the extension goes away.
    let identity = ASPasskeyCredentialIdentity(
      relyingPartyIdentifier: making.rpId, userName: making.userName, credentialID: credential,
      userHandle: making.userHandle, recordIdentifier: "\(account)/\(item)")
    let store = ASCredentialIdentityStore.shared
    // iOS answers on a queue of its own, and later: only for this request.
    let answer = { [requests] in
      DispatchQueue.main.async {
        if requests.has(request) { self.finish(made) }
      }
    }
    store.getState { state in
      guard state.isEnabled, state.supportsIncrementalUpdates else { return answer() }
      let identities: [any ASCredentialIdentity] = [identity]
      store.saveCredentialIdentities(identities) { _, _ in answer() }
    }
  }

  private func value(_ login: Login, _ what: String) -> String? {
    var length = 0
    let bytes = pb_fill_secret(login.account, login.item, what, &length)
    return Shared.string(bytes, length)
  }

  /// Ends the extension with its answer, or without one (Cancel), once:
  /// every account locks first. A core call that comes back after Cancel, or
  /// after another answer, does nothing more, and one still waiting isn't
  /// made (`work`).
  func finish(_ answer: Answer) {
    guard !finished else { return }
    close()
    onFinish(answer)
  }

  /// Ends this request: every account locks, and the core lets them go.
  /// Nothing to close once it has ended, or if another request has started
  /// since, whose accounts aren't this one's to close.
  func close() {
    finished = true
    guard let request else { return }
    self.request = nil
    requests.end(request) { _ = Shared.answer(pb_autofill_close()) }
  }
}

/// PassBundle's look in the extension: the brand's emerald where the app
/// has it, and the account tones the app labels accounts with
/// (app/lib/theme.dart, app/lib/accounts.dart).
enum Look {
  static let accent = Color(
    UIColor { traits in
      traits.userInterfaceStyle == .dark ? UIColor(hex: 0x6FD4B4) : UIColor(hex: 0x1F6F5C)
    })

  static func tone(_ name: String) -> Color {
    let (light, dark): (UInt32, UInt32) =
      switch name {
      case "emerald": (0x0E7A5A, 0x5FD6AE)
      case "amethyst": (0x6B30B8, 0xC6A3FF)
      case "topaz": (0xA9600C, 0xF2B766)
      case "ruby": (0xB01E54, 0xFF92B8)
      case "turquoise": (0x08788A, 0x62D3E3)
      case "garnet": (0xA8291F, 0xFF9D92)
      case "aquamarine": (0x1C7FB8, 0x7CC4EF)
      case "onyx": (0x353B4A, 0xC3C9D6)
      case "gold": (0x946A0C, 0xECC25E)
      default: (0x2448B8, 0x93ADFF)
      }
    return Color(
      UIColor { traits in UIColor(hex: traits.userInterfaceStyle == .dark ? dark : light) })
  }
}

extension UIColor {
  convenience init(hex: UInt32) {
    self.init(
      red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
      blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
  }
}

/// The extension's screens, in PassBundle's look and with the app's words.
struct ProviderView: View {
  @ObservedObject var provider: Provider
  /// The master password as it's typed: cleared as it's handed to the
  /// provider. A Swift string, which can't be wiped (`Provider.unlock`).
  @State private var password = ""

  var body: some View {
    NavigationStack {
      Group {
        switch provider.screen {
        case .opening, .working:
          ProgressView()
        case .saving:
          saving
        case .locked:
          unlock
        case .logins:
          list
        case .reprompt(let login):
          reprompt(login)
        case .elsewhere(let asked):
          elsewhere(asked)
        case .unavailable(let words):
          Text(words).multilineTextAlignment(.center).padding()
        }
      }
      .navigationTitle("PassBundle")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          // Not while the core is saving or signing: it answers first.
          if !provider.working {
            Button("Cancel") { provider.finish(.refused(.userCanceled)) }
          }
        }
      }
    }
    .tint(Look.accent)
  }

  /// An account's name in its color, as the app labels it.
  private func pill(_ account: String) -> some View {
    Text(provider.label(account) ?? "")
      .font(.caption.weight(.semibold))
      .foregroundStyle(Look.tone(provider.tone(account)))
      .padding(.horizontal, 8)
      .padding(.vertical, 2)
      .background(Look.tone(provider.tone(account)).opacity(0.14), in: Capsule())
  }

  private var unlock: some View {
    Form {
      Section {
        SecureField("Master password", text: $password)
          .textContentType(.password)
          .submitLabel(.go)
          .onSubmit(unlockWithPassword)
        Button("Unlock", action: unlockWithPassword)
          .disabled(password.isEmpty)
      } header: {
        VStack(alignment: .leading, spacing: 4) {
          Text("Unlock PassBundle")
          if let account = provider.unlocking, provider.accounts.count > 1 {
            Text(account.label).textCase(nil)
          }
        }
      } footer: {
        if let note = provider.note {
          Text(note).foregroundStyle(.red)
        }
      }
      if let method = provider.biometric {
        Section {
          Button("Unlock with \(method)") { provider.unlockWithBiometric() }
        }
      }
      // Back to the logins of the accounts unlocked already (or to saving
      // the passkey).
      if provider.accounts.contains(where: \.unlocked) {
        Section {
          Button(provider.making == nil ? "Back to the logins" : "Back to saving the passkey") {
            password = ""
            provider.backToLogins()
          }
        }
      }
      // Another account to unlock instead, with more than one.
      let others = provider.locked.filter { $0.id != provider.unlocking?.id }
      if !others.isEmpty {
        Section("Switch account") {
          ForEach(others) { account in
            Button(account.label) {
              password = ""
              provider.askToUnlock(account, automatically: true)
            }
          }
        }
      }
    }
  }

  /// Saving a new passkey: the site's ID first, as iOS checked it, the
  /// account's name on the site, and which PassBundle account it goes in.
  private var saving: some View {
    Form {
      Section {
        if let making = provider.making {
          LabeledContent("Website", value: making.rpId)
          if !making.userName.isEmpty {
            LabeledContent("Account", value: making.userName)
          }
        }
        let unlocked = provider.accounts.filter(\.unlocked)
        if provider.accounts.count > 1, !unlocked.isEmpty {
          Picker("Save in", selection: $provider.saveIn) {
            ForEach(unlocked) { account in
              Text(account.label).tag(Optional(account.id))
            }
          }
        }
        Button("Save passkey") { provider.save() }
          .disabled(provider.saveIn == nil)
      } header: {
        Text("Save a passkey in PassBundle")
      } footer: {
        if let note = provider.note {
          Text(note).foregroundStyle(.red)
        }
      }
      if !provider.locked.isEmpty {
        Section("Other accounts") {
          ForEach(provider.locked) { account in
            Button("Unlock \(account.label)") {
              password = ""
              provider.askToUnlock(account, automatically: true)
            }
          }
        }
      }
    }
  }

  private func reprompt(_ login: Provider.Login) -> some View {
    Form {
      Section {
        SecureField("Master password", text: $password)
          .textContentType(.password)
          .submitLabel(.go)
          .onSubmit { confirm(login) }
        Button("Continue") { confirm(login) }
          .disabled(password.isEmpty)
      } header: {
        Text("This login asks for your master password")
      } footer: {
        if let note = provider.note {
          Text(note).foregroundStyle(.red)
        }
      }
      Section {
        Button("Back to the logins") {
          password = ""
          provider.backToLogins()
        }
      }
    }
  }

  /// A login found by search, for a site that isn't its own: the browser
  /// extension's "Site doesn't match", in its words. Cancel, above, ends the
  /// request, as everywhere here.
  private func elsewhere(_ asked: Provider.Elsewhere) -> some View {
    Form {
      Section {
        VStack(alignment: .leading, spacing: 6) {
          Text(asked.title).font(.headline)
          Text(asked.words)
        }
        .padding(.vertical, 4)
      }
      if !asked.saved.isEmpty {
        Section(asked.savedHeading) {
          ForEach(Array(asked.saved.enumerated()), id: \.offset) { website in
            Text(website.element).font(.body.monospaced())
          }
        }
      }
      if let current = asked.current {
        Section("Current website") {
          Text(current).font(.body.monospaced())
        }
      }
      Section {
        Button("Autofill") { provider.pick(asked.login, anyway: true) }
        Button("Back to the logins") { provider.backToLogins() }
      }
    }
  }

  private func confirm(_ login: Provider.Login) {
    guard !password.isEmpty else { return }
    provider.confirm(login, password: password)
    password = ""
  }

  private func unlockWithPassword() {
    guard !password.isEmpty else { return }
    provider.unlock(password: password)
    password = ""
  }

  private func row(_ login: Provider.Login, action: @escaping () -> Void) -> some View {
    Button(action: action) {
      VStack(alignment: .leading, spacing: 2) {
        Text(login.name).font(.headline).foregroundStyle(Look.accent)
        HStack(spacing: 6) {
          if !login.username.isEmpty {
            Text(login.username).font(.subheadline).foregroundStyle(.secondary)
          }
          if provider.label(login.account) != nil {
            pill(login.account)
          }
        }
      }
    }
  }

  private var empty: String {
    if !provider.query.isEmpty { return "No logins match." }
    if provider.signIn != nil { return "No passkeys or logins for this website. Search your logins." }
    return "No logins for this website. Search for one."
  }

  private var list: some View {
    List {
      let passkeys = provider.query.isEmpty ? provider.passkeys : []
      if !passkeys.isEmpty {
        Section("Passkeys") {
          ForEach(passkeys) { passkey in
            row(passkey) { provider.pickPasskey(passkey) }
          }
        }
      }
      Section {
        if provider.logins.isEmpty && passkeys.isEmpty {
          Text(empty).foregroundStyle(.secondary)
        }
        ForEach(provider.logins) { login in
          row(login) { provider.pick(login) }
        }
        if let note = provider.note {
          Text(note).foregroundStyle(.red)
        }
      } header: {
        if !passkeys.isEmpty && !provider.logins.isEmpty {
          Text("Logins")
        }
      }
      // Accounts still locked here: their logins show once unlocked.
      if !provider.locked.isEmpty {
        Section("Other accounts") {
          ForEach(provider.locked) { account in
            Button("Unlock \(account.label)") {
              password = ""
              provider.askToUnlock(account, automatically: true)
            }
          }
        }
      }
    }
    .searchable(text: $provider.query, prompt: "Search all logins")
    .onChange(of: provider.query) { provider.search() }
  }
}
