//! The vault: the sync as the server sends it, and opening it with the master
//! password (docs/core.md, "The keys, in order" and "Items").

use std::collections::HashMap;

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::crypto::{EncString, PrivateKey, SymmetricKey};
use crate::error::{Error, Result};
use crate::kdf::{Kdf, MasterKey, Prelogin};

// What `/api/sync` sends, as far as the core reads it. Everything else is ignored.

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sync {
    pub profile: Profile,
    #[serde(default)]
    pub folders: Vec<FolderData>,
    #[serde(default)]
    pub collections: Vec<CollectionData>,
    #[serde(default)]
    pub ciphers: Vec<CipherData>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub email: String,
    pub key: Option<String>,
    pub private_key: Option<String>,
    #[serde(default)]
    pub organizations: Vec<OrganizationData>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationData {
    pub id: String,
    pub name: String,
    pub key: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderData {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionData {
    pub id: String,
    pub organization_id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CipherData {
    pub id: String,
    pub organization_id: Option<String>,
    pub folder_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: u8,
    pub name: Option<String>,
    pub notes: Option<String>,
    pub key: Option<String>,
    #[serde(default)]
    pub favorite: bool,
    pub deleted_date: Option<String>,
    pub login: Option<LoginData>,
    pub fields: Option<Vec<FieldData>>,
    #[serde(default)]
    pub collection_ids: Vec<String>,
    pub revision_date: Option<String>,
    // What an edit keeps as the server sent it (Kept).
    pub reprompt: Option<u8>,
    pub password_history: Option<serde_json::Value>,
    pub secure_note: Option<serde_json::Value>,
    pub ssh_key: Option<serde_json::Value>,
    pub card: Option<CardData>,
    pub identity: Option<IdentityData>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardData {
    pub cardholder_name: Option<String>,
    pub brand: Option<String>,
    pub number: Option<String>,
    pub exp_month: Option<String>,
    pub exp_year: Option<String>,
    pub code: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityData {
    pub title: Option<String>,
    pub first_name: Option<String>,
    pub middle_name: Option<String>,
    pub last_name: Option<String>,
    pub address1: Option<String>,
    pub address2: Option<String>,
    pub address3: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub company: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub ssn: Option<String>,
    pub username: Option<String>,
    pub passport_number: Option<String>,
    pub license_number: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginData {
    pub username: Option<String>,
    pub password: Option<String>,
    pub totp: Option<String>,
    pub uris: Option<Vec<UriData>>,
    // What an edit keeps as the server sent it (Kept).
    pub fido2_credentials: Option<serde_json::Value>,
    pub autofill_on_page_load: Option<bool>,
    pub password_revision_date: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UriData {
    pub uri: Option<String>,
    /// How the website is matched (matching.rs); null for the default.
    #[serde(rename = "match", default)]
    pub match_type: Option<u8>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FieldData {
    pub name: Option<String>,
    pub value: Option<String>,
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(rename = "linkedId", default)]
    pub linked_id: Option<serde_json::Value>,
}

impl Sync {
    pub fn from_json(json: &str) -> Result<Sync> {
        serde_json::from_str(json)
            .map_err(|e| Error::Malformed(format!("a sync the core can't read ({e})")))
    }
}

// The opened vault.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Login,
    SecureNote,
    Card,
    Identity,
    SshKey,
    Other(u8),
}

impl From<u8> for ItemKind {
    fn from(kind: u8) -> Self {
        match kind {
            1 => ItemKind::Login,
            2 => ItemKind::SecureNote,
            3 => ItemKind::Card,
            4 => ItemKind::Identity,
            5 => ItemKind::SshKey,
            other => ItemKind::Other(other),
        }
    }
}

/// A custom field. Hidden ones are secrets.
pub struct Field {
    pub name: String,
    pub value: Zeroizing<String>,
    pub hidden: bool,
}

/// Our own data on an item, in custom fields named "keykeeper.<something>":
/// fill rules, an expiration date, a template marker (DESIGN.md, "Choosing
/// what fills where"). Never shown or edited as custom fields, and kept by an
/// edit, as the web vault and the extension do.
pub const RESERVED_FIELD_PREFIX: &str = "keykeeper.";

/// The expiration date the web vault and the extension set ("YYYY-MM-DD").
pub const EXPIRES_FIELD: &str = "keykeeper.expires";

impl Field {
    pub fn reserved(&self) -> bool {
        self.name.starts_with(RESERVED_FIELD_PREFIX)
    }

    /// Whether the app's editor edits it: a named text or hidden field of
    /// the person's own (`kind`, upstream's field type, from `Kept`).
    pub fn edited_by_the_app(&self, kind: u8) -> bool {
        kind <= 1 && !self.reserved() && !self.name.is_empty()
    }
}

/// An opened item. What lists and search show is plain; the secrets (notes,
/// password, one-time code secret, hidden fields) stay zeroizing and leave the
/// core only when revealed or copied (docs/core.md, "Memory").
pub struct Item {
    pub id: String,
    pub kind: ItemKind,
    pub name: String,
    pub folder_id: Option<String>,
    pub organization_id: Option<String>,
    pub collection_ids: Vec<String>,
    pub favorite: bool,
    pub deleted: bool,
    pub username: Option<String>,
    pub uris: Vec<String>,
    /// How each of `uris` is matched for filling (matching.rs): None is the default.
    pub uri_matches: Vec<Option<u8>>,
    pub notes: Option<Zeroizing<String>>,
    pub password: Option<Zeroizing<String>>,
    pub totp: Option<Zeroizing<String>>,
    pub fields: Vec<Field>,
    /// The server's revision, which an edit names so it can't overwrite a newer one.
    pub revision_date: Option<String>,
    /// A card's details, for a card.
    pub card: Option<Card>,
    /// An identity's details, for an identity.
    pub identity: Option<Identity>,
    /// What an edit keeps that the app doesn't show or change.
    pub kept: Kept,
}

/// A card's or an identity's values the app copies or reveals, by name:
/// the secrets ("cardNumber", "cardCode", "ssn", "passportNumber",
/// "licenseNumber"), and the plain ones (`PLAIN_DETAILS`).
pub const SECRET_DETAILS: [&str; 5] = [
    "cardNumber",
    "cardCode",
    "ssn",
    "passportNumber",
    "licenseNumber",
];
pub const PLAIN_DETAILS: [&str; 8] = [
    "cardholderName",
    "expiration",
    "fullName",
    "company",
    "email",
    "phone",
    "identityUsername",
    "address",
];

impl Item {
    /// One of a card's or an identity's values (`SECRET_DETAILS`,
    /// `PLAIN_DETAILS`); None if it hasn't that one.
    pub fn detail(&self, what: &str) -> Option<Zeroizing<String>> {
        let text = |t: &Option<String>| t.clone().filter(|t| !t.is_empty()).map(Zeroizing::new);
        let secret = |t: &Option<Zeroizing<String>>| {
            t.as_ref()
                .filter(|t| !t.is_empty())
                .map(|t| Zeroizing::new(t.as_str().to_owned()))
        };
        if let Some(card) = &self.card {
            return match what {
                "cardNumber" => secret(&card.number),
                "cardCode" => secret(&card.code),
                "cardholderName" => text(&card.cardholder_name),
                "expiration" => card.expiration().map(Zeroizing::new),
                _ => None,
            };
        }
        let id = self.identity.as_ref()?;
        match what {
            "ssn" => secret(&id.ssn),
            "passportNumber" => secret(&id.passport_number),
            "licenseNumber" => secret(&id.license_number),
            "fullName" => Some(id.full_name())
                .filter(|n| !n.is_empty())
                .map(Zeroizing::new),
            "company" => text(&id.company),
            "email" => text(&id.email),
            "phone" => text(&id.phone),
            "identityUsername" => text(&id.username),
            "address" => id.address().map(Zeroizing::new),
            _ => None,
        }
    }
}

/// A card's details (docs/core.md, "Items"). The number and the security
/// code are secrets, like a password.
#[derive(Default)]
pub struct Card {
    pub cardholder_name: Option<String>,
    pub brand: Option<String>,
    pub exp_month: Option<String>,
    pub exp_year: Option<String>,
    pub number: Option<Zeroizing<String>>,
    pub code: Option<Zeroizing<String>>,
}

impl Card {
    /// "Visa, *1111": the brand and the number's last 4 digits, as lists show a card.
    pub fn summary(&self) -> String {
        let last4: Option<String> = self.number.as_ref().map(|n| {
            let digits: Vec<char> = n.chars().filter(char::is_ascii_digit).collect();
            digits[digits.len().saturating_sub(4)..].iter().collect()
        });
        match (self.brand.as_deref(), last4.filter(|l| !l.is_empty())) {
            (Some(brand), Some(last4)) => format!("{brand}, *{last4}"),
            (Some(brand), None) => brand.to_owned(),
            (None, Some(last4)) => format!("*{last4}"),
            (None, None) => String::new(),
        }
    }

    /// "07/2029", or what there is of it.
    pub fn expiration(&self) -> Option<String> {
        let month = self.exp_month.as_deref().map(|m| format!("{m:0>2}"));
        match (month, self.exp_year.as_deref()) {
            (Some(m), Some(y)) => Some(format!("{m}/{y}")),
            (Some(m), None) => Some(m),
            (None, Some(y)) => Some(y.to_owned()),
            (None, None) => None,
        }
    }
}

/// An identity's details (docs/core.md, "Items"). The Social Security,
/// passport, and license numbers are secrets, like a password.
#[derive(Default)]
pub struct Identity {
    pub title: Option<String>,
    pub first_name: Option<String>,
    pub middle_name: Option<String>,
    pub last_name: Option<String>,
    pub address1: Option<String>,
    pub address2: Option<String>,
    pub address3: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub company: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub username: Option<String>,
    pub ssn: Option<Zeroizing<String>>,
    pub passport_number: Option<Zeroizing<String>>,
    pub license_number: Option<Zeroizing<String>>,
}

impl Identity {
    /// "Ms Ana María López": the name, as lists show an identity.
    pub fn full_name(&self) -> String {
        [
            &self.title,
            &self.first_name,
            &self.middle_name,
            &self.last_name,
        ]
        .iter()
        .filter_map(|p| p.as_deref())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
    }

    /// The address, a line each: the street lines, then "City, State
    /// Postal code", then the country.
    pub fn address(&self) -> Option<String> {
        let place = [&self.city, &self.state]
            .iter()
            .filter_map(|p| p.as_deref())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        let place = [Some(place.as_str()), self.postal_code.as_deref()]
            .iter()
            .flatten()
            .filter(|p| !p.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let lines: Vec<&str> = [
            self.address1.as_deref(),
            self.address2.as_deref(),
            self.address3.as_deref(),
            Some(place.as_str()),
            self.country.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|l| !l.is_empty())
        .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

/// What editing an item keeps that the app doesn't show or change
/// (docs/core.md, "Editing keeps what the app doesn't show"): the item's
/// own key, whether it asks for the master password, its password history,
/// its passkeys, and the custom fields the editor doesn't edit. The
/// encrypted parts are kept as the server sent them, under the item's key.
#[derive(Default)]
pub struct Kept {
    /// The item's own key, as sent (under its owner's key), and opened.
    pub key: Option<String>,
    pub item_key: Option<SymmetricKey>,
    /// 1: ask for the master password before its secrets are used.
    pub reprompt: u8,
    /// Earlier passwords, newest first, encrypted as sent.
    pub password_history: Vec<serde_json::Value>,
    /// A note's own data, as sent.
    pub secure_note: Option<serde_json::Value>,
    /// A login's passkeys, whether it fills on page load, and when its
    /// password last changed, as sent.
    pub fido2_credentials: Option<serde_json::Value>,
    pub autofill_on_page_load: Option<bool>,
    pub password_revision_date: Option<String>,
    /// Each of `fields`' upstream type (0 text, 1 hidden, 2 yes or no, 3
    /// linked) and what a linked one is linked to.
    pub field_kinds: Vec<(u8, Option<serde_json::Value>)>,
}

/// A website's checksum, as upstream writes it: base64 of the address's SHA-256.
fn uri_checksum(uri: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    base64::engine::general_purpose::STANDARD.encode(Sha256::digest(uri.as_bytes()))
}

/// Upstream keeps the 5 newest earlier passwords.
const PASSWORD_HISTORY: usize = 5;

/// Now, as upstream's dates are written (ISO 8601, UTC, milliseconds).
fn now_iso8601() -> String {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    iso8601(since.as_secs(), since.subsec_millis())
}

/// Seconds since 1970 as a date, by the proleptic Gregorian calendar
/// (Howard Hinnant's days-to-civil).
fn iso8601(secs: u64, millis: u32) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// An item as the person edits it, before it's encrypted.
#[derive(Default)]
pub struct ItemDraft {
    /// Login or secure note.
    pub note: bool,
    pub name: String,
    pub notes: Option<Zeroizing<String>>,
    pub username: Option<String>,
    pub password: Option<Zeroizing<String>>,
    pub totp: Option<Zeroizing<String>>,
    pub uris: Vec<String>,
    pub fields: Vec<Field>,
    pub folder_id: Option<String>,
    pub organization_id: Option<String>,
    pub favorite: bool,
}

impl ItemDraft {
    /// A draft of an opened item, to edit.
    pub fn of(item: &Item) -> ItemDraft {
        let copy = |z: &Option<Zeroizing<String>>| {
            z.as_ref().map(|v| Zeroizing::new(v.as_str().to_owned()))
        };
        ItemDraft {
            note: item.kind == ItemKind::SecureNote,
            name: item.name.clone(),
            notes: copy(&item.notes),
            username: item.username.clone(),
            password: copy(&item.password),
            totp: copy(&item.totp),
            uris: item.uris.clone(),
            // The fields the editor edits: the person's own text and hidden
            // ones (Kept).
            fields: item
                .fields
                .iter()
                .zip(&item.kept.field_kinds)
                .filter(|(f, (kind, _))| f.edited_by_the_app(*kind))
                .map(|(f, _)| Field {
                    name: f.name.clone(),
                    value: Zeroizing::new(f.value.as_str().to_owned()),
                    hidden: f.hidden,
                })
                .collect(),
            folder_id: item.folder_id.clone(),
            organization_id: item.organization_id.clone(),
            favorite: item.favorite,
        }
    }
}

/// An item that doesn't open, and why: shown in the list as such, never guessed at.
pub struct DamagedItem {
    pub id: String,
    pub error: Error,
}

pub struct Vault {
    pub user_id: String,
    pub email: String,
    pub folders: HashMap<String, String>,
    pub collections: HashMap<String, String>,
    /// Each collection's organization.
    pub collection_orgs: HashMap<String, String>,
    pub organizations: HashMap<String, String>,
    pub items: Vec<Item>,
    pub damaged: Vec<DamagedItem>,
    keys: Keyring,
}

struct Keyring {
    user: SymmetricKey,
    private: PrivateKey,
    organizations: HashMap<String, SymmetricKey>,
}

fn parse_opt(value: &Option<String>) -> Result<Option<EncString>> {
    value.as_deref().map(EncString::parse).transpose()
}

fn open_text(value: &Option<String>, key: &SymmetricKey) -> Result<Option<Zeroizing<String>>> {
    parse_opt(value)?.map(|e| e.decrypt_string(key)).transpose()
}

fn plain(value: Option<Zeroizing<String>>) -> Option<String> {
    value.map(|v| v.as_str().to_owned())
}

impl Vault {
    /// Opens a sync with the master password: the master key from the KDF the
    /// prelogin gave (checked first), then the user key, the private key,
    /// organization keys, folders, collections, and items.
    pub fn unlock(sync: &Sync, prelogin: &Prelogin, password: &str) -> Result<Vault> {
        let kdf = Kdf::from_prelogin(prelogin)?;
        let master_key = MasterKey::derive(password, &sync.profile.email, &kdf)?;
        let protected = EncString::parse(
            sync.profile
                .key
                .as_deref()
                .ok_or(Error::Malformed("a profile with no key".into()))?,
        )?;
        let user = match protected.decrypt_key(&master_key.stretch()) {
            Err(Error::Refused(_)) => return Err(Error::WrongPassword),
            other => other?,
        };
        Vault::open(sync, user)
    }

    /// Opens a sync with the user key itself (after unlocking another way).
    pub fn open(sync: &Sync, user: SymmetricKey) -> Result<Vault> {
        let private_der = EncString::parse(
            sync.profile
                .private_key
                .as_deref()
                .ok_or(Error::Malformed("a profile with no private key".into()))?,
        )?
        .decrypt(&user)?;
        let private = PrivateKey::from_pkcs8_der(&private_der)?;

        let mut organizations = HashMap::new();
        let mut organization_keys = HashMap::new();
        for org in &sync.profile.organizations {
            organizations.insert(org.id.clone(), org.name.clone());
            // An organization whose key doesn't open leaves its items damaged, not the vault.
            if let Some(key) = &org.key {
                if let Ok(key) = EncString::parse(key)
                    .and_then(|e| private.decrypt(&e))
                    .and_then(|bytes| SymmetricKey::from_bytes(&bytes))
                {
                    organization_keys.insert(org.id.clone(), key);
                }
            }
        }
        let keys = Keyring {
            user,
            private,
            organizations: organization_keys,
        };

        let folders = sync
            .folders
            .iter()
            .filter_map(|f| {
                let name = EncString::parse(&f.name)
                    .and_then(|e| e.decrypt_string(&keys.user))
                    .ok()?;
                Some((f.id.clone(), name.as_str().to_owned()))
            })
            .collect();
        let collections = sync
            .collections
            .iter()
            .filter_map(|c| {
                let key = keys.organizations.get(&c.organization_id)?;
                let name = EncString::parse(&c.name)
                    .and_then(|e| e.decrypt_string(key))
                    .ok()?;
                Some((c.id.clone(), name.as_str().to_owned()))
            })
            .collect();

        let collection_orgs = sync
            .collections
            .iter()
            .map(|c| (c.id.clone(), c.organization_id.clone()))
            .collect();
        let mut items = Vec::new();
        let mut damaged = Vec::new();
        for cipher in &sync.ciphers {
            match keys.open_item(cipher) {
                Ok(item) => items.push(item),
                Err(error) => damaged.push(DamagedItem {
                    id: cipher.id.clone(),
                    error,
                }),
            }
        }
        items.sort_by_key(|i| i.name.to_lowercase());

        Ok(Vault {
            user_id: sync.profile.id.clone(),
            email: sync.profile.email.clone(),
            folders,
            collections,
            collection_orgs,
            organizations,
            items,
            damaged,
            keys,
        })
    }

    /// Encrypts a draft as the server takes it (upstream's cipher request):
    /// a new item under its organization's key, or the user key, without an
    /// item key, like the other apps. Pass the item being edited, if any, so
    /// the server can refuse an edit to a version that's since changed. An
    /// edit keeps what the app doesn't show or change (`Kept`), under the
    /// item's own key if it has one, and puts a changed password, and each
    /// changed or removed hidden field, in its history, as upstream's apps do.
    pub fn encrypt_item(
        &self,
        draft: &ItemDraft,
        editing: Option<&Item>,
    ) -> Result<serde_json::Value> {
        use serde_json::json;
        if let Some(item) = editing {
            if !matches!(item.kind, ItemKind::Login | ItemKind::SecureNote) {
                return Err(Error::Refused(
                    "the app edits logins and notes; edit cards, identities, and SSH keys in the web vault",
                ));
            }
            if (item.kind == ItemKind::SecureNote) != draft.note {
                return Err(Error::Refused("a login stays a login, and a note a note"));
            }
            if item.organization_id != draft.organization_id {
                return Err(Error::Refused(
                    "moving an item between owners is for the web vault",
                ));
            }
        }
        let owner = match &draft.organization_id {
            Some(org) => self
                .keys
                .organizations
                .get(org)
                .ok_or(Error::Damaged("that organization's key doesn't open"))?,
            None => &self.keys.user,
        };
        let kept = editing.map(|i| &i.kept);
        let key = kept.and_then(|k| k.item_key.as_ref()).unwrap_or(owner);
        let enc = |text: &str| key.encrypt_str(text).to_string();
        let enc_opt = |text: Option<&str>| text.filter(|t| !t.is_empty()).map(enc);
        let now = now_iso8601();

        // Earlier passwords, newest first: a changed password, then each
        // hidden field whose value changed or that went, each put in front,
        // as upstream keeps them.
        let mut history = kept.map(|k| k.password_history.clone()).unwrap_or_default();
        let mut password_changed = false;
        if let Some(item) = editing {
            let old = item.password.as_deref().map(String::as_str).unwrap_or("");
            let new = draft.password.as_deref().map(String::as_str).unwrap_or("");
            if !draft.note && !old.is_empty() && old != new {
                password_changed = true;
                history.insert(0, json!({ "password": enc(old), "lastUsedDate": now }));
            }
            for (field, (kind, _)) in item.fields.iter().zip(&item.kept.field_kinds) {
                if *kind != 1
                    || !field.edited_by_the_app(*kind)
                    || field.name.is_empty()
                    || field.value.is_empty()
                {
                    continue;
                }
                let still = draft.fields.iter().any(|f| {
                    f.hidden && f.name == field.name && f.value.as_str() == field.value.as_str()
                });
                if !still {
                    let entry = Zeroizing::new(format!("{}: {}", field.name, field.value.as_str()));
                    history.insert(0, json!({ "password": enc(&entry), "lastUsedDate": now }));
                }
            }
        }
        history.truncate(PASSWORD_HISTORY);

        let mut fields: Vec<serde_json::Value> = draft
            .fields
            .iter()
            .map(|f| {
                json!({
                    "name": enc(&f.name),
                    "value": enc(&f.value),
                    "type": if f.hidden { 1 } else { 0 },
                    "linkedId": null,
                })
            })
            .collect();
        // Yes-or-no and linked fields, and our own data, which the editor
        // doesn't edit, stay, each where it was among the item's fields.
        if let Some(item) = editing {
            let kinds = &item.kept.field_kinds;
            for (at, (field, (kind, linked))) in item.fields.iter().zip(kinds).enumerate() {
                if !field.edited_by_the_app(*kind) {
                    fields.insert(
                        at.min(fields.len()),
                        json!({
                            "name": enc_opt(Some(field.name.as_str())),
                            "value": enc_opt(Some(field.value.as_str())),
                            "type": kind,
                            "linkedId": linked,
                        }),
                    );
                }
            }
        }

        let mut body = json!({
            "type": if draft.note { 2 } else { 1 },
            "organizationId": draft.organization_id,
            "folderId": draft.folder_id,
            "name": enc(&draft.name),
            "notes": enc_opt(draft.notes.as_deref().map(String::as_str)),
            "favorite": draft.favorite,
            "reprompt": kept.map_or(0, |k| k.reprompt),
            "key": kept.and_then(|k| k.key.clone()),
            "fields": fields,
            "passwordHistory": if history.is_empty() { serde_json::Value::Null } else { history.into() },
            "encryptedFor": self.user_id,
            "lastKnownRevisionDate": editing.and_then(|i| i.revision_date.clone()),
        });
        if draft.note {
            body["secureNote"] = kept
                .and_then(|k| k.secure_note.clone())
                .unwrap_or_else(|| json!({ "type": 0 }));
        } else {
            body["login"] = json!({
                "username": enc_opt(draft.username.as_deref()),
                "password": enc_opt(draft.password.as_deref().map(String::as_str)),
                "totp": enc_opt(draft.totp.as_deref().map(String::as_str)),
                // A website kept from the item being edited keeps how it's matched,
                // which the web vault can set and this app doesn't show.
                // Each with its checksum, as upstream's apps write it: an
                // item with its own key shows only websites whose checksum
                // matches, so a server can't swap one in.
                "uris": draft.uris.iter().map(|u| json!({
                    "uri": enc(u),
                    "uriChecksum": enc(&uri_checksum(u)),
                    "match": editing.and_then(|i| i.uris.iter().position(|kept| kept == u).and_then(|at| i.uri_matches.get(at).copied().flatten())),
                })).collect::<Vec<_>>(),
                "fido2Credentials": kept.and_then(|k| k.fido2_credentials.clone()),
                "autofillOnPageLoad": kept.and_then(|k| k.autofill_on_page_load),
                "passwordRevisionDate": if password_changed {
                    Some(now.clone())
                } else {
                    kept.and_then(|k| k.password_revision_date.clone())
                },
            });
        }
        Ok(body)
    }

    pub(crate) fn user_key(&self) -> &SymmetricKey {
        &self.keys.user
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// The person's public key, as SPKI DER, from their private key.
    pub fn public_key_der(&self) -> Vec<u8> {
        self.keys.private.public_key_der()
    }

    /// Items whose name, username, or websites contain every word of the
    /// query, ignoring case. An empty query matches everything.
    pub fn search<'a>(&'a self, query: &str) -> Vec<&'a Item> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.items
            .iter()
            .filter(|i| !i.deleted)
            .filter(|i| {
                let mut haystack = format!(
                    "{} {} {}",
                    i.name.to_lowercase(),
                    i.username.as_deref().unwrap_or("").to_lowercase(),
                    i.uris.join(" ").to_lowercase()
                );
                // A card by its holder and brand; an identity by its name,
                // company, email, and username. Never by a secret.
                if let Some(card) = &i.card {
                    for part in [&card.cardholder_name, &card.brand].into_iter().flatten() {
                        haystack.push(' ');
                        haystack.push_str(&part.to_lowercase());
                    }
                }
                if let Some(id) = &i.identity {
                    haystack.push(' ');
                    haystack.push_str(&id.full_name().to_lowercase());
                    for part in [&id.company, &id.email, &id.username].into_iter().flatten() {
                        haystack.push(' ');
                        haystack.push_str(&part.to_lowercase());
                    }
                }
                words.iter().all(|w| haystack.contains(w.as_str()))
            })
            .collect()
    }
}

impl Keyring {
    /// The key an item's strings are under: its own item key if it has one
    /// (opened with its owner's key), else its organization's, else the user's.
    fn key_for(&self, cipher: &CipherData) -> Result<SymmetricKey> {
        let owner = match &cipher.organization_id {
            Some(org) => self
                .organizations
                .get(org)
                .ok_or(Error::Damaged("its organization's key doesn't open"))?,
            None => &self.user,
        };
        match &cipher.key {
            Some(item_key) => EncString::parse(item_key)?.decrypt_key(owner),
            None => Ok(owner.clone()),
        }
    }

    fn open_item(&self, c: &CipherData) -> Result<Item> {
        let key = self.key_for(c)?;
        let name = open_text(&c.name, &key)?
            .map(|n| n.as_str().to_owned())
            .unwrap_or_default();
        let notes = open_text(&c.notes, &key)?;
        let (mut username, mut password, mut totp, mut uris) = (None, None, None, Vec::new());
        let mut uri_matches = Vec::new();
        if let Some(login) = &c.login {
            username = plain(open_text(&login.username, &key)?);
            password = open_text(&login.password, &key)?;
            totp = open_text(&login.totp, &key)?;
            for uri in login.uris.iter().flatten() {
                if let Some(u) = open_text(&uri.uri, &key)? {
                    uris.push(u.as_str().to_owned());
                    uri_matches.push(uri.match_type);
                }
            }
        }
        let mut fields = Vec::new();
        let mut field_kinds = Vec::new();
        for f in c.fields.iter().flatten() {
            fields.push(Field {
                name: plain(open_text(&f.name, &key)?).unwrap_or_default(),
                value: open_text(&f.value, &key)?.unwrap_or_else(|| Zeroizing::new(String::new())),
                hidden: f.kind == 1,
            });
            field_kinds.push((f.kind, f.linked_id.clone()));
        }
        let open_plain =
            |text: &Option<String>| -> Result<Option<String>> { Ok(plain(open_text(text, &key)?)) };
        let card = match &c.card {
            Some(d) if ItemKind::from(c.kind) == ItemKind::Card => Some(Card {
                cardholder_name: open_plain(&d.cardholder_name)?,
                brand: open_plain(&d.brand)?,
                exp_month: open_plain(&d.exp_month)?,
                exp_year: open_plain(&d.exp_year)?,
                number: open_text(&d.number, &key)?,
                code: open_text(&d.code, &key)?,
            }),
            _ => None,
        };
        let identity = match &c.identity {
            Some(d) if ItemKind::from(c.kind) == ItemKind::Identity => Some(Identity {
                title: open_plain(&d.title)?,
                first_name: open_plain(&d.first_name)?,
                middle_name: open_plain(&d.middle_name)?,
                last_name: open_plain(&d.last_name)?,
                address1: open_plain(&d.address1)?,
                address2: open_plain(&d.address2)?,
                address3: open_plain(&d.address3)?,
                city: open_plain(&d.city)?,
                state: open_plain(&d.state)?,
                postal_code: open_plain(&d.postal_code)?,
                country: open_plain(&d.country)?,
                company: open_plain(&d.company)?,
                email: open_plain(&d.email)?,
                phone: open_plain(&d.phone)?,
                username: open_plain(&d.username)?,
                ssn: open_text(&d.ssn, &key)?,
                passport_number: open_text(&d.passport_number, &key)?,
                license_number: open_text(&d.license_number, &key)?,
            }),
            _ => None,
        };
        let login = c.login.as_ref();
        let kept = Kept {
            key: c.key.clone(),
            item_key: c.key.as_ref().map(|_| key.clone()),
            reprompt: c.reprompt.unwrap_or(0),
            password_history: c
                .password_history
                .as_ref()
                .and_then(|h| h.as_array())
                .cloned()
                .unwrap_or_default(),
            secure_note: c.secure_note.clone(),
            fido2_credentials: login.and_then(|l| l.fido2_credentials.clone()),
            autofill_on_page_load: login.and_then(|l| l.autofill_on_page_load),
            password_revision_date: login.and_then(|l| l.password_revision_date.clone()),
            field_kinds,
        };
        Ok(Item {
            id: c.id.clone(),
            kind: ItemKind::from(c.kind),
            name,
            folder_id: c.folder_id.clone(),
            organization_id: c.organization_id.clone(),
            collection_ids: c.collection_ids.clone(),
            favorite: c.favorite,
            deleted: c.deleted_date.is_some(),
            username,
            uris,
            uri_matches,
            notes,
            password,
            totp,
            fields,
            revision_date: c.revision_date.clone(),
            card,
            identity,
            kept,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_written_as_upstream_writes_them() {
        assert_eq!(iso8601(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(951_782_400, 5), "2000-02-29T00:00:00.005Z");
        assert_eq!(iso8601(1_798_675_199, 999), "2026-12-30T23:59:59.999Z");
        assert!(now_iso8601().starts_with("20") && now_iso8601().ends_with('Z'));
    }

    #[test]
    fn cards_and_identities_read_as_lists_and_views_show_them() {
        let card = Card {
            brand: Some("Visa".into()),
            number: Some(Zeroizing::new("4111 1111 1111 1234".into())),
            exp_month: Some("7".into()),
            exp_year: Some("2029".into()),
            ..Card::default()
        };
        assert_eq!(card.summary(), "Visa, *1234");
        assert_eq!(card.expiration().as_deref(), Some("07/2029"));
        let bare = Card {
            number: Some(Zeroizing::new("12".into())),
            ..Card::default()
        };
        assert_eq!(bare.summary(), "*12");
        assert_eq!(Card::default().summary(), "");
        assert_eq!(Card::default().expiration(), None);

        let identity = Identity {
            first_name: Some("Ana".into()),
            last_name: Some("López".into()),
            address1: Some("1 Example Street".into()),
            city: Some("Springfield".into()),
            state: Some("IL".into()),
            postal_code: Some("62701".into()),
            country: Some("US".into()),
            ..Identity::default()
        };
        assert_eq!(identity.full_name(), "Ana López");
        assert_eq!(
            identity.address().as_deref(),
            Some("1 Example Street\nSpringfield, IL 62701\nUS")
        );
        assert_eq!(Identity::default().address(), None);
        assert_eq!(
            Identity {
                postal_code: Some("62701".into()),
                ..Identity::default()
            }
            .address()
            .as_deref(),
            Some("62701")
        );
    }
}
