//! Two-step login's second step in the apps (docs/core.md, "Signing in";
//! DESIGN.md in the meta repo, "Two-step login in the apps"): what the
//! server offers, and each method but the security key (security_key):
//!
//! - **Typed:** an authenticator app's code (0), an emailed code (1), a
//!   YubiKey's one-time password (3, which the key types, or an Android
//!   phone reads over NFC), and a recovery code (8, which turns two-step
//!   login off, as upstream's does; the server takes it whatever it offers).
//! - **Duo (2):** the server's Universal Prompt address opens in the
//!   browser; Duo sends the browser to the server's Duo page, which hands
//!   the answer back to the app as `passbundle://duo-callback?code=&state=`.
//!   The address is checked to be Duo's, its request to come back to this
//!   server, and the answer to carry this sign-in's state, so a link from
//!   anywhere else finishes nothing. Phones and the Mac get the link from
//!   the OS; on Windows and Linux the OS opens a second copy of the app
//!   with it, which leaves it for the copy signing in (`hand_over`).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::security_key::webauthn::origin_of;

pub const AUTHENTICATOR: u8 = 0;
pub const EMAIL: u8 = 1;
pub const DUO: u8 = 2;
pub const YUBIKEY: u8 = 3;
pub const SECURITY_KEY: u8 = 7;
pub const RECOVERY_CODE: u8 = 8;

/// The methods the apps do, as the server numbers them.
const KNOWN: [u8; 5] = [AUTHENTICATOR, EMAIL, DUO, YUBIKEY, SECURITY_KEY];

/// What the server offers for the second step: its `TwoFactorProviders`,
/// and what `TwoFactorProviders2` says about them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Offer {
    /// The methods offered that the apps do, in the server's order.
    pub providers: Vec<u8>,
    /// Where email codes go, as the server shows it ("an***@example.test").
    pub email: Option<String>,
    /// The account's YubiKeys work over NFC (as it said when adding them).
    pub yubikey_nfc: bool,
    /// Duo's address, when the server gave one the core takes.
    pub duo: Option<Duo>,
    /// The security key request's options, as the server sent them
    /// (security_key::SignInRequest checks them when they're used).
    pub security_key: Option<Value>,
}

impl Offer {
    /// The server's answer asking for the second step, from the server at
    /// `server_url`; None if it isn't one.
    pub fn read(body: &Value, server_url: &str) -> Option<Offer> {
        let offered = body["TwoFactorProviders"].as_array()?;
        let mut providers: Vec<u8> = offered
            .iter()
            .filter_map(|p| {
                p.as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| p.as_u64().and_then(|n| u8::try_from(n).ok()))
            })
            .filter(|p| KNOWN.contains(p))
            .collect();
        providers.dedup();
        let details = &body["TwoFactorProviders2"];
        let detail = |p: u8| &details[p.to_string()];
        let origin = origin_of(server_url).map(|(o, _)| o);
        let duo = detail(DUO)["AuthUrl"]
            .as_str()
            .and_then(|url| Duo::read(url, origin.as_deref()?));
        // Duo whose address the core won't open isn't offered.
        if duo.is_none() {
            providers.retain(|&p| p != DUO);
        }
        let security_key = Some(detail(SECURITY_KEY).clone()).filter(Value::is_object);
        if security_key.is_none() {
            providers.retain(|&p| p != SECURITY_KEY);
        }
        Some(Offer {
            providers,
            email: detail(EMAIL)["Email"].as_str().map(str::to_owned),
            yubikey_nfc: detail(YUBIKEY)["Nfc"].as_bool().unwrap_or(false),
            duo,
            security_key,
        })
    }
}

/// A YubiKey's one-time password as the key types it: 44 modhex characters
/// (its 12-character ID, then 32), which the server checks with Yubico. A
/// keyboard layout that moves letters (Dvorak, say) types something else,
/// which is caught here. Over NFC the key gives its one-time password at
/// the end of an address (`https://my.yubico.com/yk/#...`), taken as well.
pub fn yubikey_otp(typed: &str) -> Result<String> {
    let typed = typed.trim();
    let otp = typed
        .rsplit('#')
        .next()
        .unwrap_or(typed)
        .trim()
        .to_ascii_lowercase();
    if otp.len() == 44 && otp.bytes().all(|b| b"cbdefghijklnrtuv".contains(&b)) {
        Ok(otp)
    } else {
        Err(Error::TwoStep(
            "That isn't a YubiKey's code. Click in the field, then touch your YubiKey (on a phone, you can also hold it to the back).",
        ))
    }
}

