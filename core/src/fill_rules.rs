//! Fill rules on websites, for Android's autofill (meta repo DESIGN.md,
//! "Choosing what fills where"): the person's "Never fill this field on this
//! site" list first, then the item's own rules, in the browser extension's
//! order and with its limits (passbundle-clients,
//! `apps/browser/src/autofill/keykeeper/fill-rules.ts`, `applyFillRules`).
//! What they leave goes to Android's own matching (the app's Roles.kt), as
//! upstream's matching takes what the extension's rules leave.
//!
//! Pure: from a page's fields as a browser describes them to Android
//! (`Page`), an item, and the list (`app/fill_plan.rs` reads it from the
//! account's settings document), to which of the item's values goes into
//! which field (`Plan`). The app takes each value from the core once the
//! person picks the item, as it takes every other one.
//!
//! A rule is a custom field named `keykeeper.fill` whose value is a small JSON
//! document, `{"v":1,"host":"bank.example","field":{"id":"custno"},"fill":
//! {"from":"field","name":"Customer ID"}}` (the web vault's
//! `libs/common/src/vault/keykeeper/fill-rules.ts`):
//!
//! - **Where:** only on the scheme (https when it names none) and host it was
//!   made on. Android gives a page's host without its port, so a rule made
//!   where the page named one never fills, and one made without one fills on
//!   that host, as Android fills logins. Where the browser doesn't say the
//!   page's scheme, no rule's value fills; never-fill still holds there. A
//!   never rule from before rules had a host holds everywhere, since it only
//!   holds a value back.
//! - **Which field:** by its id when the rule names one, else its name, else
//!   the label people see (the label, else its aria label, else its
//!   placeholder, in lower case with its spaces collapsed, as the extension
//!   records it). A field without the rule's id or name is never matched by
//!   its label.
//! - **Limits:** never a field upstream's matching never fills (the types it
//!   skips, search boxes, captchas and "forgot" fields, fields in a
//!   newsletter form), nor one people can't see; for someone who can't see
//!   the item's secrets (the password, hidden custom fields, a card's number
//!   and code), a secret only into a password field, where the page shows it
//!   masked; a one-time code only where the account may use one; a card's
//!   values only from a card and an identity's from an identity. A card's
//!   value fills only the page's own site: the extension also fills it into
//!   a payment service's frame there, but Android gives one web domain for
//!   the whole structure, so a structure whose web domain isn't the rule's
//!   site gets nothing from it.
//! - **Our own fields** (`keykeeper.*`) never fill a page field by name.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::vault::{Item, ItemKind, RESERVED_FIELD_PREFIX};

/// The custom field a rule is kept in.
pub const RULE_FIELD: &str = "keykeeper.fill";

/// A card's values a rule can fill, by the web vault's names for them
/// (`CARD_FILL_VALUES`). The import reads only these, as the web vault's does.
pub const CARD_VALUES: [&str; 5] = ["cardholderName", "number", "expMonth", "expYear", "code"];

/// An identity's values a rule can fill (`IDENTITY_FILL_VALUES`).
pub const IDENTITY_VALUES: [&str; 14] = [
    "firstName",
    "middleName",
    "lastName",
    "fullName",
    "company",
    "email",
    "phone",
    "address1",
    "address2",
    "address3",
    "city",
    "state",
    "postalCode",
    "country",
];

/// The page's scheme a rule fills on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

/// How a rule finds its field: the first of its id, name, and label it has
/// (`fieldMatches`, `decidingFingerprint`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fingerprint {
    Id(String),
    Name(String),
    /// The label people see, as the extension records it (lower case).
    Label(String),
}

/// What a rule puts in its field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Username,
    Password,
    Totp,
    /// A custom field, by its name.
    Field(String),
    /// One of a card's values (`CARD_VALUES`).
    Card(&'static str),
    /// One of an identity's values (`IDENTITY_VALUES`).
    Identity(&'static str),
    Never,
}

/// One fill rule, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The host, with any port, of the page it was made on; none for a never
    /// rule from before rules had one.
    pub host: Option<String>,
    /// None is https.
    pub scheme: Option<Scheme>,
    pub field: Fingerprint,
    pub fill: Source,
}

