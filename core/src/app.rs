//! The app's state, as the screens see it: signed out, locked, or unlocked,
//! and everything they ask for. One per process; the bridge (ffi.rs) holds it.
//! See docs/core.md: signing in, two-step login, unlocking from the kept sync,
//! and syncing with the kept refresh token.

use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use zeroize::Zeroizing;

use crate::agent::{self, Agent};
use crate::autolock::{AutoLock, Preferences, Readings, Reason};
use crate::clipboard;
use crate::crypto::SymmetricKey;
use crate::error::{Error, Result};
use crate::generator;
use crate::generator_policy::GeneratorPolicy;
use crate::kdf::normalize_email;
use crate::lock_policy::LockPolicy;
use crate::secrets::SecretStore;
use crate::server::{Device, PendingSignIn, Server, Session, SignIn, TwoFactor};
use crate::store::{Settings, Store};
use crate::vault::{ItemDraft, Sync, Vault};

#[derive(Debug, PartialEq, Eq)]
pub enum State {
    /// Nothing kept: sign in. The server and email of the last account, if any.
    SignedOut {
        server: Option<String>,
        email: Option<String>,
    },
    /// A vault kept on this device: unlock it with the master password.
    Locked {
        server: String,
        email: String,
    },
    Unlocked {
        server: String,
        email: String,
    },
}

/// Where signing in got to.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Done,
    /// The second step, with the methods the server offers (0 authenticator, 1 email).
    TwoFactor(Vec<u8>),
}

pub struct App {
    store: Store,
    secrets: Box<dyn SecretStore>,
    settings: Option<Settings>,
    server: Option<Server>,
    session: Option<Session>,
    pending: Option<PendingSignIn>,
    vault: Option<Vault>,
    /// The agent's address and program, when not the ones installed next to this app (tests).
    agent: Option<(PathBuf, PathBuf)>,
    preferences: Preferences,
    /// The organizations' "Vault lock" policy, from the kept sync.
    policy: LockPolicy,
    generator_policy: GeneratorPolicy,
    autolock: AutoLock,
}

impl App {
    pub fn open(store: Store, secrets: Box<dyn SecretStore>) -> App {
        let settings = store.settings();
        let server = settings
            .as_ref()
            .and_then(|s| Server::new(&s.server, Device::this_one(&s.device_identifier)).ok());
        let preferences = store.preferences();
        App {
            store,
            secrets,
            settings,
            server,
            session: None,
            pending: None,
            vault: None,
            agent: None,
            preferences,
            policy: LockPolicy::default(),
            generator_policy: GeneratorPolicy::default(),
            autolock: AutoLock::new(Instant::now(), SystemTime::now()),
        }
    }

    /// Talks to the agent at `address`, which must be the program at `program` (tests).
    pub fn use_agent_at(&mut self, address: PathBuf, program: PathBuf) {
        self.agent = Some((address, program));
    }

    pub fn state(&self) -> State {
        match (&self.settings, &self.vault, self.store.sync().is_some()) {
            (Some(s), Some(_), _) => State::Unlocked {
                server: s.server.clone(),
                email: s.email.clone(),
            },
            (Some(s), None, true) => State::Locked {
                server: s.server.clone(),
                email: s.email.clone(),
            },
            (s, _, _) => State::SignedOut {
                server: s.as_ref().map(|s| s.server.clone()),
                email: s.as_ref().map(|s| s.email.clone()),
            },
        }
    }

    pub fn vault(&self) -> Option<&Vault> {
        self.vault.as_ref()
    }

    /// Signs in to a server: the vault comes down, is kept, and opens.
    pub fn sign_in(&mut self, server_url: &str, email: &str, password: &str) -> Result<Step> {
        self.lock();
        // One identifier per install, whichever account signs in.
        let identifier = self
            .settings
            .as_ref()
            .map(|s| s.device_identifier.clone())
            .unwrap_or_else(Device::new_identifier);
        let server = Server::new(server_url, Device::this_one(&identifier))?;
        let prelogin = server.prelogin(email)?;
        let signed_in = server.sign_in_with(&prelogin, email, password)?;
        self.settings = Some(Settings {
            server: server_url.trim().trim_end_matches('/').to_owned(),
            email: normalize_email(email),
            kdf: prelogin.kdf,
            kdf_iterations: prelogin.kdf_iterations,
            kdf_memory: prelogin.kdf_memory,
            kdf_parallelism: prelogin.kdf_parallelism,
            device_identifier: identifier,
        });
        self.server = Some(server);
        match signed_in {
            SignIn::Done(session) => self.signed_in(session),
            SignIn::NeedsTwoFactor(pending) => {
                let providers = pending.providers.clone();
                self.pending = Some(pending);
                Ok(Step::TwoFactor(providers))
            }
        }
    }