/// A recovery code as typed: the server ignores spaces and case.
pub fn recovery_code(typed: &str) -> Result<String> {
    let code: String = typed.chars().filter(|c| !c.is_whitespace()).collect();
    if code.is_empty() || code.len() > 64 {
        return Err(Error::TwoStep(
            "Enter your recovery code: 32 letters and numbers.",
        ));
    }
    Ok(code)
}

/// Duo's Universal Prompt for a sign-in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duo {
    /// Its address, for the browser.
    pub url: String,
    /// The sign-in's state, which Duo's answer must carry back.
    state: String,
}

/// `%XX` and `+` in a query's value, as browsers and the server encode them.
fn query_value(query: &str, name: &str) -> Option<String> {
    let raw = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == name)?
        .1;
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Whether a host is Duo's own API host: `api-` and an ID, under
/// duosecurity.com or (for Duo Federal) duofederal.com, as the web vault's
/// Duo page also requires.
fn duo_host(host: &str) -> bool {
    let Some((id, domain)) = host
        .strip_prefix("api-")
        .and_then(|rest| rest.split_once('.'))
    else {
        return false;
    };
    !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_alphanumeric())
        && matches!(domain, "duosecurity.com" | "duofederal.com")
}

impl Duo {
    /// The server's `AuthUrl`, if it's Duo's authorize page over HTTPS with
    /// a request that comes back to this server (`server_origin`), which
    /// gives the state its answer carries.
    pub fn read(url: &str, server_origin: &str) -> Option<Duo> {
        let rest = url.strip_prefix("https://")?;
        let (authority, path_and_query) = rest.split_at(rest.find('/')?);
        let host = authority
            .strip_suffix(":443")
            .unwrap_or(authority)
            .to_ascii_lowercase();
        if !duo_host(&host) {
            return None;
        }
        let (path, query) = path_and_query.split_once('?')?;
        if path != "/oauth/v1/authorize" || query.contains('#') {
            return None;
        }
        // The request is a JWT the server signed for Duo; its claims are
        // read, not checked (Duo checks them), for where it comes back to
        // and its state.
        let request = query_value(query, "request")?;
        let claims = request.split('.').nth(1)?;
        let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).ok()?).ok()?;
        let back = claims["redirect_uri"].as_str()?;
        if origin_of(back)?.0 != server_origin {
            return None;
        }
        let state = claims["state"].as_str()?.to_owned();
        if state.len() < 16
            || state.len() > 1024
            || !state.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return None;
        }
        Some(Duo {
            url: url.to_owned(),
            state,
        })
    }

    /// Duo's answer as the server's page hands it back
    /// (`passbundle://duo-callback?code=...&state=...`), checked against
    /// this sign-in: the token for provider 2, "code|state".
    pub fn token(&self, link: &str) -> Result<String> {
        let other =
            || Error::TwoStep("That Duo answer was for another sign-in. Continue with Duo again.");
        let rest = link.trim().strip_prefix(LINK).ok_or_else(other)?;
        let query = rest.strip_prefix('?').ok_or_else(other)?;
        let code = query_value(query, "code").ok_or_else(other)?;
        let state = query_value(query, "state").ok_or_else(other)?;
        if state != self.state {
            return Err(other());
        }
        if code.is_empty()
            || code.len() > 1024
            || code.contains('|')
            || code.chars().any(char::is_control)
        {
            return Err(other());
        }
        Ok(format!("{code}|{state}"))
    }
}

/// The link Duo's way back opens (the web vault's `duo-redirect-connector.html`).
pub const LINK: &str = "passbundle://duo-callback";

/// Where a second copy of the app leaves Duo's link for the one signing in.
fn handed_file(dir: &Path) -> PathBuf {
    dir.join("duo-callback.txt")
}

/// How long a link left there is good for: Duo's sign-in context lasts five minutes.
const HANDED_FOR: Duration = Duration::from_secs(5 * 60);

