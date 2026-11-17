//! Exporting the person's own items (meta repo DESIGN.md, "Export and
//! import in the apps"): the web vault's three files, laid out as it lays
//! them out (passbundle-clients, `libs/tools/export-vault-core`,
//! `individual-vault-export.service.ts` and `base-vault-export.service.ts`;
//! `libs/common/src/models/export`), so a file from either one imports in the
//! other.
//!
//! - **`.json`**: `{"encrypted": false, "folders", "items"}`, two-space
//!   indents. Every item of the person's own that isn't in the trash, the
//!   archived ones with their `archivedDate`, without the item's own key or
//!   its attachments. An organization's items are never in it (upstream);
//!   an organization's export is the web vault's.
//! - **`.csv`**: logins and notes only, in upstream's columns.
//! - **Password-protected `.json`**: the `.json` above, encrypted under a
//!   key made from a password the person chooses with this account's own
//!   KDF settings, as the web vault makes it, byte for byte
//!   (`protect`, `open_protected`; vectors in docs/vectors/vault-export-v1.json).
//!
//! An item that doesn't open (`Vault::damaged`) isn't in any of them, and
//! the count says so. What's exported never passes through the app's Dart:
//! the core writes the file itself (`write`).

use serde::ser::{SerializeMap, Serializer};
use serde::Serialize;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::crypto::{EncString, SymmetricKey};
use crate::error::{Error, Result};
use crate::kdf::{Kdf, MasterKey};
use crate::vault::{
    Item, ItemKind, Vault, BANK_ACCOUNT_DETAILS, DRIVERS_LICENSE_DETAILS, PASSPORT_DETAILS,
};

/// The web vault's three export files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Json,
    Csv,
    PasswordProtected,
}

impl Format {
    /// By the web vault's names for them: "json", "csv", "encrypted_json".
    pub fn named(name: &str) -> Option<Format> {
        match name {
            "json" => Some(Format::Json),
            "csv" => Some(Format::Csv),
            "encrypted_json" => Some(Format::PasswordProtected),
            _ => None,
        }
    }
}

/// An export, made: its text, how many items are in it, and how many of the
/// person's own items aren't (ones that don't open, and for `.csv` the kinds
/// it doesn't hold).
pub struct Made {
    pub text: Zeroizing<String>,
    pub items: usize,
    pub left_out: usize,
}

/// A secret, borrowed or decrypted for the export only, serialized as text.
struct S<'a>(&'a str);

impl Serialize for S<'_> {
    fn serialize<Z: Serializer>(&self, s: Z) -> std::result::Result<Z::Ok, Z::Error> {
        s.serialize_str(self.0)
    }
}

/// An object whose keys come in the order given (serde_json's own map
/// sorts them).
struct Ordered<'a>(Vec<(&'a str, S<'a>)>);

impl Serialize for Ordered<'_> {
    fn serialize<Z: Serializer>(&self, s: Z) -> std::result::Result<Z::Ok, Z::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

#[derive(Serialize)]
struct Doc<'a> {
    encrypted: bool,
    folders: Vec<FolderOut<'a>>,
    items: Vec<ItemOut<'a>>,
}

#[derive(Serialize)]
struct FolderOut<'a> {
    id: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryOut<'a> {
    last_used_date: Option<String>,
    password: S<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FieldOut<'a> {
    name: Option<&'a str>,
    value: Option<S<'a>>,
    #[serde(rename = "type")]
    kind: u8,
    linked_id: Option<&'a Value>,
}

#[derive(Serialize)]
struct UriOut<'a> {
    #[serde(rename = "match")]
    match_type: Option<u8>,
    uri: &'a str,
}

