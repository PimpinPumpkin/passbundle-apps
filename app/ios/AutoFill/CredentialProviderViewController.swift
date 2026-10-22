import AuthenticationServices
import SwiftUI

/// PassBundle in iOS's AutoFill (docs/phones.md, "Autofill"): a separate
/// process with a tight memory limit, so no Flutter. Its few screens are our
/// own, in SwiftUI, around the same core, linked in. It reads the vault the
/// app keeps in the App Group, and unlocks on its own (Face ID or Touch ID,
/// or the master password), since it can't share the app's memory. A
/// password is taken from the core only once someone picks its login.
class CredentialProviderViewController: ASCredentialProviderViewController {
  private let provider = Provider()

  override func viewDidLoad() {
    super.viewDidLoad()
    provider.finish = { [weak self] credential in
      // Nothing decrypted stays behind in this process.
      _ = Shared.answer(pb_lock())
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

  /// The logins for the website someone is signing in to.
  override func prepareCredentialList(for serviceIdentifiers: [ASCredentialServiceIdentifier]) {
    provider.start(domains: serviceIdentifiers.compactMap(Provider.domain), picked: nil)
  }

  /// A suggestion picked above the keyboard: never filled without the
  /// person, since the vault opens locked in this process.
  override func provideCredentialWithoutUserInteraction(for credentialRequest: ASCredentialRequest) {
    extensionContext.cancelRequest(
      withError: NSError(domain: ASExtensionErrorDomain, code: ASExtensionError.userInteractionRequired.rawValue))
  }

  /// A suggestion picked above the keyboard, once unlocked: that login.
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
    case locked
    case logins
    /// A login that asks for the master password again, before it fills
    /// (docs/core.md, "Asking for the master password again").
    case reprompt(String)
    case unavailable(String)
  }

  struct Login: Identifiable {
    let id: String
    let name: String
    let username: String
    /// Asks for the master password again before it fills.
    var asks = false
  }

  @Published var screen = Screen.opening
  @Published var logins: [Login] = []
  /// What the person searches for, among all their logins, when the
  /// website's aren't what they want.
  @Published var query = ""
  @Published var note: String?
  /// "Face ID" or "Touch ID", when the user key is kept for it.
  @Published var biometric: String?
  var finish: (ASPasswordCredential?) -> Void = { _ in }

  private var domains: [String] = []
  private var picked: String?

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
  func start(domains: [String], picked: String?) {
    self.domains = domains
    self.picked = picked
    guard let dir = Shared.filesDir() else {
      screen = .unavailable("PassBundle's folder isn't there.")
      return
    }
    let state = (Shared.answer(pb_open(dir, false)) as? [String: Any])?["state"] as? String
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
      if unlocked { return self.show() }
      self.note = error
    }
  }

  func unlock(password: String) {
    let answer = Shared.answer(pb_unlock(password)) as? [String: Any]
    if answer?["ok"] as? Bool == true { return show() }
    note = answer?["error"] as? String ?? "That's not the master password."
  }

  /// The logins for the website, by name; a suggestion picked fills at once.
  private func show() {
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
    note = nil
    if let picked, found.contains(where: { $0.id == picked }) { return pick(picked) }
    screen = .logins
  }

  private var matching: [Login] = []
  private var confirmed: Set<String> = []

  private func asks(_ id: String) -> Bool {
    (logins + matching).first { $0.id == id }?.asks ?? false
  }

  /// The master password, for a login that asks for it again; then it fills.
  func confirm(_ id: String, password: String) {
    let answer = Shared.answer(pb_confirm_master_password(id, password)) as? [String: Any]
    guard answer?["ok"] as? Bool == true else {
      note = answer?["error"] as? String ?? "That master password isn't right."
      return
    }
    confirmed.insert(id)
    pick(id)
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
    if asks(id) && !confirmed.contains(id) {
      note = nil
      screen = .reprompt(id)
      return
    }
    guard let password = Shared.secret(id, "password") else {
      note = "That login has no password."
      return
    }
    finish(ASPasswordCredential(user: Shared.secret(id, "username") ?? "", password: password))
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
        case .opening:
          ProgressView()
        case .locked:
          unlock
        case .logins:
          list
        case .reprompt(let id):
          reprompt(id)
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

  private var list: some View {
    List {
      if provider.logins.isEmpty {
        Text(provider.query.isEmpty ? "No logins for this website. Search for one." : "No logins match.")
          .foregroundStyle(.secondary)
      }
      ForEach(provider.logins) { login in
        Button {
          provider.pick(login.id)
        } label: {
          VStack(alignment: .leading, spacing: 2) {
            Text(login.name).font(.headline).foregroundStyle(Self.brand)
            if !login.username.isEmpty {
              Text(login.username).font(.subheadline).foregroundStyle(.secondary)
            }
          }
        }
      }
      if let note = provider.note {
        Text(note).foregroundStyle(.red)
      }
    }
    .searchable(text: $provider.query, prompt: "Search all logins")
    .onChange(of: provider.query) { provider.search() }
  }
}
