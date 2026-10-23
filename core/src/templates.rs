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
//! Every item also shows as a category, with its jewel: its template's,
//! where the template has one of its own, else its kind's.

use serde::Serialize;

use crate::vault::{Item, ItemKind, Vault};

/// The field a new item from a template records it in.
pub const TEMPLATE_FIELD: &str = "keykeeper.template";
/// The field that makes a note a custom template.
pub const DEFINITION_FIELD: &str = "keykeeper.template-definition";

/// The kind of a template's field: text, hidden (a secret), or yes or no.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    Text,
    Hidden,
    Boolean,
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
    /// For a copy an MSP sent: the MSP's name.
    #[serde(skip_serializing_if = "Option::is_none")]
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
    use FieldKind::{Boolean, Hidden, Text};
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
                ("Service URL", Text),
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
                ("Port", Text),
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
                ("Port", Text),
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
                ("Purchase date", Text),
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
                ("Port", Text),
            ],
        ),
        make(
            "membership",
            "Membership",
            Base::Note,
            "identity",
            &[("Organization", Text), ("Member ID", Text), ("Phone", Text)],
        ),
    ]
}

/// What a new item from a template records: {"id", "name", "v": 1}, as the
/// web vault writes it.
pub fn marker(id: &str, name: &str) -> String {
    serde_json::json!({ "id": id, "name": name, "v": 1 }).to_string()
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
    let fields = item
        .fields
        .iter()
        .zip(&item.kept.field_kinds)
        .filter(|(f, _)| !f.reserved())
        .filter_map(|(f, (kind, _))| {
            let kind = match kind {
                0 => FieldKind::Text,
                1 => FieldKind::Hidden,
                2 => FieldKind::Boolean,
                _ => return None,
            };
            Some(TemplateField {
                label: f.name.clone(),
                kind,
                value: Some(f.value.as_str().to_owned()).filter(|v| !v.is_empty()),
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
/// groups them (templates.ts, `templateGroups`): each MSP's (once each,
/// however many of its clients the person sees), then each organization's
/// own (by the organization's name), then the person's own, then the
/// built-in ones. Empty groups are left out.
pub fn groups(vault: &Vault) -> Vec<Group> {
    let by_name = |a: &Template, b: &Template| a.name.to_lowercase().cmp(&b.name.to_lowercase());
    // A hidden field's default is a secret like any other: someone the
    // template's folder hides passwords from gets the field, empty.
    let mut all: Vec<Template> = vault
        .items
        .iter()
        .filter_map(|item| {
            let mut template = custom(item)?;
            if !vault.grid(item).see_passwords {
                for field in &mut template.fields {
                    if field.kind == FieldKind::Hidden {
                        field.value = None;
                    }
                }
            }
            Some(template)
        })
        .collect();
    all.sort_by(by_name);
    let (from_msp, custom): (Vec<Template>, Vec<Template>) = all.into_iter().partition(|t| {
        t.from.is_some()
            && t.source
                .as_ref()
                .is_some_and(|s| s.organization_id.is_some())
    });
    let mut groups = Vec::new();
    let mut msps: Vec<String> = from_msp.iter().filter_map(|t| t.from.clone()).collect();
    msps.sort_by_key(|m| m.to_lowercase());
    msps.dedup();
    for msp in msps {
        let mut seen = std::collections::HashSet::new();
        groups.push(Group {
            name: format!("From {msp}"),
            templates: from_msp
                .iter()
                .filter(|t| t.from.as_deref() == Some(msp.as_str()) && seen.insert(t.id.clone()))
                .cloned()
                .collect(),
        });
    }
    let mut organizations: Vec<(&String, &String)> = vault.organizations.iter().collect();
    organizations.sort_by_key(|(_, name)| name.to_lowercase());
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
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_marker_is_written_as_the_web_vault_writes_it() {
        assert_eq!(
            marker("wifi", "Wi-Fi network"),
            r#"{"id":"wifi","name":"Wi-Fi network","v":1}"#
        );
        assert_eq!(
            marker("x", "Say \"hi\""),
            r#"{"id":"x","name":"Say \"hi\"","v":1}"#
        );
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