/// A passkey as upstream's export has it (`Fido2CredentialExport`): its
/// values decrypted, the counter and discoverable as text.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PasskeyOut<'a> {
    credential_id: S<'a>,
    key_type: S<'a>,
    key_algorithm: S<'a>,
    key_curve: S<'a>,
    key_value: S<'a>,
    rp_id: S<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_handle: Option<S<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_name: Option<S<'a>>,
    counter: S<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rp_name: Option<S<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_display_name: Option<S<'a>>,
    discoverable: S<'a>,
    creation_date: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginOut<'a> {
    fido2_credentials: Vec<PasskeyOut<'a>>,
    uris: Vec<UriOut<'a>>,
    username: Option<&'a str>,
    password: Option<S<'a>>,
    totp: Option<S<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CardOut<'a> {
    cardholder_name: Option<&'a str>,
    brand: Option<&'a str>,
    number: Option<S<'a>>,
    exp_month: Option<&'a str>,
    exp_year: Option<&'a str>,
    code: Option<S<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SshKeyOut<'a> {
    private_key: S<'a>,
    public_key: &'a str,
    key_fingerprint: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ItemOut<'a> {
    password_history: Option<Vec<HistoryOut<'a>>>,
    revision_date: Option<String>,
    creation_date: Option<String>,
    deleted_date: Option<String>,
    archived_date: Option<String>,
    id: &'a str,
    organization_id: Option<&'a str>,
    folder_id: Option<&'a str>,
    #[serde(rename = "type")]
    kind: u8,
    reprompt: u8,
    name: &'a str,
    notes: Option<S<'a>>,
    favorite: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<Vec<FieldOut<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    login: Option<LoginOut<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    secure_note: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    card: Option<CardOut<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identity: Option<Ordered<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ssh_key: Option<SshKeyOut<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bank_account: Option<Ordered<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    drivers_license: Option<Ordered<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    passport: Option<Ordered<'a>>,
    collection_ids: Option<Vec<String>>,
}

/// A date as upstream's exports write it, which is JavaScript's
/// `toISOString`: "2026-11-01T10:00:00.123Z", to the millisecond. The
/// server's dates (to the microsecond) are cut to the millisecond, as
/// JavaScript reads them; anything else is left as it is.
pub(crate) fn js_date(date: &str) -> String {
    let date = date.trim();
    let b = date.as_bytes();
    let digits =
        |r: std::ops::Range<usize>| b.get(r).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    let shaped = b.len() >= 19
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && matches!(b[10], b'T' | b' ')
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19);
    if !shaped {
        return date.to_owned();
    }
    let rest = &date[19..];
    let (fraction, zone) = match rest.strip_prefix('.') {
        Some(after) => {
            let end = after
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(after.len());
            (&after[..end], &after[end..])
        }
        None => ("", rest),
    };
    if !matches!(zone, "" | "Z" | "z" | "+00:00" | "+0000") {
        return date.to_owned();
    }
    let mut millis: String = fraction.chars().take(3).collect();
    while millis.len() < 3 {
        millis.push('0');
    }
    format!("{}T{}.{millis}Z", &date[..10], &date[11..19])
}

/// Upstream's type number for an item's kind; None for one it doesn't
/// know, which isn't exported.
fn type_number(kind: ItemKind) -> Option<u8> {
    Some(match kind {
        ItemKind::Login => 1,
        ItemKind::SecureNote => 2,
        ItemKind::Card => 3,
        ItemKind::Identity => 4,
        ItemKind::SshKey => 5,
        ItemKind::BankAccount => 6,
        ItemKind::DriversLicense => 7,
        ItemKind::Passport => 8,
        ItemKind::Other(_) => return None,
    })
}

/// What's decrypted for one item's export beyond what the vault holds open:
/// its earlier passwords and its passkeys, wiped when the export is made.
struct Opened {
    history: Vec<(Option<String>, Zeroizing<String>)>,
    passkeys: Vec<crate::import::Passkey>,
}

