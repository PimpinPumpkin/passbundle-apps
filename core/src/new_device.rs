//! The new-device check (DESIGN.md in the meta repo, "New-device check";
//! docs/core.md, "Signing in"). An account without two-step login that signs
//! in with its master password on a device it has never used is held by the
//! server until the person types the two-digit number this device shows into
//! one of the account's signed-in devices, or, when none can approve, gives
//! a code sent by email. No key moves either way: this device already has
//! the master password, and the check only decides whether the server
//! finishes the sign-in.
//!
//! Both sides are here: this device waiting (`Held`, `Check`), and this
//! device approving another (`Waiting`, `typed_number`). A device that gets
//! in is given a token of its own (upstream's `TwoFactorToken`), which the
//! core keeps in the OS's secret store and sends back at every later sign-in
//! with the master password (secrets.rs, `Secret::DeviceToken`).

use serde_json::Value;
use zeroize::Zeroizing;

use crate::error::{Error, Result};

/// How the check stands, as the new device sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Waiting for the number to be typed on another device, or a code.
    Waiting,
    /// Approved: the sign-in finishes with the check's token.
    Approved,
    /// Denied from another device, or a wrong number was typed there.
    Denied,
    /// Gone: its time is up, or it was used. Signing in again starts anew.
    Ended,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Waiting => "waiting",
            Status::Approved => "approved",
            Status::Denied => "denied",
            Status::Ended => "ended",
        }
    }
}

/// Whether a code by email can be had, as the server says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Email {
    /// No device can approve: "Email me a code" sends one now.
    Now,
    /// Devices can approve: "Can't use your other devices?" asks for a code
    /// after a day's wait.
    Later,
    /// Asked for after the wait, which ends at `email_after`.
    Waiting,
    /// Not for this account (two-step login is required of them), or the
    /// server sends no email and there's nothing to wait for.
    Never,
}

impl Email {
    pub fn name(self) -> &'static str {
        match self {
            Email::Now => "now",
            Email::Later => "later",
            Email::Waiting => "waiting",
            Email::Never => "never",
        }
    }

    fn read(text: Option<&str>) -> Email {
        match text {
            Some("now") => Email::Now,
            Some("later") => Email::Later,
            Some("waiting") => Email::Waiting,
            _ => Email::Never,
        }
    }
}

/// A sign-in on this device that the server holds, as the waiting screen
/// shows it. Never the check's token: that stays in the core (`Held`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub id: String,
    /// The number to type on another device: two digits.
    pub number: String,
    pub status: Status,
    /// Why it was denied: "number" (a wrong number was typed) or "denied".
    pub denied_why: Option<String>,
    /// When it ends, as the server writes dates.
    pub expires_at: Option<String>,
    /// Whether any of the account's devices can approve it.
    pub approvers: bool,
    pub email: Email,
    /// When a code by email may be sent, after "Can't use your other
    /// devices?".
    pub email_after: Option<String>,
    /// Whether a code was sent, and where to ("a***@example.test").
    pub code_sent: bool,
    pub email_to: Option<String>,
}

/// A held sign-in's check and this device's token for it (43 characters):
/// in memory only, while the sign-in waits. The token finishes the sign-in
/// once it's approved, and asks how it stands; signing in again gives a new
/// one, and the old one stops working.
pub struct Held {
    pub check: Check,
    pub(crate) token: Zeroizing<String>,
}

/// An id the server gave: letters, digits, and hyphens only (a UUID).
fn an_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 64
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        .map(str::to_owned)
}

fn text(value: &Value) -> Option<String> {
    value.as_str().filter(|t| !t.is_empty()).map(str::to_owned)
}

