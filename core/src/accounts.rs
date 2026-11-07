//! Several accounts on one device (DESIGN.md in the meta repo, "Several
//! accounts in the apps"): up to five signed in at once, a work account and
//! a personal one, say. Each is an `App` of its own, with its own folder,
//! session, keys, choices, and lock, and nothing of one is reachable from
//! another: every call across the bridge (ffi.rs) names the account it's
//! for, and a call for an account that isn't here sees an app with nothing
//! in it (`App::nobody`).
//!
//! Kept in the app's folder:
//! - `accounts.json`: the accounts, in the order they were added, each
//!   with the name and color the person gave it (Work, Personal), the one
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

use std::collections::HashMap;
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

/// The colors an account can be shown in: the flat tile tones the apps draw
/// items' kinds in (app/lib/theme.dart), by name. A new account gets the
/// first one no other account here has.
pub const TONES: [&str; 10] = [
    "sapphire",
    "emerald",
    "amethyst",
    "topaz",
    "ruby",
    "turquoise",
    "garnet",
    "aquamarine",
    "onyx",
    "gold",
];

/// The longest name an account can be given, in characters.
pub const NAME_LONGEST: usize = 40;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tone: Option<String>,
}

/// What the person calls an account here, and its color: its own on this
/// device, never sent anywhere.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Label {
    /// Work, Personal: none until the person names it (the screens show
    /// its email then).
    pub name: Option<String>,
    /// One of `TONES`.
    pub tone: String,
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
    pub name: Option<String>,
    pub tone: String,
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
    /// Each account's name and color, by its id.
    labels: HashMap<String, Label>,
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
        let mut dropped = Vec::new();
        for id in ids {
            let store = Store::new(folders.join(&id));
            match store.settings() {
                Some(_) if store.has_sync() => kept.push(id),
                settings => {
                    dropped.extend(settings.map(|s| (s.server, s.email)));
                    remove_folder(&folders, &id);
                }
            }
        }
        // A refresh token a dropped folder's sign-in kept (its vault never
        // came down), unless an account that's kept is the same one.
        for (server, email) in dropped {
            let same = kept.iter().any(|id| {
                Store::new(folders.join(id))
                    .settings()
                    .is_some_and(|s| s.server == server && s.email == email)
            });
            if !same {
                secrets.forget(&server, &email);
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
            labels: HashMap::new(),
        };
        accounts.list = kept.iter().map(|id| accounts.app_for(id)).collect();
        // Each one's name and color as kept, a color of its own where it has
        // none (an install from before, a sign-in that finished as the app
        // stopped), and a name only as the person gave it.
        for id in &kept {
            let entry = index.accounts.iter().find(|e| &e.id == id);
            let name = entry
                .and_then(|e| e.name.clone())
                .and_then(|n| named(&n).ok().flatten());
            let tone = entry
                .and_then(|e| e.tone.clone())
                .filter(|t| TONES.contains(&t.as_str()));
            let tone = tone.unwrap_or_else(|| accounts.free_tone());
            accounts.labels.insert(id.clone(), Label { name, tone });
        }
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
                .map(|a| {
                    let label = self.labels.get(a.id());
                    Entry {
                        id: a.id().to_owned(),
                        name: label.and_then(|l| l.name.clone()),
                        tone: label.map(|l| l.tone.clone()),
                    }
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
                let label = self.labels.get(app.id()).cloned().unwrap_or_default();
                Summary {
                    id: app.id().to_owned(),
                    server: server.to_owned(),
                    email: email.to_owned(),
                    unlocked: app.vault().is_some(),
                    name: label.name,
                    tone: label.tone,
                }
            })
            .collect()
    }

    /// The first color no account here has (the first again when all ten
    /// are taken, which five accounts never do).
    fn free_tone(&self) -> String {
        TONES
            .iter()
            .find(|t| !self.labels.values().any(|l| l.tone == **t))
            .unwrap_or(&TONES[0])
            .to_string()
    }

    /// What an account is called here: the name the person gave it, else
    /// its email.
    pub fn label(&self, id: &str) -> String {
        self.labels
            .get(id)
            .and_then(|l| l.name.clone())
            .or_else(|| {
                self.get(id)
                    .and_then(App::account)
                    .map(|(_, e)| e.to_owned())
            })
            .unwrap_or_default()
    }

    /// Names an account here and picks its color: `name` trimmed (none to
    /// go back to its email), at most `NAME_LONGEST` characters; `tone` one of
    /// `TONES`, or none to keep its color. Kept on this device only.
    pub fn set_label(&mut self, id: &str, name: Option<&str>, tone: Option<&str>) -> Result<()> {
        if !self.list.iter().any(|a| a.id() == id) {
            return Err(Error::SignedOut);
        }
        let name = match name {
            Some(name) => named(name)?,
            None => None,
        };
        if tone.is_some_and(|t| !TONES.contains(&t)) {
            return Err(Error::Refused("that isn't one of the colors"));
        }
        let free = self.free_tone();
        let label = self.labels.entry(id.to_owned()).or_insert(Label {
            name: None,
            tone: free,
        });
        label.name = name;
        if let Some(tone) = tone {
            label.tone = tone.to_owned();
        }
        self.save();
        Ok(())
    }

    /// Each unlocked account's logins for an app or a website (autofill),
    /// with the account each is in: the one the screens show first, then
    /// the others in the order they were added.
    pub fn fillable<'a>(
        &'a self,
        target: &crate::matching::Target<'_>,
    ) -> Vec<(&'a str, &'a crate::vault::Item)> {
        self.unlocked_shown_first()
            .flat_map(|(id, vault)| vault.matching(target).into_iter().map(move |i| (id, i)))
            .collect()
    }

    /// Each unlocked account's logins with a passkey for a site (by its ID),
    /// with the account each is in, as `fillable` orders them.
    pub fn passkeys_for<'a>(&'a self, rp_id: &str) -> Vec<(&'a str, &'a crate::vault::Item)> {
        let mut found = Vec::new();
        for app in self.shown_first() {
            for item in app.passkeys_for(rp_id) {
                found.push((app.id(), item));
            }
        }
        found
    }

    /// What a search finds in every unlocked account, with the account
    /// each item is in and its vault: by name, as one account's list is,
    /// and for the same name, the one shown first.
    pub fn search_all<'a>(
        &'a self,
        query: &str,
    ) -> Vec<(&'a str, &'a Vault, &'a crate::vault::Item)> {
        let mut found: Vec<(&str, &Vault, &crate::vault::Item)> = self
            .unlocked_shown_first()
            .flat_map(|(id, vault)| vault.search(query).into_iter().map(move |i| (id, vault, i)))
            .collect();
        // Stable: within a name, the account order stays.
        found.sort_by_key(|(_, _, i)| i.name.to_lowercase());
        found
    }

    /// The accounts, the one shown first.
    fn shown_first(&self) -> impl Iterator<Item = &App> {
        let shown = self.active.as_deref();
        self.list
            .iter()
            .filter(move |a| Some(a.id()) == shown)
            .chain(self.list.iter().filter(move |a| Some(a.id()) != shown))
    }

    /// Each unlocked account's vault, the one shown first.
    fn unlocked_shown_first(&self) -> impl Iterator<Item = (&str, &Vault)> {
        self.shown_first()
            .filter_map(|a| a.vault().map(|v| (a.id(), v)))
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
            let tone = self.free_tone();
            self.labels.insert(id.clone(), Label { name: None, tone });
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
        self.labels.remove(&id);
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

/// A name for an account, as the person typed it: trimmed, none when
/// empty; refused when longer than `NAME_LONGEST` or holding a control
/// character.
fn named(name: &str) -> Result<Option<String>> {
    let name = name.trim();
    if name.chars().count() > NAME_LONGEST {
        return Err(Error::Refused("an account's name is at most 40 characters"));
    }
    if name.chars().any(char::is_control) {
        return Err(Error::Refused(
            "an account's name can't hold that character",
        ));
    }
    Ok((!name.is_empty()).then(|| name.to_owned()))
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
        index.accounts.push(Entry {
            id: id.clone(),
            name: None,
            tone: None,
        });
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
                "login": {
                    "username": enc(email),
                    "password": enc("not-a-real-password"),
                    // A site both accounts have a login for.
                    "uris": [{ "uri": enc("https://shared.example.test/sign-in"), "match": null }],
                },
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
        // And one whose vault never came down, though its session was kept.
        let unsynced = new_id();
        Store::new(dir.join("accounts").join(&unsynced))
            .save_settings(&settings(personal(), "the-install"))
            .unwrap();
        let secrets = Arc::new(MemorySecretStore::default());
        secrets.keep(personal().server, personal().email, "a refresh token");
        secrets.keep(work().server, work().email, "work's refresh token");
        // Something that isn't one of ours stays.
        fs::write(dir.join("accounts").join("theirs.txt"), "x").unwrap();
        let accounts = Accounts::open(&dir, secrets.clone());
        assert!(!dir.join("accounts").join(&unsynced).exists());
        assert!(secrets.get(personal().server, personal().email).is_none());
        assert!(secrets.get(work().server, work().email).is_some());
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
    fn names_and_colors_are_each_accounts_own_and_kept() {
        let dir = new_dir();
        let w = keep(&dir, work());
        let p = keep(&dir, personal());
        let mut accounts = open(&dir);
        // A color each, a different one; no name until one is given.
        let tones: Vec<String> = accounts.summaries().into_iter().map(|s| s.tone).collect();
        assert_eq!(tones, vec!["sapphire", "emerald"]);
        assert_eq!(accounts.label(&w), work().email);
        accounts
            .set_label(&w, Some("  Work "), Some("ruby"))
            .unwrap();
        let named = accounts.summaries();
        assert_eq!(named[0].name.as_deref(), Some("Work"));
        assert_eq!(named[0].tone, "ruby");
        assert_eq!(accounts.label(&w), "Work");
        assert_eq!(accounts.label(&p), personal().email);
        // Only a name a person would type, and only the colors there are.
        let long = "x".repeat(NAME_LONGEST + 1);
        assert!(matches!(
            accounts.set_label(&w, Some(&long), None),
            Err(Error::Refused(_))
        ));
        assert!(matches!(
            accounts.set_label(&w, Some("Wo\nrk"), None),
            Err(Error::Refused(_))
        ));
        assert!(matches!(
            accounts.set_label(&w, None, Some("plaid")),
            Err(Error::Refused(_))
        ));
        assert_eq!(
            accounts.set_label("0123456789abcdef0123456789abcdef", Some("Mine"), None),
            Err(Error::SignedOut)
        );
        assert_eq!(accounts.label(&w), "Work");
        accounts.set_label(&p, Some("Personal"), None).unwrap();
        // Kept on this device, for the next start.
        drop(accounts);
        let mut again = open(&dir);
        let kept: Vec<(Option<String>, String)> = again
            .summaries()
            .into_iter()
            .map(|s| (s.name, s.tone))
            .collect();
        assert_eq!(
            kept,
            vec![
                (Some("Work".into()), "ruby".into()),
                (Some("Personal".into()), "emerald".into())
            ]
        );
        // An empty name goes back to the email.
        again.set_label(&p, Some(" "), None).unwrap();
        assert_eq!(again.label(&p), personal().email);
        // Signing out forgets its name; a new account gets a free color.
        again.sign_out(&w);
        assert_eq!(again.free_tone(), "sapphire");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn every_unlocked_accounts_logins_fill_and_search_together_each_with_its_account() {
        use crate::matching::Target;
        let dir = new_dir();
        let w = keep(&dir, work());
        let p = keep(&dir, personal());
        let mut accounts = open(&dir);
        accounts.switch(&p).unwrap();
        accounts.app(&w).unwrap().unlock(work().password).unwrap();
        accounts
            .app(&p)
            .unwrap()
            .unlock(personal().password)
            .unwrap();
        let site = Target::Website("shared.example.test");
        // Both accounts' logins for the site, the shown one's first.
        let filled: Vec<(String, String)> = accounts
            .fillable(&site)
            .into_iter()
            .map(|(id, i)| (id.to_owned(), i.name.clone()))
            .collect();
        assert_eq!(
            filled,
            vec![
                (p.clone(), personal().login.to_owned()),
                (w.clone(), work().login.to_owned())
            ]
        );
        // Searching every account: by name, each with its account.
        let found: Vec<(String, String)> = accounts
            .search_all("")
            .into_iter()
            .map(|(id, _, i)| (id.to_owned(), i.name.clone()))
            .collect();
        assert_eq!(
            found,
            vec![
                (p.clone(), "Bank".to_owned()),
                (w.clone(), "Payroll".to_owned())
            ]
        );
        let paid: Vec<&str> = accounts
            .search_all("pay")
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(paid, vec![w.as_str()]);
        // A locked account offers nothing.
        accounts.lock(&w);
        let filled: Vec<&str> = accounts
            .fillable(&site)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(filled, vec![p.as_str()]);
        assert!(accounts.search_all("pay").is_empty());
        accounts.lock(&p);
        assert!(accounts.fillable(&site).is_empty());
        assert!(accounts.passkeys_for("shared.example.test").is_empty());
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
