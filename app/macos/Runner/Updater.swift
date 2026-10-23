// Updates from PassBundle's GitHub releases on the Mac (README, "Mac test
// releases"), the way the Android app updates itself
// (app/android/app/src/main/kotlin/com/passbundle/app/update/Updater.kt):
// the newest macOS release, when it's newer than this copy, offered in
// Settings. On a click it's downloaded over HTTPS by the app itself, checked
// against the release's SHA256SUMS, unpacked, and checked to be PassBundle at
// the release's version, signed so that it meets this copy's own designated
// requirement (the same certificate, so the keychain keeps trusting it).
// Then this copy goes to the Trash, the new one takes its place, and
// PassBundle restarts.
//
// It contacts api.github.com, and github.com for the files (which GitHub
// hands to objects.githubusercontent.com or
// release-assets.githubusercontent.com), and no other host, only to check or
// download: when the person clicks "Check for updates" or "Update", and at
// most once a day when the app starts, if the person leaves that on. Debug
// builds don't check by themselves, and read a test release from
// `update-source` in the app's folder when there is one.
//
// MainFlutterWindow.swift hands these to Dart (app/lib/updates.dart).
// Nothing here uses Flutter, so tools/mac-update-test.sh builds this file
// into a test program of its own and runs every check against real signed
// bundles.
import AppKit
import CryptoKit
import Foundation
import Security

/// The macOS releases' names (.github/workflows/macos-release.yml): tagged
/// `macos-v<major>.<minor>.<build>`, where the build number counts every
/// macOS release and only goes up, so it alone says which release is newer.
/// The app's own version (CFBundleShortVersionString) is
/// `<major>.<minor>.<build>`.
enum Releases {
  static let tagPrefix = "macos-v"

  /// Where checking asks: GitHub's API, and nothing else. Every release, Android's
  /// too, newest first; the newest macOS one among them is the one.
  static let list = URL(
    string: "https://api.github.com/repos/PimpinPumpkin/passbundle-apps/releases?per_page=100")!

  static let apiHost = "api.github.com"

  /// Where a release's files are: PassBundle's own releases, and nowhere else.
  static let downloads = "https://github.com/PimpinPumpkin/passbundle-apps/releases/download/"

  /// The hosts a download may end at: github.com, and where it hands the file.
  static let downloadHosts: Set<String> = [
    "github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com",
  ]

  static let sumsName = "SHA256SUMS"

  /// The most an update may be: far more than PassBundle is (about 30 MB zipped).
  static let maxBytes: Int64 = 300 << 20

  /// The release's file for Macs (universal: Apple silicon and Intel).
  static func zipName(_ version: String) -> String { "PassBundle-\(version)-macos.zip" }

  private static let plain = Set(
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-+")

  /// Whether `url` is a file on PassBundle's own releases:
  /// `downloads<tag>/<file name>`, the tag and the name plain (letters,
  /// digits, dots, hyphens, underscores, and plus signs, never "." or "..").
  static func isReleaseDownload(_ url: String) -> Bool {
    guard url.hasPrefix(downloads) else { return false }
    let parts = url.dropFirst(downloads.count).split(separator: "/", omittingEmptySubsequences: false)
    return parts.count == 2
      && parts.allSatisfy { part in
        !part.isEmpty && part != "." && part != ".." && part.allSatisfy { plain.contains($0) }
      }
  }

  /// Whether `url` is exactly the file `name` on the release tagged `tag`.
  static func isAsset(_ url: String, tag: String, name: String) -> Bool {
    isReleaseDownload(url) && url == downloads + tag + "/" + name
  }

  static func isApiHost(_ host: String?) -> Bool { host?.lowercased() == apiHost }

  static func isDownloadHost(_ host: String?) -> Bool {
    guard let host else { return false }
    return downloadHosts.contains(host.lowercased())
  }

  /// "0.1.12" as (0, 1, 12): three numbers of ASCII digits, nothing else.
  private static func numbers(_ version: String) -> (Int, Int, Int)? {
    let parts = version.split(separator: ".", omittingEmptySubsequences: false)
    guard parts.count == 3,
      parts.allSatisfy({ !$0.isEmpty && $0.count <= 9 && $0.allSatisfy { ("0"..."9").contains($0) } })
    else { return nil }
    return (Int(parts[0])!, Int(parts[1])!, Int(parts[2])!)
  }

  /// The version a release tag names ("0.1.12" from macos-v0.1.12), or nil for another tag.
  static func versionOf(_ tag: String) -> String? {
    guard tag.hasPrefix(tagPrefix) else { return nil }
    let version = String(tag.dropFirst(tagPrefix.count))
    return numbers(version) == nil ? nil : version
  }

  /// The build number of a release tag, or nil for another tag.
  static func buildOfTag(_ tag: String) -> Int? { versionOf(tag).flatMap { numbers($0)?.2 } }

  /// The build number of an app's version; 0 for a build that isn't a
  /// release's (a computer's own, say "0.1.0"), so any release is newer.
  static func buildOfName(_ name: String?) -> Int {
    guard let name else { return 0 }
    return numbers(name.trimmingCharacters(in: .whitespaces))?.2 ?? 0
  }

  /// Whether the release tagged `tag` is newer than the app running as `running`.
  static func isNewer(_ tag: String, than running: String?) -> Bool {
    guard let build = buildOfTag(tag) else { return false }
    return build > buildOfName(running)
  }

  /// The newest macOS release among `tags` (Android's are there too), or nil.
  static func newest(_ tags: [String]) -> String? {
    tags.compactMap { tag in buildOfTag(tag).map { (tag, $0) } }.max { $0.1 < $1.1 }?.0
  }

  /// What's new, from a release's notes: its "- " lines (the commit
  /// subjects the release workflow lists), or the notes as they are if it
  /// has none. At most `max` characters.
  static func changes(_ body: String, max: Int = 4000) -> String {
    let lines = body.components(separatedBy: "\n").map { $0.hasSuffix("\r") ? String($0.dropLast()) : $0 }
      .filter { $0.hasPrefix("- ") }
    let text = lines.isEmpty ? body.trimmingCharacters(in: .whitespacesAndNewlines) : lines.joined(separator: "\n")
    if text.count <= max { return text }
    var cut = String(text.prefix(max))
    if let end = cut.lastIndex(of: "\n") { cut = String(cut[..<end]) }
    return cut + "\n…"
  }

  /// Lowercase SHA-256 hex, or nil if `text` isn't one.
  static func hex(_ text: String?) -> String? {
    guard let text = text?.lowercased(), text.count == 64,
      text.allSatisfy({ ("0"..."9").contains($0) || ("a"..."f").contains($0) })
    else { return nil }
    return text
  }

  /// GitHub's asset `digest` ("sha256:<hex>") as bare lowercase hex, or nil for anything else.
  static func digestOf(_ digest: String?) -> String? {
    guard var text = digest?.trimmingCharacters(in: .whitespaces).lowercased() else { return nil }
    if text.hasPrefix("sha256:") { text.removeFirst("sha256:".count) }
    return hex(text)
  }

  /// SHA256SUMS as `shasum -a 256` and `sha256sum` write it ("<hex>  <name>",
  /// or "<hex> *<name>"), as file name to lowercase hex. Other lines are skipped.
  static func sums(_ text: String) -> [String: String] {
    var found: [String: String] = [:]
    for raw in text.components(separatedBy: "\n") {
      let line = raw.hasSuffix("\r") ? String(raw.dropLast()) : raw
      let chars = Array(line)
      guard chars.count > 66, chars[64] == " ", chars[65] == " " || chars[65] == "*",
        let hash = hex(String(chars[0..<64]))
      else { continue }
      let name = String(chars[66...]).trimmingCharacters(in: .whitespaces)
      if !name.isEmpty { found[name] = hash }
    }
    return found
  }

  /// The most to download for a release that says its file is `size` bytes
  /// (0 when it doesn't say): that size, or `maxBytes`; nil when the
  /// release says more than `maxBytes`, which isn't downloaded at all.
  static func limit(_ size: Int64) -> Int64? {
    if size > maxBytes { return nil }
    return size > 0 ? size : maxBytes
  }
}

/// What went wrong, in the person's words; `reveal` when the update is
/// downloaded and checked, and the person can put it in place themselves.
struct UpdateError: Error {
  let message: String
  var reveal = false

