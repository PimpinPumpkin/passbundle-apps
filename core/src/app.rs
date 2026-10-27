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
use crate::crypto::{EncString, SymmetricKey};
use crate::device_approval::{self, SignInRequest};
use crate::error::{Error, Result};
use crate::generator;
use crate::generator_policy::GeneratorPolicy;
use crate::kdf::{normalize_email, Prelogin};
use crate::lock_policy::LockPolicy;
use crate::secrets::SecretStore;
use crate::server::{Device, Download, PendingSignIn, Server, Session, SignIn, TwoFactor};
use crate::store::{Settings, Store};
use crate::synced_settings::{self, Known};
use crate::vault::{Item, ItemDraft, Sync, Vault};

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
    /// The item whose secrets the master password, asked for again, let
    /// out (docs/core.md, "Asking for the master password again"): until
    /// another item asks, or the vault locks.
    reprompted: Option<String>,
    /// Counts each time `reprompted` is given or forgotten, so an editor
    /// opened before can't save over an item's secrets (`grant`).
    grant: u64,
    autolock: AutoLock,
    /// The last item whose history was asked for, as the server sent it and
    /// opened (history.rs): to show a version's secrets, or restore it,
    /// without asking again. Forgotten when the vault locks.
    history: Option<History>,
    /// The generator's last password or passphrase, which its Copy copies
    /// (as `value("", "generated")`), so it clears like any secret. Forgotten
    /// when the vault locks.
    generated: Option<Zeroizing<String>>,
    /// Websites' icons, in memory while the vault is open (icons.rs).
    icons: crate::icons::Cache,
    /// Counts each time the icons are emptied (locking, signing out,
    /// turning them off), so one fetched before isn't kept after.
    icon_session: u64,
    /// Sign-in requests from the last listing, and when they were listed
    /// (docs/core.md, "Approving a sign-in"): an approval goes only to the
    /// key of one listed here, whose phrase the person saw. Forgotten when
    /// the vault locks.
    sign_in_requests: Option<(Instant, Vec<SignInRequest>)>,
    /// When the person last confirmed it's them (biometrics or the master
    /// password) to approve a sign-in; one confirmation, one approval.
    confirmed: Option<Instant>,
}

/// How long confirming it's you lasts, to approve a sign-in.
const CONFIRMATION_LASTS: std::time::Duration = std::time::Duration::from_secs(60);

/// An item's past versions, opened (`App::history`).
pub struct History {
    pub id: String,
    /// The person can't see the item's passwords: only when and by whom.
    pub hide_passwords: bool,
    /// Newest first; a version that doesn't open has no item.
    pub versions: Vec<Version>,
}

pub struct Version {
    pub id: String,
    pub revision_date: Option<String>,
    pub saved_by: serde_json::Value,
    pub item: Option<Item>,
}

