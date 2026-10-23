# PassBundle apps

PassBundle's own desktop and mobile apps: one Rust core and one Flutter app, so every platform looks and behaves the same. Bitwarden's format and the PassBundle server stay for storage and encryption.

- **`core/`**: the core, in Rust. Keys, the vault format, sync, search, one-time codes: everything but drawing. The design is `docs/core.md` in the meta repo (PimpinPumpkin/Passbundle), with test vectors in `docs/vectors/core-v1.json` (copied into `core/tests/vectors`).
- **`app/`**: the Flutter app, for Windows, macOS, Linux, iOS, and Android. Its own look (brand colors and the bundled Google Sans Flex font), light and dark.
- **`agent/`**: the desktop agent: the tray (menu bar on macOS), Windows Hello for the app and the browser extension, and the extension's native-messaging host. Moved here from passbundle-clients on October 21, 2026.
- **`ipc/`**: the agent's socket, shared by the agent and the core.
- **`brand/`**: the brand the agent and the core bake in (`brand/README.md`).
- **`tools/linux/`**: a Linux build image (Flutter and Rust at pinned versions).

CI's builds (each run's artifacts) are the whole desktop app: on Windows the app, the core, and the agent in one folder (what the installer in the meta repo takes as its `app` and `agent` folders), and on macOS one app bundle with the agent inside, both ad hoc signed. They're test builds: the core is built with its `test-hooks` feature and the app with `--dart-define=PB_TEST_HOOKS=true`, so `tools/measure.sh`, `tools/measure.ps1`, and `tools/mac-update-test.sh --launch` can open them on a vault of their own (`PB_DATA_DIR`, `PB_SECRETS=memory`, `PB_MEASURE`). The releases are built without either and ignore those settings, and a release build never takes `PB_CORE_LIB` (which library is the core) from its environment. The Mac test releases are signed with PassBundle's own test certificate (below).

## Status

Early, and not released.
- **The core** opens a vault (PBKDF2 and Argon2id accounts, organization keys, item keys) and refuses weak KDF settings and tampered data, checked against the vectors.
- **On a real server**, the core signs in (with two-step login), syncs, saves, edits, and trashes items. It keeps the vault on the device to unlock offline, with the refresh token in the OS's secret store.
- **The app** signs in, unlocks, browses, searches, copies through the core (cleared after 30 seconds), and adds, edits, and trashes items. Its generator makes passwords or passphrases (EFF's long word list, `core/src/words/`) and remembers the last choices on the device.
- **Folders and templates as in the web vault:** Personal and its folders, then each organization and its shared folders, nested by name, with counts; the editor's one Folder picker, grouped by owner; what each shared folder lets the person do (no secrets shown where it hides passwords, no edit or trash where it doesn't allow them). New item offers the web vault's kinds and templates (Wi-Fi, servers, databases, licenses, API keys, domains, and the person's and their organizations' own), each with its category's jewel, and an edit keeps an item's template and every field (`core/src/templates.rs`, `core/src/places.rs`).
- **Items as the web vault's panel shows them:** logins' websites' icons from the server's icon service, kept on the device for a week, with the letter where there's none (Settings turns them off; `core/src/icons.rs`); the favorite star; Open website; how strong the password is and when it changed, and Health's weak, reused, and unsecured logins (zxcvbn, on the device; `core/src/health.rs`); and the item's version history, side by side with the item now, restored as a normal edit, or cleared where the person may (`core/src/history.rs`). Phones have a Generator tab, whose Copy clears after 30 seconds like any secret.
- **Windows Hello** (Touch ID once the agent has it) unlocks the app through the desktop agent, sharing the browser extension's enrollment. The agent's tray opens the app, and the app starts the agent.
- **Not yet:** passkeys, attachments, the macOS and Linux installs, phones, and code signing.

## Tests

```bash
cd core && cargo test && cargo clippy --all-targets --features ffi -- -D warnings
tools/check-no-bitwarden-crates.sh
cd core && cargo build --release --features ffi
cd app && PB_CORE_LIB=../core/target/release/libpassbundle_core.dylib flutter test --exclude-tags screenshots
```

Against a real server (`tools/test-server.sh` starts a throwaway one from a passbundle-server build):

```bash
PB_TEST_SERVER=$(tools/test-server.sh <passbundle-server binary> 8591 /tmp/pb-server) cargo test --test server
```