  init(_ message: String, reveal: Bool = false) {
    self.message = message
    self.reveal = reveal
  }
}

/// Why a download stopped.
enum FetchError: Error {
  case refused(String)
  case tooLong
  case tooSlow
  case failed(String)
}

/// One GET by the app itself (not a browser, so nothing marks the file as
/// downloaded from the internet): over HTTPS, following redirects only to
/// HTTPS hosts that `allowed` accepts, at most `max` bytes, within `seconds`.
/// The body goes to `sink` as it comes. A test source's files (`test`) may be
/// anywhere, and may be files on this Mac.
final class Fetch: NSObject, URLSessionDataDelegate {
  private let max: Int64
  private let allowed: (String?) -> Bool
  private let test: Bool
  private let started: (Int64) -> Void
  private let sink: (Data) throws -> Void
  private var received: Int64 = 0
  private var failure: FetchError?
  private var status = 0
  private let done = DispatchSemaphore(value: 0)

  private init(
    max: Int64, allowed: @escaping (String?) -> Bool, test: Bool,
    started: @escaping (Int64) -> Void, sink: @escaping (Data) throws -> Void
  ) {
    self.max = max
    self.allowed = allowed
    self.test = test
    self.started = started
    self.sink = sink
  }

  /// Whether a request may start at, be redirected to, or end at `url`.
  static func permits(_ url: URL?, allowed: (String?) -> Bool, test: Bool) -> Bool {
    guard let url else { return false }
    if url.isFileURL { return test }
    return url.scheme?.lowercased() == "https" && (test || allowed(url.host))
  }

  /// False when there's nothing there (a 404, as with no releases yet).
  static func get(
    _ url: URL, max: Int64, allowed: @escaping (String?) -> Bool, test: Bool, seconds: TimeInterval,
    started: @escaping (Int64) -> Void = { _ in }, sink: @escaping (Data) throws -> Void
  ) throws -> Bool {
    guard permits(url, allowed: allowed, test: test) else { throw FetchError.refused("only HTTPS, from GitHub") }
    if url.isFileURL { return try readFile(url, max: max, seconds: seconds, started: started, sink: sink) }
    let config = URLSessionConfiguration.ephemeral
    config.requestCachePolicy = .reloadIgnoringLocalAndRemoteCacheData
    config.urlCache = nil
    config.httpCookieStorage = nil
    config.httpShouldSetCookies = false
    config.urlCredentialStorage = nil
    config.timeoutIntervalForRequest = 30
    config.timeoutIntervalForResource = Swift.max(seconds, 1)
    config.waitsForConnectivity = false
    config.tlsMinimumSupportedProtocolVersion = .TLSv12
    let fetch = Fetch(max: max, allowed: allowed, test: test, started: started, sink: sink)
    let queue = OperationQueue()
    queue.maxConcurrentOperationCount = 1
    let session = URLSession(configuration: config, delegate: fetch, delegateQueue: queue)
    defer { session.invalidateAndCancel() }
    var request = URLRequest(url: url)
    request.setValue("PassBundle", forHTTPHeaderField: "User-Agent")
    if url.host?.lowercased() == Releases.apiHost {
      request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
      request.setValue("2022-11-28", forHTTPHeaderField: "X-GitHub-Api-Version")
    }
    let task = session.dataTask(with: request)
    task.resume()
    if fetch.done.wait(timeout: .now() + Swift.max(seconds, 1) + 5) == .timedOut {
      queue.addOperation { if fetch.failure == nil { fetch.failure = .tooSlow } }
      task.cancel()
      fetch.done.wait()
    }
    queue.waitUntilAllOperationsAreFinished()
    if let failure = fetch.failure { throw failure }
    return fetch.status != 404
  }