/// A passkey's values, in upstream's export's order, and whether each is
/// one it always writes ("" when there's none).
const PASSKEY_VALUES: [(&str, bool); 12] = [
    ("credentialId", true),
    ("keyType", true),
    ("keyAlgorithm", true),
    ("keyCurve", true),
    ("keyValue", true),
    ("rpId", true),
    ("userHandle", false),
    ("userName", false),
    ("counter", true),
    ("rpName", false),
    ("userDisplayName", false),
    ("discoverable", true),
];

fn open_extra(vault: &Vault, item: &Item) -> Result<Opened> {
    let key = vault.item_value_key(item)?;
    let open = |raw: &Value| -> Result<Option<Zeroizing<String>>> {
        match raw.as_str() {
            None => Ok(None),
            Some(text) => Ok(Some(EncString::parse(text)?.decrypt_string(key)?)),
        }
    };
    let mut history = Vec::new();
    for entry in &item.kept.password_history {
        if let Some(password) = open(&entry["password"])? {
            let when = entry["lastUsedDate"].as_str().map(js_date);
            history.push((when, password));
        }
    }
    let mut passkeys = Vec::new();
    if item.kind == ItemKind::Login {
        for entry in item
            .kept
            .fido2_credentials
            .as_ref()
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mut values = Vec::new();
            for (name, _) in PASSKEY_VALUES {
                if let Some(value) = open(&entry[name])? {
                    values.push((name, value));
                }
            }
            passkeys.push(crate::import::Passkey {
                values,
                creation_date: entry["creationDate"].as_str().map(js_date),
            });
        }
    }
    Ok(Opened { history, passkeys })
}

fn text(t: &Option<String>) -> Option<&str> {
    t.as_deref()
}

fn secret(t: &Option<Zeroizing<String>>) -> Option<S<'_>> {
    t.as_ref().map(|v| S(v.as_str()))
}

/// The person's own items that go in an export: not an organization's, not
/// in the trash.
fn own(vault: &Vault) -> impl Iterator<Item = &Item> {
    vault
        .items
        .iter()
        .filter(|i| i.organization_id.is_none() && !i.deleted)
}

/// How many of the person's own items don't open, out of the trash: never
/// in an export.
fn damaged(vault: &Vault) -> usize {
    vault.damaged.len()
}

/// The person's folders, by name, as the web vault lists them.
fn folders(vault: &Vault) -> Vec<(&String, &String)> {
    let mut folders: Vec<(&String, &String)> = vault.folders.iter().collect();
    folders.sort_by(|a, b| {
        a.1.to_lowercase()
            .cmp(&b.1.to_lowercase())
            .then(a.0.cmp(b.0))
    });
    folders
}

/// The `.json` export.
pub fn json(vault: &Vault) -> Result<Made> {
    let items: Vec<&Item> = own(vault)
        .filter(|i| type_number(i.kind).is_some())
        .collect();
    let skipped = own(vault).count() - items.len();
    let opened: Vec<Opened> = items
        .iter()
        .map(|i| open_extra(vault, i))
        .collect::<Result<_>>()?;
    let folders = folders(vault);
    let doc = Doc {
        encrypted: false,
        folders: folders
            .iter()
            .map(|(id, name)| FolderOut { id, name })
            .collect(),
        items: items
            .iter()
            .zip(&opened)
            .map(|(item, extra)| item_out(vault, item, extra))
            .collect(),
    };
    Ok(Made {
        text: pretty(&doc)?,
        items: items.len(),
        left_out: skipped + damaged(vault),
    })
}

/// JSON with two-space indents, as `JSON.stringify(doc, null, "  ")`.
fn pretty<T: Serialize>(value: &T) -> Result<Zeroizing<String>> {
    let mut out = Zeroizing::new(Vec::with_capacity(64 * 1024));
    let mut writer = serde_json::Serializer::with_formatter(
        &mut *out,
        serde_json::ser::PrettyFormatter::with_indent(b"  "),
    );
    value
        .serialize(&mut writer)
        .map_err(|_| Error::Damaged("the export couldn't be written"))?;
    let text = String::from_utf8(std::mem::take(&mut *out))
        .map_err(|_| Error::Damaged("the export couldn't be written"))?;
    Ok(Zeroizing::new(text))
}

