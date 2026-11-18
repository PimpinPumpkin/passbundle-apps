//! Expiration dates and their reminders (DESIGN.md in the meta repo,
//! "Expiration dates"), by the same rules as the web vault and the browser
//! extension (passbundle-clients, libs/common/src/vault/keykeeper/expiration.ts
//! and apps/browser/src/keykeeper/expiration-reminders.ts). Any item can
//! expire on a date, kept on the item as a text field, `keykeeper.expires`
//! ("YYYY-MM-DD"); a card also expires at the end of its expiry month. An
//! item may say when to be reminded, in `keykeeper.remind`: whole days before
//! the date ("90,30,7,1"), or `none`. Nothing here leaves the device: a
//! reminder says how many items, never which.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::vault::{Item, Vault, EXPIRES_FIELD};

/// When to be reminded, kept beside the date: `none`, or whole days before
/// it, comma-separated. Only stored when it isn't the default.
pub const REMIND_FIELD: &str = "keykeeper.remind";

/// Items count as expiring soon this many days ahead.
pub const SOON_DAYS: i64 = 30;

/// The days before the date an item reminds at, unless it says otherwise.
pub const DEFAULT_DAYS: [u16; 3] = [30, 7, 1];

/// The furthest ahead a reminder can be.
pub const MOST_DAYS: u16 = 365;

/// A day on the calendar, without a time or a time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    }
}

impl Date {
    /// A real date, or None. Years before 100 aren't, as the web vault reads
    /// them (JavaScript's dates take them as 1900 and later).
    pub fn new(year: i32, month: u32, day: u32) -> Option<Date> {
        ((100..=9999).contains(&year)
            && (1..=12).contains(&month)
            && day >= 1
            && day <= days_in_month(year, month))
        .then_some(Date { year, month, day })
    }

    /// "YYYY-MM-DD", exactly, as the field holds it.
    pub fn parse_iso(text: &str) -> Option<Date> {
        let b = text.as_bytes();
        if !text.is_ascii() || b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let digits = |range: std::ops::Range<usize>| -> Option<u32> {
            let part = &text[range];
            part.bytes()
                .all(|c| c.is_ascii_digit())
                .then(|| part.parse().ok())?
        };
        Date::new(digits(0..4)? as i32, digits(5..7)?, digits(8..10)?)
    }

    /// As the field holds it: "YYYY-MM-DD".
    pub fn iso(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// The last day of a month.
    pub fn last_of_month(year: i32, month: u32) -> Option<Date> {
        Date::new(year, month, days_in_month(year, month))
    }

    /// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
    fn ordinal(&self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// The date so many days since 1970-01-01 (`civil_from_days`, the other
    /// way); None outside the years `new` takes.
    fn from_ordinal(days: i64) -> Option<Date> {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = i32::try_from(yoe + era * 400 + i64::from(month <= 2)).ok()?;
        Date::new(year, month, day)
    }

    /// Whole days from this date to `later`; negative when it's earlier.
    pub fn days_until(&self, later: Date) -> i64 {
        later.ordinal() - self.ordinal()
    }

    /// The date `days` after this one (before it, when negative), or None
    /// past the years `new` takes.
    pub fn plus_days(&self, days: i64) -> Option<Date> {
        Date::from_ordinal(self.ordinal() + days)
    }

    /// Today, where this device is.
    pub fn today() -> Date {
        local_today().unwrap_or_else(utc_today)
    }
}

#[cfg(unix)]
fn local_today() -> Option<Date> {
    // SAFETY: time(NULL) has no preconditions; localtime_r writes only the
    // tm it's given, which lives for the call.
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return None;
    }
    Date::new(tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32)
}

#[cfg(windows)]
fn local_today() -> Option<Date> {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: GetLocalTime only writes the SYSTEMTIME it's given.
    let mut now: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut now) };
    Date::new(
        i32::from(now.wYear),
        u32::from(now.wMonth),
        u32::from(now.wDay),
    )
}