/// A second copy of the app, opened by the OS with Duo's link (Windows and
/// Linux): leaves it in the app's folder, readable only by this person, for
/// the copy that's signing in, and ends. Anything else is ignored.
pub fn hand_over(dir: &Path, link: &str) -> bool {
    let link = link.trim();
    if !link.starts_with(LINK) || link.len() > 4096 {
        return false;
    }
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let path = handed_file(dir);
    let temporary = dir.join("duo-callback.txt.new");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options
        .open(&temporary)
        .and_then(|mut file| std::io::Write::write_all(&mut file, link.as_bytes()));
    written.is_ok() && std::fs::rename(&temporary, &path).is_ok()
}

/// The link a second copy left (`hand_over`), once: it's deleted as it's
/// read, and one older than Duo's five minutes is dropped.
pub fn handed_over(dir: &Path) -> Option<String> {
    let path = handed_file(dir);
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|made| SystemTime::now().duration_since(made).ok())
        .is_some_and(|age| age < HANDED_FOR);
    let link = std::fs::read_to_string(&path).ok();
    let _ = std::fs::remove_file(&path);
    link.filter(|l| fresh && l.starts_with(LINK))
}

/// Makes this app the one the OS opens `passbundle://` links with, for
/// Duo's way back: on Windows for this person only (their own
/// `HKEY_CURRENT_USER\Software\Classes\passbundle`), on Linux with a desktop
/// entry of its own and `xdg-mime`. Phones and the Mac don't need it.
pub fn take_links() -> Result<()> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        let program = std::env::current_exe().map_err(|_| {
            Error::TwoStep("PassBundle couldn't find itself to come back to from Duo.")
        })?;
        // Only the app itself: never a test runner (flutter test) or a tool
        // that loaded the core.
        let ours = program
            .file_stem()
            .is_some_and(|name| name.eq_ignore_ascii_case("passbundle"));
        if !ours {
            return Ok(());
        }
        register(&program)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    Ok(())
}

#[cfg(windows)]
fn register(program: &Path) -> Result<()> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let failed = || Error::TwoStep("PassBundle couldn't set itself up to come back from Duo.");
    let set = |path: &str, values: &[(Option<&str>, &str)]| -> Result<()> {
        let mut key: HKEY = std::ptr::null_mut();
        let path = wide(path);
        // SAFETY: NUL-terminated wide strings and out-pointers, valid for each call.
        unsafe {
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            ) != 0
            {
                return Err(failed());
            }
            let mut ok = true;
            for (name, value) in values {
                let name = name.map(wide);
                let value = wide(value);
                ok &= RegSetValueExW(
                    key,
                    name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                    0,
                    REG_SZ,
                    value.as_ptr() as *const u8,
                    (value.len() * 2) as u32,
                ) == 0;
            }
            RegCloseKey(key);
            if ok {
                Ok(())
            } else {
                Err(failed())
            }
        }
    };
    let program = program.to_string_lossy();
    set(
        r"Software\Classes\passbundle",
        &[(None, "URL:PassBundle"), (Some("URL Protocol"), "")],
    )?;
    set(
        r"Software\Classes\passbundle\shell\open\command",
        &[(None, &format!("\"{program}\" \"%1\""))],
    )
}

#[cfg(target_os = "linux")]
fn register(program: &Path) -> Result<()> {
    let failed = || Error::TwoStep("PassBundle couldn't set itself up to come back from Duo.");
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
        .ok_or_else(failed)?;
    let applications = data.join("applications");
    std::fs::create_dir_all(&applications).map_err(|_| failed())?;
    std::fs::write(applications.join(DESKTOP_ENTRY), desktop_entry(program))
        .map_err(|_| failed())?;
    // The default for passbundle:// links; without xdg-utils, browsers
    // still find the entry by its MimeType.
    let _ = std::process::Command::new("xdg-mime")
        .args(["default", DESKTOP_ENTRY, "x-scheme-handler/passbundle"])
        .status();
    Ok(())
}

#[cfg(target_os = "linux")]
const DESKTOP_ENTRY: &str = "com.passbundle.app-links.desktop";

