//! An organization's "Password generator" policy (docs/core.md, "New
//! passwords"): upstream's policy type 2, as the server sends it with each
//! sync. Unlike "Vault lock", it binds everyone, owners too, as in upstream's
//! own apps. Across organizations the strictest of each part wins, the way
//! upstream combines them: the highest minimums, any requirement turned on,
//! and a password over a passphrase when two organizations disagree. The
//! generator makes what the person chose, held to this.

use serde::Serialize;
use serde_json::Value;

use crate::generator::{self, Kind, Options};

/// Upstream's number for the policy.
const PASSWORD_GENERATOR: i64 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratorPolicy {
    /// The only kind it may make, if an organization says.
    pub kind: Option<Kind>,
    /// Passwords: the shortest allowed, the kinds of characters each must
    /// have, and how many digits and symbols at least.
    pub min_length: usize,
    pub uppercase: bool,
    pub lowercase: bool,
    pub digits: bool,
    pub symbols: bool,
    pub min_digits: usize,
    pub min_symbols: usize,
    /// Passphrases: the fewest words, capitals, and a number.
    pub min_words: usize,
    pub capitalize: bool,
    pub include_number: bool,
}

/// A whole number in a policy, held to what the generator can do.
fn count(data: &Value, key: &str, most: usize) -> usize {
    data.get(key)
        .and_then(Value::as_u64)
        .map_or(0, |n| usize::try_from(n).unwrap_or(most).min(most))
}

fn on(data: &Value, key: &str) -> bool {
    data.get(key).and_then(Value::as_bool) == Some(true)
}

impl GeneratorPolicy {
    /// The policy in a sync (the kept `sync.json`); none if it has none.
    pub fn from_sync(text: &str) -> GeneratorPolicy {
        let mut policy = GeneratorPolicy::default();
        let Ok(sync) = serde_json::from_str::<Value>(text) else {
            return policy;
        };
        let policies = sync.get("policies").and_then(Value::as_array);
        for p in policies.into_iter().flatten() {
            if p.get("type").and_then(Value::as_i64) != Some(PASSWORD_GENERATOR)
                || p.get("enabled").and_then(Value::as_bool) != Some(true)
            {
                continue;
            }
            let Some(data) = p.get("data").filter(|d| d.is_object()) else {
                continue;
            };
            // Once one organization says password, it's a password.
            if policy.kind != Some(Kind::Password) {
                match data.get("overridePasswordType").and_then(Value::as_str) {
                    Some("password") => policy.kind = Some(Kind::Password),
                    Some("passphrase") => policy.kind = Some(Kind::Passphrase),
                    _ => {}
                }
            }
            policy.min_length = policy.min_length.max(count(data, "minLength", 128));
            policy.uppercase |= on(data, "useUpper");
            policy.lowercase |= on(data, "useLower");
            policy.digits |= on(data, "useNumbers");
            policy.symbols |= on(data, "useSpecial");
            policy.min_digits = policy.min_digits.max(count(data, "minNumbers", 9));
            policy.min_symbols = policy.min_symbols.max(count(data, "minSpecial", 9));
            policy.min_words = policy.min_words.max(count(data, "minNumberWords", 20));
            policy.capitalize |= on(data, "capitalize");
            policy.include_number |= on(data, "includeNumber");
        }
        policy
    }

    /// The person's choices, held to the policy.
    pub fn apply(&self, chosen: &Options) -> Options {
        let mut held = Options {
            kind: self.kind.unwrap_or(chosen.kind),
            uppercase: chosen.uppercase || self.uppercase,
            lowercase: chosen.lowercase || self.lowercase,
            digits: chosen.digits || self.digits || self.min_digits > 0,
            symbols: chosen.symbols || self.symbols || self.min_symbols > 0,
            min_digits: chosen.min_digits.max(self.min_digits),
            min_symbols: chosen.min_symbols.max(self.min_symbols),
            words: chosen.words.max(self.min_words),
            capitalize: chosen.capitalize || self.capitalize,
            include_number: chosen.include_number || self.include_number,
            ..chosen.clone()
        };
        held.length = chosen
            .length
            .max(self.min_length)
            .max(generator::shortest(&held));
        held
    }