#[cfg(not(any(unix, windows)))]
fn local_today() -> Option<Date> {
    None
}

fn utc_today() -> Date {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    Date::from_ordinal(days).unwrap_or(Date {
        year: 1970,
        month: 1,
        day: 1,
    })
}

/// The date stored on an item, when it has a valid one (the first field of
/// that name, as the web vault reads it).
pub fn stored(item: &Item) -> Option<Date> {
    item.fields
        .iter()
        .find(|f| f.name == EXPIRES_FIELD)
        .and_then(|f| Date::parse_iso(f.value.as_str()))
}

/// A card expires at the end of its expiry month; a two-digit year is this
/// century's.
pub fn card_expiry(month: Option<&str>, year: Option<&str>) -> Option<Date> {
    let month: i64 = month?.trim().parse().ok()?;
    let mut year: i64 = year?.trim().parse().ok()?;
    if !(1..=12).contains(&month) || year <= 0 {
        return None;
    }
    if year < 100 {
        year += 2000;
    }
    Date::last_of_month(i32::try_from(year).ok()?, month as u32)
}

/// When an item expires: its stored date, else a card's expiry, else never.
pub fn expiry(item: &Item) -> Option<Date> {
    stored(item).or_else(|| {
        item.card
            .as_ref()
            .and_then(|c| card_expiry(c.exp_month.as_deref(), c.exp_year.as_deref()))
    })
}

/// Where an expiry date stands, from today.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Expired,
    Today,
    /// Within `SOON_DAYS`.
    Soon,
    Later,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    /// Whole days from today to the date; negative once it's passed.
    pub days: i64,
}

pub fn status(expires: Date, today: Date) -> Status {
    let days = today.days_until(expires);
    let state = if days < 0 {
        State::Expired
    } else if days == 0 {
        State::Today
    } else if days <= SOON_DAYS {
        State::Soon
    } else {
        State::Later
    };
    Status { state, days }
}

/// What a badge says: "Expires in 12 days", "Expired 5 days ago".
pub fn label(status: Status) -> String {
    match status.state {
        State::Expired if status.days == -1 => "Expired yesterday".into(),
        State::Expired => format!("Expired {} days ago", -status.days),
        State::Today => "Expires today".into(),
        _ if status.days == 1 => "Expires tomorrow".into(),
        _ => format!("Expires in {} days", status.days),
    }
}

/// An item's expiry, for the screens: `{date, state, days, label}`, or null
/// when it never expires.
pub fn view(item: &Item, today: Date) -> Value {
    match expiry(item) {
        Some(date) => {
            let s = status(date, today);
            json!({ "date": date.iso(), "state": s.state, "days": s.days, "label": label(s) })
        }
        None => Value::Null,
    }
}

/// When an item reminds: never, or at these days before its date, largest
/// first (and always once it has expired).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Remind {
    Never,
    Before(Vec<u16>),
}

impl Default for Remind {
    fn default() -> Self {
        Remind::Before(DEFAULT_DAYS.to_vec())
    }
}

