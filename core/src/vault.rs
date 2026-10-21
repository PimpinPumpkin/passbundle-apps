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
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginData {
    pub username: Option<String>,
    pub password: Option<String>,
    pub totp: Option<String>,
    pub uris: Option<Vec<UriData>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UriData {
    pub uri: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FieldData {
    pub name: Option<String>,
    pub value: Option<String>,
    #[serde(rename = "type")]
    pub kind: u8,
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
    pub notes: Option<Zeroizing<String>>,
    pub password: Option<Zeroizing<String>>,
    pub totp: Option<Zeroizing<String>>,
    pub fields: Vec<Field>,
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
            organizations,
            items,
            damaged,
            keys,
        })
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
                let haystack = format!(
                    "{} {} {}",
                    i.name.to_lowercase(),
                    i.username.as_deref().unwrap_or("").to_lowercase(),
                    i.uris.join(" ").to_lowercase()
                );
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
        if let Some(login) = &c.login {
            username = plain(open_text(&login.username, &key)?);
            password = open_text(&login.password, &key)?;
            totp = open_text(&login.totp, &key)?;
            for uri in login.uris.iter().flatten() {
                if let Some(u) = open_text(&uri.uri, &key)? {
                    uris.push(u.as_str().to_owned());
                }
            }
        }
        let mut fields = Vec::new();
        for f in c.fields.iter().flatten() {
            fields.push(Field {
                name: plain(open_text(&f.name, &key)?).unwrap_or_default(),
                value: open_text(&f.value, &key)?.unwrap_or_else(|| Zeroizing::new(String::new())),
                hidden: f.kind == 1,
            });
        }
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
            notes,
            password,
            totp,
            fields,
        })
    }
}