fn item_out<'a>(vault: &'a Vault, item: &'a Item, extra: &'a Opened) -> ItemOut<'a> {
    let kind = type_number(item.kind).unwrap_or(1);
    let fields: Vec<FieldOut<'a>> = item
        .fields
        .iter()
        .zip(&item.kept.field_kinds)
        .map(|(f, (kind, linked))| FieldOut {
            name: Some(f.name.as_str()).filter(|n| !n.is_empty()),
            value: (*kind != 3).then(|| S(f.value.as_str())),
            kind: *kind,
            linked_id: linked.as_ref(),
        })
        .collect();
    let history: Vec<HistoryOut<'a>> = extra
        .history
        .iter()
        .map(|(when, password)| HistoryOut {
            last_used_date: when.clone(),
            password: S(password.as_str()),
        })
        .collect();
    let details =
        |part: &str, specs: &[(&'static str, &'static str, bool)]| -> Option<Ordered<'a>> {
            Some(Ordered(
                specs
                    .iter()
                    .filter_map(|(property, _, _)| {
                        item.details
                            .iter()
                            .find(|d| d.key == format!("{part}.{property}"))
                            .map(|d| (*property, S(d.value.as_str())))
                    })
                    .collect(),
            ))
        };
    let mut out = ItemOut {
        password_history: (!history.is_empty()).then_some(history),
        revision_date: item.revision_date.as_deref().map(js_date),
        creation_date: item.creation_date.as_deref().map(js_date),
        deleted_date: None,
        archived_date: item.kept.archived_date.as_deref().map(js_date),
        id: &item.id,
        organization_id: None,
        folder_id: item
            .folder_id
            .as_deref()
            .filter(|f| vault.folders.contains_key(*f)),
        kind,
        reprompt: item.kept.reprompt,
        name: &item.name,
        notes: secret(&item.notes),
        favorite: item.favorite,
        fields: (!fields.is_empty()).then_some(fields),
        login: None,
        secure_note: None,
        card: None,
        identity: None,
        ssh_key: None,
        bank_account: None,
        drivers_license: None,
        passport: None,
        collection_ids: None,
    };
    match item.kind {
        ItemKind::Login => {
            out.login = Some(LoginOut {
                fido2_credentials: extra
                    .passkeys
                    .iter()
                    .map(|p| passkey_out(&p.values, &p.creation_date))
                    .collect(),
                uris: item
                    .uris
                    .iter()
                    .enumerate()
                    .map(|(at, uri)| UriOut {
                        match_type: item.uri_matches.get(at).copied().flatten(),
                        uri,
                    })
                    .collect(),
                username: text(&item.username),
                password: secret(&item.password),
                totp: secret(&item.totp),
            });
        }
        ItemKind::SecureNote => {
            out.secure_note = Some(
                item.kept
                    .secure_note
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({ "type": 0 })),
            );
        }
        ItemKind::Card => {
            if let Some(card) = &item.card {
                out.card = Some(CardOut {
                    cardholder_name: text(&card.cardholder_name),
                    brand: text(&card.brand),
                    number: secret(&card.number),
                    exp_month: text(&card.exp_month),
                    exp_year: text(&card.exp_year),
                    code: secret(&card.code),
                });
            }
        }
        ItemKind::Identity => {
            if let Some(id) = &item.identity {
                let plain = |t: &'a Option<String>| t.as_deref().map(S);
                let values: [(&str, Option<S<'a>>); 18] = [
                    ("title", plain(&id.title)),
                    ("firstName", plain(&id.first_name)),
                    ("middleName", plain(&id.middle_name)),
                    ("lastName", plain(&id.last_name)),
                    ("address1", plain(&id.address1)),
                    ("address2", plain(&id.address2)),
                    ("address3", plain(&id.address3)),
                    ("city", plain(&id.city)),
                    ("state", plain(&id.state)),
                    ("postalCode", plain(&id.postal_code)),
                    ("country", plain(&id.country)),
                    ("company", plain(&id.company)),
                    ("email", plain(&id.email)),
                    ("phone", plain(&id.phone)),
                    ("ssn", secret(&id.ssn)),
                    ("username", plain(&id.username)),
                    ("passportNumber", secret(&id.passport_number)),
                    ("licenseNumber", secret(&id.license_number)),
                ];
                out.identity = Some(Ordered(
                    values
                        .into_iter()
                        .filter_map(|(k, v)| v.map(|v| (k, v)))
                        .collect(),
                ));
            }
        }
        ItemKind::SshKey => {
            if let Some(key) = &item.ssh_key {
                out.ssh_key = Some(SshKeyOut {
                    private_key: S(key.private_key.as_deref().map_or("", String::as_str)),
                    public_key: key.public_key.as_deref().unwrap_or(""),
                    key_fingerprint: key.fingerprint.as_deref().unwrap_or(""),
                });
            }
        }
        ItemKind::BankAccount => out.bank_account = details("bankAccount", &BANK_ACCOUNT_DETAILS),
        ItemKind::DriversLicense => {
            out.drivers_license = details("driversLicense", &DRIVERS_LICENSE_DETAILS)
        }
        ItemKind::Passport => out.passport = details("passport", &PASSPORT_DETAILS),
        ItemKind::Other(_) => {}
    }
    out
}

