//! LastPass's CSV export (our fork's `LastPassCsvImporter`, with upstream's
//! new item types off, as PassBundle's server sends them): `url,username,
//! password,totp,extra,name,grouping,fav`, or a form fill's columns
//! (`profilename`, `profilelanguage`, a card's and a person's).
//!
//! - **Folders:** the grouping, backslashes as slashes and control
//!   characters left out, as LastPass names it ("(none)" is none).
//! - **Secure notes** are rows whose url is `http://sn`, their details in
//!   `extra` as `Key:value` lines under `NoteType:`. A credit card's is a
//!   card (its brand from the number, or else its type; a type that isn't a
//!   brand, and a start date, as fields), an address's an identity (its
//!   phone the identity's), and every other kind a note with `extra` as it
//!   is. LastPass's Language is left out; a line it doesn't know is a field.
//! - **A form fill** is an identity when it has a person's details, with its
//!   card as a card of its own, or a card.
//! - LastPass's direct import (signing in to LastPass) is the web vault's.

use std::collections::HashMap;

use serde_json::json;
use zeroize::Zeroizing;

use super::{blank, key_value, login, uris, value, z, Format, Item, Parsed};
use crate::csv::Table;
use crate::error::{Error, Result};
use crate::vault::{Card, Identity};

type Record = HashMap<String, Zeroizing<String>>;

fn get<'a>(r: &'a Record, column: &str) -> Option<&'a str> {
    r.get(column).map(|v| v.as_str())
}

pub(super) fn read(table: &Table) -> Result<Parsed> {
    let has = |c: &str| table.columns.iter().any(|n| n == c);
    let sites = has("url") && has("name") && (has("extra") || has("grouping"));
    let form_fills = has("profilename") && has("profilelanguage");
    if !table.records.is_empty() && !sites && !form_fills {
        return Err(Error::Refused(
            "that isn't LastPass's CSV export: it has no url, name, and grouping columns. Choose its format",
        ));
    }
    let mut parsed = Parsed::new(Format::LastPassCsv);
    for r in &table.records {
        let grouping = get(r, "grouping").map(|g| {
            g.replace('\\', "/")
                .chars()
                .filter(|c| !matches!(*c as u32, 0..=0x1f | 0x7f..=0x9f))
                .collect::<String>()
        });
        let folder = grouping
            .as_deref()
            .filter(|g| value(Some(g)).unwrap_or("(none)") != "(none)")
            .map(|g| parsed.folder_named(g));
        let mut item = base(r);
        let mut card_too = None;
        match item.kind {
            1 => {
                item.notes = value(get(r, "extra")).map(z);
                item.uris = uris(get(r, "url"));
                item.username = value(get(r, "username")).map(str::to_owned);
                item.password = value(get(r, "password")).map(z);
                item.totp = value(get(r, "totp")).map(z);
            }
            2 => secure_note(r, &mut item),
            3 => {
                item.card = Some(card(r));
                item.notes = value(get(r, "notes")).map(z);
            }
            _ => {
                item.identity = Some(identity(r));
                item.notes = value(get(r, "notes")).map(z);
                if value(get(r, "ccnum")).is_some() {
                    // A card on this form fill too.
                    let mut also = base(r);
                    also.kind = 3;
                    also.card = Some(card(r));
                    card_too = Some(also);
                }
            }
        }
        // Upstream puts only the card in the folder; both are the form fill's.
        for mut item in card_too.into_iter().chain([item]) {
            if blank(&item.name) {
                item.name = "--".to_owned();
            }
            if item.notes.as_deref().is_some_and(|n| blank(n)) {
                item.notes = None;
            }
            item.folder = folder;
            parsed.items.push(item);
        }
    }
    Ok(parsed)
}

/// A site, a secure note, or a form fill (`buildBaseCipher`).
fn base(r: &Record) -> Item {
    let mut item = login();
    if r.contains_key("profilename") && r.contains_key("profilelanguage") {
        item.name = value(get(r, "profilename")).unwrap_or("--").to_owned();
        let person = [
            "title",
            "firstname",
            "lastname",
            "address1",
            "phone",
            "username",
            "email",
        ];
        item.kind = if person.iter().any(|c| value(get(r, c)).is_some()) {
            4
        } else {
            3
        };
    } else {
        item.favorite = value(get(r, "fav")).unwrap_or("0") == "1";
        item.name = value(get(r, "name")).unwrap_or("--").to_owned();
        if get(r, "url") == Some("http://sn") {
            item.kind = 2;
        }
    }
    item
}

