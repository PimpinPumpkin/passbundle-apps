//! Expiration dates and reminders (src/expiration.rs), and the person's own
//! logins' numbers for their organizations (src/health.rs, `own_counts`), on
//! the vectors' sample vault, with dates and reminder days set on its items
//! as the web vault sets them: as text fields encrypted with the item's key.

use std::collections::HashMap;

use passbundle_core::crypto::{EncString, SymmetricKey};
use passbundle_core::expiration::{
    self, check, expiring, Counts, Date, Remind, ShownReminders, State, REMIND_FIELD,
};
use passbundle_core::health::{own_counts, OwnCounts};
use passbundle_core::kdf::Prelogin;
use passbundle_core::vault::{Expiration, ItemDraft, Sync, Vault, EXPIRES_FIELD};

const MAIL: &str = "3d4e5f60-7182-4930-abc1-d2e3f4051627";
const WIFI: &str = "4e5f6071-8293-4a41-bcd2-e3f405162738";
const BACKUP: &str = "5f607182-93a4-4b52-cde3-f40516273849";
const GATE: &str = "60718293-a4b5-4c63-def4-05162738495a";
const CARD: &str = "718293a4-b5c6-4d74-8f01-162738495a6b";

struct Sample {
    v: serde_json::Value,
    prelogin: Prelogin,
    password: String,
    user_key: SymmetricKey,
}