/// Whether a JSON value counts as there, as the extension's JavaScript reads
/// it: not null, false, 0, or "".
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

impl Rule {
    /// A stored rule (`isFillRule`), or None when it isn't one this app reads:
    /// another version, a card's or an identity's value this app doesn't know
    /// (one added later), or anything else. A rule whose id, name, or label
    /// that decides isn't text matches no field, so it's left out here too.
    pub fn read(rule: &Value) -> Option<Rule> {
        if rule["v"].as_f64() != Some(1.0)
            || !rule["field"].is_object()
            || !rule["fill"].is_object()
        {
            return None;
        }
        let host = match &rule["host"] {
            Value::Null => None,
            Value::String(host) => Some(host.clone()),
            _ => return None,
        };
        let scheme = match &rule["scheme"] {
            Value::Null => None,
            s if s == "http" => Some(Scheme::Http),
            s if s == "https" => Some(Scheme::Https),
            _ => return None,
        };
        let fill = &rule["fill"];
        let known = |names: &[&'static str]| {
            let value = fill["value"].as_str()?;
            names.iter().copied().find(|n| *n == value)
        };
        let fill = match fill["from"].as_str()? {
            "field" => Source::Field(fill["name"].as_str()?.to_owned()),
            "card" => Source::Card(known(&CARD_VALUES)?),
            "identity" => Source::Identity(known(&IDENTITY_VALUES)?),
            "username" => Source::Username,
            "password" => Source::Password,
            "totp" => Source::Totp,
            "never" => Source::Never,
            _ => return None,
        };
        let deciding = ["id", "name", "label"]
            .into_iter()
            .find(|key| truthy(&rule["field"][key]))?;
        let text = rule["field"][deciding].as_str()?.to_owned();
        let field = match deciding {
            "id" => Fingerprint::Id(text),
            "name" => Fingerprint::Name(text),
            _ => Fingerprint::Label(text),
        };
        Some(Rule {
            host,
            scheme,
            field,
            fill,
        })
    }

    /// A rule kept as text, as a custom field keeps it.
    pub fn parse(text: &str) -> Option<Rule> {
        Rule::read(&serde_json::from_str(text).ok()?)
    }

    /// The scheme it fills on: the one it records, https when it records none.
    fn fills_on(&self) -> Scheme {
        self.scheme.unwrap_or(Scheme::Https)
    }
}

/// An item's fill rules (`readFillRules`): its fields named `keykeeper.fill`,
/// in the item's order. Anything unreadable is left out.
pub fn item_rules(item: &Item) -> Vec<Rule> {
    item.fields
        .iter()
        .filter(|f| f.name == RULE_FIELD && !f.value.is_empty())
        .filter_map(|f| Rule::parse(&f.value))
        .collect()
}

/// The person's site-wide never-fill list, as its text (the JSON array the
/// extension encrypts, `site-never-fill.ts`): never rules with a site, and
/// nothing else (`onlyNeverFill`).
pub fn never_fill_list(text: &str) -> Vec<Rule> {
    let Ok(Value::Array(rules)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    rules
        .iter()
        .filter_map(Rule::read)
        .filter(|r| r.fill == Source::Never && r.host.as_deref().is_some_and(|h| !h.is_empty()))
        .collect()
}

/// A page, as a browser describes it to Android's autofill (the app's
/// Fields.kt gathers it).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    /// "https" or "http", as the browser says; none where it doesn't.
    #[serde(default)]
    pub scheme: Option<String>,
    /// Its host, without a port (Android gives none).
    pub host: String,
    #[serde(default)]
    pub fields: Vec<PageField>,
    /// Each form's name, by the place a field's `form` gives.
    #[serde(default)]
    pub forms: Vec<Option<String>>,
}

