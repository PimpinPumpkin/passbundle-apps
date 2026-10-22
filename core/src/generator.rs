//! New passwords and passphrases. Passwords: random characters from the
//! classes chosen, at least one of each. Passphrases: random words from EFF's
//! long word list (`words/`). Both drawn uniformly with the OS's random
//! numbers. The app remembers the last choices on the device.

use std::sync::OnceLock;

use rand::{rngs::OsRng, seq::SliceRandom, Rng};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

/// A password of random characters, or a passphrase of random words.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    #[default]
    Password,
    Passphrase,
}

/// What to make. Missing choices take the defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    pub kind: Kind,
    pub length: usize,
    pub uppercase: bool,
    pub lowercase: bool,
    pub digits: bool,
    pub symbols: bool,
    /// Leave out characters that look alike (0 O, 1 l I).
    pub avoid_ambiguous: bool,
    /// Passphrases: how many words, and what goes between them.
    pub words: usize,
    pub separator: String,
    /// Passphrases: each word starts with a capital letter.
    pub capitalize: bool,
    /// Passphrases: one word gets a digit at its end.
    pub include_number: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            kind: Kind::Password,
            length: 20,
            uppercase: true,
            lowercase: true,
            digits: true,
            symbols: true,
            avoid_ambiguous: true,
            // EFF's advice: at least six words, about 77 bits.
            words: 6,
            separator: "-".into(),
            capitalize: false,
            include_number: false,
        }
    }
}

/// A password or a passphrase, as the options say.
pub fn generate(options: &Options) -> Result<Zeroizing<String>> {
    match options.kind {
        Kind::Password => password(options),
        Kind::Passphrase => passphrase(options),
    }
}

pub fn password(options: &Options) -> Result<Zeroizing<String>> {
    let pick = |all: &str, ambiguous: &str| -> Vec<char> {
        all.chars()
            .filter(|c| !(options.avoid_ambiguous && ambiguous.contains(*c)))
            .collect()
    };
    let mut classes = Vec::new();
    if options.uppercase {
        classes.push(pick("ABCDEFGHIJKLMNOPQRSTUVWXYZ", "IO"));
    }
    if options.lowercase {
        classes.push(pick("abcdefghijklmnopqrstuvwxyz", "l"));
    }
    if options.digits {
        classes.push(pick("0123456789", "01"));
    }
    if options.symbols {
        classes.push(pick("!@#$%^&*-_=+?", ""));
    }
    if classes.is_empty() {
        return Err(Error::Refused("choose at least one kind of character"));
    }
    if !(classes.len()..=128).contains(&options.length) {
        return Err(Error::Refused(
            "a password needs a length from the number of kinds chosen to 128",
        ));
    }
    let all: Vec<char> = classes.iter().flatten().copied().collect();
    let mut chars: Vec<char> = classes
        .iter()
        .map(|class| class[OsRng.gen_range(0..class.len())])
        .collect();
    while chars.len() < options.length {
        chars.push(all[OsRng.gen_range(0..all.len())]);
    }
    chars.shuffle(&mut OsRng);
    let out = Zeroizing::new(chars.iter().collect::<String>());
    zeroize::Zeroize::zeroize(&mut chars);
    Ok(out)
}

/// EFF's long word list: 7,776 words, so each word adds about 12.9 bits.
fn words() -> &'static [&'static str] {
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| include_str!("words/eff-large.txt").lines().collect())
}

/// The longest word on the list.
const LONGEST_WORD: usize = 9;