impl Check {
    /// The state object the server sends (`keykeeperNewDeviceCheck`), or
    /// None if it isn't one: an id and a two-digit number at the least.
    fn read(value: &Value) -> Option<Check> {
        let id = an_id(&value["id"])?;
        let number = value["number"]
            .as_str()
            .and_then(|n| typed_number(n).ok())?;
        let status = match value["status"].as_str() {
            Some("approved") => Status::Approved,
            Some("denied") => Status::Denied,
            Some("waiting") | None => Status::Waiting,
            Some(_) => Status::Ended,
        };
        Some(Check {
            id,
            number,
            status,
            denied_why: text(&value["deniedWhy"]),
            expires_at: text(&value["expiresAt"]),
            approvers: value["approvers"].as_bool().unwrap_or(false),
            email: Email::read(value["email"].as_str()),
            email_after: text(&value["emailAfter"]),
            code_sent: value["codeSent"].as_bool().unwrap_or(false),
            email_to: text(&value["emailTo"]),
        })
    }

    /// How it stands now, from the server's answer to asking (`status`) or
    /// to "Email me a code" (`email`). An answer about another check, or
    /// with another number, isn't this one's.
    pub(crate) fn update(&mut self, value: &Value) -> Result<()> {
        let now = Check::read(value)
            .filter(|now| now.id == self.id && now.number == self.number)
            .ok_or_else(|| Error::Malformed("a new-device check it doesn't know".into()))?;
        *self = now;
        Ok(())
    }
}

/// The server holding a sign-in for the new-device check: a 400 with
/// upstream's message for it and `KeykeeperNewDeviceCheck`, which carries
/// this device's token. None for any other answer.
pub fn held(body: &Value) -> Option<Held> {
    let state = &body["KeykeeperNewDeviceCheck"];
    let check = Check::read(state)?;
    let token = state["token"]
        .as_str()
        .filter(|t| (20..=128).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_graphic()))?;
    Some(Held {
        check,
        token: Zeroizing::new(token.to_owned()),
    })
}

/// The number someone typed for another device's check: exactly two
/// digits (spaces around them don't count). Anything else is refused here,
/// before anything is sent: a wrong number denies the check, but no number
/// at all mustn't.
pub fn typed_number(typed: &str) -> Result<String> {
    let typed = typed.trim();
    if typed.len() == 2 && typed.bytes().all(|b| b.is_ascii_digit()) {
        Ok(typed.to_owned())
    } else {
        Err(Error::TwoStep(
            "Type the two-digit number shown on the new device.",
        ))
    }
}

/// The code from the email: 8 digits, with any spaces or hyphens taken out.
pub fn email_code(typed: &str) -> Result<String> {
    let code: String = typed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    if code.len() == 8 && code.bytes().all(|b| b.is_ascii_digit()) {
        Ok(code)
    } else {
        Err(Error::TwoStep("Enter the 8-digit code from the email."))
    }
}

/// The server's words for a code by email that didn't work, and other
/// refusals of a sign-in held for this check, as the apps say them.
pub(crate) fn refusal(message: &str) -> Option<Error> {
    let words = if message.starts_with("That code didn't work") {
        "That code didn't work. Check the email and try again."
    } else if message.starts_with("That code can't be used anymore") {
        "That code can't be used anymore. Start over to get a new one."
    } else if message.starts_with("Ask for a code by email first") {
        "Ask for a code by email first."
    } else if message.contains("Sign in on a device you've used before") {
        "Your organization requires two-step login for your role, so a new device needs approval from a device where you're already signed in, and none can approve it. Sign in on a device you've used before, and set up two-step login there."
    } else {
        return None;
    };
    Some(Error::TwoStep(words))
}

/// Another device's sign-in waiting for this account's approval, as the
/// approving prompt shows it. Never its number: only the new device has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub id: String,
    /// The device in the server's words ("Web vault in Firefox").
    pub device: String,
    /// Its type, as upstream numbers them.
    pub device_type: i64,
    /// Its address, as the server saw it.
    pub ip: String,
    /// Roughly where it is ("Berlin, Germany"), when the server's location
    /// database knows; None for an unknown or private address.
    pub location: Option<String>,
    /// When it was made: seconds since 1970, by the server's clock.
    pub created: u64,
    /// How old it was, and how long it had left, when the server listed it
    /// (seconds, by the server's clock).
    pub age: u64,
    pub left: u64,
    /// When a code by email may be sent for it, if someone asked for one
    /// after the wait ("Can't use your other devices?").
    pub email_after: Option<String>,
}

