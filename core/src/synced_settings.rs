//! The generator's settings, the same on each of someone's devices (meta
//! DESIGN.md, "Generator settings"). The apps keep a small document on the
//! server, encrypted with the account's user key, whose `generator` part is
//! `generator::Options` as it serializes; the web vault and the extension
//! read and write the same document. The server can't read it.
//!
//! This device remembers what the server has, as far as it knows (`Known`),
//! so a change made here that isn't saved yet wins over the server's copy at
//! the next sync, and other settings in the document go back unchanged. The
//! document says what it is (`SETTINGS_TYPE`), so nothing else encrypted with
//! the account's key passes for it, and numbers each save (`rev`), so a copy
//! older than one this device has seen is refused.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::generator::{Kind, Options};

/// What the server has, as far as this device knows, for one account.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Known {
    /// The server and email it's for; another account starts over.
    pub account: String,
    /// The whole document, other settings and all.
    pub settings: Map<String, Value>,
    /// The generator's settings in it, as this device took them.
    pub generator: Options,
}

/// What the document says it is.
pub const SETTINGS_TYPE: &str = "passbundle.settings";

/// A document's number; 0 for none.
pub fn rev_of(settings: &Map<String, Value>) -> u64 {
    settings.get("rev").and_then(Value::as_u64).unwrap_or(0)
}

/// The number for the next save: after every one seen, and the time, in
/// milliseconds, so a later save on another device numbers higher too.
pub fn next_rev(seen: u64, now_ms: u64) -> u64 {
    (seen + 1).max(now_ms)
}

/// The time, in milliseconds, for `next_rev`.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// The account a `Known` is for.
pub fn account(server: &str, email: &str) -> String {
    format!("{server}\n{email}")
}

/// The document with these generator settings over `base`'s, keeping the
/// rest of it (and generator settings other apps add), numbered after `seen`.
pub fn document(
    base: &Map<String, Value>,
    generator: &Options,
    seen: u64,
    now_ms: u64,
) -> Map<String, Value> {
    let mut settings = base.clone();
    let mut merged = base
        .get("generator")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Value::Object(ours) = serde_json::to_value(generator).expect("options always serialize")
    {
        merged.extend(ours);
    }
    settings.insert("type".into(), SETTINGS_TYPE.into());
    settings.insert("rev".into(), next_rev(seen, now_ms).into());
    settings.insert("generator".into(), Value::Object(merged));
    settings
}

/// The document as text to encrypt. serde_json keeps keys sorted.
pub fn text(settings: &Map<String, Value>) -> String {
    Value::Object(settings.clone()).to_string()
}

/// The document from its text, or nothing if it isn't one.
pub fn parse(text: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str(text) {
        Ok(Value::Object(settings))
            if settings.get("type").and_then(Value::as_str) == Some(SETTINGS_TYPE) =>
        {
            Some(settings)
        }
        _ => None,
    }
}

/// The document's generator settings applied over these. Each one that's
/// missing or isn't what it should be keeps its value here, and numbers and
/// the separator are held to what this generator allows. (The web vault
/// allows a little less: 5 characters at least, and a one-character
/// separator. It takes what it can and leaves the document's as they are.)
pub fn take(settings: &Map<String, Value>, here: &Options) -> Options {
    let empty = json!({});
    let g = settings.get("generator").unwrap_or(&empty);
    let whole = |key: &str, min: usize, max: usize, otherwise: usize| {
        g.get(key)
            .and_then(Value::as_i64)
            .map_or(otherwise, |n| n.clamp(min as i64, max as i64) as usize)
    };
    let yes_no =
        |key: &str, otherwise: bool| g.get(key).and_then(Value::as_bool).unwrap_or(otherwise);
    Options {
        kind: match g.get("kind").and_then(Value::as_str) {
            Some("password") => Kind::Password,
            Some("passphrase") => Kind::Passphrase,
            _ => here.kind,
        },
        length: whole("length", 4, 128, here.length),
        uppercase: yes_no("uppercase", here.uppercase),
        lowercase: yes_no("lowercase", here.lowercase),
        digits: yes_no("digits", here.digits),
        symbols: yes_no("symbols", here.symbols),
        avoid_ambiguous: yes_no("avoidAmbiguous", here.avoid_ambiguous),
        min_digits: whole("minDigits", 0, 9, here.min_digits),
        min_symbols: whole("minSymbols", 0, 9, here.min_symbols),
        words: whole("words", 3, 20, here.words),
        separator: g
            .get("separator")
            .and_then(Value::as_str)
            .map_or_else(|| here.separator.clone(), |s| s.chars().take(3).collect()),
        capitalize: yes_no("capitalize", here.capitalize),
        include_number: yes_no("includeNumber", here.include_number),
    }
}

/// What to do at a sync, given what this device has and knows.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// A change here the server hasn't got: save the document with it.
    Save(Map<String, Value>),
    /// Nothing changed here: read the server's.
    Read,
}

/// Whether there's a change here to save first.
pub fn step(known: Option<&Known>, account: &str, here: &Options, now_ms: u64) -> Step {
    match known {
        Some(k) if k.account == account && &k.generator != here => {
            Step::Save(document(&k.settings, here, rev_of(&k.settings), now_ms))
        }
        _ => Step::Read,
    }
}

