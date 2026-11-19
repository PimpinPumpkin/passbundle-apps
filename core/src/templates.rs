//! Item templates, as the web vault has them (passbundle-clients,
//! libs/common/src/vault/keykeeper/templates.ts and categories.ts): a kind
//! of item (a login or a note) with starting custom fields, such as a Wi-Fi
//! network's or a server's. An item made from one records it in a text field
//! named `keykeeper.template` ({"id", "name", "v": 1}); everything else is
//! ordinary custom fields, which every client stores and shows, and which an
//! edit keeps as they are (vault.rs, `Kept`).
//!
//! Custom templates are items too: a note with a text field named
//! `keykeeper.template-definition` ({"id", "base", "v": 1}, and "from" for a
//! copy an MSP sends its clients). Its name is the template's, and its custom
//! fields are the template's, values included as defaults.
//!
//! Typed fields (a date, a website, an email address, a phone number, a
//! number, a one-time code) are ordinary text or hidden fields too: the
//! definition says each one's type by its label ("types") and which date is
//! a new item's expiration date ("expires"), and an item made from the
//! template gets the same in its marker (`Typing`; DESIGN.md, "Where a
//! field's type is kept").
//!
//! Every item also shows as a category, with its jewel: its template's,
//! where the template has one of its own, else its kind's.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::vault::{Item, ItemKind, Vault};

/// The field a new item from a template records it in.
pub const TEMPLATE_FIELD: &str = "keykeeper.template";
/// The field that makes a note a custom template.
pub const DEFINITION_FIELD: &str = "keykeeper.template-definition";

/// The kind of a template's field: upstream's text, hidden (a secret), or
/// yes or no, or one a template gives a field beyond those: a date
/// ("YYYY-MM-DD"), a website, an email address, a phone number, or a number,
/// each kept in a text field, or a one-time code, whose key is kept in a
/// hidden one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    Text,
    Hidden,
    Boolean,
    Date,
    Url,
    Email,
    Phone,
    Number,
    Totp,
}

impl FieldKind {
    /// Upstream's field type it's kept as: 0 text, 1 hidden, 2 yes or no.
    pub fn stored(self) -> u8 {
        match self {
            FieldKind::Hidden | FieldKind::Totp => 1,
            FieldKind::Boolean => 2,
            _ => 0,
        }
    }

    /// One of the kinds a template gives a field, by its name in
    /// `types`; None for upstream's own and anything unknown.
    pub fn typed(name: &str) -> Option<FieldKind> {
        Some(match name {
            "date" => FieldKind::Date,
            "url" => FieldKind::Url,
            "email" => FieldKind::Email,
            "phone" => FieldKind::Phone,
            "number" => FieldKind::Number,
            "totp" => FieldKind::Totp,
            _ => return None,
        })
    }

    /// Its name in `types`, and the apps'.
    pub fn name(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Hidden => "hidden",
            FieldKind::Boolean => "boolean",
            FieldKind::Date => "date",
            FieldKind::Url => "url",
            FieldKind::Email => "email",
            FieldKind::Phone => "phone",
            FieldKind::Number => "number",
            FieldKind::Totp => "totp",
        }
    }

    /// Upstream's own kind for a field type (0, 1, or 2); None for linked.
    pub fn own(stored: u8) -> Option<FieldKind> {
        match stored {
            0 => Some(FieldKind::Text),
            1 => Some(FieldKind::Hidden),
            2 => Some(FieldKind::Boolean),
            _ => None,
        }
    }

    /// Whether it's one of the kinds a template adds.
    pub fn is_typed(self) -> bool {
        self.stored() != 2 && !matches!(self, FieldKind::Text | FieldKind::Hidden)
    }
}

/// A template's typed fields, by label, as its definition and the markers of
/// items made from it keep them: each one's kind (`types`), and the date
/// field that is a new item's expiration date (`expires`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Typing {
    pub types: BTreeMap<String, FieldKind>,
    pub expires: Option<String>,
}

/// At most this many typed fields, each label at most this long, as the web
/// vault reads them (templates.ts, `typingOf`).
const MOST_TYPED_FIELDS: usize = 100;
const LONGEST_LABEL: usize = 200;

