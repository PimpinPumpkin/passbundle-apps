//! Several accounts on one device (DESIGN.md in the meta repo, "Several
//! accounts in the apps"): up to five signed in at once, a work account and
//! a personal one, say. Each is an `App` of its own, with its own folder,
//! session, keys, choices, and lock, and nothing of one is reachable from
//! another: every call across the bridge (ffi.rs) names the account it's
//! for, and a call for an account that isn't here sees an app with nothing
//! in it (`App::nobody`).
//!
//! Kept in the app's folder:
//! - `accounts.json`: the accounts, in the order they were added, the one
//!   the screens showed last, this install's device identifier, and the
//!   server and email last signed in to or out of (for the sign-in screen).
//!   Nothing secret.
//! - `accounts/<id>/`: each account's own store (store.rs): its server,
//!   email, KDF settings, kept sync, and choices. `<id>` is made on this
//!   device (32 hex digits) and says nothing about the account; a phone's
//!   biometric unlock keeps the account's key under it too.
//! - What's the device's (how the app looks, whether screenshots are
//!   allowed), and the OS secret store's refresh tokens, each named by its
//!   account's server and email (secrets.rs).
//!
//! An install from before several accounts kept its one account in the
//! app's folder itself; opening carries it over as the first account.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::time::{Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::app::{App, State, Step};
use crate::autolock::{Readings, Reason};
use crate::error::{Error, Result};
use crate::kdf::normalize_email;
use crate::secrets::SecretStore;
use crate::server::Device;
use crate::store::{self, Appearance, Screenshots, Store};
use crate::vault::Vault;

/// The most accounts signed in on one device, as the browser extension allows.
pub const MOST: usize = 5;

/// An account's own files, which an install from before several accounts
/// kept in the app's folder: carried over into the account's folder, with
/// `settings.json` last.
const ACCOUNT_FILES: [&str; 5] = [
    "sync.json",
    "preferences.json",
    "generator.json",
    "generator-synced.json",
    "approvals.json",
];

/// accounts.json.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Index {
    /// This install's device identifier, the same for every account, as
    /// upstream's apps have it: the server sees one device, and an account
    /// signed in again is the same device it was.
    device: Option<String>,
    accounts: Vec<Entry>,
    /// The account the screens showed last.
    active: Option<String>,
    last: Option<Last>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    id: String,
}

/// The server and email last signed in to or out of, for the sign-in screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Last {
    pub server: String,
    pub email: String,
}

/// One account, as the account menu shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub id: String,
    pub server: String,
    pub email: String,
    pub unlocked: bool,
}

pub struct Accounts {
    /// The app's folder.
    dir: PathBuf,
    /// What's the device's, in the app's folder.
    device: Store,
    secrets: Arc<dyn SecretStore>,
    /// The accounts signed in, in the order they were added.
    list: Vec<App>,
    /// The one the screens show.
    active: Option<String>,
    /// An account being signed in to that isn't here yet: it joins `list`
    /// once its vault has come down (`promote`). Made when first asked for,
    /// in a folder of its own that nothing is written to before then.
    adding: Option<App>,
    /// The account whose sign-in waits for its second step: one of `list`
    /// (signing in again), or "" for `adding`.
    signing_in: Option<String>,
    install: String,
    last: Option<Last>,
    /// The agent's address and program, when not the installed one (tests).
    agent: Option<(PathBuf, PathBuf)>,
}

/// An account's id on this device: 32 hex digits, made here.
fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Whether `id` is one this device makes, so a folder named after it is
/// one of ours, inside the accounts folder.
pub fn is_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The server and email as a sign-in keeps them (`App::sign_in_as`).
fn as_kept(server: &str, email: &str) -> (String, String) {
    (
        server.trim().trim_end_matches('/').to_owned(),
        normalize_email(email),
    )
}