/// Who the server's location database is from, to credit next to a
/// location shown: its name and its address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocationSource {
    pub name: String,
    pub url: Option<String>,
}

/// The account's checks waiting for an answer
/// (`GET /api/keykeeper/new-device-checks`), newest first, and who the
/// locations are from. `fallback_now` is this device's clock, for a server
/// that doesn't say its own.
pub fn parse_waiting(body: &Value, fallback_now: u64) -> (Vec<Waiting>, Option<LocationSource>) {
    let now = body["serverDate"]
        .as_str()
        .and_then(crate::device_approval::parse_date)
        .unwrap_or(fallback_now);
    let mut waiting: Vec<Waiting> = body["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let id = an_id(&c["id"])?;
            let created = c["creationDate"]
                .as_str()
                .and_then(crate::device_approval::parse_date)?;
            let ends = c["expirationDate"]
                .as_str()
                .and_then(crate::device_approval::parse_date)?;
            let left = ends.checked_sub(now).filter(|left| *left > 0)?;
            Some(Waiting {
                id,
                device: text(&c["device"]).unwrap_or_else(|| "A device".to_owned()),
                device_type: c["deviceType"].as_i64().unwrap_or(-1),
                ip: text(&c["ipAddress"]).unwrap_or_default(),
                location: text(&c["location"]),
                created,
                age: now.saturating_sub(created),
                left,
                email_after: text(&c["emailAfter"]),
            })
        })
        .collect();
    waiting.sort_by_key(|w| std::cmp::Reverse(w.created));
    let source = text(&body["locationSource"]["name"]).map(|name| LocationSource {
        name,
        url: text(&body["locationSource"]["url"]).filter(|u| u.starts_with("https://")),
    });
    (waiting, source)
}

/// What answering another device's check did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answered {
    Approved,
    Denied,
    /// The number typed wasn't the new device's, so the server denied it:
    /// each check takes one try.
    WrongNumber,
}