impl Typing {
    /// The typing in a definition's or a marker's JSON, only what's well
    /// formed: a kind nobody knows is left out, so its field is what its own
    /// type says (a kind added later is text to an older app), and
    /// `expires` counts only when it names a date field.
    pub fn of_json(json: &serde_json::Map<String, serde_json::Value>) -> Typing {
        let mut types = BTreeMap::new();
        if let Some(serde_json::Value::Object(raw)) = json.get("types") {
            for (label, kind) in raw.iter().take(MOST_TYPED_FIELDS) {
                if label.is_empty() || label.chars().count() > LONGEST_LABEL {
                    continue;
                }
                if let Some(kind) = kind.as_str().and_then(FieldKind::typed) {
                    types.insert(label.clone(), kind);
                }
            }
        }
        let expires = json
            .get("expires")
            .and_then(|e| e.as_str())
            .filter(|e| types.get(*e) == Some(&FieldKind::Date))
            .map(str::to_owned);
        Typing { types, expires }
    }

    /// The typing of a template's fields.
    pub fn of_fields(fields: &[TemplateField]) -> Typing {
        let mut map = serde_json::Map::new();
        let mut types = serde_json::Map::new();
        for f in fields.iter().filter(|f| f.kind.is_typed()) {
            types.insert(f.label.clone(), f.kind.name().into());
            if f.kind == FieldKind::Date && f.expires && !map.contains_key("expires") {
                map.insert("expires".into(), f.label.clone().into());
            }
        }
        map.insert("types".into(), types.into());
        Typing::of_json(&map)
    }

    /// The keys a definition or a marker keeps it in: none when nothing's typed.
    pub fn keys(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut keys = serde_json::Map::new();
        if self.types.is_empty() {
            return keys;
        }
        keys.insert(
            "types".into(),
            self.types
                .iter()
                .map(|(label, kind)| (label.clone(), serde_json::Value::from(kind.name())))
                .collect::<serde_json::Map<_, _>>()
                .into(),
        );
        if let Some(expires) = &self.expires {
            keys.insert("expires".into(), expires.clone().into());
        }
        keys
    }

    /// A field's kind: the template's for it where that fits the field's own
    /// type (upstream's: 0 text, 1 hidden, 2 yes or no), else its own; None
    /// for a linked field.
    pub fn kind_of(&self, label: &str, stored: u8) -> Option<FieldKind> {
        let own = FieldKind::own(stored)?;
        Some(
            self.types
                .get(label)
                .copied()
                .filter(|kind| kind.stored() == stored)
                .unwrap_or(own),
        )
    }

    /// Whether a field is the expiration date: a date field `expires` names.
    pub fn is_expiration(&self, label: &str, stored: u8) -> bool {
        self.expires.as_deref() == Some(label)
            && self.kind_of(label, stored) == Some(FieldKind::Date)
    }
}

/// A definition's or a marker's value with this typing in place of what it
/// had, its other keys as they were; None when the value isn't one.
pub fn with_typing(value: &str, typing: &Typing) -> Option<String> {
    let mut json = match serde_json::from_str::<serde_json::Value>(value).ok()? {
        serde_json::Value::Object(map) => map,
        _ => return None,
    };
    json.remove("types");
    json.remove("expires");
    json.extend(typing.keys());
    Some(serde_json::Value::Object(json).to_string())
}

/// The kind of item a template builds on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Base {
    Login,
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TemplateField {
    pub label: String,
    pub kind: FieldKind,
    /// A custom template's default, if it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// A date field that is a new item's expiration date.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub expires: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: String,
    pub name: String,
    pub base: Base,
    /// Its category's name (`CATEGORIES`): its jewel and glyph in the apps.
    pub category: &'static str,
    pub fields: Vec<TemplateField>,
    /// For a custom template: the item that defines it, and its organization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// For a copy an MSP keeps up to date: what its definition says. Never
    /// shown as who sent it, since anyone who can edit the item could write it.
    #[serde(skip)]
    pub from: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub cipher_id: String,
    pub organization_id: Option<String>,
}

/// Templates to choose from, under a heading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Group {
    pub name: String,
    pub templates: Vec<Template>,
}