/// One of a page's fields, from the HTML the browser gives for it.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageField {
    /// The element: "input", "select", "textarea".
    #[serde(default)]
    pub tag: Option<String>,
    /// Its `type`.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// Its label, as the browser gives it.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub aria_label: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub autocomplete: Option<String>,
    /// Whether people can see it.
    #[serde(default)]
    pub visible: bool,
    /// A list to choose from (a select), rather than text.
    #[serde(default)]
    pub list: bool,
    /// The most characters it takes, when it says.
    #[serde(default)]
    pub max_length: Option<u32>,
    /// The form it's in, by its place in `Page::forms`.
    #[serde(default)]
    pub form: Option<usize>,
}

fn text(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("")
}

impl PageField {
    /// Its type, in lower case ("" for none).
    fn kind_of(&self) -> String {
        text(&self.kind).to_lowercase()
    }

    /// The label people see, as a rule records it (`shownLabel`): the label,
    /// else the aria label, else the placeholder, its spaces collapsed, in
    /// lower case.
    pub fn shown_label(&self) -> String {
        [&self.label, &self.aria_label, &self.placeholder]
            .into_iter()
            .map(text)
            .find(|t| !t.is_empty())
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }

    /// Whether this is the field a rule describes (`fieldMatches`).
    fn is(&self, fingerprint: &Fingerprint) -> bool {
        match fingerprint {
            Fingerprint::Id(id) => text(&self.id) == id,
            Fingerprint::Name(name) => text(&self.name) == name,
            Fingerprint::Label(label) => self.shown_label() == *label,
        }
    }
}

/// The field types upstream's autofill never fills (`ExcludedAutofillTypes`).
const EXCLUDED_TYPES: [&str; 8] = [
    "radio", "checkbox", "hidden", "file", "button", "image", "reset", "search",
];

/// A search box's words (`SearchFieldNames`).
const SEARCH_WORDS: [&str; 4] = ["search", "query", "find", "go"];

/// What marks a field upstream leaves alone (`FieldIgnoreList`).
const IGNORED: [&str; 3] = ["captcha", "findanything", "forgot"];

/// Whether a field is a search box by its words (`isSearchField`): its type,
/// name, id, or placeholder, split where a lower-case letter meets an
/// upper-case one and at anything but a letter.
fn search_box(field: &PageField) -> bool {
    [&field.kind, &field.name, &field.id, &field.placeholder]
        .into_iter()
        .map(text)
        .any(|value| {
            let mut spaced = String::new();
            let mut last: Option<char> = None;
            for c in value.chars() {
                if last.is_some_and(|l| l.is_ascii_lowercase()) && c.is_ascii_uppercase() {
                    spaced.push(' ');
                }
                spaced.push(c);
                last = Some(c);
            }
            spaced
                .to_lowercase()
                .split(|c: char| !c.is_ascii_lowercase())
                .any(|w| SEARCH_WORDS.contains(&w))
        })
}

/// Whether a field's id, name, or placeholder marks one upstream leaves
/// alone (`fieldHasDisqualifyingAttributeValue`): a captcha, a "forgot" link.
fn disqualified(field: &PageField) -> bool {
    [&field.id, &field.name, &field.placeholder]
        .into_iter()
        .map(|v| {
            text(v)
                .to_lowercase()
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
                .collect::<String>()
        })
        .any(|v| IGNORED.iter().any(|i| v.contains(i)))
}

/// Whether one of these says "newsletter", however it's split or written
/// (upstream's keywords, `StrongNonLoginKeywords`).
fn says_newsletter<'a>(values: impl IntoIterator<Item = &'a Option<String>>) -> bool {
    values.into_iter().any(|v| {
        text(v)
            .to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .collect::<String>()
            .contains("newsletter")
    })
}

fn newsletter_field(field: &PageField) -> bool {
    says_newsletter([
        &field.id,
        &field.name,
        &field.kind,
        &field.placeholder,
        &field.autocomplete,
        &field.label,
        &field.aria_label,
    ])
}

/// Whether a field is in a newsletter form (`isNonLoginUsernameField`): the
/// form's name says so, or the field or another in its form does.
fn in_newsletter_form(field: &PageField, page: &Page) -> bool {
    if newsletter_field(field) {
        return true;
    }
    let Some(form) = field.form else {
        return false;
    };
    says_newsletter(page.forms.get(form))
        || page
            .fields
            .iter()
            .any(|f| f.form == Some(form) && newsletter_field(f))
}