impl Answered {
    pub fn name(self) -> &'static str {
        match self {
            Answered::Approved => "approved",
            Answered::Denied => "denied",
            Answered::WrongNumber => "wrongNumber",
        }
    }

    /// From the server's answer to approving or denying.
    pub(crate) fn read(body: &Value) -> Answered {
        if body["approved"].as_bool() == Some(true) {
            Answered::Approved
        } else if body["deniedWhy"].as_str() == Some("number") {
            Answered::WrongNumber
        } else {
            Answered::Denied
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TOKEN: &str = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_abcde";

    /// The server's answer holding a sign-in, as passbundle-server sends it.
    fn held_answer(state: Value) -> Value {
        json!({
            "error": "invalid_grant",
            "error_description": "new device verification required",
            "ErrorModel": { "Message": "new device verification required", "Object": "error" },
            "KeykeeperNewDeviceCheck": state,
        })
    }

    fn state(email: &str) -> Value {
        json!({
            "id": "5f0c6f1e-6f3b-4a43-9a52-0d3f1c2b7e11",
            "token": TOKEN,
            "number": "47",
            "status": "waiting",
            "deniedWhy": null,
            "expiresAt": "2026-11-08T12:10:00.000000Z",
            "approvers": email != "now",
            "email": email,
            "emailAfter": null,
            "codeSent": false,
            "object": "keykeeperNewDeviceCheck",
        })
    }

    #[test]
    fn reads_a_held_sign_in_and_keeps_its_token_apart() {
        let held = held(&held_answer(state("later"))).unwrap();
        assert_eq!(&*held.token, TOKEN);
        assert_eq!(
            held.check,
            Check {
                id: "5f0c6f1e-6f3b-4a43-9a52-0d3f1c2b7e11".into(),
                number: "47".into(),
                status: Status::Waiting,
                denied_why: None,
                expires_at: Some("2026-11-08T12:10:00.000000Z".into()),
                approvers: true,
                email: Email::Later,
                email_after: None,
                code_sent: false,
                email_to: None,
            }
        );
        // The check the screens get never has the token.
        assert!(!format!("{:?}", held.check).contains(TOKEN));
    }

    #[test]
    fn other_answers_arent_a_held_sign_in() {
        // Two-step login, a wrong password, the setup session.
        for body in [
            json!({ "error": "invalid_grant", "TwoFactorProviders": ["0"], "TwoFactorProviders2": { "0": null } }),
            json!({ "error": "invalid_grant", "ErrorModel": { "Message": "Username or password is incorrect. Try again" } }),
            json!({ "error": "invalid_grant", "KeykeeperTwoStepSetup": TOKEN }),
        ] {
            assert!(held(&body).is_none());
        }
        // A check without a token, an id that isn't one, or a number that
        // isn't two digits.
        let mut no_token = state("later");
        no_token.as_object_mut().unwrap().remove("token");
        assert!(held(&held_answer(no_token)).is_none());
        let mut bad_id = state("later");
        bad_id["id"] = json!("../../api/sync");
        assert!(held(&held_answer(bad_id)).is_none());
        for number in [json!("4"), json!("470"), json!(47), json!("4a")] {
            let mut bad = state("later");
            bad["number"] = number;
            assert!(held(&held_answer(bad)).is_none());
        }
    }

    #[test]
    fn reads_each_way_a_code_by_email_stands() {
        for (said, email) in [
            ("now", Email::Now),
            ("later", Email::Later),
            ("waiting", Email::Waiting),
            ("never", Email::Never),
            ("something new", Email::Never),
        ] {
            assert_eq!(held(&held_answer(state(said))).unwrap().check.email, email);
        }
        // Asked for after the wait: when it may be sent.
        let mut asked = held(&held_answer(state("later"))).unwrap();
        let mut now = state("waiting");
        now["emailAfter"] = json!("2026-11-09T12:00:00.000000Z");
        now.as_object_mut().unwrap().remove("token");
        asked.check.update(&now).unwrap();
        assert_eq!(asked.check.email, Email::Waiting);
        assert_eq!(
            asked.check.email_after.as_deref(),
            Some("2026-11-09T12:00:00.000000Z")
        );
        // A code sent, and where to.
        let mut sent = state("now");
        sent["codeSent"] = json!(true);
        sent["emailTo"] = json!("a***@example.test");
        asked.check.update(&sent).unwrap();
        assert!(asked.check.code_sent);
        assert_eq!(asked.check.email_to.as_deref(), Some("a***@example.test"));
    }

    #[test]
    fn an_answer_about_another_check_isnt_this_ones() {
        let mut held = held(&held_answer(state("later"))).unwrap();
        let mut approved = state("later");
        approved["status"] = json!("approved");
        held.check.update(&approved).unwrap();
        assert_eq!(held.check.status, Status::Approved);
        let mut other = state("later");
        other["id"] = json!("another-check");
        assert!(held.check.update(&other).is_err());
        let mut renumbered = state("later");
        renumbered["number"] = json!("12");
        assert!(held.check.update(&renumbered).is_err());
        assert_eq!(held.check.number, "47");
        // Denied, and why.
        let mut denied = state("later");
        denied["status"] = json!("denied");
        denied["deniedWhy"] = json!("number");
        held.check.update(&denied).unwrap();
        assert_eq!(held.check.status, Status::Denied);
        assert_eq!(held.check.denied_why.as_deref(), Some("number"));
    }

    #[test]
    fn only_two_digits_are_a_number_to_send() {
        assert_eq!(typed_number("47"), Ok("47".into()));
        assert_eq!(typed_number(" 05 "), Ok("05".into()));
        for typed in ["", "4", "470", "4 7", "ab", "4a", "٤٧", "+4"] {
            assert_eq!(
                typed_number(typed),
                Err(Error::TwoStep(
                    "Type the two-digit number shown on the new device."
                )),
                "{typed:?}"
            );
        }
    }

    #[test]
    fn an_email_code_is_eight_digits() {
        assert_eq!(email_code("1234 5678"), Ok("12345678".into()));
        assert_eq!(email_code("1234-5678\n"), Ok("12345678".into()));
        for typed in ["", "1234567", "123456789", "1234567a"] {
            assert!(email_code(typed).is_err(), "{typed:?}");
        }
    }

    #[test]
    fn the_servers_refusals_are_said_plainly() {
        assert_eq!(
            refusal("That code didn't work. Check the email and try again."),
            Some(Error::TwoStep(
                "That code didn't work. Check the email and try again."
            ))
        );
        assert!(refusal("Your organization requires two-step login for your role, so this new device needs approval from a device where you're signed in, and none is. Sign in on a device you've used before.").is_some());
        assert_eq!(
            refusal("Username or password is incorrect. Try again"),
            None
        );
    }

    #[test]
    fn lists_another_devices_checks_by_the_servers_clock() {
        let body = json!({
            "data": [
                {
                    "id": "older",
                    "device": "Web vault in Firefox",
                    "deviceType": 10,
                    "ipAddress": "203.0.113.7",
                    "location": "Berlin, Germany",
                    "creationDate": "2026-11-08T12:00:00.000000Z",
                    "expirationDate": "2026-11-08T12:10:00.000000Z",
                    "emailAfter": null,
                },
                {
                    "id": "newer",
                    "device": "CLI on Linux",
                    "deviceType": 25,
                    "ipAddress": "10.0.0.2",
                    "location": null,
                    "creationDate": "2026-11-08T12:02:00.000000Z",
                    "expirationDate": "2026-11-08T12:12:00.000000Z",
                    "emailAfter": "2026-11-09T12:02:00.000000Z",
                },
                {
                    "id": "gone",
                    "device": "Extension in Chrome",
                    "creationDate": "2026-11-08T11:00:00.000000Z",
                    "expirationDate": "2026-11-08T11:10:00.000000Z",
                },
            ],
            "locationSource": { "name": "IP Geolocation by DB-IP", "url": "https://db-ip.com" },
            "serverDate": "2026-11-08T12:03:00.000000Z",
        });
        // This device's clock is a day off; the server's counts.
        let (waiting, source) = parse_waiting(&body, 0);
        assert_eq!(
            waiting.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["newer", "older"]
        );
        let older = &waiting[1];
        assert_eq!(older.device, "Web vault in Firefox");
        assert_eq!(older.device_type, 10);
        assert_eq!(older.location.as_deref(), Some("Berlin, Germany"));
        assert_eq!((older.age, older.left), (180, 420));
        assert_eq!(waiting[0].location, None);
        assert_eq!(
            waiting[0].email_after.as_deref(),
            Some("2026-11-09T12:02:00.000000Z")
        );
        assert_eq!(
            source,
            Some(LocationSource {
                name: "IP Geolocation by DB-IP".into(),
                url: Some("https://db-ip.com".into()),
            })
        );
        // A server without a location database credits nobody.
        let (_, none) = parse_waiting(&json!({ "data": [], "locationSource": null }), 0);
        assert_eq!(none, None);
    }

    #[test]
    fn reads_what_answering_did() {
        assert_eq!(
            Answered::read(&json!({ "approved": true, "denied": false })),
            Answered::Approved
        );
        assert_eq!(
            Answered::read(&json!({ "approved": false, "denied": true, "deniedWhy": "number" })),
            Answered::WrongNumber
        );
        assert_eq!(
            Answered::read(&json!({ "approved": false, "denied": true, "deniedWhy": "denied" })),
            Answered::Denied
        );
    }
}
