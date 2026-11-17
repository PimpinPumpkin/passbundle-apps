//! Importing a file (meta repo DESIGN.md, "Archive, export, and import in the apps"):
//! PassBundle's own exports, the browsers' own, 1Password's `.1pux` and 1Password
//! 8's CSV, and LastPass's CSV, read as upstream's importers for them read them
//! (passbundle-clients, `libs/importer/src/importers`), so a file comes in the
//! same here as in the web vault (`one_password.rs` says where the `.1pux`
//! differs).
//!
//! - **Nothing is saved until the person has seen what came in** (`read`
//!   answers what's in the file, and what's left out and why; `App` saves
//!   it once they say so).
//! - **PassBundle's own fields never come in from a file** unless they're
//!   ours and well formed (`guard_reserved_fields`, the web vault's
//!   `reserved-fields.ts`): from our own `.json`, a fill rule, an expiration
//!   date, reminder days, a template's marker, and a template's definition
//!   come back rewritten from what was read, never as an MSP's copy; every
//!   other field named like ours, and every one from another app, keeps its
//!   value under a visible name, "Imported keykeeper.fill".
//! - **A browser's export is sorted as the guided import sorts it**
//!   (`browser-import.ts`, `prepareImport`): a login already in the vault
//!   (same site, username, and password) or twice in the file is left out,
//!   one with no password is skipped, and one whose password differs from a
//!   login the person has for that site and username goes in "Check these:
//!   from <browser>", to compare. Items whose passwords the person can't see
//!   aren't compared at all.
//! - Every new item is encrypted on this device with the person's keys (or
//!   the organization's, for a shared folder), as any new item is.

use std::collections::HashMap;

mod lastpass;
mod one_password;
mod ssh_key;

use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::csv::{js_space, Table};
use crate::error::{Error, Result};
use crate::export::js_date;
use crate::vault::{
    Card, Identity, ItemKind, SshKey, Vault, BANK_ACCOUNT_DETAILS, DRIVERS_LICENSE_DETAILS,
    PASSPORT_DETAILS, RESERVED_FIELD_PREFIX,
};

/// The files the apps import, by the names the app picks them with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// PassBundle's (and the web vault's) `.json`, plain or password-protected.
    Json,
    /// PassBundle's `.csv`.
    Csv,
    /// A Chromium browser's Export (Chrome, Edge, Brave, Opera, Vivaldi).
    Chromium(&'static str),
    Firefox,
    /// Safari's, or the Passwords app's.
    Safari,
    /// 1Password 8's CSV.
    OnePasswordCsv,
    /// 1Password's `.1pux` (a zip), or the `export.data` in one.
    OnePassword1pux,
    /// LastPass's CSV.
    LastPassCsv,
}

impl Format {
    pub fn named(name: &str) -> Option<Format> {
        Some(match name {
            "json" => Format::Json,
            "csv" => Format::Csv,
            "chrome" => Format::Chromium("Google Chrome"),
            "edge" => Format::Chromium("Microsoft Edge"),
            "brave" => Format::Chromium("Brave"),
            "opera" => Format::Chromium("Opera"),
            "vivaldi" => Format::Chromium("Vivaldi"),
            "chromium" => Format::Chromium("Chromium"),
            "firefox" => Format::Firefox,
            "safari" => Format::Safari,
            "1password-csv" => Format::OnePasswordCsv,
            "1password-1pux" => Format::OnePassword1pux,
            "lastpass-csv" => Format::LastPassCsv,
            _ => return None,
        })
    }

    /// The browser a browser's export is from, by name, as its "Check
    /// these" folder names it; None for a password manager's file.
    pub fn browser(self) -> Option<&'static str> {
        match self {
            Format::Chromium(name) => Some(name),
            Format::Firefox => Some("Firefox"),
            Format::Safari => Some("Safari"),
            _ => None,
        }
    }
}

/// A custom field of an imported item.
pub struct Field {
    pub name: Option<String>,
    pub value: Option<Zeroizing<String>>,
    /// Upstream's field type: 0 text, 1 hidden, 2 yes or no, 3 linked.
    pub kind: u8,
    pub linked_id: Option<Value>,
}

/// An imported login's passkey: its values as upstream's export has them,
/// in its order, and when it was made.
pub struct Passkey {
    pub values: Vec<(&'static str, Zeroizing<String>)>,
    pub creation_date: Option<String>,
}

/// One item read from a file, before it's encrypted.
#[derive(Default)]
pub struct Item {
    /// Upstream's type: 1 login, 2 note, 3 card, 4 identity, 5 SSH key, 6
    /// bank account, 7 driver's license, 8 passport.
    pub kind: u8,
    pub name: String,
    pub notes: Option<Zeroizing<String>>,
    pub favorite: bool,
    pub reprompt: u8,
    /// Its folder, by its place in `Parsed::folders`.
    pub folder: Option<usize>,
    pub archived_date: Option<String>,
    pub username: Option<String>,
    pub password: Option<Zeroizing<String>>,
    pub totp: Option<Zeroizing<String>>,
    /// Websites, with how each is matched (null for the default).
    pub uris: Vec<(String, Option<u8>)>,
    pub passkeys: Vec<Passkey>,
    pub fields: Vec<Field>,
    /// Earlier passwords, newest first, with when each was last used.
    pub history: Vec<(Zeroizing<String>, Option<String>)>,
    pub secure_note: Option<Value>,
    pub card: Option<Card>,
    pub identity: Option<Identity>,
    pub ssh_key: Option<SshKey>,
    /// A bank account's, driver's license's, or passport's values, by
    /// upstream's property name.
    pub details: Vec<(&'static str, Zeroizing<String>)>,
}

/// Why something in a file doesn't come in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LeftOut {
    /// A browser's saved sign-in with no password (the guided import skips them).
    NoPassword,
    /// Already in the vault with the same password, or twice in the file.
    AlreadyThere,
    /// A kind of item PassBundle doesn't know.
    UnknownKind,
    /// Firefox's own account, which older versions export.
    FirefoxAccount,
    /// A file inside the export (an attachment or a document), which the
    /// apps don't import.
    Attachment,
    /// An SSH key PassBundle doesn't read (not Ed25519 or RSA, or damaged).
    SshKeyUnreadable,
    /// An item in the file that couldn't be read.
    Unreadable,
    /// A kind of item an organization's "Remove card item type" policy
    /// removes (restricted_types.rs), as the web vault's import leaves out.
    Restricted,
}

impl LeftOut {
    pub fn name(self) -> &'static str {
        match self {
            LeftOut::NoPassword => "noPassword",
            LeftOut::AlreadyThere => "alreadyThere",
            LeftOut::UnknownKind => "unknownKind",
            LeftOut::FirefoxAccount => "firefoxAccount",
            LeftOut::Attachment => "attachment",
            LeftOut::SshKeyUnreadable => "sshKeyUnreadable",
            LeftOut::Unreadable => "unreadable",
            LeftOut::Restricted => "restricted",
        }
    }
}

/// What a file holds, read and sorted, ready to be encrypted and saved.
pub struct Parsed {
    pub format: Format,
    /// The file was password-protected.
    pub protected: bool,
    pub items: Vec<Item>,
    /// Folders by name ("Work/Servers" inside "Work"), as the file has them.
    pub folders: Vec<String>,
    /// The items to compare, which go in `check_folder` (by their place in
    /// `items`).
    pub to_check: Vec<usize>,
    /// "Check these: from <browser>", when some are to compare.
    pub check_folder: Option<String>,
    pub left_out: Vec<(LeftOut, usize)>,
    /// Fields named like PassBundle's own that came in under a visible name.
    pub renamed: usize,
    /// How many of `items`, from the first, the server has already taken
    /// (a save refused partway): saving again sends only the rest.
    pub saved: usize,
}