/// A form fill's card (`parseCard`): its expiry as "2030-06".
fn card(r: &Record) -> Card {
    let mut card = Card {
        cardholder_name: value(get(r, "ccname")).map(str::to_owned),
        number: value(get(r, "ccnum")).map(z),
        code: value(get(r, "cccsc")).map(z),
        ..Card::default()
    };
    card.brand = super::card_brand(card.number.as_deref().map(String::as_str));
    if let Some(expiry) = value(get(r, "ccexp")).filter(|e| e.contains('-')) {
        let mut parts = expiry.split('-');
        let year = parts.next().unwrap_or("");
        if let Some(month) = parts.next() {
            card.exp_year = Some(year.to_owned());
            let month = match month.strip_prefix('0') {
                Some(rest) if month.chars().count() == 2 => rest,
                _ => month,
            };
            card.exp_month = Some(month.to_owned());
        }
    }
    card
}

/// A form fill's person (`parseIdentity`): the title with a capital.
fn identity(r: &Record) -> Identity {
    let plain = |c: &str| value(get(r, c)).map(str::to_owned);
    let title = plain("title").map(|t| {
        let mut chars = t.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    });
    Identity {
        title,
        first_name: plain("firstname"),
        middle_name: plain("middlename"),
        last_name: plain("lastname"),
        username: plain("username"),
        company: plain("company"),
        ssn: value(get(r, "ssn")).map(z),
        address1: plain("address1"),
        address2: plain("address2"),
        address3: plain("address3"),
        city: plain("city"),
        state: plain("state"),
        postal_code: plain("zip"),
        country: plain("country"),
        email: plain("email"),
        phone: plain("phone"),
        ..Identity::default()
    }
}

/// A secure note (`parseSecureNote`): a card's or an address's details
/// from its `NoteType:` lines, or else a note with `extra` as it is.
fn secure_note(r: &Record, item: &mut Item) {
    let extra = get(r, "extra").unwrap_or("");
    let lines: Vec<&str> = extra
        .split("\r\n")
        .flat_map(|l| l.split(['\r', '\n']))
        .collect();
    let mut kind = lines.first().copied().unwrap_or("").split(':');
    let is_type = kind.next() == Some("NoteType");
    let mut notes = Zeroizing::new(String::new());
    match kind.next() {
        Some("Credit Card") if is_type => {
            let mut d = details(item, &mut notes, &lines, &CARD_KEYS);
            let number = d.remove("number").map(|n| z(&n));
            let mut card = Card {
                cardholder_name: d.remove("cardholderName").map(|v| v.to_string()),
                code: d.remove("code").map(|c| z(&c)),
                ..Card::default()
            };
            // "June,2020": the month by its name.
            if let Some(expiry) = d
                .remove("expMonth")
                .filter(|e| !blank(e) && e.as_str() != ",")
            {
                let mut parts = expiry.split(',');
                card.exp_month = parts.next().filter(|m| !blank(m)).and_then(month_number);
                if let Some(year) = parts.next().filter(|y| !blank(y)) {
                    card.exp_year = Some(year.to_owned());
                }
            }
            let lastpass_type = d.remove("lastPassType");
            let from_type = super::card_brand_named(lastpass_type.as_deref().map(String::as_str));
            card.brand =
                super::card_brand(number.as_deref().map(String::as_str)).or(from_type.clone());
            card.number = number;
            if let Some(t) = lastpass_type.filter(|t| !blank(t)) {
                if from_type != card.brand {
                    key_value(item, &mut notes, "Type", &t, 0);
                }
            }
            if let Some(start) = d
                .remove("startDate")
                .filter(|s| !blank(&s.replace(',', "")))
            {
                key_value(item, &mut notes, "Start Date", &start, 0);
            }
            item.kind = 3;
            item.card = Some(card);
        }
        Some("Address") if is_type => {
            let mut d = details(item, &mut notes, &lines, &ADDRESS_KEYS);
            let mut take = |k: &str| d.remove(k).map(|v| v.to_string());
            item.identity = Some(Identity {
                title: take("title"),
                first_name: take("firstName"),
                last_name: take("lastName"),
                middle_name: take("middleName"),
                company: take("company"),
                address1: take("address1"),
                address2: take("address2"),
                address3: take("address3"),
                city: take("city"),
                state: take("state"),
                postal_code: take("postalCode"),
                country: take("country"),
                email: take("email"),
                username: take("username"),
                phone: take("phone"),
                ..Identity::default()
            });
            item.kind = 4;
        }
        _ => {
            item.secure_note = Some(json!({ "type": 0 }));
            item.notes = value(Some(extra)).map(z);
            return;
        }
    }
    item.notes = (!notes.is_empty()).then(|| z(&notes));
}

const CARD_KEYS: [(&str, &str); 6] = [
    ("Number", "number"),
    ("Name on Card", "cardholderName"),
    ("Security Code", "code"),
    // "June,2020", read once they're all in.
    ("Expiration Date", "expMonth"),
    // The card's brand, or a field; a start date of "," is none.
    ("Type", "lastPassType"),
    ("Start Date", "startDate"),
];

