# PassBundle apps

PassBundle's own desktop and mobile apps: one Rust core and one Flutter app, so every platform looks and behaves the same. Bitwarden's format and the PassBundle server stay for storage and encryption.

- **`core/`**: the core, in Rust. Keys, the vault format, sync, search, one-time codes: everything but drawing. The design is `docs/core.md` in the meta repo (PimpinPumpkin/Passbundle), with test vectors in `docs/vectors/core-v1.json` (copied into `core/tests/vectors`).
- **`app/`**: the Flutter app, for Windows, macOS, Linux, iOS, and Android. Its own look (brand colors and the bundled Inter font), light and dark.
- **`tools/linux/`**: a Linux build image (Flutter and Rust at pinned versions).

## Status

A prototype. The core opens a vault (PBKDF2 and Argon2id accounts, organization keys, item keys), refuses weak KDF settings and tampered data, and makes one-time codes, all checked against the vectors. The app unlocks the vectors' sample vault and browses it. Not yet: signing in to a server, sync, editing, the clipboard through the core, and the real bridge (flutter_rust_bridge; the prototype uses plain C functions).

## Tests

```bash
cd core && cargo test && cargo clippy --all-targets --features ffi -- -D warnings
tools/check-no-bitwarden-crates.sh
cd core && cargo build --release --features ffi
cd app && PB_CORE_LIB=../core/target/release/libpassbundle_core.dylib flutter test --exclude-tags screenshots
```

`flutter test --tags screenshots --update-goldens` redraws `app/test/screenshots/`.

## Rules

- The core never reads or links Bitwarden's SDK or anything under the Bitwarden License (`tools/check-no-bitwarden-crates.sh`).
- Key handling changes only with a design note and test vectors in the meta repo first.
- No key crosses into Dart, and a secret only while the person reveals it.

Licensed under the GPL, version 3 (`LICENSE`). Inter is under the SIL Open Font License (`app/assets/fonts/Inter-LICENSE.txt`).