/// The web vault's categories (categories.ts): each kind of item, and the
/// templates that bring their own jewel. The apps draw each from its name.
pub const CATEGORIES: [&str; 13] = [
    "login",
    "note",
    "card",
    "identity",
    "sshKey",
    "bank",
    "wifi",
    "server",
    "license",
    "apiKey",
    "domain",
    "email",
    "certificate",
];

/// The templates the web vault has built in, in its order, with their
/// fields and the category each shows as (templates.ts, categories.ts).
pub fn built_in() -> Vec<Template> {
    use FieldKind::{Boolean, Date, Hidden, Number, Phone, Text, Url};
    let make = |id: &str, name: &str, base, category, fields: &[(&str, FieldKind)]| Template {
        id: id.to_owned(),
        name: name.to_owned(),
        base,
        category,
        fields: fields
            .iter()
            .map(|(label, kind)| TemplateField {
                label: (*label).to_owned(),
                kind: *kind,
                value: None,
                expires: false,
            })
            .collect(),
        source: None,
        from: None,
    };
    vec![
        make(
            "api-key",
            "API key",
            Base::Note,
            "apiKey",
            &[
                ("Key", Hidden),
                ("Secret", Hidden),
                ("Service URL", Url),
                ("Scopes", Text),
            ],
        ),
        make(
            "server",
            "Server",
            Base::Login,
            "server",
            &[
                ("Hostname", Text),
                ("Port", Number),
                ("Operating system", Text),
            ],
        ),
        make(
            "database",
            "Database",
            Base::Login,
            "server",
            &[
                ("Type", Text),
                ("Hostname", Text),
                ("Port", Number),
                ("Database name", Text),
            ],
        ),
        make(
            "wifi",
            "Wi-Fi network",
            Base::Note,
            "wifi",
            &[
                ("Network name", Text),
                ("Password", Hidden),
                ("Security type", Text),
            ],
        ),
        make(
            "router",
            "Router",
            Base::Login,
            "wifi",
            &[("Network name", Text), ("Wi-Fi password", Hidden)],
        ),
        make(
            "software-license",
            "Software license",
            Base::Note,
            "license",
            &[
                ("License key", Hidden),
                ("Licensed to", Text),
                ("Version", Text),
                ("Purchase date", Date),
            ],
        ),
        make(
            "domain",
            "Domain",
            Base::Login,
            "domain",
            &[
                ("Domain name", Text),
                ("Registrar", Text),
                ("Auto-renew", Boolean),
            ],
        ),
        make(
            "certificate",
            "Certificate",
            Base::Note,
            "certificate",
            &[
                ("Common name", Text),
                ("Issuer", Text),
                ("Private key", Hidden),
            ],
        ),
        make(
            "email-account",
            "Email account",
            Base::Login,
            "email",
            &[
                ("Incoming server", Text),
                ("Outgoing server", Text),
                ("Port", Number),
            ],
        ),
        make(
            "membership",
            "Membership",
            Base::Note,
            "identity",
            &[
                ("Organization", Text),
                ("Member ID", Text),
                ("Phone", Phone),
            ],
        ),
    ]
}

/// A JSON object with its keys in this order, as the web vault writes ours
/// (serde_json's own maps sort their keys).
fn in_order(pairs: &[(&str, serde_json::Value)]) -> String {
    let parts: Vec<String> = pairs
        .iter()
        .map(|(key, value)| format!("{}:{value}", serde_json::Value::from(*key)))
        .collect();
    format!("{{{}}}", parts.join(","))
}

/// `pairs`, then the typing's keys (`types`, `expires`), in that order.
fn with_keys(mut pairs: Vec<(&str, serde_json::Value)>, typing: &Typing) -> String {
    let mut keys = typing.keys();
    for key in ["types", "expires"] {
        if let Some(value) = keys.remove(key) {
            pairs.push((key, value));
        }
    }
    in_order(&pairs)
}

/// What a new item from a template records: {"id", "name", "v": 1}, as the
/// web vault writes it, and the template's typed fields (`Typing::keys`).
pub fn marker(id: &str, name: &str, typing: &Typing) -> String {
    with_keys(
        vec![("id", id.into()), ("name", name.into()), ("v", 1.into())],
        typing,
    )
}