    /// Whether any organization sets anything.
    pub fn is_set(&self) -> bool {
        *self != GeneratorPolicy::default()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sync(policies: Value) -> String {
        json!({ "profile": { "organizations": [
            { "id": "owned", "type": 0, "status": 2 },
            { "id": "member", "type": 2, "status": 2 },
        ] }, "policies": policies })
        .to_string()
    }

    fn generator_policy(org: &str, data: Value) -> Value {
        json!({ "id": format!("p-{org}"), "organizationId": org, "type": 2, "enabled": true, "data": data, "object": "policy" })
    }

    #[test]
    fn none_without_an_enabled_password_generator_policy() {
        assert_eq!(GeneratorPolicy::from_sync("{}"), GeneratorPolicy::default());
        assert_eq!(
            GeneratorPolicy::from_sync("not json"),
            GeneratorPolicy::default()
        );
        let others = sync(json!([
            { "organizationId": "a", "type": 1, "enabled": true, "data": { "minLength": 12 } },
            { "organizationId": "b", "type": 2, "enabled": false, "data": { "minLength": 30 } },
            { "organizationId": "c", "type": 2, "enabled": true, "data": null },
            { "organizationId": "d", "type": 9, "enabled": true, "data": { "minutes": 5 } },
        ]));
        let none = GeneratorPolicy::from_sync(&others);
        assert_eq!(none, GeneratorPolicy::default());
        assert!(!none.is_set());
    }

    #[test]
    fn binds_owners_too_and_reads_each_part() {
        let policy = GeneratorPolicy::from_sync(&sync(json!([generator_policy(
            "owned",
            json!({
                "overridePasswordType": "passphrase", "minLength": 16, "useUpper": true,
                "useLower": null, "useNumbers": true, "useSpecial": false, "minNumbers": 2,
                "minSpecial": 1, "minNumberWords": 5, "capitalize": true, "includeNumber": false,
            })
        )])));
        assert_eq!(
            policy,
            GeneratorPolicy {
                kind: Some(Kind::Passphrase),
                min_length: 16,
                uppercase: true,
                digits: true,
                min_digits: 2,
                min_symbols: 1,
                min_words: 5,
                capitalize: true,
                ..GeneratorPolicy::default()
            }
        );
        assert!(policy.is_set());
    }

    #[test]
    fn the_strictest_of_each_part_wins_and_a_password_over_a_passphrase() {
        let policy = GeneratorPolicy::from_sync(&sync(json!([
            generator_policy(
                "owned",
                json!({ "overridePasswordType": "passphrase", "minLength": 12, "minNumbers": 3, "minNumberWords": 7 })
            ),
            generator_policy(
                "member",
                json!({ "overridePasswordType": "password", "minLength": 20, "minNumbers": 1, "useSpecial": true })
            ),
            generator_policy(
                "x",
                json!({ "overridePasswordType": "passphrase", "minNumberWords": 4, "includeNumber": true })
            ),
        ])));
        assert_eq!(policy.kind, Some(Kind::Password));
        assert_eq!(
            (policy.min_length, policy.min_digits, policy.min_words),
            (20, 3, 7)
        );
        assert!(policy.symbols && policy.include_number);
        // Silly numbers are held to what the generator can do.
        let big = GeneratorPolicy::from_sync(&sync(json!([generator_policy(
            "a",
            json!({ "minLength": 999, "minNumbers": 99, "minSpecial": -1, "minNumberWords": 50 })
        )])));
        assert_eq!(
            (
                big.min_length,
                big.min_digits,
                big.min_symbols,
                big.min_words
            ),
            (128, 9, 0, 20)
        );
    }

    #[test]
    fn holds_the_persons_choices_to_it() {
        let chosen = Options {
            length: 10,
            symbols: false,
            words: 3,
            ..Options::default()
        };
        assert_eq!(GeneratorPolicy::default().apply(&chosen), chosen);
        let policy = GeneratorPolicy {
            kind: Some(Kind::Passphrase),
            min_length: 14,
            min_symbols: 2,
            min_words: 5,
            include_number: true,
            ..GeneratorPolicy::default()
        };
        let held = policy.apply(&chosen);
        assert_eq!(held.kind, Kind::Passphrase);
        assert_eq!((held.length, held.words), (14, 5));
        assert!(held.symbols && held.min_symbols == 2 && held.include_number);
        // Choices stricter than the policy stay.
        assert_eq!(
            policy
                .apply(&Options {
                    length: 30,
                    words: 8,
                    ..chosen.clone()
                })
                .length,
            30
        );
        // Long enough for every minimum, whatever the policy's own length.
        let crowded = GeneratorPolicy {
            min_digits: 9,
            min_symbols: 9,
            ..GeneratorPolicy::default()
        }
        .apply(&Options {
            length: 4,
            ..Options::default()
        });
        assert_eq!(crowded.length, 20);
        let made = generator::generate(&crowded).unwrap();
        assert!(made.chars().filter(|c| c.is_ascii_digit()).count() >= 9);
    }
}
