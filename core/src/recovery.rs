//! Enrolling in account recovery at unlock (DESIGN.md in the meta repo,
//! "Enrolling at unlock", "Our apps"). Upstream enrolls someone in an
//! organization's account recovery automatically only as they accept an
//! emailed invitation; where an organization turns its Account recovery
//! administration policy on with automatic enrollment after they joined, the
//! web vault, the extension, and these apps enroll them at their next
//! unlock, silently.
//!
//! The apps' code doesn't come from the server, so, like the extension, they
//! enroll only to an organization key something other than the server
//! vouches for: the install command's fingerprint of it (`-OrgKey`), where an
//! administrator put it on the device (managed.rs on computers; the MDM's
//! managed configuration on phones). Without one nothing is sent, not even a
//! request for the key. The key the server serves is taken only if the
//! SHA-256 of its SPKI, base64url without padding, is that pin, and the user
//! key is then wrapped to those same bytes exactly as upstream's enrollment
//! wraps it (RSA-OAEP with SHA-1, a type 4 string): no new crypto. A hacked
//! server can only keep someone from being enrolled.
//!
//! Which memberships are due, the pin, the check, and the wrap are here,
//! checked against the meta repo's `docs/vectors/recovery-at-unlock-v1.json`;
//! `App::recovery_at_unlock` asks the server and sends the enrollment.

use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::crypto::{encrypt_for, EncString, SymmetricKey};
use crate::error::Result;

/// Upstream's number for the Account recovery administration policy
/// (`PolicyType.ResetPassword`).
pub const ACCOUNT_RECOVERY_POLICY: i64 = 8;

/// Upstream's number for a confirmed membership.
const CONFIRMED: i64 = 2;

/// Said where there's an organization to enroll in but no pin on this
/// device: nothing is sent, not even a request for the key.
pub const NO_PIN: &str = "This device's install names no organization key, so PassBundle doesn't enroll you in account recovery. The web vault does, the next time you unlock it.";

/// Said where the server's key for an organization isn't the pinned one:
/// nothing is encrypted or sent.
pub const NOT_THE_PINNED_KEY: &str = "The organization's key from the server isn't the one this device's install names, so PassBundle didn't enroll you in its account recovery.";

/// A membership due to be enrolled: the organization, by id and name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Due {
    pub organization_id: String,
    pub name: String,
}

/// What happened in one organization at an unlock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Enrolled,
    /// No pin on this device (`NO_PIN`): nothing was sent.
    NoPin,
    /// The server's key isn't the pinned one (`NOT_THE_PINNED_KEY`):
    /// nothing was encrypted or sent.
    NotThePinnedKey,
    /// The server refused (an MSP's staff, say) or couldn't be reached, in
    /// the core's words (error.rs). The next unlock tries again.
    Failed(String),
}

impl Outcome {
    /// Why not, in a sentence; None once enrolled. Never a key, a wrapped
    /// key, or a fingerprint.
    pub fn why(&self) -> Option<String> {
        match self {
            Outcome::Enrolled => None,
            Outcome::NoPin => Some(NO_PIN.to_owned()),
            Outcome::NotThePinnedKey => Some(NOT_THE_PINNED_KEY.to_owned()),
            Outcome::Failed(why) => Some(why.clone()),
        }
    }
}

/// One organization's try at an unlock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tried {
    pub organization_id: String,
    pub name: String,
    pub outcome: Outcome,
}