impl Sample {
    fn new() -> Sample {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
        let prelogin: Prelogin =
            serde_json::from_value(v["account"]["pbkdf2"]["prelogin"].clone()).unwrap();
        let password = v["account"]["password"].as_str().unwrap().to_owned();
        use base64::{engine::general_purpose::STANDARD, Engine};
        let user_key = SymmetricKey::from_bytes(
            &STANDARD
                .decode(v["keys"]["userKey"].as_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        Sample {
            user_key,
            v,
            prelogin,
            password,
        }
    }

    fn cipher(&mut self, id: &str) -> &mut serde_json::Value {
        self.v["sync"]["ciphers"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|c| c["id"] == id)
            .unwrap()
    }

    /// A text field on one of the person's own items without a key of its own.
    fn field(&mut self, id: &str, name: &str, value: &str) {
        let enc = |text: &str| self.user_key.encrypt_str(text).to_string();
        let field = serde_json::json!({ "name": enc(name), "value": enc(value), "type": 0, "linkedId": null });
        let cipher = self.cipher(id);
        if cipher["fields"].is_null() {
            cipher["fields"] = serde_json::json!([]);
        }
        cipher["fields"].as_array_mut().unwrap().push(field);
    }

    /// A new password, saved as a new revision.
    fn password(&mut self, id: &str, password: &str) {
        let enc = self.user_key.encrypt_str(password).to_string();
        let cipher = self.cipher(id);
        cipher["login"]["password"] = enc.into();
        cipher["revisionDate"] = "2026-11-07T12:00:00.000Z".into();
    }

    fn open(&self) -> Vault {
        Vault::unlock(
            &Sync::from_json(&self.v["sync"].to_string()).unwrap(),
            &self.prelogin,
            &self.password,
        )
        .unwrap()
    }
}

fn day(text: &str) -> Date {
    Date::parse_iso(text).unwrap()
}

#[test]
fn an_item_reminds_at_each_of_its_days_once_and_once_it_has_expired() {
    let mut sample = Sample::new();
    sample.field(MAIL, EXPIRES_FIELD, "2026-12-01");
    sample.field(GATE, EXPIRES_FIELD, "2026-12-01");
    sample.field(GATE, REMIND_FIELD, "7,1");
    let vault = sample.open();
    let mut shown = ShownReminders::new();
    let on = |date: &str, shown: &mut ShownReminders| check(&vault, shown, day(date));
    let c = |soon, expired| Counts { soon, expired };

    // 40 days off: nothing yet. The card expires in 2029.
    assert_eq!(on("2026-10-22", &mut shown).due, c(0, 0));
    // 30 days before: the mail item, by default; the gate code waits for 7.
    let at30 = on("2026-11-01", &mut shown);
    assert_eq!((at30.due, at30.standing), (c(1, 0), c(1, 0)));
    // Asked again the same day, or the next: said once.
    assert_eq!(on("2026-11-01", &mut shown).due, c(0, 0));
    let next = on("2026-11-02", &mut shown);
    assert_eq!((next.due, next.standing), (c(0, 0), c(1, 0)));
    // 7 days before: both.
    assert_eq!(on("2026-11-24", &mut shown).due, c(2, 0));
    // The day before, then once expired.
    assert_eq!(on("2026-11-30", &mut shown).due, c(2, 0));
    let expired = on("2026-12-02", &mut shown);
    assert_eq!((expired.due, expired.standing), (c(0, 2), c(0, 2)));
    assert_eq!(on("2026-12-20", &mut shown).due, c(0, 0));
    assert_eq!(
        expiration::words(expired.due).as_deref(),
        Some("2 items have expired")
    );
    // What was shown is the item's id, the date, and the step: never a name.
    let kept = serde_json::to_string(&shown).unwrap();
    assert!(kept.contains(MAIL) && kept.contains("2026-12-01") && kept.contains("expired"));
    assert!(!kept.contains("Example Mail") && !kept.contains("Gate code"));
}

#[test]
fn a_new_date_starts_over_and_none_never_reminds() {
    let mut sample = Sample::new();
    sample.field(MAIL, EXPIRES_FIELD, "2026-12-01");
    let mut shown = ShownReminders::new();
    assert_eq!(
        check(&sample.open(), &mut shown, day("2026-11-30"))
            .due
            .soon,
        1
    );

    // Renewed: a year later, it starts over at 30 days.
    let mut renewed = Sample::new();
    renewed.field(MAIL, EXPIRES_FIELD, "2027-12-01");
    let vault = renewed.open();
    assert_eq!(check(&vault, &mut shown, day("2026-12-02")).due.total(), 0);
    assert_eq!(check(&vault, &mut shown, day("2027-11-01")).due.soon, 1);

    // "Don't remind me": nothing, even once it has expired.
    let mut never = Sample::new();
    never.field(MAIL, EXPIRES_FIELD, "2026-12-01");
    never.field(MAIL, REMIND_FIELD, "none");
    let vault = never.open();
    let mut shown = ShownReminders::new();
    for date in ["2026-11-01", "2026-11-30", "2026-12-02"] {
        let found = check(&vault, &mut shown, day(date));
        assert_eq!(
            (found.due, found.standing),
            (Counts::default(), Counts::default())
        );
    }
    // It's still expiring, in Health and Expiring soon.
    assert_eq!(
        expiring(&vault, day("2026-12-02"))[0].1.state,
        State::Expired
    );

    // A reminder value that isn't one counts as the default.
    let mut odd = Sample::new();
    odd.field(MAIL, EXPIRES_FIELD, "2026-12-01");
    odd.field(MAIL, REMIND_FIELD, "soon");
    let vault = odd.open();
    assert_eq!(Remind::of(vault.item(MAIL).unwrap()), Remind::default());
    assert_eq!(
        check(&vault, &mut ShownReminders::new(), day("2026-11-01"))
            .due
            .soon,
        1
    );
}

#[test]
fn expiring_soon_is_everything_expired_or_within_30_days_cards_included() {
    let mut sample = Sample::new();
    sample.field(MAIL, EXPIRES_FIELD, "2026-11-19");
    sample.field(GATE, EXPIRES_FIELD, "2026-11-02");
    sample.field(BACKUP, EXPIRES_FIELD, "2027-06-01");
    // The card's month (07/2029) counts once it's near.
    let vault = sample.open();
    let ids = |today: &str| -> Vec<(String, String)> {
        let mut found: Vec<_> = expiring(&vault, day(today))
            .into_iter()
            .map(|(i, s)| (i.id.clone(), expiration::label(s)))
            .collect();
        found.sort();
        found
    };
    assert_eq!(
        ids("2026-11-07"),
        vec![
            (MAIL.to_owned(), "Expires in 12 days".to_owned()),
            (GATE.to_owned(), "Expired 5 days ago".to_owned()),
        ]
    );
    let card = expiring(&vault, day("2029-07-20"));
    assert!(card
        .iter()
        .any(|(i, s)| i.id == CARD && expiration::label(*s) == "Expires in 11 days"));
    let view = expiration::view(vault.item(MAIL).unwrap(), day("2026-11-07"));
    assert_eq!(
        view,
        serde_json::json!({ "date": "2026-11-19", "state": "soon", "days": 12, "label": "Expires in 12 days" })
    );
    assert!(expiration::view(vault.item(WIFI).unwrap(), day("2026-11-07")).is_null());
}

fn opened(key: &SymmetricKey, text: &serde_json::Value) -> String {
    EncString::parse(text.as_str().unwrap())
        .unwrap()
        .decrypt_string(key)
        .unwrap()
        .as_str()
        .to_owned()
}

/// The fields of an item `encrypt_item` made, opened: (name, value).
fn fields_of(body: &serde_json::Value, key: &SymmetricKey) -> Vec<(String, String)> {
    body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (opened(key, &f["name"]), opened(key, &f["value"])))
        .collect()
}

#[test]
fn the_editor_sets_changes_and_clears_the_date_and_its_reminder_days() {
    let mut sample = Sample::new();
    sample.field(GATE, "keykeeper.fill", "{\"v\":1}");
    sample.field(GATE, EXPIRES_FIELD, "2026-12-01");
    sample.field(GATE, REMIND_FIELD, "7,1");
    let vault = sample.open();
    let key = sample.user_key.clone();
    let item = vault.item(GATE).unwrap();
    let ours = |body: &serde_json::Value| -> Vec<(String, String)> {
        fields_of(body, &key)
            .into_iter()
            .filter(|(n, _)| n.starts_with("keykeeper."))
            .collect()
    };

    // An edit that doesn't name them keeps them, as the web vault left them.
    let kept = vault
        .encrypt_item(&ItemDraft::of(item), Some(item))
        .unwrap();
    assert_eq!(
        ours(&kept),
        vec![
            ("keykeeper.fill".into(), "{\"v\":1}".into()),
            (EXPIRES_FIELD.into(), "2026-12-01".into()),
            (REMIND_FIELD.into(), "7,1".into()),
        ]
    );

    // A new date and days replace them; other fields of ours stay.
    let mut draft = ItemDraft::of(item);
    draft.expiration = Some(Expiration {
        date: Some(day("2027-03-31")),
        remind: Remind::parse("90,30,7,1").unwrap(),
    });
    assert_eq!(
        ours(&vault.encrypt_item(&draft, Some(item)).unwrap()),
        vec![
            ("keykeeper.fill".into(), "{\"v\":1}".into()),
            (EXPIRES_FIELD.into(), "2027-03-31".into()),
            (REMIND_FIELD.into(), "90,30,7,1".into()),
        ]
    );

    // The default days aren't stored; no date clears both.
    draft.expiration = Some(Expiration {
        date: Some(day("2027-03-31")),
        remind: Remind::default(),
    });
    assert_eq!(
        ours(&vault.encrypt_item(&draft, Some(item)).unwrap()),
        vec![
            ("keykeeper.fill".into(), "{\"v\":1}".into()),
            (EXPIRES_FIELD.into(), "2027-03-31".into()),
        ]
    );
    draft.expiration = Some(Expiration {
        date: None,
        remind: Remind::Never,
    });
    assert_eq!(
        ours(&vault.encrypt_item(&draft, Some(item)).unwrap()),
        vec![("keykeeper.fill".into(), "{\"v\":1}".into())]
    );

    // A new item gets them too.
    let new = ItemDraft {
        name: "Domain".into(),
        note: true,
        expiration: Some(Expiration {
            date: Some(day("2027-01-15")),
            remind: Remind::Never,
        }),
        ..ItemDraft::default()
    };
    assert_eq!(
        ours(&vault.encrypt_item(&new, None).unwrap()),
        vec![
            (EXPIRES_FIELD.into(), "2027-01-15".into()),
            (REMIND_FIELD.into(), "none".into()),
        ]
    );
}

#[test]
fn own_logins_are_counted_in_numbers_only_never_an_organizations() {
    let mut sample = Sample::new();
    let vault = sample.open();
    assert!(vault.in_an_organization());
    let mut weak = HashMap::new();
    // Example Mail and Backup service are the person's; Office Wi-Fi is Contoso's.
    assert_eq!(
        own_counts(&vault, &mut weak),
        OwnCounts {
            logins: 2,
            weak: 0,
            reused: 0
        }
    );
    assert_eq!(weak.len(), 2);

    sample.password(MAIL, "password1");
    assert_eq!(
        own_counts(&sample.open(), &mut weak),
        OwnCounts {
            logins: 2,
            weak: 1,
            reused: 0
        }
    );

    // Backup service's password, used again by Example Mail: both reused.
    let mut twice = Sample::new();
    twice.password(MAIL, "not-a-real-password-3");
    assert_eq!(own_counts(&twice.open(), &mut HashMap::new()).reused, 2);
    // Office Wi-Fi's password, used again in Personal, isn't "reused": the
    // organization's items aren't counted.
    let mut reused = Sample::new();
    reused.password(MAIL, "not-a-real-password-2");
    assert_eq!(own_counts(&reused.open(), &mut HashMap::new()).reused, 0);
    // In the trash, it doesn't count.
    let mut trashed = Sample::new();
    trashed.cipher(BACKUP)["deletedDate"] = "2026-11-01T00:00:00.000Z".into();
    assert_eq!(own_counts(&trashed.open(), &mut HashMap::new()).logins, 1);

    // What's sent is the numbers, and nothing else.
    let sent = serde_json::to_value(own_counts(&sample.open(), &mut HashMap::new())).unwrap();
    assert_eq!(
        sent,
        serde_json::json!({ "logins": 2, "weak": 1, "reused": 0 })
    );
}
