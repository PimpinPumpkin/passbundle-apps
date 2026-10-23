// The Mac updater (app/macos/Runner/Updater.swift) on its own: built with
// this file by tools/mac-update-test.sh, which makes the signed bundles it
// updates (fixtures/*.app). First the release names, then each way an update
// goes: an older copy updated to a newer one, and every download it must
// refuse, with the older copy left as it was.
//
//   update-test <fixtures folder>
import Foundation
import Security

var failures = 0

func expect(_ ok: Bool, _ what: String, line: Int = #line) {
  if ok {
    print("ok    \(what)")
  } else {
    failures += 1
    print("FAIL  \(what) (main.swift:\(line))")
  }
}

let fm = FileManager.default

// MARK: Release names

func testReleases() {
  expect(Releases.versionOf("macos-v0.1.12") == "0.1.12", "a tag gives its version")
  expect(Releases.buildOfTag("macos-v0.1.12") == 12 && Releases.buildOfTag("macos-v0.2.240") == 240, "a tag gives its build")
  expect(
    [
      "android-v0.1.12", "v0.1.12", "macos-v0.1.12-rc", "macos-v0.1", "macos-v0.1.+5", "macos-v0.1.-5",
      "macos-v0.1.١", "macos-v.1.2", "macos-v0..2", " macos-v0.1.2",
    ].allSatisfy { Releases.buildOfTag($0) == nil },
    "other platforms' tags, and anything not three plain numbers, aren't Mac releases")
  expect(Releases.isNewer("macos-v0.1.13", than: "0.1.12"), "a higher build is newer")
  expect(
    !Releases.isNewer("macos-v0.1.12", than: "0.1.12") && !Releases.isNewer("macos-v0.1.11", than: "0.1.12"),
    "the same or a lower build isn't")
  expect(Releases.isNewer("macos-v0.2.14", than: "0.1.13"), "a new version line keeps counting builds")
  expect(!Releases.isNewer("android-v0.1.99", than: "0.1.12"), "an Android release is never a Mac update")
  expect(
    Releases.isNewer("macos-v0.1.1", than: "0.1.0") && Releases.isNewer("macos-v0.1.1", than: nil)
      && Releases.isNewer("macos-v0.1.1", than: "dev"),
    "a build of its own is older than any release")
  expect(
    Releases.newest(["android-v0.1.40", "macos-v0.1.9", "macos-v0.1.12", "macos-v0.1.10", "canary"]) == "macos-v0.1.12",
    "the newest Mac release among Android's, by build number")
  expect(Releases.newest(["android-v0.1.40"]) == nil, "no Mac release among Android's is none")

  let base = "https://github.com/PimpinPumpkin/passbundle-apps/releases/download"
  expect(Releases.isReleaseDownload("\(base)/macos-v0.1.12/PassBundle-0.1.12-macos.zip"), "a file on PassBundle's releases")
  expect(
    Releases.isAsset("\(base)/macos-v0.1.12/SHA256SUMS", tag: "macos-v0.1.12", name: "SHA256SUMS")
      && !Releases.isAsset("\(base)/macos-v0.1.11/SHA256SUMS", tag: "macos-v0.1.12", name: "SHA256SUMS")
      && !Releases.isAsset("\(base)/macos-v0.1.12/other.zip", tag: "macos-v0.1.12", name: "SHA256SUMS"),
    "an asset is exactly its release's file")
  expect(
    [
      "https://github.com/someone/passbundle-apps/releases/download/macos-v0.1.12/x.zip",
      "https://github.com/PimpinPumpkin/passbundle-apps-fork/releases/download/macos-v0.1.12/x.zip",
      "https://github.com.example.test/PimpinPumpkin/passbundle-apps/releases/download/a/b.zip",
      "http://github.com/PimpinPumpkin/passbundle-apps/releases/download/a/b.zip",
      "\(base)/../../../other/repo/b.zip", "\(base)/macos-v0.1.12/../b.zip", "\(base)/%2e%2e/b.zip",
      "\(base)/macos-v0.1.12/b.zip?x=1", "\(base)/macos-v0.1.12/sub/b.zip", "\(base)/macos-v0.1.12/",
      "\(base)/b.zip",
    ].allSatisfy { !Releases.isReleaseDownload($0) },
    "nothing else: other repositories and hosts, plain HTTP, paths that leave the releases, queries")
  expect(
    ["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com", "GitHub.com"]
      .allSatisfy(Releases.isDownloadHost)
      && !["api.github.com", "githubusercontent.com", "evil.objects.githubusercontent.com", "example.test"]
      .contains(where: Releases.isDownloadHost) && !Releases.isDownloadHost(nil),
    "a download ends only at GitHub's own hosts")
  expect(Releases.isApiHost("api.github.com") && !Releases.isApiHost("github.com"), "checking asks only GitHub's API")
  let https = { (u: String) in URL(string: u) }
  expect(
    Fetch.permits(https("https://github.com/x"), allowed: Releases.isDownloadHost, test: false)
      && !Fetch.permits(https("http://github.com/x"), allowed: Releases.isDownloadHost, test: false)
      && !Fetch.permits(https("https://example.test/x"), allowed: Releases.isDownloadHost, test: false)
      && !Fetch.permits(URL(fileURLWithPath: "/etc/hosts"), allowed: Releases.isDownloadHost, test: false)
      && !Fetch.permits(https("ftp://github.com/x"), allowed: Releases.isDownloadHost, test: false),
    "requests and redirects: HTTPS to GitHub only")
  expect(
    Fetch.permits(URL(fileURLWithPath: "/tmp/x"), allowed: Releases.isDownloadHost, test: true)
      && !Fetch.permits(https("http://example.test/x"), allowed: Releases.isDownloadHost, test: true),
    "a test source may read files, and other HTTPS hosts, but never plain HTTP")

  let hash = String(repeating: "ab", count: 32)
  let sums = Releases.sums("\(hash)  PassBundle-0.1.2-macos.zip\r\n\(hash.uppercased()) *other.zip\nnot a line\n\(hash) x")
  expect(sums == ["PassBundle-0.1.2-macos.zip": hash, "other.zip": hash], "SHA256SUMS as shasum and sha256sum write it")
  expect(
    Releases.digestOf("sha256:\(hash.uppercased())") == hash && Releases.digestOf("sha512:\(hash)") == nil
      && Releases.digestOf(nil) == nil && Releases.digestOf("sha256:abc") == nil,
    "GitHub's digest, as bare hex")
  expect(
    Releases.limit(0) == Releases.maxBytes && Releases.limit(1234) == 1234 && Releases.limit(Releases.maxBytes + 1) == nil,
    "a download is at most what its release says, and never more than PassBundle could be")
  let body = "What's new since 0.1.11:\r\n\r\n- Faster sync\r\n- Settings has updates\r\n\r\nFiles: the zip and SHA256SUMS."
  expect(Releases.changes(body) == "- Faster sync\n- Settings has updates", "what's new is the notes' list")
  expect(Releases.changes("  Just words.\n") == "Just words.", "or the notes as they are")
  let long = (1...500).map { "- change \($0)" }.joined(separator: "\n")
  expect(Releases.changes(long, max: 100).count <= 102 && Releases.changes(long, max: 100).hasSuffix("\n…"), "at most so much")
  expect(Releases.zipName("0.1.2") == "PassBundle-0.1.2-macos.zip", "the release's file for Macs")
}