/// The memberships in a sync (as the server sends it, or as it's kept here)
/// due to be enrolled, as the web vault and the extension decide it
/// (passbundle-clients, `dueForRecovery`): confirmed; not from an MSP (the
/// server never enrolls an MSP's staff, so their user key isn't sent); not
/// enrolled yet; in an organization that offers account recovery, by
/// upstream's checks (it uses policies, the server sends email, and the
/// organization has keys); and whose Account recovery administration policy
/// is on with automatic enrollment. Every role, owners and admins too.
pub fn due(sync: &Value) -> Vec<Due> {
    let policies = sync.get("policies").and_then(Value::as_array);
    let enrolls_everyone = |organization: &str| {
        policies.into_iter().flatten().any(|p| {
            p.get("organizationId").and_then(Value::as_str) == Some(organization)
                && p.get("type").and_then(Value::as_i64) == Some(ACCOUNT_RECOVERY_POLICY)
                && p.get("enabled").and_then(Value::as_bool) == Some(true)
                && p.pointer("/data/autoEnrollEnabled")
                    .and_then(Value::as_bool)
                    == Some(true)
        })
    };
    sync.pointer("/profile/organizations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|o| {
            let yes = |field: &str| o.get(field).and_then(Value::as_bool) == Some(true);
            o.get("status").and_then(Value::as_i64) == Some(CONFIRMED)
                && !yes("keykeeperFromMsp")
                && !yes("resetPasswordEnrolled")
                && yes("usePolicies")
                && yes("useResetPassword")
                && yes("hasPublicAndPrivateKeys")
        })
        .filter_map(|o| {
            let id = o.get("id")?.as_str()?;
            enrolls_everyone(id).then(|| Due {
                organization_id: id.to_owned(),
                name: o
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect()
}

/// The install command's fingerprint of an organization's public key (SPKI,
/// DER): its SHA-256, base64url without padding, as an owner's or admin's
/// browser makes it from the organization's private key (DESIGN.md in the
/// meta repo, "Joins a hacked server can't forge").
pub fn fingerprint(public_key: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(public_key))
}

/// A pin as an administrator set it, if it is one: 43 base64url characters,
/// as the install command carries it, with spaces around them trimmed.
/// Anything else is none, and with none nothing is sent.
pub fn pin(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (value.len() == 43
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
    .then(|| value.to_owned())
}

/// The organization's public key (SPKI, DER) from the server's answer to
/// `GET /api/organizations/<id>/keys` (`publicKey`, in base64), only if it's
/// the pinned one: the bytes it hashes to the pin are the bytes to wrap to.
pub fn pinned_key(answer: &Value, pin: &str) -> Option<Vec<u8>> {
    let key = STANDARD
        .decode(answer.get("publicKey")?.as_str()?)
        .ok()
        .filter(|key| !key.is_empty())?;
    (fingerprint(&key) == pin).then_some(key)
}

/// The user key wrapped to the pinned organization key exactly as upstream's
/// enrollment wraps it (`encapsulateKeyUnsigned`): RSA-OAEP with SHA-1 of
/// the 64-byte key, a type 4 string, with a new seed each time. Only ever
/// given the bytes `pinned_key` checked.
pub fn wrap(public_key: &[u8], user_key: &SymmetricKey) -> Result<EncString> {
    encrypt_for(public_key, &user_key.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::PrivateKey;
    use serde_json::json;

    fn vectors() -> Value {
        serde_json::from_str(include_str!("../tests/vectors/recovery-at-unlock-v1.json")).unwrap()
    }

    fn b64(v: &Value) -> Vec<u8> {
        STANDARD.decode(v.as_str().unwrap()).unwrap()
    }

    /// docs/vectors/recovery-at-unlock-v1.json in the meta repo, which plain
    /// Node made and upstream's own crypto checks.
    #[test]
    fn fingerprints_are_the_vectors() {
        let v = vectors();
        let organization = &v["organization"];
        assert_eq!(
            fingerprint(&b64(&organization["publicKey"])),
            organization["fingerprint"].as_str().unwrap()
        );
        assert_eq!(
            fingerprint(&b64(&v["other"]["publicKey"])),
            v["other"]["fingerprint"].as_str().unwrap()
        );
        assert_ne!(organization["fingerprint"], v["other"]["fingerprint"]);
        // The public key is the private key's.
        let private = PrivateKey::from_pkcs8_der(&b64(&organization["privateKey"])).unwrap();
        assert_eq!(private.public_key_der(), b64(&organization["publicKey"]));
    }

    #[test]
    fn only_the_organizations_own_key_is_taken_for_its_pin() {
        let v = vectors();
        for c in v["checks"].as_array().unwrap() {
            let answer = json!({ "object": "organizationPublicKey", "publicKey": c["publicKey"] });
            let taken = pinned_key(&answer, c["pin"].as_str().unwrap());
            assert_eq!(
                taken.is_some(),
                c["takes"].as_bool().unwrap(),
                "{}",
                c["why"]
            );
            if let Some(key) = taken {
                assert_eq!(key, b64(&c["publicKey"]), "{}", c["why"]);
            }
        }
        let pin = v["organization"]["fingerprint"].as_str().unwrap();
        // No key at all, or not a string.
        assert_eq!(pinned_key(&json!({}), pin), None);
        assert_eq!(pinned_key(&json!({ "publicKey": null }), pin), None);
        assert_eq!(pinned_key(&json!({ "publicKey": 7 }), pin), None);
    }

    #[test]
    fn a_pin_is_43_base64url_characters_and_nothing_else() {
        let v = vectors();
        for f in v["pinForms"].as_array().unwrap() {
            assert_eq!(
                pin(f["value"].as_str()).as_deref(),
                f["pin"].as_str(),
                "{}",
                f["why"]
            );
        }
        assert_eq!(pin(None), None);
    }

    #[test]
    fn the_vectors_wrap_and_ours_open_to_the_user_key_with_the_organizations_private_key() {
        let v = vectors();
        let private = PrivateKey::from_pkcs8_der(&b64(&v["organization"]["privateKey"])).unwrap();
        let user_key = SymmetricKey::from_bytes(&b64(&v["enrollment"]["userKey"])).unwrap();
        let theirs =
            EncString::parse(v["enrollment"]["resetPasswordKey"].as_str().unwrap()).unwrap();
        assert_eq!(&*private.decrypt(&theirs).unwrap(), &*user_key.to_bytes());

        // Ours: type 4 (RSA-OAEP with SHA-1), a new seed each time, and it
        // opens the same way.
        let public = b64(&v["organization"]["publicKey"]);
        let ours = wrap(&public, &user_key).unwrap();
        assert!(ours.to_string().starts_with("4."));
        assert_ne!(ours, wrap(&public, &user_key).unwrap());
        assert_eq!(&*private.decrypt(&ours).unwrap(), &*user_key.to_bytes());

        // Not to the other key's private key: that isn't the key it went to.
        let other = PrivateKey::generate().unwrap();
        assert!(other.decrypt(&ours).is_err());
    }

    /// A sync with one membership and its policy, everything that makes it
    /// due set; `edit` changes one thing.
    fn sync_with(edit: impl FnOnce(&mut Value)) -> Value {
        let mut sync = json!({
            "profile": {
                "id": "6a1f0c2e-8b3d-4c5e-9f60-7a8b9c0d1e2f",
                "organizations": [{
                    "id": "org-1", "name": "Contoso", "type": 2, "status": 2, "enabled": true,
                    "usePolicies": true, "useResetPassword": true, "hasPublicAndPrivateKeys": true,
                    "resetPasswordEnrolled": false, "keykeeperFromMsp": false,
                }],
            },
            "policies": [{
                "id": "p-1", "organizationId": "org-1", "type": 8, "enabled": true,
                "data": { "autoEnrollEnabled": true },
            }],
        });
        edit(&mut sync);
        sync
    }

    fn due_ids(sync: &Value) -> Vec<String> {
        due(sync).into_iter().map(|d| d.organization_id).collect()
    }

    #[test]
    fn a_confirmed_member_whose_organization_enrolls_everyone_is_due() {
        assert_eq!(
            due(&sync_with(|_| {})),
            vec![Due {
                organization_id: "org-1".into(),
                name: "Contoso".into()
            }]
        );
    }

    #[test]
    fn each_condition_alone_keeps_a_membership_from_being_due() {
        let org = "/profile/organizations/0";
        type Edit = Box<dyn Fn(&mut Value)>;
        let cases: Vec<(&str, Edit)> = vec![
            (
                "invited",
                Box::new(move |s| s.pointer_mut(org).unwrap()["status"] = 0.into()),
            ),
            (
                "accepted, not confirmed",
                Box::new(move |s| s.pointer_mut(org).unwrap()["status"] = 1.into()),
            ),
            (
                "revoked",
                Box::new(move |s| s.pointer_mut(org).unwrap()["status"] = (-1).into()),
            ),
            (
                "no status",
                Box::new(move |s| {
                    s.pointer_mut(org)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove("status");
                }),
            ),
            (
                "enrolled already",
                Box::new(move |s| {
                    s.pointer_mut(org).unwrap()["resetPasswordEnrolled"] = true.into()
                }),
            ),
            (
                "from an MSP",
                Box::new(move |s| s.pointer_mut(org).unwrap()["keykeeperFromMsp"] = true.into()),
            ),
            (
                "the organization doesn't use policies",
                Box::new(move |s| s.pointer_mut(org).unwrap()["usePolicies"] = false.into()),
            ),
            (
                "the server doesn't send email",
                Box::new(move |s| s.pointer_mut(org).unwrap()["useResetPassword"] = false.into()),
            ),
            (
                "the organization has no keys",
                Box::new(move |s| {
                    s.pointer_mut(org).unwrap()["hasPublicAndPrivateKeys"] = false.into()
                }),
            ),
            (
                "the policy off",
                Box::new(|s| s["policies"][0]["enabled"] = false.into()),
            ),
            (
                "automatic enrollment off",
                Box::new(|s| s["policies"][0]["data"]["autoEnrollEnabled"] = false.into()),
            ),
            (
                "automatic enrollment unsaid",
                Box::new(|s| s["policies"][0]["data"] = json!({})),
            ),
            ("no policy", Box::new(|s| s["policies"] = json!([]))),
            (
                "only another organization's policy",
                Box::new(|s| s["policies"][0]["organizationId"] = "org-2".into()),
            ),
            (
                "another policy, on with the same data",
                Box::new(|s| s["policies"][0]["type"] = 9.into()),
            ),
            (
                "no organizations",
                Box::new(|s| s["profile"]["organizations"] = json!([])),
            ),
        ];
        for (why, edit) in cases {
            assert_eq!(
                due_ids(&sync_with(|s| edit(s))),
                Vec::<String>::new(),
                "{why}"
            );
        }
    }

    #[test]
    fn every_role_is_due_owners_and_admins_too() {
        for role in 0..=4 {
            let sync = sync_with(|s| s["profile"]["organizations"][0]["type"] = role.into());
            assert_eq!(due_ids(&sync), vec!["org-1".to_owned()], "role {role}");
        }
    }

    #[test]
    fn only_the_organizations_that_enroll_everyone_are_due() {
        let sync = sync_with(|s| {
            let mut second = s["profile"]["organizations"][0].clone();
            second["id"] = "org-2".into();
            second["name"] = "Fabrikam".into();
            let mut third = second.clone();
            third["id"] = "org-3".into();
            let orgs = s["profile"]["organizations"].as_array_mut().unwrap();
            orgs.push(second);
            orgs.push(third);
            let mut policy = s["policies"][0].clone();
            policy["organizationId"] = "org-2".into();
            s["policies"].as_array_mut().unwrap().push(policy);
        });
        assert_eq!(due_ids(&sync), vec!["org-1".to_owned(), "org-2".to_owned()]);
    }
}
