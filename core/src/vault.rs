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
    /// Upstream's three fields on the person's grant, the closest preset
    /// of their grid (`keykeeper`), for a server that sends no grid.
    #[serde(default)]
    pub read_only: Option<bool>,
    #[serde(default)]
    pub hide_passwords: Option<bool>,
    #[serde(default)]
    pub manage: Option<bool>,
    /// What the person can do in this shared folder (docs/core.md, "Folders
    /// and permissions"), as PassBundle's server sends it.
    #[serde(default)]
    pub keykeeper: Option<Grid>,
}

/// What someone can do in a shared folder, or with an item: PassBundle's
/// grid of capabilities (passbundle-server's keykeeper_folder_permissions.rs,
/// passbundle-clients' folder-permissions.ts). Having the folder at all is
/// "see items". The server enforces creating, editing, deleting, managing,
/// and viewing history; the apps keep to seeing passwords and sharing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Grid {
    pub see_passwords: bool,
    pub create: bool,
    pub edit: bool,
    pub delete: bool,
    pub export: bool,
    pub share: bool,
    pub print: bool,
    pub view_history: bool,
    pub manage: bool,
}

impl Grid {
    /// Everything: the person's own items.
    pub const ALL: Grid = Grid {
        see_passwords: true,
        create: true,
        edit: true,
        delete: true,
        export: true,
        share: true,
        print: true,
        view_history: true,
        manage: true,
    };

    /// The preset upstream's three fields name, as the server reads a grant
    /// saved without a grid.
    pub fn preset(read_only: bool, hide_passwords: bool, manage: bool) -> Grid {
        if manage {
            return Grid::ALL;
        }
        let edit = !read_only;
        let see_passwords = !hide_passwords;
        Grid {
            see_passwords,
            create: edit,
            edit,
            delete: edit,
            export: false,
            share: edit && see_passwords,
            print: see_passwords,
            view_history: true,
            manage: false,
        }
    }

    /// For each capability, the more open of the two: an item in several
    /// shared folders, as the server decides.
    pub fn or(self, other: Grid) -> Grid {
        Grid {
            see_passwords: self.see_passwords || other.see_passwords,
            create: self.create || other.create,
            edit: self.edit || other.edit,
            delete: self.delete || other.delete,
            export: self.export || other.export,
            share: self.share || other.share,
            print: self.print || other.print,
            view_history: self.view_history || other.view_history,
            manage: self.manage || other.manage,
        }
    }
}

impl CollectionData {
    /// The person's grid here: the server's, or the preset of upstream's
    /// fields. A field the server leaves out counts as the least access
    /// (read only, passwords hidden, no managing): the server decides, and
    /// the app never offers more than it said.
    pub fn grid(&self) -> Grid {
        self.keykeeper.unwrap_or_else(|| {
            Grid::preset(
                self.read_only.unwrap_or(true),
                self.hide_passwords.unwrap_or(true),
                self.manage.unwrap_or(false),
            )
        })
    }
}

/// What an item's upstream fields say the person may do with it, as the
/// server works them out from their shared folders.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CipherPermissions {
    pub delete: Option<bool>,
    pub restore: Option<bool>,
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
    pub creation_date: Option<String>,
    /// The person may change it (upstream's `edit`), see its passwords
    /// (`viewPassword`), and move it to the trash (`permissions.delete`).
    /// For an organization's item, each is no when the server doesn't say;
    /// the person's own items are theirs to do anything with.
    pub edit: Option<bool>,
    pub view_password: Option<bool>,
    pub permissions: Option<CipherPermissions>,
    // What an edit keeps as the server sent it (Kept).
    pub reprompt: Option<u8>,
    pub password_history: Option<serde_json::Value>,
    pub secure_note: Option<serde_json::Value>,
    pub ssh_key: Option<SshKeyData>,
    pub card: Option<CardData>,
    pub identity: Option<IdentityData>,
    /// Upstream's newer kinds (6, 7, 8), as sent: shown, not edited.
    pub bank_account: Option<serde_json::Value>,
    pub drivers_license: Option<serde_json::Value>,
    pub passport: Option<serde_json::Value>,
    #[serde(default)]
    pub attachments: Option<Vec<AttachmentData>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentData {
    pub id: String,
    pub file_name: Option<String>,
    pub key: Option<String>,
    /// Bytes, as a number or a string of one.
    pub size: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyData {
    pub private_key: Option<String>,
    pub public_key: Option<String>,
    pub key_fingerprint: Option<String>,
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
    /// Upstream's newer kinds, which the web vault makes where the server
    /// offers them: the apps show them.
    BankAccount,
    DriversLicense,
    Passport,
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
            6 => ItemKind::BankAccount,
            7 => ItemKind::DriversLicense,
            8 => ItemKind::Passport,
            other => ItemKind::Other(other),
        }
    }
}

