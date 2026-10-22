import AuthenticationServices
import SwiftUI

/// PassBundle in iOS's AutoFill (docs/phones.md, "Autofill" and "Passkeys"):
/// a separate process with a tight memory limit, so no Flutter. Its few
/// screens are our own, in SwiftUI, around the same core, linked in. It reads
/// the vault the app keeps in the App Group, and unlocks on its own (Face ID
/// or Touch ID, or the master password), since it can't share the app's
/// memory. A password is taken from the core only once someone picks its
/// login, and a passkey signs only once someone picks it.
class CredentialProviderViewController: ASCredentialProviderViewController {
  private let provider = Provider()

  override func viewDidLoad() {
    super.viewDidLoad()
    provider.onFinish = { [weak self] answer in
      // Nothing decrypted stays behind in this process.
      _ = Shared.answer(pb_lock())
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

  /// The logins for the website someone is signing in to.
  override func prepareCredentialList(for serviceIdentifiers: [ASCredentialServiceIdentifier]) {
    provider.start(domains: serviceIdentifiers.compactMap(Provider.domain), picked: nil)
  }

  /// A site asking for a passkey (and maybe a password): its passkeys, then
  /// its logins. iOS has checked the site's ID against the page or the app.
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
  /// the person, since the vault opens locked in this process.
  override func provideCredentialWithoutUserInteraction(for credentialRequest: ASCredentialRequest) {
    extensionContext.cancelRequest(
      withError: NSError(domain: ASExtensionErrorDomain, code: ASExtensionError.userInteractionRequired.rawValue))
  }

  /// A suggestion picked from iOS's own list, once unlocked: that login, or
  /// that passkey.
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

  /// A site making a passkey: saved in a new login for the site, once the
  /// person unlocks and says so.
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

/// What the extension's screens show, and what they do with the core.
final class Provider: ObservableObject {
  enum Screen {
    case opening
    /// The core saving a passkey or signing, over the network maybe.
    case working
    case locked
    case logins
    /// A login that asks for the master password again, before it fills or
    /// signs (docs/core.md, "Asking for the master password again").
    case reprompt(String)
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

  struct Login: Identifiable {
    let id: String
    let name: String
    let username: String
    /// Asks for the master password again before it fills.
    var asks = false
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

  @Published var screen = Screen.opening
  @Published var logins: [Login] = []
  /// The site's passkeys, when it asks for one.
  @Published var passkeys: [Login] = []
  /// What the person searches for, among all their logins, when the
  /// website's aren't what they want.
  @Published var query = ""
  @Published var note: String?
  /// "Face ID" or "Touch ID", when the user key is kept for it.
  @Published var biometric: String?
  /// What ends the extension (the view controller's).
  var onFinish: (Answer) -> Void = { _ in }
  /// Ended: nothing answers iOS twice, or after Cancel.
  private(set) var finished = false
  var signIn: PasskeySignIn?
  var making: PasskeyMaking?

  private var domains: [String] = []
  private var picked: String?
  /// Whether the person proved who they are in this request: unlocking, or
  /// the master password for a login that asks for it.
  private var verified = false

  /// Whether the core is saving or signing: Cancel waits for it.
  var working: Bool {
    if case .working = screen { return true }
    return false
  }

  /// Ends the extension with the answer, once: a core call that comes back
  /// after Cancel, or after another answer, does nothing more.
  func finish(_ answer: Answer) {
    guard !finished else { return }
    finished = true
    onFinish(answer)
  }

  /// The domain a service identifier names (a website's, or its address's host).
  static func domain(_ service: ASCredentialServiceIdentifier) -> String? {
    switch service.type {
    case .domain: return service.identifier
    case .URL: return URL(string: service.identifier)?.host
    @unknown default: return nil
    }
  }

  /// Opens the kept vault: the logins at once if it's unlocked, else the
  /// unlock screen, which asks for Face ID or Touch ID first where it's on.
  /// A passkey always has the person unlock, which is its proof of them.
  func start(domains: [String], picked: String?) {
    self.domains = domains
    self.picked = picked
    guard let dir = Shared.filesDir() else {
      screen = .unavailable("PassBundle's folder isn't there.")
      return
    }
    var state = (Shared.answer(pb_open(dir, false)) as? [String: Any])?["state"] as? String
    // pb_open starts every request with the vault locked; were it ever open
    // here, a passkey would still have the person unlock first.
    if state == "unlocked" && (signIn != nil || making != nil) {
      _ = Shared.answer(pb_lock())
      state = "locked"
    }
    switch state {
    case "unlocked":
      show()
    case "locked":
      screen = .locked
      if Shared.biometricKept() {
        biometric = Shared.biometricMethod()
        unlockWithBiometric()
      }
    default:
      screen = .unavailable("Sign in to PassBundle in its app first.")
    }
  }

  func unlockWithBiometric() {
    Shared.unlockWithBiometric { [weak self] unlocked, error in
      guard let self else { return }
      if unlocked {
        self.verified = true
        return self.show()
      }
      self.note = error
    }
  }

  func unlock(password: String) {
    let answer = Shared.answer(pb_unlock(password)) as? [String: Any]
    if answer?["ok"] as? Bool == true {
      verified = true
      return show()
    }
    note = answer?["error"] as? String ?? "That's not the master password."
  }

  /// The logins for the website, by name, and its passkeys when it asks for
  /// one; a suggestion picked fills or signs at once. Making a passkey asks
  /// to save it.
  private func show() {
    note = nil
    if making != nil {
      screen = .saving
      return
    }
    var found: [Login] = []
    defer { matching = found }
    for domain in domains {
      let answer = Shared.answer(pb_fillable(domain)) as? [String: Any]
      for item in (answer?["items"] as? [[String: Any]]) ?? [] {
        guard let id = item["id"] as? String, !found.contains(where: { $0.id == id }) else { continue }
        found.append(
          Login(
            id: id, name: item["name"] as? String ?? "", username: item["username"] as? String ?? "",
            asks: item["reprompt"] as? Bool ?? false))
      }
    }
    logins = found
    passkeys = signIn.map(passkeysFor) ?? []
    if let picked {
      if passkeys.contains(where: { $0.id == picked }) { return pickPasskey(picked) }
      if found.contains(where: { $0.id == picked }) { return pick(picked) }
    }
    screen = .logins
  }

  /// The site's passkeys: the ones it names, or, naming none, the ones it
  /// can find by itself (discoverable), as WebAuthn has it.
  private func passkeysFor(_ signIn: PasskeySignIn) -> [Login] {
    let answer = Shared.answer(pb_passkeys(signIn.rpId)) as? [String: Any]
    let allowed = Set(signIn.allowed.map(Shared.base64url))
    return ((answer?["items"] as? [[String: Any]]) ?? []).compactMap { item in
      guard let id = item["id"] as? String, let credential = item["credentialId"] as? String else { return nil }
      if allowed.isEmpty ? item["discoverable"] as? Bool == false : !allowed.contains(credential) { return nil }
      let name = item["name"] as? String ?? ""
      let user = item["userName"] as? String ?? ""
      return Login(
        id: id, name: name, username: user.isEmpty ? name : user, asks: item["reprompt"] as? Bool ?? false)
    }
  }

  private var matching: [Login] = []
  private var confirmed: Set<String> = []
  /// What the master password asked for, once given: a passkey's sign-in,
  /// or a login's fill.
  private var confirmingPasskey = false

  private func asks(_ id: String, passkey: Bool) -> Bool {
    (passkey ? passkeys : logins + matching).first { $0.id == id }?.asks ?? false
  }

  /// The master password, for a login that asks for it again; then it fills,
  /// or signs.
  func confirm(_ id: String, password: String) {
    let answer = Shared.answer(pb_confirm_master_password(id, password)) as? [String: Any]
    guard answer?["ok"] as? Bool == true else {
      note = answer?["error"] as? String ?? "That master password isn't right."
      return
    }
    confirmed.insert(id)
    verified = true
    if confirmingPasskey {
      pickPasskey(id)
    } else {
      pick(id)
    }
  }

  /// The website's logins, or, while searching, every login that has it.
  func search() {
    guard !query.isEmpty else {
      logins = matching
      return
    }
    let items = (Shared.answer(pb_list(query)) as? [[String: Any]]) ?? []
    logins = items.compactMap { item in
      guard item["kind"] as? String == "login", let id = item["id"] as? String else { return nil }
      return Login(
        id: id, name: item["name"] as? String ?? "", username: item["subtitle"] as? String ?? "",
        asks: item["reprompt"] as? Bool ?? false)
    }
  }

  /// Fills a login: its username and password, from the core only now. One
  /// that asks for the master password again asks first.
  func pick(_ id: String) {
    if asks(id, passkey: false) && !confirmed.contains(id) {
      note = nil
      confirmingPasskey = false
      screen = .reprompt(id)
      return
    }
    guard let password = Shared.secret(id, "password") else {
      note = "That login has no password."
      return
    }
    finish(.password(ASPasswordCredential(user: Shared.secret(id, "username") ?? "", password: password)))
  }

  /// Signs in to the site with a login's passkey: the core signs the site's
  /// hash only now. One that asks for the master password again asks first.
  func pickPasskey(_ id: String) {
    guard let signIn else { return }
    if asks(id, passkey: true) && !confirmed.contains(id) {
      note = nil
      confirmingPasskey = true
      screen = .reprompt(id)
      return
    }
    screen = .working
    let hash = Shared.base64url(signIn.clientDataHash)
    let verified = self.verified ? "true" : "false"
    // The core may save the passkey's raised counter first: off the main
    // thread, with Cancel hidden until it's back.
    DispatchQueue.global(qos: .userInitiated).async {
      let answer = Shared.answer(pb_passkey_assert(id, signIn.rpId, hash, verified)) as? [String: Any]
      DispatchQueue.main.async { self.signedIn(answer, signIn) }
    }
  }

  private func signedIn(_ answer: [String: Any]?, _ signIn: PasskeySignIn) {
    guard !finished else { return }
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

  /// Saves the new passkey, in a new login for the site: the core makes it
  /// and saves it before iOS hears of it.
  func save() {
    guard let making, !finished else { return }
    let request: [String: Any] = [
      "rpId": making.rpId,
      "userHandle": Shared.base64url(making.userHandle),
      "userName": making.userName,
      "userVerified": verified,
      "discoverable": true,
      "exclude": making.excluded.map(Shared.base64url),
    ]
    guard let body = try? JSONSerialization.data(withJSONObject: request),
      let json = String(data: body, encoding: .utf8)
    else { return }
    // Saved over the network: off the main thread, with Cancel hidden
    // until it's back, so the site never misses a passkey the vault kept.
    screen = .working
    DispatchQueue.global(qos: .userInitiated).async {
      let answer = Shared.answer(pb_passkey_create(json)) as? [String: Any]
      DispatchQueue.main.async { self.made(answer, making) }
    }
  }

  private func made(_ answer: [String: Any]?, _ making: PasskeyMaking) {
    guard !finished else { return }
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
    // iOS suggests it from now on, as it would after the app's next sync:
    // answered once iOS has it, before the extension goes away.
    let identity = ASPasskeyCredentialIdentity(
      relyingPartyIdentifier: making.rpId, userName: making.userName, credentialID: credential,
      userHandle: making.userHandle, recordIdentifier: item)
    let store = ASCredentialIdentityStore.shared
    let answer = { DispatchQueue.main.async { self.finish(made) } }
    store.getState { state in
      guard state.isEnabled, state.supportsIncrementalUpdates else { return answer() }
      let identities: [any ASCredentialIdentity] = [identity]
      store.saveCredentialIdentities(identities) { _, _ in answer() }
    }
  }
}

/// The extension's screens, in PassBundle's look: the brand color, the
/// mark's name, and the same words as the app.
struct ProviderView: View {
  @ObservedObject var provider: Provider
  @State private var password = ""

  private static let brand = Color(red: 0x1F / 255, green: 0x6F / 255, blue: 0x5C / 255)

  var body: some View {
    NavigationStack {
      Group {
        switch provider.screen {
        case .opening, .working:
          ProgressView()
        case .locked:
          unlock
        case .logins:
          list
        case .reprompt(let id):
          reprompt(id)
        case .saving:
          saving
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
    .tint(Self.brand)
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
        Text("Unlock PassBundle")
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
    }
  }

  private func reprompt(_ id: String) -> some View {
    Form {
      Section {
        SecureField("Master password", text: $password)
          .textContentType(.password)
          .submitLabel(.go)
          .onSubmit { confirm(id) }
        Button("Continue") { confirm(id) }
          .disabled(password.isEmpty)
      } header: {
        Text("This login asks for your master password")
      } footer: {
        if let note = provider.note {
          Text(note).foregroundStyle(.red)
        }
      }
    }
  }

  /// Saving a new passkey: the site's ID first, as iOS checked it, and the
  /// account's name.
  private var saving: some View {
    Form {
      Section {
        if let making = provider.making {
          LabeledContent("Website", value: making.rpId)
          if !making.userName.isEmpty {
            LabeledContent("Account", value: making.userName)
          }
        }
        Button("Save passkey") { provider.save() }
      } header: {
        Text("Save a passkey in PassBundle")
      } footer: {
        if let note = provider.note {
          Text(note).foregroundStyle(.red)
        }
      }
    }
  }

  private func confirm(_ id: String) {
    guard !password.isEmpty else { return }
    provider.confirm(id, password: password)
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
        Text(login.name).font(.headline).foregroundStyle(Self.brand)
        if !login.username.isEmpty {
          Text(login.username).font(.subheadline).foregroundStyle(.secondary)
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
            row(passkey) { provider.pickPasskey(passkey.id) }
          }
        }
      }
      Section {
        if provider.logins.isEmpty && passkeys.isEmpty {
          Text(empty).foregroundStyle(.secondary)
        }
        ForEach(provider.logins) { login in
          row(login) { provider.pick(login.id) }
        }
        if let note = provider.note {
          Text(note).foregroundStyle(.red)
        }
      } header: {
        if !passkeys.isEmpty && !provider.logins.isEmpty {
          Text("Logins")
        }
      }
    }
    .searchable(text: $provider.query, prompt: "Search all logins")
    .onChange(of: provider.query) { provider.search() }
  }
}