impl App {
    pub fn open(store: Store, secrets: Box<dyn SecretStore>) -> App {
        let settings = store.settings();
        let server = settings
            .as_ref()
            .and_then(|s| Server::new(&s.server, Device::this_one(&s.device_identifier)).ok());
        let preferences = store.preferences();
        crate::icons::forget_kept(store.dir());
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
            reprompted: None,
            grant: 0,
            autolock: AutoLock::new(Instant::now(), SystemTime::now()),
            history: None,
            generated: None,
            icons: crate::icons::Cache::default(),
            icon_session: 0,
            sign_in_requests: None,
            confirmed: None,
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

    /// Makes an account on the server (docs/core.md, "Making an account"),
    /// then signs in to it, as `sign_in`. Slow: the master key twice, and a
    /// new RSA key pair (`register` takes one made beforehand, away from the
    /// app).
    pub fn sign_up(
        &mut self,
        server_url: &str,
        email: &str,
        name: Option<&str>,
        password: &str,
        hint: Option<&str>,
    ) -> Result<Step> {
        // A server address that can't be used is refused before the slow part.
        Server::new(server_url, Device::this_one(&Device::new_identifier()))?;
        let account = crate::signup::NewAccount::make(email, name, password, hint)?;
        self.register(server_url, &account, password)
    }

    /// Sends a new account to the server, then signs in to it with the
    /// settings it was made with (not what the server says now). Once the
    /// server has it, a failure says so (`Error::AccountMade`): signing in
    /// is what's left, not making it again.
    pub fn register(
        &mut self,
        server_url: &str,
        account: &crate::signup::NewAccount,
        password: &str,
    ) -> Result<Step> {
        let identifier = self.device_identifier();
        let server = Server::new(server_url, Device::this_one(&identifier))?;
        server.register(account)?;
        let prelogin = Prelogin::of(&account.kdf);
        self.sign_in_as(
            server_url,
            server,
            identifier,
            &prelogin,
            &account.email,
            password,
        )
        .map_err(|e| Error::AccountMade {
            sure: true,
            why: Box::new(e),
        })
    }

    /// One identifier per install, whichever account signs in.
    fn device_identifier(&self) -> String {
        self.settings
            .as_ref()
            .map(|s| s.device_identifier.clone())
            .unwrap_or_else(Device::new_identifier)
    }

    /// Signs in to a server: the vault comes down, is kept, and opens.
    pub fn sign_in(&mut self, server_url: &str, email: &str, password: &str) -> Result<Step> {
        let identifier = self.device_identifier();
        let server = Server::new(server_url, Device::this_one(&identifier))?;
        let prelogin = server.prelogin(email)?;
        self.sign_in_as(server_url, server, identifier, &prelogin, email, password)
    }

    fn sign_in_as(
        &mut self,
        server_url: &str,
        server: Server,
        identifier: String,
        prelogin: &Prelogin,
        email: &str,
        password: &str,
    ) -> Result<Step> {
        self.lock();
        let signed_in = server.sign_in_with(prelogin, email, password)?;
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
        // A server without them, or one that's gone for a moment, leaves the
        // generator's settings as they are here.
        let _ = self.sync_settings();
        Ok(())
    }

    /// The generator's settings and the server's copy (synced_settings):
    /// saves a change made here, or takes the server's, or gives the server
    /// these if it has none this account can read.
    pub fn sync_settings(&mut self) -> Result<()> {
        let (Some(server), Some(settings)) = (&self.server, &self.settings) else {
            return Err(Error::SignedOut);
        };
        let Some(session) = self.session.as_mut() else {
            return Err(Error::SignedOut);
        };
        let key = session.user_key().clone();
        let account = synced_settings::account(&settings.server, &settings.email);
        let here = self.store.generator_options();
        let known = self.store.synced_settings();
        let seen = synced_settings::seen(known.as_ref(), &account);
        let now = synced_settings::now_ms();
        let save = |session: &mut Session, doc: serde_json::Map<String, serde_json::Value>| {
            let data = key.encrypt_str(&synced_settings::text(&doc)).to_string();
            server.save_settings(session, &data).map(|()| doc)
        };
        let ours = |doc| Known {
            account: account.clone(),
            settings: doc,
            generator: here.clone(),
        };
        let known = match synced_settings::step(known.as_ref(), &account, &here, now) {
            synced_settings::Step::Save(doc) => ours(save(session, doc)?),
            synced_settings::Step::Read => {
                let remote = server.settings(session)?.and_then(|data| {
                    let text = EncString::parse(&data).ok()?.decrypt_string(&key).ok()?;
                    synced_settings::parse(&text)
                });
                match (remote, known) {
                    // None this account can read: this device's.
                    (None, _) => ours(save(
                        session,
                        synced_settings::document(&serde_json::Map::new(), &here, seen, now),
                    )?),
                    // Older than one this device has seen: a server handing
                    // back an old copy. This device's goes back instead.
                    (Some(doc), Some(k)) if synced_settings::rev_of(&doc) < seen => ours(save(
                        session,
                        synced_settings::document(&k.settings, &here, seen, now),
                    )?),
                    (Some(doc), _) => {
                        let taken = synced_settings::take(&doc, &here);
                        self.store.save_generator_options(&taken)?;
                        Known {
                            account,
                            settings: doc,
                            generator: taken,
                        }
                    }
                }
            }
        };
        self.store.save_synced_settings(&known)
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
        if self.vault.is_some() {
            self.generated = Some(Zeroizing::new(made.as_str().to_owned()));
        }
        Ok((made, held))
    }

    /// Takes what the generator shows as its last value, for its Copy to
    /// copy (`value("", "generated")`): what the person sees, even if
    /// something else made one since (Android's offer to save a sign-in
    /// opens an editor with a generator of its own). Only while unlocked.
    pub fn keep_generated(&mut self, shown: &str) -> Result<()> {
        if self.vault.is_none() {
            return Err(Error::SignedOut);
        }
        self.generated = Some(Zeroizing::new(shown.to_owned()));
        Ok(())
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
        self.sign_in_requests = None;
        self.confirmed = None;
        self.reprompted = None;
        self.grant += 1;
        self.history = None;
        self.generated = None;
        self.forget_icons();
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

    /// Checks the master password for an item that asks for it again; if
    /// it's right, that item's secrets may be used (`may_use`).
    pub fn confirm_master_password(&mut self, id: &str, password: &str) -> Result<()> {
        let settings = self.settings.as_ref().ok_or(Error::SignedOut)?;
        let text = self.store.sync().ok_or(Error::SignedOut)?;
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        vault.item(id).ok_or(Error::Damaged("no such item"))?;
        if !vault.is_master_password(&Sync::from_json(&text)?, &settings.prelogin(), password)? {
            return Err(Error::WrongPassword);
        }
        self.reprompted = Some(id.to_owned());
        self.grant += 1;
        Ok(())
    }

    /// Checks the master password as proof the person is who they are (a
    /// passkey's user verification), without granting anything.
    pub fn check_master_password(&self, password: &str) -> Result<()> {
        let settings = self.settings.as_ref().ok_or(Error::SignedOut)?;
        let text = self.store.sync().ok_or(Error::SignedOut)?;
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        if !vault.is_master_password(&Sync::from_json(&text)?, &settings.prelogin(), password)? {
            return Err(Error::WrongPassword);
        }
        Ok(())
    }

    /// Whether approving a sign-in asks for biometrics or the master
    /// password first, on this device (on until turned off).
    pub fn confirm_approvals(&self) -> bool {
        self.store.approvals().confirm
    }

    /// Turning it on needs nothing; turning it off needs the person to have
    /// confirmed it's them in the last minute (as approving does), and uses
    /// that confirmation up, so nobody at an unlocked device turns it off and
    /// approves without them.
    pub fn set_confirm_approvals(&mut self, confirm: bool) -> Result<()> {
        if !confirm && self.confirm_approvals() {
            let confirmed = self.take_confirmation();
            if !confirmed {
                return Err(Error::ConfirmFirst);
            }
        }
        self.store
            .save_approvals(&crate::store::Approvals { confirm })
    }

    /// Whether the person confirmed it's them in the last minute; either way
    /// the confirmation is used up.
    fn take_confirmation(&mut self) -> bool {
        self.confirmed
            .take()
            .is_some_and(|at| at.elapsed() < CONFIRMATION_LASTS)
    }

    /// The master password, as the person confirming it's them, to approve a
    /// sign-in or turn off asking for that.
    pub fn confirm_with_master_password(&mut self, password: &str) -> Result<()> {
        self.confirmed = None;
        self.check_master_password(password)?;
        self.confirmed = Some(Instant::now());
        Ok(())
    }

    /// Biometrics, as the person confirming it's them for the same: the user
    /// key the OS kept for biometric unlock (the phone's Keystore or
    /// Keychain), given back after its prompt, must be the open vault's
    /// (compared in constant time). Only the approval prompt's own call comes
    /// here, never unlocking or a passkey's check that it's the person. A
    /// key that isn't this vault's means that unlock is out of date.
    pub fn confirm_approval_with_key(&mut self, key: SymmetricKey) -> Result<()> {
        self.confirmed = None;
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        if !same_key(vault.user_key(), &key) {
            return Err(Error::Refused(
                "that unlock is out of date; use your master password, then turn it on again",
            ));
        }
        self.confirmed = Some(Instant::now());
        Ok(())
    }

    /// On a computer: Windows Hello or Touch ID through the desktop agent,
    /// its prompt saying it's to approve a sign-in, then
    /// `confirm_approval_with_key`. False if the person cancelled. A key that
    /// no longer opens the vault turns that unlock off, as unlocking does.
    pub fn confirm_approval_with_biometric(&mut self) -> Result<bool> {
        self.confirmed = None;
        let mut agent = self.agent()?;
        let Some(key) = agent.unlock_to_approve()? else {
            return Ok(false);
        };
        match self.confirm_approval_with_key(key) {
            Err(Error::Refused(why)) => {
                let _ = agent.remove();
                Err(Error::Refused(why))
            }
            other => other.map(|()| true),
        }
    }

    /// Sign-in requests waiting for this account's answer, newest first,
    /// from the server, while unlocked (docs/core.md, "Approving a
    /// sign-in"). They're kept, so an approval goes only to one of them.
    pub fn sign_in_requests(&mut self) -> Result<Vec<SignInRequest>> {
        let call = self.sign_in_requests_call()?;
        let fetched = call.fetch()?;
        self.keep_sign_in_requests(&call, fetched)
    }

    /// What asking the server for sign-in requests needs, so the bridge can
    /// ask without holding the app (the app asks every few seconds, and the
    /// screens read the core meanwhile): the server, and a session token
    /// that's good for a minute more (refreshed here first if not).
    pub fn sign_in_requests_call(&mut self) -> Result<SignInRequestsCall> {
        let email = self.vault.as_ref().ok_or(Error::SignedOut)?.email.clone();
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if session.expires_at <= now + 60 {
            server.refresh(session)?;
        }
        Ok(SignInRequestsCall {
            server: server.clone(),
            access_token: session.access_token.clone(),
            email,
        })
    }

    /// Keeps what the server listed (`SignInRequestsCall::fetch`), for the
    /// account it was asked for, if its vault is still the one open.
    pub fn keep_sign_in_requests(
        &mut self,
        call: &SignInRequestsCall,
        (body, server_now): (serde_json::Value, Option<u64>),
    ) -> Result<Vec<SignInRequest>> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        if vault.email != call.email {
            return Err(Error::SignedOut);
        }
        // The server's clock, or this device's if it didn't say.
        let server_now = server_now.unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        });
        let requests = device_approval::parse_pending(&body, &call.email, server_now);
        self.sign_in_requests = Some((Instant::now(), requests));
        Ok(self.kept_sign_in_requests())
    }

    /// The requests from the last listing as they stand now, without the
    /// server: one whose time is up is gone.
    pub fn kept_sign_in_requests(&self) -> Vec<SignInRequest> {
        let Some((listed, requests)) = &self.sign_in_requests else {
            return Vec::new();
        };
        let elapsed = listed.elapsed().as_secs();
        requests.iter().filter_map(|r| r.after(elapsed)).collect()
    }

    /// Approves or denies a listed sign-in request. Approving sends the user
    /// key wrapped to the request's public key, and only when:
    /// - `fingerprint` is the phrase of the request the core listed (what
    ///   the person saw), and the server still has it waiting with that key;
    /// - the person confirmed it's them in the last minute, where this
    ///   device asks for that (`confirm_approvals`).
    pub fn answer_sign_in_request(
        &mut self,
        id: &str,
        fingerprint: &str,
        approve: bool,
    ) -> Result<()> {
        let answered = self.answer_sign_in_request_once(id, fingerprint, approve);
        // One confirmation, one answer: used up by an approval, a denial, or
        // anything that went wrong.
        self.confirmed = None;
        answered
    }

    fn answer_sign_in_request_once(
        &mut self,
        id: &str,
        fingerprint: &str,
        approve: bool,
    ) -> Result<()> {
        let request = self
            .kept_sign_in_requests()
            .into_iter()
            .find(|r| r.id == id)
            .ok_or(Error::Refused(
                "that sign-in request has expired or was answered",
            ))?;
        if approve {
            if request.fingerprint != fingerprint {
                return Err(Error::Refused(
                    "the fingerprint phrase isn't that sign-in request's",
                ));
            }
            let confirmed = self
                .confirmed
                .is_some_and(|at| at.elapsed() < CONFIRMATION_LASTS);
            if self.confirm_approvals() && !confirmed {
                return Err(Error::ConfirmFirst);
            }
        }
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        // Still waiting, with the key the person saw the phrase of.
        let now = server.sign_in_request(session, id)?;
        if now["requestApproved"].as_bool().is_some() || !now["responseDate"].is_null() {
            return Err(Error::Refused("that sign-in request was already answered"));
        }
        let wrapped = if approve {
            let key = now["publicKey"].as_str().and_then(|k| {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, k).ok()
            });
            if key.as_deref() != Some(request.public_key()) {
                return Err(Error::Refused(
                    "that sign-in request changed since it was shown; deny it",
                ));
            }
            Some(device_approval::wrap_user_key(
                request.public_key(),
                vault.user_key(),
            )?)
        } else {
            None
        };
        server.answer_sign_in_request(session, id, wrapped.as_ref())?;
        if let Some((_, requests)) = &mut self.sign_in_requests {
            requests.retain(|r| r.id != id);
        }
        Ok(())
    }

    /// Which giving of the master password is current: the editor of an
    /// item that asks for it hands this back when saving, and a save from
    /// an editor opened under another is refused.
    pub fn grant(&self) -> u64 {
        self.grant
    }

    /// Whether an item's secrets may leave the core now: it doesn't ask for
    /// the master password again, or it was given for it.
    pub fn may_use(&self, item: &Item) -> bool {
        item.kept.reprompt == 0 || self.reprompted.as_deref() == Some(item.id.as_str())
    }

    /// An attachment's file, downloaded and opened (docs/core.md,
    /// "Attachments"): its name and contents, in memory.
    pub fn attachment(
        &mut self,
        id: &str,
        attachment_id: &str,
    ) -> Result<(String, Zeroizing<Vec<u8>>)> {
        self.attachment_to_fetch(id, attachment_id)?.open()
    }

    /// What fetching an attachment needs, once the server has said where its
    /// file is: the rest (the download, decrypting, saving) happens without
    /// the app, so a long download doesn't hold up its other calls.
    pub fn attachment_to_fetch(&mut self, id: &str, attachment_id: &str) -> Result<AttachmentFile> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let item = vault.item(id).ok_or(Error::Damaged("no such item"))?;
        if !self.may_use(item) {
            return Err(Error::MasterPasswordFirst);
        }
        let attachment = item
            .attachments
            .iter()
            .find(|a| a.id == attachment_id)
            .ok_or(Error::Damaged("no such attachment"))?;
        let key = attachment
            .key
            .clone()
            .ok_or(Error::Damaged("an attachment that can't be opened"))?;
        let name = attachment.name.clone();
        let (Some(server), Some(settings), Some(session)) =
            (&self.server, &self.settings, self.session.as_mut())
        else {
            return Err(Error::SignedOut);
        };
        let download = match server.attachment_download(session, id, attachment_id) {
            Err(Error::SignedOut) => {
                self.session = None;
                self.secrets.forget(&settings.server, &settings.email);
                return Err(Error::SignedOut);
            }
            other => other?,
        };
        // Asking may have refreshed the session, and the server may have
        // issued a new refresh token, as for a sync.
        self.secrets
            .keep(&settings.server, &settings.email, &session.refresh_token);
        Ok(AttachmentFile {
            name,
            key,
            download,
        })
    }

    /// Saves an attachment to the Downloads folder (the home folder if
    /// there's none), next to anything of the same name rather than over
    /// it; answers where.
    pub fn save_attachment(&mut self, id: &str, attachment_id: &str) -> Result<PathBuf> {
        self.attachment_to_fetch(id, attachment_id)?.save()
    }

    /// Saves an attachment in `dir` (an iPhone's Documents folder, which
    /// Files shows), the same way.
    pub fn save_attachment_in(
        &mut self,
        id: &str,
        attachment_id: &str,
        dir: &std::path::Path,
    ) -> Result<PathBuf> {
        self.attachment_to_fetch(id, attachment_id)?.save_in(dir)
    }

    /// Saves an item, new (no id) or edited, then syncs. An organization's
    /// item being edited goes into `collection_ids` (its shared folders) as
    /// well, when they're given and differ from the ones it's in.
    pub fn save(
        &mut self,
        id: Option<&str>,
        draft: &ItemDraft,
        collection_ids: &[String],
    ) -> Result<()> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let editing = id.and_then(|id| vault.item(id));
        // The editor reveals an item's secrets into its fields; one that asks
        // for the master password can't be edited before it's given, or its
        // empty fields would be saved over its secrets.
        if editing.is_some_and(|item| !self.may_use(item)) {
            return Err(Error::MasterPasswordFirst);
        }
        // Nor can one whose secrets the person may not see: the editor
        // would have saved blanks over them (docs/core.md, "Folders and
        // permissions"). The web vault edits those.
        if editing.is_some_and(|item| !vault.grid(item).see_passwords) {
            return Err(Error::Refused(
                "you can't see this item's passwords, so edit it in the web vault",
            ));
        }
        let moved = editing.filter(|item| {
            let mut held = item.collection_ids.clone();
            let mut asked = collection_ids.to_vec();
            held.sort();
            asked.sort();
            asked.dedup();
            item.organization_id.is_some() && !asked.is_empty() && held != asked
        });
        // A move between shared folders is checked as the server checks it
        // (change_item_folders), before anything is saved: share in each
        // folder it leaves, and create in each it joins, with share on the
        // item. Folders the person doesn't reach stay as they are.
        if let Some(item) = moved {
            let grid = vault.grid(item);
            if !grid.edit {
                return Err(Error::Refused(
                    "your organization doesn't let you change this item's shared folders",
                ));
            }
            let leaves = item
                .collection_ids
                .iter()
                .filter(|c| !collection_ids.contains(c));
            for c in leaves {
                if vault.collection_grids.get(c).is_some_and(|g| !g.share) {
                    return Err(Error::Refused(
                        "your organization doesn't let you move items out of this shared folder",
                    ));
                }
            }
            let joins = collection_ids
                .iter()
                .filter(|c| !item.collection_ids.contains(c));
            for c in joins {
                if !grid.share || !vault.collection_grids.get(c).is_some_and(|g| g.create) {
                    return Err(Error::Refused(
                        "your organization doesn't let you add this item to that shared folder",
                    ));
                }
            }
        }
        let moved = moved.is_some();
        let body = vault.encrypt_item(draft, editing)?;
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.save_item(session, id, body, collection_ids)?;
        // Then its shared folders, which the server changes on their own:
        // saved either way, so a refusal still leaves this device up to date.
        let placed = match (moved, id) {
            (true, Some(id)) => server.set_item_collections(session, id, collection_ids),
            _ => Ok(()),
        };
        // The move's refusal, if any, before the sync's.
        let synced = self.sync();
        placed.and(synced)
    }

    /// Makes a folder, its name under the user key, as folders are
    /// (docs/core.md, "Items"), then syncs. Answers its id.
    pub fn create_folder(&mut self, name: &str) -> Result<String> {
        let name = Self::folder_name(name)?;
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let encrypted = vault.user_key().encrypt_str(&name).to_string();
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        let id = server.create_folder(session, &encrypted)?;
        self.sync()?;
        Ok(id)
    }

    /// Renames a folder, then syncs.
    pub fn rename_folder(&mut self, id: &str, name: &str) -> Result<()> {
        let name = Self::folder_name(name)?;
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        if !vault.folders.contains_key(id) {
            return Err(Error::Damaged("no such folder"));
        }
        let encrypted = vault.user_key().encrypt_str(&name).to_string();
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.rename_folder(session, id, &encrypted)?;
        self.sync()
    }

    /// Deletes a folder, then syncs; its items stay, in no folder.
    pub fn delete_folder(&mut self, id: &str) -> Result<()> {
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.delete_folder(session, id)?;
        self.sync()
    }

    /// A folder's name, trimmed; refused when empty.
    fn folder_name(name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Refused("a folder needs a name"));
        }
        Ok(name.to_owned())
    }

    /// The logins with a passkey for a site (by its ID), that the core can
    /// sign with, for a phone's list of passkeys (docs/phones.md, "Passkeys").
    pub fn passkeys_for(&self, rp_id: &str) -> Vec<&Item> {
        let Some(vault) = self.vault.as_ref() else {
            return Vec::new();
        };
        vault
            .items
            .iter()
            .filter(|i| !i.deleted)
            .filter(|i| {
                i.passkey
                    .as_ref()
                    .is_some_and(|p| p.usable && p.rp_id == rp_id)
            })
            .collect()
    }

    /// Signs in to a site with a login's passkey (docs/core.md, "Passkeys"):
    /// the authenticator data and the signature over it and the client data
    /// hash. `user_verified` says the person proved who they are in this
    /// sign-in. A login that asks for the master password asks first (and
    /// giving it counts as proof). A passkey that counts its uses has its
    /// counter raised and the login saved before it signs; offline, it's
    /// refused.
    pub fn passkey_assert(
        &mut self,
        id: &str,
        rp_id: &str,
        client_data_hash: &[u8; 32],
        user_verified: bool,
    ) -> Result<PasskeyAssertion> {
        use crate::passkey::{authenticator_data, credential_id, flags};
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let item = vault.item(id).ok_or(Error::Damaged("no such item"))?;
        if !self.may_use(item) {
            return Err(Error::MasterPasswordFirst);
        }
        if item.deleted {
            return Err(Error::Refused("that login is in the trash"));
        }
        // Proof is what the caller just asked for, never an earlier grant.
        let verified = user_verified;
        let passkey = item
            .passkey
            .as_ref()
            .ok_or(Error::Damaged("a login with no passkey"))?;
        if passkey.rp_id != rp_id {
            return Err(Error::Refused("it's a passkey for another site"));
        }
        let key = passkey.key()?;
        let raw_id = credential_id(&passkey.credential_id)?;
        let user_handle = match &passkey.user_handle {
            Some(handle) => crate::passkey::base64_either(handle)
                .ok_or(Error::Damaged("a passkey's user handle that isn't base64"))?,
            None => Vec::new(),
        };
        let mut counter = passkey.counter;
        if counter > 0 {
            counter = counter
                .checked_add(1)
                .ok_or(Error::Refused("a passkey's counter is at its end"))?;
            let stored = item
                .kept
                .fido2_credentials
                .as_ref()
                .ok_or(Error::Damaged("a login with no passkey"))?;
            let body = vault.encrypt_item_with(
                &ItemDraft::of(item),
                Some(item),
                Some(passkey.with_counter(stored, counter)),
            )?;
            let collection_ids = item.collection_ids.clone();
            let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
                return Err(Error::SignedOut);
            };
            server.save_item(session, Some(id), body, &collection_ids)?;
            // Saved: a sync that fails now only leaves this device behind, and
            // the next use is refused by the server's revision check rather
            // than reusing a counter.
            let _ = self.sync();
        }
        let mut set = flags::USER_PRESENT | flags::BACKUP_ELIGIBLE | flags::BACKED_UP;
        if verified {
            set |= flags::USER_VERIFIED;
        }
        let authenticator_data = authenticator_data(rp_id, set, counter, None);
        let signature = key.sign(&authenticator_data, client_data_hash);
        Ok(PasskeyAssertion {
            authenticator_data,
            signature,
            credential_id: raw_id,
            user_handle,
        })
    }

    /// Makes a passkey for a site (docs/core.md, "Passkeys", "Making one"):
    /// a new ES256 key in upstream's format, saved into the login `new.login`
    /// names (replacing a passkey it had, and filling in its username if it
    /// had none) or a new login for the site, before the site hears of it.
    /// Refused, with nothing made, if the vault already has one of the site's
    /// `exclude` credentials; offline, it's refused too.
    pub fn passkey_create(&mut self, new: &NewPasskey<'_>) -> Result<PasskeyMade> {
        use crate::passkey::{
            attestation_object, authenticator_data, credential_id, flags, Attested, Key,
        };
        use base64::Engine;
        let b64url = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let excluded = vault.items.iter().filter(|i| !i.deleted).any(|i| {
            i.passkey.as_ref().is_some_and(|p| {
                p.rp_id == new.rp_id
                    && credential_id(&p.credential_id).is_ok_and(|id| new.exclude.contains(&id))
            })
        });
        if excluded {
            return Err(Error::Refused(ALREADY_HAS_PASSKEY));
        }
        let editing = match new.login {
            Some(id) => {
                let item = vault.item(id).ok_or(Error::Damaged("no such item"))?;
                if item.kind != crate::vault::ItemKind::Login {
                    return Err(Error::Refused("a passkey goes in a login"));
                }
                if !self.may_use(item) {
                    return Err(Error::MasterPasswordFirst);
                }
                Some(item)
            }
            None => None,
        };
        let mut draft = match editing {
            Some(item) => ItemDraft::of(item),
            // Named after the site's ID, which the passkey is bound to, not
            // the name the site gives itself, which any site can make up.
            None => ItemDraft {
                name: new.rp_id.to_owned(),
                uris: vec![format!("https://{}", new.rp_id)],
                ..ItemDraft::default()
            },
        };
        if draft.username.as_deref().is_none_or(str::is_empty) {
            draft.username = new.user_name.map(str::to_owned);
        }
        let key = Key::generate();
        let stored_id = crate::passkey::new_credential_id();
        let raw_id = credential_id(&stored_id)?;
        let value_key = vault.value_key(&draft, editing)?;
        let enc = |text: &str| serde_json::Value::String(value_key.encrypt_str(text).to_string());
        let credential = serde_json::json!([{
            "credentialId": enc(&stored_id),
            "keyType": enc("public-key"),
            "keyAlgorithm": enc("ECDSA"),
            "keyCurve": enc("P-256"),
            "keyValue": enc(&key.to_stored()?),
            "rpId": enc(new.rp_id),
            "rpName": new.rp_name.map(enc),
            "userHandle": enc(&b64url(new.user_handle)),
            "userName": new.user_name.map(enc),
            "counter": enc("0"),
            "userDisplayName": new.user_display_name.map(enc),
            "discoverable": enc(if new.discoverable { "true" } else { "false" }),
            "creationDate": crate::vault::now_iso8601(),
        }]);
        let body = vault.encrypt_item_with(&draft, editing, Some(credential))?;
        let collection_ids = editing
            .map(|i| i.collection_ids.clone())
            .unwrap_or_default();
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        let saved = server.save_item(session, new.login, body, &collection_ids)?;
        let item_id = match new.login {
            Some(id) => id.to_owned(),
            None => saved["id"]
                .as_str()
                .map(str::to_owned)
                .ok_or(Error::Damaged("the server didn't say which login it saved"))?,
        };
        // Saved: the site gets its answer even if this sync fails.
        let _ = self.sync();
        let mut set = flags::USER_PRESENT | flags::BACKUP_ELIGIBLE | flags::BACKED_UP;
        if new.user_verified {
            set |= flags::USER_VERIFIED;
        }
        let authenticator_data = authenticator_data(
            new.rp_id,
            set,
            0,
            Some(&Attested {
                credential_id: &raw_id,
                public: key.public(),
            }),
        );
        Ok(PasskeyMade {
            attestation_object: attestation_object(&authenticator_data),
            authenticator_data,
            public_key: key.public_der()?,
            credential_id: raw_id,
            item_id,
        })
    }

    /// Favorites an item or not, then syncs: the person's own mark, as
    /// upstream's partial update keeps it, so an item they can't edit takes
    /// it too.
    pub fn set_favorite(&mut self, id: &str, favorite: bool) -> Result<()> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let item = vault.item(id).ok_or(Error::Damaged("no such item"))?;
        let folder = item.folder_id.clone();
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.set_favorite(session, id, folder.as_deref(), favorite)?;
        self.sync()
    }

    /// How the app looks on this device: whether logins show their
    /// websites' icons.
    pub fn appearance(&self) -> crate::store::Appearance {
        self.store.appearance()
    }

    pub fn set_appearance(&mut self, appearance: crate::store::Appearance) -> Result<()> {
        self.store.save_appearance(&appearance)?;
        if !appearance.show_icons {
            self.forget_icons();
        }
        Ok(())
    }

    /// What fetching a website's icon needs, away from the app (ffi.rs,
    /// `pb_icon`): the server's address, and which emptying of the icons
    /// it's for (`keep_icon`), while the vault is open and icons are on.
    pub fn icon_source(&self) -> Option<(String, u64)> {
        self.vault.as_ref()?;
        if !self.store.appearance().show_icons {
            return None;
        }
        let server = self.settings.as_ref()?.server.clone();
        Some((server, self.icon_session))
    }

    /// A host's icon as the server gave it while the vault's been open:
    /// Some(Some(icon)), Some(None) when it had none, None to ask it.
    pub fn kept_icon(&mut self, host: &str) -> Option<Option<&[u8]>> {
        self.icon_source()?;
        self.icons.get(host)
    }

    /// Keeps what the server answered for a host, if the icons haven't been
    /// emptied since `icon_source` said `session`.
    pub fn keep_icon(&mut self, session: u64, host: &str, icon: Option<Vec<u8>>) {
        if self.icon_source().is_some_and(|(_, now)| now == session) {
            self.icons.put(host, icon);
        }
    }

    fn forget_icons(&mut self) {
        self.icons.clear();
        self.icon_session += 1;
    }

    /// An item's past versions (history.rs), from the server, opened: kept
    /// until another item's are asked for, or the vault locks.
    pub fn history(&mut self, id: &str) -> Result<&History> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let item = vault.item(id).ok_or(Error::Damaged("no such item"))?;
        if !self.may_use(item) {
            return Err(Error::MasterPasswordFirst);
        }
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        let answer = server.item_history(session, id)?;
        let hide_passwords = answer["hidePasswords"].as_bool() != Some(false);
        let versions = answer["data"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| Version {
                id: v["id"].as_str().unwrap_or_default().to_owned(),
                revision_date: v["revisionDate"].as_str().map(str::to_owned),
                saved_by: v["savedBy"].clone(),
                // Without the item's passwords, the server sends no contents.
                item: (!hide_passwords && v["cipher"].is_object())
                    .then(|| vault.open_version(&v["cipher"]).ok())
                    .flatten(),
            })
            .collect();
        self.history = Some(History {
            id: id.to_owned(),
            hide_passwords,
            versions,
        });
        Ok(self.history.as_ref().expect("just kept"))
    }

    /// The kept history of an item, if it's the one asked for last.
    pub fn kept_history(&self, id: &str) -> Option<&History> {
        self.history.as_ref().filter(|h| h.id == id)
    }

    /// Puts an item back as one of its versions had it (asked for with
    /// `history`), as a normal edit, then syncs.
    pub fn restore_version(&mut self, id: &str, version_id: &str) -> Result<()> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let current = vault.item(id).ok_or(Error::Damaged("no such item"))?;
        if !self.may_use(current) {
            return Err(Error::MasterPasswordFirst);
        }
        let grid = vault.grid(current);
        if !grid.edit || !grid.see_passwords || current.deleted {
            return Err(Error::Refused("you can't restore this item's versions"));
        }
        let version = self
            .kept_history(id)
            .and_then(|h| h.versions.iter().find(|v| v.id == version_id))
            .and_then(|v| v.item.as_ref())
            .ok_or(Error::Damaged("no such version"))?;
        let body = vault.encrypt_restore(version, current)?;
        let collections = current.collection_ids.clone();
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.save_item(session, Some(id), body, &collections)?;
        self.history = None;
        self.sync()
    }

    /// Clears an item's history for everyone (no `version_id`), or deletes
    /// one version, where the person may (the server decides too).
    pub fn delete_history(&mut self, id: &str, version_id: Option<&str>) -> Result<()> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let item = vault.item(id).ok_or(Error::Damaged("no such item"))?;
        if !vault.may_clear_history(item) {
            return Err(Error::Refused("you can't clear this item's history"));
        }
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.delete_history(session, id, version_id)?;
        self.history = None;
        Ok(())
    }

    pub fn trash(&mut self, id: &str) -> Result<()> {
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.trash_item(session, id)?;
        self.sync()
    }

    /// Copies one of an item's values: "username", "password", "totp" (the
    /// current code), "notes", "field:<index>", or a card's or an identity's
    /// (`Item::detail`). Secrets clear after 30 seconds.
    pub fn copy(&self, id: &str, what: &str) -> Result<()> {
        if id.is_empty() && what == "generated" {
            return clipboard::copy_secret(&self.value(id, what)?, clipboard::CLEAR_AFTER);
        }
        let item = self
            .vault
            .as_ref()
            .and_then(|v| v.item(id))
            .ok_or(Error::Damaged("no such item"))?;
        let plain_detail = item.details.iter().any(|d| d.key == what && !d.secret);
        if what == "username" || crate::vault::PLAIN_DETAILS.contains(&what) || plain_detail {
            return clipboard::copy_text(&self.value_to_copy(id, what)?);
        }
        clipboard::copy_secret(&self.value_to_copy(id, what)?, clipboard::CLEAR_AFTER)
    }

    /// The value `copy` copies, on a computer or (through its native piece)
    /// a phone: never a secret the person's shared folder hides from them,
    /// as the screens never offer it (`Item::is_secret`).
    pub fn value_to_copy(&self, id: &str, what: &str) -> Result<zeroize::Zeroizing<String>> {
        if let Some(vault) = self.vault.as_ref() {
            if let Some(item) = vault.item(id) {
                if item.is_secret(what) && !vault.grid(item).see_passwords {
                    return Err(Error::Refused(
                        "your organization hides this item's passwords from you",
                    ));
                }
            }
        }
        self.value(id, what)
    }

    /// The value `copy` copies. On phones the app's native piece puts it on
    /// the clipboard itself (docs/phones.md, "Copying").
    pub fn value(&self, id: &str, what: &str) -> Result<zeroize::Zeroizing<String>> {
        // The generator's last password, for its Copy.
        if id.is_empty() && what == "generated" {
            return self
                .generated
                .as_ref()
                .filter(|_| self.vault.is_some())
                .map(|g| Zeroizing::new(g.as_str().to_owned()))
                .ok_or(Error::Damaged("nothing to copy"));
        }
        let item = self
            .vault
            .as_ref()
            .and_then(|v| v.item(id))
            .ok_or(Error::Damaged("no such item"))?;
        if what != "username" && !self.may_use(item) {
            return Err(Error::MasterPasswordFirst);
        }
        if what == "username" {
            return Ok(zeroize::Zeroizing::new(
                item.username.clone().unwrap_or_default(),
            ));
        }
        if let Some(detail) = item.detail(what) {
            return Ok(detail);
        }
        if let Some(detail) = item.details.iter().find(|d| d.key == what) {
            return Ok(Zeroizing::new(detail.value.as_str().to_owned()));
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

/// Why making a passkey was refused when the site already has one of its
/// credentials in the vault (WebAuthn's InvalidStateError).
pub const ALREADY_HAS_PASSKEY: &str =
    "PassBundle already has a passkey for this account on this site";

/// Asking the server for sign-in requests without holding the app
/// (`App::sign_in_requests_call`).
pub struct SignInRequestsCall {
    server: Server,
    access_token: Zeroizing<String>,
    email: String,
}

impl SignInRequestsCall {
    /// The server's list, and its clock.
    pub fn fetch(&self) -> Result<(serde_json::Value, Option<u64>)> {
        self.server.pending_sign_in_requests(&self.access_token)
    }
}

/// Whether two keys are the same, in constant time.
fn same_key(a: &SymmetricKey, b: &SymmetricKey) -> bool {
    let (a, b) = (a.to_bytes(), b.to_bytes());
    a.len() == b.len() && a.iter().zip(b.iter()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// A passkey to make (`App::passkey_create`): what the site asked for.
pub struct NewPasskey<'a> {
    pub rp_id: &'a str,
    pub rp_name: Option<&'a str>,
    /// The site's ID for the account.
    pub user_handle: &'a [u8],
    pub user_name: Option<&'a str>,
    pub user_display_name: Option<&'a str>,
    /// The person proved who they are in this sign-up.
    pub user_verified: bool,
    pub discoverable: bool,
    /// The login it goes in; None for a new login for the site.
    pub login: Option<&'a str>,
    /// The site's credentials it already has (`excludeCredentials`), raw.
    pub exclude: &'a [Vec<u8>],
}

/// What making a passkey answers (WebAuthn's attestation), for the OS to
/// hand the site, and the login it went in.
pub struct PasskeyMade {
    pub credential_id: Vec<u8>,
    pub attestation_object: Vec<u8>,
    pub authenticator_data: Vec<u8>,
    /// SubjectPublicKeyInfo, DER.
    pub public_key: Vec<u8>,
    pub item_id: String,
}

/// What signing in with a passkey answers (WebAuthn's assertion), for the
/// OS to hand the site.
pub struct PasskeyAssertion {
    pub authenticator_data: Vec<u8>,
    pub signature: Vec<u8>,
    pub credential_id: Vec<u8>,
    pub user_handle: Vec<u8>,
}

/// An attachment to fetch, open, and save (`App::attachment_to_fetch`),
/// away from the app: nothing here needs it.
pub struct AttachmentFile {
    name: String,
    key: SymmetricKey,
    download: Download,
}

impl AttachmentFile {
    /// Its name and contents: downloaded, then opened in memory.
    pub fn open(self) -> Result<(String, Zeroizing<Vec<u8>>)> {
        let file = self.download.fetch()?;
        Ok((self.name, crate::crypto::decrypt_file(file, &self.key)?))
    }

    /// Saves it to the Downloads folder (the home folder if there's none).
    pub fn save(self) -> Result<PathBuf> {
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .ok_or(Error::Damaged("no folder to save to"))?;
        let path = self.save_in(&dir)?;
        mark_downloaded(&path);
        Ok(path)
    }

    /// Saves it in `dir`, next to anything of the same name rather than
    /// over it, under a name that can't leave the folder; answers where.
    pub fn save_in(self, dir: &std::path::Path) -> Result<PathBuf> {
        use std::io::Write;
        let (name, contents) = self.open()?;
        let (path, mut file) = create_free(dir, &name)?;
        let written = file
            .write_all(contents.as_slice())
            .and_then(|()| file.sync_all());
        drop(file);
        if written.is_err() {
            // Nothing half written stays behind; the file is this call's own.
            let _ = std::fs::remove_file(&path);
            return Err(Error::Damaged("the file couldn't be written"));
        }
        Ok(path)
    }

    /// Writes it into a file the OS opened for it (Android's Downloads,
    /// through MediaStore), so it never passes through Java.
    #[cfg(unix)]
    pub fn write_to(self, fd: std::os::fd::RawFd) -> Result<()> {
        use std::io::Write;
        use std::os::fd::FromRawFd;
        let (_, contents) = self.open()?;
        // SAFETY: `fd` is an open, writable file the caller owns and closes;
        // ManuallyDrop keeps this from closing it too.
        let mut file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
        file.write_all(contents.as_slice())
            .and_then(|()| file.flush())
            .map_err(|_| Error::Damaged("the file couldn't be written"))
    }
}

/// Marks a saved file as downloaded, as browsers do, so the OS asks before
/// opening an app or script that came in one (macOS's quarantine, Windows'
/// Mark of the Web). Best effort: the file is saved either way.
fn mark_downloaded(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let value = format!("0081;{now:08x};PassBundle;");
        if let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) {
            // SAFETY: both strings are NUL-terminated and live for the call.
            unsafe {
                libc::setxattr(
                    path.as_ptr(),
                    c"com.apple.quarantine".as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                    0,
                );
            }
        }
    }
    #[cfg(windows)]
    {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let _ = std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\n");
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = path;
}

/// Creates a new file in `dir` for one named `name`: the name made safe
/// (`safe_name`), then "name (1).ext", and so on. Each try creates the file
/// only if nothing is there, not even a link, so it never writes over
/// anything or through a link out of the folder. Only the person can read it.
fn create_free(dir: &std::path::Path, name: &str) -> Result<(PathBuf, std::fs::File)> {
    let safe = safe_name(name);
    let (stem, ext) = match safe.rfind('.') {
        Some(at) if at > 0 => (&safe[..at], &safe[at..]),
        _ => (safe.as_str(), ""),
    };
    for n in 0..10_000 {
        let path = if n == 0 {
            dir.join(&safe)
        } else {
            dir.join(format!("{stem} ({n}){ext}"))
        };
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(Error::Damaged("the file couldn't be written")),
        }
    }
    Err(Error::Damaged("the file couldn't be written"))
}

/// An attachment's name made safe for a file name: one name in the folder
/// (no separators or other characters file systems refuse), not hidden, not
/// one of Windows' device names, without the invisible characters that can
/// make "invoice\u{202E}fdp.exe" look like "invoiceexe.pdf", and short
/// enough for any file system.
fn safe_name(name: &str) -> String {
    /// Invisible characters that change how a name reads: bidirectional
    /// controls, zero-width ones, and other formatting characters.
    fn invisible(c: char) -> bool {
        matches!(c,
            '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}')
    }
    let safe: String = name
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|".contains(c) || c.is_control() || invisible(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Not hidden (no leading dot, spaces before it or not), and no trailing
    // dots or spaces, which Windows drops.
    let safe = safe
        .trim_start_matches(|c: char| c == '.' || c.is_whitespace())
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    let mut safe = if safe.is_empty() {
        "attachment".to_owned()
    } else {
        safe.to_owned()
    };
    // Windows' device names, with or without an extension, and with spaces
    // before it, aren't files.
    let device = safe
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_uppercase();
    let numbered = |prefix: &str| {
        device.strip_prefix(prefix).is_some_and(|n| {
            matches!(
                n,
                "0" | "1"
                    | "2"
                    | "3"
                    | "4"
                    | "5"
                    | "6"
                    | "7"
                    | "8"
                    | "9"
                    | "\u{B9}"
                    | "\u{B2}"
                    | "\u{B3}"
            )
        })
    };
    if matches!(
        device.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT")
    {
        safe.insert(0, '_');
    }
    // At most 200 bytes, which leaves room for " (n)" within the usual
    // 255: the stem shortens, at a character's edge, and the extension stays.
    const MOST: usize = 200;
    if safe.len() > MOST {
        let (stem, ext) = match safe.rfind('.') {
            Some(at) if at > 0 && safe.len() - at <= 16 => (&safe[..at], &safe[at..]),
            _ => (safe.as_str(), ""),
        };
        let mut end = MOST - ext.len();
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        safe = format!("{}{ext}", &stem[..end]);
    }
    safe
}

#[cfg(test)]
mod attachment_tests {
    use super::{create_free, safe_name};

    fn new_dir() -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pb-free-path-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_saved_file_never_replaces_another_or_leaves_its_folder() {
        let dir = new_dir();
        let made = |name: &str| create_free(&dir, name).unwrap().0;
        assert_eq!(made("codes.txt"), dir.join("codes.txt"));
        assert_eq!(made("codes.txt"), dir.join("codes (1).txt"));
        assert_eq!(made("codes.txt"), dir.join("codes (2).txt"));
        assert_eq!(made("../../etc/passwd"), dir.join("_.._etc_passwd"));
        assert_eq!(made(""), dir.join("attachment"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_folder_is_never_written_through() {
        use std::os::unix::fs::PermissionsExt;
        let dir = new_dir();
        let outside = dir.join("outside");
        std::fs::create_dir_all(dir.join("downloads")).unwrap();
        let downloads = dir.join("downloads");
        // A dangling link, and one to a file that's there.
        std::os::unix::fs::symlink(&outside, downloads.join("codes.txt")).unwrap();
        std::fs::write(dir.join("kept.txt"), "kept").unwrap();
        std::os::unix::fs::symlink(dir.join("kept.txt"), downloads.join("notes.txt")).unwrap();
        let (path, _) = create_free(&downloads, "codes.txt").unwrap();
        assert_eq!(path, downloads.join("codes (1).txt"));
        assert!(!outside.exists());
        let (path, _) = create_free(&downloads, "notes.txt").unwrap();
        assert_eq!(path, downloads.join("notes (1).txt"));
        assert_eq!(
            std::fs::read_to_string(dir.join("kept.txt")).unwrap(),
            "kept"
        );
        // Only the person can read what's saved.
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// macOS asks before opening an app or script saved from an attachment.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_saved_file_is_marked_as_downloaded() {
        use super::mark_downloaded;
        let dir = new_dir();
        let (path, _) = create_free(&dir, "tool.command").unwrap();
        mark_downloaded(&path);
        let marked = std::process::Command::new("xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&marked.stdout).contains(";PassBundle;"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(safe_name(".hidden"), "hidden");
        assert_eq!(safe_name(" .bashrc"), "bashrc");
        assert_eq!(safe_name("notes. "), "notes");
        assert_eq!(safe_name("nul.txt"), "_nul.txt");
        assert_eq!(safe_name("COM1"), "_COM1");
        assert_eq!(safe_name("CON .txt"), "_CON .txt");
        assert_eq!(safe_name("com\u{B9}.txt"), "_com\u{B9}.txt");
        assert_eq!(safe_name("CONIN$"), "_CONIN$");
        assert_eq!(safe_name("console.txt"), "console.txt");
        assert_eq!(safe_name("invoice\u{202E}fdp.exe"), "invoice_fdp.exe");
        assert_eq!(safe_name("a\u{200B}b.txt"), "a_b.txt");
        let long = format!("{}.pdf", "\u{e9}".repeat(150));
        let short = safe_name(&long);
        assert!(short.len() <= 200 && short.ends_with(".pdf"), "{short}");
        assert!(short.starts_with('\u{e9}'));
    }
}