/// A custom field. Hidden ones are secrets; yes-or-no ones hold "true" or
/// "false".
pub struct Field {
    pub name: String,
    pub value: Zeroizing<String>,
    pub hidden: bool,
    pub boolean: bool,
}

/// One of a bank account's, a driver's license's, or a passport's values
/// (upstream's newer kinds): what the view calls it, and whether it's a
/// secret, shown only when revealed, like a password.
pub struct Detail {
    /// Upstream's name for it, as `kind.property` ("bankAccount.iban").
    pub key: String,
    pub label: &'static str,
    pub value: Zeroizing<String>,
    pub secret: bool,
}

/// The values of upstream's newer kinds, in the order the web vault shows
/// them (passbundle-clients, item-versions.ts): (property, label, secret).
pub const BANK_ACCOUNT_DETAILS: [(&str, &str, bool); 10] = [
    ("bankName", "Bank name", false),
    ("nameOnAccount", "Name on account", false),
    ("accountType", "Account type", false),
    ("accountNumber", "Account number", true),
    ("routingNumber", "Routing number", false),
    ("branchNumber", "Branch number", false),
    ("pin", "PIN", true),
    ("swiftCode", "SWIFT code", true),
    ("iban", "IBAN", true),
    ("bankContactPhone", "Bank's phone", false),
];
pub const DRIVERS_LICENSE_DETAILS: [(&str, &str, bool); 11] = [
    ("firstName", "First name", false),
    ("middleName", "Middle name", false),
    ("lastName", "Last name", false),
    ("dateOfBirth", "Date of birth", false),
    ("licenseNumber", "License number", true),
    ("issuingCountry", "Issuing country", false),
    ("issuingState", "Issuing state", false),
    ("issueDate", "Issue date", false),
    ("expirationDate", "Expiration date", false),
    ("issuingAuthority", "Issuing authority", false),
    ("licenseClass", "License class", false),
];
pub const PASSPORT_DETAILS: [(&str, &str, bool); 13] = [
    ("givenName", "First name", false),
    ("surname", "Last name", false),
    ("dateOfBirth", "Date of birth", false),
    ("sex", "Sex", false),
    ("birthPlace", "Place of birth", false),
    ("nationality", "Nationality", false),
    ("issuingCountry", "Issuing country", false),
    ("passportNumber", "Passport number", true),
    ("passportType", "Passport type", false),
    ("nationalIdentificationNumber", "National ID number", true),
    ("issuingAuthority", "Issuing authority", false),
    ("issueDate", "Issue date", false),
    ("expirationDate", "Expiration date", false),
];

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

    /// Whether the app's editor edits it: a named text, hidden, or yes-or-no
    /// field of the person's own (`kind`, upstream's field type, from
    /// `Kept`). Linked fields, nameless ones (or blank), and our own data stay as they are.
    pub fn edited_by_the_app(&self, kind: u8) -> bool {
        kind <= 2 && !self.reserved() && !self.name.trim().is_empty()
    }

    /// Upstream's field type: 0 text, 1 hidden, 2 yes or no.
    pub fn kind(&self) -> u8 {
        if self.hidden {
            1
        } else if self.boolean {
            2
        } else {
            0
        }
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
    /// When it was made, as the server says (ISO 8601).
    pub creation_date: Option<String>,
    /// What the server lets the person do with it (upstream's fields; see
    /// `Vault::grid`): change it, see its secrets, and move it to the trash.
    pub editable: bool,
    pub view_password: bool,
    pub deletable: bool,
    /// A card's details, for a card.
    pub card: Option<Card>,
    /// An identity's details, for an identity.
    pub identity: Option<Identity>,
    /// An SSH key's, for an SSH key.
    pub ssh_key: Option<SshKey>,
    /// Its attachments (docs/core.md, "Attachments"), each with its file's key.
    pub attachments: Vec<Attachment>,
    /// A login's passkey, if it has one (docs/core.md, "Passkeys").
    pub passkey: Option<Passkey>,
    /// A bank account's, driver's license's, or passport's values.
    pub details: Vec<Detail>,
    /// What an edit keeps that the app doesn't show or change.
    pub kept: Kept,
}

