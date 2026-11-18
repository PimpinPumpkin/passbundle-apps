//! Expiration dates and reminders (src/expiration.rs), and the person's own
//! logins' numbers for their organizations (src/health.rs, `own_counts`), on
//! the vectors' sample vault, with dates and reminder days set on its items
//! as the web vault sets them: as text fields encrypted with the item's key.

use std::collections::HashMap;

use passbundle_core::crypto::{EncString, SymmetricKey};
use passbundle_core::expiration::{
    self, check, expiring, mark_delivered, plan, schedule, Counts, Date, Remind, Schedule, Shown,
    ShownReminders, State, MOST_REMINDER_DAYS, REMIND_FIELD,
};
use passbundle_core::health::{own_counts, OwnCounts};
use passbundle_core::kdf::Prelogin;
use passbundle_core::vault::{Expiration, ItemDraft, Sync, Vault, EXPIRES_FIELD};

const MAIL: &str = "3d4e5f60-7182-4930-abc1-d2e3f4051627";
const WIFI: &str = "4e5f6071-8293-4a41-bcd2-e3f405162738";
const BACKUP: &str = "5f607182-93a4-4b52-cde3-f40516273849";
const GATE: &str = "60718293-a4b5-4c63-def4-05162738495a";
const CARD: &str = "718293a4-b5c6-4d74-8f01-162738495a6b";
const IDENTITY: &str = "8293a4b5-c6d7-4e85-9012-2738495a6b7c";

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

/// A phone's schedule as the days it says something, and how many.
fn said(schedule: &Schedule) -> Vec<(String, Counts)> {
    schedule
        .iter()
        .map(|(day, steps)| (day.clone(), expiration::counts(steps)))
        .collect()
}

fn c(soon: u32, expired: u32) -> Counts {
    Counts { soon, expired }
}

/// The days a phone reminds on while PassBundle is closed (docs/phones.md),
/// by the in-app rules: each item's own days, then once expired, each step
/// once for a date, nothing for `none`; several items on one day are one
/// day's count; today's, and what's further off than the item's days, wait.
#[test]
fn a_phone_schedules_each_coming_day_by_each_items_own_days() {
    let mut sample = Sample::new();
    // 30, 7, and 1 day before, by default.
    sample.field(MAIL, EXPIRES_FIELD, "2026-12-01");
    // 7 and 1 day before, and its date a week sooner.
    sample.field(GATE, EXPIRES_FIELD, "2026-11-20");
    sample.field(GATE, REMIND_FIELD, "7,1");
    // The same date as the gate code, with the default days.
    sample.field(IDENTITY, EXPIRES_FIELD, "2026-11-20");
    // "Don't remind me": never, even once it has expired.
    sample.field(CARD, EXPIRES_FIELD, "2026-11-20");
    sample.field(CARD, REMIND_FIELD, "none");
    let vault = sample.open();
    let today = day("2026-11-07");

    let ahead = schedule(&vault, &ShownReminders::new(), today);
    assert_eq!(
        said(&ahead),
        vec![
            // A week before the gate code and the identity: both.
            ("2026-11-13".into(), c(2, 0)),
            ("2026-11-19".into(), c(2, 0)),
            // Once they've expired.
            ("2026-11-21".into(), c(0, 2)),
            ("2026-11-24".into(), c(1, 0)),
            ("2026-11-30".into(), c(1, 0)),
            ("2026-12-02".into(), c(0, 1)),
        ]
    );
    // Today's are the app's own to say (Example Mail at 30 days, the
    // identity at 30), and the card says nothing.
    assert!(!ahead.contains_key("2026-11-07"));
    assert!(ahead.values().all(|steps| !steps.contains_key(CARD)));
    // What's kept is ids, dates, and steps, never a name.
    assert_eq!(
        ahead["2026-11-13"][GATE],
        Shown {
            date: "2026-11-20".into(),
            level: "7".into()
        }
    );
    let kept = serde_json::to_string(&ahead).unwrap();
    assert!(!kept.contains("Example Mail") && !kept.contains("Gate code"));

    // As the app says them, each day: the same days and counts.
    let mut shown = ShownReminders::new();
    let mut found = Vec::new();
    for ahead in 0..=60 {
        let on = today.plus_days(ahead).unwrap();
        let due = check(&vault, &mut shown, on).due;
        if ahead > 0 && due.total() > 0 {
            found.push((on.iso(), due));
        }
    }
    assert_eq!(
        found,
        said(&schedule(&vault, &ShownReminders::new(), today))
    );

    // All of it, as the phone gets it: from today, with its words.
    let days = plan([&ahead], today);
    assert_eq!(days.len(), 6);
    assert_eq!(days[0], (day("2026-11-13"), c(2, 0)));
    assert_eq!(
        expiration::words(days[2].1).as_deref(),
        Some("2 items have expired")
    );
}

