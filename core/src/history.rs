//! An item's past versions, as the web vault shows them (passbundle-clients,
//! libs/vault/src/keykeeper/item-versions.ts; passbundle-server,
//! keykeeper_cipher_history.rs). The server keeps each version encrypted as
//! it was stored and sends them to whoever may see the item's history; the
//! core opens each like the item, compares them field by field with the
//! item as it is now, and restores one by saving it as a normal edit
//! (`Vault::encrypt_restore`). Someone who can't see the item's passwords
//! gets when each was saved and by whom, and nothing else.

use serde::Serialize;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::vault::{Item, ItemKind, EXPIRES_FIELD};

/// One field of an item, as the version view compares it.
pub struct Entry {
    pub key: String,
    pub label: String,
    pub value: Zeroizing<String>,
    /// What decides "changed", when the value shown isn't enough (passkeys).
    pub compare: Option<String>,
    /// Hidden until shown, and never to someone who can't see passwords.
    pub secret: bool,
}

/// The kind of item, as the version view says it.
pub fn kind_name(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Login => "Login",
        ItemKind::SecureNote => "Secure note",
        ItemKind::Card => "Card",
        ItemKind::Identity => "Identity",
        ItemKind::SshKey => "SSH key",
        ItemKind::BankAccount => "Bank account",
        ItemKind::DriversLicense => "Driver's license",
        ItemKind::Passport => "Passport",
        ItemKind::Other(_) => "Item",
    }
}