/// A login's passkey (docs/core.md, "Passkeys"): upstream's first
/// `fido2Credentials` entry, what lists and signing need. Its private key
/// stays encrypted until it signs (`Passkey::key`). One whose values don't
/// open is left out, and the rest of its login still opens.
pub struct Passkey {
    /// As stored: a UUID, or `b64.` and the ID in base64url.
    pub credential_id: String,
    pub rp_id: String,
    pub rp_name: Option<String>,
    /// The site's ID for the account, base64url, as stored.
    pub user_handle: Option<String>,
    pub user_name: Option<String>,
    pub user_display_name: Option<String>,
    pub counter: u32,
    pub discoverable: bool,
    pub creation_date: Option<String>,
    /// ES256 (ECDSA on P-256), the only kind the core signs with.
    pub usable: bool,
    key_value: Option<EncString>,
    item_key: SymmetricKey,
}

impl Passkey {
    /// Its private key, decrypted only now.
    pub fn key(&self) -> Result<crate::passkey::Key> {
        if !self.usable {
            return Err(Error::Refused("it's a passkey that isn't ES256 (P-256)"));
        }
        let stored = self
            .key_value
            .as_ref()
            .ok_or(Error::Damaged("a passkey with no key"))?
            .decrypt_string(&self.item_key)?;
        crate::passkey::Key::from_stored(&stored)
    }

    /// The stored credentials with this one's counter set to `counter`,
    /// encrypted under the item's key, for saving.
    pub fn with_counter(&self, stored: &serde_json::Value, counter: u32) -> serde_json::Value {
        let mut stored = stored.clone();
        if let Some(first) = stored.get_mut(0) {
            first["counter"] = serde_json::Value::String(
                self.item_key.encrypt_str(&counter.to_string()).to_string(),
            );
        }
        stored
    }

    /// Opens the first credential of `stored`, upstream's encrypted
    /// `fido2Credentials`; None if there's none or it doesn't open.
    fn open(stored: Option<&serde_json::Value>, key: &SymmetricKey) -> Option<Passkey> {
        let first = stored?.as_array()?.first()?;
        let text = |name: &str| -> Option<Option<String>> {
            match first.get(name).and_then(|v| v.as_str()) {
                None => Some(None),
                Some(raw) => EncString::parse(raw)
                    .and_then(|e| e.decrypt_string(key))
                    .ok()
                    .map(|v| Some(v.as_str().to_owned())),
            }
        };
        let credential_id = text("credentialId")??;
        let rp_id = text("rpId")??;
        let kind = (text("keyType")?, text("keyAlgorithm")?, text("keyCurve")?);
        let usable = kind
            == (
                Some("public-key".to_owned()),
                Some("ECDSA".to_owned()),
                Some("P-256".to_owned()),
            );
        let counter = match text("counter")? {
            None => 0,
            Some(n) => n.trim().parse().ok()?,
        };
        Some(Passkey {
            credential_id,
            rp_id,
            rp_name: text("rpName")?,
            user_handle: text("userHandle")?,
            user_name: text("userName")?,
            user_display_name: text("userDisplayName")?,
            counter,
            discoverable: text("discoverable")?.is_some_and(|d| d.eq_ignore_ascii_case("true")),
            creation_date: first
                .get("creationDate")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            usable,
            key_value: match first.get("keyValue").and_then(|v| v.as_str()) {
                None => None,
                Some(raw) => Some(EncString::parse(raw).ok()?),
            },
            item_key: key.clone(),
        })
    }
}

/// What an attachment whose name or key doesn't open is called.
pub const CANT_OPEN_ATTACHMENT: &str = "An attachment that can't be opened";