// MARK: Updates, with signed bundles

let fixtures = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : ".")
let runs = fixtures.appendingPathComponent("runs", isDirectory: true)

func run(_ program: String, _ arguments: [String]) -> Bool { Updater.run(program, arguments) }

func copy(_ from: URL, to: URL) {
  try? fm.createDirectory(at: to.deletingLastPathComponent(), withIntermediateDirectories: true)
  precondition(run("/usr/bin/ditto", [from.path, to.path]), "ditto \(from.path)")
}

func appVersion(_ app: URL) -> String? {
  (NSDictionary(contentsOf: app.appendingPathComponent("Contents/Info.plist")) as? [String: Any])?[
    "CFBundleShortVersionString"] as? String
}

func requirementText(_ app: URL) -> String {
  guard let requirement = InstalledApp.onDisk(app).requirement else { return "(none)" }
  var text: CFString?
  SecRequirementCopyString(requirement, [], &text)
  return (text as String?) ?? "(none)"
}

/// One update's folders: the installed copy, the Trash, the updater's own, and the release.
struct Setup {
  let dir: URL
  let installed: URL
  let trash: URL
  let work: URL
  let release: URL
}

func setup(_ name: String, installing old: String = "old.app", under place: String = "Applications") -> Setup {
  let dir = runs.appendingPathComponent(name, isDirectory: true)
  let s = Setup(
    dir: dir, installed: dir.appendingPathComponent("\(place)/PassBundle.app", isDirectory: true),
    trash: dir.appendingPathComponent("Trash", isDirectory: true),
    work: dir.appendingPathComponent("work", isDirectory: true),
    release: dir.appendingPathComponent("release", isDirectory: true))
  copy(fixtures.appendingPathComponent(old), to: s.installed)
  try! fm.createDirectory(at: s.trash, withIntermediateDirectories: true)
  try! fm.createDirectory(at: s.release, withIntermediateDirectories: true)
  return s
}