const ADDRESS_KEYS: [(&str, &str); 15] = [
    ("Title", "title"),
    ("First Name", "firstName"),
    ("Last Name", "lastName"),
    ("Middle Name", "middleName"),
    ("Company", "company"),
    ("Address 1", "address1"),
    ("Address 2", "address2"),
    ("Address 3", "address3"),
    ("City / Town", "city"),
    ("State", "state"),
    ("Zip / Postal Code", "postalCode"),
    ("Country", "country"),
    ("Email Address", "email"),
    ("Username", "username"),
    ("Phone", "phone"),
];

/// A secure note's `Key:value` lines (`parseSecureNoteMapping`): those in
/// `keys` by their property, a blank one, NoteType, and Language left out,
/// Notes and every line after it the item's notes, and any other a field.
fn details(
    item: &mut Item,
    notes: &mut String,
    lines: &[&str],
    keys: &[(&str, &'static str)],
) -> HashMap<&'static str, Zeroizing<String>> {
    let mut found = HashMap::new();
    let mut in_notes = false;
    for line in lines {
        if in_notes {
            notes.push('\n');
            notes.push_str(line);
            continue;
        }
        if blank(line) {
            continue;
        }
        let (key, val) = match line.find(':') {
            Some(at) => (&line[..at], &line[at + 1..]),
            None => (*line, ""),
        };
        if blank(key) || blank(val) || key == "NoteType" || key == "Language" {
            continue;
        }
        let val = if key.to_lowercase().contains("phone") || key.to_lowercase().contains("fax") {
            phone_number(val)
        } else {
            z(val)
        };
        if key == "Notes" {
            if !blank(notes) {
                notes.push('\n');
                notes.push_str(&val);
            } else {
                notes.clear();
                notes.push_str(&val);
            }
            in_notes = true;
        } else if let Some((_, property)) = keys.iter().find(|(k, _)| *k == key) {
            found.insert(*property, val);
        } else {
            key_value(item, notes, key, &val, 0);
        }
    }
    found
}

/// An address's phone as LastPass keeps it, `{"num":"5550100","ext":"12",
/// "cc3l":"USA"}`: the number, with its extension. Anything else as it is.
fn phone_number(text: &str) -> Zeroizing<String> {
    let trimmed = text.trim_matches(crate::csv::js_space);
    let shaped = trimmed.encode_utf16().count() <= 1000
        && trimmed.starts_with('{')
        && trimmed.ends_with('}')
        && trimmed.contains("\"num\"");
    if !shaped {
        return z(text);
    }
    let Ok(phone) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return z(text);
    };
    let Some(number) = phone["num"].as_str() else {
        return z(text);
    };
    let number = number.trim_matches(crate::csv::js_space);
    match phone["ext"]
        .as_str()
        .map(|e| e.trim_matches(crate::csv::js_space))
    {
        Some(ext) if !ext.is_empty() => Zeroizing::new(format!("{number} ext. {ext}")),
        _ => z(number),
    }
}

/// A month's number from its name, as JavaScript's `Date.parse` reads
/// "<name> 1, 2012": its first three letters, any case ("Sept" is 9), or
/// its number; None for anything else.
fn month_number(text: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let text = text.trim_matches(crate::csv::js_space);
    let word: String = text
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect::<String>()
        .to_lowercase();
    if word.len() >= 3 {
        let at = MONTHS.iter().position(|m| word.starts_with(m))?;
        return Some((at + 1).to_string());
    }
    if !word.is_empty() || text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = text.parse().ok()?;
    (1..=12).contains(&n).then(|| n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_by_name_as_javascript_reads_them() {
        for (text, want) in [
            ("June", Some("6")),
            ("june", Some("6")),
            (" Sept ", Some("9")),
            ("Junk", Some("6")),
            ("06", Some("6")),
            ("12", Some("12")),
            ("13", None),
            ("0", None),
            ("Ju", None),
            ("Foo", None),
            ("1x", None),
            ("", None),
        ] {
            assert_eq!(month_number(text).as_deref(), want, "{text:?}");
        }
    }

    #[test]
    fn a_phone_as_lastpass_keeps_it() {
        assert_eq!(
            phone_number(r#"{"num":"5550100","ext":"12","cc3l":"USA"}"#).as_str(),
            "5550100 ext. 12"
        );
        assert_eq!(
            phone_number(r#"{"num":" 5550199 ","ext":"","cc3l":"USA"}"#).as_str(),
            "5550199"
        );
        assert_eq!(phone_number("+1 555 0100").as_str(), "+1 555 0100");
        assert_eq!(phone_number(r#"{"num":5}"#).as_str(), r#"{"num":5}"#);
        let long = format!("{{{}}}", "\"num\"".repeat(400));
        assert_eq!(phone_number(&long).as_str(), long);
    }
}