impl Remind {
    /// What the field says: `none`, or whole days from 1 to `MOST_DAYS`,
    /// comma-separated, in any order, each once. Anything else isn't one.
    pub fn parse(text: &str) -> Option<Remind> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("none") {
            return Some(Remind::Never);
        }
        let mut days = Vec::new();
        for part in text.split(',') {
            let part = part.trim();
            if part.is_empty() || part.len() > 3 || !part.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let day: u16 = part.parse().ok()?;
            if !(1..=MOST_DAYS).contains(&day) {
                return None;
            }
            days.push(day);
        }
        days.sort_unstable_by(|a, b| b.cmp(a));
        days.dedup();
        Some(Remind::Before(days))
    }

    /// The item's: its field's when that's well formed, else the default.
    pub fn of(item: &Item) -> Remind {
        item.fields
            .iter()
            .find(|f| f.name == REMIND_FIELD)
            .and_then(|f| Remind::parse(f.value.as_str()))
            .unwrap_or_default()
    }

    pub fn is_default(&self) -> bool {
        *self == Remind::default()
    }

    /// What the field holds: None for the default, which isn't stored.
    pub fn stored(&self) -> Option<String> {
        match self {
            _ if self.is_default() => None,
            Remind::Never => Some("none".into()),
            Remind::Before(days) => Some(
                days.iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        }
    }

    /// As the editor and the item's view say it: "30, 7, and 1 day before",
    /// "7 and 1 day before", "90 days before", "Don't remind me".
    pub fn words(&self) -> String {
        let days = match self {
            Remind::Never => return "Don't remind me".into(),
            Remind::Before(days) => days,
        };
        let Some(last) = days.last() else {
            return "Only once it has expired".into();
        };
        let unit = if *last == 1 { "day" } else { "days" };
        let numbers: Vec<String> = days.iter().map(u16::to_string).collect();
        let listed = match numbers.len() {
            1 => numbers[0].clone(),
            2 => format!("{} and {}", numbers[0], numbers[1]),
            n => format!("{}, and {}", numbers[..n - 1].join(", "), numbers[n - 1]),
        };
        format!("{listed} {unit} before")
    }
}

/// The reminder an item is due for: one of its days before the date, or
/// once it has expired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Before(u16),
    Expired,
}

impl Step {
    /// As what was shown is kept: "30", "7", "1", "expired", as the
    /// extension keeps it.
    pub fn key(self) -> String {
        match self {
            Step::Before(days) => days.to_string(),
            Step::Expired => "expired".into(),
        }
    }

    fn from_key(key: &str) -> Option<Step> {
        if key == "expired" {
            return Some(Step::Expired);
        }
        key.parse().ok().map(Step::Before)
    }

    /// Lower is more urgent: once expired, then the fewest days before.
    fn urgency(self) -> u32 {
        match self {
            Step::Expired => 0,
            Step::Before(days) => u32::from(days) + 1,
        }
    }
}

/// The step an item is at: once expired, else the fewest of its days that
/// it's within; none when it's further off than all of them, or never reminds.
pub fn step(status: Status, remind: &Remind) -> Option<Step> {
    let Remind::Before(days) = remind else {
        return None;
    };
    if status.days < 0 {
        return Some(Step::Expired);
    }
    days.iter()
        .copied()
        .filter(|&d| status.days <= i64::from(d))
        .min()
        .map(Step::Before)
}

/// What was last shown for an item: the expiry date it was for, and the step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shown {
    pub date: String,
    pub level: String,
}

/// What was shown, by item id: kept per account on this device, and
/// forgotten at sign-out (store.rs).
pub type ShownReminders = BTreeMap<String, Shown>;

/// Whether to remind: a step more urgent than the last one shown for the
/// same date. A new date starts over.
pub fn should_remind(shown: Option<&Shown>, date: &str, step: Step) -> bool {
    match shown {
        Some(s) if s.date == date => {
            Step::from_key(&s.level).is_none_or(|was| step.urgency() < was.urgency())
        }
        _ => true,
    }
}

/// How many items expire soon (at one of their days before) and how many
/// have expired.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub soon: u32,
    pub expired: u32,
}

impl Counts {
    pub fn total(self) -> u32 {
        self.soon + self.expired
    }

    fn add(&mut self, step: Step) {
        match step {
            Step::Expired => self.expired += 1,
            Step::Before(_) => self.soon += 1,
        }
    }
}

/// What a check found: the reminders due now (`due`, each shown once), and
/// every item at one of its steps, shown before or not (`standing`, what the
/// desktop agent's tray says).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Check {
    pub due: Counts,
    pub standing: Counts,
}