/// Publishes `candidate` as the macOS release `version` in `s.release`, among
/// an Android release, a draft, and an older Mac release: the releases list,
/// as GitHub answers it, with file:// links.
func publish(
  _ s: Setup, _ candidate: String, as version: String, tamper: Bool = false, digest: String? = nil,
  size: Int64? = nil, quarantine: Bool = false
) -> URL {
  let staged = s.release.appendingPathComponent("stage/PassBundle.app", isDirectory: true)
  copy(fixtures.appendingPathComponent(candidate), to: staged)
  if quarantine {
    for path in [staged.path, staged.appendingPathComponent("Contents/MacOS/passbundle").path] {
      precondition(run("/usr/bin/xattr", ["-w", "com.apple.quarantine", "0083;00000000;Safari;", path]))
    }
  }
  let name = Releases.zipName(version)
  let zip = s.release.appendingPathComponent(name)
  precondition(run("/usr/bin/ditto", ["-c", "-k", "--keepParent", staged.path, zip.path]))
  let hash = Updater.sha256(zip)!
  try! "\(hash)  \(name)\n".write(to: s.release.appendingPathComponent("SHA256SUMS"), atomically: true, encoding: .utf8)
  let realSize = Int64((try! zip.resourceValues(forKeys: [.fileSizeKey])).fileSize!)
  if tamper {
    let handle = try! FileHandle(forUpdating: zip)
    try! handle.seek(toOffset: UInt64(realSize / 2))
    let byte = try! handle.read(upToCount: 1)!
    try! handle.seek(toOffset: UInt64(realSize / 2))
    try! handle.write(contentsOf: Data([byte[0] ^ 0xff]))
    try! handle.close()
  }
  func asset(_ file: String, _ bytes: Int64, _ digest: String?) -> [String: Any] {
    var a: [String: Any] = [
      "name": file, "size": bytes,
      "browser_download_url": s.release.appendingPathComponent(file).absoluteString,
    ]
    if let digest { a["digest"] = "sha256:\(digest)" }
    return a
  }
  let releases: [[String: Any]] = [
    ["tag_name": "android-v0.1.40", "draft": false, "prerelease": false, "body": "- Android", "assets": []],
    ["tag_name": "macos-v0.1.90", "draft": true, "prerelease": false, "body": "- A draft", "assets": []],
    ["tag_name": "macos-v0.1.80", "draft": false, "prerelease": true, "body": "- A prerelease", "assets": []],
    [
      "tag_name": "macos-v\(version)", "draft": false, "prerelease": false,
      "body": "What's new since 0.1.1:\n\n- One thing\n- Another\n\nFiles: the zip.",
      "assets": [asset(name, size ?? realSize, digest ?? hash), asset("SHA256SUMS", 0, nil)],
    ],
    ["tag_name": "macos-v0.1.1", "draft": false, "prerelease": false, "body": "- The first", "assets": []],
  ]
  let list = s.release.appendingPathComponent("releases.json")
  try! JSONSerialization.data(withJSONObject: releases).write(to: list)
  return list
}