  /// A test source's file on this Mac, with the same limits.
  private static func readFile(
    _ url: URL, max: Int64, seconds: TimeInterval, started: (Int64) -> Void, sink: (Data) throws -> Void
  ) throws -> Bool {
    guard let handle = try? FileHandle(forReadingFrom: url) else { return false }
    defer { try? handle.close() }
    let deadline = Date().addingTimeInterval(seconds)
    let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize).map(Int64.init) ?? -1
    if size > max { throw FetchError.tooLong }
    started(size)
    var read: Int64 = 0
    while true {
      if Date() > deadline { throw FetchError.tooSlow }
      guard let data = try handle.read(upToCount: 64 << 10), !data.isEmpty else { break }
      read += Int64(data.count)
      if read > max { throw FetchError.tooLong }
      try sink(data)
    }
    return true
  }

  func urlSession(
    _ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void
  ) {
    if Fetch.permits(request.url, allowed: allowed, test: test) {
      completionHandler(request)
    } else {
      failure = .refused("redirected away from GitHub")
      completionHandler(nil)
      task.cancel()
    }
  }

  func urlSession(
    _ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void
  ) {
    guard failure == nil, let http = response as? HTTPURLResponse,
      Fetch.permits(http.url, allowed: allowed, test: test)
    else {
      if failure == nil { failure = .refused("ended away from GitHub") }
      return completionHandler(.cancel)
    }
    status = http.statusCode
    if status == 404 { return completionHandler(.cancel) }
    guard (200..<300).contains(status) else {
      failure = .failed("HTTP \(status)")
      return completionHandler(.cancel)
    }
    if response.expectedContentLength > max {
      failure = .tooLong
      return completionHandler(.cancel)
    }
    started(response.expectedContentLength)
    completionHandler(.allow)
  }

  func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
    guard failure == nil else { return }
    received += Int64(data.count)
    if received > max {
      failure = .tooLong
      return dataTask.cancel()
    }
    do {
      try sink(data)
    } catch {
      failure = .failed("couldn't keep the download")
      dataTask.cancel()
    }
  }

  func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
    if failure == nil, status != 404, let error {
      failure = (error as? URLError)?.code == .timedOut ? .tooSlow : .failed(error.localizedDescription)
    }
    done.signal()
  }
}

/// The copy of PassBundle being updated: where it is, and what an update
/// must be to take its place.
struct InstalledApp {
  let bundleURL: URL
  let bundleID: String
  let version: String?
  /// What an update's signature must meet: this copy's designated
  /// requirement. Nil when this copy isn't signed with a certificate (a
  /// build of its own, signed ad hoc, whose requirement is its own hash).
  let requirement: SecRequirement?

  /// The running app: its requirement from its own running code.
  static func running() -> InstalledApp {
    let bundle = Bundle.main
    var requirement: SecRequirement?
    var me: SecCode?
    if SecCodeCopySelf([], &me) == errSecSuccess, let me,
      SecCodeCheckValidity(me, [], nil) == errSecSuccess
    {
      var mine: SecStaticCode?
      // The running code's own designated requirement (SecCodeRef is a
      // SecStaticCodeRef as far as Code Signing Services' C API goes).
      if SecCodeCopyStaticCode(me, [], &mine) == errSecSuccess, let mine, !isAdHoc(mine) {
        SecCodeCopyDesignatedRequirement(unsafeBitCast(me, to: SecStaticCode.self), [], &requirement)
      }
    }
    return InstalledApp(
      bundleURL: bundle.bundleURL, bundleID: bundle.bundleIdentifier ?? "",
      version: bundle.infoDictionary?["CFBundleShortVersionString"] as? String, requirement: requirement)
  }

  /// A copy on disk, for tests: its requirement from its signature.
  static func onDisk(_ url: URL) -> InstalledApp {
    var requirement: SecRequirement?
    var code: SecStaticCode?
    if SecStaticCodeCreateWithPath(url as CFURL, [], &code) == errSecSuccess, let code,
      SecStaticCodeCheckValidity(code, [], nil) == errSecSuccess, !isAdHoc(code)
    {
      SecCodeCopyDesignatedRequirement(code, [], &requirement)
    }
    let info = NSDictionary(contentsOf: url.appendingPathComponent("Contents/Info.plist")) as? [String: Any]
    return InstalledApp(
      bundleURL: url, bundleID: info?["CFBundleIdentifier"] as? String ?? "",
      version: info?["CFBundleShortVersionString"] as? String, requirement: requirement)
  }

  static func isAdHoc(_ code: SecStaticCode) -> Bool {
    var info: CFDictionary?
    guard SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &info) == errSecSuccess,
      let dict = info as? [String: Any]
    else { return true }
    let flags = (dict[kSecCodeInfoFlags as String] as? NSNumber)?.uint32Value ?? 0
    return flags & SecCodeSignatureFlags.adhoc.rawValue != 0
      || (dict[kSecCodeInfoCertificates as String] as? [Any])?.isEmpty != false
  }
}

