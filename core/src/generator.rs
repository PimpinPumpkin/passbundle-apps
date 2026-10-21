//! New passwords: random characters from the classes chosen, at least one of
//! each, drawn uniformly with the OS's random numbers.

use rand::{rngs::OsRng, seq::SliceRandom, Rng};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

#[derive(Clone, Debug)]
pub struct Options {
    pub length: usize,
    pub uppercase: bool,
    pub lowercase: bool,
    pub digits: bool,
    pub symbols: bool,
    /// Leave out characters that look alike (0 O, 1 l I).
    pub avoid_ambiguous: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            length: 20,
            uppercase: true,
            lowercase: true,
            digits: true,
            symbols: true,
            avoid_ambiguous: true,
        }
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
}