func updater(_ s: Setup, source: URL, debug: Bool = true, prefs: UpdatePrefs = UpdatePrefs(nil)) -> Updater {
  var n = 0
  return Updater(
    installed: { InstalledApp.onDisk(s.installed) }, source: source, debug: debug, workDir: s.work,
    trash: { url in
      n += 1
      let to = s.trash.appendingPathComponent("\(n)-\(url.lastPathComponent)")
      try fm.moveItem(at: url, to: to)
      return to
    }, prefs: prefs)
}

/// What a refused update must leave: the older copy, unchanged, where it was.
func untouched(_ s: Setup, _ what: String, line: Int = #line) {
  let left = (try? fm.contentsOfDirectory(atPath: s.trash.path)) ?? []
  expect(appVersion(s.installed) == "0.1.1" && left.isEmpty, "\(what): the older copy stays", line: line)
  var code: SecStaticCode?
  let whole =
    SecStaticCodeCreateWithPath(s.installed as CFURL, [], &code) == errSecSuccess
    && SecStaticCodeCheckValidity(
      code!, SecCSFlags(rawValue: kSecCSCheckAllArchitectures | kSecCSStrictValidate | kSecCSCheckNestedCode), nil)
      == errSecSuccess
  expect(whole, "\(what): and its signature is whole", line: line)
}

