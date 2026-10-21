//! Locking by itself (docs/core.md, "Locking by itself"): after the time the
//! person chooses without using the computer, and when the screen locks or
//! the computer sleeps, if they choose. The decision is made here from what
//! the OS says (`readings`), so it's tested with made-up readings.

use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The person's choices, kept on this device for whoever signs in on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    /// Minutes without using the computer before locking; None locks only when the app closes.
    pub lock_after_minutes: Option<u32>,
    /// Also lock when the screen locks or the computer sleeps.
    pub lock_on_sleep: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            lock_after_minutes: Some(15),
            lock_on_sleep: true,
        }
    }
}

/// The longest wait: a day.
pub const MAX_MINUTES: u32 = 24 * 60;

impl Preferences {
    pub fn check(&self) -> Result<()> {
        match self.lock_after_minutes {
            Some(m) if m == 0 || m > MAX_MINUTES => Err(Error::Refused(
                "the time before locking is from 1 minute to a day",
            )),
            _ => Ok(()),
        }
    }
}

/// What the OS says right now; None where it doesn't say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Readings {
    /// Time since the last keyboard, mouse, or touch input anywhere on the computer.
    pub idle: Option<Duration>,
    /// Whether this person's screen is locked.
    pub screen_locked: Option<bool>,
    /// Time the computer has spent asleep since it started.
    pub asleep: Option<Duration>,
}

/// Why the vault locked by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Idle,
    ScreenLocked,
    Slept,
}

impl Reason {
    pub fn name(self) -> &'static str {
        match self {
            Reason::Idle => "idle",
            Reason::ScreenLocked => "screenLocked",
            Reason::Slept => "slept",
        }
    }
}

/// Anything longer asleep between two checks counts as the computer having
/// slept (the clocks agree to milliseconds otherwise).
const SLEPT: Duration = Duration::from_secs(2);

/// One unlocked stretch: when the app was last used, and what the last check saw.
#[derive(Debug)]
pub struct AutoLock {
    used: Instant,
    used_wall: SystemTime,
    /// The computer's time asleep at the last check, to see it sleep between checks.
    asleep: Option<Duration>,
    /// The OS's idle time at the last check: idle before a sleep, since waking resets it.
    idle: Option<Duration>,
}

impl AutoLock {
    /// Starts counting from now (unlocking).
    pub fn new(now: Instant, wall: SystemTime) -> AutoLock {
        AutoLock {
            used: now,
            used_wall: wall,
            asleep: None,
            idle: None,
        }
    }

    /// The app was used just now.
    pub fn used(&mut self, now: Instant, wall: SystemTime) {
        self.used = now;
        self.used_wall = wall;
    }

    /// Time since the app was last used: the longer of the two clocks, so
    /// neither the wall clock being set back nor a monotonic clock that stops
    /// while asleep makes it shorter.
    fn app_idle(&self, now: Instant, wall: SystemTime) -> Duration {
        let wall_idle = wall.duration_since(self.used_wall).unwrap_or_default();
        now.saturating_duration_since(self.used).max(wall_idle)
    }

    /// Whether to lock now, and why.
    pub fn check(
        &mut self,
        prefs: &Preferences,
        r: &Readings,
        now: Instant,
        wall: SystemTime,
    ) -> Option<Reason> {
        let limit = prefs
            .lock_after_minutes
            .map(|m| Duration::from_secs(u64::from(m) * 60));
        let app_idle = self.app_idle(now, wall);
        // The computer's own idle time where the OS says, since using other
        // apps is using the computer; the app's own use where it doesn't.
        let idle = r.idle.map_or(app_idle, |os| os.min(app_idle));

        let slept = match (self.asleep, r.asleep) {
            (Some(before), Some(after)) if after > before + SLEPT => Some(after - before),
            _ => None,
        };
        let idle_before = self.idle;
        if r.asleep.is_some() {
            self.asleep = r.asleep;
        }
        self.idle = Some(idle);

        if let Some(slept) = slept {
            if prefs.lock_on_sleep {
                return Some(Reason::Slept);
            }
            // Waking resets the OS's idle time; the time asleep still counts.
            if limit.is_some_and(|limit| idle_before.unwrap_or_default() + slept >= limit) {
                return Some(Reason::Idle);
            }
        }
        if prefs.lock_on_sleep && r.screen_locked == Some(true) {
            return Some(Reason::ScreenLocked);
        }
        if limit.is_some_and(|limit| idle >= limit) {
            return Some(Reason::Idle);
        }
        None
    }
}