/// A custom template's definition, as the web vault writes one (templates.ts,
/// `newTemplateDefinitionField`): its id, its base, and its typed fields.
pub fn definition(id: &str, base: Base, typing: &Typing) -> String {
    let base = match base {
        Base::Login => "login",
        Base::Note => "note",
    };
    with_keys(
        vec![("id", id.into()), ("base", base.into()), ("v", 1.into())],
        typing,
    )
}

/// A new custom template's definition, with a fresh id.
pub fn new_definition(base: Base, typing: &Typing) -> String {
    definition(&uuid::Uuid::new_v4().to_string(), base, typing)
}

/// A field's value as a JSON object, or None.
fn object(item: &Item, name: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let field = item.fields.iter().find(|f| f.name == name)?;
    match serde_json::from_str::<serde_json::Value>(field.value.as_str()).ok()? {
        serde_json::Value::Object(map) => Some(map),
        _ => None,
    }
}

/// The template an item came from: its id, its name, and its category, if
/// it has one of its own. A built-in one by its id; a custom one by the name
/// recorded when the item was made, so it shows even to people who can't
/// see the template itself.
pub fn template_of(item: &Item) -> Option<(String, String, Option<&'static str>)> {
    let marker = object(item, TEMPLATE_FIELD)?;
    let id = marker.get("id")?.as_str()?.to_owned();
    if let Some(t) = built_in().into_iter().find(|t| t.id == id) {
        return Some((t.id, t.name, Some(t.category)));
    }
    let name = marker.get("name")?.as_str()?.trim().to_owned();
    (!name.is_empty()).then_some((id, name, None))
}

/// A custom template's definition: its id, base, and the MSP it came from.
pub struct Definition {
    pub id: String,
    pub base: Base,
    pub from: Option<String>,
}

/// Whether an item is a custom template, and which: a valid definition
/// field (templates.ts, `templateDefinition`).
pub fn definition_of(item: &Item) -> Option<Definition> {
    let d = object(item, DEFINITION_FIELD)?;
    if d.get("v").and_then(|v| v.as_u64()) != Some(1) {
        return None;
    }
    let id = d.get("id")?.as_str()?;
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return None;
    }
    let base = match d.get("base")?.as_str()? {
        "login" => Base::Login,
        "note" => Base::Note,
        _ => return None,
    };
    let from = d
        .get("from")
        .and_then(|f| f.as_str())
        .map(str::trim)
        .filter(|f| !f.is_empty() && f.chars().count() <= 100)
        .map(str::to_owned);
    Some(Definition {
        id: id.to_owned(),
        base,
        from,
    })
}

/// A copy of a template an MSP keeps up to date in a client's Templates
/// shared folder: lists show it only when that folder is open.
pub fn is_msp_copy(item: &Item) -> bool {
    definition_of(item).is_some_and(|d| d.from.is_some())
}

/// One of the person's own templates or one of their organizations', not a
/// copy an MSP keeps up to date: what Item templates lists, and the other
/// lists leave out unless its folder or shared folder is open (DESIGN.md,
/// "The Item templates view").
pub fn is_own_template(item: &Item) -> bool {
    item.kind == ItemKind::SecureNote && definition_of(item).is_some_and(|d| d.from.is_none())
}

/// The typing that applies to an item's fields: a custom template's own
/// definition's, or else the marker's of the template the item came from.
pub fn typing_of(item: &Item) -> Typing {
    if definition_of(item).is_some() {
        return object(item, DEFINITION_FIELD)
            .map(|d| Typing::of_json(&d))
            .unwrap_or_default();
    }
    object(item, TEMPLATE_FIELD)
        .filter(|m| m.get("id").is_some_and(|id| id.is_string()))
        .map(|m| Typing::of_json(&m))
        .unwrap_or_default()
}

/// The text field holding an item's expiration date, as the template it
/// came from marks it (a template itself has none: its date is a default for
/// new items). `expiration::stored` reads it first.
pub fn expiration_field(item: &Item) -> Option<&crate::vault::Field> {
    if definition_of(item).is_some() {
        return None;
    }
    let label = typing_of(item).expires?;
    item.fields
        .iter()
        .zip(&item.kept.field_kinds)
        .find(|(f, (kind, _))| f.name == label && *kind == 0 && !f.reserved())
        .map(|(f, _)| f)
}