pub fn passphrase(options: &Options) -> Result<Zeroizing<String>> {
    if !(3..=20).contains(&options.words) {
        return Err(Error::Refused("a passphrase needs from 3 to 20 words"));
    }
    if options.separator.chars().count() > 3 {
        return Err(Error::Refused("the separator can have up to 3 characters"));
    }
    let list = words();
    let number_at = options
        .include_number
        .then(|| OsRng.gen_range(0..options.words));
    // Room for everything up front, so the passphrase is never copied while it grows.
    let mut out = Zeroizing::new(String::with_capacity(
        options.words * (LONGEST_WORD + 1 + options.separator.len()),
    ));
    for at in 0..options.words {
        if at > 0 {
            out.push_str(&options.separator);
        }
        let word = list[OsRng.gen_range(0..list.len())];
        let mut letters = word.chars();
        match letters.next() {
            Some(first) if options.capitalize => {
                out.push(first.to_ascii_uppercase());
                out.push_str(letters.as_str());
            }
            _ => out.push_str(word),
        }
        if number_at == Some(at) {
            out.push(char::from(b'0' + OsRng.gen_range(0..10u8)));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_the_length_and_every_kind_chosen_and_nothing_ambiguous() {
        for _ in 0..200 {
            let p = password(&Options::default()).unwrap();
            assert_eq!(p.chars().count(), 20);
            assert!(p.chars().any(|c| c.is_ascii_uppercase()));
            assert!(p.chars().any(|c| c.is_ascii_lowercase()));
            assert!(p.chars().any(|c| c.is_ascii_digit()));
            assert!(p.chars().any(|c| !c.is_ascii_alphanumeric()));
            assert!(!p.chars().any(|c| "0O1lI".contains(c)), "{}", p.as_str());
        }
        let digits = password(&Options {
            length: 6,
            uppercase: false,
            lowercase: false,
            symbols: false,
            ..Options::default()
        })
        .unwrap();
        assert!(digits.chars().all(|c| c.is_ascii_digit()) && digits.len() == 6);
        assert_ne!(
            password(&Options::default()).unwrap(),
            password(&Options::default()).unwrap()
        );
    }

    #[test]
    fn refuses_no_kinds_or_a_silly_length() {
        let none = Options {
            uppercase: false,
            lowercase: false,
            digits: false,
            symbols: false,
            ..Options::default()
        };
        assert!(password(&none).is_err());
        assert!(password(&Options {
            length: 3,
            ..Options::default()
        })
        .is_err());
        assert!(password(&Options {
            length: 129,
            ..Options::default()
        })
        .is_err());
    }

    fn with(options: Options) -> Vec<String> {
        generate(&options)
            .unwrap()
            .split('_')
            .map(String::from)
            .collect()
    }

    fn passphrase_options() -> Options {
        Options {
            kind: Kind::Passphrase,
            separator: "_".into(),
            ..Options::default()
        }
    }

    #[test]
    fn the_word_list_is_effs_long_list() {
        let list = words();
        assert_eq!(list.len(), 7776);
        assert_eq!(
            list.iter().collect::<std::collections::HashSet<_>>().len(),
            7776
        );
        assert_eq!((list[0], list[7775]), ("abacus", "zoom"));
        assert!(list.iter().all(|w| (3..=LONGEST_WORD).contains(&w.len())
            && w.chars().all(|c| c.is_ascii_lowercase() || c == '-')));
    }

    #[test]
    fn a_passphrase_has_the_words_chosen_from_the_list() {
        for _ in 0..200 {
            let parts = with(passphrase_options());
            assert_eq!(parts.len(), 6);
            assert!(
                parts.iter().all(|p| words().contains(&p.as_str())),
                "{parts:?}"
            );
        }
        let four = with(Options {
            words: 4,
            ..passphrase_options()
        });
        assert_eq!(four.len(), 4);
        let spaced = generate(&Options {
            separator: String::new(),
            words: 3,
            ..passphrase_options()
        })
        .unwrap();
        assert!(spaced.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
        assert_ne!(with(passphrase_options()), with(passphrase_options()));
    }

    #[test]
    fn a_passphrase_can_be_capitalized_and_have_a_number() {
        for _ in 0..200 {
            let parts = with(Options {
                capitalize: true,
                include_number: true,
                ..passphrase_options()
            });
            assert!(parts
                .iter()
                .all(|p| p.starts_with(|c: char| c.is_ascii_uppercase())));
            let numbered: Vec<_> = parts
                .iter()
                .filter(|p| p.ends_with(|c: char| c.is_ascii_digit()))
                .collect();
            assert_eq!(numbered.len(), 1, "{parts:?}");
            for part in &parts {
                let word = part
                    .trim_end_matches(|c: char| c.is_ascii_digit())
                    .to_lowercase();
                assert!(words().contains(&word.as_str()), "{part}");
            }
        }
    }

    #[test]
    fn refuses_a_silly_number_of_words_or_a_long_separator() {
        for words in [0, 2, 21] {
            assert!(generate(&Options {
                words,
                ..passphrase_options()
            })
            .is_err());
        }
        assert!(generate(&Options {
            separator: "----".into(),
            ..passphrase_options()
        })
        .is_err());
    }

    #[test]
    fn options_read_from_json_with_defaults_for_what_is_missing() {
        let read: Options = serde_json::from_str(r#"{"kind":"passphrase","words":4}"#).unwrap();
        assert_eq!(
            read,
            Options {
                kind: Kind::Passphrase,
                words: 4,
                ..Options::default()
            }
        );
        assert_eq!(
            serde_json::from_str::<Options>("{}").unwrap(),
            Options::default()
        );
        let written = serde_json::to_value(Options::default()).unwrap();
        assert_eq!(written["kind"], "password");
        assert_eq!(written["includeNumber"], false);
    }
}