impl Parsed {
    fn new(format: Format) -> Parsed {
        Parsed {
            format,
            protected: false,
            items: Vec::new(),
            folders: Vec::new(),
            to_check: Vec::new(),
            check_folder: None,
            left_out: Vec::new(),
            renamed: 0,
            saved: 0,
        }
    }

    fn leave_out(&mut self, why: LeftOut) {
        self.leave_out_some(why, 1);
    }

    fn leave_out_some(&mut self, why: LeftOut, count: usize) {
        if count == 0 {
            return;
        }
        match self.left_out.iter_mut().find(|(w, _)| *w == why) {
            Some((_, n)) => *n += count,
            None => self.left_out.push((why, count)),
        }
    }

    /// The folder of exactly this name, made once (LastPass's importer
    /// names its folders itself, without the ones they're inside).
    fn folder_named(&mut self, name: &str) -> usize {
        match self.folders.iter().position(|f| f == name) {
            Some(at) => at,
            None => {
                self.folders.push(name.to_owned());
                self.folders.len() - 1
            }
        }
    }

    /// The folder named so (upstream's `processFolder`): the same one each
    /// time, backslashes as slashes, no slash at the start, and each folder
    /// it's inside there too. None for a name with nothing in it.
    fn folder(&mut self, name: &str) -> Option<usize> {
        if blank(name) {
            return None;
        }
        let name = name.replace('\\', "/");
        let name = name.trim_start_matches('/').to_owned();
        let at = match self.folders.iter().position(|f| *f == name) {
            Some(at) => at,
            None => {
                self.folders.push(name.clone());
                self.folders.len() - 1
            }
        };
        let parts: Vec<&str> = name.split('/').collect();
        for i in (1..parts.len()).rev() {
            let parent = parts[..i].join("/");
            if !self.folders.contains(&parent) {
                self.folders.push(parent);
            }
        }
        Some(at)
    }

    /// How many items of each kind, by upstream's type number.
    pub fn kinds(&self) -> HashMap<u8, usize> {
        let mut kinds = HashMap::new();
        for item in &self.items {
            *kinds.entry(item.kind).or_default() += 1;
        }
        kinds
    }
}

/// What reading a file came to.
pub enum Read {
    Parsed(Parsed),
    /// A password-protected file: its password, please.
    NeedsPassword,
    /// That isn't the file's password.
    WrongPassword,
}

/// Blank, as upstream's `isNullOrWhitespace`.
fn blank(text: &str) -> bool {
    text.trim_matches(js_space).is_empty()
}

/// A value, or None when it's blank (upstream's `getValueOrDefault`).
fn value(text: Option<&str>) -> Option<&str> {
    text.filter(|t| !blank(t))
}

fn z(text: &str) -> Zeroizing<String> {
    Zeroizing::new(text.to_owned())
}

/// Upstream's `fixUri`: trimmed, `http://` in front of one with no scheme
/// that has a dot, and at most 1,000 characters.
fn fix_uri(uri: &str) -> String {
    let mut uri = uri.trim_matches(js_space).to_owned();
    if !uri.contains("://") && uri.contains('.') {
        uri = format!("http://{uri}");
    }
    if uri.encode_utf16().count() > 1000 {
        let mut cut = String::new();
        let mut units = 0;
        for c in uri.chars() {
            units += c.len_utf16();
            if units > 1000 {
                break;
            }
            cut.push(c);
        }
        uri = cut;
    }
    uri
}

/// Upstream's `makeUriArray`: each address fixed, the blank ones left out.
fn uris<'a>(list: impl IntoIterator<Item = &'a str>) -> Vec<(String, Option<u8>)> {
    list.into_iter()
        .map(fix_uri)
        .filter(|u| !blank(u))
        .map(|u| (u, None))
        .collect()
}

/// An address's host name, without a port, lowercase (upstream's
/// `Utils.getHostname`, near enough): None for one that has none.
fn hostname(url: &str) -> Option<String> {
    let url = url.trim_matches(js_space).to_lowercase();
    if url.starts_with("data:") || url.starts_with("about:") || url.starts_with("file:") {
        return None;
    }
    let rest = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host.starts_with('[') {
        host.split_once(']')
            .map_or(host, |(h, _)| h)
            .trim_start_matches('[')
    } else {
        host.rsplit_once(':')
            .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
            .map_or(host, |(h, _)| h)
    };
    (!host.is_empty()).then(|| host.to_owned())
}

/// Upstream's `nameFromUrl`: the host name without "www.".
fn name_from_url(url: &str) -> Option<String> {
    let host = hostname(url)?;
    Some(host.strip_prefix("www.").map(str::to_owned).unwrap_or(host))
}

/// Upstream's `cleanupCipher`: a name of "--" for one with none, and no
/// notes for blank ones.
fn clean(item: &mut Item) {
    if blank(&item.name) {
        item.name = "--".to_owned();
    }
    if item.notes.as_deref().is_some_and(|n| blank(n)) {
        item.notes = None;
    }
}

/// A new login, as upstream's `initLoginCipher` makes one.
fn login() -> Item {
    Item {
        kind: 1,
        ..Item::default()
    }
}

/// A card's brand from its number, as upstream's `getCardBrandByPatterns`
/// tells it (its patterns as they are, matched against the number as it
/// is); None for one it doesn't know.
fn card_brand(number: Option<&str>) -> Option<String> {
    let number = number.filter(|n| !n.trim_matches(js_space).is_empty())?;
    let digits = |n: &str| n.bytes().all(|b| b.is_ascii_digit());
    let after = |prefix: &str, then: usize| {
        number
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.len() == then && digits(rest))
    };
    let three = |prefix: &str, from: u32, to: u32| {
        number.strip_prefix(prefix).is_some_and(|rest| {
            rest.get(..3)
                .filter(|d| digits(d))
                .and_then(|d| d.parse::<u32>().ok())
                .is_some_and(|d| (from..=to).contains(&d))
        })
    };
    let brand = if number.starts_with('4') {
        "Visa"
    } else if (51..=55).any(|p| after(&p.to_string(), 14))
        || (2221..=2229).any(|p| after(&p.to_string(), 12))
        || (223..=229).any(|p| after(&p.to_string(), 13))
        || (23..=26).any(|p| after(&p.to_string(), 14))
        || (270..=271).any(|p| after(&p.to_string(), 13))
        || after("2720", 12)
    {
        "Mastercard"
    } else if number.starts_with("34") || number.starts_with("37") {
        "Amex"
    } else if number.starts_with("6011")
        || three("622", 126, 129)
        || three("622", 130, 199)
        || three("622", 200, 899)
        || three("622", 900, 925)
        || three("622", 644, 649)
        || number.starts_with("65")
    {
        "Discover"
    } else if number.starts_with("36") || (300..=305).any(|p| number.starts_with(&p.to_string())) {
        "Diners Club"
    } else if number.starts_with("3528")
        || number.starts_with("3529")
        || (353..=358).any(|p| {
            number
                .strip_prefix(&p.to_string())
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        })
    {
        "JCB"
    } else {
        return None;
    };
    Some(brand.to_owned())
}

/// A card's brand from the name another app gives it ("American Express",
/// "MasterCard", "visa"), as our fork's `cardBrandFromName` reads one.
fn card_brand_named(name: Option<&str>) -> Option<String> {
    let key: String = name?
        .to_lowercase()
        .chars()
        .filter(char::is_ascii_lowercase)
        .collect();
    let brand = match key.as_str() {
        "visa" | "visaelectron" => "Visa",
        "mastercard" | "mc" => "Mastercard",
        "amex" | "americanexpress" => "Amex",
        "discover" => "Discover",
        "diners" | "dinersclub" => "Diners Club",
        "jcb" => "JCB",
        "maestro" => "Maestro",
        "unionpay" | "chinaunionpay" => "UnionPay",
        "rupay" => "RuPay",
        _ => return None,
    };
    Some(brand.to_owned())
}