/// Settings of the updater's own: whether it checks once a day, and when it
/// last did. The app's preferences, or (tests) memory.
final class UpdatePrefs {
  private let defaults: UserDefaults?
  private var memory: [String: Any] = [:]

  init(_ defaults: UserDefaults?) { self.defaults = defaults }

  var auto: Bool {
    get { (defaults?.object(forKey: "updates.auto") ?? memory["auto"]) as? Bool ?? true }
    set { if let defaults { defaults.set(newValue, forKey: "updates.auto") } else { memory["auto"] = newValue } }
  }

  var lastCheck: Double {
    get { (defaults?.object(forKey: "updates.lastCheck") ?? memory["lastCheck"]) as? Double ?? 0 }
    set { if let defaults { defaults.set(newValue, forKey: "updates.lastCheck") } else { memory["lastCheck"] = newValue } }
  }
}

final class Updater {
  /// An update the last check found: the release and its files.
  struct Offer {
    let version: String
    let zipName: String
    let zipURL: URL
    let size: Int64
    /// GitHub's own SHA-256 of the file, when the release answer has it.
    let digest: String?
    let sumsURL: URL
    let notes: String
    /// From a test source, whose files may be anywhere.
    let test: Bool
  }

  /// An update in place: where, the old copy in the Trash, and the old copy's
  /// helpers still running (the desktop agent), to stop before restarting.
  struct Replaced {
    let app: URL
    let trashed: URL
    let helpers: [pid_t]
  }

  private let installed: () -> InstalledApp
  private let source: URL?
  private let debug: Bool
  private let workDir: URL
  private let trash: (URL) throws -> URL
  private let translocated: (URL) -> Bool
  private let prefs: UpdatePrefs
  /// The longest a download may take, however it goes.
  var downloadSeconds: TimeInterval = 15 * 60
  /// How far a download is, 0 to 100 (from the download's own thread).
  var progress: ((Int) -> Void)?

  private let lock = NSLock()
  private var offer: Offer?
  private var downloading = false
  /// The update, downloaded and checked, when it couldn't take this copy's
  /// place: for the person to put there themselves.
  private var checkedCopy: URL?

  init(
    installed: @escaping () -> InstalledApp, source: URL?, debug: Bool, workDir: URL,
    trash: @escaping (URL) throws -> URL, prefs: UpdatePrefs,
    translocated: @escaping (URL) -> Bool = Updater.isTranslocated
  ) {
    self.installed = installed
    self.source = source
    self.debug = debug
    self.workDir = workDir
    self.trash = trash
    self.prefs = prefs
    self.translocated = translocated
  }

  /// The updater for the running app.
  static func forThisApp() -> Updater {
    #if DEBUG
      let source = testSource()
      let debug = true
    #else
      let source: URL? = nil
      let debug = false
    #endif
    let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
    return Updater(
      installed: InstalledApp.running, source: source, debug: debug,
      workDir: caches.appendingPathComponent("com.passbundle.app/updates", isDirectory: true),
      trash: { url in
        var trashed: NSURL?
        try FileManager.default.trashItem(at: url, resultingItemURL: &trashed)
        return (trashed as URL?) ?? url
      }, prefs: UpdatePrefs(.standard))
  }

  #if DEBUG
    /// Debug builds only: where a test's releases are (`update-source` in
    /// the app's folder, PB_DATA_DIR or Application Support/PassBundle,
    /// holding a URL, which may be a file:// one), instead of GitHub. Release
    /// builds always ask GitHub.
    static func testSource() -> URL? {
      let dir =
        ProcessInfo.processInfo.environment["PB_DATA_DIR"].map { URL(fileURLWithPath: $0) }
        ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("PassBundle")
      guard let text = try? String(contentsOf: dir.appendingPathComponent("update-source"), encoding: .utf8)
      else { return nil }
      let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
      return trimmed.isEmpty ? nil : URL(string: trimmed)
    }
  #endif

  /// This copy's version, no store (PassBundle for the Mac comes only from
  /// GitHub), whether it checks once a day, and that updating restarts it.
  func status() -> [String: Any] {
    var answer: [String: Any] = ["auto": prefs.auto, "restarts": true]
    if let version = installed().version { answer["version"] = version }
    return answer
  }

  func setAuto(_ on: Bool) { locked { prefs.auto = on } }

  /// Looks for a newer release (slow: off the main thread): `{ok, available,
  /// version, notes}`, or `{ok: false, error}`. `quiet` is the check when
  /// the app starts, which happens at most once a day, and never in a debug
  /// build or with the person's daily check off: then `{ok, available:
  /// false, skipped: true}`.
  func check(quiet: Bool) -> [String: Any] {
    let skipped: [String: Any] = ["ok": true, "available": false, "skipped": true]
    let now = Date().timeIntervalSince1970
    let go: Bool = locked {
      if quiet {
        if debug || !prefs.auto { return false }
        let last = prefs.lastCheck
        if now >= last && now - last < 24 * 60 * 60 { return false }
      }
      prefs.lastCheck = now
      offer = nil
      return true
    }
    if !go { return skipped }
    do {
      guard let found = try newer() else { return ["ok": true, "available": false] }
      locked { offer = found }
      return ["ok": true, "available": true, "version": found.version, "notes": found.notes]
    } catch let error as UpdateError {
      return ["ok": false, "error": error.message]
    } catch {
      return ["ok": false, "error": "Couldn't check for updates. Check your connection and try again."]
    }
  }