/// Everything someone sees of an item, in the order its page shows it.
pub fn entries(item: &Item) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut add = |key: String, label: &str, value: String, secret: bool| {
        out.push(Entry {
            key,
            label: label.to_owned(),
            value: Zeroizing::new(value),
            compare: None,
            secret,
        });
    };
    let text = |t: &Option<String>| t.clone().unwrap_or_default();
    let secret = |t: &Option<Zeroizing<String>>| {
        t.as_ref()
            .map(|v| v.as_str().to_owned())
            .unwrap_or_default()
    };
    add("type".into(), "Type", kind_name(item.kind).into(), false);
    add("name".into(), "Name", item.name.clone(), false);
    match item.kind {
        ItemKind::Login => {
            add(
                "login.username".into(),
                "Username",
                text(&item.username),
                false,
            );
            add(
                "login.password".into(),
                "Password",
                secret(&item.password),
                true,
            );
            add(
                "login.totp".into(),
                "One-time code secret",
                secret(&item.totp),
                true,
            );
            for (i, uri) in item.uris.iter().enumerate() {
                add(format!("login.uri.{i}"), "Website", uri.clone(), false);
            }
        }
        ItemKind::Card => {
            if let Some(card) = &item.card {
                let expiration = [&card.exp_month, &card.exp_year]
                    .into_iter()
                    .flatten()
                    .filter(|v| !v.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" / ");
                add(
                    "card.cardholderName".into(),
                    "Cardholder name",
                    text(&card.cardholder_name),
                    false,
                );
                add("card.brand".into(), "Brand", text(&card.brand), false);
                add("card.number".into(), "Number", secret(&card.number), true);
                add("card.expiration".into(), "Expiration", expiration, false);
                add(
                    "card.code".into(),
                    "Security code",
                    secret(&card.code),
                    true,
                );
            }
        }
        ItemKind::Identity => {
            if let Some(id) = &item.identity {
                for (key, label, value, hidden) in [
                    ("title", "Title", text(&id.title), false),
                    ("firstName", "First name", text(&id.first_name), false),
                    ("middleName", "Middle name", text(&id.middle_name), false),
                    ("lastName", "Last name", text(&id.last_name), false),
                    ("username", "Username", text(&id.username), false),
                    ("company", "Company", text(&id.company), false),
                    ("ssn", "Social Security number", secret(&id.ssn), true),
                    (
                        "passportNumber",
                        "Passport number",
                        secret(&id.passport_number),
                        true,
                    ),
                    (
                        "licenseNumber",
                        "License number",
                        secret(&id.license_number),
                        true,
                    ),
                    ("email", "Email", text(&id.email), false),
                    ("phone", "Phone", text(&id.phone), false),
                    ("address1", "Address", text(&id.address1), false),
                    ("address2", "Address line 2", text(&id.address2), false),
                    ("address3", "Address line 3", text(&id.address3), false),
                    ("city", "City or town", text(&id.city), false),
                    ("state", "State or province", text(&id.state), false),
                    ("postalCode", "Postal code", text(&id.postal_code), false),
                    ("country", "Country", text(&id.country), false),
                ] {
                    add(format!("identity.{key}"), label, value, hidden);
                }
            }
        }
        ItemKind::SshKey => {
            if let Some(key) = &item.ssh_key {
                add(
                    "sshKey.privateKey".into(),
                    "Private key",
                    secret(&key.private_key),
                    true,
                );
                add(
                    "sshKey.publicKey".into(),
                    "Public key",
                    text(&key.public_key),
                    false,
                );
                add(
                    "sshKey.keyFingerprint".into(),
                    "Fingerprint",
                    text(&key.fingerprint),
                    false,
                );
            }
        }
        _ => {}
    }
    for d in &item.details {
        add(
            d.key.clone(),
            d.label,
            d.value.as_str().to_owned(),
            d.secret,
        );
    }
    let expires = item
        .fields
        .iter()
        .find(|f| f.name == EXPIRES_FIELD)
        .map(|f| f.value.as_str().to_owned())
        .unwrap_or_default();
    add("expires".into(), "Expires", expires, false);
    // Notes show only when revealed in the app, here as on the item's page.
    add(
        "notes".into(),
        "Notes",
        item.notes
            .as_ref()
            .map(|n| n.as_str().to_owned())
            .unwrap_or_default(),
        true,
    );
    // Custom fields by name; a name used twice is told apart by its place.
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (field, (kind, _)) in item.fields.iter().zip(&item.kept.field_kinds) {
        if field.reserved() {
            continue;
        }
        let n = seen.entry(field.name.as_str()).or_default();
        let key = format!("field.{}.{n}", field.name);
        *n += 1;
        let value = match kind {
            2 => {
                if field.value.as_str() == "true" {
                    "Yes".to_owned()
                } else {
                    "No".to_owned()
                }
            }
            3 => "Linked".to_owned(),
            _ => field.value.as_str().to_owned(),
        };
        add(key, &field.name, value, *kind == 1);
    }
    add(
        "reprompt".into(),
        "Asks for the master password",
        if item.kept.reprompt != 0 { "On" } else { "Off" }.into(),
        false,
    );
    // A passkey is compared by its credential, shown by who it's for.
    if let Some(passkey) = &item.passkey {
        let shown = [
            Some(
                passkey
                    .rp_name
                    .clone()
                    .unwrap_or_else(|| passkey.rp_id.clone()),
            ),
            passkey.user_name.clone(),
        ]
        .into_iter()
        .flatten()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
        let at = out
            .iter()
            .position(|e| e.key == "expires")
            .unwrap_or(out.len());
        out.insert(
            at,
            Entry {
                key: "login.passkey.0".into(),
                label: "Passkey".into(),
                value: Zeroizing::new(shown),
                compare: Some(passkey.credential_id.clone()),
                secret: false,
            },
        );
    }
    out
}

/// A row of the side-by-side view: this version's value and the item's now.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub key: String,
    pub label: String,
    /// None for a secret not shown.
    pub version: Option<String>,
    pub current: Option<String>,
    pub secret: bool,
    pub changed: bool,
}

