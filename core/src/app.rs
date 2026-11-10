//! One account's state, as the screens see it: signed out, locked, or
//! unlocked, and everything they ask for. One per account signed in on this
//! device (accounts.rs, which the bridge, ffi.rs, holds), each with its own
//! folder, keys, choices, and lock. See docs/core.md: signing in, two-step
//! login, the new-device check, unlocking from the kept sync, and syncing
//! with the kept refresh token.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

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
use crate::new_device;
use crate::secrets::{Secret, SecretStore};
use crate::security_key::{KeyAnswer, SecurityKeyRequest};
use crate::server::{Device, Download, Next, PendingSignIn, Server, Session, SignIn, TwoFactor};
use crate::store::{Settings, Store};
use crate::synced_settings::{self, Known};
use crate::two_step::{self, Offer};
use crate::vault::{Item, ItemDraft, Sync, Vault};

mod bulk;
pub use bulk::{Bulk, Outcome, Plan, Skip};
mod files;
pub use files::{attach, too_large, Adding, Attached, Phase, Progress, MAX_FILE};

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

/// An item saved (`App::save`): its id, and whether the sync after it came
/// through. When it didn't, the item is saved all the same, and this device
/// shows it; the rest of the vault catches up at the next sync.
#[derive(Debug, PartialEq, Eq)]
pub struct Saved {
    pub id: String,
    pub synced: bool,
}

/// A second step made ready (Duo's address, a security key's request), or
/// a sign-in that went through without it.
#[derive(Debug, PartialEq, Eq)]
pub enum Prepared<T> {
    Ready(T),
    SignedIn,
}

/// Where signing in got to.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Done,
    /// The second step, with what the server offers (two_step.rs).
    TwoFactor(Offer),
    /// The new-device check holds it (new_device.rs): the number to type on
    /// another of the account's devices, and how it stands. Ask how it
    /// stands every few seconds (`new_device_status`).
    NewDevice(new_device::Check),
}

pub struct App {
    /// This account's own id on this device (accounts.rs): its folder's
    /// name, and what a phone's biometric unlock keeps its key under. Empty
    /// for an app opened on a folder of its own (`App::open`).
    id: String,
    store: Store,
    /// What's the device's, not the account's: how the app looks, and
    /// whether screenshots are allowed. The account's own store, for an app
    /// opened on a folder of its own.
    device: Store,
    /// This install's device identifier, for signing in when this account
    /// has none kept yet (accounts.rs): one per install, as upstream's apps
    /// have it.
    install_identifier: Option<String>,
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
    /// Attachments this app started on the server that the item doesn't
    /// show: on their way, or to delete there (files.rs). In memory only.
    uploads: Vec<files::Unfinished>,
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
    /// Opened beside the app by iOS's AutoFill extension
    /// (`Accounts::open_beside`): it never syncs, which would write the
    /// app's kept vault in the folder the app keeps it in. A passkey it saves
    /// goes to the server, and the app's next sync brings it.
    beside: bool,
    /// With "Immediately": autofill or a passkey unlocked the vault away
    /// from the app's screens and showed what there is to pick (`offered`).
    /// It locks when the pick's answer is handed back (`left`), or at this
    /// time (on either clock) if nothing is picked.
    pick_by: Option<(Instant, SystemTime)>,
    /// The person's own logins' numbers last told the server this unlock
    /// (DESIGN.md in the meta repo, "Members' own passwords"), and whether
    /// the server doesn't take them; forgotten when the vault locks.
    own_health: Option<crate::health::OwnCounts>,
    own_health_unheard: bool,
    /// Each own login's weak-or-not, by id and revision (health::own_counts).
    own_weak: std::collections::HashMap<String, bool>,
    /// The desktop agent's tray was told of items to be reminded about, so
    /// signing out tells it there are none.
    told_agent: bool,
}

/// Importing from the browsers, when an organization keeps items out of
/// Personal (upstream's personal ownership policy), as the guided import says it.
#[cfg(not(phone))]
const KEEPS_ITEMS_OUT: &str = "your organization keeps items out of Personal, so PassBundle can't import your browsers' passwords there. Ask an admin for a shared folder you can add to, then export from your browser and import that file into the shared folder in the web vault";