  /// The newest macOS release, if it's newer than this copy and has its files.
  private func newer() throws -> Offer? {
    let test = source != nil
    var body = Data()
    guard try Fetch.get(source ?? Releases.list, max: 4 << 20, allowed: Releases.isApiHost, test: test, seconds: 60, sink: { body.append($0) })
    else { return nil }
    let json = try JSONSerialization.jsonObject(with: body)
    // GitHub's list; a test source may be one release.
    let releases = (json as? [[String: Any]]) ?? (json as? [String: Any]).map { [$0] } ?? []
    let shipped = releases.filter { ($0["draft"] as? Bool) != true && ($0["prerelease"] as? Bool) != true }
    guard let tag = Releases.newest(shipped.compactMap { $0["tag_name"] as? String }),
      let release = shipped.first(where: { $0["tag_name"] as? String == tag }),
      let version = Releases.versionOf(tag)
    else { return nil }
    guard Releases.isNewer(tag, than: installed().version) else { return nil }
    let assets = release["assets"] as? [[String: Any]] ?? []
    let name = Releases.zipName(version)
    guard let zip = assets.first(where: { $0["name"] as? String == name }),
      let zipURL = zip["browser_download_url"] as? String
    else { throw UpdateError("PassBundle \(version) has no download for the Mac.") }
    guard let sums = assets.first(where: { $0["name"] as? String == Releases.sumsName }),
      let sumsURL = sums["browser_download_url"] as? String
    else { throw UpdateError("PassBundle \(version) has no checksums, so it can't be installed from here.") }
    // Only these files on PassBundle's own releases (a test's may be anywhere).
    if !test
      && !(Releases.isAsset(zipURL, tag: tag, name: name) && Releases.isAsset(sumsURL, tag: tag, name: Releases.sumsName))
    {
      throw UpdateError("PassBundle \(version)'s files aren't on PassBundle's releases, so it can't be installed from here.")
    }
    guard let zipLink = URL(string: zipURL), let sumsLink = URL(string: sumsURL) else { return nil }
    return Offer(
      version: version, zipName: name, zipURL: zipLink, size: (zip["size"] as? NSNumber)?.int64Value ?? 0,
      digest: Releases.digestOf(zip["digest"] as? String), sumsURL: sumsLink,
      notes: Releases.changes(release["body"] as? String ?? ""), test: test)
  }

  /// Downloads the update the last check found, checks it, and puts it in
  /// this copy's place (slow: off the main thread): `{ok, restart}` once
  /// it's there, and then PassBundle restarts; or `{ok: false, error,
  /// reveal}`, where `reveal` says the checked update can be shown in Finder.
  func install() -> (answer: [String: Any], replaced: Replaced?) {
    let started: Offer? = locked {
      guard let offer, !downloading else { return nil }
      downloading = true
      checkedCopy = nil
      return offer
    }
    guard let offer = started else {
      let busy = locked { downloading }
      return (fail(busy ? "The update is already downloading." : "Check for updates first."), nil)
    }
    defer { locked { downloading = false } }
    do {
      let replaced = try fetchCheckAndReplace(offer)
      return (["ok": true, "restart": true], replaced)
    } catch let error as UpdateError {
      return (["ok": false, "error": error.message, "reveal": error.reveal && revealable() != nil], nil)
    } catch {
      return (fail("Couldn't download the update. Check your connection and try again."), nil)
    }
  }

  /// The checked update, when it couldn't take this copy's place.
  func revealable() -> URL? {
    locked { checkedCopy }.flatMap { FileManager.default.fileExists(atPath: $0.path) ? $0 : nil }
  }

  private func fail(_ error: String) -> [String: Any] { ["ok": false, "error": error] }

  private func locked<T>(_ body: () throws -> T) rethrows -> T {
    lock.lock()
    defer { lock.unlock() }
    return try body()
  }

