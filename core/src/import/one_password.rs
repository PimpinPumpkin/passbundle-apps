//! 1Password's `.1pux` (our fork's `OnePassword1PuxImporter`, with
//! upstream's new item types off, as PassBundle's server sends them): a zip
//! with `export.data` in it, or that file by itself, as the web vault takes
//! either. Each item comes in by its category as the web vault maps them:
//! logins, passwords, databases, routers, servers, and API credentials as
//! logins; cards and bank accounts as cards; identities, driver's licenses,
//! outdoor licenses, memberships, passports, rewards programs, and Social
//! Security numbers as identities; secure notes, software licenses, email
//! accounts, and medical records as notes; SSH keys as SSH keys (in
//! OpenSSH's format, `ssh_key.rs`); and anything else, documents too, as a
//! login, or a note when it has nothing to sign in with. What the item's
//! kind has no place for is a field, or a line of the notes when it's long
//! or on more than one line.
//!
//! - **Where the apps differ from the web vault**, which reads only the
//!   first account and puts items in folders by their first tag: every
//!   account and vault is read, and with more than one vault, each vault is
//!   a folder (with its account's name first when there's more than one
//!   account) and the first tag a folder inside it, as our fork's Proton
//!   Pass import keeps its vaults; a tag that starts with its vault's name
//!   ("Work/Servers" in Work) isn't put inside it twice. With one vault, the
//!   folders are the web vault's. A login with only a one-time password
//!   stays a login, where the web vault makes it a note and loses the code.
//! - **The zip is read in memory, never written anywhere.** It's refused
//!   before anything is unpacked when its entries say they hold more than
//!   1 GB together, or `export.data` more than 100 MB, and `export.data` is
//!   read no further than that whatever it says. Only `export.data` is
//!   unpacked: the files in `files/` (attachments and documents) aren't
//!   imported, and the summary says how many were left out.
//! - **An item that can't be read** (no details, or an SSH key that isn't
//!   one PassBundle reads) is left out and counted, as the web vault skips
//!   it, and the rest come in.

use std::io::{Cursor, Read as _};

use serde_json::{json, Value};
use zeroize::Zeroizing;

use super::{blank, clean, key_value, login, uris, value, z, Format, Item, LeftOut, Parsed};
use crate::csv::js_space;
use crate::error::{Error, Result};
use crate::vault::{Card, Identity, SshKey};

/// The most a `.1pux`'s entries may say they hold, together.
const MOST_UNPACKED: u64 = 1 << 30;
/// The most its `export.data` may hold: as much as the largest file
/// PassBundle imports (`app/transfer.rs`).
const MOST_DATA: u64 = 100 * 1024 * 1024;
/// The most entries a `.1pux` may have.
const MOST_ENTRIES: usize = 100_000;

const NOT_1PUX: &str = "that isn't a 1Password export PassBundle can read. Check that it's the .1pux file 1Password saved, and choose its format";
const DAMAGED: &str =
    "that .1pux is damaged, so PassBundle can't open it. Export it again from 1Password";
const NO_DATA: &str = "that .1pux has no items in it (no export.data), so it isn't one of 1Password's exports. Export it again from 1Password";
const TOO_BIG: &str =
    "that .1pux unpacks to more than PassBundle imports (1 GB), so PassBundle didn't open it";
const DATA_TOO_BIG: &str = "that .1pux holds more items than PassBundle imports (100 MB of them)";

/// A zip's first bytes.
pub(super) fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