impl Accounts {
    /// The accounts kept in the app's folder `dir`, all locked; an install
    /// from before several accounts carried over first.
    pub fn open(dir: &Path, secrets: Arc<dyn SecretStore>) -> Accounts {
        let device = Store::new(dir);
        let mut index: Index = device
            .read("accounts.json")
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        index.accounts.retain(|e| is_id(&e.id));
        carry_over(dir, &mut index);
        let folders = dir.join("accounts");
        // In the index's order, then any folder signed in to that it doesn't
        // list (a sign-in that finished as the app stopped). A folder with
        // no vault (one signed out of, or a sign-in given up) goes.
        let mut ids: Vec<String> = Vec::new();
        for entry in &index.accounts {
            if !ids.contains(&entry.id) {
                ids.push(entry.id.clone());
            }
        }
        let mut found: Vec<String> = fs::read_dir(&folders)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|name| is_id(name) && !ids.contains(name))
            .collect();
        found.sort();
        ids.extend(found);
        let mut kept = Vec::new();
        for id in ids {
            let store = Store::new(folders.join(&id));
            if store.settings().is_some() && store.has_sync() {
                kept.push(id);
            } else {
                remove_folder(&folders, &id);
            }
        }
        let install = index
            .device
            .clone()
            .or_else(|| {
                kept.iter().find_map(|id| {
                    Store::new(folders.join(id))
                        .settings()
                        .map(|s| s.device_identifier)
                })
            })
            .unwrap_or_else(Device::new_identifier);
        let active = index
            .active
            .clone()
            .filter(|a| kept.contains(a))
            .or_else(|| kept.first().cloned());
        let mut accounts = Accounts {
            dir: dir.to_owned(),
            device,
            secrets,
            list: Vec::new(),
            active,
            adding: None,
            signing_in: None,
            install,
            last: index.last.clone(),
            agent: None,
        };
        accounts.list = kept.iter().map(|id| accounts.app_for(id)).collect();
        accounts.save();
        accounts
    }

    fn folders(&self) -> PathBuf {
        self.dir.join("accounts")
    }

    /// An account's app, on its own folder.
    fn app_for(&self, id: &str) -> App {
        let mut app = App::open_in(
            id,
            Store::new(self.folders().join(id)),
            self.device.clone(),
            Box::new(self.secrets.clone()),
        );
        app.use_install_identifier(&self.install);
        if let Some((address, program)) = &self.agent {
            app.use_agent_at(address.clone(), program.clone());
        }
        app
    }

    /// Keeps accounts.json. A failure to write it leaves the folders, which
    /// are what count, as they are: the next start finds them again.
    fn save(&self) {
        let index = Index {
            device: Some(self.install.clone()),
            accounts: self
                .list
                .iter()
                .map(|a| Entry {
                    id: a.id().to_owned(),
                })
                .collect(),
            active: self.active.clone(),
            last: self.last.clone(),
        };
        let _ = self.device.write(
            "accounts.json",
            &serde_json::to_string_pretty(&index).expect("the index always serializes"),
        );
    }

    /// The accounts signed in, in the order they were added.
    pub fn summaries(&self) -> Vec<Summary> {
        self.list
            .iter()
            .map(|app| {
                let (server, email) = app.account().unwrap_or_default();
                Summary {
                    id: app.id().to_owned(),
                    server: server.to_owned(),
                    email: email.to_owned(),
                    unlocked: app.vault().is_some(),
                }
            })
            .collect()
    }

    /// The account the screens show, if any is signed in.
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// The server and email last signed in to or out of.
    pub fn last(&self) -> Option<&Last> {
        self.last.as_ref()
    }

    /// The screens show this account now (it's kept for the next start).
    /// Nothing of the one they showed changes: it stays unlocked or locked.
    pub fn switch(&mut self, id: &str) -> Result<()> {
        if !self.list.iter().any(|a| a.id() == id) {
            return Err(Error::SignedOut);
        }
        self.active = Some(id.to_owned());
        self.save();
        Ok(())
    }

    /// An account's app: one signed in, or for "" the account being signed
    /// in to (made when first asked for). None for any other id.
    pub fn app(&mut self, id: &str) -> Option<&mut App> {
        if id.is_empty() {
            if self.adding.is_none() {
                let adding = self.app_for(&new_id());
                self.adding = Some(adding);
            }
            return self.adding.as_mut();
        }
        self.list.iter_mut().find(|a| a.id() == id)
    }

    /// An account's app, if it's here: one signed in, or for "" the account
    /// being signed in to, if one is.
    pub fn get(&self, id: &str) -> Option<&App> {
        if id.is_empty() {
            return self.adding.as_ref();
        }
        self.list.iter().find(|a| a.id() == id)
    }

    /// The one the screens show.
    pub fn active_app(&mut self) -> Option<&mut App> {
        let id = self.active.clone()?;
        self.list.iter_mut().find(|a| a.id() == id)
    }

    /// An account's state; for "", signed out, with the server and email
    /// being signed in to, or last signed in to or out of.
    pub fn state(&self, id: &str) -> State {
        match self.get(id) {
            Some(app) if !id.is_empty() || app.account().is_some() => app.state(),
            _ if !id.is_empty() => State::SignedOut {
                server: None,
                email: None,
            },
            _ => State::SignedOut {
                server: self.last.as_ref().map(|l| l.server.clone()),
                email: self.last.as_ref().map(|l| l.email.clone()),
            },
        }
    }

    /// Which app a sign-in to this server and email goes to: the account's,
    /// if it's here (signing in again), or a new one's ("").
    fn route(&self, server: &str, email: &str) -> Result<String> {
        let (server, email) = as_kept(server, email);
        if let Some(app) = self
            .list
            .iter()
            .find(|a| a.account() == Some((server.as_str(), email.as_str())))
        {
            return Ok(app.id().to_owned());
        }
        if self.list.len() >= MOST {
            return Err(Error::TooManyAccounts);
        }
        Ok(String::new())
    }

    /// A sign-in that starts gives up any other waiting for its second step.
    fn give_up_others(&mut self, id: &str) {
        if let Some(other) = self.signing_in.take() {
            if other != id {
                if let Some(app) = self.app_kept(&other) {
                    app.cancel_sign_in();
                }
            }
        }
    }

    fn app_kept(&mut self, id: &str) -> Option<&mut App> {
        if id.is_empty() {
            return self.adding.as_mut();
        }
        self.list.iter_mut().find(|a| a.id() == id)
    }

    /// Signs in to an account: the vault comes down, is kept, and opens, and
    /// the screens show it. An account that's here already (the same server
    /// and email) is signed in to again in its own folder; any other is
    /// added, up to `MOST`.
    pub fn sign_in(&mut self, server: &str, email: &str, password: &str) -> Result<Step> {
        let id = self.route(server, email)?;
        self.give_up_others(&id);
        let signed_in = self
            .app(&id)
            .expect("routed to one")
            .sign_in(server, email, password);
        self.after(&id, signed_in)
    }

    /// Makes an account on the server, then signs in to it (`App::register`);
    /// it's added as a new one.
    pub fn register(
        &mut self,
        server: &str,
        account: &crate::signup::NewAccount,
        password: &str,
    ) -> Result<Step> {
        let id = self.route(server, &account.email)?;
        if !id.is_empty() {
            return Err(Error::AlreadyHere);
        }
        self.give_up_others(&id);
        let made = self
            .app(&id)
            .expect("routed to one")
            .register(server, account, password);
        self.after(&id, made)
    }

    /// Two-step login's second step, for the sign-in waiting for it.
    pub fn finish_sign_in(&mut self, provider: u8, code: &str, remember: bool) -> Result<Step> {
        let id = self.signing_in.clone().ok_or(Error::SignedOut)?;
        let finished = self
            .app_kept(&id)
            .ok_or(Error::SignedOut)?
            .finish_sign_in(provider, code, remember);
        self.after(&id, finished)
    }

    /// Asks the server to email the code, for the sign-in waiting for one.
    pub fn send_email_code(&mut self) -> Result<()> {
        let id = self.signing_in.clone().ok_or(Error::SignedOut)?;
        self.get(&id).ok_or(Error::SignedOut)?.send_email_code()
    }

    /// Where a sign-in got to.
    fn after(&mut self, id: &str, step: Result<Step>) -> Result<Step> {
        let waiting = self.get(id).is_some_and(App::signing_in);
        self.signing_in = waiting.then(|| id.to_owned());
        match step {
            Ok(Step::Done) => self.promote(id).map(|()| Step::Done),
            other => other,
        }
    }

    /// A sign-in is done: a new account joins the others, unless the server
    /// says it's one of them under another email or server (its id), and the
    /// screens show it.
    fn promote(&mut self, id: &str) -> Result<()> {
        let id = if id.is_empty() {
            let app = self.adding.take().expect("the account signed in to");
            let user = app.vault().map(|v| v.user_id.clone());
            if user.is_some() && self.list.iter().any(|a| a.user_id_kept() == user) {
                self.discard(app);
                return Err(Error::AlreadyHere);
            }
            let id = app.id().to_owned();
            self.list.push(app);
            id
        } else {
            id.to_owned()
        };
        if let Some((server, email)) = self.get(&id).and_then(App::account) {
            self.last = Some(Last {
                server: server.to_owned(),
                email: email.to_owned(),
            });
        }
        self.active = Some(id);
        self.save();
        Ok(())
    }

    /// Drops an account that never joined (`adding`): its refresh token and
    /// its folder. Never its biometric unlock: what the desktop agent keeps
    /// is by the account's id on the server, which may be another account's
    /// here (`promote`), and a new account has none on a phone.
    fn discard(&self, mut app: App) {
        app.lock();
        if let Some((server, email)) = app.account() {
            self.secrets.forget(server, email);
        }
        remove_folder(&self.folders(), app.id());
    }

    /// Gives up the sign-in waiting for its second step, and the account
    /// being added: the screens go back to the account they showed.
    pub fn cancel_sign_in(&mut self) {
        if let Some(id) = self.signing_in.take() {
            if let Some(app) = self.app_kept(&id) {
                app.cancel_sign_in();
            }
        }
        if let Some(app) = self.adding.take() {
            self.discard(app);
        }
    }

    /// Locks one account; the others stay as they are.
    pub fn lock(&mut self, id: &str) {
        if let Some(app) = self.app_kept(id) {
            app.lock();
        }
    }

    /// Signs out of one account: its vault, session, and biometric unlock
    /// are forgotten (`App::sign_out`), then its folder. The others stay as
    /// they are; the screens show the first of them, if there's one.
    pub fn sign_out(&mut self, id: &str) {
        if id.is_empty() {
            self.cancel_sign_in();
            return;
        }
        let Some(at) = self.list.iter().position(|a| a.id() == id) else {
            return;
        };
        self.list[at].sign_out();
        self.remove_at(at);
    }

    /// Takes an account that's signed out (by `sign_out`, or a policy's
    /// "sign out" when its time came) off this device.
    fn remove_at(&mut self, at: usize) {
        let mut app = self.list.remove(at);
        app.lock();
        if let Some((server, email)) = app.account() {
            self.last = Some(Last {
                server: server.to_owned(),
                email: email.to_owned(),
            });
        }
        let id = app.id().to_owned();
        remove_folder(&self.folders(), &id);
        if self.signing_in.as_deref() == Some(id.as_str()) {
            self.signing_in = None;
        }
        if self.active.as_deref() == Some(id.as_str()) {
            self.active = self.list.first().map(|a| a.id().to_owned());
        }
        self.save();
    }

    /// Takes off this device each account whose policy signed it out.
    fn sweep(&mut self) {
        while let Some(at) = self.list.iter().position(|a| !a.has_vault()) {
            self.remove_at(at);
        }
    }

    /// Locks each account whose time is up, by its own choices and policy
    /// (`App::tick`); answers which locked, and why.
    pub fn tick(&mut self, readings: &Readings) -> Vec<(String, Reason)> {
        let mut locked = Vec::new();
        for app in &mut self.list {
            if let Some(reason) = app.tick(readings) {
                locked.push((app.id().to_owned(), reason));
            }
        }
        self.sweep();
        locked
    }

    /// `tick`, at a given time (tests).
    #[cfg(test)]
    pub fn tick_at(&mut self, readings: &Readings, now: Instant, wall: SystemTime) -> Vec<String> {
        let mut locked = Vec::new();
        for app in &mut self.list {
            if app.tick_at(readings, now, wall).is_some() {
                locked.push(app.id().to_owned());
            }
        }
        self.sweep();
        locked
    }

    /// The app left the screen: each account locks if its own choice, or
    /// its policy, is "Immediately" (`App::left`). Whether any did.
    pub fn left(&mut self) -> bool {
        let mut any = false;
        for app in &mut self.list {
            any |= app.left();
        }
        self.sweep();
        any
    }

    /// The computer is about to sleep (or a phone's screen went off): each
    /// account locks if its own choice, or its policy, says to. Whether any did.
    pub fn sleeping(&mut self) -> bool {
        let mut any = false;
        for app in &mut self.list {
            any |= app.sleeping();
        }
        self.sweep();
        any
    }

    /// Autofill or a passkey unlocked an account away from the screens and
    /// showed a list to pick from (`App::offered`).
    pub fn offered(&mut self) {
        for app in &mut self.list {
            app.offered();
        }
    }

    /// The person used the app just now, in this account (its own wait
    /// starts again; the others keep theirs).
    pub fn used(&mut self, id: &str) {
        if let Some(app) = self.app_kept(id) {
            app.used();
        }
    }

    /// How the app looks on this device, for every account.
    pub fn appearance(&self) -> Appearance {
        self.device.appearance()
    }

    pub fn set_appearance(&mut self, appearance: Appearance) -> Result<()> {
        self.device.save_appearance(&appearance)?;
        for app in self.list.iter_mut().chain(self.adding.as_mut()) {
            app.appearance_changed();
        }
        Ok(())
    }

    /// Whether screenshots of PassBundle are allowed on this computer.
    pub fn screenshots(&self) -> Screenshots {
        self.device.screenshots()
    }

    pub fn set_screenshots(&mut self, screenshots: Screenshots) -> Result<()> {
        self.device.save_screenshots(&screenshots)
    }

    /// Every account talks to the agent at `address`, which must be the
    /// program at `program` (tests).
    pub fn use_agent_at(&mut self, address: PathBuf, program: PathBuf) {
        for app in self.list.iter_mut().chain(self.adding.as_mut()) {
            app.use_agent_at(address.clone(), program.clone());
        }
        self.agent = Some((address, program));
    }

    /// Each account's open vault, with its id: what's unlocked now.
    pub fn unlocked(&self) -> impl Iterator<Item = (&str, &Vault)> {
        self.list
            .iter()
            .filter_map(|a| a.vault().map(|v| (a.id(), v)))
    }
}