/// How long a list that autofill or a passkey showed, having unlocked the
/// vault with "Immediately" chosen, waits to be picked from (`App::offered`).
pub const PICK_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

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
        let device = store.clone();
        App::open_in("", store, device, secrets)
    }

    /// One account of several (accounts.rs): `id` is its own on this device,
    /// `store` its folder, and `device` the app's, for what's the device's.
    pub fn open_in(id: &str, store: Store, device: Store, secrets: Box<dyn SecretStore>) -> App {
        let settings = store.settings();
        let server = settings
            .as_ref()
            .and_then(|s| Server::new(&s.server, Device::this_one(&s.device_identifier)).ok());
        let preferences = store.preferences();
        if let Some(dir) = store.dir() {
            crate::icons::forget_kept(dir);
        }
        App {
            id: id.to_owned(),
            store,
            device,
            install_identifier: None,
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
            uploads: Vec::new(),
            reprompted: None,
            grant: 0,
            autolock: AutoLock::new(Instant::now(), SystemTime::now()),
            history: None,
            generated: None,
            icons: crate::icons::Cache::default(),
            icon_session: 0,
            sign_in_requests: None,
            confirmed: None,
            beside: false,
            pick_by: None,
            own_health: None,
            own_health_unheard: false,
            own_weak: Default::default(),
            told_agent: false,
        }
    }

    /// An app for no account: signed out, with nothing kept and nowhere to
    /// keep anything (`Store::none`). What a call for an account that isn't
    /// on this device sees (accounts.rs).
    pub fn nobody() -> App {
        App::open_in(
            "",
            Store::none(),
            Store::none(),
            Box::new(crate::secrets::MemorySecretStore::default()),
        )
    }

    /// This account's own id on this device (accounts.rs).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The server and email signed in to (or being signed in to).
    pub fn account(&self) -> Option<(&str, &str)> {
        self.settings
            .as_ref()
            .map(|s| (s.server.as_str(), s.email.as_str()))
    }

    /// Opened beside the app by another process (iOS's AutoFill extension):
    /// it never syncs (`sync`).
    pub(crate) fn read_beside(&mut self) {
        self.beside = true;
    }

    /// The device identifier a sign-in uses when this account has none kept.
    pub fn use_install_identifier(&mut self, identifier: &str) {
        self.install_identifier = Some(identifier.to_owned());
    }

    /// Whether a sign-in waits for its second step (`finish_sign_in`), or
    /// for the new-device check (`new_device_status`).
    pub fn signing_in(&self) -> bool {
        self.pending.is_some()
    }

    /// Gives up on a sign-in waiting for its second step: the account is
    /// as it was kept (its server and email as on this device).
    pub fn cancel_sign_in(&mut self) {
        self.pending = None;
        self.settings = self.store.settings();
        self.server = self
            .settings
            .as_ref()
            .and_then(|s| Server::new(&s.server, Device::this_one(&s.device_identifier)).ok());
    }

    /// Whether this account has a vault kept on this device (it's signed
    /// in, locked or not).
    pub fn has_vault(&self) -> bool {
        self.store.has_sync()
    }

    /// The account's id on its server, from the open vault or the kept sync.
    pub fn user_id_kept(&self) -> Option<String> {
        self.user_id().ok()
    }

    /// Talks to the agent at `address`, which must be the program at `program` (tests).
    pub fn use_agent_at(&mut self, address: PathBuf, program: PathBuf) {
        self.agent = Some((address, program));
    }

    pub fn state(&self) -> State {
        match (&self.settings, &self.vault, self.store.has_sync()) {
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
        // A server address that can't be used, or that isn't a PassBundle
        // server's, is refused before the slow part.
        Server::new(server_url, Device::this_one(&Device::new_identifier()))?.check()?;
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

    /// One identifier per install, whichever account signs in: this
    /// account's, or the install's (accounts.rs), or a new one.
    fn device_identifier(&self) -> String {
        self.settings
            .as_ref()
            .map(|s| s.device_identifier.clone())
            .or_else(|| self.install_identifier.clone())
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
        let (kept_server, kept_email) = (
            server_url.trim().trim_end_matches('/').to_owned(),
            normalize_email(email),
        );
        // What this device was given when it last signed in to the account,
        // kept by its server and email whether or not the account is on
        // this device now: so it isn't new to the account again.
        let remembered = self
            .secrets
            .get_secret(Secret::DeviceToken, &kept_server, &kept_email);
        let signed_in = server.sign_in_remembered(prelogin, email, password, remembered)?;
        self.settings = Some(Settings {
            server: kept_server,
            email: kept_email,
            kdf: prelogin.kdf,
            kdf_iterations: prelogin.kdf_iterations,
            kdf_memory: prelogin.kdf_memory,
            kdf_parallelism: prelogin.kdf_parallelism,
            device_identifier: identifier,
        });
        self.server = Some(server);
        match signed_in {
            SignIn::Done(session) => self.signed_in(session),
            SignIn::NeedsTwoFactor(pending) | SignIn::NeedsApproval(pending) => {
                self.pending = Some(pending);
                self.waiting_step()
            }
        }
    }

    /// What a sign-in waits for now, as the server last said: two-step
    /// login or the new-device check. A device token the server answered
    /// with two-step login instead is forgotten, as upstream's apps do.
    fn waiting_step(&mut self) -> Result<Step> {
        let pending = self.pending.as_mut().ok_or(Error::SignedOut)?;
        if std::mem::take(&mut pending.forget_remembered) {
            if let Some(s) = &self.settings {
                self.secrets
                    .forget_secret(Secret::DeviceToken, &s.server, &s.email);
            }
        }
        Ok(match pending.check() {
            Some(check) => Step::NewDevice(check.clone()),
            None => Step::TwoFactor(pending.offer.clone()),
        })
    }

    /// Where a waiting sign-in got to after the server's answer.
    fn next_step(&mut self, next: Result<Next>) -> Result<Step> {
        match next? {
            Next::Done(session) => {
                self.pending = None;
                self.signed_in(session)
            }
            Next::TwoFactor | Next::NewDevice => self.waiting_step(),
        }
    }

    /// How the new-device check holding this sign-in stands now (asked
    /// every 3 seconds while it waits): waiting, denied, or ended, with what
    /// a code by email can do. Once it's approved, the sign-in finishes with
    /// the check's token, and this is `Done`.
    pub fn new_device_status(&mut self) -> Result<Step> {
        let (Some(server), Some(pending)) = (&self.server, &mut self.pending) else {
            return Err(Error::SignedOut);
        };
        server.new_device_status(pending)?;
        match pending.check().map(|c| c.status) {
            Some(new_device::Status::Approved) => {
                let token = pending
                    .check
                    .as_ref()
                    .map(|held| held.token.clone())
                    .ok_or(Error::SignedOut)?;
                let next = server.send_sign_in(pending, None, Some(token.as_str()));
                self.next_step(next)
            }
            Some(_) => self.waiting_step(),
            None => Err(Error::SignedOut),
        }
    }

    /// "Email me a code" (or, with devices that can approve, "Can't use
    /// your other devices?"), for the sign-in the new-device check holds.
    pub fn new_device_email(&mut self) -> Result<Step> {
        let (Some(server), Some(pending)) = (&self.server, &mut self.pending) else {
            return Err(Error::SignedOut);
        };
        server.new_device_email(pending)?;
        self.waiting_step()
    }

    /// Finishes the sign-in the new-device check holds with the code sent
    /// by email (8 digits, checked first). A wrong one can be tried again.
    pub fn finish_new_device_with_code(&mut self, code: &str) -> Result<Step> {
        let code = new_device::email_code(code)?;
        let (Some(server), Some(pending)) = (&self.server, &mut self.pending) else {
            return Err(Error::SignedOut);
        };
        if pending.check.is_none() {
            return Err(Error::SignedOut);
        }
        let next = server.send_sign_in(pending, None, Some(&code));
        self.next_step(next)
    }

    /// Two-step login's second step with a code: an authenticator app's, an
    /// emailed one, a YubiKey's, or a recovery code (two_step.rs). A wrong
    /// code can be tried again. Duo and a security key have their own
    /// (`finish_with_duo`, `finish_with_key`).
    pub fn finish_sign_in(&mut self, provider: u8, code: &str, remember: bool) -> Result<Step> {
        let code = match provider {
            two_step::AUTHENTICATOR | two_step::EMAIL => code.trim().to_owned(),
            two_step::YUBIKEY => two_step::yubikey_otp(code)?,
            two_step::RECOVERY_CODE => two_step::recovery_code(code)?,
            _ => {
                return Err(Error::TwoStep(
                    "PassBundle can't finish that kind of two-step login with a code.",
                ))
            }
        };
        let finished = self.finish_second_step(provider, code, remember);
        match (provider, finished) {
            (two_step::RECOVERY_CODE, Err(Error::WrongCode)) => Err(Error::TwoStep(
                "That recovery code isn't right. Check it, or use another way to sign in.",
            )),
            (two_step::YUBIKEY, Err(Error::WrongCode)) => Err(Error::TwoStep(
                "That YubiKey code didn't work. Touch your YubiKey again, or use another way to sign in.",
            )),
            (_, finished) => finished,
        }
    }

    fn finish_second_step(&mut self, provider: u8, code: String, remember: bool) -> Result<Step> {
        let (Some(server), Some(pending)) = (&self.server, &mut self.pending) else {
            return Err(Error::SignedOut);
        };
        let next = server.send_sign_in(
            pending,
            Some(&TwoFactor {
                provider,
                code,
                remember,
            }),
            None,
        )?;
        match next {
            // Two-step login again: the code wasn't right.
            Next::TwoFactor => Err(Error::WrongCode),
            next => self.next_step(Ok(next)),
        }
    }

    /// The master password hint, for the lock screen (`Server::password_hint`):
    /// the account kept here asks its server, with no session. None when the
    /// server emailed it to the account's email.
    pub fn password_hint(&self) -> Result<Option<String>> {
        let (Some(server), Some(settings)) = (&self.server, &self.settings) else {
            return Err(Error::SignedOut);
        };
        server.password_hint(&settings.email)
    }

    /// What the server offers now, new from it when what this sign-in has
    /// is spent or old (`fresh_for`). A sign-in that went through meanwhile
    /// (two-step login turned off elsewhere) finishes.
    fn fresh_offer(&mut self, fresh_for: Duration) -> Result<Option<Offer>> {
        let (Some(server), Some(pending)) = (&self.server, &mut self.pending) else {
            return Err(Error::SignedOut);
        };
        if pending.spent || pending.offered_at.elapsed() > fresh_for {
            if let Some(session) = server.renew_offer(pending)? {
                self.pending = None;
                self.signed_in(session)?;
                return Ok(None);
            }
        }
        Ok(self.pending.as_ref().map(|p| p.offer.clone()))
    }

    /// Duo's Universal Prompt address for this sign-in, for the browser:
    /// a new one unless the server made this one in the last minute (Duo
    /// gives a sign-in five minutes), or `SignedIn` if the sign-in went
    /// through without it.
    pub fn duo_address(&mut self) -> Result<Prepared<String>> {
        let Some(offer) = self.fresh_offer(Duration::from_secs(60))? else {
            return Ok(Prepared::SignedIn);
        };
        let duo = offer.duo.ok_or(Error::TwoStep(
            "The server doesn't offer Duo for this account now. Use another way to sign in.",
        ))?;
        Ok(Prepared::Ready(duo.url))
    }

    /// Two-step login's second step with Duo: its answer as the browser
    /// hands it back (two_step.rs, `Duo::token`).
    pub fn finish_with_duo(&mut self, link: &str, remember: bool) -> Result<Step> {
        let duo = self
            .pending
            .as_ref()
            .and_then(|p| p.offer.duo.clone())
            .ok_or(Error::TwoStep(
                "That Duo answer was for another sign-in. Continue with Duo again.",
            ))?;
        let token = duo.token(link)?;
        match self.finish_second_step(two_step::DUO, token, remember) {
            Err(Error::WrongCode) => Err(Error::TwoStep(
                "Duo didn't let this sign-in through. Continue with Duo to try again.",
            )),
            finished => finished,
        }
    }

    /// The security key request for this sign-in (security_key), with a new
    /// challenge if the last was tried or is a few minutes old, or
    /// `SignedIn` if the sign-in went through without it.
    pub fn security_key_request(&mut self) -> Result<Prepared<SecurityKeyRequest>> {
        let Some(offer) = self.fresh_offer(Duration::from_secs(240))? else {
            return Ok(Prepared::SignedIn);
        };
        let server = self.server.as_ref().ok_or(Error::SignedOut)?;
        let options = offer.security_key.ok_or(Error::TwoStep(
            "The server doesn't offer a security key for this account now. Use another way to sign in.",
        ))?;
        SecurityKeyRequest::new(&options, server.base()).map(Prepared::Ready)
    }

    /// Two-step login's second step with a security key's answer to
    /// `request`, checked first (security_key::webauthn).
    pub fn finish_with_key(
        &mut self,
        request: &SecurityKeyRequest,
        answer: &KeyAnswer,
        remember: bool,
    ) -> Result<Step> {
        let token = request.token(answer)?;
        match self.finish_second_step(two_step::SECURITY_KEY, token, remember) {
            Err(Error::WrongCode) => Err(Error::TwoStep(
                "The server didn't take the security key's answer. Try again, or use another way to sign in.",
            )),
            finished => finished,
        }
    }

    /// Asks the server to email the code, for a sign-in waiting on one.
    pub fn send_email_code(&self) -> Result<()> {
        let (Some(server), Some(pending)) = (&self.server, &self.pending) else {
            return Err(Error::SignedOut);
        };
        server.send_email_code(pending)
    }

    fn signed_in(&mut self, mut session: Session) -> Result<Step> {
        let settings = self.settings.as_ref().expect("set before signing in");
        // What the server gave this device to show next time: kept before
        // anything else, so a sign-in that stops part way still has it.
        if let Some(token) = session.device_token.take() {
            self.secrets.keep_secret(
                Secret::DeviceToken,
                &settings.server,
                &settings.email,
                &token,
            );
        }
        self.store.save_settings(settings)?;
        keep_refresh_token(&*self.secrets, &self.store, settings, &session);
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
        self.session = kept_refresh_token(&*self.secrets, &self.store, settings, vault.user_key())
            .map(|refresh_token| Session {
                access_token: Default::default(),
                refresh_token,
                expires_at: 0,
                user_key: vault.user_key().clone(),
                device_token: None,
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

    /// How long ago the vault kept here was synced, by its file's time.
    pub fn kept_age(&self) -> Option<std::time::Duration> {
        self.store.sync_age()
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
    /// the app's own Keystore key and the user key it sealed go instead, on
    /// iOS the Keychain item, and in the Mac app the user key sealed to the
    /// Secure Enclave (its Swift): this account's, by its id on this device.
    pub fn remove_biometric(&self) -> Result<()> {
        #[cfg(target_os = "android")]
        {
            crate::android::remove_biometric(&self.id);
            Ok(())
        }
        #[cfg(all(any(target_os = "ios", target_os = "macos"), feature = "ffi"))]
        {
            crate::ffi::remove_biometric(&self.id);
            Ok(())
        }
        #[cfg(not(any(
            target_os = "android",
            all(any(target_os = "ios", target_os = "macos"), feature = "ffi")
        )))]
        self.agent()?.remove()
    }

    /// Brings the vault up to date from the server and keeps the new sync.
    /// Signed out (the session ended) leaves the kept vault open, to read.
    /// Never beside the app (`beside`).
    pub fn sync(&mut self) -> Result<()> {
        if self.beside {
            return Err(Error::Refused(
                "PassBundle's app brings this account up to date",
            ));
        }
        // The account the vault kept here is (its id on the server).
        let kept = self.user_id().ok();
        // What this app started on the server and didn't finish is deleted
        // there first, so the vault that comes doesn't list it (files.rs).
        self.delete_unfinished();
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
                forget_refresh_token(&*self.secrets, &self.store, settings);
                return Err(Error::SignedOut);
            }
            other => other?,
        };
        if let Err(e) = same_account(kept.as_deref(), &sync.profile.id) {
            self.session = None;
            forget_refresh_token(&*self.secrets, &self.store, settings);
            return Err(e);
        }
        self.store.save_sync(&text)?;
        self.policy = LockPolicy::from_sync(&text);
        self.generator_policy = GeneratorPolicy::from_sync(&text);
        // The server may have issued a new refresh token.
        keep_refresh_token(&*self.secrets, &self.store, settings, session);
        self.vault = Some(Vault::open(&sync, session.user_key().clone())?);
        self.forget_deleted();
        // A server without them, or one that's gone for a moment, leaves the
        // generator's settings as they are here.
        let _ = self.sync_settings();
        self.report_own_health();
        Ok(())
    }

    /// Tells the server how many of the person's own logins have weak or
    /// reused passwords (health.rs, `own_counts`), numbers only, for their
    /// organizations' owners and admins: only while an organization they've
    /// joined has "Members' own passwords" on, which the server says
    /// (`own_health_seen_by`), once each unlock and again when the numbers
    /// change, and not again this unlock once a server says it doesn't take
    /// them. Never fails a sync.
    fn report_own_health(&mut self) {
        if self.own_health_unheard {
            return;
        }
        let (Some(server), Some(session), Some(vault)) =
            (&self.server, self.session.as_mut(), &self.vault)
        else {
            return;
        };
        if !vault.in_an_organization() {
            return;
        }
        let counts = crate::health::own_counts(vault, &mut self.own_weak);
        if self.own_health == Some(counts) {
            return;
        }
        match server.own_health_seen_by(session) {
            Ok(Some(seen_by)) if seen_by.is_empty() => return,
            Ok(Some(_)) => {}
            Ok(None) => {
                self.own_health_unheard = true;
                return;
            }
            // Tried again at the next sync.
            Err(_) => return,
        }
        match server.report_own_health(session, &counts) {
            Ok(Some(true)) => self.own_health = Some(counts),
            // Turned off since: nothing was kept, so it's sent again later.
            Ok(Some(false)) => {}
            Ok(None) => self.own_health_unheard = true,
            // Tried again at the next sync.
            Err(_) => {}
        }
    }

    /// What the person's own logins' numbers were last told the server as,
    /// this unlock (`report_own_health`).
    pub fn own_health_reported(&self) -> Option<crate::health::OwnCounts> {
        self.own_health
    }

    /// The expiration reminders due today (expiration.rs), each marked shown
    /// so it's said once, and every item at one of its steps whether shown
    /// before or not. On a computer, the desktop agent's tray is told those
    /// two numbers (and never which items), on a thread of its own.
    pub fn reminders(&mut self) -> Result<crate::expiration::Check> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let mut shown = self.store.shown_reminders();
        let found = crate::expiration::check(vault, &mut shown, crate::expiration::Date::today());
        self.store.save_shown_reminders(&shown)?;
        if found.standing.total() > 0 || self.told_agent {
            self.tell_agent(found.standing);
        }
        Ok(found)
    }

    /// Tells the desktop agent how many of this account's items to remind
    /// about, without waiting for it: an agent that doesn't know the command
    /// never answers. Phones have no agent.
    fn tell_agent(&mut self, counts: crate::expiration::Counts) {
        #[cfg(not(phone))]
        {
            let Ok(user_id) = self.user_id() else { return };
            let found = match &self.agent {
                Some(found) => Ok(found.clone()),
                None => Agent::installed(),
            };
            let Ok((address, program)) = found else {
                return;
            };
            self.told_agent = counts.total() > 0;
            std::thread::spawn(move || {
                if let Ok(mut agent) = Agent::connect(&address, &program, &user_id) {
                    let _ = agent.reminders(counts);
                }
            });
        }
        #[cfg(phone)]
        let _ = counts;
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
    /// Its screens are in front, so a wait for a pick (`offered`) is over.
    pub fn used(&mut self) {
        self.autolock.used(Instant::now(), SystemTime::now());
        self.pick_by = None;
    }

    /// Autofill or a passkey unlocked the vault while the app's screens
    /// weren't in front, and handed back a list to pick from (Android's
    /// "Unlock PassBundle", docs/phones.md, "Locking"). With "Immediately"
    /// chosen, or the policy's, the vault stays open for the pick only: it
    /// locks when the pick's fill or passkey is handed back (`left`), or after
    /// `PICK_WAIT` if nothing is picked.
    pub fn offered(&mut self) {
        self.offered_at(Instant::now(), SystemTime::now());
    }

    /// `offered`, at a given time (tests).
    pub fn offered_at(&mut self, now: Instant, wall: SystemTime) {
        if self.vault.is_some() && self.held().immediately() {
            self.pick_by = Some((now + PICK_WAIT, wall + PICK_WAIT));
        }
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
        let checked = self.autolock.check(&held, readings, now, wall);
        // "Immediately", and a list autofill or a passkey showed that nothing
        // was picked from in time (`offered`).
        let unpicked = held.immediately()
            && self
                .pick_by
                .is_some_and(|(by, by_wall)| now >= by || wall >= by_wall);
        let reason = checked.or(unpicked.then_some(Reason::Idle))?;
        self.time_up();
        Some(reason)
    }

    /// The app left the screen (a phone's app switcher or home, a computer's
    /// window closed or minimized, or on Android autofill or a passkey
    /// handing back its answer): locks if the person chose "Immediately" on
    /// a phone, or the policy says so. Returns whether it locked.
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
        self.own_health = None;
        self.own_health_unheard = false;
        self.own_weak.clear();
        self.pick_by = None;
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
        if self.store.has_sync() {
            let _ = self.remove_biometric();
        }
        // The agent's tray stops saying this account's items expire.
        if self.told_agent {
            self.tell_agent(crate::expiration::Counts::default());
        }
        if let Some(s) = &self.settings {
            forget_refresh_token(&*self.secrets, &self.store, s);
        }
        self.store.forget_vault();
        self.forget_uploads();
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

    /// Sign-ins on new devices waiting for this account's approval
    /// (new_device.rs), from the server, newest first, while unlocked, and
    /// who the locations are from. Listed beside the sign-in requests
    /// (`sign_in_requests`); never their numbers.
    pub fn new_device_checks(
        &mut self,
    ) -> Result<(Vec<new_device::Waiting>, Option<new_device::LocationSource>)> {
        let call = self.sign_in_requests_call()?;
        let fetched = call.fetch_new_devices();
        self.keep_new_device_checks(&call, fetched)
    }

    /// What `SignInRequestsCall::fetch_new_devices` listed, for the account
    /// it was asked for, if its vault is still the one open.
    pub fn keep_new_device_checks(
        &self,
        call: &SignInRequestsCall,
        fetched: Result<serde_json::Value>,
    ) -> Result<(Vec<new_device::Waiting>, Option<new_device::LocationSource>)> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        if vault.email != call.email {
            return Err(Error::SignedOut);
        }
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Ok(new_device::parse_waiting(&fetched?, now))
    }

    /// Approves another device's sign-in with the number the person typed,
    /// the one that device shows (two digits, checked before anything is
    /// sent), or denies it (`number` None). Approving sends no key, so it
    /// asks for no confirmation first: the number, shown only on the new
    /// device, is the proof that the person is at both. A wrong number
    /// denies it (`WrongNumber`): each takes one try.
    pub fn answer_new_device_check(
        &mut self,
        id: &str,
        number: Option<&str>,
    ) -> Result<new_device::Answered> {
        if let Some(typed) = number {
            new_device::typed_number(typed)?;
        }
        if self.vault.is_none() {
            return Err(Error::SignedOut);
        }
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.answer_new_device(session, id, number)
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
                forget_refresh_token(&*self.secrets, &self.store, settings);
                return Err(Error::SignedOut);
            }
            other => other?,
        };
        // Asking may have refreshed the session, and the server may have
        // issued a new refresh token, as for a sync.
        keep_refresh_token(&*self.secrets, &self.store, settings, session);
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
    ///
    /// Once the server has it, it's saved, whatever happens next: a sync
    /// that fails then (the connection dropped, say) doesn't make it a
    /// failure, or Save again would make a second one (`Saved::synced`).
    /// The item as the server saved it goes in the vault kept here, so it
    /// shows until a sync brings the rest.
    pub fn save(
        &mut self,
        id: Option<&str>,
        draft: &ItemDraft,
        collection_ids: &[String],
    ) -> Result<Saved> {
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
        let moved = editing.filter(|item| moves(item, collection_ids));
        // A move between shared folders is checked before anything is saved.
        if let Some(item) = moved {
            may_place(vault, item, collection_ids)?;
        }
        let moved = moved.is_some();
        let body = vault.encrypt_item(draft, editing)?;
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        let saved = server.save_item(session, id, body, collection_ids)?;
        // Then its shared folders, which the server changes on their own:
        // saved either way, so a refusal still leaves this device up to date.
        let placed = match (moved, id) {
            (true, Some(id)) => server.set_item_collections(session, id, collection_ids),
            _ => Ok(()),
        };
        let synced = self.sync();
        if synced.is_err() {
            // Shown as saved; a vault the answer doesn't open stays as it was.
            let _ = self.keep_saved("ciphers", &saved);
        }
        // The move's refusal, if any: the item is saved, but not moved.
        placed?;
        Ok(Saved {
            id: id
                .map(str::to_owned)
                .or_else(|| saved["id"].as_str().map(str::to_owned))
                .unwrap_or_default(),
            synced: synced.is_ok(),
        })
    }

    /// Puts an item or a folder as the server saved it (its answer to the
    /// save, which is as a sync has it) in the vault kept here (`list`:
    /// "ciphers" or "folders"), in place of the one it was or as a new one,
    /// and opens the vault again with it. For a save whose sync didn't come
    /// through; the next sync replaces the whole vault with the server's.
    /// Nothing is kept if the vault doesn't open with it.
    fn keep_saved(&mut self, list: &str, saved: &serde_json::Value) -> Result<()> {
        let id = saved["id"]
            .as_str()
            .ok_or(Error::Damaged("the server didn't say which item it saved"))?;
        let key = self
            .vault
            .as_ref()
            .ok_or(Error::SignedOut)?
            .user_key()
            .clone();
        let mut kept: serde_json::Value =
            serde_json::from_str(&self.store.sync().ok_or(Error::SignedOut)?)
                .map_err(|_| Error::Damaged("the vault kept here"))?;
        if kept[list].is_null() {
            kept[list] = serde_json::json!([]);
        }
        let listed = kept[list]
            .as_array_mut()
            .ok_or(Error::Damaged("the vault kept here"))?;
        match listed.iter_mut().find(|c| c["id"] == id) {
            Some(was) => *was = saved.clone(),
            None => listed.push(saved.clone()),
        }
        let text = kept.to_string();
        let vault = Vault::open(&Sync::from_json(&text)?, key)?;
        self.store.save_sync(&text)?;
        self.vault = Some(vault);
        Ok(())
    }

    /// Makes a folder, its name under the user key, as folders are
    /// (docs/core.md, "Items"), then syncs. Answers its id. Once the server
    /// has made it, it's made: a sync that fails then leaves it in the vault
    /// kept here (as `save` does), so trying again doesn't make another.
    pub fn create_folder(&mut self, name: &str) -> Result<String> {
        let name = Self::folder_name(name)?;
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let encrypted = vault.user_key().encrypt_str(&name).to_string();
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        let id = server.create_folder(session, &encrypted)?;
        if self.sync().is_err() {
            let made = serde_json::json!({ "id": id, "name": encrypted, "object": "folder" });
            let _ = self.keep_saved("folders", &made);
        }
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

    /// The profiles in the browsers on this computer, chosen by the rules
    /// for this account (browser_import.rs, DESIGN.md's "Which browser
    /// profiles"). Unlocked only; reads no passwords.
    #[cfg(not(phone))]
    pub fn browser_profiles(
        &self,
        home: &passbundle_browsers::Home,
    ) -> Result<crate::browser_import::Found> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        Ok(crate::browser_import::find(
            home,
            &vault.email,
            self.store.sync().as_deref(),
        ))
    }

    /// Imports the passwords saved in the browser profiles the person just
    /// chose (`chosen`, by id), and no others (browser_import.rs): each new
    /// login is encrypted here with this account's keys, as any new item
    /// is, and saved to Personal, at most `BATCH` to a request; the ones to
    /// compare go in their browser's "Check these" folder, made first if the
    /// vault doesn't have it. Then syncs. Firefox's Primary Passwords, by
    /// profile id, are used for this read only. Nothing is read when the
    /// account is locked, has no session, or an organization keeps its items
    /// out of Personal. A request the server refuses partway leaves the
    /// batches before it saved: importing again leaves those out.
    #[cfg(not(phone))]
    pub fn import_from_browsers(
        &mut self,
        home: &passbundle_browsers::Home,
        keys: &dyn passbundle_browsers::Keys,
        chosen: &[String],
        primary_passwords: &std::collections::HashMap<String, Zeroizing<String>>,
    ) -> Result<crate::browser_import::Imported> {
        use crate::browser_import::{self as import, Organizations, BATCH};
        if self.vault.is_none() || self.server.is_none() || self.session.is_none() {
            return Err(Error::SignedOut);
        }
        if Organizations::from_sync(self.store.sync().as_deref())
            .keeps_items_out
            .is_some()
        {
            return Err(Error::Refused(KEEPS_ITEMS_OUT));
        }
        let gathered = import::gather(home, keys, chosen, primary_passwords);
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let plan = import::plan(vault, gathered.logins);
        let check_browsers = plan.check_browsers();
        let check_folders: Vec<String> = check_browsers
            .iter()
            .map(|b| import::check_folder(*b))
            .collect();
        let mut kept: Vec<Option<String>> = check_folders
            .iter()
            .map(|name| {
                vault
                    .folders
                    .iter()
                    .find(|(_, n)| *n == name)
                    .map(|(id, _)| id.clone())
            })
            .collect();
        let names: Vec<String> = check_folders
            .iter()
            .map(|n| vault.user_key().encrypt_str(n).to_string())
            .collect();
        let ciphers = plan
            .new
            .iter()
            .map(|new| vault.encrypt_item(&new.draft, None))
            .collect::<Result<Vec<_>>>()?;
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        // The folders first, so every request puts its items in the same ones.
        for (id, name) in kept.iter_mut().zip(&names) {
            if id.is_none() && !ciphers.is_empty() {
                *id = Some(server.create_folder(session, name)?);
            }
        }
        let folders: Vec<(Option<String>, String)> = kept.into_iter().zip(names).collect();
        let mut sent = Ok(());
        for (n, batch) in ciphers.chunks(BATCH).enumerate() {
            let first = n * BATCH;
            let folder_of: Vec<(usize, usize)> = plan.new[first..first + batch.len()]
                .iter()
                .enumerate()
                .filter(|(_, new)| new.to_check)
                .filter_map(|(i, new)| {
                    check_browsers
                        .iter()
                        .position(|b| *b == new.browser)
                        .map(|folder| (i, folder))
                })
                .collect();
            sent = server.import_items(session, batch, &folders, &folder_of);
            if sent.is_err() {
                break;
            }
        }
        let synced = self.sync().is_ok();
        sent?;
        Ok(import::Imported {
            profiles: gathered.profiles,
            added: plan.new.len(),
            to_check: plan.to_check(),
            already_there: plan.already_there,
            empty: gathered.empty,
            never_save: gathered.never_save,
            unreadable: gathered.unreadable,
            check_folders,
            synced,
        })
    }

    /// Turns off the chosen browser profiles' own password saving, leaving
    /// what they saved (browser_import.rs). Unlocked only.
    #[cfg(not(phone))]
    pub fn turn_off_browser_saving(
        &self,
        home: &passbundle_browsers::Home,
        chosen: &[String],
    ) -> Result<
        Vec<(
            String,
            Option<passbundle_browsers::Profile>,
            crate::browser_import::TurnOff,
        )>,
    > {
        if self.vault.is_none() {
            return Err(Error::SignedOut);
        }
        Ok(crate::browser_import::turn_off(home, chosen))
    }

    /// Whether importing from this computer's browsers was offered to this
    /// account here already: once by itself, then from the menu.
    pub fn browser_import_offered(&self) -> bool {
        self.store.browser_import_offered()
    }

    pub fn set_browser_import_offered(&self) -> Result<()> {
        self.store.save_browser_import_offered()
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
        self.device.appearance()
    }

    /// Whether PassBundle's windows may be in screenshots, screen
    /// recordings, and screen sharing on this computer (`store::Screenshots`).
    pub fn screenshots(&self) -> crate::store::Screenshots {
        self.device.screenshots()
    }

    pub fn set_screenshots(&mut self, screenshots: crate::store::Screenshots) -> Result<()> {
        self.device.save_screenshots(&screenshots)
    }

    pub fn set_appearance(&mut self, appearance: crate::store::Appearance) -> Result<()> {
        self.device.save_appearance(&appearance)?;
        self.appearance_changed();
        Ok(())
    }

    /// How the app looks changed (here, or for another account on this
    /// device): with icons off, the ones kept go.
    pub fn appearance_changed(&mut self) {
        if !self.device.appearance().show_icons {
            self.forget_icons();
        }
    }

    /// What fetching a website's icon needs, away from the app (ffi.rs,
    /// `pb_icon`): the server's address, and which emptying of the icons
    /// it's for (`keep_icon`), while the vault is open and icons are on.
    pub fn icon_source(&self) -> Option<(String, u64)> {
        self.vault.as_ref()?;
        if !self.device.appearance().show_icons {
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

    /// The server's list of sign-ins on new devices waiting for approval
    /// (new_device.rs), with its clock in it (`serverDate`).
    pub fn fetch_new_devices(&self) -> Result<serde_json::Value> {
        self.server.waiting_new_devices(&self.access_token)
    }
}

/// Whether putting an organization's item in `collection_ids` (its shared
/// folders) changes where it is: they're given, and aren't the ones it's in.
fn moves(item: &Item, collection_ids: &[String]) -> bool {
    let mut held = item.collection_ids.clone();
    let mut asked = collection_ids.to_vec();
    held.sort();
    asked.sort();
    asked.dedup();
    item.organization_id.is_some() && !asked.is_empty() && held != asked
}

/// Whether the person may move an organization's item into `collection_ids`
/// from the shared folders it's in, checked as the server checks it
/// (change_item_folders): share in each folder it leaves, and create in each
/// it joins, with share on the item. Folders the person doesn't reach stay
/// as they are. The one check for a move, from the editor (`App::save`) or
/// for several items at once (`App::bulk`).
fn may_place(vault: &Vault, item: &Item, collection_ids: &[String]) -> Result<()> {
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
    Ok(())
}

/// Keeps the session's refresh token: in the OS's secret store, or where
/// that doesn't keep them (the Mac's: secrets.rs), sealed under a key from
/// the account's user key in the account's folder.
fn keep_refresh_token(
    secrets: &dyn SecretStore,
    store: &Store,
    settings: &Settings,
    session: &Session,
) {
    if secrets.keeps_refresh_tokens() {
        secrets.keep(&settings.server, &settings.email, &session.refresh_token);
    } else {
        let sealed = crate::secrets::seal_refresh_token(session.user_key(), &session.refresh_token);
        if let Err(e) = store.save_sealed_refresh_token(&sealed) {
            // Nothing to stop for: the session goes on, and at worst the
            // next unlock asks to sign in again. Said on standard error,
            // without the token, so a run from a terminal shows why.
            use std::io::Write;
            let why = match e {
                Error::Disk { full: true } => "this device is out of space",
                _ => "its folder couldn't be written",
            };
            let _ = writeln!(
                std::io::stderr(),
                "PassBundle: the session wasn't kept for next time ({why}), so the next unlock may ask to sign in again."
            );
        }
    }
}

/// The refresh token kept for the account, with its vault just opened by
/// `user_key`: a sealed one opens only with it.
fn kept_refresh_token(
    secrets: &dyn SecretStore,
    store: &Store,
    settings: &Settings,
    user_key: &SymmetricKey,
) -> Option<Zeroizing<String>> {
    if secrets.keeps_refresh_tokens() {
        secrets.get(&settings.server, &settings.email)
    } else {
        crate::secrets::open_refresh_token(user_key, &store.sealed_refresh_token()?)
    }
}

/// Forgets the account's refresh token, wherever it's kept.
fn forget_refresh_token(secrets: &dyn SecretStore, store: &Store, settings: &Settings) {
    secrets.forget(&settings.server, &settings.email);
    store.forget_sealed_refresh_token();
}

/// A sync is for the account whose vault is kept here, by its id on the
/// server, or for a new one (nothing kept yet): a server that answers with
/// another account's vault is refused, and nothing of it is kept. An
/// account's id never changes, then, once it's on this device, and two
/// accounts here never share one (accounts.rs refuses a new one that would):
/// the desktop agent keeps each account's biometric key by it (DESIGN.md in
/// the meta repo, "Several accounts in the apps").
fn same_account(kept: Option<&str>, synced: &str) -> Result<()> {
    match kept {
        Some(kept) if kept != synced => Err(Error::Refused(
            "the server sent another account's vault; sign out, then sign in again",
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod same_account_tests {
    use super::*;

    #[test]
    fn a_sync_is_for_the_account_kept_here_or_a_new_one() {
        assert_eq!(same_account(None, "a"), Ok(()));
        assert_eq!(same_account(Some("a"), "a"), Ok(()));
        assert!(matches!(
            same_account(Some("a"), "b"),
            Err(Error::Refused(_))
        ));
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

#[cfg(test)]
mod device_token_tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::secrets::MemorySecretStore;

    /// A stand-in server: prelogin, then each sign-in answered with the
    /// next of `answers` (status, JSON). Each sign-in's form, as sent.
    fn stand_in(answers: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let log = sent.clone();
        std::thread::spawn(move || {
            let mut answers = answers.into_iter();
            for mut request in server.incoming_requests() {
                let json = |status: u16, body: &str| {
                    tiny_http::Response::from_string(body)
                        .with_status_code(status)
                        .with_header(
                            tiny_http::Header::from_bytes("Content-Type", "application/json")
                                .unwrap(),
                        )
                };
                if request.url() == "/identity/accounts/prelogin" {
                    let prelogin = r#"{"kdf":0,"kdfIterations":600000}"#;
                    let _ = request.respond(json(200, prelogin));
                    continue;
                }
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                log.lock().unwrap().push(body);
                let (status, answer) = answers.next().unwrap_or((500, "{}"));
                let _ = request.respond(json(status, answer));
            }
        });
        (url, sent)
    }

    const TWO_STEP: &str =
        r#"{"error":"invalid_grant","TwoFactorProviders":["0"],"TwoFactorProviders2":{"0":null}}"#;
    const HELD: &str = r#"{"error":"invalid_grant","error_description":"new device verification required","KeykeeperNewDeviceCheck":{"id":"5f0c6f1e-6f3b-4a43-9a52-0d3f1c2b7e11","token":"AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_abcde","number":"47","status":"waiting","deniedWhy":null,"expiresAt":"2026-11-08T12:10:00.000000Z","approvers":true,"email":"later","emailAfter":null,"codeSent":false,"object":"keykeeperNewDeviceCheck"}}"#;

    #[test]
    fn the_kept_device_token_is_sent_and_forgotten_only_when_two_step_login_is_asked_for() {
        let (url, sent) = stand_in(vec![(400, HELD), (400, TWO_STEP)]);
        let email = "ana@example.test";
        let secrets = Arc::new(MemorySecretStore::default());
        // Kept from a sign-in before, by the server and email as typed:
        // the account isn't on this device now.
        assert!(secrets.keep_secret(Secret::DeviceToken, &url, email, "device-token-1"));
        let dir = std::env::temp_dir().join(format!("pb-device-token-{}", uuid::Uuid::new_v4()));
        let mut app = App::open(Store::new(&dir), Box::new(secrets.clone()));

        // Sent, and the server holds it anyway (a new device to it): the
        // number to show, and the token stays.
        let Step::NewDevice(check) = app
            .sign_in(
                &format!("{url}/"),
                " Ana@Example.test ",
                "correct horse battery",
            )
            .unwrap()
        else {
            panic!("the new-device check should hold it");
        };
        assert_eq!(check.number, "47");
        assert!(app.signing_in());
        assert!(secrets
            .get_secret(Secret::DeviceToken, &url, email)
            .is_some());

        // Two-step login instead: the token is no good, and goes.
        let step = app.sign_in(&url, email, "correct horse battery").unwrap();
        assert!(matches!(step, Step::TwoFactor(_)));
        assert!(secrets
            .get_secret(Secret::DeviceToken, &url, email)
            .is_none());

        let sent = sent.lock().unwrap();
        for form in sent.iter() {
            assert!(form.contains("twoFactorProvider=5"), "{form}");
            assert!(form.contains("twoFactorToken=device-token-1"), "{form}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_number_that_isnt_two_digits_is_refused_before_anything_is_sent() {
        let mut app = App::nobody();
        assert_eq!(
            app.answer_new_device_check("check-1", Some("4")),
            Err(Error::TwoStep(
                "Type the two-digit number shown on the new device."
            ))
        );
        // Two digits, but no vault open here: nothing to approve from.
        assert_eq!(
            app.answer_new_device_check("check-1", Some("47")),
            Err(Error::SignedOut)
        );
    }
}
