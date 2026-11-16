//! A person's plans as the server tells them in each sync (DESIGN.md in the
//! meta repo, "Plans and billing", "How a plan reaches the clients"):
//! `keykeeperPlan` on the profile, and on each organization in it. Settings
//! shows them (app/lib/plan.dart): the person's own plan, and the plan of each
//! organization they own or are an admin of, with its state and dates. The
//! apps never sell a plan, show a price, or link to paying ("Not yet": Apple's
//! and Google's rules), so nothing here is about paying.
//!
//! Nothing is enforced from this, and nothing here works out a state, a date,
//! or what a plan gives: the server's one function does, and this carries its
//! answer. Where the server doesn't enforce plans (`enforced` isn't true on
//! the profile, as on every self-hosted server unless its operator turns
//! PLANS_ENFORCED on, or a server too old to say), there's nothing to show.

use serde::Serialize;
use serde_json::Value;

/// Upstream's numbers for an organization's owner and admin.
const OWNER: i64 = 0;
const ADMIN: i64 = 1;

/// One plan, as the server said it at the last sync. The dates, whether it
/// renews, and a payment due are only for whoever manages the plan: for
/// anyone else the server sends none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// "free", "premium", "families", "business", or "msp"; none for an
    /// organization with no plan.
    pub plan: Option<String>,
    /// "trial", "active", "paymentDue", or "ended"; none with no plan.
    pub state: Option<String>,
    pub free_until: Option<String>,
    /// "trial" or "offer".
    pub free_why: Option<String>,
    pub until: Option<String>,
    pub renews: Option<bool>,
    /// The last moment the plan is in effect, where the server says it.
    pub ends_at: Option<String>,
    /// Attachments, in whole GB, as the plan gives them now.
    pub storage_gb: Option<u64>,
    /// How many people it covers; none for no limit.
    pub people: Option<u64>,
    /// The MSP whose pool covers this organization.
    pub covered_by: Option<CoveredBy>,
    pub can_manage: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoveredBy {
    pub id: String,
    pub name: String,
}

/// An organization the person owns or is an admin of, and its plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationPlan {
    pub id: String,
    pub name: String,
    /// "owner" or "admin".
    pub role: String,
    pub plan: Plan,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plans {
    /// Whether this server enforces plans. When not, there's nothing else.
    pub enforced: bool,
    /// The person's own plan: "free" when they have none.
    pub own: Option<Plan>,
    /// The profile's `premiumFromOrganization`: Premium's features come
    /// through an organization or an MSP.
    pub premium_from_organization: bool,
    /// The organizations they own or are an admin of, by name.
    pub organizations: Vec<OrganizationPlan>,
}

/// A text the server sent, or none for anything else.
fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// One of the values given, or none.
fn one_of(v: &Value, key: &str, values: &[&str]) -> Option<String> {
    text(v, key).filter(|t| values.contains(&t.as_str()))
}

/// A date the server sent: an RFC 3339 moment, such as
/// "2026-12-15T12:00:00.000000Z". Anything else is none.
fn date(v: &Value, key: &str) -> Option<String> {
    text(v, key).filter(|t| {
        let b = t.as_bytes();
        b.len() >= 20
            && b[4] == b'-'
            && b[7] == b'-'
            && b[10] == b'T'
            && b[13] == b':'
            && b[16] == b':'
            && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
                .iter()
                .all(|&i| b[i].is_ascii_digit())
            && (t.ends_with('Z') || t[19..].contains('+') || t[19..].contains('-'))
    })
}

impl Plan {
    /// `keykeeperPlan` as the server sent it, or none where it doesn't say
    /// this server enforces plans.
    fn from_value(v: Option<&Value>) -> Option<Plan> {
        let v = v.filter(|v| v.get("enforced").and_then(Value::as_bool) == Some(true))?;
        let covered_by = v.get("coveredBy").and_then(|c| {
            Some(CoveredBy {
                id: text(c, "id")?,
                name: text(c, "name")?,
            })
        });
        Some(Plan {
            plan: one_of(
                v,
                "plan",
                &["free", "premium", "families", "business", "msp"],
            ),
            state: one_of(v, "state", &["trial", "active", "paymentDue", "ended"]),
            free_until: date(v, "freeUntil"),
            free_why: one_of(v, "freeWhy", &["trial", "offer"]),
            until: date(v, "until"),
            renews: v.get("renews").and_then(Value::as_bool),
            ends_at: date(v, "endsAt"),
            storage_gb: v.get("storageGb").and_then(Value::as_u64),
            people: v.get("people").and_then(Value::as_u64),
            covered_by,
            can_manage: v.get("canManage").and_then(Value::as_bool) == Some(true),
        })
    }
}