/// Removes an account's folder: only one named as this device names them.
fn remove_folder(folders: &Path, id: &str) {
    if is_id(id) {
        let _ = fs::remove_dir_all(folders.join(id));
    }
}

/// An install from before several accounts kept its one account in the
/// app's folder itself: it becomes an account like any other, in a folder
/// of its own, and the one the screens show. Only its own files move;
/// what's the device's stays. One that was signed out leaves its server and
/// email for the sign-in screen, and its device identifier.
///
/// Biometric unlock on a computer is the desktop agent's, by the account's
/// id on the server, so it carries over as it is. A phone's was kept for
/// the one account; the phone's own code removes it, and the app offers it
/// again after the next unlock with the master password.
fn carry_over(dir: &Path, index: &mut Index) {
    let legacy = Store::new(dir);
    let Some(settings) = legacy.settings() else {
        return;
    };
    if index.device.is_none() {
        index.device = Some(settings.device_identifier.clone());
    }
    let folders = dir.join("accounts");
    if !legacy.has_sync() {
        index.last = Some(Last {
            server: settings.server,
            email: settings.email,
        });
        for name in ACCOUNT_FILES.iter().chain(&["settings.json"]) {
            let _ = fs::remove_file(dir.join(name));
        }
        return;
    }
    // The same account in a folder already (only tests put one back), or a
    // new folder.
    let id = index
        .accounts
        .iter()
        .map(|e| e.id.clone())
        .find(|id| {
            Store::new(folders.join(id))
                .settings()
                .is_some_and(|s| s.server == settings.server && s.email == settings.email)
        })
        .unwrap_or_else(new_id);
    let to = folders.join(&id);
    if store::private_dir(&folders)
        .and_then(|()| store::private_dir(&to))
        .is_err()
    {
        return;
    }
    for name in ACCOUNT_FILES {
        let from = dir.join(name);
        if from.is_file() {
            let _ = fs::rename(&from, to.join(name));
        }
    }
    if fs::rename(dir.join("settings.json"), to.join("settings.json")).is_err() {
        return;
    }
    if !index.accounts.iter().any(|e| e.id == id) {
        index.accounts.push(Entry { id: id.clone() });
    }
    index.active = Some(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autolock::Preferences;
    use crate::crypto::{PrivateKey, SymmetricKey};
    use crate::kdf::{Kdf, MasterKey};
    use crate::secrets::MemorySecretStore;
    use crate::store::Settings;
    use std::sync::OnceLock;
    use std::time::Duration;

    /// A vault of its own, as a server would send it: its own user key,
    /// private key, and master password, and one login.
    struct Made {
        email: &'static str,
        password: &'static str,
        server: &'static str,
        user_id: String,
        sync: String,
        login: &'static str,
    }

    fn make(
        email: &'static str,
        password: &'static str,
        server: &'static str,
        login: &'static str,
    ) -> Made {
        let master = MasterKey::derive(
            password,
            email,
            &Kdf::Pbkdf2 {
                iterations: 600_000,
            },
        )
        .unwrap();
        let user = SymmetricKey::generate();
        let private = PrivateKey::generate().unwrap();
        let user_id = uuid::Uuid::new_v4().to_string();
        let enc = |text: &str| user.encrypt_str(text).to_string();
        let sync = serde_json::json!({
            "object": "sync",
            "profile": {
                "id": user_id,
                "email": email,
                "key": master.stretch().encrypt(&user.to_bytes()).to_string(),
                "privateKey": user.encrypt(&private.to_pkcs8_der()).to_string(),
                "organizations": [],
            },
            "folders": [],
            "collections": [],
            "ciphers": [{
                "id": uuid::Uuid::new_v4().to_string(),
                "organizationId": null,
                "folderId": null,
                "type": 1,
                "key": null,
                "favorite": false,
                "reprompt": 0,
                "revisionDate": "2026-11-06T12:00:00.000Z",
                "creationDate": "2026-11-06T12:00:00.000Z",
                "deletedDate": null,
                "collectionIds": [],
                "name": enc(login),
                "notes": null,
                "login": { "username": enc(email), "password": enc("not-a-real-password"), "uris": [] },
            }],
            "policies": [],
        })
        .to_string();
        Made {
            email,
            password,
            server,
            user_id,
            sync,
            login,
        }
    }

    /// A work account and a personal one, on servers of their own: made
    /// once (the KDF and the RSA key take a moment).
    fn work() -> &'static Made {
        static MADE: OnceLock<Made> = OnceLock::new();
        MADE.get_or_init(|| {
            make(
                "ana@work.example.test",
                "work password 1",
                "https://vault.work.example.test",
                "Payroll",
            )
        })
    }

    fn personal() -> &'static Made {
        static MADE: OnceLock<Made> = OnceLock::new();
        MADE.get_or_init(|| {
            make(
                "ana@home.example.test",
                "personal password 1",
                "https://vault.passbundle.example.test",
                "Bank",
            )
        })
    }

    fn settings(made: &Made, device: &str) -> Settings {
        Settings {
            server: made.server.into(),
            email: made.email.into(),
            kdf: 0,
            kdf_iterations: 600_000,
            kdf_memory: None,
            kdf_parallelism: None,
            device_identifier: device.into(),
        }
    }

    fn new_dir() -> PathBuf {
        std::env::temp_dir().join(format!("pb-accounts-{}", uuid::Uuid::new_v4().simple()))
    }

    /// An account kept in its own folder, as a sign-in leaves it.
    fn keep(dir: &Path, made: &Made) -> String {
        let id = new_id();
        let store = Store::new(dir.join("accounts").join(&id));
        store.save_settings(&settings(made, "the-install")).unwrap();
        store.save_sync(&made.sync).unwrap();
        id
    }

    fn open(dir: &Path) -> Accounts {
        Accounts::open(dir, Arc::new(MemorySecretStore::default()))
    }

    fn names(accounts: &mut Accounts, id: &str) -> Vec<String> {
        accounts
            .app(id)
            .and_then(|a| a.vault())
            .map(|v| v.items.iter().map(|i| i.name.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn two_accounts_unlock_lock_and_see_only_their_own() {
        let dir = new_dir();
        let w = keep(&dir, work());
        let p = keep(&dir, personal());
        let mut accounts = open(&dir);
        let ids: Vec<String> = accounts.summaries().into_iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&w) && ids.contains(&p));
        // All locked at start.
        assert!(accounts.summaries().iter().all(|s| !s.unlocked));

        // Each opens with its own master password only.
        assert_eq!(
            accounts.app(&w).unwrap().unlock(personal().password),
            Err(Error::WrongPassword)
        );
        accounts.app(&w).unwrap().unlock(work().password).unwrap();
        assert!(matches!(accounts.state(&w), State::Unlocked { .. }));
        assert!(matches!(accounts.state(&p), State::Locked { .. }));
        assert_eq!(names(&mut accounts, &w), vec![work().login]);
        assert!(names(&mut accounts, &p).is_empty());

        accounts
            .app(&p)
            .unwrap()
            .unlock(personal().password)
            .unwrap();
        assert_eq!(names(&mut accounts, &p), vec![personal().login]);
        assert_eq!(names(&mut accounts, &w), vec![work().login]);
        // Each its own keys.
        let key = |accounts: &mut Accounts, id: &str| {
            accounts
                .app(id)
                .unwrap()
                .vault()
                .unwrap()
                .user_key()
                .to_bytes()
                .to_vec()
        };
        assert_ne!(key(&mut accounts, &w), key(&mut accounts, &p));
        // One account's user key doesn't open the other's vault.
        let work_key = SymmetricKey::from_bytes(&key(&mut accounts, &w)).unwrap();
        accounts.lock(&p);
        assert!(accounts.app(&p).unwrap().unlock_with_key(work_key).is_err());
        assert!(matches!(accounts.state(&p), State::Locked { .. }));

        // Locking one leaves the other open.
        accounts
            .app(&p)
            .unwrap()
            .unlock(personal().password)
            .unwrap();
        accounts.lock(&w);
        assert!(matches!(accounts.state(&w), State::Locked { .. }));
        assert!(matches!(accounts.state(&p), State::Unlocked { .. }));
        assert!(names(&mut accounts, &w).is_empty());
        assert_eq!(names(&mut accounts, &p), vec![personal().login]);
        let unlocked: Vec<&str> = accounts.unlocked().map(|(id, _)| id).collect();
        assert_eq!(unlocked, vec![p.as_str()]);

        // An id that isn't here reaches nothing.
        assert!(accounts.app("0123456789abcdef0123456789abcdef").is_none());
        assert!(accounts.get("not an id").is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn each_keeps_its_own_choices_and_locks_on_its_own_time() {
        let dir = new_dir();
        let w = keep(&dir, work());
        let p = keep(&dir, personal());
        let mut accounts = open(&dir);
        // Work locks after a minute, Personal after 15, and neither on sleep.
        let one = Preferences {
            lock_after_minutes: Some(1),
            lock_on_sleep: false,
        };
        let fifteen = Preferences {
            lock_after_minutes: Some(15),
            lock_on_sleep: true,
        };
        accounts.app(&w).unwrap().set_preferences(one).unwrap();
        accounts.app(&p).unwrap().set_preferences(fifteen).unwrap();
        // Kept in each account's own folder.
        assert_eq!(Store::new(dir.join("accounts").join(&w)).preferences(), one);
        assert_eq!(
            Store::new(dir.join("accounts").join(&p)).preferences(),
            fifteen
        );
        assert!(!dir.join("preferences.json").exists());

        accounts.app(&w).unwrap().unlock(work().password).unwrap();
        accounts
            .app(&p)
            .unwrap()
            .unlock(personal().password)
            .unwrap();
        let start = Instant::now();
        let wall = SystemTime::now();
        let idle = |secs: u64| Readings {
            idle: Some(Duration::from_secs(secs)),
            ..Readings::default()
        };
        // Two minutes idle: Work's minute is up, Personal's 15 aren't.
        let locked = accounts.tick_at(
            &idle(120),
            start + Duration::from_secs(120),
            wall + Duration::from_secs(120),
        );
        assert_eq!(locked, vec![w.clone()]);
        assert!(matches!(accounts.state(&w), State::Locked { .. }));
        assert!(matches!(accounts.state(&p), State::Unlocked { .. }));
        // The computer sleeping locks Personal, whose choice that is.
        assert!(accounts.sleeping());
        assert!(matches!(accounts.state(&p), State::Locked { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn signing_out_of_one_leaves_the_others() {
        let dir = new_dir();
        let w = keep(&dir, work());
        let p = keep(&dir, personal());
        let mut accounts = open(&dir);
        accounts.switch(&w).unwrap();
        accounts
            .app(&p)
            .unwrap()
            .unlock(personal().password)
            .unwrap();
        accounts.app(&w).unwrap().unlock(work().password).unwrap();
        accounts.sign_out(&w);
        // Gone from this device: its folder, and from the list.
        assert!(!dir.join("accounts").join(&w).exists());
        let left: Vec<String> = accounts.summaries().into_iter().map(|s| s.id).collect();
        assert_eq!(left, vec![p.clone()]);
        assert!(accounts.app(&w).is_none());
        // The other is as it was, and the screens show it.
        assert!(matches!(accounts.state(&p), State::Unlocked { .. }));
        assert_eq!(accounts.active(), Some(p.as_str()));
        assert_eq!(names(&mut accounts, &p), vec![personal().login]);
        // The sign-in screen offers the one signed out of.
        assert_eq!(
            accounts.last(),
            Some(&Last {
                server: work().server.into(),
                email: work().email.into()
            })
        );
        // And the next start finds only the one.
        drop(accounts);
        let mut again = open(&dir);
        assert_eq!(again.summaries().len(), 1);
        assert_eq!(again.active(), Some(p.as_str()));
        again.sign_out(&p);
        assert!(again.summaries().is_empty());
        assert_eq!(again.active(), None);
        assert!(matches!(
            again.state(""),
            State::SignedOut { email: Some(e), .. } if e == personal().email
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_policy_that_signs_out_takes_only_its_own_account_off() {
        let dir = new_dir();
        let w = keep(&dir, work());
        let p = keep(&dir, personal());
        // Work's organization signs out after a minute.
        let mut sync: serde_json::Value = serde_json::from_str(&work().sync).unwrap();
        sync["profile"]["organizations"] = serde_json::json!([{
            "id": "11111111-2222-4333-8444-555555555555", "name": "Work", "key": null,
            "status": 2, "type": 2, "enabled": true, "usePolicies": true,
        }]);
        sync["policies"] = serde_json::json!([{
            "id": "p", "organizationId": "11111111-2222-4333-8444-555555555555", "type": 9,
            "enabled": true, "data": { "minutes": 1, "action": "logOut", "type": "custom" },
        }]);
        Store::new(dir.join("accounts").join(&w))
            .save_sync(&sync.to_string())
            .unwrap();
        let mut accounts = open(&dir);
        accounts.app(&w).unwrap().unlock(work().password).unwrap();
        accounts
            .app(&p)
            .unwrap()
            .unlock(personal().password)
            .unwrap();
        assert!(accounts.app(&w).unwrap().lock_policy().sign_out);
        let later = Duration::from_secs(120);
        let idle = Readings {
            idle: Some(later),
            ..Readings::default()
        };
        let locked = accounts.tick_at(&idle, Instant::now() + later, SystemTime::now() + later);
        assert_eq!(locked, vec![w.clone()]);
        let left: Vec<String> = accounts.summaries().into_iter().map(|s| s.id).collect();
        assert_eq!(left, vec![p.clone()]);
        assert!(!dir.join("accounts").join(&w).exists());
        assert!(matches!(accounts.state(&p), State::Unlocked { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn carries_over_an_install_from_before_several_accounts() {
        let dir = new_dir();
        // As an earlier build kept its one account: in the app's folder.
        let legacy = Store::new(&dir);
        legacy
            .save_settings(&settings(work(), "kept-device"))
            .unwrap();
        legacy.save_sync(&work().sync).unwrap();
        let chosen = Preferences {
            lock_after_minutes: Some(5),
            lock_on_sleep: false,
        };
        legacy.save_preferences(&chosen).unwrap();
        legacy
            .save_approvals(&crate::store::Approvals { confirm: false })
            .unwrap();
        // The device's own choices.
        legacy
            .save_screenshots(&Screenshots { allow: true })
            .unwrap();
        legacy
            .save_appearance(&Appearance { show_icons: false })
            .unwrap();

        let mut accounts = open(&dir);
        let summaries = accounts.summaries();
        assert_eq!(summaries.len(), 1);
        let id = summaries[0].id.clone();
        assert!(is_id(&id));
        assert_eq!(summaries[0].email, work().email);
        assert_eq!(accounts.active(), Some(id.as_str()));
        // Its files moved into its own folder; the device's stayed.
        let own = dir.join("accounts").join(&id);
        for name in [
            "settings.json",
            "sync.json",
            "preferences.json",
            "approvals.json",
        ] {
            assert!(own.join(name).is_file(), "{name} moved");
            assert!(!dir.join(name).exists(), "{name} isn't left behind");
        }
        assert!(dir.join("screenshots.json").is_file());
        assert!(dir.join("appearance.json").is_file());
        assert_eq!(accounts.screenshots(), Screenshots { allow: true });
        assert_eq!(accounts.appearance(), Appearance { show_icons: false });
        // The same account: its choices, device identifier, and vault.
        let app = accounts.app(&id).unwrap();
        assert_eq!(app.preferences(), chosen);
        assert!(!app.confirm_approvals());
        app.unlock(work().password).unwrap();
        assert_eq!(names(&mut accounts, &id), vec![work().login]);
        assert_eq!(
            Store::new(&own).settings().unwrap().device_identifier,
            "kept-device"
        );
        // Done once: the next start opens the same account.
        drop(accounts);
        let again = open(&dir);
        assert_eq!(
            again
                .summaries()
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            vec![id.clone()]
        );
        assert_eq!(again.install, "kept-device");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn carries_over_a_signed_out_install_as_the_sign_in_screens_server_and_email() {
        let dir = new_dir();
        let legacy = Store::new(&dir);
        legacy
            .save_settings(&settings(work(), "kept-device"))
            .unwrap();
        legacy.save_preferences(&Preferences::default()).unwrap();
        let accounts = open(&dir);
        assert!(accounts.summaries().is_empty());
        assert_eq!(accounts.active(), None);
        assert_eq!(accounts.install, "kept-device");
        assert!(matches!(
            accounts.state(""),
            State::SignedOut { server: Some(s), email: Some(e) } if s == work().server && e == work().email
        ));
        assert!(!dir.join("settings.json").exists());
        assert!(!dir.join("preferences.json").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_a_sign_in_that_finished_as_the_app_stopped_and_drops_what_was_given_up() {
        let dir = new_dir();
        let w = keep(&dir, work());
        // A sign-in given up: a folder with choices but no vault.
        let given_up = new_id();
        Store::new(dir.join("accounts").join(&given_up))
            .save_preferences(&Preferences::default())
            .unwrap();
        // Something that isn't one of ours stays.
        fs::write(dir.join("accounts").join("theirs.txt"), "x").unwrap();
        let accounts = open(&dir);
        // Not in accounts.json (there's none), but signed in: kept.
        assert_eq!(
            accounts
                .summaries()
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            vec![w.clone()]
        );
        assert!(!dir.join("accounts").join(&given_up).exists());
        assert!(dir.join("accounts").join("theirs.txt").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn switching_shows_another_without_changing_either() {
        let dir = new_dir();
        let w = keep(&dir, work());
        let p = keep(&dir, personal());
        let mut accounts = open(&dir);
        accounts.app(&w).unwrap().unlock(work().password).unwrap();
        accounts.switch(&p).unwrap();
        assert_eq!(accounts.active(), Some(p.as_str()));
        assert!(matches!(accounts.state(&w), State::Unlocked { .. }));
        assert!(matches!(accounts.state(&p), State::Locked { .. }));
        assert_eq!(
            accounts.switch("0123456789abcdef0123456789abcdef"),
            Err(Error::SignedOut)
        );
        assert_eq!(accounts.active(), Some(p.as_str()));
        drop(accounts);
        // Kept for the next start.
        assert_eq!(open(&dir).active(), Some(p.as_str()));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_sign_in_is_refused_at_the_most_and_routed_to_one_already_here() {
        let dir = new_dir();
        let w = keep(&dir, work());
        let accounts = open(&dir);
        // The same server and email (as typed, any case, with a slash): that account.
        assert_eq!(
            accounts
                .route(
                    &format!("  {}/ ", work().server),
                    &work().email.to_uppercase()
                )
                .unwrap(),
            w
        );
        assert_eq!(
            accounts
                .route(work().server, "someone@else.example.test")
                .unwrap(),
            ""
        );
        drop(accounts);
        for _ in 1..MOST {
            keep(&dir, personal());
        }
        let accounts = open(&dir);
        assert_eq!(accounts.summaries().len(), MOST);
        assert_eq!(
            accounts.route(work().server, "someone@else.example.test"),
            Err(Error::TooManyAccounts)
        );
        // One that's here can still sign in again.
        assert_eq!(accounts.route(work().server, work().email).unwrap(), w);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_account_the_server_says_is_one_already_here_is_dropped() {
        let dir = new_dir();
        let w = keep(&dir, work());
        let mut accounts = open(&dir);
        // As if signing in to the same account under another email had
        // brought down its vault: it doesn't join, and leaves nothing.
        let adding = accounts.app("").unwrap();
        let adding_id = adding.id().to_owned();
        let store = Store::new(dir.join("accounts").join(&adding_id));
        let mut other = settings(work(), "the-install");
        other.email = "renamed@work.example.test".into();
        store.save_settings(&other).unwrap();
        store.save_sync(&work().sync).unwrap();
        accounts.adding = Some(accounts.app_for(&adding_id));
        accounts
            .adding
            .as_mut()
            .unwrap()
            .unlock(work().password)
            .unwrap();
        assert_eq!(accounts.promote(""), Err(Error::AlreadyHere));
        assert_eq!(
            accounts
                .summaries()
                .into_iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            vec![w]
        );
        assert!(!dir.join("accounts").join(&adding_id).exists());
        assert!(accounts.adding.is_none());
        let _ = work().user_id.len();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn giving_up_on_adding_an_account_leaves_nothing() {
        let dir = new_dir();
        let w = keep(&dir, work());
        let mut accounts = open(&dir);
        accounts.switch(&w).unwrap();
        // The sign-in screen's own choices go in the new account's folder.
        let adding = accounts.app("").unwrap();
        let adding_id = adding.id().to_owned();
        adding.set_preferences(Preferences::default()).unwrap();
        assert!(dir.join("accounts").join(&adding_id).exists());
        accounts.cancel_sign_in();
        assert!(!dir.join("accounts").join(&adding_id).exists());
        assert_eq!(accounts.active(), Some(w.as_str()));
        assert_eq!(accounts.summaries().len(), 1);
        // Asked again, it's a new one.
        assert_ne!(accounts.app("").unwrap().id(), adding_id);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_devices_own_choices_are_every_accounts() {
        let dir = new_dir();
        let w = keep(&dir, work());
        let p = keep(&dir, personal());
        let mut accounts = open(&dir);
        accounts
            .set_appearance(Appearance { show_icons: false })
            .unwrap();
        assert_eq!(
            accounts.app(&w).unwrap().appearance(),
            Appearance { show_icons: false }
        );
        assert_eq!(
            accounts.app(&p).unwrap().appearance(),
            Appearance { show_icons: false }
        );
        accounts
            .app(&w)
            .unwrap()
            .set_screenshots(Screenshots { allow: true })
            .unwrap();
        assert_eq!(accounts.screenshots(), Screenshots { allow: true });
        assert!(dir.join("appearance.json").is_file());
        assert!(!dir
            .join("accounts")
            .join(&w)
            .join("appearance.json")
            .exists());
        let _ = fs::remove_dir_all(dir);
    }
}
