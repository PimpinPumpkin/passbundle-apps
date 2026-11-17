//! An organization's "Remove card item type" policy: upstream's
//! `RestrictedItemTypes`, type 15, as the server sends it with each sync. Its
//! `data` lists the item types it removes, by upstream's numbers; null or
//! missing means cards only, as the web vault's switch saves it. It binds
//! everyone in the organization, owners and admins too, as in upstream's own
//! apps. The rule is upstream's (passbundle-clients,
//! restricted-item-types.service.ts, `restricted$` and `isCipherRestricted`):
//!
//! - **A type is removed** when any organization's policy that's on lists it.
//! - **An item of a removed type is left out** unless it's in an
//!   organization that allows that type: one the person is in with no such
//!   policy on, or whose own policy doesn't list it. The person's own items
//!   of a removed type are left out.
//! - **No new item of a removed type**, anywhere.
//!
//! The vault applies it as it opens (vault.rs, `Vault::open`), so every list,
//! search, count, filling, the export, and "All accounts" leave those items
//! out, and an id of one answers as if it weren't there. Each account's
//! vault reads its own sync, and the next sync brings a change.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::Value;

use crate::vault::ItemKind;

/// Upstream's number for the policy.
const REMOVE_ITEM_TYPES: u64 = 15;

/// Upstream's number for a card: what the policy removes when it doesn't say.
const CARD: u8 = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestrictedTypes {
    /// Every type a policy that's on removes, by upstream's numbers.
    removed: BTreeSet<u8>,
    /// The types each organization's own policy removes, for those with one on.
    removed_by: HashMap<String, BTreeSet<u8>>,
    /// The organizations the person is in, from the sync's profile.
    member_of: HashSet<String>,
}

/// The types a policy's `data` lists. Null or missing is cards, as upstream
/// reads it; anything else that isn't a list is read the same way, so a
/// policy that's on never removes nothing by accident (upstream's apps
/// can't read one either). Entries that aren't a type are ignored.
fn listed(data: &Value) -> BTreeSet<u8> {
    match data {
        Value::Array(types) => types
            .iter()
            .filter_map(Value::as_u64)
            .filter_map(|t| u8::try_from(t).ok())
            .collect(),
        _ => BTreeSet::from([CARD]),
    }
}

/// Upstream's number for a kind of item.
pub fn number(kind: ItemKind) -> u8 {
    match kind {
        ItemKind::Login => 1,
        ItemKind::SecureNote => 2,
        ItemKind::Card => 3,
        ItemKind::Identity => 4,
        ItemKind::SshKey => 5,
        ItemKind::BankAccount => 6,
        ItemKind::DriversLicense => 7,
        ItemKind::Passport => 8,
        ItemKind::Other(n) => n,
    }
}

impl RestrictedTypes {
    /// The policy in a sync (the kept `sync.json`); none if it has none.
    pub fn from_sync(text: &str) -> RestrictedTypes {
        let Ok(sync) = serde_json::from_str::<Value>(text) else {
            return RestrictedTypes::default();
        };
        let organizations = sync
            .pointer("/profile/organizations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|o| o.get("id").and_then(Value::as_str));
        RestrictedTypes::read(&sync["policies"], organizations)
    }

    /// From a sync's `policies` and the ids of the organizations the person
    /// is in.
    pub fn read<'a>(
        policies: &Value,
        organizations: impl IntoIterator<Item = &'a str>,
    ) -> RestrictedTypes {
        let mut restricted = RestrictedTypes {
            member_of: organizations.into_iter().map(str::to_owned).collect(),
            ..RestrictedTypes::default()
        };
        for p in policies.as_array().into_iter().flatten() {
            if p.get("type").and_then(Value::as_u64) != Some(REMOVE_ITEM_TYPES)
                || p.get("enabled").and_then(Value::as_bool) != Some(true)
            {
                continue;
            }
            let types = listed(p.get("data").unwrap_or(&Value::Null));
            restricted.removed.extend(types.iter().copied());
            if let Some(org) = p.get("organizationId").and_then(Value::as_str) {
                restricted
                    .removed_by
                    .entry(org.to_owned())
                    .or_default()
                    .extend(types);
            }
        }
        restricted
    }

    /// Whether an item of this type (upstream's number), in this
    /// organization or the person's own (None), is left out.
    pub fn hides(&self, kind: u8, organization: Option<&str>) -> bool {
        if !self.removed.contains(&kind) {
            return false;
        }
        match organization {
            None => true,
            Some(org) => {
                !self.member_of.contains(org)
                    || self
                        .removed_by
                        .get(org)
                        .is_some_and(|types| types.contains(&kind))
            }
        }
    }

    /// Whether a new item of this kind may not be made, anywhere.
    pub fn refuses_new(&self, kind: ItemKind) -> bool {
        self.removed.contains(&number(kind))
    }

    /// The kinds no new item may be, by upstream's numbers.
    pub fn removed(&self) -> impl Iterator<Item = u8> + '_ {
        self.removed.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const WORK: &str = "o-work";
    const CLUB: &str = "o-club";

    /// A sync with the person in two organizations (an owner of one, a
    /// member of the other) and these policies.
    fn sync(policies: Value) -> String {
        json!({
            "profile": { "organizations": [
                { "id": WORK, "name": "Work", "type": 0 },
                { "id": CLUB, "name": "Club", "type": 2 },
            ] },
            "policies": policies,
        })
        .to_string()
    }

    fn policy(org: &str, enabled: bool, data: Value) -> Value {
        json!({ "id": format!("p-{org}"), "organizationId": org, "type": 15, "enabled": enabled, "data": data, "object": "policy" })
    }