/// What this device has seen of the account's document: its number.
pub fn seen(known: Option<&Known>, account: &str) -> u64 {
    known
        .filter(|k| k.account == account)
        .map_or(0, |k| rev_of(&k.settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;

    fn doc(text: &str) -> Map<String, Value> {
        parse(text).unwrap()
    }

    #[test]
    fn the_document_names_the_apps_options_as_the_web_vault_does() {
        let made = document(&Map::new(), &Options::default(), 0, NOW);
        assert_eq!(
            text(&made),
            r#"{"generator":{"avoidAmbiguous":true,"capitalize":false,"digits":true,"includeNumber":false,"kind":"password","length":20,"lowercase":true,"minDigits":1,"minSymbols":1,"separator":"-","symbols":true,"uppercase":true,"words":6},"rev":1800000000000,"type":"passbundle.settings"}"#
        );
    }

    #[test]
    fn only_passbundles_settings_document_is_one() {
        assert!(parse(r#"{"type":"passbundle.settings","generator":{}}"#).is_some());
        // Something else encrypted with the account's key doesn't pass for it.
        assert!(parse(r#"{"generator":{"length":8}}"#).is_none());
        assert!(parse(r#""Work email""#).is_none());
        assert!(parse("[1]").is_none());
        assert!(parse("null").is_none());
        assert!(parse("not json").is_none());
    }

    #[test]
    fn each_save_numbers_after_every_one_seen_and_after_the_time() {
        assert_eq!(next_rev(41, 7), 42);
        assert_eq!(next_rev(41, 1000), 1000);
        assert_eq!(
            rev_of(&doc(r#"{"type":"passbundle.settings","rev":41}"#)),
            41
        );
        assert_eq!(
            rev_of(&doc(r#"{"type":"passbundle.settings","rev":-1}"#)),
            0
        );
        assert_eq!(
            rev_of(&doc(r#"{"type":"passbundle.settings","rev":"9"}"#)),
            0
        );
        assert_eq!(rev_of(&Map::new()), 0);
    }

    #[test]
    fn saving_keeps_the_rest_of_the_document_and_other_apps_generator_settings() {
        let base = doc(
            r#"{"type":"passbundle.settings","rev":5,"somethingElse":{"kept":true},"generator":{"length":24,"pin":{"digits":6}}}"#,
        );
        let here = Options {
            length: 40,
            ..Options::default()
        };
        let saved = document(&base, &here, rev_of(&base), 3);
        assert_eq!(saved["rev"], 6);
        assert_eq!(saved["somethingElse"], json!({ "kept": true }));
        assert_eq!(saved["generator"]["length"], 40);
        assert_eq!(saved["generator"]["pin"], json!({ "digits": 6 }));
    }

    #[test]
    fn taking_them_keeps_whats_here_for_anything_missing_or_wrong() {
        let here = Options::default();
        let taken = take(
            &doc(
                r#"{"type":"passbundle.settings","generator":{"kind":"passphrase","length":"long","symbols":false,"words":8}}"#,
            ),
            &here,
        );
        assert_eq!(taken.kind, Kind::Passphrase);
        assert_eq!(taken.length, 20);
        assert!(!taken.symbols);
        assert_eq!(taken.words, 8);
        assert_eq!(take(&Map::new(), &here), here);
    }

    #[test]
    fn taking_them_holds_numbers_and_the_separator_to_the_generators_limits() {
        let taken = take(
            &doc(
                r#"{"type":"passbundle.settings","generator":{"length":9999,"minDigits":-3,"minSymbols":12,"words":1,"separator":"🙂🙂🙂🙂","digits":"yes"}}"#,
            ),
            &Options::default(),
        );
        assert_eq!(taken.length, 128);
        assert_eq!(taken.min_digits, 0);
        assert_eq!(taken.min_symbols, 9);
        assert_eq!(taken.words, 3);
        assert_eq!(taken.separator, "🙂🙂🙂");
        assert!(taken.digits);
    }

    #[test]
    fn they_come_back_the_same() {
        let changed = Options {
            kind: Kind::Passphrase,
            length: 4,
            symbols: false,
            avoid_ambiguous: false,
            min_digits: 4,
            words: 9,
            separator: "-+-".into(),
            capitalize: true,
            include_number: true,
            ..Options::default()
        };
        assert_eq!(
            take(
                &document(&Map::new(), &changed, 0, NOW),
                &Options::default()
            ),
            changed
        );
    }

    #[test]
    fn a_change_here_is_saved_first() {
        let account = account("https://pb.test", "a@example.test");
        let known = Known {
            account: account.clone(),
            settings: document(&Map::new(), &Options::default(), 0, NOW),
            generator: Options::default(),
        };
        let here = Options {
            length: 40,
            ..Options::default()
        };
        let Step::Save(saved) = step(Some(&known), &account, &here, 3) else {
            panic!("a change here should be saved")
        };
        assert_eq!(saved["generator"]["length"], 40);
        assert_eq!(saved["rev"], NOW + 1);
        // Nothing changed here, or it's another account's: read the server's.
        assert_eq!(
            step(Some(&known), &account, &Options::default(), 3),
            Step::Read
        );
        assert_eq!(step(Some(&known), "another", &here, 3), Step::Read);
        assert_eq!(step(None, &account, &here, 3), Step::Read);
        assert_eq!(seen(Some(&known), &account), NOW);
        assert_eq!(seen(Some(&known), "another"), 0);
    }
}