/// The fields a rule never fills, as upstream's matching never would
/// (`isExcludedFromFillRules`).
fn excluded(field: &PageField, page: &Page) -> bool {
    let kind = field.kind_of();
    text(&field.tag).eq_ignore_ascii_case("span")
        || EXCLUDED_TYPES.contains(&kind.as_str())
        || search_box(field)
        || disqualified(field)
        || in_newsletter_form(field, page)
}

/// What the person may do with the item, which limits what its rules fill.
#[derive(Clone, Copy, Debug)]
pub struct May {
    /// They may see its secrets (its folders' grid, `Vault::grid`).
    pub see_secrets: bool,
    /// Its one-time code may fill (`Vault::shows_code`).
    pub codes: bool,
}

/// How a field takes its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Take {
    /// As the item has it.
    Text,
    /// A card's number, digits only, as Android's card fill writes it.
    Digits,
    /// A card's month as two digits ("07").
    Month,
    /// A card's year as four digits ("2029").
    Year,
    /// A card's year as two, where the field takes only two characters.
    ShortYear,
    /// A list's option that is the value, or none.
    Option,
    /// A month list's option for the card's month (by name or number).
    MonthOption,
    /// A year list's option for the card's year (2029 or 29).
    YearOption,
    /// A state list's option, by code or name.
    StateOption,
    /// A country list's option, by code or name.
    CountryOption,
}

/// One value a rule fills.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Fill {
    /// The field, by its place in `Page::fields`.
    pub field: usize,
    /// The item's value, by the name the core gives it (`App::value`):
    /// "username", "password", "totp", "field:<index>", or a card's or an
    /// identity's (`Item::detail`).
    pub what: String,
    pub take: Take,
}

/// What the rules decide on a page: the values they fill, and the fields
/// they keep from anything filling (never-fill). Every other field is left
/// to Android's own matching.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub fills: Vec<Fill>,
    pub never: Vec<usize>,
}

/// The core's name for a source's value on this item, when the item has
/// that value to fill (`staticValueFor`); None for none.
fn value_of(item: &Item, source: &Source, may: May) -> Option<String> {
    let login = item.kind == ItemKind::Login;
    let some = |t: Option<&str>| t.is_some_and(|t| !t.is_empty());
    match source {
        Source::Username => (login && some(item.username.as_deref())).then(|| "username".into()),
        Source::Password => {
            (login && some(item.password.as_deref().map(String::as_str))).then(|| "password".into())
        }
        Source::Totp => (login && may.codes && some(item.totp.as_deref().map(String::as_str)))
            .then(|| "totp".into()),
        // The first of the person's own custom fields with that name.
        Source::Field(name) => item
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| !f.name.starts_with(RESERVED_FIELD_PREFIX) && f.name == *name)
            .filter(|(_, f)| !f.value.is_empty())
            .map(|(at, _)| format!("field:{at}")),
        Source::Card(value) if item.kind == ItemKind::Card => {
            let what = match *value {
                "number" => "cardNumber",
                "code" => "cardCode",
                other => other,
            };
            item.detail(what).map(|_| what.to_owned())
        }
        Source::Identity(value) if item.kind == ItemKind::Identity => {
            item.detail(value).map(|_| (*value).to_owned())
        }
        _ => None,
    }
}

/// Whether a source is one of the item's secrets (`isSecretSource`): the
/// password, its one-time code, a card's number and code, or a custom field
/// with that name that's hidden (or linked, which might fill one).
fn is_secret(item: &Item, source: &Source) -> bool {
    match source {
        Source::Username | Source::Never => false,
        Source::Password | Source::Totp => true,
        Source::Card(value) => *value == "number" || *value == "code",
        Source::Identity(value) => item.is_secret(value),
        Source::Field(name) => item.fields.iter().enumerate().any(|(at, f)| {
            !f.name.starts_with(RESERVED_FIELD_PREFIX)
                && f.name == *name
                && (f.hidden || item.kept.field_kinds.get(at).is_some_and(|k| k.0 == 3))
        }),
    }
}