/// The key of a template's one-time code field: the hidden field at
/// `index` among the item's fields, when its kind is a one-time code.
pub fn field_code_key(item: &Item, index: usize) -> Option<&str> {
    let field = item.fields.get(index)?;
    let stored = item.kept.field_kinds.get(index)?.0;
    (field_kinds(item).get(index).copied().flatten() == Some(FieldKind::Totp) && stored == 1)
        .then(|| field.value.as_str())
        .filter(|key| !key.trim().is_empty())
}

/// Each of an item's fields' kinds, in its order: None for a linked field
/// and our own data.
pub fn field_kinds(item: &Item) -> Vec<Option<FieldKind>> {
    let typing = typing_of(item);
    item.fields
        .iter()
        .zip(&item.kept.field_kinds)
        .map(|(f, (kind, _))| {
            if f.reserved() {
                None
            } else {
                typing.kind_of(&f.name, *kind)
            }
        })
        .collect()
}

/// A new custom template made from an item ("Save as template"): logins for
/// a login, notes for a note, and the item's own custom fields by label and
/// kind (a template's typed ones included, and its expiration date), never
/// their values, which belong to that item. None for an item that's already
/// a template, or a kind templates don't cover (templates.ts,
/// `templateFieldsFromItem`).
pub fn from_item(item: &Item) -> Option<(Base, Vec<TemplateField>)> {
    let base = match item.kind {
        ItemKind::Login => Base::Login,
        ItemKind::SecureNote => Base::Note,
        _ => return None,
    };
    if definition_of(item).is_some() {
        return None;
    }
    let typing = typing_of(item);
    let fields = item
        .fields
        .iter()
        .zip(&item.kept.field_kinds)
        .filter(|(f, (kind, _))| f.edited_by_the_app(*kind))
        .filter_map(|(f, (kind, _))| {
            let field_kind = typing.kind_of(&f.name, *kind)?;
            Some(TemplateField {
                label: f.name.clone(),
                kind: field_kind,
                value: None,
                expires: typing.is_expiration(&f.name, *kind),
            })
        })
        .collect();
    Some((base, fields))
}

/// The category an item shows as (categories.ts, `categoryOfItem`): its
/// built-in template's, if that has one of its own, else its kind's.
pub fn category(item: &Item) -> &'static str {
    if let Some((_, _, Some(category))) = template_of(item) {
        return category;
    }
    match item.kind {
        ItemKind::Login => "login",
        ItemKind::SecureNote => "note",
        ItemKind::Card => "card",
        ItemKind::Identity | ItemKind::DriversLicense | ItemKind::Passport => "identity",
        ItemKind::SshKey => "sshKey",
        ItemKind::BankAccount => "bank",
        ItemKind::Other(_) => "note",
    }
}

/// The custom template an item defines, if it's one: a note in use (not in
/// the trash), its name the template's, its fields (text, hidden, and yes
/// or no) the template's, with their values as defaults.
pub fn custom(item: &Item) -> Option<Template> {
    let definition = definition_of(item)?;
    if item.kind != ItemKind::SecureNote || item.deleted {
        return None;
    }
    let typing = typing_of(item);
    let fields = item
        .fields
        .iter()
        .zip(&item.kept.field_kinds)
        .filter(|(f, _)| !f.reserved())
        .filter_map(|(f, (stored, _))| {
            Some(TemplateField {
                label: f.name.clone(),
                kind: typing.kind_of(&f.name, *stored)?,
                value: Some(f.value.as_str().to_owned()).filter(|v| !v.is_empty()),
                expires: typing.is_expiration(&f.name, *stored),
            })
        })
        .collect();
    let name = item.name.trim();
    Some(Template {
        id: definition.id,
        name: if name.is_empty() {
            "Untitled template".to_owned()
        } else {
            name.to_owned()
        },
        base: definition.base,
        category: match definition.base {
            Base::Login => "login",
            Base::Note => "note",
        },
        fields,
        source: Some(Source {
            cipher_id: item.id.clone(),
            organization_id: item.organization_id.clone(),
        }),
        from: definition.from,
    })
}