    /// Two-step login's second step. A wrong code can be tried again.
    pub fn finish_sign_in(&mut self, provider: u8, code: &str, remember: bool) -> Result<Step> {
        let (Some(server), Some(pending)) = (&self.server, &self.pending) else {
            return Err(Error::SignedOut);
        };
        let session = server.finish_sign_in(
            pending,
            &TwoFactor {
                provider,
                code: code.to_owned(),
                remember,
            },
        )?;
        self.pending = None;
        self.signed_in(session)
    }

    /// Asks the server to email the code, for a sign-in waiting on one.
    pub fn send_email_code(&self) -> Result<()> {
        let (Some(server), Some(pending)) = (&self.server, &self.pending) else {
            return Err(Error::SignedOut);
        };
        server.send_email_code(pending)
    }

    fn signed_in(&mut self, session: Session) -> Result<Step> {
        let settings = self.settings.as_ref().expect("set before signing in");
        self.store.save_settings(settings)?;
        self.secrets
            .keep(&settings.server, &settings.email, &session.refresh_token);
        self.session = Some(session);
        self.sync()?;
        self.autolock = AutoLock::new(Instant::now(), SystemTime::now());
        Ok(Step::Done)
    }

    /// Unlocks the vault kept on this device, without the network. The kept
    /// refresh token, if any, lets `sync` bring it up to date afterwards.
    pub fn unlock(&mut self, password: &str) -> Result<()> {
        let settings = self.settings.as_ref().ok_or(Error::SignedOut)?;
        let text = self.store.sync().ok_or(Error::SignedOut)?;
        let vault = Vault::unlock(&Sync::from_json(&text)?, &settings.prelogin(), password)?;
        self.opened(vault);
        Ok(())
    }

    fn opened(&mut self, vault: Vault) {
        let settings = self.settings.as_ref().expect("checked before opening");
        self.session = self
            .secrets
            .get(&settings.server, &settings.email)
            .map(|refresh_token| Session {
                access_token: Default::default(),
                refresh_token,
                expires_at: 0,
                user_key: vault.user_key().clone(),
            });
        self.vault = Some(vault);
        let kept = self.store.sync();
        self.policy = kept
            .as_deref()
            .map(LockPolicy::from_sync)
            .unwrap_or_default();
        self.generator_policy = kept
            .as_deref()
            .map(GeneratorPolicy::from_sync)
            .unwrap_or_default();
        self.autolock = AutoLock::new(Instant::now(), SystemTime::now());
    }

    /// The account's id: the open vault's, or the kept sync's.
    fn user_id(&self) -> Result<String> {
        if let Some(vault) = &self.vault {
            return Ok(vault.user_id.clone());
        }
        Ok(
            Sync::from_json(&self.store.sync().ok_or(Error::SignedOut)?)?
                .profile
                .id,
        )
    }

    fn agent(&self) -> Result<Agent> {
        let (address, program) = match &self.agent {
            Some(found) => found.clone(),
            None => Agent::installed()?,
        };
        Agent::connect(&address, &program, &self.user_id()?)
    }

    /// Whether this account can unlock with Windows Hello or Touch ID here
    /// (docs/core.md, "Unlock with Windows Hello or Touch ID").
    pub fn biometric_status(&self) -> Result<agent::Status> {
        self.agent()?.status()
    }