fn passkey_out<'a>(
    values: &'a [(&'static str, Zeroizing<String>)],
    created: &'a Option<String>,
) -> PasskeyOut<'a> {
    let get = |name: &str| {
        values
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| S(v.as_str()))
    };
    let always = |name: &str, default: &'static str| get(name).unwrap_or(S(default));
    PasskeyOut {
        credential_id: always("credentialId", ""),
        key_type: always("keyType", ""),
        key_algorithm: always("keyAlgorithm", ""),
        key_curve: always("keyCurve", ""),
        key_value: always("keyValue", ""),
        rp_id: always("rpId", ""),
        user_handle: get("userHandle"),
        user_name: get("userName"),
        counter: always("counter", "0"),
        rp_name: get("rpName"),
        user_display_name: get("userDisplayName"),
        discoverable: always("discoverable", "false"),
        creation_date: created.clone(),
    }
}

/// The `.csv` export's columns, in upstream's order (`buildCsvExport`,
/// `buildCommonCipher`).
pub const CSV_COLUMNS: [&str; 12] = [
    "folder",
    "favorite",
    "type",
    "name",
    "notes",
    "fields",
    "reprompt",
    "archivedDate",
    "login_uri",
    "login_username",
    "login_password",
    "login_totp",
];

/// The `.csv` export: the person's own logins and notes. A field is its
/// name, ": ", and its value, one to a line; a login's websites are one
/// value, separated by commas.
pub fn csv(vault: &Vault) -> Result<Made> {
    let mut rows = Vec::new();
    let mut left_out = damaged(vault);
    for item in own(vault) {
        let kind = match item.kind {
            ItemKind::Login => "login",
            ItemKind::SecureNote => "note",
            _ => {
                left_out += 1;
                continue;
            }
        };
        let z = |t: &str| Zeroizing::new(t.to_owned());
        let fields = item
            .fields
            .iter()
            .zip(&item.kept.field_kinds)
            .map(|(f, (k, _))| {
                // A linked field has no value: upstream writes "null".
                let value = if *k == 3 { "null" } else { f.value.as_str() };
                format!("{}: {value}", f.name)
            })
            .collect::<Vec<_>>();
        let fields = Zeroizing::new(fields.join("\n"));
        let login = item.kind == ItemKind::Login;
        let opt = |t: Option<&str>| z(if login { t.unwrap_or("") } else { "" });
        rows.push(vec![
            z(item
                .folder_id
                .as_ref()
                .and_then(|f| vault.folders.get(f))
                .map_or("", String::as_str)),
            z(if item.favorite { "1" } else { "" }),
            z(kind),
            z(&item.name),
            z(item.notes.as_deref().map_or("", String::as_str)),
            fields,
            z(&item.kept.reprompt.to_string()),
            z(&item
                .kept
                .archived_date
                .as_deref()
                .map(js_date)
                .unwrap_or_default()),
            z(&if login {
                item.uris.join(",")
            } else {
                String::new()
            }),
            opt(item.username.as_deref()),
            opt(item.password.as_deref().map(String::as_str)),
            opt(item.totp.as_deref().map(String::as_str)),
        ]);
    }
    Ok(Made {
        items: rows.len(),
        text: crate::csv::write(&CSV_COLUMNS, &rows),
        left_out,
    })
}