/// An attachment: its file's name and size, and the key its file opens with.
/// The file itself stays on the server until someone saves it. One whose
/// name or key doesn't open is still listed, with no key, so the rest of its
/// item still opens.
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub size: Option<u64>,
    pub key: Option<SymmetricKey>,
}

/// An SSH key (docs/core.md, "Items"): its public key and fingerprint, and
/// its private key, a secret like a password. The web vault makes and edits
/// them; the apps show and copy them.
#[derive(Clone, Default)]
pub struct SshKey {
    pub public_key: Option<String>,
    pub fingerprint: Option<String>,
    pub private_key: Option<Zeroizing<String>>,
}

/// A card's, an identity's, or an SSH key's values the app copies or
/// reveals, by name: the secrets ("cardNumber", "cardCode", "ssn",
/// "passportNumber", "licenseNumber", "sshPrivateKey"), and the plain ones
/// (`PLAIN_DETAILS`).
pub const SECRET_DETAILS: [&str; 6] = [
    "cardNumber",
    "cardCode",
    "ssn",
    "passportNumber",
    "licenseNumber",
    "sshPrivateKey",
];
pub const PLAIN_DETAILS: [&str; 10] = [
    "sshPublicKey",
    "sshFingerprint",
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
    /// Whether a value, by the name the app reveals and copies it with
    /// ("password", "field:2", "cardNumber", "bankAccount.iban"), is one of
    /// the item's secrets, which someone whose shared folder hides
    /// passwords is never given (docs/core.md, "Folders and permissions").
    /// Only the values known to be plain aren't: the username, the notes, a
    /// card's, an identity's, or an SSH key's plain details
    /// (`PLAIN_DETAILS`), a newer kind's plain ones, and plain text or
    /// yes-or-no custom fields. Anything else, a name added later included,
    /// is a secret.
    pub fn is_secret(&self, what: &str) -> bool {
        let plain = match what {
            "username" | "notes" => true,
            what if PLAIN_DETAILS.contains(&what) => true,
            what if what.contains('.') => self.details.iter().any(|d| d.key == what && !d.secret),
            other => other
                .strip_prefix("field:")
                .and_then(|i| i.parse::<usize>().ok())
                .and_then(|i| self.fields.get(i))
                .is_some_and(|f| !f.hidden),
        };
        !plain
    }

    /// One of a card's or an identity's values (`SECRET_DETAILS`,
    /// `PLAIN_DETAILS`); None if it hasn't that one.
    pub fn detail(&self, what: &str) -> Option<Zeroizing<String>> {
        let text = |t: &Option<String>| t.clone().filter(|t| !t.is_empty()).map(Zeroizing::new);
        let secret = |t: &Option<Zeroizing<String>>| {
            t.as_ref()
                .filter(|t| !t.is_empty())
                .map(|t| Zeroizing::new(t.as_str().to_owned()))
        };
        if let Some(key) = &self.ssh_key {
            return match what {
                "sshPrivateKey" => secret(&key.private_key),
                "sshPublicKey" => text(&key.public_key),
                "sshFingerprint" => text(&key.fingerprint),
                _ => None,
            };
        }
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
#[derive(Clone, Default)]
pub struct Card {
    pub cardholder_name: Option<String>,
    pub brand: Option<String>,
    pub exp_month: Option<String>,
    pub exp_year: Option<String>,
    pub number: Option<Zeroizing<String>>,
    pub code: Option<Zeroizing<String>>,
}

impl Card {
    /// "Visa, *1111": the brand and the number's last 4 digits, as lists
    /// show a card. A number of 4 digits or fewer shows none of them.
    pub fn summary(&self) -> String {
        let last4: Option<String> = self.number.as_ref().and_then(|n| {
            let digits: Vec<char> = n.chars().filter(char::is_ascii_digit).collect();
            (digits.len() > 4).then(|| digits[digits.len() - 4..].iter().collect())
        });
        match (self.brand.as_deref().filter(|b| !b.is_empty()), last4) {
            (Some(brand), Some(last4)) => format!("{brand}, *{last4}"),
            (Some(brand), None) => brand.to_owned(),
            (None, Some(last4)) => format!("*{last4}"),
            (None, None) => String::new(),
        }
    }

    /// "07/2029", or what there is of it.
    pub fn expiration(&self) -> Option<String> {
        let month = self
            .exp_month
            .as_deref()
            .filter(|m| !m.trim().is_empty())
            .map(|m| format!("{:0>2}", m.trim()));
        match (
            month,
            self.exp_year.as_deref().filter(|y| !y.trim().is_empty()),
        ) {
            (Some(m), Some(y)) => Some(format!("{m}/{y}")),
            (Some(m), None) => Some(m),
            (None, Some(y)) => Some(y.to_owned()),
            (None, None) => None,
        }
    }
}

/// An identity's details (docs/core.md, "Items"). The Social Security,
/// passport, and license numbers are secrets, like a password.
#[derive(Clone, Default)]
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
pub(crate) fn now_iso8601() -> String {
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
    /// A secure note; else a login, unless it's a card or an identity.
    pub note: bool,
    /// A card's details, for a card.
    pub card: Option<Card>,
    /// An identity's details, for an identity.
    pub identity: Option<Identity>,
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
    /// A new item from a template (templates.rs): its id and name, which go
    /// with the item as the web vault records them. An edit keeps whatever
    /// template the item came from.
    pub template: Option<(String, String)>,
}

impl ItemDraft {
    /// What kind of item it makes.
    pub fn kind(&self) -> ItemKind {
        if self.card.is_some() {
            ItemKind::Card
        } else if self.identity.is_some() {
            ItemKind::Identity
        } else if self.note {
            ItemKind::SecureNote
        } else {
            ItemKind::Login
        }
    }

    /// A draft of an opened item, to edit.
    pub fn of(item: &Item) -> ItemDraft {
        let copy = |z: &Option<Zeroizing<String>>| {
            z.as_ref().map(|v| Zeroizing::new(v.as_str().to_owned()))
        };
        ItemDraft {
            note: item.kind == ItemKind::SecureNote,
            card: item.card.clone(),
            identity: item.identity.clone(),
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
                    boolean: f.boolean,
                })
                .collect(),
            folder_id: item.folder_id.clone(),
            organization_id: item.organization_id.clone(),
            favorite: item.favorite,
            template: None,
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
    /// What the person can do in each collection (shared folder).
    pub collection_grids: HashMap<String, Grid>,
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
        Vault::open(sync, user_key_from_password(sync, prelogin, password)?)
    }

    /// Whether `password` is this account's master password, checked the
    /// way unlocking checks it (docs/core.md, "Asking for the master password
    /// again"): it must open the kept protected user key to the key this
    /// vault is open with. Nothing is kept or sent.
    pub fn is_master_password(
        &self,
        sync: &Sync,
        prelogin: &Prelogin,
        password: &str,
    ) -> Result<bool> {
        match user_key_from_password(sync, prelogin, password) {
            Ok(user) => Ok(user == self.keys.user),
            Err(Error::WrongPassword) => Ok(false),
            Err(e) => Err(e),
        }
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
        let collection_grids = sync
            .collections
            .iter()
            .map(|c| (c.id.clone(), c.grid()))
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
            collection_grids,
            organizations,
            items,
            damaged,
            keys,
        })
    }

    /// What the person may do with an item (docs/core.md, "Folders and
    /// permissions"): everything with their own; with an organization's, the
    /// most open of its shared folders they have, as the server decides,
    /// held to what the server says of the item itself. An organization's
    /// item in none of the person's shared folders (an admin's view of one
    /// with no folder) goes by the item's own fields.
    pub fn grid(&self, item: &Item) -> Grid {
        if item.organization_id.is_none() {
            return Grid::ALL;
        }
        let folders = self.item_grids(item);
        let mut grid = folders.iter().copied().reduce(Grid::or).unwrap_or(Grid {
            see_passwords: item.view_password,
            create: false,
            edit: item.editable,
            delete: item.deletable,
            export: false,
            share: item.editable && item.view_password,
            print: item.view_password,
            view_history: true,
            manage: false,
        });
        grid.edit &= item.editable;
        grid.see_passwords &= item.view_password;
        grid.delete &= item.deletable;
        grid
    }

    /// The person's grid in each of an item's shared folders they have.
    fn item_grids(&self, item: &Item) -> Vec<Grid> {
        item.collection_ids
            .iter()
            .filter_map(|c| self.collection_grids.get(c).copied())
            .collect()
    }

    /// Whether the person may clear an item's history, or delete one of its
    /// versions: as the server decides, view item history and delete in the
    /// same shared folder (or their own item).
    pub fn may_clear_history(&self, item: &Item) -> bool {
        if item.organization_id.is_none() {
            return true;
        }
        let folders = self.item_grids(item);
        if folders.is_empty() {
            return item.deletable;
        }
        folders.iter().any(|g| g.view_history && g.delete)
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
        self.encrypt_item_with(draft, editing, None)
    }

    /// The key an item's values are encrypted with: its own item key if it
    /// has one, else its owner's (the organization's, or the user's).
    pub fn value_key<'a>(
        &'a self,
        draft: &ItemDraft,
        editing: Option<&'a Item>,
    ) -> Result<&'a SymmetricKey> {
        if let Some(key) = editing.and_then(|i| i.kept.item_key.as_ref()) {
            return Ok(key);
        }
        match &draft.organization_id {
            Some(org) => self
                .keys
                .organizations
                .get(org)
                .ok_or(Error::Damaged("that organization's key doesn't open")),
            None => Ok(&self.keys.user),
        }
    }

    /// `encrypt_item`, with the login's passkeys (`fido2Credentials`, already
    /// encrypted) in place of the ones it has: a passkey's raised counter.
    pub fn encrypt_item_with(
        &self,
        draft: &ItemDraft,
        editing: Option<&Item>,
        passkeys: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        use serde_json::json;
        if let Some(item) = editing {
            if !matches!(
                item.kind,
                ItemKind::Login | ItemKind::SecureNote | ItemKind::Card | ItemKind::Identity
            ) {
                return Err(Error::Refused(
                    "the app edits logins, notes, cards, and identities; edit SSH keys in the web vault",
                ));
            }
            if item.kind != draft.kind() {
                return Err(Error::Refused("an item stays the kind it is"));
            }
            if item.organization_id != draft.organization_id {
                return Err(Error::Refused(
                    "moving an item between owners is for the web vault",
                ));
            }
        }
        let kept = editing.map(|i| &i.kept);
        let key = self.value_key(draft, editing)?;
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
            if draft.kind() == ItemKind::Login && !old.is_empty() && old != new {
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
                    "type": f.kind(),
                    "linkedId": null,
                })
            })
            .collect();
        // A new item from a template records it, as the web vault does
        // (passbundle-clients, templates.ts): a text field after the rest.
        if let (None, Some((id, name))) = (editing, &draft.template) {
            fields.push(json!({
                "name": enc(crate::templates::TEMPLATE_FIELD),
                "value": enc(&crate::templates::marker(id, name)),
                "type": 0,
                "linkedId": null,
            }));
        }
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
            "type": match draft.kind() {
                ItemKind::SecureNote => 2,
                ItemKind::Card => 3,
                ItemKind::Identity => 4,
                _ => 1,
            },
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
        let secret =
            |text: &Option<Zeroizing<String>>| enc_opt(text.as_deref().map(String::as_str));
        let plain = |text: &Option<String>| enc_opt(text.as_deref());
        if let Some(card) = &draft.card {
            body["card"] = json!({
                "cardholderName": plain(&card.cardholder_name),
                "brand": plain(&card.brand),
                "number": secret(&card.number),
                "expMonth": plain(&card.exp_month),
                "expYear": plain(&card.exp_year),
                "code": secret(&card.code),
            });
        } else if let Some(id) = &draft.identity {
            body["identity"] = json!({
                "title": plain(&id.title),
                "firstName": plain(&id.first_name),
                "middleName": plain(&id.middle_name),
                "lastName": plain(&id.last_name),
                "address1": plain(&id.address1),
                "address2": plain(&id.address2),
                "address3": plain(&id.address3),
                "city": plain(&id.city),
                "state": plain(&id.state),
                "postalCode": plain(&id.postal_code),
                "country": plain(&id.country),
                "company": plain(&id.company),
                "email": plain(&id.email),
                "phone": plain(&id.phone),
                "ssn": secret(&id.ssn),
                "username": plain(&id.username),
                "passportNumber": secret(&id.passport_number),
                "licenseNumber": secret(&id.license_number),
            });
        } else if draft.note {
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
                "fido2Credentials": passkeys.or_else(|| kept.and_then(|k| k.fido2_credentials.clone())),
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

/// The user key the master password opens: the master key from the KDF
/// the prelogin gave (checked first), stretched, opening the profile's
/// protected user key. A wrong password is `WrongPassword`.
fn user_key_from_password(
    sync: &Sync,
    prelogin: &Prelogin,
    password: &str,
) -> Result<SymmetricKey> {
    let kdf = Kdf::from_prelogin(prelogin)?;
    let master_key = MasterKey::derive(password, &sync.profile.email, &kdf)?;
    let protected = EncString::parse(
        sync.profile
            .key
            .as_deref()
            .ok_or(Error::Malformed("a profile with no key".into()))?,
    )?;
    match protected.decrypt_key(&master_key.stretch()) {
        Err(Error::Refused(_)) => Err(Error::WrongPassword),
        other => other,
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
        let own = c.organization_id.is_none();
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
                boolean: f.kind == 2,
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
        // A file with no key of its own (very old ones) is under the item's key.
        let mut attachments = Vec::new();
        for a in c.attachments.iter().flatten() {
            let name = open_plain(&a.file_name).ok().flatten();
            let file_key = match &a.key {
                Some(wrapped) => EncString::parse(wrapped)
                    .and_then(|k| k.decrypt_key(&key))
                    .ok(),
                None => Some(key.clone()),
            };
            let opens = name.is_some() && file_key.is_some();
            attachments.push(Attachment {
                id: a.id.clone(),
                name: name
                    .filter(|_| opens)
                    .unwrap_or_else(|| CANT_OPEN_ATTACHMENT.to_owned()),
                size: a.size.as_ref().and_then(|s| {
                    s.as_u64()
                        .or_else(|| s.as_str().and_then(|t| t.parse().ok()))
                }),
                key: file_key.filter(|_| opens),
            });
        }
        let ssh_key = match &c.ssh_key {
            Some(d) if ItemKind::from(c.kind) == ItemKind::SshKey => Some(SshKey {
                public_key: open_plain(&d.public_key)?,
                fingerprint: open_plain(&d.key_fingerprint)?,
                private_key: open_text(&d.private_key, &key)?,
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
        // A bank account's, driver's license's, or passport's values, each
        // under the item's key; one that doesn't open fails the item, as
        // any other value does.
        let mut details = Vec::new();
        let newer = match ItemKind::from(c.kind) {
            ItemKind::BankAccount => {
                Some(("bankAccount", &c.bank_account, &BANK_ACCOUNT_DETAILS[..]))
            }
            ItemKind::DriversLicense => Some((
                "driversLicense",
                &c.drivers_license,
                &DRIVERS_LICENSE_DETAILS[..],
            )),
            ItemKind::Passport => Some(("passport", &c.passport, &PASSPORT_DETAILS[..])),
            _ => None,
        };
        if let Some((part, Some(values), specs)) = newer {
            for (property, label, secret) in specs {
                let raw = values
                    .get(property)
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                if let Some(value) = open_text(&raw, &key)?.filter(|v| !v.is_empty()) {
                    details.push(Detail {
                        key: format!("{part}.{property}"),
                        label,
                        value,
                        secret: *secret,
                    });
                }
            }
        }
        let login = c.login.as_ref();
        let passkey = login
            .filter(|_| ItemKind::from(c.kind) == ItemKind::Login)
            .and_then(|l| Passkey::open(l.fido2_credentials.as_ref(), &key));
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
            creation_date: c.creation_date.clone(),
            // The least access where the server doesn't say (CipherData).
            editable: c.edit.unwrap_or(own),
            view_password: c.view_password.unwrap_or(own),
            deletable: c.permissions.as_ref().and_then(|p| p.delete).unwrap_or(own),
            card,
            identity,
            ssh_key,
            attachments,
            passkey,
            details,
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
        // Too short to show any of it; empty parts are left out.
        let bare = Card {
            brand: Some(String::new()),
            number: Some(Zeroizing::new("1234".into())),
            exp_month: Some(String::new()),
            exp_year: Some("2029".into()),
            ..Card::default()
        };
        assert_eq!(bare.summary(), "");
        assert_eq!(bare.expiration().as_deref(), Some("2029"));
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
