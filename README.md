# PassBundle apps

PassBundle's own desktop and mobile apps: one Rust core and one Flutter app, so every platform looks and behaves the same. Bitwarden's format and the PassBundle server stay for storage and encryption.

- **`core/`**: the core, in Rust. Keys, the vault format, sync, search, one-time codes: everything but drawing. The design is `docs/core.md` in the meta repo (PimpinPumpkin/Passbundle), with test vectors in `docs/vectors/core-v1.json` (copied into `core/tests/vectors`).
- **`app/`**: the Flutter app, for Windows, macOS, Linux, iOS, and Android. Its own look (brand colors and the bundled Inter font), light and dark.
- **`agent/`**: the desktop agent: the tray (menu bar on macOS), Windows Hello for the app and the browser extension, and the extension's native-messaging host. Moved here from passbundle-clients on October 21, 2026.
- **`ipc/`**: the agent's socket, shared by the agent and the core.
- **`brand/`**: the brand the agent and the core bake in (`brand/README.md`).
- **`tools/linux/`**: a Linux build image (Flutter and Rust at pinned versions).

CI's builds (each run's artifacts) are the whole desktop app: on Windows the app, the core, and the agent in one folder (what the installer in the meta repo takes as its `app` and `agent` folders), and on macOS one app bundle with the agent inside, both ad hoc signed until code signing.

## Status

Early, and not released.
- **The core** opens a vault (PBKDF2 and Argon2id accounts, organization keys, item keys) and refuses weak KDF settings and tampered data, checked against the vectors.
- **On a real server**, the core signs in (with two-step login), syncs, saves, edits, and trashes items. It keeps the vault on the device to unlock offline, with the refresh token in the OS's secret store.
- **The app** signs in, unlocks, browses, searches, copies through the core (cleared after 30 seconds), and adds, edits, and trashes items. Its generator makes passwords or passphrases (EFF's long word list, `core/src/words/`) and remembers the last choices on the device.
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

`flutter test --tags screenshots --update-goldens` redraws `app/test/screenshots/`.

## Android test releases

Every commit on `main` that passes CI becomes a test release for Android (`.github/workflows/release.yml`): built again from that commit, signed with PassBundle's test key, and published on this repository's [releases](https://github.com/PimpinPumpkin/passbundle-apps/releases) as `android-v0.1.<n>`. Each has `passbundle-0.1.<n>-arm64-v8a.apk` for phones, `passbundle-0.1.<n>-x86_64.apk` for the emulator and Chromebooks, and `SHA256SUMS`, with the commit subjects since the last release as its notes. The first two numbers are `app/pubspec.yaml`'s version; `<n>` is the workflow's run number, so each release's version code is higher than the last.

- **The key:** the test key's certificate SHA-256 is `C9:95:FA:F2:33:19:9D:EC:1B:74:53:A5:2C:F4:05:D1:65:73:23:D9:B4:BF:98:EF:CB:52:A7:18:A7:1D:9E:3A`. A copy of PassBundle signed with another key (a debug build, or one from before these releases) can't update to a release: uninstall it once, then install the release. Release builds take the key from `PASSBUNDLE_KEYSTORE_PATH`, `PASSBUNDLE_KEYSTORE_PASSWORD`, and `PASSBUNDLE_KEY_ALIAS` (the workflow sets them from the repository's secrets), or from a properties file outside the repository that `PASSBUNDLE_KEY_PROPERTIES` names (`storeFile`, `storePassword`, `keyAlias`, `keyPassword`). With neither, they're signed with the debug key, so a fork's runs build with it and publish nothing.
- **Updating itself:** Settings has Updates: the version, "Check for updates", and "Update to 0.1.<n>" with what's new when there's a newer release. PassBundle downloads the file for the phone's chip type over HTTPS, checks it against the release's `SHA256SUMS`, checks it's PassBundle signed with the installed app's own key, and hands it to Android's installer, which asks before it installs. At most once a day, as the app opens, it checks by itself and only shows a notice; Settings turns that off. It contacts `api.github.com`, and `github.com` (which hands the file to GitHub's download host), only to check or download, and nothing else. Installed by an app store (Google Play, F-Droid, and others), it leaves updates to the store. Debug builds don't check by themselves, and read a test release from `update-source` in the app's folder when there is one (`app/android/app/src/main/kotlin/com/passbundle/app/update/`).
- **Google Play:** a build for Play would leave the updater out, since Play doesn't allow its permission to install apps for an app like this.

## Rules

- The core never reads or links Bitwarden's SDK or anything under the Bitwarden License (`tools/check-no-bitwarden-crates.sh`).
- Key handling changes only with a design note and test vectors in the meta repo first.
- No key crosses into Dart, and a secret only while the person reveals it.

Licensed under the GPL, version 3 (`LICENSE`). Inter is under the SIL Open Font License (`app/assets/fonts/Inter-LICENSE.txt`).
