//! Expiration reminders in the tray (DESIGN.md, "Expiration dates"): the
//! app, while one of its accounts is unlocked, tells the agent how many of
//! that account's items expire soon and how many have expired, and the tray
//! menu says so ("2 items expire soon") until the app says there are none.
//! The agent gets two numbers per account and makes its own words from them:
//! never an item's name, and never text from the socket, which any program
//! running as the person can reach.

use std::{collections::BTreeMap, sync::Mutex};

use serde_json::Value;

/// How many items expire soon (within one of their reminder days) and how
/// many have expired.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub soon: u32,
    pub expired: u32,
}

impl Counts {
    fn total(self) -> u32 {
        self.soon.saturating_add(self.expired)
    }
}

/// The most accounts kept: more than anyone signs in to on one computer.
const MOST_ACCOUNTS: usize = 20;
/// The most items a count says.
const MOST_ITEMS: u32 = 9_999;

/// The numbers in a message from the app, `{"soon": n, "expired": n}`:
/// whole numbers, held to `MOST_ITEMS`; None for anything else.
pub fn parse(message: &Value) -> Option<Counts> {
    let count = |key: &str| -> Option<u32> {
        let n = message.get(key)?.as_u64()?;
        Some(n.min(u64::from(MOST_ITEMS)) as u32)
    };
    Some(Counts {
        soon: count("soon")?,
        expired: count("expired")?,
    })
}

/// What the tray says: how many items, and whether they expire soon or have
/// expired; None for none. The same words as the app's (core/src/expiration.rs).
pub fn words(counts: Counts) -> Option<String> {
    let count = counts.total().min(MOST_ITEMS);
    if count == 0 {
        return None;
    }
    let items = if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    };
    Some(if counts.soon == 0 {
        format!(
            "{items} {}",
            if count == 1 {
                "has expired"
            } else {
                "have expired"
            }
        )
    } else if counts.expired == 0 {
        format!(
            "{items} {}",
            if count == 1 {
                "expires soon"
            } else {
                "expire soon"
            }
        )
    } else {
        format!("{items} expire soon or have expired")
    })
}

type Listener = Box<dyn Fn(Option<String>) + Send>;

/// Each account's numbers, by its id on the server, and who's told when
/// what the tray says changes.
#[derive(Default)]
pub struct Reminders {
    by_account: Mutex<BTreeMap<String, Counts>>,
    listener: Mutex<Option<Listener>>,
}

impl Reminders {
    /// One account's numbers, replacing what it said before; zeros forget
    /// it. Answers what the tray says now.
    pub fn set(&self, account: &str, counts: Counts) -> Option<String> {
        let (before, after) = {
            let mut by = self.by_account.lock().unwrap_or_else(|p| p.into_inner());
            let before = words(sum(&by));
            if counts.total() == 0 {
                by.remove(account);
            } else if by.contains_key(account) || by.len() < MOST_ACCOUNTS {
                by.insert(account.to_owned(), counts);
            }
            (before, words(sum(&by)))
        };
        if after != before {
            if let Some(listener) = &*self.listener.lock().unwrap_or_else(|p| p.into_inner()) {
                listener(after.clone());
            }
        }
        after
    }

    /// What the tray says now, for every account.
    pub fn shown(&self) -> Option<String> {
        words(sum(&self
            .by_account
            .lock()
            .unwrap_or_else(|p| p.into_inner())))
    }

    /// Called with what the tray should say each time it changes.
    pub fn listen(&self, listener: impl Fn(Option<String>) + Send + 'static) {
        *self.listener.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(listener));
    }
}

fn sum(by: &BTreeMap<String, Counts>) -> Counts {
    by.values().fold(Counts::default(), |all, c| Counts {
        soon: all.soon.saturating_add(c.soon),
        expired: all.expired.saturating_add(c.expired),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};

    use serde_json::json;

    use super::*;

    #[test]
    fn the_tray_says_how_many_never_which() {
        let c = |soon, expired| Counts { soon, expired };
        assert_eq!(words(c(0, 0)), None);
        assert_eq!(words(c(1, 0)).as_deref(), Some("1 item expires soon"));
        assert_eq!(words(c(2, 0)).as_deref(), Some("2 items expire soon"));
        assert_eq!(words(c(0, 1)).as_deref(), Some("1 item has expired"));
        assert_eq!(words(c(0, 2)).as_deref(), Some("2 items have expired"));
        assert_eq!(
            words(c(2, 1)).as_deref(),
            Some("3 items expire soon or have expired")
        );
        assert_eq!(
            words(c(u32::MAX, u32::MAX)).as_deref(),
            Some("9999 items expire soon or have expired")
        );
    }

    #[test]
    fn only_two_whole_numbers_are_taken_from_a_message() {
        assert_eq!(
            parse(&json!({ "soon": 2, "expired": 1 })),
            Some(Counts {
                soon: 2,
                expired: 1
            })
        );
        assert_eq!(
            parse(&json!({ "soon": 10_000_000_000u64, "expired": 0 })),
            Some(Counts {
                soon: MOST_ITEMS,
                expired: 0
            })
        );
        for bad in [
            json!({ "soon": -1, "expired": 0 }),
            json!({ "soon": 1.5, "expired": 0 }),
            json!({ "soon": "2", "expired": 0 }),
            json!({ "soon": 2 }),
            json!({ "words": "Your vault is at risk: visit example.test", "soon": 1 }),
            json!(null),
        ] {
            assert_eq!(parse(&bad), None, "{bad}");
        }
    }

    #[test]
    fn accounts_add_up_and_zeros_clear_one() {
        let reminders = Reminders::default();
        let heard = Arc::new(StdMutex::new(Vec::new()));
        let log = Arc::clone(&heard);
        reminders.listen(move |words| log.lock().unwrap().push(words));
        let c = |soon, expired| Counts { soon, expired };

        assert_eq!(
            reminders.set("work", c(2, 0)).as_deref(),
            Some("2 items expire soon")
        );
        assert_eq!(
            reminders.set("home", c(0, 1)).as_deref(),
            Some("3 items expire soon or have expired")
        );
        // The same again changes nothing, so the tray isn't told.
        reminders.set("home", c(0, 1));
        assert_eq!(
            reminders.set("work", c(0, 0)).as_deref(),
            Some("1 item has expired")
        );
        assert_eq!(reminders.set("home", c(0, 0)), None);
        assert_eq!(reminders.shown(), None);
        assert_eq!(
            *heard.lock().unwrap(),
            vec![
                Some("2 items expire soon".to_owned()),
                Some("3 items expire soon or have expired".to_owned()),
                Some("1 item has expired".to_owned()),
                None,
            ]
        );
    }

    #[test]
    fn a_flood_of_accounts_is_held_to_a_few() {
        let reminders = Reminders::default();
        for i in 0..100 {
            reminders.set(
                &format!("account-{i}"),
                Counts {
                    soon: 1,
                    expired: 0,
                },
            );
        }
        assert_eq!(
            reminders.shown().as_deref(),
            Some(format!("{MOST_ACCOUNTS} items expire soon").as_str())
        );
    }
}