/// What was shown already, and what's due today, aren't scheduled again;
/// a step further on still is.
#[test]
fn a_step_already_shown_or_due_today_is_not_scheduled_again() {
    let mut sample = Sample::new();
    sample.field(MAIL, EXPIRES_FIELD, "2026-12-01");
    sample.field(GATE, EXPIRES_FIELD, "2026-12-01");
    let vault = sample.open();
    // The app said Example Mail's week before, on the 24th.
    let mut shown = ShownReminders::new();
    shown.insert(
        MAIL.into(),
        Shown {
            date: "2026-12-01".into(),
            level: "7".into(),
        },
    );
    // The 26th: the gate code is due at 7 days today, unsaid as yet.
    let ahead = schedule(&vault, &shown, day("2026-11-26"));
    assert_eq!(
        said(&ahead),
        vec![
            ("2026-11-30".into(), c(2, 0)),
            ("2026-12-02".into(), c(0, 2)),
        ]
    );
    // Nothing is marked shown by working it out.
    assert_eq!(shown.len(), 1);
}

/// A day the phone said counts as shown, so the app doesn't say it again
/// that day; a day it didn't (it was off) the app says, as before; and a
/// date changed since the day was worked out is reminded about anew.
#[test]
fn what_the_phone_said_is_shown_and_what_it_didnt_the_app_says() {
    let mut sample = Sample::new();
    sample.field(MAIL, EXPIRES_FIELD, "2026-12-01");
    sample.field(GATE, EXPIRES_FIELD, "2026-11-20");
    sample.field(GATE, REMIND_FIELD, "7,1");
    let vault = sample.open();
    // Worked out on the 12th, with the 30-day step said in the app then.
    let mut shown = ShownReminders::new();
    check(&vault, &mut shown, day("2026-11-12"));
    let ahead = schedule(&vault, &shown, day("2026-11-12"));
    assert_eq!(said(&ahead)[0], ("2026-11-13".into(), c(1, 0)));

    // The 13th: the phone said it at 9:00, so the app opening later doesn't.
    let mut after = shown.clone();
    mark_delivered(
        &vault,
        &ahead,
        &[day("2026-11-13")],
        day("2026-11-13"),
        &mut after,
    );
    assert_eq!(check(&vault, &mut after, day("2026-11-13")).due, c(0, 0));
    // The phone was off at 9:00: the app says it, as it would have.
    let mut off = shown.clone();
    mark_delivered(&vault, &ahead, &[], day("2026-11-13"), &mut off);
    assert_eq!(check(&vault, &mut off, day("2026-11-13")).due, c(1, 0));
    // Said on the 13th, opened on the 19th: only the day before is new.
    let mut later = shown.clone();
    mark_delivered(
        &vault,
        &ahead,
        &[day("2026-11-13")],
        day("2026-11-19"),
        &mut later,
    );
    let found = check(&vault, &mut later, day("2026-11-19"));
    // (Example Mail stands at its 30 days, said on the 12th.)
    assert_eq!((found.due, found.standing), (c(1, 0), c(2, 0)));
    // A day that hasn't come yet marks nothing, whatever the phone says.
    let mut early = shown.clone();
    mark_delivered(
        &vault,
        &ahead,
        &[day("2026-11-19")],
        day("2026-11-13"),
        &mut early,
    );
    assert_eq!(early, shown);
    // And a delivered day never takes back a step further on.
    let mut further = shown.clone();
    further.insert(
        GATE.into(),
        Shown {
            date: "2026-11-20".into(),
            level: "expired".into(),
        },
    );
    let before = further.clone();
    mark_delivered(
        &vault,
        &ahead,
        &[day("2026-11-13")],
        day("2026-11-21"),
        &mut further,
    );
    assert_eq!(further, before);

    // Renewed after the 12th, somewhere else: the phone's 9:00 still said the
    // old date's week, but that's not this date's. A week before the new
    // date, the app reminds, and the phone works out the new date's days.
    let mut renewed = Sample::new();
    renewed.field(MAIL, EXPIRES_FIELD, "2026-12-01");
    renewed.field(GATE, EXPIRES_FIELD, "2026-11-27");
    renewed.field(GATE, REMIND_FIELD, "7,1");
    let now = renewed.open();
    let mut kept = shown.clone();
    mark_delivered(
        &now,
        &ahead,
        &[day("2026-11-13")],
        day("2026-11-14"),
        &mut kept,
    );
    assert!(!kept.contains_key(GATE));
    assert_eq!(check(&now, &mut kept, day("2026-11-14")).due, c(0, 0));
    assert_eq!(
        said(&schedule(&now, &kept, day("2026-11-14")))[0],
        ("2026-11-20".into(), c(1, 0))
    );
    assert_eq!(check(&now, &mut kept, day("2026-11-20")).due, c(1, 0));
}