/// Reads a file of the format the person chose, with the password a
/// password-protected `.json` is under, if they gave one; sorted against
/// the vault for a browser's export. Nothing is saved. A `.1pux` is read as
/// the zip it is; every other file as the web vault reads one, as UTF-8
/// with anything that isn't read as the replacement character.
pub fn read(format: Format, file: &[u8], password: Option<&str>, vault: &Vault) -> Result<Read> {
    if format == Format::OnePasswordCsv && one_password::is_zip(file) {
        return Err(Error::Refused(
            "that's 1Password's .1pux export, not its CSV. Choose 1Password (.1pux) as its format",
        ));
    }
    let text = Zeroizing::new(match format {
        Format::OnePassword1pux => String::new(),
        _ => String::from_utf8_lossy(file).into_owned(),
    });
    let text = text.as_str();
    let mut parsed = match format {
        Format::Json => {
            let file: Value = serde_json::from_str(text.trim_matches(js_space))
                .map_err(|_| Error::Refused(NOT_JSON))?;
            let mut protected = false;
            let opened;
            let doc = if crate::export::is_protected(&file) {
                let Some(password) = password else {
                    return Ok(Read::NeedsPassword);
                };
                opened = match crate::export::open_protected(&file, password) {
                    Ok(plain) => plain,
                    Err(crate::export::Unopened::WrongPassword) => return Ok(Read::WrongPassword),
                    Err(crate::export::Unopened::Settings) => {
                        return Err(Error::Refused(UNKNOWN_SETTINGS))
                    }
                    Err(crate::export::Unopened::Damaged) => return Err(Error::Refused(DAMAGED)),
                };
                protected = true;
                serde_json::from_str::<Value>(&opened).map_err(|_| Error::Refused(DAMAGED))?
            } else {
                file
            };
            let mut parsed = json(&doc)?;
            parsed.protected = protected;
            parsed
        }
        Format::Csv => bitwarden_csv(&table(text)?)?,
        Format::Chromium(name) => chromium(&table(text)?, name, text)?,
        Format::Firefox => firefox(&table(text)?, text)?,
        Format::Safari => safari(&table(text)?, text)?,
        Format::OnePasswordCsv => one_password_csv(&table(text)?)?,
        Format::OnePassword1pux => one_password::read(file)?,
        Format::LastPassCsv => lastpass::read(&table(text)?)?,
    };
    // No item of a kind an organization's "Remove card item type" policy
    // removes, wherever it would go (upstream's import.service.ts).
    let before = parsed.items.len();
    parsed
        .items
        .retain(|i| !vault.restricted.refuses_new(ItemKind::from(i.kind)));
    for _ in parsed.items.len()..before {
        parsed.leave_out(LeftOut::Restricted);
    }
    // Our own fields come back only from our own .json.
    parsed.renamed = guard_reserved_fields(&mut parsed.items, format == Format::Json);
    if let Some(browser) = format.browser() {
        sort(&mut parsed, vault, browser);
    }
    Ok(Read::Parsed(parsed))
}

const NOT_JSON: &str = "that file isn't a .json export PassBundle can read. Check that it's the file you exported, and choose its format";
const DAMAGED: &str = "that file is damaged, so PassBundle can't open it. Export it again";
const UNKNOWN_SETTINGS: &str = "that file uses encryption settings PassBundle can't open";
const ACCOUNT_KEY: &str = "that file is encrypted with an account's own key, which only the web vault imports. Import it there, or export it again with a file password";
const UNCLOSED: &str = "that file has a quote that's never closed, so PassBundle can't tell where its values end. Export it again";
const NO_PASSWORDS: &str = "that export has no passwords in it";

fn table(text: &str) -> Result<Table> {
    Table::parse(text).map_err(|_| Error::Refused(UNCLOSED))
}

/// A browser's export with no passwords in it: its first line names a
/// password column, and nothing follows it (`isEmptyPasswordExport`).
fn empty_export(text: &str) -> bool {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split('\n').map(|l| l.trim_end_matches('\r'));
    let first = lines.next().unwrap_or("");
    let names_password = first.split(',').any(|column| {
        column
            .trim()
            .trim_start_matches('"')
            .trim_end_matches('"')
            .trim()
            .eq_ignore_ascii_case("password")
    });
    names_password && lines.all(|l| l.trim().is_empty())
}

/// PassBundle's `.json` (`BitwardenJsonImporter`): an individual export's
/// folders, or an organization's collections (whose items then go in no
/// folder), and each item as the export has it.
fn json(doc: &Value) -> Result<Parsed> {
    if doc["encrypted"] == true {
        return Err(Error::Refused(ACCOUNT_KEY));
    }
    let Some(items) = doc["items"].as_array() else {
        return Err(Error::Refused(NOT_JSON));
    };
    let mut parsed = Parsed::new(Format::Json);
    let mut folder_ids = HashMap::new();
    if doc["collections"].is_null() {
        for f in doc["folders"].as_array().into_iter().flatten() {
            if let (Some(id), Some(name)) = (f["id"].as_str(), f["name"].as_str()) {
                // As the file has them: two of the same name stay two.
                folder_ids.insert(id.to_owned(), parsed.folders.len());
                parsed.folders.push(name.to_owned());
            }
        }
    }
    for c in items {
        let kind = c["type"].as_u64().filter(|k| (1..=8).contains(k));
        let Some(kind) = kind else {
            parsed.leave_out(LeftOut::UnknownKind);
            continue;
        };
        let kind = kind as u8;
        let s = |v: &Value| v.as_str().map(str::to_owned);
        let secret = |v: &Value| v.as_str().map(z);
        let mut item = Item {
            kind,
            name: s(&c["name"]).unwrap_or_default(),
            notes: secret(&c["notes"]),
            favorite: c["favorite"] == true,
            reprompt: u8::from(c["reprompt"].as_u64() == Some(1)),
            folder: c["folderId"]
                .as_str()
                .and_then(|id| folder_ids.get(id).copied()),
            archived_date: c["archivedDate"].as_str().and_then(iso_date),
            ..Item::default()
        };
        for f in c["fields"].as_array().into_iter().flatten() {
            let kind = f["type"].as_u64().filter(|k| *k <= 3).unwrap_or(0) as u8;
            item.fields.push(Field {
                name: s(&f["name"]),
                value: secret(&f["value"]),
                kind,
                linked_id: Some(f["linkedId"].clone()).filter(|l| !l.is_null()),
            });
        }
        // At most 5 earlier passwords, as upstream keeps them.
        for h in c["passwordHistory"]
            .as_array()
            .into_iter()
            .flatten()
            .take(5)
        {
            if let Some(password) = h["password"].as_str() {
                item.history
                    .push((z(password), h["lastUsedDate"].as_str().and_then(iso_date)));
            }
        }
        match kind {
            1 => {
                let l = &c["login"];
                item.username = s(&l["username"]);
                item.password = secret(&l["password"]);
                item.totp = secret(&l["totp"]);
                for u in l["uris"].as_array().into_iter().flatten() {
                    if let Some(uri) = u["uri"].as_str() {
                        let matching = u["match"].as_u64().filter(|m| *m <= 5).map(|m| m as u8);
                        item.uris.push((uri.to_owned(), matching));
                    }
                }
                for p in l["fido2Credentials"].as_array().into_iter().flatten() {
                    item.passkeys.push(passkey(p));
                }
            }
            2 => {
                item.secure_note =
                    Some(json!({ "type": c["secureNote"]["type"].as_u64().unwrap_or(0) }));
            }
            3 => {
                let d = &c["card"];
                item.card = Some(Card {
                    cardholder_name: s(&d["cardholderName"]),
                    brand: s(&d["brand"]),
                    exp_month: s(&d["expMonth"]),
                    exp_year: s(&d["expYear"]),
                    number: secret(&d["number"]),
                    code: secret(&d["code"]),
                });
            }
            4 => {
                let d = &c["identity"];
                item.identity = Some(Identity {
                    title: s(&d["title"]),
                    first_name: s(&d["firstName"]),
                    middle_name: s(&d["middleName"]),
                    last_name: s(&d["lastName"]),
                    address1: s(&d["address1"]),
                    address2: s(&d["address2"]),
                    address3: s(&d["address3"]),
                    city: s(&d["city"]),
                    state: s(&d["state"]),
                    postal_code: s(&d["postalCode"]),
                    country: s(&d["country"]),
                    company: s(&d["company"]),
                    email: s(&d["email"]),
                    phone: s(&d["phone"]),
                    username: s(&d["username"]),
                    ssn: secret(&d["ssn"]),
                    passport_number: secret(&d["passportNumber"]),
                    license_number: secret(&d["licenseNumber"]),
                });
            }
            5 => {
                let d = &c["sshKey"];
                item.ssh_key = Some(SshKey {
                    public_key: s(&d["publicKey"]),
                    fingerprint: s(&d["keyFingerprint"]),
                    private_key: secret(&d["privateKey"]),
                });
            }
            _ => {
                let (part, specs) = newer_kind(kind);
                for (property, _, _) in specs {
                    if let Some(v) = c[part][*property].as_str().filter(|v| !v.is_empty()) {
                        item.details.push((*property, z(v)));
                    }
                }
            }
        }
        clean(&mut item);
        parsed.items.push(item);
    }
    Ok(parsed)
}