/// A password-protected file's KDF settings must be within these to be
/// opened (upstream's `password-protected-kdf-config.ts`): a file says what
/// it was made with, and Argon2id takes its memory before anything else,
/// so a file can't make the device hang. PBKDF2's floor is lower than the
/// one for signing in, as older exports were made with fewer iterations.
pub const PBKDF2_ITERATIONS: std::ops::RangeInclusive<u64> = 5_000..=2_000_000;
pub const ARGON2_ITERATIONS: std::ops::RangeInclusive<u64> = 2..=10;
pub const ARGON2_MEMORY_MIB: std::ops::RangeInclusive<u64> = 16..=1024;
pub const ARGON2_PARALLELISM: std::ops::RangeInclusive<u64> = 1..=16;

/// The password-protected file, as `JSON.stringify` lays out the web
/// vault's (`buildPasswordExport`): Argon2id's memory and parallelism come
/// last, as the web vault adds them after the rest.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Protected<'a> {
    encrypted: bool,
    password_protected: bool,
    salt: &'a str,
    kdf_type: u8,
    kdf_iterations: u32,
    #[serde(rename = "encKeyValidation_DO_NOT_EDIT")]
    validation: String,
    data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    kdf_memory: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kdf_parallelism: Option<u32>,
}

/// The key a password-protected file is under: the KDF over the password,
/// salted with the salt's text as it's written in the file (its base64,
/// not the bytes it encodes), then stretched with HKDF-Expand "enc" and
/// "mac", as the master key is (upstream's `deriveVaultExportKey`).
pub fn file_key(password: &str, salt: &str, kdf: &Kdf) -> Result<SymmetricKey> {
    Ok(MasterKey::derive_salted(password, salt.as_bytes(), kdf)?.stretch())
}

/// Encrypts a `.json` export as the web vault's password-protected one,
/// with this account's KDF settings: a new random salt (16 bytes, in
/// base64), a new random UUID as the value that shows a password is the
/// file's, each under its own random IV.
pub fn protect(plain: &str, password: &str, kdf: &Kdf) -> Result<Zeroizing<String>> {
    use base64::Engine;
    use rand::RngCore;
    let mut salt = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let salt = base64::engine::general_purpose::STANDARD.encode(salt);
    let validation = uuid::Uuid::new_v4().to_string();
    protect_with(
        plain,
        password,
        kdf,
        &salt,
        &validation,
        crate::crypto::new_iv(),
        crate::crypto::new_iv(),
    )
}

/// `protect`, with the salt, the validation value, and the IVs given: for
/// the vectors, which fix them.
pub fn protect_with(
    plain: &str,
    password: &str,
    kdf: &Kdf,
    salt: &str,
    validation: &str,
    validation_iv: [u8; 16],
    data_iv: [u8; 16],
) -> Result<Zeroizing<String>> {
    if password.is_empty() {
        return Err(Error::Refused(
            "a password-protected export needs a password",
        ));
    }
    let key = file_key(password, salt, kdf)?;
    let (kdf_type, kdf_iterations, kdf_memory, kdf_parallelism) = match *kdf {
        Kdf::Pbkdf2 { iterations } => (0, iterations, None, None),
        Kdf::Argon2id {
            iterations,
            memory_mib,
            parallelism,
        } => (1, iterations, Some(memory_mib), Some(parallelism)),
    };
    let file = Protected {
        encrypted: true,
        password_protected: true,
        salt,
        kdf_type,
        kdf_iterations,
        validation: key
            .encrypt_with_iv(validation.as_bytes(), validation_iv)
            .to_string(),
        data: key.encrypt_with_iv(plain.as_bytes(), data_iv).to_string(),
        kdf_memory,
        kdf_parallelism,
    };
    pretty(&file)
}