func refused(_ name: String, _ candidate: String, as version: String = "0.1.2", saying words: [String],
  installing old: String = "old.app", under place: String = "Applications", reveal: Bool = false,
  tamper: Bool = false, digest: String? = nil, size: Int64? = nil, seconds: TimeInterval? = nil, line: Int = #line
) {
  let s = setup(name, installing: old, under: place)
  let u = updater(s, source: publish(s, candidate, as: version, tamper: tamper, digest: digest, size: size))
  if let seconds { u.downloadSeconds = seconds }
  let found = u.check(quiet: false)
  expect(found["available"] as? Bool == true, "\(name): the check finds it", line: line)
  let (answer, replaced) = u.install()
  let error = answer["error"] as? String ?? ""
  expect(
    answer["ok"] as? Bool == false && replaced == nil && words.contains { error.contains($0) },
    "\(name): refused (\(error))", line: line)
  expect((answer["reveal"] as? Bool ?? false) == reveal, "\(name): \(reveal ? "can" : "can't") be shown in Finder", line: line)
  if reveal, let copy = u.revealable() {
    expect(appVersion(copy) == version, "\(name): the copy to show is the checked update", line: line)
  }
  untouched(s, name, line: line)
}

func testUpdates() {
  // An older copy updates to a newer one, even with the download marked as one.
  do {
    let s = setup("updates")
    print("      the installed copy's designated requirement: \(requirementText(s.installed))")
    let u = updater(s, source: publish(s, "new.app", as: "0.1.2", quarantine: true))
    var seen: [Int] = []
    u.progress = { seen.append($0) }
    expect(u.status()["version"] as? String == "0.1.1" && u.status()["restarts"] as? Bool == true, "status: the version, and that it restarts")
    let found = u.check(quiet: false)
    expect(
      found["ok"] as? Bool == true && found["available"] as? Bool == true && found["version"] as? String == "0.1.2",
      "the check finds the newest Mac release, past Android's, a draft, and a prerelease")
    expect(found["notes"] as? String == "- One thing\n- Another", "with what's new")
    let (answer, replaced) = u.install()
    expect(answer["ok"] as? Bool == true && answer["restart"] as? Bool == true, "the update installs (\(answer))")
    expect(appVersion(s.installed) == "0.1.2", "the newer copy is where the older one was")
    let old = InstalledApp.onDisk(fixtures.appendingPathComponent("old.app"))
    expect(
      Updater.problem(app: s.installed, meets: old.requirement!, id: "com.passbundle.app", version: "0.1.2", newerThan: "0.1.1") == nil,
      "and meets the older copy's requirement there")
    expect(replaced?.app.path == s.installed.path && appVersion(replaced!.trashed) == "0.1.1", "the older copy is in the Trash")
    expect(!Updater.hasQuarantine(s.installed) && !Updater.hasQuarantine(s.installed.appendingPathComponent("Contents/MacOS/passbundle")),
      "macOS's download mark is gone, once checked")
    expect(!fm.fileExists(atPath: s.work.path), "nothing is left in the updater's folder")
    expect(seen.last == 100 && seen == seen.sorted(), "the download says how far it is (\(seen.count) steps)")
    let again = u.check(quiet: false)
    expect(again["ok"] as? Bool == true && again["available"] as? Bool == false, "then it's the newest")
  }

  refused("tampered", "new.app", saying: ["didn't match the release"], tamper: true)
  refused("digest", "new.app", saying: ["checksums don't agree"], digest: String(repeating: "0", count: 64))
  refused("ad-hoc", "adhoc.app", saying: ["different certificate"])
  refused("other-certificate", "other.app", saying: ["different certificate"])
  refused("wrong-version", "version.app", saying: ["isn't the version its release says"])
  refused("another-app", "wrongid.app", saying: ["different certificate", "isn't PassBundle"])
  refused("broken-signature", "broken.app", saying: ["signature doesn't check out"])
  refused("built-here", "new.app", saying: ["isn't signed the way PassBundle's releases are"], installing: "oldadhoc.app")
  refused("larger", "new.app", saying: ["larger than its release says"], size: 4096)
  refused("slow", "new.app", saying: ["took too long"], seconds: 0)
  refused("translocated", "new.app", saying: ["temporary place"], under: "AppTranslocation/X/d", reveal: true)

  // A folder the person can't change: checked, then offered to show in Finder.
  do {
    let s = setup("read-only")
    let folder = s.installed.deletingLastPathComponent()
    let u = updater(s, source: publish(s, "new.app", as: "0.1.2"))
    _ = u.check(quiet: false)
    try! fm.setAttributes([.posixPermissions: 0o555], ofItemAtPath: folder.path)
    let (answer, replaced) = u.install()
    try! fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: folder.path)
    expect(
      answer["ok"] as? Bool == false && replaced == nil && (answer["error"] as? String ?? "").contains("can't replace itself"),
      "read-only: says it can't replace itself (\(answer["error"] ?? ""))")
    expect(answer["reveal"] as? Bool == true && u.revealable().map(appVersion) == "0.1.2", "read-only: and the checked update can be shown")
    untouched(s, "read-only")
  }

  // Nothing newer.
  do {
    let s = setup("newest", installing: "new.app")
    let u = updater(s, source: publish(s, "new.app", as: "0.1.2"))
    let found = u.check(quiet: false)
    expect(found["ok"] as? Bool == true && found["available"] as? Bool == false, "the same version isn't offered")
    expect(u.install().answer["error"] as? String == "Check for updates first.", "and nothing installs")
  }

  // The check on start: at most once a day, never in a debug build, and not when turned off.
  do {
    let s = setup("daily")
    let source = publish(s, "new.app", as: "0.1.2")
    expect(updater(s, source: source).check(quiet: true)["skipped"] as? Bool == true, "a debug build doesn't check by itself")
    let prefs = UpdatePrefs(nil)
    let u = updater(s, source: source, debug: false, prefs: prefs)
    expect(u.check(quiet: true)["available"] as? Bool == true, "a release build checks as it opens")
    expect(u.check(quiet: true)["skipped"] as? Bool == true, "once a day")
    expect(u.check(quiet: false)["available"] as? Bool == true, "but the button always checks")
    prefs.lastCheck = 0
    u.setAuto(false)
    expect(u.check(quiet: true)["skipped"] as? Bool == true && u.status()["auto"] as? Bool == false, "turned off, it doesn't")
  }

  // The built app, signed twice as the releases are.
  let realOld = fixtures.appendingPathComponent("real-old.app")
  if fm.fileExists(atPath: realOld.path) {
    let s = setup("real-app", installing: "real-old.app")
    print("      the built app's designated requirement: \(requirementText(s.installed))")
    let u = updater(s, source: publish(s, "real-new.app", as: "0.1.2"))
    _ = u.check(quiet: false)
    let (answer, replaced) = u.install()
    expect(answer["ok"] as? Bool == true && replaced != nil, "the built app updates (\(answer))")
    expect(appVersion(s.installed) == "0.1.2", "the built app's newer copy is in place")
    let core = s.installed.appendingPathComponent("Contents/Frameworks/libpassbundle_core.dylib")
    expect(fm.fileExists(atPath: core.path), "with its core")
    let s2 = setup("real-app-adhoc", installing: "real-old.app")
    let u2 = updater(s2, source: publish(s2, "real-adhoc.app", as: "0.1.2"))
    _ = u2.check(quiet: false)
    expect((u2.install().answer["error"] as? String ?? "").contains("different certificate"), "the built app signed ad hoc is refused")
    untouched(s2, "the built app signed ad hoc")
  } else {
    print("      (no built app: tools/mac-update-test.sh <PassBundle.app> updates one too)")
  }
}

