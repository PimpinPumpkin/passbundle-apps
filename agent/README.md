# Desktop agent

PassBundle's desktop agent: a tray app (menu bar on macOS) with no windows, written in Rust, that runs beside the desktop app (the Flutter app in `app/`). The browser extension talks to it where it would talk to upstream's Electron desktop app, which we don't ship. The design is in the meta repo's DESIGN.md, "Desktop agent".

Version 0.1 is the foundation: the tray icon and menu (the version, Open PassBundle when the app is installed with the agent, Open web vault, Quit, and while the app has said so, "2 items expire soon", which opens the app: the app tells the agent two numbers per account over its socket, `keykeeper.reminders` with `soon` and `expired`, and the agent makes its own words; see `src/reminders.rs`), one agent per signed-in user, settings by policy, and an updater that, on Windows, replaces the agent, its host, and the app together, and installs only releases signed with our offline update key. It also unlocks the browser extension with Windows Hello (DESIGN.md, "Biometric unlock"): the extension speaks upstream's native-messaging protocol to `passbundle-agent-host`, which the browser starts, and the host pipes it to the agent over a socket only the signed-in user can open.

## Why it's its own workspace

Upstream's `apps/desktop/desktop_native` crates are GPL-3.0, but some of them link Bitwarden's SDK crates (from `sdk-internal`), which declare a license file instead of a standard open-source license, and `bitwarden-russh`, which declares none. `biometric` uses the SDK crates and `desktop_core` uses `bitwarden-russh`. The agent links neither: it's a separate Cargo workspace, and anything it takes from upstream's GPL code is copied in with a note saying where from.

## Build and test

```bash
cd agent
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

The name, domain, and web vault URL come from `brand/brand.json` at build time, and the tray icons from `brand/agent/` (see `brand/README.md`). This repo's CI runs all of this on Windows and macOS (and the tests on Linux), checks the release build is under 10 MB, and runs it in the tray. Tier 1, in the meta repo, builds it with the stand-in authenticator for the extension's end-to-end tests.

It lived in passbundle-clients (`apps/agent`) until October 21, 2026; its history before then is there.

## Settings by policy

Read only from places a standard user can't write:

| Setting | Windows (`HKLM\SOFTWARE\Policies\PassBundle\Agent`) | macOS (managed preferences, `com.passbundle.agent`) |
| --- | --- | --- |
| Web vault to open | `ServerUrl` (string, `https://` only) | `ServerUrl` (string) |
| Updates of the agent, its host, and the app | `Updates` (DWORD, 0 turns them off) | `Updates` (boolean) |

Without a policy the agent opens the brand's web vault. No build updates itself yet, since none has an update key compiled in ("The update key", below): `passbundle-agent --update` says "This build doesn't update itself." Until then a Windows install gets new versions from the RMM, and a test release with the app, whose own updater replaces its whole folder. MSPs that push PassBundle through their RMM can turn updates off, for all three alike.

## Updates

On Windows the agent, its native-messaging host, and the app are one install (`%ProgramFiles%\PassBundle`, from the meta repo's `installer/windows/Install-PassBundle.ps1`), and they're updated together, all three or none (`src/update.rs`, `src/swap.rs`). The installer's scheduled task runs `passbundle-agent --update` as SYSTEM as the computer starts and daily. It checks `https://passbundle.com/agent/windows-x86_64.json`, a manifest like:

```json
{
  "product": "passbundle-agent",
  "version": "0.1.60",
  "target": "windows-x86_64",
  "url": "https://passbundle.com/agent/PassBundle-0.1.60-windows-x64.zip",
  "size": 15728640,
  "sha256": "<64 hex digits>"
}
```

and its minisign signature, `windows-x86_64.json.minisig`. The zip is a Windows release's (the app's `.github/workflows/windows-release.yml` builds it): one `PassBundle` folder with the agent, its host, the app, the app's core and updater, and `release.json`, which lists every path in it. The task takes it only if:

- the signature verifies with the public key compiled in (`update-key.pub`; minisign's prehashed signatures only);
- the manifest is for this product and platform, and its version is newer than the installed folder's `release.json` (a folder without one, not installed from a release, is left alone);
- the zip is at `https://passbundle.com/agent/<name>.zip`, the manifests' own place on the same site, over HTTPS, with no redirect followed;
- the download has the stated size and SHA-256, hashed as it's written to the work folder and stopped at that size, at most 200 MB;
- it unpacks to one `PassBundle` folder of plain files and folders (no links, `..`, absolute paths, or device names; at most 8 times its size and 100,000 entries), which is that version for `windows-x64`, has `passbundle-agent.exe`, `passbundle-agent-host.exe`, `passbundle.exe`, and `passbundle_core.dll`, and holds exactly what its `release.json` lists.

Nothing is touched unless the installed folder, the one it's in, and everything in it can be changed only by SYSTEM, Administrators, and TrustedInstaller, and no part of it is a link. The work folder (`%ProgramFiles%\.PassBundle-agent-update`) is one the task makes for itself, with its own access list: SYSTEM and Administrators may change it, Users may read and run what's in it. The task runs no other program by name and loads libraries only from System32 once it starts. On Windows the agent and its host link the C runtime in (`.cargo/config.toml`, which every build from this folder takes; `src/lib.rs` refuses to build for Windows without it), so they need no Visual C++ runtime, whose libraries a computer without it would look for on PATH. CI and the release workflow check their imports with `cargo run --example imports -- <program>...`.

Programs running from the folder (the agent for each person signed in, the app, a host a browser started) can't be replaced, so a checked update that can't go in waits in the work folder, and goes in at the next run that finds nothing running from the folder: usually as the computer starts, before anyone signs in. Putting it in place is a list of renames on one disk, written to the work folder first: the old version's files and folders out, the new one's in, the agent's program last out and first in. If a rename fails, or the new agent doesn't answer `--version`, they're undone, last first, and the old version is as it was; a run cut off partway (the computer turned off) is undone by the next one before anything else. The installer's host manifest (`com.passbundle.agent.json`) stays where it is. TLS is the operating system's (SChannel), with its certificate store.

On macOS nothing runs `--update`: the agent is inside the app, where replacing it alone would break the app's signature, so it's updated only with the app, and `--update` says so. The tray agent itself never accepts commands from the network. `-DisableAgentUpdates` on the installer leaves the task out and turns all of it off by policy.

### The update key

There is no `update-key.pub` yet, so builds don't update themselves. To create the key, on an offline machine:

```bash
minisign -G -p update-key.pub -s passbundle-update.key
```

Keep `passbundle-update.key` and its password offline (DESIGN.md, "Updates"). Commit only `update-key.pub`, here. A release is a Windows release's zip, from its GitHub release (`windows-v<version>`, built from `main` with no secrets) after checking it against that release's `SHA256SUMS` and signed manifest. Write its manifest and sign it:

```bash
zip=PassBundle-0.1.60-windows-x64.zip
jq -n -c --arg version 0.1.60 --arg url "https://passbundle.com/agent/$zip" \
  --argjson size "$(wc -c < "$zip")" --arg sha256 "$(shasum -a 256 "$zip" | cut -d' ' -f1)" \
  '{product: "passbundle-agent", version: $version, target: "windows-x86_64", url: $url, size: $size, sha256: $sha256}' > windows-x86_64.json
minisign -S -s passbundle-update.key -m windows-x86_64.json
```

Then publish the manifest, its `.minisig`, and the zip under the site's `agent/` folder.

Tests use throwaway keys. The unit tests and `tests/update.rs` make their own; the meta repo's tier 2 on Windows builds the agent with `--features test-update`, which compiles in `PB_TEST_UPDATE_KEY` and a stand-in for the site (`PB_TEST_UPDATE_SOURCE`) instead, and signs with `examples/test-release.rs`. Release builds refuse that feature.

## Not built yet

The MSP background work (confirming members, keeping template copies current) stays in the web vault and the extension: the agent holds the user key only behind Windows Hello, and doing it in the background would mean keeping the key unlocked.


- The native-messaging host registered for Firefox. The meta repo's `Install-PassBundle.ps1` installs the agent with autostart, the update task, and the host registered for Chrome and Edge; its Mac installer registers it for the same two.
- Signed builds (Authenticode on Windows, a Developer ID on macOS).
- Touch ID on macOS, which needs the agent signed with our Apple developer ID.
- A check of Windows Hello on real hardware: CI machines have none, so the tests use a stand-in (`--features test-authenticator`, which release builds refuse).
