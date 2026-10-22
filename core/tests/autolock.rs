//! Locking by itself, on the vectors' sample vault kept on this device: the
//! app unlocks it, then hears made-up readings from the OS (docs/core.md,
//! "Locking by itself"). The decisions themselves are tested in src/autolock.rs.

use std::time::{Duration, Instant, SystemTime};

use passbundle_core::app::{App, State};
use passbundle_core::autolock::{Preferences, Readings, Reason};
use passbundle_core::secrets::MemorySecretStore;
use passbundle_core::store::{Settings, Store};

const MIN: Duration = Duration::from_secs(60);

struct Sample {
    dir: std::path::PathBuf,
    password: String,
}

impl Sample {
    fn new() -> Sample {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
        let pre = &v["account"]["pbkdf2"]["prelogin"];
        let dir = std::env::temp_dir().join(format!(
            "pb-core-autolock-{}",
            uuid::Uuid::new_v4().simple()
        ));
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
            password: v["account"]["password"].as_str().unwrap().into(),
        }
    }

    fn open(&self) -> App {
        App::open(
            Store::new(&self.dir),
            Box::new(MemorySecretStore::default()),
        )
    }

    fn unlocked(&self) -> App {
        let mut app = self.open();
        app.unlock(&self.password).unwrap();
        app
    }
}

impl Drop for Sample {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn is_unlocked(app: &App) -> bool {
    matches!(app.state(), State::Unlocked { .. })
}

/// The OS saying nobody has touched the computer for `idle`.
fn idle(idle: Duration) -> Readings {
    Readings {
        idle: Some(idle),
        screen_locked: Some(false),
        asleep: Some(Duration::ZERO),
    }
}

#[test]
fn locks_after_the_default_time_and_unlocking_starts_the_count_again() {
    let sample = Sample::new();
    let mut app = sample.unlocked();
    // 15 minutes on a computer, 5 on a phone.
    let wait = MIN * app.preferences().lock_after_minutes.unwrap();
    let (now, wall) = (Instant::now(), SystemTime::now());

    assert_eq!(
        app.tick_at(&idle(wait - MIN), now + wait - MIN, wall + wait - MIN),
        None
    );
    assert!(is_unlocked(&app));
    assert_eq!(
        app.tick_at(&idle(wait), now + wait, wall + wait),
        Some(Reason::Idle)
    );
    assert!(matches!(app.state(), State::Locked { .. }));
    assert!(app.vault().is_none(), "locking drops the vault");
    // Locked, a tick has nothing to do.
    assert_eq!(
        app.tick_at(&idle(60 * MIN), now + 60 * MIN, wall + 60 * MIN),
        None
    );

    // Unlocking again counts from then, not from the first unlock.
    app.unlock(&sample.password).unwrap();
    let (now, wall) = (Instant::now(), SystemTime::now());
    assert_eq!(app.tick_at(&idle(MIN), now + MIN, wall + MIN), None);
    assert!(is_unlocked(&app));
}

#[test]
fn using_the_app_keeps_it_open_where_the_os_doesnt_say() {
    let sample = Sample::new();
    let mut app = sample.unlocked();
    app.set_preferences(Preferences {
        lock_after_minutes: Some(1),
        lock_on_sleep: true,
    })
    .unwrap();
    let silent = Readings::default();
    let (now, wall) = (Instant::now(), SystemTime::now());
    assert_eq!(
        app.tick_at(&silent, now + 2 * MIN, wall + 2 * MIN),
        Some(Reason::Idle)
    );

    app.unlock(&sample.password).unwrap();
    // Used just now: a tick a few seconds on doesn't lock.
    app.used();
    let (now, wall) = (Instant::now(), SystemTime::now());
    assert_eq!(
        app.tick_at(
            &silent,
            now + Duration::from_secs(5),
            wall + Duration::from_secs(5)
        ),
        None
    );
}

#[test]
fn the_screen_locking_and_the_computer_sleeping_lock_it_if_chosen() {
    let sample = Sample::new();
    let mut app = sample.unlocked();
    let (now, wall) = (Instant::now(), SystemTime::now());
    let screen = Readings {
        idle: Some(Duration::ZERO),
        screen_locked: Some(true),
        asleep: None,
    };
    assert_eq!(app.tick_at(&screen, now, wall), Some(Reason::ScreenLocked));

    app.unlock(&sample.password).unwrap();
    assert!(app.sleeping(), "the OS's notice before sleeping locks it");
    assert!(matches!(app.state(), State::Locked { .. }));
    assert!(!app.sleeping(), "already locked");

    // Chosen off: neither locks it.
    app.set_preferences(Preferences {
        lock_after_minutes: Some(15),
        lock_on_sleep: false,
    })
    .unwrap();
    app.unlock(&sample.password).unwrap();
    assert!(!app.sleeping());
    let (now, wall) = (Instant::now(), SystemTime::now());
    assert_eq!(app.tick_at(&screen, now, wall), None);
    assert!(is_unlocked(&app));
}

#[test]
fn the_choices_are_kept_on_this_device_and_checked() {
    let sample = Sample::new();
    let mut app = sample.open();
    assert_eq!(app.preferences(), Preferences::default());
    let chosen = Preferences {
        lock_after_minutes: Some(60),
        lock_on_sleep: false,
    };
    app.set_preferences(chosen).unwrap();
    assert!(app
        .set_preferences(Preferences {
            lock_after_minutes: Some(passbundle_core::autolock::MAX_MINUTES + 1),
            lock_on_sleep: true,
        })
        .is_err());
    // Signing out keeps them, for whoever signs in on this device next.
    app.sign_out();
    assert_eq!(sample.open().preferences(), chosen);
}

/// Phones: "Immediately" locks when the app leaves the screen, and never
/// while it's used.
#[cfg(phone)]
#[test]
fn on_a_phone_immediately_locks_when_the_app_leaves_the_screen() {
    let sample = Sample::new();
    let mut app = sample.unlocked();
    assert_eq!(app.preferences().lock_after_minutes, Some(5));
    assert!(!app.left(), "five minutes: leaving doesn't lock yet");
    app.set_preferences(Preferences {
        lock_after_minutes: Some(0),
        lock_on_sleep: true,
    })
    .unwrap();
    let (now, wall) = (Instant::now(), SystemTime::now());
    assert_eq!(
        app.tick_at(&Readings::default(), now + 60 * MIN, wall + 60 * MIN),
        None
    );
    assert!(app.left());
    assert!(matches!(app.state(), State::Locked { .. }));
}