/// Why a password-protected file didn't open.
#[derive(Debug, PartialEq, Eq)]
pub enum Unopened {
    /// The password isn't the file's.
    WrongPassword,
    /// Its KDF settings are outside the bounds above, or of a kind PassBundle
    /// doesn't know.
    Settings,
    /// It's missing a part, or a part doesn't open with the right key.
    Damaged,
}

/// A JSON number that's a whole number, as JavaScript's `Number.isInteger`.
fn whole(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| {
        v.as_f64()
            .filter(|f| f.fract() == 0.0 && *f >= 0.0 && *f <= u64::MAX as f64)
            .map(|f| f as u64)
    })
}

/// The KDF a password-protected file says it was made with, if it's within
/// the bounds (`PBKDF2_ITERATIONS` and the rest).
pub fn file_kdf(file: &Value) -> Option<Kdf> {
    let within = |name: &str, range: &std::ops::RangeInclusive<u64>| {
        whole(&file[name])
            .filter(|n| range.contains(n))
            .map(|n| n as u32)
    };
    match whole(&file["kdfType"])? {
        0 => Some(Kdf::Pbkdf2 {
            iterations: within("kdfIterations", &PBKDF2_ITERATIONS)?,
        }),
        1 => Some(Kdf::Argon2id {
            iterations: within("kdfIterations", &ARGON2_ITERATIONS)?,
            memory_mib: within("kdfMemory", &ARGON2_MEMORY_MIB)?,
            parallelism: within("kdfParallelism", &ARGON2_PARALLELISM)?,
        }),
        _ => None,
    }
}

/// Whether a parsed `.json` file is a password-protected one.
pub fn is_protected(file: &Value) -> bool {
    file["encrypted"] == true && file["passwordProtected"] == true
}

/// Opens a password-protected file (upstream's
/// `BitwardenPasswordProtectedImporter`): its KDF settings checked before
/// anything is derived, then the validation value, then the export inside.
pub fn open_protected(
    file: &Value,
    password: &str,
) -> std::result::Result<Zeroizing<String>, Unopened> {
    let part = |name: &str| file[name].as_str().filter(|s| !s.is_empty());
    let (Some(salt), Some(validation), Some(data)) = (
        part("salt"),
        part("encKeyValidation_DO_NOT_EDIT"),
        part("data"),
    ) else {
        return Err(Unopened::Damaged);
    };
    if !is_protected(file) {
        return Err(Unopened::Damaged);
    }
    let kdf = file_kdf(file).ok_or(Unopened::Settings)?;
    if password.trim_matches(crate::csv::js_space).is_empty() {
        return Err(Unopened::WrongPassword);
    }
    let key = file_key(password, salt, &kdf).map_err(|_| Unopened::Settings)?;
    EncString::parse(validation)
        .and_then(|v| v.decrypt(&key))
        .map_err(|_| Unopened::WrongPassword)?;
    EncString::parse(data)
        .and_then(|d| d.decrypt_string(&key))
        .map_err(|_| Unopened::Damaged)
}

