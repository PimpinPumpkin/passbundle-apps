//! Fill rules on a website in Android's autofill (`crate::fill_rules`): an
//! item's own rules after the account's "Never fill this field on this site"
//! list, with what the person may see of the item.
//!
//! The list is where the browser extension keeps it (meta repo DESIGN.md,
//! "Never fill this field on this site"): the `fill` part of the account's
//! settings document, which the server keeps encrypted with the account's
//! user key and every sync brings here with the generator's settings
//! (synced_settings.rs, `App::sync_settings`), as `{"neverFill": "<the list,
//! encrypted with the user key once more>"}`. It's opened only here, with
//! the unlocked vault's user key, and never kept open.

use super::App;
use crate::crypto::EncString;
use crate::error::{Error, Result};
use crate::fill_rules::{self, May, Page, Plan, Rule};
use crate::synced_settings;

impl App {
    /// Which of an item's values its fill rules put into which of a web
    /// page's fields, and which fields its rules and the account's site-wide
    /// never-fill list keep empty (`fill_rules::plan`). Only an item in use;
    /// none while locked.
    pub fn fill_plan(&self, id: &str, page: &Page) -> Result<Plan> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let item = vault
            .item(id)
            .filter(|i| i.in_use())
            .ok_or(Error::Damaged("no such item"))?;
        let may = May {
            see_secrets: vault.grid(item).see_passwords,
            codes: vault.shows_code(item),
        };
        Ok(fill_rules::plan(page, item, &self.site_never_fill(), may))
    }

    /// The account's "Never fill this field on this site" list, opened with
    /// its user key; none while it's locked, before a sync has brought the
    /// settings document, or when the list doesn't open with this key (one
    /// from before the account's key changed).
    pub fn site_never_fill(&self) -> Vec<Rule> {
        let (Some(vault), Some(settings)) = (&self.vault, &self.settings) else {
            return Vec::new();
        };
        let account = synced_settings::account(&settings.server, &settings.email);
        let Some(known) = self
            .store
            .synced_settings()
            .filter(|k| k.account == account)
        else {
            return Vec::new();
        };
        let Some(data) = known
            .settings
            .get("fill")
            .and_then(|fill| fill.get("neverFill"))
            .and_then(|list| list.as_str())
        else {
            return Vec::new();
        };
        EncString::parse(data)
            .ok()
            .and_then(|list| list.decrypt_string(vault.user_key()).ok())
            .map(|text| fill_rules::never_fill_list(&text))
            .unwrap_or_default()
    }
}