/// Templates to choose from, grouped as the web vault's New item picker
/// groups them (templates.ts, `templateGroups`): by where they are, each
/// organization's (by the organization's name), then the person's own, then
/// the built-in ones. Who a template came from isn't shown: anyone who can
/// edit an item could write any name into it. A template an MSP copies into
/// many clients is offered once, however many of them the person sees: a copy
/// identical to one already offered in another organization (the MSP's own
/// template, if the person sees it, or the first copy) is left out. Empty
/// groups are left out. `may_use` says whether a template note's secrets may
/// leave the core now (`App::may_use`: it doesn't ask for the master password
/// again, or it was given).
pub fn groups(vault: &Vault, may_use: impl Fn(&Item) -> bool) -> Vec<Group> {
    let by_name = |a: &Template, b: &Template| a.name.to_lowercase().cmp(&b.name.to_lowercase());
    // A hidden field's default is a secret like any other: someone the
    // template's folder hides passwords from gets the field, empty, and so
    // does everyone while the template asks for the master password.
    let mut all: Vec<Template> = vault
        .items
        .iter()
        .filter_map(|item| {
            let mut template = custom(item)?;
            if !vault.grid(item).see_passwords || !may_use(item) {
                for field in &mut template.fields {
                    if field.kind.stored() == 1 {
                        field.value = None;
                    }
                }
            }
            Some(template)
        })
        .collect();
    let org_of = |t: &Template| t.source.as_ref().and_then(|s| s.organization_id.clone());
    let mut organizations: Vec<(&String, &String)> = vault.organizations.iter().collect();
    organizations.sort_by_key(|(_, name)| name.to_lowercase());
    let place = |t: &Template| {
        org_of(t)
            .and_then(|org| organizations.iter().position(|(id, _)| **id == org))
            .unwrap_or(organizations.len())
    };
    // Originals before copies, then by where they are, so the one offered is
    // the original when the person sees it.
    all.sort_by_key(|t| (t.from.is_some(), place(t)));
    let same = |a: &Template, b: &Template| {
        a.id == b.id && a.name == b.name && a.base == b.base && a.fields == b.fields
    };
    let mut offered: Vec<Template> = Vec::new();
    for t in all {
        let twin = offered
            .iter()
            .any(|o| org_of(o) != org_of(&t) && same(o, &t));
        if !(t.from.is_some() && twin) {
            offered.push(t);
        }
    }
    offered.sort_by(by_name);
    let custom = offered;
    let mut groups = Vec::new();
    for (id, name) in organizations {
        let templates: Vec<Template> = custom
            .iter()
            .filter(|t| t.source.as_ref().and_then(|s| s.organization_id.as_ref()) == Some(id))
            .cloned()
            .collect();
        if !templates.is_empty() {
            groups.push(Group {
                name: name.clone(),
                templates,
            });
        }
    }
    let mine: Vec<Template> = custom
        .iter()
        .filter(|t| {
            t.source
                .as_ref()
                .is_some_and(|s| s.organization_id.is_none())
        })
        .cloned()
        .collect();
    if !mine.is_empty() {
        groups.push(Group {
            name: "My templates".to_owned(),
            templates: mine,
        });
    }
    groups.push(Group {
        name: "Built-in".to_owned(),
        templates: built_in(),
    });
    // None that makes a kind of item an organization's "Remove card item
    // type" policy removes (restricted_types.rs).
    let allowed = |t: &Template| {
        !vault.restricted.refuses_new(match t.base {
            Base::Login => ItemKind::Login,
            Base::Note => ItemKind::SecureNote,
        })
    };
    for group in &mut groups {
        group.templates.retain(allowed);
    }
    groups.retain(|g| !g.templates.is_empty());
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_is_written_as_the_web_vault_writes_it() {
        let plain = Typing::default();
        assert_eq!(
            marker("wifi", "Wi-Fi network", &plain),
            r#"{"id":"wifi","name":"Wi-Fi network","v":1}"#
        );
        assert_eq!(
            marker("x", "Say \"hi\"", &plain),
            r#"{"id":"x","name":"Say \"hi\"","v":1}"#
        );
        // With the template's typed fields, read back the same.
        let api_key = built_in().into_iter().find(|t| t.id == "api-key").unwrap();
        let typing = Typing::of_fields(&api_key.fields);
        let written: serde_json::Value =
            serde_json::from_str(&marker("api-key", "API key", &typing)).unwrap();
        assert_eq!(
            written,
            serde_json::json!({ "id": "api-key", "name": "API key", "v": 1, "types": { "Service URL": "url" } })
        );
        assert_eq!(
            Typing::of_json(written.as_object().unwrap()),
            typing,
            "the marker carries what the template says"
        );
    }

    #[test]
    fn types_are_read_only_where_well_formed_and_fitting() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"types":{"Renews":"date","Site":"url","Code":"totp","Later":"fingerprint","Odd":7,"":"url"},"expires":"Renews"}"#,
        )
        .unwrap();
        let typing = Typing::of_json(json.as_object().unwrap());
        assert_eq!(typing.types.len(), 3);
        assert_eq!(typing.expires.as_deref(), Some("Renews"));
        // A kind nobody knows is text; one that doesn't fit the field's own
        // type is that type; a linked field has none.
        assert_eq!(typing.kind_of("Later", 0), Some(FieldKind::Text));
        assert_eq!(typing.kind_of("Renews", 0), Some(FieldKind::Date));
        assert_eq!(typing.kind_of("Renews", 1), Some(FieldKind::Hidden));
        assert_eq!(typing.kind_of("Code", 0), Some(FieldKind::Text));
        assert_eq!(typing.kind_of("Code", 1), Some(FieldKind::Totp));
        assert_eq!(typing.kind_of("Site", 3), None);
        // Expires names only a date.
        let not_a_date: serde_json::Value =
            serde_json::from_str(r#"{"types":{"Site":"url"},"expires":"Site"}"#).unwrap();
        assert_eq!(
            Typing::of_json(not_a_date.as_object().unwrap()).expires,
            None
        );
        // Written over a marker, keeping the rest of it.
        assert_eq!(
            with_typing(
                r#"{"id":"t","name":"N","v":1,"types":{"Old":"url"}}"#,
                &Typing::default()
            )
            .as_deref(),
            Some(r#"{"id":"t","name":"N","v":1}"#)
        );
        assert_eq!(with_typing("not json", &typing), None);
    }

    #[test]
    fn built_in_templates_type_their_fields_as_the_web_vault_does() {
        let kinds = |id: &str| -> Vec<(String, FieldKind)> {
            built_in()
                .into_iter()
                .find(|t| t.id == id)
                .unwrap()
                .fields
                .into_iter()
                .filter(|f| f.kind.is_typed())
                .map(|f| (f.label, f.kind))
                .collect()
        };
        assert_eq!(kinds("api-key"), [("Service URL".into(), FieldKind::Url)]);
        assert_eq!(kinds("server"), [("Port".into(), FieldKind::Number)]);
        assert_eq!(kinds("database"), [("Port".into(), FieldKind::Number)]);
        assert_eq!(
            kinds("software-license"),
            [("Purchase date".into(), FieldKind::Date)]
        );
        assert_eq!(kinds("email-account"), [("Port".into(), FieldKind::Number)]);
        assert_eq!(kinds("membership"), [("Phone".into(), FieldKind::Phone)]);
        assert!(kinds("wifi").is_empty());
    }

    #[test]
    fn every_built_in_template_has_a_category_the_apps_draw() {
        let all = built_in();
        assert_eq!(all.len(), 10);
        for t in &all {
            assert!(CATEGORIES.contains(&t.category), "{}", t.id);
            assert!(!t.fields.is_empty(), "{}", t.id);
        }
        // The web vault's: Wi-Fi and routers aquamarine, servers and
        // databases garnet, membership as an identity.
        let category = |id: &str| all.iter().find(|t| t.id == id).unwrap().category;
        assert_eq!(category("router"), "wifi");
        assert_eq!(category("database"), "server");
        assert_eq!(category("membership"), "identity");
        assert_eq!(
            all.iter()
                .find(|t| t.id == "domain")
                .unwrap()
                .fields
                .last()
                .unwrap()
                .kind,
            FieldKind::Boolean
        );
    }
}