  private func fetchCheckAndReplace(_ offer: Offer) throws -> Replaced {
    let fm = FileManager.default
    let app = installed()
    guard let requirement = app.requirement else {
      throw UpdateError(
        "This copy of PassBundle isn't signed the way PassBundle's releases are (it was built on a computer), so it can't update itself. Download PassBundle \(offer.version) from GitHub instead."
      )
    }
    // Never more than a release says, nor than any PassBundle could be.
    guard let limit = Releases.limit(offer.size) else {
      throw UpdateError("The update is larger than PassBundle accepts, so it wasn't downloaded.")
    }
    var sumsText = Data()
    let gotSums: Bool
    do {
      gotSums = try Fetch.get(
        offer.sumsURL, max: 64 << 10, allowed: Releases.isDownloadHost, test: offer.test, seconds: 60,
        sink: { sumsText.append($0) })
    } catch {
      throw UpdateError("Couldn't download the update's checksums.")
    }
    guard gotSums else { throw UpdateError("Couldn't download the update's checksums.") }
    guard let expected = Releases.sums(String(decoding: sumsText, as: UTF8.self))[offer.zipName] else {
      throw UpdateError("The update isn't in its release's checksums, so it wasn't installed.")
    }
    if let digest = offer.digest, digest != expected {
      throw UpdateError("The update's checksums don't agree, so it wasn't installed.")
    }

    // A folder of the updater's own, empty, and names of its own: a
    // release's file name never becomes a path here.
    try? fm.removeItem(at: workDir)
    try fm.createDirectory(at: workDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    if let free = try? workDir.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
      .volumeAvailableCapacityForImportantUsage, free < 3 * limit + (64 << 20)
    {
      throw UpdateError("There isn't enough space on this Mac for the update.")
    }
    let zip = workDir.appendingPathComponent("update.zip")
    guard fm.createFile(atPath: zip.path, contents: nil, attributes: [.posixPermissions: 0o600]) else {
      throw UpdateError("Couldn't keep the download.")
    }
    try download(offer, to: zip, limit: limit)
    // The file as it is on disk, which is what gets unpacked.
    let size = (try? zip.resourceValues(forKeys: [.fileSizeKey]).fileSize).map(Int64.init) ?? 0
    guard size > 4, offer.size <= 0 || size == offer.size, Updater.isZip(zip), Updater.sha256(zip) == expected
    else {
      try? fm.removeItem(at: zip)
      throw UpdateError("The download didn't match the release, so it wasn't installed.")
    }
    progress?(100)

    // What it holds once unpacked, from its own list, before anything is
    // unpacked: a zip that would unpack far bigger than it is isn't.
    guard let listing = Updater.unpackedSize(zip) else {
      throw UpdateError("The download isn't a PassBundle app, so it wasn't installed.")
    }
    guard listing.files <= Updater.maxUnpackedFiles, listing.bytes <= Updater.unpackedLimit(zipSize: size) else {
      throw UpdateError("The update would unpack to more than PassBundle accepts, so it wasn't installed.")
    }
    if let free = try? workDir.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
      .volumeAvailableCapacityForImportantUsage, free < listing.bytes + (64 << 20)
    {
      throw UpdateError("There isn't enough space on this Mac for the update.")
    }
    let unpacked = workDir.appendingPathComponent("unpacked", isDirectory: true)
    try fm.createDirectory(at: unpacked, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    guard Updater.run("/usr/bin/ditto", ["-x", "-k", zip.path, unpacked.path]) else {
      throw UpdateError("The download isn't a PassBundle app, so it wasn't installed.")
    }
    try? fm.removeItem(at: zip)
    let items = (try? fm.contentsOfDirectory(at: unpacked, includingPropertiesForKeys: nil)) ?? []
    guard items.count == 1, let new = items.first, new.pathExtension == "app", Updater.isRealFolder(new) else {
      throw UpdateError("The download isn't a PassBundle app, so it wasn't installed.")
    }
    if let problem = Updater.problem(
      app: new, meets: requirement, id: app.bundleID, version: offer.version, newerThan: app.version)
    {
      throw UpdateError(problem)
    }
    // Checked: what macOS would ask about a download (it wasn't marked as
    // one, coming from the app itself) is answered.
    Updater.removeQuarantine(new)
    locked { checkedCopy = new }

    // The old copy's helpers (the desktop agent) keep running from it, in
    // the Trash, until they're stopped; which they are, as PassBundle restarts.
    let helpers = Updater.processes(under: app.bundleURL.appendingPathComponent("Contents/Helpers", isDirectory: true))
    let trashed = try Updater.replace(app.bundleURL, with: new, trash: trash, translocated: translocated) { placed in
      Updater.problem(app: placed, meets: requirement, id: app.bundleID, version: offer.version, newerThan: app.version)
    }
    locked { checkedCopy = nil }
    try? fm.removeItem(at: workDir)
    return Replaced(app: app.bundleURL, trashed: trashed, helpers: helpers)
  }

  /// Downloads the update's file into `to`, at most `limit` bytes and
  /// `downloadSeconds`, saying how far it is.
  private func download(_ offer: Offer, to file: URL, limit: Int64) throws {
    guard let handle = try? FileHandle(forWritingTo: file) else { throw UpdateError("Couldn't keep the download.") }
    var total = offer.size
    var read: Int64 = 0
    var shown = -1
    let progress = self.progress
    let found: Bool
    do {
      defer { try? handle.close() }
      found = try Fetch.get(
        offer.zipURL, max: limit, allowed: Releases.isDownloadHost, test: offer.test, seconds: downloadSeconds,
        started: { length in if length > 0 { total = length } },
        sink: { data in
          try handle.write(contentsOf: data)
          read += Int64(data.count)
          if total > 0 {
            let percent = Int(min(99, 100 * read / total))
            if percent != shown {
              shown = percent
              progress?(percent)
            }
          }
        })
    } catch FetchError.tooLong {
      throw UpdateError("The download is larger than its release says, so it wasn't installed.")
    } catch FetchError.tooSlow {
      throw UpdateError("The download took too long, so it stopped. Try again later.")
    } catch {
      throw UpdateError("Couldn't download the update. Check your connection and try again.")
    }
    guard found else { throw UpdateError("Couldn't download the update: its file isn't there.") }
  }

  /// What's wrong with `app` as an update of this copy, in the person's
  /// words, or nil: its signature is whole (every architecture, everything
  /// nested in it, strictly) and meets `requirement`, this copy's own; it's
  /// `id` at `version`, as its sealed Info.plist says; and that's newer
  /// than `running`.
  static func problem(
    app: URL, meets requirement: SecRequirement, id: String, version: String, newerThan running: String?
  ) -> String? {
    var code: SecStaticCode?
    guard SecStaticCodeCreateWithPath(app as CFURL, [], &code) == errSecSuccess, let code else {
      return "The download isn't an app, so it wasn't installed."
    }
    let flags = SecCSFlags(
      rawValue: kSecCSCheckAllArchitectures | kSecCSStrictValidate | kSecCSCheckNestedCode | kSecCSRestrictSymlinks)
    let status = SecStaticCodeCheckValidity(code, flags, requirement)
    if status == errSecCSReqFailed {
      return "The update is signed with a different certificate than this copy of PassBundle, so it can't replace it. To move to it, download PassBundle \(version) from GitHub and replace this copy yourself."
    }
    if status != errSecSuccess {
      return "The update's signature doesn't check out, so it wasn't installed."
    }
    var info: CFDictionary?
    guard SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &info) == errSecSuccess,
      let dict = info as? [String: Any], let plist = dict[kSecCodeInfoPList as String] as? [String: Any]
    else { return "The update's signature doesn't check out, so it wasn't installed." }
    guard dict[kSecCodeInfoIdentifier as String] as? String == id, plist["CFBundleIdentifier"] as? String == id else {
      return "The download isn't PassBundle, so it wasn't installed."
    }
    guard plist["CFBundleShortVersionString"] as? String == version else {
      return "The download isn't the version its release says, so it wasn't installed."
    }
    guard Releases.buildOfName(version) > Releases.buildOfName(running) else {
      return "The download isn't newer than this PassBundle."
    }
    return nil
  }

  /// The most a release's zip may hold once unpacked: 1 GB, and 8 times the
  /// zip (PassBundle unpacks to about 2.5 times its zip), but never less
  /// than 64 MB; and at most 100,000 files.
  static let maxUnpacked: Int64 = 1 << 30
  static let maxUnpackedFiles = 100_000

  static func unpackedLimit(zipSize: Int64) -> Int64 {
    min(maxUnpacked, max(8 * zipSize, 64 << 20))
  }

  /// What a zip's files add up to once unpacked, and how many there are, as
  /// its central directory lists them; nil for anything that isn't a plain
  /// zip (ZIP64 too, which no release needs, being far smaller than 4 GB).
  static func unpackedSize(_ zip: URL) -> (bytes: Int64, files: Int)? {
    guard let handle = try? FileHandle(forReadingFrom: zip) else { return nil }
    defer { try? handle.close() }
    guard let size = try? handle.seekToEnd(), size >= 22 else { return nil }
    // The end of the central directory: 22 bytes, then a comment of up to 64 KB.
    let tailLength = min(size, 22 + 0xFFFF)
    guard (try? handle.seek(toOffset: size - tailLength)) != nil,
      let tail = try? handle.read(upToCount: Int(tailLength)), tail.count == Int(tailLength)
    else { return nil }
    let t = [UInt8](tail)
    func u16(_ b: [UInt8], _ i: Int) -> Int { Int(b[i]) | Int(b[i + 1]) << 8 }
    func u32(_ b: [UInt8], _ i: Int) -> Int64 {
      Int64(b[i]) | Int64(b[i + 1]) << 8 | Int64(b[i + 2]) << 16 | Int64(b[i + 3]) << 24
    }
    guard let end = stride(from: t.count - 22, through: 0, by: -1).first(where: {
      t[$0] == 0x50 && t[$0 + 1] == 0x4B && t[$0 + 2] == 0x05 && t[$0 + 3] == 0x06
    }) else { return nil }
    let files = u16(t, end + 10)
    let directorySize = u32(t, end + 12)
    let directoryAt = u32(t, end + 16)
    let endAt = Int64(size) - Int64(tailLength) + Int64(end)
    guard files != 0xFFFF, directorySize != 0xFFFF_FFFF, directoryAt != 0xFFFF_FFFF,
      directorySize <= 64 << 20, directoryAt + directorySize <= endAt
    else { return nil }
    guard (try? handle.seek(toOffset: UInt64(directoryAt))) != nil,
      let directory = try? handle.read(upToCount: Int(directorySize)), directory.count == Int(directorySize)
    else { return nil }
    let d = [UInt8](directory)
    var at = 0
    var bytes: Int64 = 0
    for _ in 0..<files {
      guard at + 46 <= d.count, d[at] == 0x50, d[at + 1] == 0x4B, d[at + 2] == 0x01, d[at + 3] == 0x02 else {
        return nil
      }
      let unpacked = u32(d, at + 24)
      if unpacked == 0xFFFF_FFFF { return nil }
      bytes += unpacked
      at += 46 + u16(d, at + 28) + u16(d, at + 30) + u16(d, at + 32)
    }
    return (bytes, files)
  }

  /// Whether macOS runs `url` from a temporary, read-only copy (App
  /// Translocation: an app opened where it was downloaded, still marked as
  /// a download), as Security's SecTranslocateIsTranslocatedURL says. It
  /// has no public header, so it's found as Sparkle finds it; where it isn't
  /// there, the copy can't be written to either, which the updater also
  /// checks.
  static func isTranslocated(_ url: URL) -> Bool {
    typealias Check = @convention(c) (
      CFURL, UnsafeMutablePointer<Bool>, UnsafeMutablePointer<Unmanaged<CFError>?>?
    ) -> DarwinBoolean
    guard let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "SecTranslocateIsTranslocatedURL") else {
      return false
    }
    var translocated = false
    return unsafeBitCast(symbol, to: Check.self)(url as CFURL, &translocated, nil).boolValue && translocated
  }