/// What this computer's OS says right now.
pub fn readings() -> Readings {
    Readings {
        idle: os::idle(),
        screen_locked: os::screen_locked(),
        asleep: os::asleep(),
    }
}

/// The time between a clock that counts sleep and one that doesn't.
#[cfg(unix)]
#[allow(dead_code)] // Unused where the OS has no such pair.
fn asleep_between(with_sleep: libc::clockid_t, without: libc::clockid_t) -> Option<Duration> {
    let read = |clock| {
        let mut t = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: clock_gettime writes one timespec, which `t` is.
        (unsafe { libc::clock_gettime(clock, &mut t) } == 0)
            .then(|| Duration::new(t.tv_sec as u64, t.tv_nsec as u32))
    };
    let awake = read(without)?;
    Some(read(with_sleep)?.saturating_sub(awake))
}

#[cfg(windows)]
mod os {
    use std::time::Duration;

    use windows_sys::Win32::System::RemoteDesktop::{
        WTSFreeMemory, WTSQuerySessionInformationW, WTSSessionInfoEx, WTSINFOEXW,
        WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTS_SESSIONSTATE_LOCK,
        WTS_SESSIONSTATE_UNLOCK,
    };
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::System::WindowsProgramming::{
        QueryInterruptTime, QueryUnbiasedInterruptTime,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    pub fn idle() -> Option<Duration> {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: `info` is a LASTINPUTINFO with its size set, as the call requires.
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return None;
        }
        // Both in milliseconds since the computer started, in 32 bits that wrap every 49.7 days.
        // SAFETY: no arguments.
        let now = unsafe { GetTickCount() };
        Some(Duration::from_millis(u64::from(
            now.wrapping_sub(info.dwTime),
        )))
    }

    pub fn screen_locked() -> Option<bool> {
        let mut buffer: windows_sys::core::PWSTR = std::ptr::null_mut();
        let mut bytes = 0u32;
        // SAFETY: on success the call hands over a buffer it allocated, freed below.
        let ok = unsafe {
            WTSQuerySessionInformationW(
                WTS_CURRENT_SERVER_HANDLE,
                WTS_CURRENT_SESSION,
                WTSSessionInfoEx,
                &mut buffer,
                &mut bytes,
            )
        };
        if ok == 0 || buffer.is_null() {
            return None;
        }
        let mut locked = None;
        if bytes as usize >= std::mem::size_of::<WTSINFOEXW>() {
            // SAFETY: the buffer holds a WTSINFOEXW; its size was checked.
            let info = unsafe { &*(buffer as *const WTSINFOEXW) };
            if info.Level == 1 {
                // SAFETY: level 1 means the union holds WTSInfoExLevel1.
                let flags = unsafe { info.Data.WTSInfoExLevel1.SessionFlags } as u32;
                locked = match flags {
                    WTS_SESSIONSTATE_LOCK => Some(true),
                    WTS_SESSIONSTATE_UNLOCK => Some(false),
                    _ => None,
                };
            }
        }
        // SAFETY: the buffer came from WTSQuerySessionInformationW.
        unsafe { WTSFreeMemory(buffer.cast()) };
        locked
    }

    pub fn asleep() -> Option<Duration> {
        let (mut total, mut awake) = (0u64, 0u64);
        // SAFETY: each writes one u64 (100-nanosecond units since the computer
        // started: with time asleep, and without).
        unsafe {
            QueryInterruptTime(&mut total);
            if QueryUnbiasedInterruptTime(&mut awake) == 0 {
                return None;
            }
        }
        Some(Duration::from_nanos(
            total.saturating_sub(awake).saturating_mul(100),
        ))
    }
}

#[cfg(target_os = "macos")]
mod os {
    use std::ffi::c_void;
    use std::time::Duration;

    type CFTypeRef = *const c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
        fn CGSessionCopyCurrentDictionary() -> CFTypeRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithBytes(
            alloc: CFTypeRef,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> CFTypeRef;
        fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(boolean: CFTypeRef) -> u8;
        fn CFRelease(cf: CFTypeRef);
    }