/// The items a person is reminded about: theirs and their organizations',
/// in use (not in the trash or archived), and not an MSP's template copies
/// (which only their Templates folder shows).
fn remindable(item: &Item) -> bool {
    item.in_use() && !crate::templates::is_msp_copy(item)
}

/// Looks for reminders due today and marks them shown in `shown`, which
/// also forgets items no longer in the vault.
pub fn check(vault: &Vault, shown: &mut ShownReminders, today: Date) -> Check {
    let mut found = Check::default();
    for item in vault.items.iter().filter(|i| remindable(i)) {
        let Some(date) = expiry(item) else { continue };
        let Some(step) = step(status(date, today), &Remind::of(item)) else {
            continue;
        };
        found.standing.add(step);
        let iso = date.iso();
        if should_remind(shown.get(&item.id), &iso, step) {
            shown.insert(
                item.id.clone(),
                Shown {
                    date: iso,
                    level: step.key(),
                },
            );
            found.due.add(step);
        }
    }
    shown.retain(|id, _| vault.items.iter().any(|i| &i.id == id));
    found
}

/// How far ahead a phone's reminders are worked out, in days.
pub const SCHEDULE_DAYS: i64 = 60;

/// The most days a phone has a reminder waiting for, in all: iOS keeps at
/// most 64 waiting notifications for an app.
pub const MOST_REMINDER_DAYS: usize = 30;

/// The reminders a phone says while PassBundle is closed (DESIGN.md,
/// "Reminders, on the device"): for each day ("YYYY-MM-DD"), the steps its
/// notification covers, by item id, as what was shown is kept. Ids, dates,
/// and steps, never a name. Kept per account on this device (store.rs).
pub type Schedule = BTreeMap<String, ShownReminders>;

/// The reminders due on each day after `today`, up to `SCHEDULE_DAYS`
/// ahead, by the same rules as `check`: each item's own days, then once
/// expired, each step once for a date, nothing for `none`. What was shown
/// counts as said, and so does what's due today, which is the app's own to
/// say (`check`). At most `MOST_REMINDER_DAYS` days, the soonest.
pub fn schedule(vault: &Vault, shown: &ShownReminders, today: Date) -> Schedule {
    let items: Vec<(&Item, Date, String, Remind)> = vault
        .items
        .iter()
        .filter(|i| remindable(i))
        .filter_map(|i| {
            let date = expiry(i)?;
            Some((i, date, date.iso(), Remind::of(i)))
        })
        .filter(|(_, _, _, remind)| *remind != Remind::Never)
        .collect();
    let mut said = shown.clone();
    let mut found = Schedule::new();
    for ahead in 0..=SCHEDULE_DAYS {
        let Some(day) = today.plus_days(ahead) else {
            break;
        };
        let mut due = ShownReminders::new();
        for (item, date, iso, remind) in &items {
            let Some(step) = step(status(*date, day), remind) else {
                continue;
            };
            if should_remind(said.get(&item.id), iso, step) {
                let at = Shown {
                    date: iso.clone(),
                    level: step.key(),
                };
                said.insert(item.id.clone(), at.clone());
                if ahead > 0 {
                    due.insert(item.id.clone(), at);
                }
            }
        }
        if !due.is_empty() {
            found.insert(day.iso(), due);
            if found.len() == MOST_REMINDER_DAYS {
                break;
            }
        }
    }
    found
}

/// How many items a day's reminders are about, and whether each expires
/// soon or has expired.
pub fn counts(steps: &ShownReminders) -> Counts {
    let mut counts = Counts::default();
    for shown in steps.values() {
        if let Some(step) = Step::from_key(&shown.level) {
            counts.add(step);
        }
    }
    counts
}