/// Reads a `.1pux`, or its `export.data` by itself.
pub(super) fn read(bytes: &[u8]) -> Result<Parsed> {
    let (data, files) = if is_zip(bytes) {
        let (data, files) = unzip(bytes)?;
        (data, Some(files))
    } else {
        (Zeroizing::new(bytes.to_vec()), None)
    };
    let text = Zeroizing::new(String::from_utf8_lossy(&data).into_owned());
    drop(data);
    let doc: Value =
        serde_json::from_str(text.trim_matches(js_space)).map_err(|_| Error::Refused(NOT_1PUX))?;
    let accounts = doc["accounts"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or(Error::Refused(NOT_1PUX))?;
    let mut vaults = Vec::new();
    for account in accounts {
        let attrs = &account["attrs"];
        let name = ["accountName", "name", "email"]
            .iter()
            .find_map(|k| value(attrs[*k].as_str()))
            .map(str::to_owned);
        for vault in account["vaults"].as_array().into_iter().flatten() {
            vaults.push((name.clone(), vault));
        }
    }
    let many_vaults = vaults.len() > 1;
    let many_accounts = accounts.len() > 1;
    let mut parsed = Parsed::new(Format::OnePassword1pux);
    let mut named_files = 0;
    for (account, vault) in vaults {
        let vault_name = value(vault["attrs"]["name"].as_str()).filter(|_| many_vaults);
        let account = account.filter(|_| many_vaults && many_accounts);
        for src in vault["items"].as_array().into_iter().flatten() {
            named_files += files_named(src);
            match item(src) {
                Ok((mut item, tag)) => {
                    if let Some(folder) = folder_of(account.as_deref(), vault_name, tag.as_deref())
                    {
                        item.folder = parsed.folder(&folder);
                    }
                    parsed.items.push(item);
                }
                Err(why) => parsed.leave_out(why),
            }
        }
    }
    parsed.leave_out_some(LeftOut::Attachment, files.unwrap_or(named_files));
    Ok(parsed)
}

/// An item's folder: its first tag, inside its account's and vault's folder
/// when they're given, and not inside its vault twice.
fn folder_of(account: Option<&str>, vault: Option<&str>, tag: Option<&str>) -> Option<String> {
    let mut parts: Vec<&str> = account.into_iter().chain(vault).collect();
    let tag = match (vault, tag) {
        (Some(vault), Some(tag)) => {
            let rest = tag
                .get(..vault.len())
                .filter(|head| head.eq_ignore_ascii_case(vault))
                .map(|_| &tag[vault.len()..])
                .filter(|rest| rest.is_empty() || rest.starts_with(['/', '\\']));
            match rest {
                Some(rest) => Some(rest.trim_start_matches(['/', '\\'])).filter(|r| !r.is_empty()),
                None => Some(tag),
            }
        }
        (_, tag) => tag,
    };
    parts.extend(tag);
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// `export.data` from the zip, and how many files are in its `files/`.
fn unzip(bytes: &[u8]) -> Result<(Zeroizing<Vec<u8>>, usize)> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| Error::Refused(DAMAGED))?;
    if zip.len() > MOST_ENTRIES {
        return Err(Error::Refused(TOO_BIG));
    }
    // What it says it holds, before anything's unpacked.
    let mut said: u64 = 0;
    let mut files = 0;
    let mut data = None;
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i).map_err(|_| Error::Refused(DAMAGED))?;
        said = said.saturating_add(entry.size());
        let name = entry.name_raw();
        if name == b"export.data" {
            data = Some((i, entry.size()));
        } else if name.starts_with(b"files/") && !entry.is_dir() {
            files += 1;
        }
    }
    if said > MOST_UNPACKED {
        return Err(Error::Refused(TOO_BIG));
    }
    let (at, size) = data.ok_or(Error::Refused(NO_DATA))?;
    if size > MOST_DATA {
        return Err(Error::Refused(DATA_TOO_BIG));
    }
    let mut entry = zip.by_index(at).map_err(|_| Error::Refused(DAMAGED))?;
    let mut out = Zeroizing::new(Vec::with_capacity(size as usize + 1));
    (&mut entry)
        .take(MOST_DATA + 1)
        .read_to_end(&mut out)
        .map_err(|_| Error::Refused(DAMAGED))?;
    if out.len() as u64 > MOST_DATA {
        return Err(Error::Refused(DATA_TOO_BIG));
    }
    Ok((out, files))
}