    /// Which of a card in Work, a card in Club, and the person's own card
    /// are left out.
    fn cards_hidden(r: &RestrictedTypes) -> [bool; 3] {
        [
            r.hides(3, Some(WORK)),
            r.hides(3, Some(CLUB)),
            r.hides(3, None),
        ]
    }

    #[test]
    fn with_no_policy_nothing_is_left_out_and_anything_can_be_made() {
        for text in [
            sync(json!([])),
            sync(Value::Null),
            "{}".into(),
            "not json".into(),
        ] {
            let r = RestrictedTypes::from_sync(&text);
            for kind in 1..=8 {
                assert!(!r.hides(kind, None));
                assert!(!r.hides(kind, Some(WORK)));
                assert!(!r.refuses_new(ItemKind::from(kind)));
            }
            assert_eq!(r.removed().count(), 0);
        }
        // Other policies don't count.
        let r = RestrictedTypes::from_sync(&sync(json!([
            { "organizationId": WORK, "type": 6, "enabled": true, "data": null },
            { "organizationId": WORK, "type": 14, "enabled": true, "data": [3] },
        ])));
        assert_eq!(cards_hidden(&r), [false; 3]);
    }

    #[test]
    fn a_policy_with_no_list_removes_cards_only_owners_too() {
        // Null, as the web vault's switch saves it, or no data at all.
        for data in [Value::Null, json!("missing")] {
            let mut p = policy(WORK, true, data.clone());
            if data == json!("missing") {
                p.as_object_mut().unwrap().remove("data");
            }
            let r = RestrictedTypes::from_sync(&sync(json!([p])));
            // The person owns Work: its policy binds them all the same.
            assert_eq!(cards_hidden(&r), [true, false, true]);
            assert!(r.refuses_new(ItemKind::Card));
            for kind in [1, 2, 4, 5, 6, 7, 8] {
                assert!(!r.hides(kind, None), "{kind}");
                assert!(!r.refuses_new(ItemKind::from(kind)), "{kind}");
            }
            assert_eq!(r.removed().collect::<Vec<_>>(), vec![3]);
        }
    }

    #[test]
    fn a_policy_listing_cards_and_identities_removes_both() {
        let r = RestrictedTypes::from_sync(&sync(json!([policy(WORK, true, json!([3, 4]))])));
        assert_eq!(cards_hidden(&r), [true, false, true]);
        assert!(r.hides(4, None) && r.hides(4, Some(WORK)) && !r.hides(4, Some(CLUB)));
        assert!(!r.hides(1, None) && !r.hides(2, Some(WORK)));
        assert!(r.refuses_new(ItemKind::Card) && r.refuses_new(ItemKind::Identity));
        assert!(!r.refuses_new(ItemKind::Login) && !r.refuses_new(ItemKind::SecureNote));
        // An empty list removes nothing, as upstream reads it; entries that
        // aren't a type are ignored; anything that isn't a list is cards.
        let r = RestrictedTypes::from_sync(&sync(json!([policy(WORK, true, json!([]))])));
        assert_eq!(cards_hidden(&r), [false; 3]);
        assert!(!r.refuses_new(ItemKind::Card));
        let r = RestrictedTypes::from_sync(&sync(json!([policy(
            WORK,
            true,
            json!(["4", 4, -1, 300])
        )])));
        assert_eq!(r.removed().collect::<Vec<_>>(), vec![4]);
        let r = RestrictedTypes::from_sync(&sync(json!([policy(WORK, true, json!({}))])));
        assert_eq!(r.removed().collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn an_organization_without_the_policy_keeps_its_own_cards() {
        // Work removes cards; Club doesn't: Club's card stays, Work's and
        // the person's own go, and no new card can be made anywhere.
        let r = RestrictedTypes::from_sync(&sync(json!([policy(WORK, true, Value::Null)])));
        assert_eq!(cards_hidden(&r), [true, false, true]);
        assert!(r.refuses_new(ItemKind::Card));
        // Club's own policy removing identities only still allows its cards.
        let r = RestrictedTypes::from_sync(&sync(json!([
            policy(WORK, true, Value::Null),
            policy(CLUB, true, json!([4])),
        ])));
        assert_eq!(cards_hidden(&r), [true, false, true]);
        assert!(r.hides(4, Some(CLUB)) && !r.hides(4, Some(WORK)) && r.hides(4, None));
        // Both remove cards: every card goes.
        let r = RestrictedTypes::from_sync(&sync(json!([
            policy(WORK, true, json!([3])),
            policy(CLUB, true, Value::Null),
        ])));
        assert_eq!(cards_hidden(&r), [true; 3]);
        // A card of an organization the person isn't in (not in their
        // profile) isn't allowed by anyone, as upstream has it.
        let r = RestrictedTypes::from_sync(&sync(json!([policy(WORK, true, Value::Null)])));
        assert!(r.hides(3, Some("o-elsewhere")));
    }

    #[test]
    fn a_policy_thats_off_removes_nothing() {
        let r = RestrictedTypes::from_sync(&sync(json!([
            policy(WORK, false, Value::Null),
            policy(CLUB, false, json!([3, 4])),
        ])));
        assert_eq!(cards_hidden(&r), [false; 3]);
        assert!(!r.refuses_new(ItemKind::Card) && !r.refuses_new(ItemKind::Identity));
        // Nor one that doesn't say it's on.
        let mut p = policy(WORK, true, Value::Null);
        p["enabled"] = json!("true");
        let r = RestrictedTypes::from_sync(&sync(json!([p])));
        assert_eq!(cards_hidden(&r), [false; 3]);
    }

    #[test]
    fn upstreams_numbers_for_each_kind() {
        for n in 1..=9 {
            assert_eq!(number(ItemKind::from(n)), n);
        }
    }
}
