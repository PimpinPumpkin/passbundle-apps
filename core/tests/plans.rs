//! A person's plans, read from the sync kept on this device (src/plans.rs;
//! DESIGN.md in the meta repo, "Plans and billing"): what Settings shows, on
//! the vectors' sample vault with `keykeeperPlan` as PassBundle's server
//! sends it where it enforces plans, and nothing where it doesn't.

use passbundle_core::app::App;
use passbundle_core::plans::{CoveredBy, Plan, Plans};
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::store::{Settings, Store};
use serde_json::{json, Value};

/// The sample's organization, which the sample's person owns.
const CONTOSO: &str = "8f9a7c1e-3b2d-4e5f-9a8b-7c6d5e4f3a2b";

struct Sample {
    dir: std::path::PathBuf,
    password: String,
}

impl Sample {
    /// The sample vault kept on this device, its profile changed by `change`.
    fn new(change: impl FnOnce(&mut Value)) -> Sample {
        let mut v: Value = serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
        let pre = &v["account"]["pbkdf2"]["prelogin"];
        let dir =
            std::env::temp_dir().join(format!("pb-core-plans-{}", uuid::Uuid::new_v4().simple()));
        let store = Store::new(&dir);
        store
            .save_settings(&Settings {
                server: "https://vault.example.test".into(),
                email: v["account"]["salt"].as_str().unwrap().into(),
                kdf: 0,
                kdf_iterations: pre["kdfIterations"].as_u64().unwrap() as u32,
                kdf_memory: None,
                kdf_parallelism: None,
                device_identifier: "d".into(),
            })
            .unwrap();
        change(&mut v["sync"]["profile"]);
        store.save_sync(&v["sync"].to_string()).unwrap();
        Sample {
            dir,
            password: v["account"]["password"].as_str().unwrap().into(),
        }
    }

    fn app(&self) -> App {
        App::open(
            Store::new(&self.dir),
            Box::new(MemorySecretStore::default()),
        )
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_server_that_doesnt_enforce_plans_shows_nothing() {
    // As every self-hosted server sends it unless its operator turns
    // PLANS_ENFORCED on.
    let off = Sample::new(|profile| {
        profile["keykeeperPlan"] = json!({ "enforced": false });
        profile["organizations"][0]["keykeeperPlan"] = json!({ "enforced": false });
    });
    assert_eq!(off.app().plans(), Plans::default());
    // A server too old to say anything about plans.
    let old = Sample::new(|_| {});
    assert_eq!(old.app().plans(), Plans::default());
    // And nobody signed in.
    assert_eq!(App::nobody().plans(), Plans::default());
}

#[test]
fn the_plans_are_read_from_the_kept_sync_locked_or_not() {
    let sample = Sample::new(|profile| {
        profile["premiumFromOrganization"] = true.into();
        profile["keykeeperPlan"] = json!({
            "enforced": true, "plan": "free", "state": null, "freeUntil": null,
            "freeWhy": null, "until": null, "renews": null, "storageGb": 5,
            "people": null, "coveredBy": null, "canManage": true,
        });
        profile["organizations"][0]["keykeeperPlan"] = json!({
            "enforced": true, "plan": "business", "state": "trial",
            "freeUntil": "2026-12-15T12:00:00.000000Z", "freeWhy": "trial",
            "until": null, "renews": false, "storageGb": 5, "people": null,
            "coveredBy": null, "canManage": true,
        });
    });
    let mut app = sample.app();
    let locked = app.plans();
    app.unlock(&sample.password).unwrap();
    let unlocked = app.plans();
    assert_eq!(locked, unlocked);

    assert!(unlocked.enforced);
    assert!(unlocked.premium_from_organization);
    assert_eq!(
        unlocked.own,
        Some(Plan {
            plan: Some("free".into()),
            storage_gb: Some(5),
            can_manage: true,
            ..Plan::default()
        })
    );
    assert_eq!(unlocked.organizations.len(), 1);
    let contoso = &unlocked.organizations[0];
    assert_eq!(
        (
            contoso.id.as_str(),
            contoso.name.as_str(),
            contoso.role.as_str()
        ),
        (CONTOSO, "Contoso", "owner")
    );
    assert_eq!(
        contoso.plan,
        Plan {
            plan: Some("business".into()),
            state: Some("trial".into()),
            free_until: Some("2026-12-15T12:00:00.000000Z".into()),
            free_why: Some("trial".into()),
            renews: Some(false),
            storage_gb: Some(5),
            can_manage: true,
            ..Plan::default()
        }
    );
}

#[test]
fn a_client_of_an_msp_whose_pool_has_ended_says_so() {
    let sample = Sample::new(|profile| {
        profile["keykeeperPlan"] = json!({ "enforced": true, "plan": "free", "canManage": true });
        profile["organizations"][0]["keykeeperPlan"] = json!({
            "enforced": true, "plan": "msp", "state": "ended", "storageGb": 0,
            "coveredBy": { "id": "msp-1", "name": "Contoso IT" }, "canManage": false,
            "endsAt": "2026-11-29T12:00:00.000000Z",
        });
    });
    let plans = sample.app().plans();
    let client = &plans.organizations[0].plan;
    assert_eq!(client.state.as_deref(), Some("ended"));
    assert_eq!(
        client.ends_at.as_deref(),
        Some("2026-11-29T12:00:00.000000Z")
    );
    assert_eq!(
        client.covered_by,
        Some(CoveredBy {
            id: "msp-1".into(),
            name: "Contoso IT".into()
        })
    );
    assert!(!client.can_manage);
}
