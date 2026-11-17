//! Two-step codes need Premium (DESIGN.md in the meta repo, "Plans and
//! billing", decision 2, decided by the owner on November 16, 2026), as in
//! upstream's apps: an item's current code shows, copies, and fills only for
//! someone with Premium, of their own (the profile's `premium`) or through an
//! organization or an MSP (`premiumFromOrganization`), or for an item in an
//! organization that gives codes (the item's `organizationUseTotp`).
//!
//! Without either, the key stays in the item, and the editor still shows and
//! changes it: nothing is lost, and the code comes back with Premium. Where a
//! server doesn't enforce plans it says everyone has Premium, so every code
//! shows as it always has; a server that doesn't say counts as one that does.
//!
//! The rule is here once, and every place a code shows or fills asks it: the
//! item's view and its code (`pb_item`, `pb_totp`), copying it
//! (`App::value`, which phones' copying and filling go through too), and
//! what Android's autofill offers (`fillable`).

use crate::vault::{Item, Profile, Vault};

/// What the item's view says where its code would be, as the web vault and
/// the extension say it, and why copying or filling one is refused.
pub const NEEDS_PREMIUM: &str = "two-step login codes are part of Premium";

/// The rule: Premium, or an organization that gives codes.
pub fn shown(premium: bool, organization_gives_codes: bool) -> bool {
    premium || organization_gives_codes
}

impl Profile {
    /// Whether the person has Premium's features, of their own or through an
    /// organization or an MSP, as the server works it out. Only a server
    /// that says `premium: false` takes it away.
    pub fn has_premium(&self) -> bool {
        self.premium != Some(false) || self.premium_from_organization == Some(true)
    }
}

impl Vault {
    /// Whether an item's current code may show, be copied, and fill.
    pub fn shows_code(&self, item: &Item) -> bool {
        shown(self.premium, item.organization_use_totp)
    }

    /// Whether an item has a code to show: a key, and the rule allows it.
    pub fn has_code(&self, item: &Item) -> bool {
        item.totp.is_some() && self.shows_code(item)
    }

    /// Whether an item has a key whose code is held back for want of Premium.
    pub fn code_needs_premium(&self, item: &Item) -> bool {
        item.totp.is_some() && !self.shows_code(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(premium: Option<bool>, from_organization: Option<bool>) -> Profile {
        Profile {
            id: "user".into(),
            email: "ana@example.test".into(),
            key: None,
            private_key: None,
            organizations: Vec::new(),
            premium,
            premium_from_organization: from_organization,
        }
    }

    #[test]
    fn premium_of_their_own_or_through_an_organization_or_an_organization_that_gives_codes() {
        // Premium of their own.
        assert!(profile(Some(true), Some(false)).has_premium());
        // Through an organization or an MSP.
        assert!(profile(Some(false), Some(true)).has_premium());
        assert!(profile(Some(true), Some(true)).has_premium());
        // Neither.
        assert!(!profile(Some(false), Some(false)).has_premium());
        assert!(!profile(Some(false), None).has_premium());
        // A server that doesn't say (none we know of) counts as one where
        // plans aren't enforced: as before.
        assert!(profile(None, None).has_premium());

        // An organization that gives codes shows them without Premium.
        assert!(shown(true, false));
        assert!(shown(false, true));
        assert!(shown(true, true));
        assert!(!shown(false, false));
    }

    #[test]
    fn where_plans_arent_enforced_the_server_says_everyone_has_premium() {
        // What a server that doesn't enforce plans sends every account.
        let sync: crate::vault::Sync = serde_json::from_str(
            r#"{"profile": {"id": "u", "email": "ana@example.test", "premium": true, "premiumFromOrganization": false}}"#,
        )
        .unwrap();
        assert!(sync.profile.has_premium());
        // A Free account where they are.
        let sync: crate::vault::Sync = serde_json::from_str(
            r#"{"profile": {"id": "u", "email": "ana@example.test", "premium": false, "premiumFromOrganization": false}}"#,
        )
        .unwrap();
        assert!(!sync.profile.has_premium());
    }
}
