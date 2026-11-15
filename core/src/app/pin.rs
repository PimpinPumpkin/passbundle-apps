//! Unlocking with a PIN (DESIGN.md in the meta repo, "Unlock with a PIN"):
//! when an account may have one, setting it, unlocking with it, and when
//! it's removed. The key, the wrap, and the file are in `crate::pin`.
//!
//! A PIN opens the vault and nothing else. Nothing here sets what an item's
//! master password re-prompt or a sign-in's approval check (`reprompted`,
//! `confirmed`), and a vault a PIN opened doesn't hand its user key to the
//! device's biometric store (`opened_with_pin`).

use super::App;
use crate::error::{Error, Result};
use crate::lock_policy::LockPolicy;
use crate::pin::{self, For, Kept, Opened, Record, MAX_WRONG_TRIES};
use crate::store::Store;
use crate::vault::{Sync, Vault};

/// Whether this account can unlock with a PIN on this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinStatus {
    /// It has one.
    On,
    /// It has none, and may set one.
    Off,
    /// This device has nowhere to seal one (Linux, a Mac without a Secure
    /// Enclave, the iPhone's AutoFill extension): the setting isn't shown.
    Unsupported,
    /// An organization's "Remove Unlock with PIN" policy forbids it.
    Policy,
}

impl PinStatus {
    /// As the app reads it.
    pub fn name(self) -> &'static str {
        match self {
            PinStatus::On => "on",
            PinStatus::Off => "off",
            PinStatus::Unsupported => "unsupported",
            PinStatus::Policy => "policy",
        }
    }
}

const POLICY: Error = Error::Refused("your organization doesn't allow unlocking with a PIN");
const CANT_SEAL: Error = Error::Refused("this device can't keep a PIN safely");

impl App {
    /// Whether a PIN can be kept here at all: only where the record can be
    /// sealed to the device, and never from beside the app.
    fn keeps_pins(&self) -> bool {
        !self.beside && self.store.dir().is_some() && self.secrets.seals_to_device()
    }

    /// The organizations' policy as the vault kept here has it: the open
    /// vault's, or, while locked, read from the kept sync.
    fn pin_policy(&self) -> LockPolicy {
        if self.vault.is_some() {
            return self.policy;
        }
        self.store
            .sync()
            .as_deref()
            .map(LockPolicy::from_sync)
            .unwrap_or_default()
    }

    /// Whether this account has a PIN, may set one, or why not. While locked
    /// with no PIN the answer is `Off` without reading the kept vault.
    pub fn pin_status(&self) -> PinStatus {
        if !self.keeps_pins() {
            return PinStatus::Unsupported;
        }
        let kept = Kept::read(&self.store).is_some();
        if self.vault.is_none() && !kept {
            return PinStatus::Off;
        }
        if self.pin_policy().remove_unlock_with_pin {
            return PinStatus::Policy;
        }
        if kept {
            PinStatus::On
        } else {
            PinStatus::Off
        }
    }

    /// Whether a PIN opened the vault that's open now: until it locks, or
    /// the master password unlocks it. Biometric unlock can't be turned on
    /// then (`enroll_biometric`, `ffi::user_key_for_biometric`).
    pub fn opened_with_pin(&self) -> bool {
        self.opened_with_pin
    }

    /// Sets the account's PIN, while unlocked. The master password is asked
    /// for again and checked first, the way unlocking checks it, whatever
    /// opened the vault. Then the user key is wrapped under a key made from
    /// the PIN, that record sealed to the device, and kept with a count of
    /// zero wrong tries. A PIN set before is replaced.
    pub fn set_pin(&self, master_password: &str, pin: &str) -> Result<()> {
        let settings = self.settings.as_ref().ok_or(Error::SignedOut)?;
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        if !self.keeps_pins() {
            return Err(CANT_SEAL);
        }
        if self.policy.remove_unlock_with_pin {
            return Err(POLICY);
        }
        pin::check(pin)?;
        let sync = Sync::from_json(&self.store.sync().ok_or(Error::SignedOut)?)?;
        if !vault.is_master_password(&sync, &settings.prelogin(), master_password)? {
            return Err(Error::WrongPassword);
        }
        // What the master password opens, as the sync has it: when it
        // changes, the PIN goes (`after_sync`).
        let protected = sync.profile.key.as_deref().ok_or(Error::WrongPassword)?;
        let whom = For {
            server: &settings.server,
            user_id: &sync.profile.id,
            account: &self.id,
        };
        let record = Record::make(pin, vault.user_key(), &whom)?;
        let sealed = self
            .secrets
            .seal_to_device(&self.id, record.to_json().as_bytes())
            .ok_or(CANT_SEAL)?;
        Kept::new(protected, &sealed).write(&self.store)
    }