/// Upstream's newer kinds: where their values are in an item, and which.
fn newer_kind(kind: u8) -> (&'static str, &'static [(&'static str, &'static str, bool)]) {
    match kind {
        6 => ("bankAccount", &BANK_ACCOUNT_DETAILS),
        7 => ("driversLicense", &DRIVERS_LICENSE_DETAILS),
        _ => ("passport", &PASSPORT_DETAILS),
    }
}

/// A date the server takes ("2026-11-01T10:00:00.000Z"), from an ISO 8601
/// one; None for anything else.
fn iso_date(text: &str) -> Option<String> {
    let date = js_date(text);
    let b = date.as_bytes();
    let shaped = b.len() == 24
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b'T',
            13 | 16 => *c == b':',
            19 => *c == b'.',
            23 => *c == b'Z',
            _ => c.is_ascii_digit(),
        });
    shaped.then_some(date)
}

/// A passkey from an export (`Fido2CredentialExport.toView`): its counter a
/// whole number, and discoverable true or false, as the web vault keeps them.
fn passkey(p: &Value) -> Passkey {
    let mut values = Vec::new();
    for name in [
        "credentialId",
        "keyType",
        "keyAlgorithm",
        "keyCurve",
        "keyValue",
        "rpId",
        "userHandle",
        "userName",
        "counter",
        "rpName",
        "userDisplayName",
        "discoverable",
    ] {
        let raw = &p[name];
        let text = match name {
            "counter" => Some(
                raw.as_u64()
                    .or_else(|| raw.as_str().and_then(|c| c.trim().parse().ok()))
                    .unwrap_or(0)
                    .to_string(),
            ),
            "discoverable" => {
                Some((raw == true || raw.as_str().is_some_and(|d| d == "true")).to_string())
            }
            _ => raw.as_str().map(str::to_owned),
        };
        if let Some(text) = text {
            values.push((name, Zeroizing::new(text)));
        }
    }
    Passkey {
        values,
        creation_date: p["creationDate"].as_str().and_then(iso_date),
    }
}

fn get<'a>(record: &'a HashMap<String, Zeroizing<String>>, column: &str) -> Option<&'a str> {
    record.get(column).map(|v| v.as_str())
}

/// PassBundle's `.csv` (`BitwardenCsvImporter`).
fn bitwarden_csv(table: &Table) -> Result<Parsed> {
    if !(table.records.is_empty()
        || (table.columns.iter().any(|c| c == "name") && table.columns.iter().any(|c| c == "type")))
    {
        return Err(Error::Refused(
            "that isn't a .csv export from PassBundle or the web vault: it has no name and type columns. Choose its format",
        ));
    }
    let mut parsed = Parsed::new(Format::Csv);
    for r in &table.records {
        let folder = get(r, "folder").and_then(|f| parsed.folder(f));
        let mut item = Item {
            folder,
            favorite: value(get(r, "favorite")).unwrap_or("0") != "0",
            name: value(get(r, "name")).unwrap_or("--").to_owned(),
            notes: value(get(r, "notes")).map(z),
            reprompt: u8::from(leading_number(value(get(r, "reprompt")).unwrap_or("0")) == Some(1)),
            archived_date: value(get(r, "archivedDate")).and_then(iso_date),
            ..Item::default()
        };
        if let Some(fields) = value(get(r, "fields")) {
            for line in fields.split(['\n', '\r']) {
                if blank(line) {
                    continue;
                }
                let Some(at) = line.rfind(": ") else { continue };
                let rest = &line[at + 2..];
                item.fields.push(Field {
                    name: Some(line[..at].to_owned()),
                    value: (!rest.is_empty()).then(|| z(rest)),
                    kind: 0,
                    linked_id: None,
                });
            }
        }
        let either = |a: &str, b: &str| get(r, a).filter(|v| !v.is_empty()).or_else(|| get(r, b));
        match get(r, "type").map(str::to_lowercase).as_deref() {
            Some("note") => {
                item.kind = 2;
                item.secure_note = Some(json!({ "type": 0 }));
            }
            _ => {
                item.kind = 1;
                item.totp = value(either("login_totp", "totp")).map(z);
                item.username = value(either("login_username", "username")).map(str::to_owned);
                item.password = value(either("login_password", "password")).map(z);
                if let Some(list) = value(either("login_uri", "uri")) {
                    // One value holding several, as a row of CSV of its own.
                    let row = crate::csv::rows(list)
                        .ok()
                        .and_then(|r| r.into_iter().next());
                    if let Some(row) = row {
                        item.uris = uris(row.iter().map(|u| u.as_str()));
                    }
                }
            }
        }
        parsed.items.push(item);
    }
    Ok(parsed)
}

