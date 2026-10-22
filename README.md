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
- **Phones:** on Android, the app with biometric unlock, and autofill that fills, saves, and updates sign-ins in other apps and browsers. On iPhone, the app with Face ID unlock, and AutoFill that fills websites in Safari and other apps, with iOS suggesting logins above the keyboard. Both are tested in CI on emulators and simulators.
- **Not yet:** passkeys, attachments, and code signing.

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

## Rules

- The core never reads or links Bitwarden's SDK or anything under the Bitwarden License (`tools/check-no-bitwarden-crates.sh`).
- Key handling changes only with a design note and test vectors in the meta repo first.
- No key crosses into Dart, and a secret only while the person reveals it.

Licensed under the GPL, version 3 (`LICENSE`). Inter is under the SIL Open Font License (`app/assets/fonts/Inter-LICENSE.txt`).