    /// kCGEventSourceStateCombinedSessionState: input from anywhere in this login session.
    const COMBINED_SESSION: i32 = 0;
    /// kCGAnyInputEventType.
    const ANY_INPUT: u32 = !0;
    /// kCFStringEncodingUTF8.
    const UTF8: u32 = 0x0800_0100;

    pub fn idle() -> Option<Duration> {
        // SAFETY: plain values in and out.
        let seconds =
            unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION, ANY_INPUT) };
        (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
    }

    pub fn screen_locked() -> Option<bool> {
        const KEY: &[u8] = b"CGSSessionScreenIsLocked";
        // SAFETY: each object made or copied here is released here, and the
        // value looked up is only read while the dictionary holding it lives.
        unsafe {
            let session = CGSessionCopyCurrentDictionary();
            if session.is_null() {
                return None;
            }
            let key = CFStringCreateWithBytes(
                std::ptr::null(),
                KEY.as_ptr(),
                KEY.len() as isize,
                UTF8,
                0,
            );
            let locked = if key.is_null() {
                None
            } else {
                // The key is there only while the screen is locked.
                let value = CFDictionaryGetValue(session, key);
                CFRelease(key);
                Some(
                    !value.is_null()
                        && CFGetTypeID(value) == CFBooleanGetTypeID()
                        && CFBooleanGetValue(value) != 0,
                )
            };
            CFRelease(session);
            locked
        }
    }

    pub fn asleep() -> Option<Duration> {
        // On macOS the raw monotonic clock counts sleep and the uptime clock doesn't.
        super::asleep_between(libc::CLOCK_MONOTONIC_RAW, libc::CLOCK_UPTIME_RAW)
    }
}

#[cfg(target_os = "linux")]
mod os {
    use std::cell::RefCell;
    use std::time::Duration;

    use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
    use dbus::blocking::Connection;

    const TIMEOUT: Duration = Duration::from_millis(250);

    thread_local! {
        static SESSION_BUS: RefCell<Option<Connection>> = const { RefCell::new(None) };
        static SYSTEM_BUS: RefCell<Option<Connection>> = const { RefCell::new(None) };
    }

    /// A call on a bus, connected once per thread (the app checks from one).
    fn on<T>(
        bus: &'static std::thread::LocalKey<RefCell<Option<Connection>>>,
        connect: fn() -> Result<Connection, dbus::Error>,
        call: impl FnOnce(&Connection) -> Option<T>,
    ) -> Option<T> {
        bus.with(|cell| {
            let mut cell = cell.borrow_mut();
            if cell.is_none() {
                *cell = connect().ok();
            }
            call(cell.as_ref()?)
        })
    }

    /// GNOME's count. Other desktops don't share one, and the app's own use stands in.
    pub fn idle() -> Option<Duration> {
        on(&SESSION_BUS, Connection::new_session, |bus| {
            let (ms,): (u64,) = bus
                .with_proxy(
                    "org.gnome.Mutter.IdleMonitor",
                    "/org/gnome/Mutter/IdleMonitor/Core",
                    TIMEOUT,
                )
                .method_call("org.gnome.Mutter.IdleMonitor", "GetIdletime", ())
                .ok()?;
            Some(Duration::from_millis(ms))
        })
    }

    /// systemd-logind's word for this session, which GNOME, KDE, and most lock screens keep.
    pub fn screen_locked() -> Option<bool> {
        on(&SYSTEM_BUS, Connection::new_system, |bus| {
            bus.with_proxy(
                "org.freedesktop.login1",
                "/org/freedesktop/login1/session/auto",
                TIMEOUT,
            )
            .get("org.freedesktop.login1.Session", "LockedHint")
            .ok()
        })
    }