/// At most 30 days with a reminder waiting (iOS keeps 64 notifications for
/// an app at most), the soonest, for one account or all of them.
#[test]
fn at_most_thirty_days_are_scheduled_the_soonest() {
    let mut sample = Sample::new();
    // A reminder each of the 40 days before, then once expired.
    let every: Vec<String> = (1..=40).rev().map(|d| d.to_string()).collect();
    sample.field(MAIL, EXPIRES_FIELD, "2026-12-20");
    sample.field(MAIL, REMIND_FIELD, &every.join(","));
    let vault = sample.open();
    let today = day("2026-11-07");
    let ahead = schedule(&vault, &ShownReminders::new(), today);
    assert_eq!(ahead.len(), MOST_REMINDER_DAYS);
    assert_eq!(ahead.keys().next().unwrap(), "2026-11-10");
    assert_eq!(ahead.keys().last().unwrap(), "2026-12-09");

    // Two accounts' together: still 30 days, the soonest of either.
    let mut other = Sample::new();
    other.field(GATE, EXPIRES_FIELD, "2026-11-09");
    other.field(GATE, REMIND_FIELD, "1");
    let theirs = schedule(&other.open(), &ShownReminders::new(), today);
    let days = plan([&ahead, &theirs], today);
    assert_eq!(days.len(), MOST_REMINDER_DAYS);
    assert_eq!(days[0], (day("2026-11-08"), c(1, 0)));
    assert_eq!(days[1], (day("2026-11-10"), c(1, 1)));
    assert_eq!(days[29].0, day("2026-12-08"));
}

/// Every account's days as one: each day's counts added up, from today
/// (what was waiting for a day gone by isn't said), never a day with
/// nothing to say.
#[test]
fn two_accounts_days_add_up_from_today() {
    let mut work = Sample::new();
    work.field(MAIL, EXPIRES_FIELD, "2026-12-01");
    let mut home = Sample::new();
    home.field(GATE, EXPIRES_FIELD, "2026-12-01");
    home.field(IDENTITY, EXPIRES_FIELD, "2026-11-25");
    home.field(IDENTITY, REMIND_FIELD, "1");
    let today = day("2026-11-07");
    let mut theirs = schedule(&work.open(), &ShownReminders::new(), today);
    let ours = schedule(&home.open(), &ShownReminders::new(), today);
    // A day gone by, and a day with something unreadable, kept from before.
    theirs.insert("2026-11-01".into(), theirs["2026-11-24"].clone());
    let mut odd = ShownReminders::new();
    odd.insert(
        MAIL.into(),
        Shown {
            date: "2026-12-01".into(),
            level: "soon".into(),
        },
    );
    theirs.insert("2026-11-08".into(), odd);
    assert_eq!(
        plan([&theirs, &ours], today),
        vec![
            (day("2026-11-24"), c(3, 0)),
            (day("2026-11-26"), c(0, 1)),
            (day("2026-11-30"), c(2, 0)),
            (day("2026-12-02"), c(0, 2)),
        ]
    );
    assert_eq!(
        plan([&theirs], day("2026-11-30")),
        vec![(day("2026-11-30"), c(1, 0)), (day("2026-12-02"), c(0, 1)),]
    );
}