  /// Puts `new` where `target` is: next to it first (one disk, so the last
  /// step is a rename), then `target` to the Trash, then `new` in its place,
  /// checked again there with `check`. Anything that fails on the way puts
  /// `target` back. The Trash's copy, once it's done.
  static func replace(
    _ target: URL, with new: URL, trash: (URL) throws -> URL, translocated: (URL) -> Bool = isTranslocated,
    check: (URL) -> String?
  ) throws -> URL {
    let fm = FileManager.default
    let parent = target.deletingLastPathComponent()
    let folder = fm.displayName(atPath: parent.path)
    if translocated(target) {
      throw UpdateError(
        "macOS is running this copy of PassBundle from a temporary place, since it was opened where it was downloaded, so it can't replace itself. The new version is downloaded and checked: show it in Finder, and drag it to Applications.",
        reveal: true)
    }
    guard fm.isWritableFile(atPath: parent.path), fm.isWritableFile(atPath: target.path) else {
      throw UpdateError(
        "PassBundle can't replace itself in \(folder), which only an administrator can change. The new version is downloaded and checked: show it in Finder, and drag it to \(folder) yourself.",
        reveal: true)
    }
    let staged = parent.appendingPathComponent(".PassBundle-update.app", isDirectory: true)
    try? fm.removeItem(at: staged)
    do {
      try fm.moveItem(at: new, to: staged)
    } catch {
      throw UpdateError("Couldn't put the new version in \(folder).", reveal: true)
    }
    let trashed: URL
    do {
      trashed = try trash(target)
    } catch {
      try? fm.moveItem(at: staged, to: new)
      throw UpdateError(
        "Couldn't move this copy of PassBundle to the Trash, so the new version isn't in place. Show it in Finder, and put it in \(folder) yourself.",
        reveal: true)
    }
    do {
      try fm.moveItem(at: staged, to: target)
    } catch {
      try? fm.moveItem(at: trashed, to: target)
      try? fm.moveItem(at: staged, to: new)
      throw UpdateError("Couldn't put the new version in place, so this copy stays.", reveal: true)
    }
    if let problem = check(target) {
      // Not what was checked: it goes, and this copy comes back.
      try? fm.removeItem(at: target)
      try? fm.moveItem(at: trashed, to: target)
      throw UpdateError(problem)
    }
    return trashed
  }