/// The desktop entry that opens `passbundle://` links with this program,
/// its path quoted as the Desktop Entry Specification says.
#[cfg(any(target_os = "linux", test))]
fn desktop_entry(program: &Path) -> String {
    let mut quoted = String::from("\"");
    for c in program.to_string_lossy().chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        if c == '%' {
            quoted.push('%');
        }
        quoted.push(c);
    }
    quoted.push('"');
    format!(
        "[Desktop Entry]\nType=Application\nName=PassBundle\nComment=Comes back from Duo to finish signing in\nExec={quoted} %u\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/passbundle;\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A Duo address as PassBundle's server makes it, coming back to `back`.
    fn duo_url(host: &str, back: &str, state: &str) -> String {
        let claims = json!({
            "response_type": "code",
            "scope": "openid",
            "client_id": "DIXXXXXXXXXXXXXXXXXX",
            "redirect_uri": back,
            "state": state,
            "duo_uname": "ana@example.test",
        });
        let jwt = format!(
            "eyJhbGciOiJIUzUxMiJ9.{}.c2lnbmF0dXJl",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        format!("https://{host}/oauth/v1/authorize?response_type=code&client_id=DIXXXXXXXXXXXXXXXXXX&request={jwt}")
    }

    const STATE: &str = "Q1w2E3r4T5y6U7i8O9p0AaSsDdFfGgHh";

    #[test]
    fn reads_what_the_server_offers() {
        let body = json!({
            "error": "invalid_grant",
            "TwoFactorProviders": ["7", "2", "3", "0", "1", "6", "5"],
            "TwoFactorProviders2": {
                "0": null,
                "1": { "Email": "an***@example.test" },
                "2": { "AuthUrl": duo_url("api-1a2b3c4d.duosecurity.com", "https://vault.example.test/duo-redirect-connector.html?client=desktop", STATE) },
                "3": { "Nfc": true },
                "7": { "challenge": "AAAAAAAAAAAAAAAAAAAAAA", "rpId": "vault.example.test" },
            },
        });
        let offer = Offer::read(&body, "https://vault.example.test").unwrap();
        // Organization Duo (6) and "remember" (5) aren't for people to pick.
        assert_eq!(offer.providers, vec![7, 2, 3, 0, 1]);
        assert_eq!(offer.email.as_deref(), Some("an***@example.test"));
        assert!(offer.yubikey_nfc);
        assert!(offer.duo.is_some());
        assert_eq!(
            offer.security_key.as_ref().unwrap()["rpId"],
            "vault.example.test"
        );
        // Numbers as numbers, and nothing about them.
        let bare = Offer::read(
            &json!({ "TwoFactorProviders": [0, 1] }),
            "https://vault.example.test",
        )
        .unwrap();
        assert_eq!(bare.providers, vec![0, 1]);
        assert!(bare.email.is_none() && !bare.yubikey_nfc);
        assert!(Offer::read(
            &json!({ "error": "invalid_grant" }),
            "https://vault.example.test"
        )
        .is_none());
    }

    #[test]
    fn duo_and_a_security_key_without_what_they_need_arent_offered() {
        let body = json!({
            "TwoFactorProviders": ["2", "7", "0"],
            "TwoFactorProviders2": {
                "2": { "AuthUrl": "https://evil.example.org/oauth/v1/authorize?request=x" },
                "7": null,
            },
        });
        let offer = Offer::read(&body, "https://vault.example.test").unwrap();
        assert_eq!(offer.providers, vec![0]);
    }

    #[test]
    fn duos_address_must_be_duos_and_come_back_here() {
        let back = "https://vault.example.test/duo-redirect-connector.html?client=mobile";
        let origin = "https://vault.example.test";
        for host in [
            "api-1a2b3c4d.duosecurity.com",
            "API-XYZ.duofederal.com",
            "api-1a2b3c4d.duosecurity.com:443",
        ] {
            assert!(
                Duo::read(&duo_url(host, back, STATE), origin).is_some(),
                "{host}"
            );
        }
        for host in [
            "duosecurity.com",
            "api-.duosecurity.com",
            "api-1a2b.duosecurity.com.evil.example",
            "api-1a2b.evil.example",
            "api-1a2b.duosecurity.com:8443",
            "ana@api-1a2b.duosecurity.com",
        ] {
            assert!(
                Duo::read(&duo_url(host, back, STATE), origin).is_none(),
                "{host}"
            );
        }
        // Coming back to another server, an odd state, or over plain HTTP.
        assert!(Duo::read(
            &duo_url(
                "api-1a2b.duosecurity.com",
                "https://evil.example.org/x",
                STATE
            ),
            origin
        )
        .is_none());
        assert!(Duo::read(&duo_url("api-1a2b.duosecurity.com", back, "short"), origin).is_none());
        assert!(Duo::read(
            &duo_url("api-1a2b.duosecurity.com", back, STATE).replacen("https", "http", 1),
            origin
        )
        .is_none());
        let other_path = duo_url("api-1a2b.duosecurity.com", back, STATE)
            .replace("/oauth/v1/authorize", "/frame/web/v1/auth");
        assert!(Duo::read(&other_path, origin).is_none());
    }

    #[test]
    fn duos_answer_must_carry_this_sign_ins_state() {
        let duo = Duo::read(
            &duo_url(
                "api-1a2b.duosecurity.com",
                "https://vault.example.test/duo-redirect-connector.html?client=desktop",
                STATE,
            ),
            "https://vault.example.test",
        )
        .unwrap();
        assert_eq!(
            duo.token(&format!(
                "passbundle://duo-callback?code=c0de%2Bx&state={STATE}"
            ))
            .unwrap(),
            format!("c0de+x|{STATE}")
        );
        for link in [
            format!("passbundle://duo-callback?code=c0de&state=Other{STATE}"),
            format!("passbundle://duo-callback?state={STATE}"),
            format!("passbundle://duo-callback?code=&state={STATE}"),
            format!("passbundle://duo-callback?code=a%7Cb&state={STATE}"),
            format!("https://vault.example.test/?code=c0de&state={STATE}"),
            format!("passbundle://webauthn-callback?code=c0de&state={STATE}"),
        ] {
            assert!(duo.token(&link).is_err(), "{link}");
        }
    }

    #[test]
    fn a_yubikeys_code_is_44_modhex_characters() {
        let otp = "cccccbhuinjdtrbftdrldjlrgkdeubffnkhjfjifvlbr";
        assert_eq!(yubikey_otp(otp).unwrap(), otp);
        assert_eq!(
            yubikey_otp(&format!("  {}\n", otp.to_uppercase())).unwrap(),
            otp
        );
        // As a phone reads it over NFC.
        assert_eq!(
            yubikey_otp(&format!("https://my.yubico.com/yk/#{otp}")).unwrap(),
            otp
        );
        // Typed with Dvorak, cut short, or a six-digit code.
        assert!(yubikey_otp("jjjjjxdcgbhekpxuekpnhenpcteh.xuuqbtdhuhgu.np").is_err());
        assert!(yubikey_otp(&otp[..40]).is_err());
        assert!(yubikey_otp("123456").is_err());
    }

    #[test]
    fn a_recovery_code_loses_its_spaces() {
        assert_eq!(recovery_code(" abcd efgh\tijkl ").unwrap(), "abcdefghijkl");
        assert!(recovery_code("   ").is_err());
        assert!(recovery_code(&"a".repeat(65)).is_err());
    }

    #[test]
    fn a_second_copy_hands_duos_link_to_the_one_signing_in_once() {
        let dir = std::env::temp_dir().join(format!("pb-two-step-{}", uuid::Uuid::new_v4()));
        let link = format!("passbundle://duo-callback?code=c0de&state={STATE}");
        assert!(handed_over(&dir).is_none());
        assert!(!hand_over(&dir, "https://evil.example.org"));
        assert!(hand_over(&dir, &link));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(handed_file(&dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "only this person reads it");
        }
        assert_eq!(handed_over(&dir).as_deref(), Some(link.as_str()));
        assert!(handed_over(&dir).is_none(), "once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_links_desktop_entry_quotes_the_program() {
        let entry = desktop_entry(Path::new("/home/ana/Pass \"Bundle\"/$100%/passbundle"));
        assert!(
            entry.contains("Exec=\"/home/ana/Pass \\\"Bundle\\\"/\\$100%%/passbundle\" %u\n"),
            "{entry}"
        );
        assert!(entry.contains("MimeType=x-scheme-handler/passbundle;\n"));
        assert!(entry.contains("NoDisplay=true\n"));
    }
}