/// The files an item names (a document's, and a file field's), for an
/// `export.data` read by itself, whose files aren't there to count.
fn files_named(src: &Value) -> usize {
    let details = &src["details"];
    let document = usize::from(details["documentAttributes"].is_object());
    let fields = details["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|s| s["fields"].as_array().into_iter().flatten())
        .filter(|f| f["value"]["file"].is_object())
        .count();
    document + fields
}

/// `capitalize`: each word's first letter, if it's one of `\w`'s, in capitals.
fn capitalize(text: &str) -> String {
    let mut out = String::new();
    let mut in_word = false;
    for c in text.trim_matches(js_space).chars() {
        if js_space(c) {
            in_word = false;
            out.push(c);
        } else if !in_word && (c.is_ascii_alphanumeric() || c == '_') {
            in_word = true;
            out.push(c.to_ascii_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// One item, with its first tag (as a folder's name), or why it's left out.
fn item(src: &Value) -> std::result::Result<(Item, Option<String>), LeftOut> {
    let details = &src["details"];
    if !details.is_object() {
        return Err(LeftOut::Unreadable);
    }
    let category = src["categoryUuid"].as_str().unwrap_or("");
    let mut item = login();
    match category {
        "001" | "102" | "005" | "109" | "110" | "112" => {}
        "002" | "101" => {
            item.kind = 3;
            item.card = Some(Card::default());
        }
        "003" | "100" | "111" | "113" => {
            item.kind = 2;
            item.secure_note = Some(json!({ "type": 0 }));
        }
        "004" | "103" | "104" | "105" | "106" | "107" | "108" => {
            item.kind = 4;
            item.identity = Some(Identity::default());
        }
        "114" => {
            item.kind = 5;
            item.ssh_key = Some(SshKey::default());
        }
        _ => {}
    }
    // favIndex is the item's place among the favorites; 0 is none.
    item.favorite = src["favIndex"].as_f64().is_some_and(|i| i > 0.0);
    let mut notes = Zeroizing::new(String::new());
    let mut tag = None;

    let overview = &src["overview"];
    if overview.is_object() {
        item.name = value(overview["title"].as_str()).unwrap_or("").to_owned();
        if let Some(list) = overview["urls"].as_array() {
            item.uris = uris(list.iter().filter_map(|u| u["url"].as_str()));
        }
        if let Some(first) = overview["tags"]
            .as_array()
            .and_then(|t| t.first())
            .and_then(Value::as_str)
        {
            tag = Some(capitalize(first)).filter(|t| !blank(t));
        }
    }

    for field in details["loginFields"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f.is_object())
    {
        let v = field["value"].as_str().unwrap_or("");
        match field["designation"].as_str() {
            Some("username") if !v.is_empty() => {
                item.kind = 1;
                item.username = Some(v.to_owned());
                continue;
            }
            Some("password") if !v.is_empty() => {
                item.kind = 1;
                item.password = Some(z(v));
                continue;
            }
            _ => {}
        }
        let name = field["name"].as_str().unwrap_or("");
        match field["fieldType"].as_str() {
            Some("P") => key_value(&mut item, &mut notes, name, v, 1),
            // A checkbox is yes when it has anything in it.
            Some("C") => {
                let checked = if v.is_empty() { "false" } else { "true" };
                key_value(&mut item, &mut notes, name, checked, 2);
            }
            _ => key_value(&mut item, &mut notes, name, v, 0),
        }
    }

    // The legacy Password item keeps its password in its details.
    if category == "005" {
        item.password = details["password"].as_str().map(z);
    }

    // At most 5, newest first, as many as the item had before any blank
    // ones are left out.
    if let Some(history) = details["passwordHistory"].as_array() {
        let most = history.len().min(5);
        let mut kept: Vec<(&str, f64)> = history
            .iter()
            .filter_map(|h| {
                let password = h["value"].as_str().filter(|p| !blank(p))?;
                Some((password, h["time"].as_f64()?))
            })
            .collect();
        kept.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        item.history = kept
            .into_iter()
            .take(most)
            .map(|(password, time)| {
                // Seconds, or milliseconds for a time of 13 digits or more.
                let ms = if js_number(time).len() >= 13 {
                    time
                } else {
                    time * 1000.0
                };
                (z(password), iso_date(ms))
            })
            .collect();
    }

    for section in details["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s.is_object())
    {
        let title = section["title"].as_str().unwrap_or("");
        for field in section["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|f| f.is_object())
        {
            section_field(category, field, title, &mut item, &mut notes)?;
        }
    }

    // The item's own notes, then what the fields put there.
    if let Some(plain) = details["notesPlain"].as_str().filter(|n| !blank(n)) {
        let plain = Zeroizing::new(plain.replace("\r\n", "\n").replace('\r', "\n"));
        let plain = plain.trim_end_matches(js_space);
        notes = Zeroizing::new(if blank(&notes) {
            plain.to_owned()
        } else {
            format!("{plain}\n\n{}", notes.trim_end_matches(js_space))
        });
    }

    // A login with nothing to sign in with is a note; a one-time password is
    // something.
    if item.kind == 1
        && item.username.as_deref().is_none_or(blank)
        && item.password.as_deref().is_none_or(|p| blank(p))
        && item.totp.as_deref().is_none_or(|t| blank(t))
        && item.uris.is_empty()
    {
        item.kind = 2;
        item.secure_note = Some(json!({ "type": 0 }));
    }
    item.notes = Some(z(&notes));
    clean(&mut item);
    // 1Password says an item is archived, not when.
    if src["state"] == "archived" {
        item.archived_date = Some(crate::vault::now_iso8601());
    }
    Ok((item, tag))
}

/// A field's name: its title, or else its section's.
fn field_name<'a>(title: &'a str, section: &'a str) -> &'a str {
    if !blank(title) {
        title
    } else if !blank(section) {
        section
    } else {
        ""
    }
}

/// A field's value as text (`extractValue`): a date as JavaScript's
/// `toUTCString` writes it, a month and year as its number. None for one
/// that isn't text (an address, an email, an SSH key, a file).
fn text_of(key: &str, raw: &Value) -> Option<Zeroizing<String>> {
    match key {
        "date" => {
            let seconds = raw
                .as_f64()
                .or_else(|| raw.as_str().and_then(|s| s.trim().parse().ok()))
                .unwrap_or(f64::NAN);
            Some(Zeroizing::new(utc_string(seconds * 1000.0)))
        }
        "monthYear" => raw
            .as_f64()
            .map(js_number)
            .or_else(|| raw.as_str().map(str::to_owned))
            .map(Zeroizing::new),
        _ => raw.as_str().map(z),
    }
}

/// One of a section's fields (`parseSectionFields`): into the item's own
/// place for it, or else a field.
fn section_field(
    category: &str,
    field: &Value,
    section: &str,
    item: &mut Item,
    notes: &mut String,
) -> std::result::Result<(), LeftOut> {
    // Its value is an object of one kind of value: a string, a concealed
    // one, a date, an address, and so on.
    let Some((key, raw)) = field["value"].as_object().and_then(|v| v.iter().next()) else {
        return Ok(());
    };
    if raw.is_null() || raw == "" {
        return Ok(());
    }
    let key = key.as_str();
    let title = field["title"].as_str().unwrap_or("");
    let id = field["id"].as_str().unwrap_or("");
    let name = field_name(title, section);
    let text = text_of(key, raw);
    let text = text.as_deref().map(String::as_str);
    match item.kind {
        1 => {
            if let Some(t) = text {
                if fill_login(item, title, id, t)
                    || (category == "110" && fill_server(item, id, t))
                    || (category == "112" && fill_api(item, title, t))
                {
                    return Ok(());
                }
            }
        }
        3 => {
            if let Some(t) = text {
                if fill_card(item, id, t) || (category == "101" && fill_bank(item, id, t)) {
                    return Ok(());
                }
            }
        }
        4 => {
            if fill_identity(item, notes, id, key, raw, text) {
                return Ok(());
            }
            if key == "address" {
                let part = |k: &str| value(raw[k].as_str()).map(str::to_owned);
                let identity = item.identity.get_or_insert_with(Identity::default);
                identity.address1 = part("street");
                identity.city = part("city");
                if let Some(country) = part("country") {
                    identity.country = Some(country.to_uppercase());
                }
                identity.postal_code = part("zip");
                identity.state = part("state");
                return Ok(());
            }
            if let Some(t) = text {
                if fill_kind(item, category, id, t) {
                    return Ok(());
                }
            }
        }
        5 if key == "sshKey" => {
            // The metadata's private key: the other one isn't always in the
            // same format. 1Password keeps no key under a password.
            let pem = raw["metadata"]["privateKey"].as_str().unwrap_or("");
            let key = super::ssh_key::convert(pem).ok_or(LeftOut::SshKeyUnreadable)?;
            item.ssh_key = Some(SshKey {
                private_key: Some(key.private_key),
                public_key: Some(key.public_key),
                fingerprint: Some(key.fingerprint),
            });
            return Ok(());
        }
        _ => {}
    }
    if key == "email" {
        let address = raw["email_address"].as_str().unwrap_or("");
        key_value(item, notes, name, address, 0);
        key_value(
            item,
            notes,
            "provider",
            raw["provider"].as_str().unwrap_or(""),
            0,
        );
        return Ok(());
    }
    // Not a password field that's in the history already.
    if title == "password"
        && text.is_some_and(|t| item.history.iter().any(|(p, _)| p.as_str() == t))
    {
        return Ok(());
    }
    // A guarded field asks for the master password for the whole item.
    if field["guarded"] == true && item.reprompt == 0 {
        item.reprompt = 1;
    }
    if let Some(t) = text {
        key_value(item, notes, name, t, if key == "concealed" { 1 } else { 0 });
    }
    Ok(())
}

fn fill_login(item: &mut Item, title: &str, id: &str, t: &str) -> bool {
    if item.username.as_deref().is_none_or(blank) && title == "username" {
        item.username = Some(t.to_owned());
    } else if item.password.as_deref().is_none_or(|p| blank(p)) && title == "password" {
        item.password = Some(z(t));
    } else if item.totp.as_deref().is_none_or(|p| blank(p)) && id.starts_with("TOTP_") {
        item.totp = Some(z(t));
    } else {
        return false;
    }
    true
}

fn fill_server(item: &mut Item, id: &str, t: &str) -> bool {
    if item.uris.is_empty() && id == "url" {
        item.uris = uris([t]);
        return true;
    }
    false
}

fn fill_api(item: &mut Item, title: &str, t: &str) -> bool {
    if item.password.as_deref().is_none_or(|p| blank(p)) && title == "credential" {
        item.password = Some(z(t));
    } else if item.uris.is_empty() && title == "hostname" {
        item.uris = uris([t]);
    } else {
        return false;
    }
    true
}

fn empty(text: Option<&str>) -> bool {
    text.is_none_or(blank)
}

fn fill_card(item: &mut Item, id: &str, t: &str) -> bool {
    let card = item.card.get_or_insert_with(Card::default);
    match id {
        "ccnum" if empty(card.number.as_deref().map(String::as_str)) => {
            card.number = Some(z(t));
            card.brand = super::card_brand(Some(t));
        }
        "cvv" if empty(card.code.as_deref().map(String::as_str)) => card.code = Some(z(t)),
        "cardholder" if empty(card.cardholder_name.as_deref()) => {
            card.cardholder_name = Some(t.to_owned());
        }
        // "203012": the year, then the month.
        "expiry" if empty(card.exp_month.as_deref()) && empty(card.exp_year.as_deref()) => {
            let digits: Vec<char> = t.trim_matches(js_space).chars().collect();
            let part = |from: usize, to: usize| -> String {
                digits[from.min(digits.len())..to.min(digits.len())]
                    .iter()
                    .collect()
            };
            let month = part(4, 6);
            card.exp_month = Some(match month.strip_prefix('0') {
                Some(_) => month.chars().nth(1).map(String::from).unwrap_or_default(),
                None => month,
            });
            card.exp_year = Some(part(0, 4));
        }
        // The brand comes from the number.
        "type" => {}
        _ => return false,
    }
    true
}

fn fill_bank(item: &mut Item, id: &str, t: &str) -> bool {
    let card = item.card.get_or_insert_with(Card::default);
    if empty(card.cardholder_name.as_deref()) && id == "owner" {
        card.cardholder_name = Some(t.to_owned());
        return true;
    }
    false
}

fn fill_identity(
    item: &mut Item,
    notes: &mut String,
    id: &str,
    key: &str,
    raw: &Value,
    text: Option<&str>,
) -> bool {
    let identity = item.identity.get_or_insert_with(Identity::default);
    let slot = match id {
        "firstname" => Some(&mut identity.first_name),
        "lastname" => Some(&mut identity.last_name),
        "initial" => Some(&mut identity.middle_name),
        "defphone" => Some(&mut identity.phone),
        "company" => Some(&mut identity.company),
        _ => None,
    };
    if let (Some(slot), Some(t)) = (slot, text) {
        if empty(slot.as_deref()) {
            *slot = Some(t.to_owned());
            return true;
        }
    }
    if empty(identity.email.as_deref()) {
        if key == "email" {
            identity.email = value(raw["email_address"].as_str()).map(str::to_owned);
            let provider = raw["provider"].as_str().unwrap_or("");
            key_value(item, notes, "provider", provider, 0);
            return true;
        }
        if let ("email", Some(t)) = (id, text) {
            identity.email = Some(t.to_owned());
            return true;
        }
    }
    if let ("username", Some(t)) = (id, text) {
        if empty(identity.username.as_deref()) {
            identity.username = Some(t.to_owned());
            return true;
        }
    }
    false
}

/// A name as upstream splits one (`processFullName`): the first word, then
/// the middle one and the rest when there are three or more.
fn full_name(identity: &mut Identity, full: &str) {
    if blank(full) {
        return;
    }
    let parts: Vec<&str> = full.split(' ').collect();
    let first = value(parts.first().copied());
    let (middle, last) = match parts.len() {
        2 => (None, value(Some(parts[1])).map(str::to_owned)),
        n if n >= 3 => (value(Some(parts[1])), Some(parts[2..].join(" "))),
        _ => (None, None),
    };
    if let Some(first) = first {
        identity.first_name = Some(first.to_owned());
    }
    if let Some(middle) = middle {
        identity.middle_name = Some(middle.to_owned());
    }
    if let Some(last) = last.filter(|l| !blank(l)) {
        identity.last_name = Some(last);
    }
}

/// The identity kinds' own fields, each by its id, into the identity when
/// its place there is empty.
fn fill_kind(item: &mut Item, category: &str, id: &str, t: &str) -> bool {
    let identity = item.identity.get_or_insert_with(Identity::default);
    let name_id = match category {
        "103" | "106" => "fullname",
        "104" | "108" => "name",
        "105" | "107" => "member_name",
        _ => return false,
    };
    if id == name_id && empty(identity.first_name.as_deref()) {
        full_name(identity, t);
        return true;
    }
    let plain = |slot: &mut Option<String>| {
        if empty(slot.as_deref()) {
            *slot = Some(t.to_owned());
            true
        } else {
            false
        }
    };
    let secret = |slot: &mut Option<Zeroizing<String>>| {
        if empty(slot.as_deref().map(String::as_str)) {
            *slot = Some(z(t));
            true
        } else {
            false
        }
    };
    match (category, id) {
        // A driver's license.
        ("103", "address") => plain(&mut identity.address1),
        ("103" | "104", "country") => plain(&mut identity.country),
        ("103" | "104", "state") => plain(&mut identity.state),
        ("103", "number") => secret(&mut identity.license_number),
        // A membership, and a rewards program.
        ("105", "org_name") | ("107", "company_name") => plain(&mut identity.company),
        ("105", "phone") => plain(&mut identity.phone),
        // A passport.
        ("106", "issuing_country") => plain(&mut identity.country),
        ("106", "number") => secret(&mut identity.passport_number),
        // A Social Security number.
        ("108", "number") => secret(&mut identity.ssn),
        _ => false,
    }
}

/// A number as JavaScript's `String(n)` writes it, near enough: a whole
/// one without a decimal point.
fn js_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e21 {
        format!("{}", n as i128)
    } else {
        n.to_string()
    }
}

/// The proleptic Gregorian date of a day since 1970 (Howard Hinnant's
/// days-to-civil), before 1970 too.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

/// A time in milliseconds, as JavaScript's `Date` keeps one: whole
/// milliseconds, and None past its range (±100,000,000 days).
fn js_time(ms: f64) -> Option<(i64, i64)> {
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return None;
    }
    let ms = ms.trunc() as i64;
    Some((ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000)))
}

/// JavaScript's `toUTCString`: "Thu, 01 Jan 1981 12:01:00 GMT".
fn utc_string(ms: f64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let Some((days, rest)) = js_time(ms) else {
        return "Invalid Date".to_owned();
    };
    let (year, month, day) = civil(days);
    let year = if year < 0 {
        format!("-{:04}", -year)
    } else {
        format!("{year:04}")
    };
    let secs = rest / 1000;
    format!(
        "{}, {day:02} {} {year} {:02}:{:02}:{:02} GMT",
        DAYS[(days + 4).rem_euclid(7) as usize],
        MONTHS[month as usize - 1],
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// JavaScript's `toISOString`, for the years the server takes (0 to 9999).
fn iso_date(ms: f64) -> Option<String> {
    let (days, rest) = js_time(ms)?;
    let (year, month, day) = civil(days);
    if !(0..=9999).contains(&year) {
        return None;
    }
    let secs = rest / 1000;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60,
        rest % 1000
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_as_javascript_writes_them() {
        assert_eq!(
            utc_string(347_198_460_000.0),
            "Thu, 01 Jan 1981 12:01:00 GMT"
        );
        assert_eq!(utc_string(-1e12), "Sun, 24 Apr 1938 22:13:20 GMT");
        assert_eq!(
            utc_string(-62_198_755_200_000.0),
            "Fri, 01 Jan -0001 00:00:00 GMT"
        );
        assert_eq!(
            utc_string(253_402_300_800_000.0),
            "Sat, 01 Jan 10000 00:00:00 GMT"
        );
        assert_eq!(utc_string(500.0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(utc_string(f64::NAN), "Invalid Date");
        assert_eq!(utc_string(9e15), "Invalid Date");
        assert_eq!(
            iso_date(1_458_322_355_700.0).as_deref(),
            Some("2016-03-18T17:32:35.700Z")
        );
        assert_eq!(iso_date(-62_198_755_200_000.0), None);
        assert_eq!(js_number(203_012.0), "203012");
        assert_eq!(js_number(2.5), "2.5");
        assert_eq!(js_number(1_458_322_355.5).len(), 12);
    }

    #[test]
    fn folders_by_vault_then_tag() {
        assert_eq!(
            folder_of(None, None, Some("Finance")).as_deref(),
            Some("Finance")
        );
        assert_eq!(folder_of(None, None, None), None);
        assert_eq!(
            folder_of(None, Some("Family"), Some("Finance")).as_deref(),
            Some("Family/Finance")
        );
        assert_eq!(
            folder_of(None, Some("Family"), None).as_deref(),
            Some("Family")
        );
        // A tag named after its vault isn't put in it twice.
        assert_eq!(
            folder_of(None, Some("Work"), Some("Work")).as_deref(),
            Some("Work")
        );
        assert_eq!(
            folder_of(None, Some("Work"), Some("work/Servers")).as_deref(),
            Some("Work/Servers")
        );
        assert_eq!(
            folder_of(None, Some("Work"), Some("Workshop")).as_deref(),
            Some("Work/Workshop")
        );
        assert_eq!(
            folder_of(Some("Acme"), Some("Shared"), Some("Servers")).as_deref(),
            Some("Acme/Shared/Servers")
        );
    }

    #[test]
    fn a_tag_capitalized_as_upstream_does() {
        assert_eq!(capitalize("  starter kit "), "Starter Kit");
        assert_eq!(capitalize("work/servers"), "Work/servers");
        // `\w` is ASCII's: a word starts at its first letter or digit.
        assert_eq!(capitalize("éclair 2fa _x"), "éClair 2fa _x");
        assert_eq!(capitalize("a-b c -x"), "A-b C -X");
    }
}