  /// The processes running a program from inside `folder`.
  static func processes(under folder: URL) -> [pid_t] {
    // As the kernel names programs: /private/tmp, not /tmp.
    guard let real = realpath(folder.path, nil) else { return [] }
    let prefix = String(cString: real) + "/"
    free(real)
    var pids = [pid_t](repeating: 0, count: 8192)
    let count = proc_listallpids(&pids, Int32(pids.count * MemoryLayout<pid_t>.size))
    guard count > 0 else { return [] }
    var found: [pid_t] = []
    var path = [CChar](repeating: 0, count: 4 * Int(MAXPATHLEN))
    for pid in pids.prefix(Int(count)) where pid > 0 && pid != getpid() {
      if proc_pidpath(pid, &path, UInt32(path.count)) > 0, String(cString: path).hasPrefix(prefix) {
        found.append(pid)
      }
    }
    return found
  }

  /// Opens `app` (with `opener`) once the processes `pids` have ended, or
  /// after 30 seconds: a small shell, started now, that outlives this one.
  static func relaunch(_ app: URL, after pids: [pid_t], opener: String = "/usr/bin/open") throws {
    let script = """
      opener=$1; app=$2; shift 2
      i=0
      while [ "$i" -lt 150 ]; do
        alive=
        for pid in "$@"; do kill -0 "$pid" 2> /dev/null && alive=1; done
        [ -z "$alive" ] && break
        sleep 0.2
        i=$((i + 1))
      done
      exec "$opener" "$app"
      """
    let shell = Process()
    shell.executableURL = URL(fileURLWithPath: "/bin/sh")
    shell.arguments = ["-c", script, "passbundle-restart", opener, app.path] + pids.map { String($0) }
    shell.standardInput = FileHandle.nullDevice
    shell.standardOutput = FileHandle.nullDevice
    shell.standardError = FileHandle.nullDevice
    try shell.run()
  }

  /// Removes macOS's mark of a download from everything in `app`.
  static func removeQuarantine(_ app: URL) {
    removexattr(app.path, "com.apple.quarantine", XATTR_NOFOLLOW)
    guard let all = FileManager.default.enumerator(at: app, includingPropertiesForKeys: nil) else { return }
    for case let item as URL in all {
      removexattr(item.path, "com.apple.quarantine", XATTR_NOFOLLOW)
    }
  }

  static func hasQuarantine(_ url: URL) -> Bool {
    getxattr(url.path, "com.apple.quarantine", nil, 0, 0, XATTR_NOFOLLOW) >= 0
  }

  static func isZip(_ file: URL) -> Bool {
    guard let handle = try? FileHandle(forReadingFrom: file) else { return false }
    defer { try? handle.close() }
    return (try? handle.read(upToCount: 2)) == Data("PK".utf8)
  }

  static func sha256(_ file: URL) -> String? {
    guard let handle = try? FileHandle(forReadingFrom: file) else { return nil }
    defer { try? handle.close() }
    var hasher = SHA256()
    while let data = try? handle.read(upToCount: 1 << 20), !data.isEmpty {
      hasher.update(data: data)
    }
    return hasher.finalize().map { String(format: "%02x", $0) }.joined()
  }

  /// A folder, not a link to one.
  static func isRealFolder(_ url: URL) -> Bool {
    let values = try? url.resourceValues(forKeys: [.isSymbolicLinkKey, .isDirectoryKey])
    return values?.isDirectory == true && values?.isSymbolicLink != true
  }

  static func run(_ program: String, _ arguments: [String]) -> Bool {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: program)
    process.arguments = arguments
    process.standardInput = FileHandle.nullDevice
    process.standardOutput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
    do {
      try process.run()
    } catch {
      return false
    }
    process.waitUntilExit()
    return process.terminationStatus == 0
  }

  /// Shows the checked update in Finder, for the person to put in place.
  func reveal() -> Bool {
    guard let copy = revealable() else { return false }
    NSWorkspace.shared.activateFileViewerSelecting([copy])
    return true
  }
}