/// Writes an export where the person chose (or where a phone hands it on
/// from): a new file, or over the one there, readable only by this person
/// on a Mac and Linux.
pub fn write(path: &std::path::Path, text: &str) -> Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()));
    written.map_err(|e| match e.kind() {
        std::io::ErrorKind::StorageFull => Error::Disk { full: true },
        _ => Error::Damaged("the file couldn't be written"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_as_javascript_writes_them() {
        assert_eq!(
            js_date("2026-11-01T10:00:00.123456Z"),
            "2026-11-01T10:00:00.123Z"
        );
        assert_eq!(js_date("2026-11-01T10:00:00Z"), "2026-11-01T10:00:00.000Z");
        assert_eq!(
            js_date("2026-11-01T10:00:00.5+00:00"),
            "2026-11-01T10:00:00.500Z"
        );
        assert_eq!(js_date("2026-11-01T10:00:00"), "2026-11-01T10:00:00.000Z");
        // Not a date it knows: as it is.
        assert_eq!(
            js_date("2026-11-01T10:00:00+02:00"),
            "2026-11-01T10:00:00+02:00"
        );
        assert_eq!(js_date("yesterday"), "yesterday");
    }

    #[test]
    fn a_files_kdf_settings_are_checked_before_anything_is_derived() {
        let file = |kdf: Value| {
            let mut f = serde_json::json!({
                "encrypted": true, "passwordProtected": true, "salt": "c2FsdA==",
                "encKeyValidation_DO_NOT_EDIT": "2.AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                "data": "2.AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAA==|AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            });
            for (k, v) in kdf.as_object().unwrap() {
                f[k] = v.clone();
            }
            f
        };
        for refused in [
            serde_json::json!({ "kdfType": 0, "kdfIterations": 4999 }),
            serde_json::json!({ "kdfType": 0, "kdfIterations": 2_000_001 }),
            serde_json::json!({ "kdfType": 0, "kdfIterations": 600000.5 }),
            serde_json::json!({ "kdfType": 1, "kdfIterations": 3, "kdfMemory": 1025, "kdfParallelism": 4 }),
            serde_json::json!({ "kdfType": 1, "kdfIterations": 11, "kdfMemory": 64, "kdfParallelism": 4 }),
            serde_json::json!({ "kdfType": 1, "kdfIterations": 3, "kdfMemory": 64 }),
            serde_json::json!({ "kdfType": 2, "kdfIterations": 600000 }),
        ] {
            assert_eq!(
                open_protected(&file(refused.clone()), "any password"),
                Err(Unopened::Settings),
                "{refused}"
            );
        }
        assert_eq!(
            file_kdf(&serde_json::json!({ "kdfType": 0, "kdfIterations": 600000.0 })),
            Some(Kdf::Pbkdf2 {
                iterations: 600_000
            })
        );
        // A blank password is never the file's, and nothing is derived for it.
        let fine = file(serde_json::json!({ "kdfType": 0, "kdfIterations": 5000 }));
        assert_eq!(open_protected(&fine, "  "), Err(Unopened::WrongPassword));
        assert_eq!(
            open_protected(&serde_json::json!({ "encrypted": true }), "x"),
            Err(Unopened::Damaged)
        );
    }

    #[test]
    fn what_one_protects_the_other_opens() {
        let kdf = Kdf::Pbkdf2 { iterations: 5_000 };
        let file = protect("{\"encrypted\": false}", "file password", &kdf).unwrap();
        let parsed: Value = serde_json::from_str(&file).unwrap();
        assert_eq!(parsed["salt"].as_str().unwrap().len(), 24);
        assert!(parsed.get("kdfMemory").is_none());
        assert_eq!(
            open_protected(&parsed, "file password").unwrap().as_str(),
            "{\"encrypted\": false}"
        );
        assert_eq!(
            open_protected(&parsed, "not it"),
            Err(Unopened::WrongPassword)
        );
        // Two exports of the same never share a salt or an IV.
        let again: Value = serde_json::from_str(
            &protect("{\"encrypted\": false}", "file password", &kdf).unwrap(),
        )
        .unwrap();
        assert_ne!(parsed["salt"], again["salt"]);
        assert_ne!(parsed["data"], again["data"]);
    }
}
