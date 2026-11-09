//! An organization's "Vault lock" policy (docs/core.md, "Locking by policy"):
//! upstream's vault timeout policy, type 9, as the server sends it with each
//! sync, `{"type", "minutes", "action"}`. Every organization's applies, but
//! not to the organization's owners, as in upstream's own apps (the web vault
//! and the extension), and the strictest of each part wins. The core locks by
//! the stricter of this and the person's own choice, which it keeps as they
//! chose it.
//!
//! With it, upstream's "Remove Unlock with PIN" policy, type 14, which has no
//! data: an organization that turns it on doesn't want quicker ways than the
//! master password pushed on its members, so the phone apps don't offer
//! unlocking with biometrics on their own (they still let the person turn it
//! on in Settings, as upstream's apps do).

use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;

use crate::autolock::{Preferences, MAX_MINUTES};

/// Upstream's number for the policy.
const VAULT_LOCK: i64 = 9;

/// Upstream's number for "Remove Unlock with PIN".
const REMOVE_UNLOCK_WITH_PIN: i64 = 14;

/// Upstream's number for an organization's owner.
const OWNER: i64 = 0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LockPolicy {
    /// The longest wait any organization allows, in minutes.
    pub max_minutes: Option<u32>,
    /// Lock as the app leaves the screen.
    pub immediately: bool,
    /// Lock when the screen locks or the computer sleeps.
    pub on_system_lock: bool,
    /// Sign out instead of locking, when it's time.
    pub sign_out: bool,
    /// An organization's "Remove Unlock with PIN" is on: the apps don't
    /// offer quicker unlocking on their own.
    pub remove_unlock_with_pin: bool,
}

impl LockPolicy {
    /// The policy in a sync (the kept `sync.json`); none if it has none.
    pub fn from_sync(text: &str) -> LockPolicy {
        let mut policy = LockPolicy::default();
        let Ok(sync) = serde_json::from_str::<Value>(text) else {
            return policy;
        };
        // The organizations this person owns, whose policy doesn't bind them.
        let memberships = sync
            .pointer("/profile/organizations")
            .and_then(Value::as_array);
        let owned: HashSet<&str> = memberships
            .into_iter()
            .flatten()
            .filter(|o| o.get("type").and_then(Value::as_i64) == Some(OWNER))
            .filter_map(|o| o.get("id").and_then(Value::as_str))
            .collect();
        let policies = sync.get("policies").and_then(Value::as_array);
        for p in policies.into_iter().flatten() {
            let org = p.get("organizationId").and_then(Value::as_str);
            let kind = p.get("type").and_then(Value::as_i64);
            if p.get("enabled").and_then(Value::as_bool) != Some(true)
                || org.is_some_and(|org| owned.contains(org))
            {
                continue;
            }
            if kind == Some(REMOVE_UNLOCK_WITH_PIN) {
                policy.remove_unlock_with_pin = true;
                continue;
            }
            if kind != Some(VAULT_LOCK) {
                continue;
            }
            let Some(data) = p.get("data").filter(|d| d.is_object()) else {
                continue;
            };
            match data.get("type").and_then(Value::as_str) {
                None | Some("custom") => {
                    let minutes = data
                        .get("minutes")
                        .and_then(Value::as_u64)
                        .filter(|m| (1..=u64::from(MAX_MINUTES)).contains(m));
                    if let Some(m) = minutes.and_then(|m| u32::try_from(m).ok()) {
                        policy.max_minutes = Some(policy.max_minutes.map_or(m, |old| old.min(m)));
                    }
                }
                Some("immediately") => policy.immediately = true,
                Some("onSystemLock") => policy.on_system_lock = true,
                // "never" and "onAppRestart" limit nothing here: the app
                // always locks when it closes. The server sends no others.
                Some(_) => {}
            }
            if data.get("action").and_then(Value::as_str) == Some("logOut") {
                policy.sign_out = true;
            }
        }
        policy
    }