impl Plans {
    /// The plans in a sync (the kept `sync.json`); nothing where the server
    /// doesn't enforce plans.
    pub fn from_sync(text: &str) -> Plans {
        let Ok(sync) = serde_json::from_str::<Value>(text) else {
            return Plans::default();
        };
        let profile = sync.get("profile");
        let Some(own) = Plan::from_value(profile.and_then(|p| p.get("keykeeperPlan"))) else {
            return Plans::default();
        };
        let mut organizations: Vec<OrganizationPlan> = profile
            .and_then(|p| p.get("organizations"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|o| {
                let role = match o.get("type").and_then(Value::as_i64) {
                    Some(OWNER) => "owner",
                    Some(ADMIN) => "admin",
                    _ => return None,
                };
                Some(OrganizationPlan {
                    id: self::text(o, "id")?,
                    name: self::text(o, "name").unwrap_or_default(),
                    role: role.to_owned(),
                    plan: Plan::from_value(o.get("keykeeperPlan"))?,
                })
            })
            .collect();
        organizations.sort_by_key(|o| o.name.to_lowercase());
        Plans {
            enforced: true,
            own: Some(own),
            premium_from_organization: profile
                .and_then(|p| p.get("premiumFromOrganization"))
                .and_then(Value::as_bool)
                == Some(true),
            organizations,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const DEC_15: &str = "2026-12-15T12:00:00.000000Z";

    /// `keykeeperPlan` as the server sends it, with what a test changes.
    fn plan(fields: Value) -> Value {
        let mut v = json!({
            "enforced": true,
            "plan": "business",
            "state": "active",
            "freeUntil": null,
            "freeWhy": null,
            "until": null,
            "renews": null,
            "storageGb": 12,
            "people": null,
            "coveredBy": null,
            "canManage": true,
        });
        for (k, x) in fields.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    }

    fn sync(own: Value, organizations: Value) -> String {
        json!({ "profile": {
            "id": "user", "email": "ana@example.test",
            "keykeeperPlan": own, "organizations": organizations,
        } })
        .to_string()
    }

    #[test]
    fn nothing_where_the_server_doesnt_enforce_plans() {
        assert_eq!(Plans::from_sync("not json"), Plans::default());
        assert_eq!(Plans::from_sync("{}"), Plans::default());
        // A server too old to say.
        assert_eq!(
            Plans::from_sync(&json!({ "profile": { "id": "u", "organizations": [] } }).to_string()),
            Plans::default()
        );
        // A server that doesn't enforce them, even with organizations that
        // say otherwise.
        let off = sync(
            json!({ "enforced": false }),
            json!([{ "id": "o", "name": "Acme", "type": 0, "keykeeperPlan": plan(json!({})) }]),
        );
        assert_eq!(Plans::from_sync(&off), Plans::default());
        assert!(!Plans::from_sync(&off).enforced);
    }

    #[test]
    fn reads_the_persons_own_plan() {
        let own = plan(json!({
            "plan": "premium", "state": "paymentDue", "until": DEC_15, "renews": true,
            "storageGb": 5, "endsAt": "2026-12-29T12:00:00.000000Z",
        }));
        let plans = Plans::from_sync(&sync(own, json!([])));
        assert!(plans.enforced);
        assert_eq!(
            plans.own,
            Some(Plan {
                plan: Some("premium".into()),
                state: Some("paymentDue".into()),
                until: Some(DEC_15.into()),
                renews: Some(true),
                ends_at: Some("2026-12-29T12:00:00.000000Z".into()),
                storage_gb: Some(5),
                can_manage: true,
                ..Plan::default()
            })
        );
        assert!(plans.organizations.is_empty());
        assert!(!plans.premium_from_organization);
    }

    #[test]
    fn reads_free_and_premium_through_an_organization() {
        let mut s: Value = serde_json::from_str(&sync(
            plan(json!({ "plan": "free", "state": null, "storageGb": 5 })),
            json!([]),
        ))
        .unwrap();
        s["profile"]["premiumFromOrganization"] = true.into();
        let plans = Plans::from_sync(&s.to_string());
        let own = plans.own.unwrap();
        assert_eq!(own.plan.as_deref(), Some("free"));
        assert_eq!(own.state, None);
        assert!(plans.premium_from_organization);
    }

    #[test]
    fn reads_the_organizations_the_person_owns_or_is_an_admin_of_by_name() {
        let plans = Plans::from_sync(&sync(
            plan(json!({ "plan": "free", "state": null })),
            json!([
                { "id": "z", "name": "Zephyr", "type": 1, "keykeeperPlan": plan(json!({
                    "state": "ended", "storageGb": 0, "canManage": false,
                })) },
                { "id": "a", "name": "Acme Dental", "type": 0, "keykeeperPlan": plan(json!({
                    "state": "trial", "freeUntil": DEC_15, "freeWhy": "trial", "renews": false,
                })) },
                // A member's organization isn't listed.
                { "id": "m", "name": "Members", "type": 2, "keykeeperPlan": plan(json!({})) },
                // Nor one whose plan the server doesn't say.
                { "id": "n", "name": "No word", "type": 0 },
            ]),
        ));
        let listed: Vec<(&str, &str, Option<&str>)> = plans
            .organizations
            .iter()
            .map(|o| (o.name.as_str(), o.role.as_str(), o.plan.state.as_deref()))
            .collect();
        assert_eq!(
            listed,
            [
                ("Acme Dental", "owner", Some("trial")),
                ("Zephyr", "admin", Some("ended")),
            ]
        );
        let acme = &plans.organizations[0].plan;
        assert_eq!(acme.free_until.as_deref(), Some(DEC_15));
        assert_eq!(acme.free_why.as_deref(), Some("trial"));
        assert_eq!(acme.renews, Some(false));
        assert!(!plans.organizations[1].plan.can_manage);
    }

    #[test]
    fn reads_a_client_covered_by_its_msp() {
        let plans = Plans::from_sync(&sync(
            plan(json!({ "plan": "free", "state": null })),
            json!([{ "id": "c", "name": "Client", "type": 0, "keykeeperPlan": plan(json!({
                "plan": "msp", "canManage": false,
                "coveredBy": { "id": "msp-1", "name": "Contoso IT" },
            })) }]),
        ));
        let client = &plans.organizations[0].plan;
        assert_eq!(client.plan.as_deref(), Some("msp"));
        assert_eq!(
            client.covered_by,
            Some(CoveredBy {
                id: "msp-1".into(),
                name: "Contoso IT".into()
            })
        );
    }

    #[test]
    fn leaves_out_what_it_doesnt_understand() {
        let plans = Plans::from_sync(&sync(
            json!({
                "enforced": true, "plan": "platinum", "state": "frozen",
                "freeUntil": "soon", "freeWhy": "because", "until": 5, "renews": "yes",
                "endsAt": "2026-12-15", "storageGb": -1, "people": 2.5,
                "coveredBy": { "id": 7 }, "canManage": "true",
            }),
            json!("not a list"),
        ));
        assert!(plans.enforced);
        assert_eq!(plans.own, Some(Plan::default()));
        assert!(plans.organizations.is_empty());
    }

    #[test]
    fn says_it_as_settings_reads_it() {
        let plans = Plans::from_sync(&sync(
            plan(json!({ "plan": "premium", "until": DEC_15, "renews": true, "storageGb": 5 })),
            json!([]),
        ));
        assert_eq!(
            serde_json::to_value(&plans).unwrap(),
            json!({
                "enforced": true,
                "own": {
                    "plan": "premium", "state": "active", "freeUntil": null, "freeWhy": null,
                    "until": DEC_15, "renews": true, "endsAt": null, "storageGb": 5,
                    "people": null, "coveredBy": null, "canManage": true,
                },
                "premiumFromOrganization": false,
                "organizations": [],
            })
        );
        assert_eq!(
            serde_json::to_value(Plans::default()).unwrap(),
            json!({ "enforced": false, "own": null, "premiumFromOrganization": false, "organizations": [] })
        );
    }
}