// MARK: Restarting

func testRestart() {
  let dir = runs.appendingPathComponent("restart", isDirectory: true)
  try! fm.createDirectory(at: dir, withIntermediateDirectories: true)
  // A helper running from the old copy, as the desktop agent does.
  let helpers = dir.appendingPathComponent("PassBundle.app/Contents/Helpers", isDirectory: true)
  let program = helpers.appendingPathComponent("PassBundle Agent.app/Contents/MacOS/passbundle-agent")
  try! fm.createDirectory(at: program.deletingLastPathComponent(), withIntermediateDirectories: true)
  try! fm.copyItem(at: fixtures.appendingPathComponent("sleeper"), to: program)
  let agent = Process()
  agent.executableURL = program
  agent.arguments = ["30"]
  try! agent.run()
  Thread.sleep(forTimeInterval: 0.3)
  let found = Updater.processes(under: helpers)
  expect(found == [agent.processIdentifier], "the old copy's helpers are found by where they run from")
  expect(Updater.processes(under: dir.appendingPathComponent("elsewhere")).isEmpty, "and nothing else")

  // The restart waits for the app and its helpers to end.
  let app = Process()
  app.executableURL = fixtures.appendingPathComponent("sleeper")
  app.arguments = ["1"]
  try! app.run()
  let opened = dir.appendingPathComponent("opened")
  try! Updater.relaunch(opened, after: [app.processIdentifier, agent.processIdentifier], opener: "/usr/bin/touch")
  Thread.sleep(forTimeInterval: 0.5)
  expect(!fm.fileExists(atPath: opened.path), "the new copy doesn't open while the old one runs")
  kill(agent.processIdentifier, SIGTERM)
  agent.waitUntilExit()
  app.waitUntilExit()
  for _ in 0..<30 where !fm.fileExists(atPath: opened.path) { Thread.sleep(forTimeInterval: 0.1) }
  expect(fm.fileExists(atPath: opened.path), "and opens once they've ended")
}

testReleases()
testUpdates()
testRestart()
print(failures == 0 ? "All passed." : "\(failures) failed.")
exit(failures == 0 ? 0 : 1)