/// A version next to the item as it is now, field by field: every field
/// either has, the ones that differ marked. Fields neither has are left out,
/// and the type and "asks for the master password" only show when they
/// differ. Secrets' values are left out unless `reveal`.
pub fn rows(version: &Item, current: &Item, reveal: bool) -> Vec<Row> {
    let before = entries(version);
    let after = entries(current);
    let find = |list: &[Entry], key: &str| list.iter().position(|e| e.key == key);
    let mut keys: Vec<String> = after.iter().map(|e| e.key.clone()).collect();
    for e in &before {
        if find(&after, &e.key).is_none() {
            keys.push(e.key.clone());
        }
    }
    let mut rows = Vec::new();
    for key in keys {
        let was = find(&before, &key).map(|i| &before[i]);
        let now = find(&after, &key).map(|i| &after[i]);
        let decides = |e: Option<&Entry>| {
            e.map(|e| {
                e.compare
                    .clone()
                    .unwrap_or_else(|| e.value.as_str().to_owned())
            })
            .unwrap_or_default()
        };
        let changed = decides(was) != decides(now);
        let empty = |e: Option<&Entry>| e.is_none_or(|e| e.value.is_empty());
        if empty(was) && empty(now) && !changed {
            continue;
        }
        if (key == "type" || key == "reprompt") && !changed {
            continue;
        }
        let secret = was.is_some_and(|e| e.secret) || now.is_some_and(|e| e.secret);
        let shown = |e: Option<&Entry>| {
            if secret && !reveal {
                None
            } else {
                Some(e.map(|e| e.value.as_str().to_owned()).unwrap_or_default())
            }
        };
        rows.push(Row {
            label: now.or(was).map(|e| e.label.clone()).unwrap_or_default(),
            version: shown(was),
            current: shown(now),
            secret,
            changed,
            key,
        });
    }
    rows
}

/// Whether two versions look the same to someone reading them.
pub fn same(a: &Item, b: &Item) -> bool {
    rows(a, b, true).iter().all(|r| !r.changed)
}

/// Which versions are worth listing, newest first: apps save the whole item
/// every time, so the server keeps a version for a save that changed nothing
/// anyone sees. Of each run of versions that look the same, the oldest
/// stays (its "saved by" names who wrote those contents); versions that look
/// like the item now, newer than any change, are the item itself.
pub fn distinct(newest_first: &[&Item], current: &Item) -> Vec<bool> {
    let mut newest = 0;
    while newest < newest_first.len() && same(newest_first[newest], current) {
        newest += 1;
    }
    (0..newest_first.len())
        .map(|i| {
            i >= newest
                && newest_first
                    .get(i + 1)
                    .is_none_or(|older| !same(newest_first[i], older))
        })
        .collect()
}

/// "Saved by you", "Saved by Ana", "Saved by another member", or nothing when
/// nobody is recorded, from the server's `{id, name, email}`.
pub fn saved_by(editor: &Value, me: &str) -> Option<String> {
    if editor.is_null() {
        return None;
    }
    if editor["id"].as_str() == Some(me) {
        return Some("Saved by you".into());
    }
    let who = editor["name"]
        .as_str()
        .filter(|n| !n.is_empty())
        .or_else(|| editor["email"].as_str().filter(|e| !e.is_empty()));
    Some(match who {
        Some(who) => format!("Saved by {who}"),
        None => "Saved by another member".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn who_saved_a_version_is_said_as_the_web_vault_says_it() {
        assert_eq!(saved_by(&Value::Null, "me"), None);
        assert_eq!(
            saved_by(&json!({ "id": "me", "name": "Ana", "email": null }), "me").as_deref(),
            Some("Saved by you")
        );
        assert_eq!(
            saved_by(
                &json!({ "id": "x", "name": "", "email": "bo@example.test" }),
                "me"
            )
            .as_deref(),
            Some("Saved by bo@example.test")
        );
        assert_eq!(
            saved_by(&json!({ "id": null, "name": null, "email": null }), "me").as_deref(),
            Some("Saved by another member")
        );
    }
}