/// The phone said these days' reminders (`delivered`): each step one of
/// them covered (`scheduled`, up to `today`) counts as shown, so the app
/// doesn't say it again. Only for the date the item has now (one changed
/// since is reminded about as a new date), and only a step more urgent than
/// what was shown. A day the phone didn't say (it was off) isn't marked, and
/// `check` says what's due then, as before.
pub fn mark_delivered(
    vault: &Vault,
    scheduled: &Schedule,
    delivered: &[Date],
    today: Date,
    shown: &mut ShownReminders,
) {
    for day in delivered.iter().filter(|d| **d <= today) {
        let Some(steps) = scheduled.get(&day.iso()) else {
            continue;
        };
        for (id, at) in steps {
            let Some(item) = vault.item(id) else {
                continue;
            };
            if expiry(item).map(|d| d.iso()).as_deref() != Some(at.date.as_str()) {
                continue;
            }
            let Some(step) = Step::from_key(&at.level) else {
                continue;
            };
            if should_remind(shown.get(id), &at.date, step) {
                shown.insert(id.clone(), at.clone());
            }
        }
    }
}

/// Every account's schedule as one, for the phone to say: from `today` on,
/// each day's counts added up across accounts, the soonest
/// `MOST_REMINDER_DAYS` days.
pub fn plan<'a>(
    schedules: impl IntoIterator<Item = &'a Schedule>,
    today: Date,
) -> Vec<(Date, Counts)> {
    let mut days: BTreeMap<Date, Counts> = BTreeMap::new();
    for schedule in schedules {
        for (day, steps) in schedule {
            let Some(date) = Date::parse_iso(day).filter(|d| *d >= today) else {
                continue;
            };
            let found = counts(steps);
            if found.total() == 0 {
                continue;
            }
            let sum = days.entry(date).or_default();
            sum.soon += found.soon;
            sum.expired += found.expired;
        }
    }
    days.into_iter().take(MOST_REMINDER_DAYS).collect()
}

/// The items expired or expiring within `SOON_DAYS`, whatever their
/// reminder days: the "Expiring soon" list and Health.
pub fn expiring(vault: &Vault, today: Date) -> Vec<(&Item, Status)> {
    vault
        .items
        .iter()
        .filter(|i| remindable(i))
        .filter_map(|i| {
            let s = status(expiry(i)?, today);
            (s.state != State::Later).then_some((i, s))
        })
        .collect()
}