    pub fn asleep() -> Option<Duration> {
        // CLOCK_BOOTTIME counts time suspended; CLOCK_MONOTONIC doesn't.
        super::asleep_between(libc::CLOCK_BOOTTIME, libc::CLOCK_MONOTONIC)
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod os {
    use std::time::Duration;

    pub fn idle() -> Option<Duration> {
        None
    }

    pub fn screen_locked() -> Option<bool> {
        None
    }

    pub fn asleep() -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    struct Clock {
        now: Instant,
        wall: SystemTime,
    }

    impl Clock {
        fn new() -> Clock {
            Clock {
                now: Instant::now(),
                wall: SystemTime::now(),
            }
        }

        /// Time passing awake: both clocks move.
        fn pass(&mut self, d: Duration) {
            self.now += d;
            self.wall += d;
        }

        /// Time passing asleep: the wall clock moves, and (as on macOS and
        /// Linux) the monotonic one doesn't.
        fn sleep(&mut self, d: Duration) {
            self.wall += d;
        }
    }

    fn prefs(minutes: Option<u32>, on_sleep: bool) -> Preferences {
        Preferences {
            lock_after_minutes: minutes,
            lock_on_sleep: on_sleep,
        }
    }

    fn os(idle: Option<Duration>, screen_locked: bool, asleep: Duration) -> Readings {
        Readings {
            idle,
            screen_locked: Some(screen_locked),
            asleep: Some(asleep),
        }
    }

    #[test]
    fn locks_after_the_time_without_using_the_computer() {
        let mut c = Clock::new();
        let mut lock = AutoLock::new(c.now, c.wall);
        let p = prefs(Some(5), true);
        let zero = Duration::ZERO;
        c.pass(4 * MIN);
        assert_eq!(
            lock.check(&p, &os(Some(4 * MIN), false, zero), c.now, c.wall),
            None
        );
        c.pass(MIN);
        assert_eq!(
            lock.check(&p, &os(Some(5 * MIN), false, zero), c.now, c.wall),
            Some(Reason::Idle)
        );
    }

    #[test]
    fn using_other_apps_is_using_the_computer() {
        let mut c = Clock::new();
        let mut lock = AutoLock::new(c.now, c.wall);
        let p = prefs(Some(5), true);
        c.pass(60 * MIN);
        // The app hasn't been touched for an hour, but the computer was used a minute ago.
        assert_eq!(
            lock.check(&p, &os(Some(MIN), false, Duration::ZERO), c.now, c.wall),
            None
        );
    }

    #[test]
    fn where_the_os_doesnt_say_the_apps_own_use_counts() {
        let mut c = Clock::new();
        let mut lock = AutoLock::new(c.now, c.wall);
        let p = prefs(Some(5), true);
        let r = Readings::default();
        c.pass(4 * MIN);
        lock.used(c.now, c.wall);
        c.pass(4 * MIN);
        assert_eq!(lock.check(&p, &r, c.now, c.wall), None);
        c.pass(MIN);
        assert_eq!(lock.check(&p, &r, c.now, c.wall), Some(Reason::Idle));
    }

    #[test]
    fn an_os_idle_time_that_seems_wrong_never_outlasts_the_apps_own_use() {
        let mut c = Clock::new();
        let mut lock = AutoLock::new(c.now, c.wall);
        let p = prefs(Some(5), true);
        c.pass(MIN);
        lock.used(c.now, c.wall);
        // The OS says nobody's touched anything for an hour; the app was used just now.
        assert_eq!(
            lock.check(
                &p,
                &os(Some(60 * MIN), false, Duration::ZERO),
                c.now,
                c.wall
            ),
            None
        );
    }

    #[test]
    fn setting_the_clock_back_doesnt_stretch_the_wait() {
        let mut c = Clock::new();
        let mut lock = AutoLock::new(c.now, c.wall);
        let p = prefs(Some(5), true);
        c.now += 5 * MIN;
        c.wall -= 60 * MIN;
        assert_eq!(
            lock.check(&p, &Readings::default(), c.now, c.wall),
            Some(Reason::Idle)
        );
    }

    #[test]
    fn locks_when_the_screen_locks_if_chosen() {
        let mut c = Clock::new();
        let zero = Duration::ZERO;
        let mut lock = AutoLock::new(c.now, c.wall);
        c.pass(Duration::from_secs(2));
        assert_eq!(
            lock.check(
                &prefs(Some(15), true),
                &os(Some(zero), true, zero),
                c.now,
                c.wall
            ),
            Some(Reason::ScreenLocked)
        );
        let mut lock = AutoLock::new(c.now, c.wall);
        assert_eq!(
            lock.check(
                &prefs(Some(15), false),
                &os(Some(zero), true, zero),
                c.now,
                c.wall
            ),
            None
        );
    }

    #[test]
    fn locks_after_the_computer_sleeps_if_chosen() {
        let mut c = Clock::new();
        let mut lock = AutoLock::new(c.now, c.wall);
        let p = prefs(None, true);
        let base = 3 * MIN;
        assert_eq!(
            lock.check(&p, &os(Some(Duration::ZERO), false, base), c.now, c.wall),
            None
        );
        c.sleep(30 * MIN);
        // Waking: someone touched a key, so the OS's idle time starts again.
        assert_eq!(
            lock.check(
                &p,
                &os(Some(Duration::ZERO), false, base + 30 * MIN),
                c.now,
                c.wall
            ),
            Some(Reason::Slept)
        );
    }

    #[test]
    fn a_short_doze_isnt_sleep() {
        let mut c = Clock::new();
        let mut lock = AutoLock::new(c.now, c.wall);
        let p = prefs(None, true);
        let base = 3 * MIN;
        assert_eq!(lock.check(&p, &os(None, false, base), c.now, c.wall), None);
        c.pass(Duration::from_secs(2));
        let jitter = Duration::from_millis(40);
        assert_eq!(
            lock.check(&p, &os(None, false, base + jitter), c.now, c.wall),
            None
        );
    }

    #[test]
    fn time_asleep_counts_toward_the_wait_when_sleep_alone_doesnt_lock() {
        let mut c = Clock::new();
        let p = prefs(Some(15), false);
        let base = 3 * MIN;

        // Idle 10 minutes, asleep 10 more: past 15.
        let mut lock = AutoLock::new(c.now, c.wall);
        c.pass(10 * MIN);
        assert_eq!(
            lock.check(&p, &os(Some(10 * MIN), false, base), c.now, c.wall),
            None
        );
        c.sleep(10 * MIN);
        assert_eq!(
            lock.check(
                &p,
                &os(Some(Duration::ZERO), false, base + 10 * MIN),
                c.now,
                c.wall
            ),
            Some(Reason::Idle)
        );

        // Idle 1 minute, asleep 10: not yet.
        let mut lock = AutoLock::new(c.now, c.wall);
        c.pass(MIN);
        assert_eq!(
            lock.check(&p, &os(Some(MIN), false, base + 10 * MIN), c.now, c.wall),
            None
        );
        c.sleep(10 * MIN);
        assert_eq!(
            lock.check(
                &p,
                &os(Some(Duration::ZERO), false, base + 20 * MIN),
                c.now,
                c.wall
            ),
            None
        );
    }

    #[test]
    fn never_by_time_when_the_choice_is_when_the_app_closes() {
        let mut c = Clock::new();
        let mut lock = AutoLock::new(c.now, c.wall);
        c.pass(24 * 60 * MIN);
        assert_eq!(
            lock.check(
                &prefs(None, false),
                &os(Some(24 * 60 * MIN), true, Duration::ZERO),
                c.now,
                c.wall
            ),
            None
        );
    }

    #[test]
    fn the_choices_are_from_a_minute_to_a_day() {
        assert!(prefs(Some(1), true).check().is_ok());
        assert!(prefs(Some(MAX_MINUTES), true).check().is_ok());
        assert!(prefs(None, true).check().is_ok());
        assert!(prefs(Some(0), true).check().is_err());
        assert!(prefs(Some(MAX_MINUTES + 1), true).check().is_err());
        assert_eq!(
            Preferences::default(),
            prefs(Some(15), true),
            "15 minutes, and when the screen locks or the computer sleeps"
        );
    }

    /// This computer's OS answers without failing, and time asleep only grows.
    /// What it says is printed, to see what CI's machines report.
    #[test]
    fn the_os_answers() {
        let first = readings();
        let second = readings();
        eprintln!("readings: {first:?}");
        if let (Some(a), Some(b)) = (first.asleep, second.asleep) {
            assert!(b + Duration::from_millis(50) >= a, "{a:?} then {b:?}");
        }
        if cfg!(any(windows, target_os = "macos")) {
            assert!(first.asleep.is_some());
        }
    }
}
