//! Two-step codes need Premium (src/codes.rs; DESIGN.md in the meta repo,
//! "Plans and billing", decision 2): on the vectors' sample vault kept on
//! this device, with a key on its shared login too, and the profile's
//! `premium` and `premiumFromOrganization` and the shared login's
//! `organizationUseTotp` as a server would send them. Where codes show, a
//! code shows, copies, and fills as before; where they don't, it's refused
//! with one sentence, and the key stays in the item for the editor.

use passbundle_core::app::App;
use passbundle_core::kdf::Prelogin;
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::store::{Settings, Store};
use passbundle_core::vault::{Sync, Vault};
use serde_json::Value;

/// The key put on the shared login.
const KEY: &str = "JBSWY3DPEHPK3PXP";

/// What the profile and the shared login say.
#[derive(Clone, Copy, Debug)]
struct Says {
    premium: Option<bool>,
    from_organization: Option<bool>,
    organization_gives_codes: Option<bool>,
}

struct Sample {
    dir: std::path::PathBuf,
    password: String,
    /// The personal login with a key.
    personal: String,
    /// The shared login, given a key here.
    shared: String,
}

fn cipher<'a>(v: &'a mut Value, id: &str) -> &'a mut Value {
    v["sync"]["ciphers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["id"] == id)
        .unwrap()
}

impl Sample {
    fn new(says: Says) -> Sample {
        let mut v: Value = serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
        let password = v["account"]["password"].as_str().unwrap().to_owned();
        let pre = v["account"]["pbkdf2"]["prelogin"].clone();
        let personal = v["ids"]["personal"].as_str().unwrap().to_owned();
        let shared = v["ids"]["shared"].as_str().unwrap().to_owned();
        let organization = v["ids"]["organization"].as_str().unwrap().to_owned();

        // A key on the shared login, encrypted with its organization's key
        // as the web vault would (the login has no key of its own).
        let prelogin: Prelogin = serde_json::from_value(pre.clone()).unwrap();
        let opened = Vault::unlock(
            &Sync::from_json(&v["sync"].to_string()).unwrap(),
            &prelogin,
            &password,
        )
        .unwrap();
        let key = opened.encrypt_collection_name(&organization, KEY).unwrap();
        let login = cipher(&mut v, &shared);
        login["login"]["totp"] = key.into();
        // The person may see its secrets, as PassBundle's server says it.
        login["edit"] = true.into();
        login["viewPassword"] = true.into();
        v["sync"]["collections"][0]["hidePasswords"] = false.into();
        let login = cipher(&mut v, &shared);
        if let Some(gives) = says.organization_gives_codes {
            login["organizationUseTotp"] = gives.into();
        }
        if let Some(premium) = says.premium {
            v["sync"]["profile"]["premium"] = premium.into();
        }
        if let Some(from) = says.from_organization {
            v["sync"]["profile"]["premiumFromOrganization"] = from.into();
        }

        let dir =
            std::env::temp_dir().join(format!("pb-core-codes-{}", uuid::Uuid::new_v4().simple()));
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
        store.save_sync(&v["sync"].to_string()).unwrap();
        Sample {
            dir,
            password,
            personal,
            shared,
        }
    }

    fn unlocked(&self) -> App {
        let mut app = App::open(
            Store::new(&self.dir),
            Box::new(MemorySecretStore::default()),
        );
        app.unlock(&self.password).unwrap();
        app
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Whether the login's code shows, checked everywhere the core gives one:
/// the rule, the view's `hasTotp` and `codeNeedsPremium`, and the current
/// code for copying and filling. Where it doesn't show, the key is still
/// there and the refusal is the one sentence.
fn shows(app: &App, id: &str) -> bool {
    let vault = app.vault().unwrap();
    let item = vault.item(id).unwrap();
    assert!(item.totp.is_some(), "the key stays in the item either way");
    let shown = vault.shows_code(item);
    assert_eq!(vault.has_code(item), shown);
    assert_eq!(vault.code_needs_premium(item), !shown);
    match app.value(id, "totp") {
        Ok(code) => {
            assert!(shown, "a code given where codes don't show");
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
            assert!(app.value_to_copy(id, "totp").is_ok());
        }
        Err(refused) => {
            assert!(!shown, "a code refused where codes show: {refused}");
            assert_eq!(
                refused.to_string(),
                "Two-step login codes are part of Premium."
            );
            assert_eq!(
                app.value_to_copy(id, "totp").unwrap_err().to_string(),
                "Two-step login codes are part of Premium."
            );
        }
    }
    // Nothing else is held back.
    assert!(app.value(id, "password").is_ok());
    assert!(app.value(id, "username").is_ok());
    shown
}

#[test]
fn codes_show_with_premium_of_their_own_or_through_an_organization_or_from_an_organization_that_gives_them(
) {
    let yes = Some(true);
    let no = Some(false);
    // (what the server says, personal login's code shows, shared login's code shows)
    let cases = [
        // Where plans aren't enforced, the server says everyone has Premium,
        // and every shared item's organization gives codes: as before.
        (
            Says {
                premium: yes,
                from_organization: no,
                organization_gives_codes: yes,
            },
            true,
            true,
        ),
        // Premium of their own, in an organization that doesn't give codes.
        (
            Says {
                premium: yes,
                from_organization: no,
                organization_gives_codes: no,
            },
            true,
            true,
        ),
        // Premium through an organization or an MSP.
        (
            Says {
                premium: no,
                from_organization: yes,
                organization_gives_codes: no,
            },
            true,
            true,
        ),
        // No Premium, but the shared login's organization gives codes.
        (
            Says {
                premium: no,
                from_organization: no,
                organization_gives_codes: yes,
            },
            false,
            true,
        ),
        // Neither.
        (
            Says {
                premium: no,
                from_organization: no,
                organization_gives_codes: no,
            },
            false,
            false,
        ),
        // Neither, from a server that leaves out what it doesn't give.
        (
            Says {
                premium: no,
                from_organization: None,
                organization_gives_codes: None,
            },
            false,
            false,
        ),
        // A server that says nothing (none we know of): as before.
        (
            Says {
                premium: None,
                from_organization: None,
                organization_gives_codes: None,
            },
            true,
            true,
        ),
    ];
    for (says, personal, shared) in cases {
        let sample = Sample::new(says);
        let app = sample.unlocked();
        assert_eq!(
            shows(&app, &sample.personal),
            personal,
            "{says:?}: the personal login"
        );
        assert_eq!(
            shows(&app, &sample.shared),
            shared,
            "{says:?}: the shared login"
        );
    }
}

#[test]
fn a_code_comes_back_with_premium_at_the_next_sync() {
    let free = Says {
        premium: Some(false),
        from_organization: Some(false),
        organization_gives_codes: Some(false),
    };
    let sample = Sample::new(free);
    assert!(!shows(&sample.unlocked(), &sample.personal));

    // Premium again: the sync the app keeps says so, and the same key's
    // code shows once the vault opens from it.
    let store = Store::new(&sample.dir);
    let mut sync: Value = serde_json::from_str(&store.sync().unwrap()).unwrap();
    sync["profile"]["premium"] = true.into();
    store.save_sync(&sync.to_string()).unwrap();
    let app = sample.unlocked();
    assert!(shows(&app, &sample.personal));
    assert!(shows(&app, &sample.shared));
}