/// What a reminder says: how many items, and whether they expire soon or
/// already have. Never an item's name. None for none.
pub fn words(counts: Counts) -> Option<String> {
    let count = counts.total();
    if count == 0 {
        return None;
    }
    let items = if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    };
    Some(if counts.soon == 0 {
        format!(
            "{items} {}",
            if count == 1 {
                "has expired"
            } else {
                "have expired"
            }
        )
    } else if counts.expired == 0 {
        format!(
            "{items} {}",
            if count == 1 {
                "expires soon"
            } else {
                "expire soon"
            }
        )
    } else {
        format!("{items} expire soon or have expired")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Date {
        Date::parse_iso(text).unwrap()
    }

    #[test]
    fn dates_are_read_exactly_as_the_web_vault_reads_them() {
        assert_eq!(d("2026-11-07").iso(), "2026-11-07");
        for bad in [
            "2026-02-30",
            "2026-13-01",
            "2026-00-10",
            "2026-1-5",
            "26-01-05",
            " 2026-01-05",
            "2026-01-05T00:00",
            "0099-01-01",
            "２０２６-01-05",
            "",
        ] {
            assert_eq!(Date::parse_iso(bad), None, "{bad}");
        }
        assert!(Date::parse_iso("2028-02-29").is_some());
        assert!(Date::parse_iso("2100-02-29").is_none());
        assert!(Date::parse_iso("2000-02-29").is_some());
    }

    #[test]
    fn days_between_dates_count_whole_days_across_months_and_years() {
        assert_eq!(d("2026-11-07").days_until(d("2026-11-19")), 12);
        assert_eq!(d("2026-12-31").days_until(d("2027-01-01")), 1);
        assert_eq!(d("2028-02-28").days_until(d("2028-03-01")), 2);
        assert_eq!(d("2026-11-07").days_until(d("2026-11-02")), -5);
        assert_eq!(d("1970-01-01").ordinal(), 0);
        // And back: the dates so many days on, across months, leap days, and years.
        assert_eq!(d("2026-11-07").plus_days(25), Some(d("2026-12-02")));
        assert_eq!(d("2028-02-28").plus_days(1), Some(d("2028-02-29")));
        assert_eq!(d("2026-12-31").plus_days(60), Some(d("2027-03-01")));
        assert_eq!(d("2026-01-05").plus_days(-6), Some(d("2025-12-30")));
        assert_eq!(d("9999-12-31").plus_days(1), None);
        for days in [-600_000, -1, 0, 1, 20_765, 2_900_000] {
            assert_eq!(Date::from_ordinal(days).map(|d| d.ordinal()), Some(days));
        }
        let today = Date::today();
        assert_eq!(today.days_until(today), 0);
        assert!(utc_today().days_until(today).abs() <= 1);
    }

    #[test]
    fn a_card_expires_at_the_end_of_its_month() {
        assert_eq!(card_expiry(Some("7"), Some("2029")), Some(d("2029-07-31")));
        assert_eq!(card_expiry(Some("02"), Some("28")), Some(d("2028-02-29")));
        assert_eq!(
            card_expiry(Some(" 12 "), Some("2027")),
            Some(d("2027-12-31"))
        );
        for (m, y) in [
            ("13", "2029"),
            ("0", "2029"),
            ("", "2029"),
            ("7", ""),
            ("7", "-1"),
            ("x", "2029"),
        ] {
            assert_eq!(card_expiry(Some(m), Some(y)), None, "{m}/{y}");
        }
        assert_eq!(card_expiry(None, Some("2029")), None);
    }

    #[test]
    fn status_and_labels_say_what_the_web_vault_says() {
        let today = d("2026-11-07");
        let says = |date: &str| label(status(d(date), today));
        assert_eq!(says("2026-11-19"), "Expires in 12 days");
        assert_eq!(says("2026-11-08"), "Expires tomorrow");
        assert_eq!(says("2026-11-07"), "Expires today");
        assert_eq!(says("2026-11-06"), "Expired yesterday");
        assert_eq!(says("2026-11-02"), "Expired 5 days ago");
        assert_eq!(status(d("2026-12-07"), today).state, State::Soon);
        assert_eq!(status(d("2026-12-08"), today).state, State::Later);
        assert_eq!(status(d("2026-11-07"), today).state, State::Today);
        assert_eq!(status(d("2026-11-06"), today).state, State::Expired);
    }

    #[test]
    fn reminder_days_are_read_in_any_order_each_once_or_not_at_all() {
        assert_eq!(
            Remind::parse("90,30,7,1"),
            Some(Remind::Before(vec![90, 30, 7, 1]))
        );
        assert_eq!(
            Remind::parse("1, 7,30,7"),
            Some(Remind::Before(vec![30, 7, 1]))
        );
        assert_eq!(Remind::parse("none"), Some(Remind::Never));
        assert_eq!(Remind::parse(" NONE "), Some(Remind::Never));
        assert_eq!(Remind::parse("365"), Some(Remind::Before(vec![365])));
        for bad in [
            "", "0", "366", "-1", "7,", ",7", "7;1", "seven", "1.5", "+7", "0007", "7,,1",
        ] {
            assert_eq!(Remind::parse(bad), None, "{bad:?}");
        }
        assert!(Remind::default().is_default());
        assert_eq!(Remind::parse("1,7,30").unwrap().stored(), None);
        assert_eq!(Remind::Never.stored().as_deref(), Some("none"));
        assert_eq!(
            Remind::parse("7,90,30,1").unwrap().stored().as_deref(),
            Some("90,30,7,1")
        );
    }

    #[test]
    fn reminder_days_are_said_in_plain_words() {
        assert_eq!(Remind::default().words(), "30, 7, and 1 day before");
        assert_eq!(
            Remind::parse("90,30,7,1").unwrap().words(),
            "90, 30, 7, and 1 day before"
        );
        assert_eq!(Remind::parse("7,1").unwrap().words(), "7 and 1 day before");
        assert_eq!(
            Remind::parse("60,14").unwrap().words(),
            "60 and 14 days before"
        );
        assert_eq!(Remind::parse("1").unwrap().words(), "1 day before");
        assert_eq!(Remind::Never.words(), "Don't remind me");
    }

    #[test]
    fn an_item_is_at_the_fewest_of_its_days_it_is_within() {
        let today = d("2026-11-07");
        let at = |date: &str, remind: &Remind| step(status(d(date), today), remind);
        let default = Remind::default();
        // As the extension: 30, 7, and 1 day before, and once expired.
        assert_eq!(at("2026-12-08", &default), None);
        assert_eq!(at("2026-12-07", &default), Some(Step::Before(30)));
        assert_eq!(at("2026-11-14", &default), Some(Step::Before(7)));
        assert_eq!(at("2026-11-08", &default), Some(Step::Before(1)));
        assert_eq!(at("2026-11-07", &default), Some(Step::Before(1)));
        assert_eq!(at("2026-11-06", &default), Some(Step::Expired));
        let early = Remind::parse("90,30,7,1").unwrap();
        assert_eq!(at("2027-01-20", &early), Some(Step::Before(90)));
        let late = Remind::parse("7,1").unwrap();
        assert_eq!(at("2026-11-27", &late), None);
        assert_eq!(at("2026-11-06", &late), Some(Step::Expired));
        assert_eq!(at("2026-11-06", &Remind::Never), None);
        assert_eq!(at("2026-11-08", &Remind::Never), None);
    }

    #[test]
    fn each_step_reminds_once_for_a_date_and_a_new_date_starts_over() {
        let shown = |date: &str, level: &str| Shown {
            date: date.into(),
            level: level.into(),
        };
        assert!(should_remind(None, "2026-12-01", Step::Before(30)));
        let s = shown("2026-12-01", "30");
        assert!(!should_remind(Some(&s), "2026-12-01", Step::Before(30)));
        assert!(should_remind(Some(&s), "2026-12-01", Step::Before(7)));
        assert!(should_remind(Some(&s), "2026-12-01", Step::Expired));
        assert!(!should_remind(
            Some(&shown("2026-12-01", "expired")),
            "2026-12-01",
            Step::Before(1)
        ));
        // A new date, or something this app doesn't know, starts over.
        assert!(should_remind(Some(&s), "2027-12-01", Step::Before(30)));
        assert!(should_remind(
            Some(&shown("2026-12-01", "soon")),
            "2026-12-01",
            Step::Before(30)
        ));
        // New days go on from where the item stands: 14 after 30 reminds, 90 doesn't.
        assert!(should_remind(Some(&s), "2026-12-01", Step::Before(14)));
        assert!(!should_remind(Some(&s), "2026-12-01", Step::Before(90)));
    }

    #[test]
    fn a_reminder_says_how_many_never_which() {
        let c = |soon, expired| Counts { soon, expired };
        assert_eq!(words(c(0, 0)), None);
        assert_eq!(words(c(1, 0)).as_deref(), Some("1 item expires soon"));
        assert_eq!(words(c(2, 0)).as_deref(), Some("2 items expire soon"));
        assert_eq!(words(c(0, 1)).as_deref(), Some("1 item has expired"));
        assert_eq!(words(c(0, 2)).as_deref(), Some("2 items have expired"));
        assert_eq!(
            words(c(2, 1)).as_deref(),
            Some("3 items expire soon or have expired")
        );
    }
}