/// JavaScript's `parseInt(text, 10)`: the number its leading digits make.
fn leading_number(text: &str) -> Option<u64> {
    let text = text.trim_start_matches(js_space);
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// What's wrong with a file chosen as a browser's export that isn't one.
fn not_the_browsers(browser: &str) -> Error {
    Error::Refused(match browser {
        "Google Chrome" => "that isn't Chrome's password export. Check that it's the file Chrome saved, or choose the browser it came from",
        "Microsoft Edge" => "that isn't Edge's password export. Check that it's the file Edge saved, or choose the browser it came from",
        "Brave" => "that isn't Brave's password export. Check that it's the file Brave saved, or choose the browser it came from",
        "Opera" => "that isn't Opera's password export. Check that it's the file Opera saved, or choose the browser it came from",
        "Vivaldi" => "that isn't Vivaldi's password export. Check that it's the file Vivaldi saved, or choose the browser it came from",
        "Chromium" => "that isn't Chromium's password export. Check that it's the file Chromium saved, or choose the browser it came from",
        "Firefox" => "that isn't Firefox's password export. Check that it's the file Firefox saved, or choose the browser it came from",
        _ => "that isn't the Passwords app's or Safari's export. Check that it's the file it saved, or choose the browser it came from",
    })
}

/// A Chromium browser's Export (`ChromeCsvImporter`): `name,url,username,
/// password,note`, an Android app's login as `android://<hash>@<package>/`.
fn chromium(table: &Table, browser: &'static str, text: &str) -> Result<Parsed> {
    let has = |c: &str| table.columns.iter().any(|n| n == c);
    if !(has("name") && has("url") && has("username") && has("password")) || has("httpRealm") {
        return Err(not_the_browsers(browser));
    }
    if table.records.is_empty() && empty_export(text) {
        return Err(Error::Refused(NO_PASSWORDS));
    }
    let mut parsed = Parsed::new(Format::Chromium(browser));
    for r in &table.records {
        let url = get(r, "url").unwrap_or("");
        let package = android_package(url);
        let address = package.map_or_else(|| url.to_owned(), |p| format!("androidapp://{p}"));
        let name = get(r, "name").filter(|n| !n.is_empty()).or(package);
        let mut item = login();
        item.name = value(name).unwrap_or("--").to_owned();
        item.username = value(get(r, "username")).map(str::to_owned);
        item.password = value(get(r, "password")).map(z);
        item.uris = uris([address.as_str()]);
        item.notes = value(get(r, "note")).map(z);
        clean(&mut item);
        parsed.items.push(item);
    }
    Ok(parsed)
}

/// The package of an `android://<hash>@<package>/` address.
fn android_package(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("android://")?;
    let (_, after) = rest.split_once('@')?;
    Some(after.split('/').next().unwrap_or(after))
}

/// Firefox's Export (`FirefoxCsvImporter`): named after each login's site,
/// and without Firefox's own account that older versions export.
fn firefox(table: &Table, text: &str) -> Result<Parsed> {
    let has = |c: &str| table.columns.iter().any(|n| n == c);
    let firefoxs = ["httpRealm", "formActionOrigin", "guid", "timeCreated"]
        .iter()
        .any(|c| has(c));
    if !((has("url") || has("hostname")) && has("username") && has("password") && firefoxs) {
        return Err(not_the_browsers("Firefox"));
    }
    if table.records.is_empty() && empty_export(text) {
        return Err(Error::Refused(NO_PASSWORDS));
    }
    let mut parsed = Parsed::new(Format::Firefox);
    for r in &table.records {
        if get(r, "url") == Some("chrome://FirefoxAccounts") {
            parsed.leave_out(LeftOut::FirefoxAccount);
            continue;
        }
        let url = value(get(r, "url")).or_else(|| value(get(r, "hostname")));
        let mut item = login();
        item.name = url
            .and_then(name_from_url)
            .filter(|n| !blank(n))
            .unwrap_or_else(|| "--".to_owned());
        item.username = value(get(r, "username")).map(str::to_owned);
        item.password = value(get(r, "password")).map(z);
        item.uris = url.map(|u| uris([u])).unwrap_or_default();
        clean(&mut item);
        parsed.items.push(item);
    }
    Ok(parsed)
}

/// Safari's or the Passwords app's Export (`SafariCsvImporter`):
/// `Title,URL,Username,Password,Notes,OTPAuth`.
fn safari(table: &Table, text: &str) -> Result<Parsed> {
    let has = |c: &str| table.columns.iter().any(|n| n == c);
    if !(has("Title") && (has("URL") || has("Url")) && has("Username") && has("Password")) {
        return Err(not_the_browsers("Safari"));
    }
    if table.records.is_empty() && empty_export(text) {
        return Err(Error::Refused(NO_PASSWORDS));
    }
    let mut parsed = Parsed::new(Format::Safari);
    for r in &table.records {
        let mut item = login();
        item.name = value(get(r, "Title")).unwrap_or("--").to_owned();
        item.username = value(get(r, "Username")).map(str::to_owned);
        item.password = value(get(r, "Password")).map(z);
        item.uris = uris([get(r, "Url").or_else(|| get(r, "URL")).unwrap_or("")]);
        item.totp = value(get(r, "OTPAuth")).map(z);
        item.notes = value(get(r, "Notes")).map(z);
        clean(&mut item);
        parsed.items.push(item);
    }
    Ok(parsed)
}

/// 1Password 8's CSV (our fork's `OnePassword8CsvImporter`): `Title, Url,
/// Username, Password, OTPAuth, Favorite, Archived, Tags, Notes`. The
/// one-time password is the TOTP, a favorite is a favorite, an archived
/// item is archived, the first tag is the folder (every tag in the notes
/// when there's more than one), and a column it doesn't know is a field.
fn one_password_csv(table: &Table) -> Result<Parsed> {
    const TITLE: [&str; 1] = ["title"];
    const URL: [&str; 3] = ["url", "website", "urls"];
    const USERNAME: [&str; 1] = ["username"];
    const PASSWORD: [&str; 1] = ["password"];
    const TOTP: [&str; 3] = ["otpauth", "one-time password", "otp"];
    const FAVORITE: [&str; 1] = ["favorite"];
    const ARCHIVED: [&str; 1] = ["archived"];
    const TAGS: [&str; 1] = ["tags"];
    const NOTES: [&str; 2] = ["notes", "notesplain"];
    let mapped: Vec<&str> = [
        &TITLE[..],
        &URL[..],
        &USERNAME[..],
        &PASSWORD[..],
        &TOTP[..],
        &FAVORITE[..],
        &ARCHIVED[..],
        &TAGS[..],
        &NOTES[..],
    ]
    .concat();
    let lower: Vec<String> = table
        .columns
        .iter()
        .map(|c| c.trim().to_lowercase())
        .collect();
    let knows = |names: &[&str]| lower.iter().any(|c| names.contains(&c.as_str()));
    if !table.records.is_empty()
        && !(knows(&TITLE) && (knows(&URL) || knows(&USERNAME) || knows(&PASSWORD)))
    {
        return Err(Error::Refused(
            "that isn't 1Password 8's CSV export: it has no Title, Url, Username, or Password columns. Choose its format",
        ));
    }
    let is_true = |v: Option<&str>| {
        v.is_some_and(|v| {
            let v = v.trim().to_lowercase();
            v == "true" || v == "yes" || v == "1"
        })
    };
    let mut parsed = Parsed::new(Format::OnePasswordCsv);
    for r in &table.records {
        let values: HashMap<String, &str> = table
            .columns
            .iter()
            .filter_map(|c| r.get(c).map(|v| (c.trim().to_lowercase(), v.as_str())))
            .collect();
        let pick = |names: &[&str]| {
            names
                .iter()
                .find_map(|n| values.get(*n).copied().filter(|v| !blank(v)))
        };
        if values.values().all(|v| blank(v)) {
            continue;
        }
        let mut item = login();
        let url = pick(&URL);
        if let Some(url) = url {
            item.uris = uris(url.split(['\n', '\r']));
        }
        item.username = value(pick(&USERNAME)).map(str::to_owned);
        item.password = value(pick(&PASSWORD)).map(z);
        item.totp = pick(&TOTP).map(|t| t.trim()).filter(|t| !blank(t)).map(z);
        item.name = value(pick(&TITLE))
            .map(str::to_owned)
            .or_else(|| url.and_then(|_| item.uris.first().and_then(|(u, _)| name_from_url(u))))
            .unwrap_or_else(|| "--".to_owned());
        let mut notes = value(pick(&NOTES)).unwrap_or("").to_owned();
        item.favorite = is_true(pick(&FAVORITE));
        // Columns 1Password may add later come over as fields.
        for column in &table.columns {
            let lowered = column.trim().to_lowercase();
            if mapped.contains(&lowered.as_str()) {
                continue;
            }
            let Some(v) = r.get(column) else { continue };
            let hidden = ["password", "secret", "key"]
                .iter()
                .any(|w| column.to_lowercase().contains(w));
            key_value(&mut item, &mut notes, column, v, if hidden { 1 } else { 0 });
        }
        let tags: Vec<String> = {
            let mut tags: Vec<String> = Vec::new();
            for tag in pick(&TAGS).unwrap_or("").split([',', ';', '\n']) {
                let tag = tag.trim();
                if !tag.is_empty() && !tags.iter().any(|t| t == tag) {
                    tags.push(tag.to_owned());
                }
            }
            tags
        };
        if tags.len() > 1 {
            if !notes.is_empty() && !notes.ends_with('\n') {
                notes.push('\n');
            }
            notes.push_str(&format!("Tags: {}", tags.join(", ")));
        }
        item.notes = Some(z(&notes));
        // A login with nothing to sign in with is a note; a TOTP is something.
        if item.totp.is_none()
            && item.username.as_deref().is_none_or(blank)
            && item.password.as_deref().is_none_or(|p| blank(p))
            && item.uris.is_empty()
        {
            item.kind = 2;
            item.secure_note = Some(json!({ "type": 0 }));
        }
        clean(&mut item);
        if let Some(first) = tags.first() {
            item.folder = parsed.folder(first);
        }
        // 1Password says an item is archived, not when.
        if is_true(pick(&ARCHIVED)) {
            item.archived_date = Some(crate::vault::now_iso8601());
        }
        parsed.items.push(item);
    }
    Ok(parsed)
}

/// Upstream's `processKvp`: a field, or, for a long value or one on more
/// than one line, a line of the notes ("Key: value").
fn key_value(item: &mut Item, notes: &mut String, key: &str, v: &str, kind: u8) {
    if blank(v) {
        return;
    }
    let key = if blank(key) { "" } else { key };
    if v.encode_utf16().count() > 200 || v.trim_matches(js_space).contains(['\n', '\r']) {
        if !notes.is_empty() && !notes.ends_with('\n') {
            notes.push('\n');
        }
        let lines: Vec<&str> = v
            .split("\r\n")
            .flat_map(|l| l.split(['\r', '\n']))
            .collect();
        notes.push_str(&format!("{key}: {}\n", lines.join("\n")));
    } else {
        item.fields.push(Field {
            name: Some(key.to_owned()),
            value: Some(z(v)),
            kind,
            linked_id: None,
        });
    }
}

/// The site a login is for, as the guided import compares them: its first
/// address's host (with its port), lowercase, or its name.
fn site_of(uri: Option<&str>, name: &str) -> String {
    let Some(uri) = uri.filter(|u| !u.is_empty()) else {
        return name.trim().to_lowercase();
    };
    let lower = uri.trim().to_lowercase();
    if lower.starts_with("androidapp://") {
        return lower.trim_end_matches('/').to_owned();
    }
    let rest = lower.split_once("://").map_or(lower.as_str(), |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    authority
        .rsplit_once('@')
        .map_or(authority, |(_, h)| h)
        .to_owned()
}

fn key_of(site: &str, username: Option<&str>) -> String {
    format!(
        "{site}\u{0}{}",
        username.unwrap_or("").trim().to_lowercase()
    )
}

/// Sorts a browser's export against the vault (`prepareImport`).
fn sort(parsed: &mut Parsed, vault: &Vault, browser: &str) {
    let mut known: HashMap<String, Vec<Zeroizing<String>>> = HashMap::new();
    for item in &vault.items {
        // Not items whose passwords the person can't see: "already there"
        // or "a different password" would confirm or rule out a guess.
        if item.kind != ItemKind::Login || item.deleted || !vault.grid(item).see_passwords {
            continue;
        }
        let site = site_of(item.uris.first().map(String::as_str), &item.name);
        known
            .entry(key_of(&site, item.username.as_deref()))
            .or_default()
            .push(item.password.clone().unwrap_or_default());
    }
    let items = std::mem::take(&mut parsed.items);
    for item in items {
        if item.kind != 1 {
            continue;
        }
        let password = item.password.clone().unwrap_or_default();
        if password.is_empty() {
            parsed.leave_out(LeftOut::NoPassword);
            continue;
        }
        let site = site_of(item.uris.first().map(|(u, _)| u.as_str()), &item.name);
        let passwords = known
            .entry(key_of(&site, item.username.as_deref()))
            .or_default();
        if passwords.iter().any(|p| p.as_str() == password.as_str()) {
            parsed.leave_out(LeftOut::AlreadyThere);
            continue;
        }
        if !passwords.is_empty() {
            parsed.to_check.push(parsed.items.len());
        }
        passwords.push(password);
        parsed.items.push(item);
    }
    if !parsed.to_check.is_empty() {
        parsed.check_folder = Some(format!("Check these: from {browser}"));
    }
}

/// The visible name a field with one of our names gets (`visibleName`).
pub fn visible_name(name: &str) -> String {
    format!("Imported {}", name.trim())
}

/// Whether a field's name is one of ours (`isReservedFieldName`): any case,
/// any spaces around.
fn reserved(name: &str) -> bool {
    name.trim()
        .to_lowercase()
        .starts_with(RESERVED_FIELD_PREFIX)
}

/// Short page text: at most 200 characters, with no line breaks or other
/// control characters (reserved-fields.ts, `text`).
fn page_text(v: &Value) -> Option<&str> {
    let t = v.as_str()?;
    (t.encode_utf16().count() <= 200 && !t.chars().any(|c| (c as u32) < 0x20 || c as u32 == 0x7f))
        .then_some(t)
}

fn template_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A fill rule rewritten from only what a rule has (`wellFormedRule`), as
/// the extension writes them; None when it isn't one.
fn well_formed_rule(value: &str) -> Option<String> {
    let rule: Value = serde_json::from_str(value).ok()?;
    // `isFillRule`.
    if rule["v"].as_f64() != Some(1.0) || !rule["field"].is_object() || !rule["fill"].is_object() {
        return None;
    }
    if !(rule["host"].is_null() || rule["host"].is_string()) {
        return None;
    }
    let scheme = &rule["scheme"];
    if !(scheme.is_null() || scheme == "http" || scheme == "https") {
        return None;
    }
    let from = rule["fill"]["from"].as_str();
    match from {
        Some("field") if !rule["fill"]["name"].is_string() => return None,
        Some("field" | "username" | "password" | "totp" | "never") => {}
        _ => return None,
    }
    let mut field = Vec::new();
    for key in ["id", "name", "label"] {
        if let Some(part) = page_text(&rule["field"][key]).filter(|p| !p.is_empty()) {
            field.push(format!("{}:{}", Value::from(key), Value::from(part)));
        }
    }
    if field.is_empty() {
        return None;
    }
    let fill = if from == Some("field") {
        let name = page_text(&rule["fill"]["name"]).filter(|n| !n.is_empty() && !reserved(n))?;
        format!("{{\"from\":\"field\",\"name\":{}}}", Value::from(name))
    } else {
        format!("{{\"from\":{}}}", Value::from(from?))
    };
    let host = rule["host"].as_str().map(str::to_lowercase);
    if let Some(host) = &host {
        let ok = (1..=253).contains(&host.len())
            && host
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".:[]-".contains(&b));
        if !ok {
            return None;
        }
    }
    let mut out = String::from("{\"v\":1");
    if let Some(host) = host {
        out.push_str(&format!(",\"host\":{}", Value::from(host)));
    }
    if let Some(scheme) = scheme.as_str() {
        out.push_str(&format!(",\"scheme\":{}", Value::from(scheme)));
    }
    out.push_str(&format!(
        ",\"field\":{{{}}},\"fill\":{fill}}}",
        field.join(",")
    ));
    Some(out)
}

/// Reminder days rewritten the way the editor stores them (`parseRemindDays`).
fn well_formed_remind(value: &str) -> Option<String> {
    let text = value.trim_matches(js_space);
    if text.eq_ignore_ascii_case("none") {
        return Some("none".to_owned());
    }
    let mut days: Vec<u16> = Vec::new();
    for part in text.split(',') {
        let part = part.trim_matches(js_space);
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let day: u16 = part.parse().ok()?;
        if !days.contains(&day) {
            days.push(day);
        }
    }
    if days
        .iter()
        .any(|d| !(1..=crate::expiration::MOST_DAYS).contains(d))
    {
        return None;
    }
    days.sort_unstable_by(|a, b| b.cmp(a));
    Some(
        days.iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// A date "YYYY-MM-DD" that's a real day (`parseIsoDate`).
fn well_formed_date(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        && crate::expiration::Date::parse_iso(value).is_some_and(|d| d.iso() == value)
}

/// One of our fields as it may come back from our own export
/// (`wellFormed`), or None.
fn well_formed(field: &Field, seen: &[String]) -> Option<String> {
    let name = field.name.as_deref().unwrap_or("");
    let value = field.value.as_deref().map_or("", String::as_str);
    let once = !seen.iter().any(|s| s == name);
    match name {
        "keykeeper.fill" => (field.kind == 1).then(|| well_formed_rule(value)).flatten(),
        crate::vault::EXPIRES_FIELD => (once && well_formed_date(value)).then(|| value.to_owned()),
        crate::expiration::REMIND_FIELD => once.then(|| well_formed_remind(value)).flatten(),
        crate::templates::TEMPLATE_FIELD => once
            .then(|| {
                let marker: Value = serde_json::from_str(value).ok()?;
                let id = marker["id"].as_str().filter(|id| template_id(id))?;
                let name = page_text(&marker["name"]).filter(|n| !n.is_empty())?;
                Some(crate::templates::marker(id, name))
            })
            .flatten(),
        crate::templates::DEFINITION_FIELD => once
            .then(|| {
                let d: Value = serde_json::from_str(value).ok()?;
                if !d.is_object() || d["v"].as_f64() != Some(1.0) {
                    return None;
                }
                let id = d["id"].as_str().filter(|id| template_id(id))?;
                let base = d["base"]
                    .as_str()
                    .filter(|b| *b == "login" || *b == "note")?;
                // Never as an MSP's copy: that comes only from the MSP.
                Some(format!(
                    "{{\"id\":{},\"base\":{},\"v\":1}}",
                    Value::from(id),
                    Value::from(base)
                ))
            })
            .flatten(),
        _ => None,
    }
}

/// Makes imported items safe to keep (`guardReservedFields`): our fields
/// only from our own `.json` (`ours`) and well formed, rewritten; every
/// other field named like ours keeps its value under a visible name.
/// Answers how many were renamed.
pub fn guard_reserved_fields(items: &mut [Item], ours: bool) -> usize {
    let mut renamed = 0;
    for item in items {
        let mut seen: Vec<String> = Vec::new();
        for field in &mut item.fields {
            let name = field.name.clone().unwrap_or_default();
            if !reserved(&name) {
                continue;
            }
            if let Some(value) = ours.then(|| well_formed(field, &seen)).flatten() {
                seen.push(name);
                field.value = Some(Zeroizing::new(value));
                continue;
            }
            field.name = Some(visible_name(&name));
            renamed += 1;
        }
    }
    renamed
}

/// An imported item, encrypted as the server takes a new one (upstream's
/// cipher request): under the organization's key for a shared folder, else
/// the user key, with no item key of its own, as the apps make new items.
pub fn encrypt(vault: &Vault, item: &Item, organization_id: Option<&str>) -> Result<Value> {
    let key = vault.owner_key(organization_id)?;
    let enc = |text: &str| key.encrypt_str(text).to_string();
    let enc_opt = |text: Option<&str>| text.filter(|t| !t.is_empty()).map(enc);
    let secret = |text: &Option<Zeroizing<String>>| enc_opt(text.as_deref().map(String::as_str));
    let plain = |text: &Option<String>| enc_opt(text.as_deref());
    let fields: Vec<Value> = item
        .fields
        .iter()
        .map(|f| {
            json!({
                "name": enc_opt(f.name.as_deref()),
                "value": if f.kind == 3 { None } else { secret(&f.value) },
                "type": f.kind,
                "linkedId": f.linked_id,
            })
        })
        .collect();
    let history: Vec<Value> = item
        .history
        .iter()
        .map(|(password, when)| {
            json!({
                "password": enc(password),
                "lastUsedDate": when.clone().unwrap_or_else(crate::vault::now_iso8601),
            })
        })
        .collect();
    let mut body = json!({
        "type": item.kind,
        "organizationId": organization_id,
        "folderId": null,
        "name": enc(&item.name),
        "notes": secret(&item.notes),
        "favorite": item.favorite,
        "reprompt": item.reprompt,
        "key": null,
        "fields": if fields.is_empty() { Value::Null } else { fields.into() },
        "passwordHistory": if history.is_empty() { Value::Null } else { history.into() },
        "encryptedFor": vault.user_id,
        "lastKnownRevisionDate": null,
        "archivedDate": item.archived_date,
    });
    match item.kind {
        1 => {
            let passkeys: Vec<Value> = item
                .passkeys
                .iter()
                .map(|p| {
                    let mut entry = serde_json::Map::new();
                    for (name, value) in &p.values {
                        entry.insert((*name).to_owned(), Value::from(enc(value)));
                    }
                    entry.insert(
                        "creationDate".to_owned(),
                        Value::from(
                            p.creation_date
                                .clone()
                                .unwrap_or_else(crate::vault::now_iso8601),
                        ),
                    );
                    Value::Object(entry)
                })
                .collect();
            body["login"] = json!({
                "username": plain(&item.username),
                "password": secret(&item.password),
                "totp": secret(&item.totp),
                "uris": item.uris.iter().map(|(u, matching)| json!({
                    "uri": enc(u),
                    "uriChecksum": enc(&crate::vault::uri_checksum(u)),
                    "match": matching,
                })).collect::<Vec<_>>(),
                "fido2Credentials": if passkeys.is_empty() { Value::Null } else { passkeys.into() },
                "autofillOnPageLoad": null,
                "passwordRevisionDate": null,
            });
        }
        2 => {
            body["secureNote"] = item
                .secure_note
                .clone()
                .unwrap_or_else(|| json!({ "type": 0 }));
        }
        3 => {
            let card = item.card.clone().unwrap_or_default();
            body["card"] = json!({
                "cardholderName": plain(&card.cardholder_name),
                "brand": plain(&card.brand),
                "number": secret(&card.number),
                "expMonth": plain(&card.exp_month),
                "expYear": plain(&card.exp_year),
                "code": secret(&card.code),
            });
        }
        4 => {
            let id = item.identity.clone().unwrap_or_default();
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
        }
        5 => {
            let k = item.ssh_key.clone().unwrap_or_default();
            body["sshKey"] = json!({
                "privateKey": secret(&k.private_key),
                "publicKey": plain(&k.public_key),
                "keyFingerprint": plain(&k.fingerprint),
            });
        }
        kind => {
            let (part, _) = newer_kind(kind);
            let mut values = serde_json::Map::new();
            for (property, v) in &item.details {
                values.insert((*property).to_owned(), Value::from(enc(v)));
            }
            body[part] = Value::Object(values);
        }
    }
    Ok(body)
}

/// The organization that keeps this person's items out of Personal
/// (upstream's "Remove individual vault" policy, type 5), by name, from the
/// kept sync: its owners and admins aren't bound by it, as upstream has it.
pub fn keeps_items_out(sync: Option<&str>) -> Option<String> {
    let sync: Value = serde_json::from_str(sync?).ok()?;
    let organizations: Vec<&Value> = ["/profile/organizations", "/profile/organizationsNew"]
        .iter()
        .filter_map(|p| sync.pointer(p).and_then(Value::as_array))
        .flatten()
        .collect();
    sync["policies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["type"].as_u64() == Some(5) && p["enabled"] == true)
        .find_map(|p| {
            let org = organizations
                .iter()
                .find(|o| o["id"] == p["organizationId"])?;
            let exempt = matches!(org["type"].as_u64(), Some(0 | 1));
            (!exempt).then(|| {
                org["name"]
                    .as_str()
                    .unwrap_or("Your organization")
                    .to_owned()
            })
        })
}

/// The organization that doesn't let this person export their own items
/// (upstream's "Remove individual vault export" policy, type 10), by name.
/// Its owners and admins aren't bound by it. PassBundle's server doesn't
/// offer that policy, so it binds only on a server that does.
pub fn forbids_export(sync: Option<&str>) -> Option<String> {
    let sync: Value = serde_json::from_str(sync?).ok()?;
    let organizations = sync
        .pointer("/profile/organizations")
        .and_then(Value::as_array);
    sync["policies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["type"].as_u64() == Some(10) && p["enabled"] == true)
        .find_map(|p| {
            let org = organizations
                .into_iter()
                .flatten()
                .find(|o| o["id"] == p["organizationId"])?;
            let exempt = matches!(org["type"].as_u64(), Some(0 | 1));
            (!exempt).then(|| {
                org["name"]
                    .as_str()
                    .unwrap_or("Your organization")
                    .to_owned()
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, value: &str, kind: u8) -> Field {
        Field {
            name: Some(name.to_owned()),
            value: Some(z(value)),
            kind,
            linked_id: None,
        }
    }

    fn guarded(fields: Vec<Field>, ours: bool) -> (Vec<(String, String)>, usize) {
        let mut items = vec![Item {
            kind: 1,
            fields,
            ..Item::default()
        }];
        let renamed = guard_reserved_fields(&mut items, ours);
        (
            items[0]
                .fields
                .iter()
                .map(|f| {
                    (
                        f.name.clone().unwrap_or_default(),
                        f.value.as_deref().map_or("", String::as_str).to_owned(),
                    )
                })
                .collect(),
            renamed,
        )
    }

    #[test]
    fn our_fields_come_back_from_our_json_only_well_formed_and_rewritten() {
        let rule = r#"{"scheme":"https","fill":{"from":"field","name":"Member ID","extra":1},"host":"Bank.Example.TEST","v":1,"field":{"label":"Member number","id":"mid","junk":"x"}}"#;
        let (fields, renamed) = guarded(
            vec![
                field("keykeeper.fill", rule, 1),
                field("keykeeper.expires", "2027-02-28", 0),
                field("keykeeper.expires", "2027-03-01", 0),
                field("keykeeper.remind", " 7, 90,7 ,1", 0),
                field(
                    "keykeeper.template",
                    r#"{"name":"Wi-Fi","id":"wifi","v":1,"x":2}"#,
                    0,
                ),
                field(
                    "keykeeper.template-definition",
                    r#"{"id":"t-1","base":"note","from":"Acme MSP","v":1}"#,
                    0,
                ),
                field("Keykeeper.Fill", rule, 1),
                field("keykeeper.fill", rule, 0),
                field("keykeeper.expires", "2027-02-30", 0),
                field("PIN", "1234", 1),
            ],
            true,
        );
        assert_eq!(
            fields,
            [
                ("keykeeper.fill".into(), r#"{"v":1,"host":"bank.example.test","scheme":"https","field":{"id":"mid","label":"Member number"},"fill":{"from":"field","name":"Member ID"}}"#.into()),
                ("keykeeper.expires".into(), "2027-02-28".into()),
                // A second date is never ours.
                ("Imported keykeeper.expires".into(), "2027-03-01".into()),
                ("keykeeper.remind".into(), "90,7,1".into()),
                ("keykeeper.template".into(), r#"{"id":"wifi","name":"Wi-Fi","v":1}"#.into()),
                // Never as an MSP's copy.
                ("keykeeper.template-definition".into(), r#"{"id":"t-1","base":"note","v":1}"#.into()),
                ("Imported Keykeeper.Fill".into(), rule.into()),
                // A fill rule is a hidden field, or it isn't one.
                ("Imported keykeeper.fill".into(), rule.into()),
                ("Imported keykeeper.expires".into(), "2027-02-30".into()),
                ("PIN".into(), "1234".into()),
            ]
        );
        assert_eq!(renamed, 4);
    }

    #[test]
    fn from_anywhere_else_a_field_named_like_ours_is_a_visible_one() {
        let (fields, renamed) = guarded(
            vec![
                field(
                    " keykeeper.fill ",
                    r#"{"v":1,"field":{"id":"a"},"fill":{"from":"password"}}"#,
                    1,
                ),
                field("keykeeper.expires", "2027-02-28", 0),
            ],
            false,
        );
        assert_eq!(fields[0].0, "Imported keykeeper.fill");
        assert_eq!(fields[1].0, "Imported keykeeper.expires");
        assert_eq!(renamed, 2);
    }

    #[test]
    fn a_rule_that_isnt_one_never_comes_back_as_one() {
        for bad in [
            r#"{"v":2,"field":{"id":"a"},"fill":{"from":"password"}}"#,
            r#"{"v":1,"field":{},"fill":{"from":"password"}}"#,
            r#"{"v":1,"field":{"id":"a"},"fill":{"from":"everything"}}"#,
            r#"{"v":1,"field":{"id":"a"},"fill":{"from":"field","name":"keykeeper.fill"}}"#,
            r#"{"v":1,"field":{"id":"a\nb"},"fill":{"from":"password"}}"#,
            r#"{"v":1,"host":"evil example","field":{"id":"a"},"fill":{"from":"password"}}"#,
            r#"{"v":1,"scheme":"ftp","field":{"id":"a"},"fill":{"from":"password"}}"#,
            "not json",
        ] {
            assert_eq!(well_formed_rule(bad), None, "{bad}");
        }
        assert_eq!(
            well_formed_rule(r#"{"v":1,"field":{"name":"otp"},"fill":{"from":"totp"}}"#).as_deref(),
            Some(r#"{"v":1,"field":{"name":"otp"},"fill":{"from":"totp"}}"#)
        );
    }

    #[test]
    fn addresses_and_names_as_upstreams_importers_make_them() {
        assert_eq!(fix_uri(" example.test/login "), "http://example.test/login");
        assert_eq!(fix_uri("localhost"), "localhost");
        assert_eq!(fix_uri("https://a.test"), "https://a.test");
        assert_eq!(fix_uri(&"x".repeat(1200)).len(), 1000);
        assert_eq!(
            name_from_url("https://www.Example.test:8443/a").as_deref(),
            Some("example.test")
        );
        assert_eq!(name_from_url("about:blank"), None);
        assert_eq!(
            android_package("android://abc123==@com.example.app/"),
            Some("com.example.app")
        );
        assert_eq!(leading_number("1x"), Some(1));
        assert_eq!(leading_number("x"), None);
        assert_eq!(
            iso_date("2026-11-01T10:00:00.123456Z").as_deref(),
            Some("2026-11-01T10:00:00.123Z")
        );
        assert_eq!(
            iso_date("2026-11-01T10:00:00.123Z").as_deref(),
            Some("2026-11-01T10:00:00.123Z")
        );
        assert_eq!(iso_date("last week"), None);
    }

    #[test]
    fn an_export_with_only_its_column_names_has_no_passwords() {
        assert!(empty_export("\u{feff}name,url,username,password,note\r\n"));
        assert!(empty_export("\"url\",\"username\",\"password\"\n\n"));
        assert!(!empty_export(
            "name,url,username,password\nA,https://a.test,ana,pw"
        ));
        assert!(!empty_export("title,notes\n"));
    }

    #[test]
    fn folders_by_name_with_the_ones_theyre_in() {
        let mut parsed = Parsed::new(Format::Csv);
        assert_eq!(parsed.folder("\\Work\\Servers"), Some(0));
        assert_eq!(parsed.folder("Work"), Some(1));
        assert_eq!(parsed.folder("Work/Servers"), Some(0));
        assert_eq!(parsed.folder("  "), None);
        assert_eq!(parsed.folders, ["Work/Servers", "Work"]);
    }

    #[test]
    fn the_policies_that_bind_and_the_owners_and_admins_they_dont() {
        let sync = |role: u64, kind: u64| {
            json!({
                "profile": { "organizations": [{ "id": "o1", "name": "Acme", "type": role }] },
                "policies": [{ "organizationId": "o1", "type": kind, "enabled": true }],
            })
            .to_string()
        };
        assert_eq!(keeps_items_out(Some(&sync(2, 5))).as_deref(), Some("Acme"));
        assert_eq!(keeps_items_out(Some(&sync(1, 5))), None);
        assert_eq!(keeps_items_out(Some(&sync(2, 10))), None);
        assert_eq!(forbids_export(Some(&sync(2, 10))).as_deref(), Some("Acme"));
        assert_eq!(forbids_export(Some(&sync(0, 10))), None);
        assert_eq!(forbids_export(None), None);
    }
}