    /// Turns it on, while unlocked: the agent asks, then keeps the user key
    /// sealed (the same enrollment as the extension's). Returns whether the person agreed.
    pub fn enroll_biometric(&self) -> Result<bool> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        self.agent()?.enroll(vault.user_key(), &vault.email)
    }

    /// Unlocks the kept vault with the user key the agent returns. False if
    /// the person cancelled. A key that no longer opens the vault (the account's
    /// keys changed) turns it off, and the master password is needed.
    pub fn unlock_with_biometric(&mut self) -> Result<bool> {
        let mut agent = self.agent()?;
        let Some(key): Option<SymmetricKey> = agent.unlock()? else {
            return Ok(false);
        };
        match self.unlock_with_key(key) {
            Err(Error::Refused(why)) => {
                let _ = agent.remove();
                Err(Error::Refused(why))
            }
            other => other.map(|()| true),
        }
    }

    /// Opens the kept vault with a user key the OS kept for biometric unlock
    /// (the desktop agent's, or a phone's Keystore or Keychain), without the
    /// KDF. A key that no longer opens it (the account's keys changed) is
    /// refused, and the caller turns that unlock off.
    pub fn unlock_with_key(&mut self, key: SymmetricKey) -> Result<()> {
        let text = self.store.sync().ok_or(Error::SignedOut)?;
        match Vault::open(&Sync::from_json(&text)?, key) {
            Ok(vault) => {
                self.opened(vault);
                Ok(())
            }
            Err(Error::Refused(_)) | Err(Error::Damaged(_)) => Err(Error::Refused(
                "that unlock is out of date; use your master password, then turn it on again",
            )),
            Err(e) => Err(e),
        }
    }

    /// Turns it off (for the extension too: it's one enrollment). On Android
    /// the app's own Keystore key and the user key it sealed go instead.
    pub fn remove_biometric(&self) -> Result<()> {
        #[cfg(target_os = "android")]
        {
            crate::android::remove_biometric();
            Ok(())
        }
        #[cfg(all(target_os = "ios", feature = "ffi"))]
        {
            crate::ffi::remove_biometric();
            Ok(())
        }
        #[cfg(not(any(target_os = "android", all(target_os = "ios", feature = "ffi"))))]
        self.agent()?.remove()
    }

    /// Brings the vault up to date from the server and keeps the new sync.
    /// Signed out (the session ended) leaves the kept vault open, to read.
    pub fn sync(&mut self) -> Result<()> {
        let (Some(server), Some(settings)) = (&self.server, &self.settings) else {
            return Err(Error::SignedOut);
        };
        let Some(session) = self.session.as_mut() else {
            return Err(Error::SignedOut);
        };
        let synced = server.sync(session);
        let (sync, text) = match synced {
            Err(Error::SignedOut) => {
                self.session = None;
                self.secrets.forget(&settings.server, &settings.email);
                return Err(Error::SignedOut);
            }
            other => other?,
        };
        self.store.save_sync(&text)?;
        self.policy = LockPolicy::from_sync(&text);
        self.generator_policy = GeneratorPolicy::from_sync(&text);
        // The server may have issued a new refresh token.
        self.secrets
            .keep(&settings.server, &settings.email, &session.refresh_token);
        self.vault = Some(Vault::open(&sync, session.user_key().clone())?);
        Ok(())
    }

    /// When to lock by itself (docs/core.md, "Locking by itself").
    pub fn preferences(&self) -> Preferences {
        self.preferences
    }

    /// The organizations' "Vault lock" policy (docs/core.md, "Locking by policy").
    pub fn lock_policy(&self) -> LockPolicy {
        self.policy
    }

    /// What it locks by: the person's choices held to the policy.
    fn held(&self) -> Preferences {
        self.policy.apply(&self.preferences)
    }

    /// It's time: locks, or signs out if the policy says so.
    fn time_up(&mut self) {
        if self.policy.sign_out {
            self.sign_out();
        } else {
            self.lock();
        }
    }

    /// The generator's last choices, kept on this device, held to the
    /// organizations' policy.
    pub fn generator_options(&self) -> generator::Options {
        self.generator_policy.apply(&self.store.generator_options())
    }

    /// The organizations' "Password generator" policy (docs/core.md, "New passwords").
    pub fn generator_policy(&self) -> GeneratorPolicy {
        self.generator_policy
    }

    /// A new password or passphrase, from the choices held to the policy,
    /// which become the next choices. Answers what it made, and the choices
    /// it made it with.
    pub fn generate(
        &mut self,
        options: &generator::Options,
    ) -> Result<(Zeroizing<String>, generator::Options)> {
        let held = self.generator_policy.apply(options);
        let made = generator::generate(&held)?;
        self.store.save_generator_options(&held)?;
        Ok((made, held))
    }

    pub fn set_preferences(&mut self, preferences: Preferences) -> Result<()> {
        preferences.check()?;
        self.store.save_preferences(&preferences)?;
        self.preferences = preferences;
        Ok(())
    }

    /// The app was used just now (the screens say, at most about once a second).
    pub fn used(&mut self) {
        self.autolock.used(Instant::now(), SystemTime::now());
    }

    /// Locks if it's time to, from what the OS says; returns why it locked.
    /// The app asks every two seconds while unlocked.
    pub fn tick(&mut self, readings: &Readings) -> Option<Reason> {
        self.tick_at(readings, Instant::now(), SystemTime::now())
    }

    /// `tick`, at a given time (tests).
    pub fn tick_at(
        &mut self,
        readings: &Readings,
        now: Instant,
        wall: SystemTime,
    ) -> Option<Reason> {
        self.vault.as_ref()?;
        let held = self.held();
        let reason = self.autolock.check(&held, readings, now, wall)?;
        self.time_up();
        Some(reason)
    }

    /// The app left the screen (a phone's app switcher or home, a computer's
    /// window closed or minimized): locks if the person chose "Immediately"
    /// on a phone, or the policy says so. Returns whether it locked.
    pub fn left(&mut self) -> bool {
        if self.vault.is_none() || !self.held().immediately() {
            return false;
        }
        self.time_up();
        true
    }

    /// The OS says the computer is about to sleep: locks if the person chose
    /// that, or the policy says so. Returns whether it locked.
    pub fn sleeping(&mut self) -> bool {
        if self.vault.is_none() || !self.held().lock_on_sleep {
            return false;
        }
        self.time_up();
        true
    }

    /// Drops every key and everything decrypted.
    pub fn lock(&mut self) {
        self.vault = None;
        self.session = None;
        self.pending = None;
    }

    /// Forgets the vault and the session on this device, and unlocking with
    /// Windows Hello or Touch ID if it was on; keeps the server and email.
    pub fn sign_out(&mut self) {
        if self.store.sync().is_some() {
            let _ = self.remove_biometric();
        }
        if let Some(s) = &self.settings {
            self.secrets.forget(&s.server, &s.email);
        }
        self.store.forget_vault();
        self.lock();
    }

    /// Saves an item, new (no id) or edited, then syncs.
    pub fn save(
        &mut self,
        id: Option<&str>,
        draft: &ItemDraft,
        collection_ids: &[String],
    ) -> Result<()> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let editing = id.and_then(|id| vault.item(id));
        let body = vault.encrypt_item(draft, editing)?;
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.save_item(session, id, body, collection_ids)?;
        self.sync()
    }

    pub fn trash(&mut self, id: &str) -> Result<()> {
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.trash_item(session, id)?;
        self.sync()
    }

    /// Copies one of an item's values: "username", "password", "totp" (the
    /// current code), "notes", or "field:<index>". Secrets clear after 30 seconds.
    pub fn copy(&self, id: &str, what: &str) -> Result<()> {
        if what == "username" {
            return clipboard::copy_text(&self.value(id, what)?);
        }
        clipboard::copy_secret(&self.value(id, what)?, clipboard::CLEAR_AFTER)
    }

    /// The value `copy` copies. On phones the app's native piece puts it on
    /// the clipboard itself (docs/phones.md, "Copying").
    pub fn value(&self, id: &str, what: &str) -> Result<zeroize::Zeroizing<String>> {
        let item = self
            .vault
            .as_ref()
            .and_then(|v| v.item(id))
            .ok_or(Error::Damaged("no such item"))?;
        if what == "username" {
            return Ok(zeroize::Zeroizing::new(
                item.username.clone().unwrap_or_default(),
            ));
        }
        let secret = match what {
            "password" => item.password.as_ref().map(|p| p.as_str().to_owned()),
            "notes" => item.notes.as_ref().map(|n| n.as_str().to_owned()),
            "totp" => {
                let totp = crate::totp::Totp::parse(
                    item.totp
                        .as_deref()
                        .ok_or(Error::Damaged("no one-time code"))?,
                )?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                Some(totp.code_at(now))
            }
            other => other
                .strip_prefix("field:")
                .and_then(|i| i.parse::<usize>().ok())
                .and_then(|i| item.fields.get(i))
                .map(|f| f.value.as_str().to_owned()),
        }
        .map(zeroize::Zeroizing::new)
        .ok_or(Error::Damaged("nothing to copy"))?;
        Ok(secret)
    }
}