    /// Unlocks the vault kept on this device with the account's PIN, without
    /// the network, in the order the design gives:
    ///
    /// 1. No PIN kept, or a file this version can't read: `NoPin`.
    /// 2. The organization's policy and what the PIN was made for, against
    ///    the kept vault: a PIN that shouldn't be there any more is removed.
    /// 3. The count of wrong tries, before anything is derived: at 5 the PIN
    ///    is removed (`PinTurnedOff`).
    /// 4. The device opens the seal; one that no longer opens removes the
    ///    PIN, and isn't a wrong try.
    /// 5. The count is written one higher before Argon2id runs; if it can't
    ///    be written, the try is refused.
    /// 6. A right PIN opens the vault and writes the count back to zero. A
    ///    wrong one says how many tries are left (`WrongPin`), and the fifth
    ///    removes the PIN (`PinTurnedOff`).
    pub fn unlock_with_pin(&mut self, pin: &str) -> Result<()> {
        let settings = self.settings.as_ref().ok_or(Error::SignedOut)?;
        let text = self.store.sync().ok_or(Error::SignedOut)?;
        // The AutoFill extension, and a device that can't seal, have no PIN;
        // neither touches a file the app may have.
        if !self.keeps_pins() {
            return Err(Error::NoPin);
        }
        // 1.
        let Some(mut kept) = Kept::read(&self.store) else {
            pin::forget(&self.store);
            return Err(Error::NoPin);
        };
        // 2.
        if LockPolicy::from_sync(&text).remove_unlock_with_pin {
            pin::forget(&self.store);
            return Err(POLICY);
        }
        let sync = Sync::from_json(&text)?;
        if !kept.made_for(sync.profile.key.as_deref()) {
            pin::forget(&self.store);
            return Err(Error::NoPin);
        }
        // 3.
        if kept.wrong_tries >= MAX_WRONG_TRIES {
            pin::forget(&self.store);
            return Err(Error::PinTurnedOff);
        }
        // Something that can't be a PIN isn't a try: nothing is derived.
        pin::check(pin)?;
        // 4.
        let record = kept
            .sealed()
            .and_then(|sealed| self.secrets.open_from_device(&self.id, &sealed))
            .and_then(|record| Record::from_json(&record).ok())
            .filter(|record| record.usable().is_ok());
        let Some(record) = record else {
            pin::forget(&self.store);
            return Err(Error::NoPin);
        };
        // 5.
        kept.wrong_tries += 1;
        kept.write(&self.store)?;
        // 6.
        let whom = For {
            server: &settings.server,
            user_id: &sync.profile.id,
            account: &self.id,
        };
        let key = match record.open(pin, &whom) {
            Ok(Opened::Key(key)) => key,
            Ok(Opened::WrongPin) if kept.wrong_tries >= MAX_WRONG_TRIES => {
                pin::forget(&self.store);
                return Err(Error::PinTurnedOff);
            }
            Ok(Opened::WrongPin) => {
                return Err(Error::WrongPin {
                    left: MAX_WRONG_TRIES - kept.wrong_tries,
                })
            }
            // A record no PIN opens.
            Err(_) => {
                pin::forget(&self.store);
                return Err(Error::NoPin);
            }
        };
        match Vault::open(&sync, key) {
            Ok(vault) => {
                kept.wrong_tries = 0;
                // If this can't be written the count stays one higher, which
                // only ever costs the person a try.
                let _ = kept.write(&self.store);
                self.opened(vault);
                self.opened_with_pin = true;
                Ok(())
            }
            // The right PIN for a key that no longer opens the vault kept
            // here (the account's key changed): out of date, and removed.
            Err(Error::Refused(_)) | Err(Error::Damaged(_)) => {
                pin::forget(&self.store);
                Err(Error::NoPin)
            }
            Err(e) => Err(e),
        }
    }

    /// Turns the account's PIN off: the file goes. Another account's is
    /// never touched, and neither is the app's from beside it.
    pub fn remove_pin(&self) {
        if !self.beside {
            pin::forget(&self.store);
        }
    }
}

/// After a sync is kept: a PIN goes when an organization's policy now
/// forbids one, or when the account's protected user key isn't the one the
/// PIN was made for (the master password, the KDF settings, or the
/// account's key changed).
pub(super) fn after_sync(store: &Store, policy: &LockPolicy, protected_user_key: Option<&str>) {
    let Some(kept) = Kept::read(store) else {
        return;
    };
    if policy.remove_unlock_with_pin || !kept.made_for(protected_user_key) {
        pin::forget(store);
    }
}