/// Whether a rule's value may go into this field (`mayFill`, `mayApply`):
/// one people can see, not one upstream leaves alone, and for someone who
/// can't see the item's secrets, a secret only into a password field. A
/// one-time code lasts seconds, and its fields are text: it fills anyway.
fn may_fill(item: &Item, source: &Source, field: &PageField, page: &Page, may: May) -> bool {
    let kind = field.kind_of();
    if !field.visible || kind == "hidden" || excluded(field, page) {
        return false;
    }
    let hidden = *source != Source::Totp && is_secret(item, source);
    !hidden || may.see_secrets || kind == "password"
}

/// How a field takes a source's value.
fn take(source: &Source, field: &PageField) -> Take {
    if field.list {
        return match source {
            Source::Card("expMonth") => Take::MonthOption,
            Source::Card("expYear") => Take::YearOption,
            Source::Identity("state") => Take::StateOption,
            Source::Identity("country") => Take::CountryOption,
            _ => Take::Option,
        };
    }
    match source {
        Source::Card("number") => Take::Digits,
        Source::Card("expMonth") => Take::Month,
        Source::Card("expYear") if field.max_length == Some(2) => Take::ShortYear,
        Source::Card("expYear") => Take::Year,
        _ => Take::Text,
    }
}

impl Page {
    /// The page's host as rules name it.
    fn rule_host(&self) -> String {
        self.host.trim().trim_end_matches('.').to_lowercase()
    }

    fn rule_scheme(&self) -> Option<Scheme> {
        match self.scheme.as_deref().map(str::to_lowercase).as_deref() {
            Some("https") => Some(Scheme::Https),
            Some("http") => Some(Scheme::Http),
            _ => None,
        }
    }

    /// Whether a rule made for a site applies here (`ruleAppliesOn`): the
    /// same scheme and host. Where the scheme isn't known, only a never rule.
    fn has(&self, rule: &Rule) -> bool {
        let host = self.rule_host();
        let same_scheme = match self.rule_scheme() {
            Some(scheme) => rule.fills_on() == scheme,
            None => rule.fill == Source::Never,
        };
        rule.host
            .as_deref()
            .is_some_and(|h| !host.is_empty() && h == host)
            && same_scheme
    }
}

/// Which of the item's values its rules put into which of the page's
/// fields, and which fields they keep empty, after the person's site-wide
/// never-fill list (`applyFillRules`). The list holds for every item and
/// comes first, so a rule someone put on a shared item never fills a field
/// the person said never to fill on that site. Each field goes to the first
/// rule that names it; a never rule keeps it from everything, and a value
/// rule fills it within the limits (`may_fill`), or leaves it to Android's
/// own matching when it may not or the item has nothing for it.
pub fn plan(page: &Page, item: &Item, never_fill: &[Rule], may: May) -> Plan {
    let mut plan = Plan::default();
    // A never rule from before rules had a host holds anywhere.
    let applies = |r: &Rule| {
        if r.host.is_none() {
            r.fill == Source::Never
        } else {
            page.has(r)
        }
    };
    let own = item_rules(item);
    let rules: Vec<&Rule> = never_fill
        .iter()
        .filter(|r| r.fill == Source::Never)
        .chain(&own)
        .filter(|r| applies(r))
        .collect();
    for (at, field) in page.fields.iter().enumerate() {
        let Some(rule) = rules.iter().find(|r| field.is(&r.field)) else {
            continue;
        };
        if rule.fill == Source::Never {
            plan.never.push(at);
            continue;
        }
        if !may_fill(item, &rule.fill, field, page, may) {
            continue;
        }
        if let Some(what) = value_of(item, &rule.fill, may) {
            plan.fills.push(Fill {
                field: at,
                what,
                take: take(&rule.fill, field),
            });
        }
    }
    plan
}

#[cfg(test)]
mod tests;