    /// The person's choices, held to the policy.
    pub fn apply(&self, chosen: &Preferences) -> Preferences {
        let lock_after_minutes = if self.immediately {
            Some(0)
        } else {
            match (chosen.lock_after_minutes, self.max_minutes) {
                (Some(m), Some(max)) => Some(m.min(max)),
                (None, Some(max)) => Some(max),
                (m, None) => m,
            }
        };
        Preferences {
            lock_after_minutes,
            lock_on_sleep: chosen.lock_on_sleep || self.on_system_lock,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sync(policies: Value) -> String {
        json!({ "profile": { "organizations": [
            { "id": "owned", "type": 0, "status": 2 },
            { "id": "admin", "type": 1, "status": 2 },
            { "id": "custom", "type": 4, "status": 2 },
        ] }, "policies": policies })
        .to_string()
    }

    fn vault_lock(org: &str, data: Value) -> Value {
        json!({ "id": format!("p-{org}"), "organizationId": org, "type": 9, "enabled": true, "data": data, "object": "policy" })
    }

    #[test]
    fn none_without_an_enabled_vault_lock_policy() {
        assert_eq!(LockPolicy::from_sync("{}"), LockPolicy::default());
        assert_eq!(LockPolicy::from_sync("not json"), LockPolicy::default());
        let others = sync(json!([
            { "organizationId": "a", "type": 1, "enabled": true, "data": { "minLength": 12 } },
            { "organizationId": "b", "type": 9, "enabled": false, "data": { "minutes": 5 } },
            { "organizationId": "c", "type": 9, "enabled": true, "data": null },
            vault_lock("d", json!({ "type": "never", "minutes": 0 })),
            vault_lock("e", json!({ "type": "onAppRestart" })),
        ]));
        assert_eq!(LockPolicy::from_sync(&others), LockPolicy::default());
    }

    #[test]
    fn reads_each_part() {
        let read = |data: Value| LockPolicy::from_sync(&sync(json!([vault_lock("a", data)])));
        assert_eq!(
            read(json!({ "type": "custom", "minutes": 15, "action": "lock" })),
            LockPolicy {
                max_minutes: Some(15),
                ..Default::default()
            }
        );
        // No type is a longest wait, as upstream's clients read it.
        assert_eq!(read(json!({ "minutes": 240 })).max_minutes, Some(240));
        assert_eq!(
            read(json!({ "type": "immediately", "minutes": null, "action": "logOut" })),
            LockPolicy {
                immediately: true,
                sign_out: true,
                ..Default::default()
            }
        );
        assert_eq!(
            read(json!({ "type": "onSystemLock" })),
            LockPolicy {
                on_system_lock: true,
                ..Default::default()
            }
        );
        // Minutes the server wouldn't keep are no wait at all.
        for minutes in [json!(0), json!(1441), json!(-5), json!(2.5), json!("15")] {
            assert_eq!(
                read(json!({ "type": "custom", "minutes": minutes })).max_minutes,
                None
            );
        }
    }

    #[test]
    fn binds_admins_and_everyone_else_but_not_owners() {
        let policy = |org: &str| {
            LockPolicy::from_sync(&sync(json!([vault_lock(
                org,
                json!({ "minutes": 5, "action": "logOut" })
            )])))
        };
        assert_eq!(policy("owned"), LockPolicy::default());
        let bound = LockPolicy {
            max_minutes: Some(5),
            sign_out: true,
            ..Default::default()
        };
        assert_eq!(policy("admin"), bound);
        assert_eq!(policy("custom"), bound);
        // An organization the profile doesn't list (a member's).
        assert_eq!(policy("member"), bound);
    }

    #[test]
    fn the_strictest_of_each_part_wins() {
        let policy = LockPolicy::from_sync(&sync(json!([
            vault_lock(
                "a",
                json!({ "type": "custom", "minutes": 60, "action": "lock" })
            ),
            vault_lock("b", json!({ "type": "custom", "minutes": 15 })),
            vault_lock("c", json!({ "type": "onSystemLock", "action": "logOut" })),
            vault_lock("d", json!({ "type": "custom", "minutes": 30 })),
        ])));
        assert_eq!(
            policy,
            LockPolicy {
                max_minutes: Some(15),
                immediately: false,
                on_system_lock: true,
                sign_out: true,
                remove_unlock_with_pin: false,
            }
        );
    }

    #[test]
    fn reads_remove_unlock_with_pin_for_everyone_but_owners() {
        let pin = |org: &str, enabled: bool| json!({ "id": format!("pin-{org}"), "organizationId": org, "type": 14, "enabled": enabled, "data": null, "object": "policy" });
        let read = |policies: Value| LockPolicy::from_sync(&sync(policies));
        assert_eq!(
            read(json!([pin("member", true)])),
            LockPolicy {
                remove_unlock_with_pin: true,
                ..Default::default()
            }
        );
        assert!(read(json!([pin("admin", true)])).remove_unlock_with_pin);
        // Off, or the person's own organization: nothing.
        assert_eq!(read(json!([pin("member", false)])), LockPolicy::default());
        assert_eq!(read(json!([pin("owned", true)])), LockPolicy::default());
        // Beside a vault lock policy, both are read.
        let both = read(json!([
            pin("member", true),
            vault_lock("b", json!({ "type": "custom", "minutes": 15 })),
        ]));
        assert!(both.remove_unlock_with_pin);
        assert_eq!(both.max_minutes, Some(15));
        // It changes nothing about when the vault locks.
        let chose = Preferences {
            lock_after_minutes: Some(30),
            lock_on_sleep: false,
        };
        assert_eq!(both.apply(&chose).lock_after_minutes, Some(15));
        assert_eq!(read(json!([pin("member", true)])).apply(&chose), chose);
    }

    #[test]
    fn holds_the_persons_choice_to_it() {
        let chose = |m: Option<u32>, sleep: bool| Preferences {
            lock_after_minutes: m,
            lock_on_sleep: sleep,
        };
        let at_most_15 = LockPolicy {
            max_minutes: Some(15),
            ..Default::default()
        };
        // Shorter stays; longer, or only when the app closes, comes down to it.
        assert_eq!(
            at_most_15.apply(&chose(Some(5), false)),
            chose(Some(5), false)
        );
        assert_eq!(
            at_most_15.apply(&chose(Some(240), true)),
            chose(Some(15), true)
        );
        assert_eq!(
            at_most_15.apply(&chose(None, false)),
            chose(Some(15), false)
        );
        // A phone's "Immediately" is already stricter.
        assert_eq!(
            at_most_15.apply(&chose(Some(0), false)),
            chose(Some(0), false)
        );

        let immediately = LockPolicy {
            immediately: true,
            ..Default::default()
        };
        assert_eq!(
            immediately.apply(&chose(None, false)),
            chose(Some(0), false)
        );
        assert_eq!(
            immediately.apply(&chose(Some(5), true)),
            chose(Some(0), true)
        );

        let on_system_lock = LockPolicy {
            on_system_lock: true,
            ..Default::default()
        };
        assert_eq!(
            on_system_lock.apply(&chose(Some(30), false)),
            chose(Some(30), true)
        );
        assert_eq!(on_system_lock.apply(&chose(None, false)), chose(None, true));

        // No policy: the person's own choice.
        assert_eq!(
            LockPolicy::default().apply(&chose(None, false)),
            chose(None, false)
        );
    }
}
