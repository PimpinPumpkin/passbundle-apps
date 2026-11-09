import AuthenticationServices
import SwiftUI

/// PassBundle in iOS's AutoFill (docs/phones.md, "Autofill"): a separate
/// process with a tight memory limit, so no Flutter. Its few screens are our
/// own, in SwiftUI, around the same core, linked in. It reads the vault the
/// app keeps in the App Group, beside the app and without changing anything
/// there, and unlocks on its own (Face ID or Touch ID, or the master
/// password), since it can't share the app's memory. With several accounts,
/// it lists the logins of each account unlocked here, labeled, and fills a
/// login only from its own account. A password is taken from the core only
/// once someone picks its login, and every account locks as the request
/// ends, however it ends.
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
    provider.onFinish = { [weak self] credential in
      guard let context = self?.extensionContext else { return }
      if let credential {
        context.completeRequest(withSelectedCredential: credential, completionHandler: nil)
      } else {
        context.cancelRequest(
          withError: NSError(domain: ASExtensionErrorDomain, code: ASExtensionError.userCanceled.rawValue))
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

  /// A suggestion picked above the keyboard: never filled without the
  /// person, since every account is locked in this process.
  override func provideCredentialWithoutUserInteraction(for credentialRequest: ASCredentialRequest) {
    extensionContext.cancelRequest(
      withError: NSError(domain: ASExtensionErrorDomain, code: ASExtensionError.userInteractionRequired.rawValue))
  }

  /// A suggestion picked above the keyboard, once its account is unlocked:
  /// that login.
  override func prepareInterfaceToProvideCredential(for credentialRequest: ASCredentialRequest) {
    let identity = credentialRequest.credentialIdentity
    provider.start(
      domains: [Provider.domain(identity.serviceIdentifier)].compactMap { $0 },
      picked: identity.recordIdentifier)
  }
}

/// What the extension's screens show, and what they do with the core.
final class Provider: ObservableObject {
  enum Screen {
    case opening
    /// The core unlocking or checking a master password (the KDF).
    case working
    /// An account to unlock (`unlocking`).
    case locked
    case logins
    /// A login that asks for the master password again, before it fills
    /// (docs/core.md, "Asking for the master password again").
    case reprompt(Login)
    case unavailable(String)
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
  /// What ends the extension (the view controller's).
  var onFinish: (ASPasswordCredential?) -> Void = { _ in }
  /// Ended: nothing answers iOS twice, or after Cancel.
  private(set) var finished = false

  private var domains: [String] = []
  /// A suggestion picked above the keyboard: `<account>/<item>`.
  private var picked: (account: String, item: String)?
  private var matching: [Login] = []
  private var confirmed: Set<String> = []

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
    self.domains = domains
    self.picked = picked.flatMap { record in
      let parts = record.split(separator: "/", maxSplits: 1).map(String.init)
      return parts.count == 2 ? (parts[0], parts[1]) : nil
    }
    guard let dir = Shared.filesDir() else {
      screen = .unavailable("PassBundle's folder isn't there.")
      return
    }
    let opened = Shared.answer(pb_autofill_open(dir)) as? [String: Any]
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

  func unlockWithBiometric() {
    guard let account = unlocking?.id else { return }
    Shared.unlockWithBiometric(account: account) { [weak self] unlocked, error in
      guard let self, !self.finished else { return }
      if unlocked { return self.unlocked(account) }
      self.note = error
      if !Shared.biometricKept(account) { self.biometric = nil }
    }
  }

  func unlock(password: String) {
    guard let account = unlocking?.id else { return }
    screen = .working
    DispatchQueue.global(qos: .userInitiated).async {
      let answer = Shared.answer(pb_unlock(account, password)) as? [String: Any]
      DispatchQueue.main.async {
        guard !self.finished else { return }
        if answer?["ok"] as? Bool == true { return self.unlocked(account) }
        self.screen = .locked
        self.note = answer?["error"] as? String ?? "That's not the master password."
      }
    }
  }

  private func unlocked(_ account: String) {
    if let at = accounts.firstIndex(where: { $0.id == account }) {
      accounts[at].unlocked = true
    }
    show()
  }

  /// The accounts still locked here, to unlock for their logins too.
  var locked: [Account] { accounts.filter { !$0.unlocked } }

  /// The website's logins from each unlocked account, the one the app shows
  /// first, by name; a suggestion picked fills at once.
  private func show() {
    note = nil
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
      if let login = found.first(where: { $0.account == picked.account && $0.item == picked.item }) {
        return pick(login)
      }
    }
    if query.isEmpty {
      logins = found
    } else {
      search()
    }
    screen = .logins
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
  func confirm(_ login: Login, password: String) {
    screen = .working
    DispatchQueue.global(qos: .userInitiated).async {
      let answer =
        Shared.answer(pb_confirm_master_password(login.account, login.item, password)) as? [String: Any]
      DispatchQueue.main.async {
        guard !self.finished else { return }
        guard answer?["ok"] as? Bool == true else {
          self.screen = .reprompt(login)
          self.note = answer?["error"] as? String ?? "That master password isn't right."
          return
        }
        self.confirmed.insert(login.id)
        self.pick(login)
      }
    }
  }

  /// Fills a login: its username and password, from its own account's core
  /// only now. One that asks for the master password again asks first.
  func pick(_ login: Login) {
    if login.asks && !confirmed.contains(login.id) {
      note = nil
      screen = .reprompt(login)
      return
    }
    guard let password = value(login, "password") else {
      note = "That login has no password."
      screen = .logins
      return
    }
    finish(ASPasswordCredential(user: value(login, "username") ?? "", password: password))
  }

  private func value(_ login: Login, _ what: String) -> String? {
    var length = 0
    let bytes = pb_fill_secret(login.account, login.item, what, &length)
    return Shared.string(bytes, length)
  }

  /// Ends the extension with the login, or without one (Cancel), once:
  /// every account locks first.
  func finish(_ credential: ASPasswordCredential?) {
    guard !finished else { return }
    close()
    onFinish(credential)
  }

  /// Every account locks, and the core lets them go.
  func close() {
    finished = true
    _ = Shared.answer(pb_autofill_close())
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
  @State private var password = ""

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
        case .reprompt(let login):
          reprompt(login)
        case .unavailable(let words):
          Text(words).multilineTextAlignment(.center).padding()
        }
      }
      .navigationTitle("PassBundle")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button("Cancel") { provider.finish(nil) }
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

  private var list: some View {
    List {
      Section {
        if provider.logins.isEmpty {
          Text(provider.query.isEmpty ? "No logins for this website. Search for one." : "No logins match.")
            .foregroundStyle(.secondary)
        }
        ForEach(provider.logins) { login in
          Button {
            provider.pick(login)
          } label: {
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
        if let note = provider.note {
          Text(note).foregroundStyle(.red)
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