`flutter test --tags screenshots --update-goldens` redraws `app/test/screenshots/`. With `PB_SHOWCASE` set to a folder `cargo run --example showcase -- <folder>` filled, they're drawn on a fuller sample vault (folders in folders, shared folders, items from templates), which isn't committed.

The Mac updater on its own, against bundles signed as the Mac test releases are (with a built app, that app too; `--launch` opens it):

```bash
tools/mac-update-test.sh [--launch] [app/build/macos/Build/Products/Release/passbundle.app]
```

## Android test releases

Every commit on `main` that passes CI becomes a test release for Android (`.github/workflows/release.yml`): built again from that commit, signed with PassBundle's test key, and published on this repository's [releases](https://github.com/PimpinPumpkin/passbundle-apps/releases) as `android-v0.1.<n>`. Each has `passbundle-0.1.<n>-arm64-v8a.apk` for phones, `passbundle-0.1.<n>-x86_64.apk` for the emulator and Chromebooks, and `SHA256SUMS`, with the commit subjects since the last release as its notes. The first two numbers are `app/pubspec.yaml`'s version; `<n>` is the workflow's run number, so each release's version code is higher than the last.

- **The key:** the test key's certificate SHA-256 is `C9:95:FA:F2:33:19:9D:EC:1B:74:53:A5:2C:F4:05:D1:65:73:23:D9:B4:BF:98:EF:CB:52:A7:18:A7:1D:9E:3A`. A copy of PassBundle signed with another key (a debug build, or one from before these releases) can't update to a release: uninstall it once, then install the release.
- **The key never meets the build:** the workflow builds the APKs in one job, with the debug key, no secrets, and no way to write; a second job, the only one with the key, signs them again with the Android SDK's `apksigner` and runs nothing from the repository; a third, the only one that can write, publishes them. Release builds on a computer can take the key straight from `PASSBUNDLE_KEYSTORE_PATH`, `PASSBUNDLE_KEYSTORE_PASSWORD`, and `PASSBUNDLE_KEY_ALIAS`, or from a properties file outside the repository that `PASSBUNDLE_KEY_PROPERTIES` names (`storeFile`, `storePassword`, `keyAlias`, `keyPassword`); with neither, they're signed with the debug key. A fork has no key, so its runs build with the debug key and publish nothing.
- **Updating itself:** Settings has Updates: the version, "Check for updates", and "Update to 0.1.<n>" with what's new when there's a newer release. PassBundle downloads the file for the phone's chip type over HTTPS, only from this repository's releases and at most 150 MB, checks it against the release's `SHA256SUMS`, checks it's PassBundle at the release's version, signed with the installed app's own key, and hands it to Android's installer, which asks before it installs. At most once a day, as the app opens, it checks by itself and only shows a notice; Settings turns that off. It contacts `api.github.com`, and `github.com` (which hands the file to `objects.githubusercontent.com` or `release-assets.githubusercontent.com`), only to check or download, and no other host. Installed by an app store (Google Play, F-Droid, and others), it leaves updates to the store. Debug builds don't check by themselves, and read a test release from `update-source` in the app's folder when there is one (`app/android/app/src/main/kotlin/com/passbundle/app/update/`).
- **Google Play:** a build for Play would leave the updater out, since Play doesn't allow its permission to install apps for an app like this.

## Mac test releases

Every commit on `main` that passes CI also becomes a test release for the Mac (`.github/workflows/macos-release.yml`), published on this repository's [releases](https://github.com/PimpinPumpkin/passbundle-apps/releases) as `macos-v0.1.<n>`: `PassBundle-0.1.<n>-macos.zip`, the app for Macs with Apple silicon and with Intel (universal), and `SHA256SUMS`, with the commit subjects since the last Mac release as its notes. `<n>` counts the Mac's releases on their own, so it isn't Android's number for the same commit. A Mac release is never marked as the latest release: that stays Android's.

- **The certificate:** PassBundle's own test certificate ("PassBundle test builds", self-signed, for code signing only), since there's no Apple Developer account; so the app isn't notarized either. Its SHA-256 is `01:58:11:39:51:F6:3B:39:BA:C0:6B:A4:3D:9D:C8:38:AD:E6:CD:E2:DA:E8:BC:55:60:8F:D4:29:7D:3D:24:AC`. Every program and library in the app is signed with it, with the hardened runtime, so the app's designated requirement (`identifier "com.passbundle.app" and certificate root = H"5833cb215225bb004aa67e200cb6c64bd95fc479"`) is the same in every release, and the keychain keeps trusting PassBundle after each update without asking. The hardened runtime's library validation is off (`app/macos/Runner/Release.entitlements`), since it loads only libraries signed by an Apple developer team, which this certificate isn't. In its place, the app is signed with a library load constraint: it loads exactly its own libraries (FlutterMacOS, App, and the core, by each architecture's code directory hash, and Rosetta's translations of them) and the OS's, so a library swapped into its folder, however it's signed, doesn't load. macOS 14 and later enforce the constraint; on macOS 12 and 13 any validly signed library put in the app's folder would load. The rest of the hardened runtime (which ignores `DYLD_INSERT_LIBRARIES` and keeps other programs from reading PassBundle's memory) stays on.
- **The key never meets the build:** as for Android, the build job has no secrets and can't write. The sign job, the only one with the certificate (the `PASSBUNDLE_MAC_CERT_P12_BASE64` and `PASSBUNDLE_MAC_CERT_PASSWORD` secrets), puts it in a keychain of its own, signs with Apple's `codesign` and entitlements and a library load constraint of its own making, runs nothing from the repository, checks every program in the app against the certificate's pinned SHA-256, the app's designated requirement, and its entitlements (exactly library validation off, and none on anything nested), and deletes the keychain. The build job checks the core was built without its test hooks. The publish job, the only one that can write, has no secrets. A fork has no certificate, so it publishes nothing.
- **Installing it the first time:** download the zip, open it, and move PassBundle to Applications, replacing an older copy. macOS won't open it the first time, since Apple hasn't checked it: on macOS 15 and later, open it once, then choose Open Anyway in System Settings, Privacy & Security; on earlier versions, Control-click PassBundle and choose Open. A copy from before these releases (signed ad hoc) can't update itself to one, and macOS may ask once whether the release may use PassBundle's saved sign-in in the keychain: choose Always Allow.
- **Updating itself:** Settings has Updates, as on Android: the version, "Check for updates", and "Update to 0.1.<n>" with what's new. PassBundle downloads the zip itself over HTTPS (so macOS doesn't take it for a download to ask about), only from this repository's releases and at most 300 MB, checks it against the release's `SHA256SUMS` (and GitHub's own SHA-256 of it), checks it unpacks to at most 8 times its size (1 GB at most), unpacks it, and checks the app inside: its signature whole, meeting the running copy's designated requirement, and PassBundle at the release's version, newer than this copy. Then the running copy goes to the Trash, the new one takes its place, the old copy's desktop agent stops, and PassBundle opens again (locked), starting the new agent. Where PassBundle can't change the folder it's in (only an administrator can, as with the meta repo's installer, or for someone who isn't one; or macOS runs it from a temporary place, since it was opened where it was downloaded), it says so and shows the checked update in Finder, to drag into place. It contacts the same hosts as on Android, only to check or download, at most once a day by itself (Settings turns that off). Debug builds don't check by themselves, and read a test release (a URL, `file://` too) from `update-source` in the app's folder (`PB_DATA_DIR`, else `~/Library/Application Support/PassBundle`) when there is one (`app/macos/Runner/Updater.swift`).
- **Tests:** `tools/mac-update-test.sh` builds the updater into a test program of its own and updates real bundles signed as the releases are: an older copy updates to a newer one, and a tampered download, a copy signed ad hoc or with another certificate, the wrong version, another app, a broken signature, a download larger than its release says or too slow, a zip that unpacks far bigger than it is, and a folder PassBundle can't change are each refused, leaving the older copy as it was. It also checks the library load constraint: a signed app loads its own framework and core, and not another library signed with the same certificate, nor one swapped in for its core, natively and under Rosetta. CI runs it on every commit with throwaway certificates, on the app it built too, and opens that app signed as the releases are. It signs with PassBundle's test certificate when `PB_MAC_SIGN_P12` and `PB_MAC_SIGN_PASSWORD` name it.

## Rules

- The core never reads or links Bitwarden's SDK or anything under the Bitwarden License (`tools/check-no-bitwarden-crates.sh`).
- Key handling changes only with a design note and test vectors in the meta repo first.
- No key crosses into Dart, and a secret only while the person reveals it.

Licensed under the GPL, version 3 (`LICENSE`). Google Sans Flex is under the SIL Open Font License (`app/assets/fonts/GoogleSansFlex-OFL.txt`).
