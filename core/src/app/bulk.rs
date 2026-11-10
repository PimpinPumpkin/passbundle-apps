//! One thing done to several items at once (docs/core.md, "Selecting
//! several items"): favorites, the trash, a folder, a shared folder.
//!
//! Nothing here decides on its own what a person may do. Each item is
//! checked with what a single item's action is checked with (`Vault::grid`,
//! `App::may_use`, and `may_place`, the editor's own check for a move
//! between shared folders). An item that doesn't pass is left as it is and
//! said with why (`Skip`), and the server checks every one again. Nothing is
//! encrypted: each call is one a single item's action makes, or the
//! server's call for several (the trash, a folder).

use std::collections::HashSet;

use serde_json::{json, Value};

use super::{may_place, moves, App};
use crate::error::{Error, Result};
use crate::vault::{Item, Vault};

/// What to do to every item of a selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bulk {
    /// Favorite them, or not.
    Favorite(bool),
    /// Move them to the trash.
    Trash,
    /// Put the person's own items in one of their folders, or in none.
    Folder(Option<String>),
    /// Move an organization's items to one of its shared folders (`to`),
    /// out of the one they're shown in (`from`; without it, or for an item
    /// that isn't in it, the first it's in that the person has), as the
    /// editor's folder picker does. The other shared folders an item is in
    /// stay as they are.
    SharedFolder { to: String, from: Option<String> },
}

impl Bulk {
    /// As the app sends it: `{"do": "favorite", "favorite": true}`,
    /// `{"do": "trash"}`, `{"do": "folder", "folderId": null}`,
    /// `{"do": "sharedFolder", "to": "...", "from": null}`.
    pub fn from_json(text: &str) -> Result<Bulk> {
        let unread = Error::Damaged("what to do with the selected items");
        let v: Value = serde_json::from_str(text).map_err(|_| unread.clone())?;
        let id = |name: &str| {
            v[name]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        match v["do"].as_str() {
            Some("favorite") => v["favorite"].as_bool().map(Bulk::Favorite).ok_or(unread),
            Some("trash") => Ok(Bulk::Trash),
            Some("folder") => Ok(Bulk::Folder(id("folderId"))),
            Some("sharedFolder") => Ok(Bulk::SharedFolder {
                to: id("to").ok_or(unread)?,
                from: id("from"),
            }),
            _ => Err(unread),
        }
    }
}

/// Why an item of a selection is left as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// It isn't in the vault any more, or it's in the trash already.
    Gone,
    /// It asks for the master password, which wasn't given.
    MasterPassword,
    /// The organization doesn't let the person delete it.
    NoDelete,
    /// An organization's item, which stays in its shared folders: the
    /// person's own folders are for their own items, as in the editor.
    Shared,
    /// Not that organization's (the person's own, or another
    /// organization's): moving an item between owners is the web vault's.
    OtherOwner,
    /// The person can't see its passwords, so can't change it here (as the
    /// editor refuses to save one).
    HiddenPasswords,
    /// The organization doesn't let the person move it out of the shared
    /// folder it's in, or into that one (`may_place`).
    NoMove,
}

impl Skip {
    /// Its name for the app.
    pub fn name(self) -> &'static str {
        match self {
            Skip::Gone => "gone",
            Skip::MasterPassword => "masterPassword",
            Skip::NoDelete => "noDelete",
            Skip::Shared => "shared",
            Skip::OtherOwner => "otherOwner",
            Skip::HiddenPasswords => "hiddenPasswords",
            Skip::NoMove => "noMove",
        }
    }
}

fn skipped_json(skipped: &[(String, Skip)]) -> Value {
    skipped
        .iter()
        .map(|(id, why)| json!({ "id": id, "why": why.name() }))
        .collect()
}

/// What a bulk action would do (`App::bulk_plan`), from the vault kept
/// here, before anything is sent.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// The items it would be done for, in the order asked.
    pub go: Vec<String>,
    /// The items it would leave as they are, and why.
    pub skipped: Vec<(String, Skip)>,
}

impl Plan {
    pub fn json(&self) -> Value {
        json!({ "ok": true, "go": self.go, "skipped": skipped_json(&self.skipped) })
    }
}

/// How a bulk action went (`App::bulk`): every item asked for is in one of
/// `done`, `skipped`, and `failed`.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The items it's done for (those that were so already among them).
    pub done: Vec<String>,
    /// The items left as they are, and why (`Plan::skipped`).
    pub skipped: Vec<(String, Skip)>,
    /// The items it was to be done for and wasn't: the server refused
    /// them, or couldn't be reached.
    pub failed: Vec<String>,
    /// Why, for `failed`: the first error.
    pub error: Option<Error>,
    /// Whether the sync after it came through. When it didn't, the lists
    /// here show the items as they were until the next one.
    pub synced: bool,
}

impl Outcome {
    pub fn json(&self) -> Value {
        json!({
            "ok": true,
            "done": self.done,
            "skipped": skipped_json(&self.skipped),
            "failed": self.failed,
            "error": self.error.as_ref().map(Error::to_string),
            "offline": self.error.as_ref().is_some_and(Error::offline),
            "signedOut": self.error == Some(Error::SignedOut),
            "synced": self.synced,
        })
    }
}

/// One item's part of a bulk action.
enum Step {
    /// As asked already: nothing to send.
    Already,
    /// Its own call: the favorite mark, with the folder it keeps.
    Favorite {
        folder: Option<String>,
        favorite: bool,
    },
    /// Its own call: the shared folders it's to be in.
    Place(Vec<String>),
    /// In the one call for all of them (the trash, a folder).
    Together,
}

/// What a bulk action does to one item, or why it leaves it. Only checks
/// the single-item actions make: the trash asks for delete on the item, as
/// the item's page offers it; a move between shared folders is the
/// editor's (`App::save`), which refuses an item whose passwords the person
/// can't see and checks the rest with `may_place`.
fn step(vault: &Vault, item: &Item, action: &Bulk) -> std::result::Result<Step, Skip> {
    match action {
        // The person's own mark: an item they can't edit takes it too.
        Bulk::Favorite(favorite) if item.favorite == *favorite => Ok(Step::Already),
        Bulk::Favorite(favorite) => Ok(Step::Favorite {
            folder: item.folder_id.clone(),
            favorite: *favorite,
        }),
        Bulk::Trash if vault.grid(item).delete => Ok(Step::Together),
        Bulk::Trash => Err(Skip::NoDelete),
        Bulk::Folder(_) if item.organization_id.is_some() => Err(Skip::Shared),
        Bulk::Folder(folder) if item.folder_id == *folder => Ok(Step::Already),
        Bulk::Folder(_) => Ok(Step::Together),
        Bulk::SharedFolder { to, from } => {
            let owner = item.organization_id.as_ref();
            if owner.is_none() || owner != vault.collection_orgs.get(to) {
                return Err(Skip::OtherOwner);
            }
            if !vault.grid(item).see_passwords {
                return Err(Skip::HiddenPasswords);
            }
            let placed = placed(vault, item, to, from.as_deref());
            if !moves(item, &placed) {
                return Ok(Step::Already);
            }
            match may_place(vault, item, &placed) {
                Ok(()) => Ok(Step::Place(placed)),
                Err(_) => Err(Skip::NoMove),
            }
        }
    }
}

/// The shared folders an item is in after a move to `to`: `to` in place of
/// the one it's shown in (`from` when it's in it, or the first it's in that
/// the person has), and the others as they are. The editor's rule
/// (editor.dart, `collectionIds`).
fn placed(vault: &Vault, item: &Item, to: &str, from: Option<&str>) -> Vec<String> {
    let held = &item.collection_ids;
    let has = |id: &&String| vault.collections.contains_key(id.as_str());
    let shown = from
        .filter(|from| held.iter().any(|c| c == from))
        .or_else(|| held.iter().find(has).map(String::as_str))
        .or_else(|| held.first().map(String::as_str));
    let rest = held
        .iter()
        .filter(|c| Some(c.as_str()) != shown && c.as_str() != to);
    std::iter::once(to.to_owned())
        .chain(rest.cloned())
        .collect()
}

impl App {
    /// What `bulk` would do with these items, from the vault kept here:
    /// which it would act on, and which it would leave, with why. Nothing
    /// is sent. Items that ask for the master password are among the left
    /// (`Skip::MasterPassword`), as `bulk` leaves them when it isn't given.
    pub fn bulk_plan(&self, ids: &[String], action: &Bulk) -> Result<Plan> {
        let (steps, skipped) = self.bulk_steps(ids, action, false)?;
        Ok(Plan {
            go: steps.into_iter().map(|(id, _)| id).collect(),
            skipped,
        })
    }

    /// Each item's part of a bulk action, and the items it leaves. An item
    /// named twice counts once.
    #[allow(clippy::type_complexity)]
    fn bulk_steps(
        &self,
        ids: &[String],
        action: &Bulk,
        password_given: bool,
    ) -> Result<(Vec<(String, Step)>, Vec<(String, Skip)>)> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let there = match action {
            Bulk::Folder(Some(folder)) => vault.folders.contains_key(folder),
            Bulk::SharedFolder { to, .. } => vault.collections.contains_key(to),
            _ => true,
        };
        if !there {
            return Err(Error::Damaged("no such folder"));
        }
        let mut seen = HashSet::new();
        let mut steps = Vec::new();
        let mut skipped = Vec::new();
        for id in ids {
            if !seen.insert(id.as_str()) {
                continue;
            }
            let Some(item) = vault.item(id).filter(|item| !item.deleted) else {
                skipped.push((id.clone(), Skip::Gone));
                continue;
            };
            match step(vault, item, action) {
                Err(why) => skipped.push((id.clone(), why)),
                // An item that asks for the master password offers nothing
                // until it's given (the item's page shows no actions), so a
                // bulk action leaves it unless it was given for this one.
                Ok(_) if !password_given && !self.may_use(item) => {
                    skipped.push((id.clone(), Skip::MasterPassword));
                }
                Ok(step) => steps.push((id.clone(), step)),
            }
        }
        Ok((steps, skipped))
    }

    /// Does one thing to several items, then syncs once. Items the person
    /// may not do it to are left as they are and said (`Outcome::skipped`),
    /// as `bulk_plan` says beforehand.
    ///
    /// Items that ask for the master password are acted on only when it's
    /// given here (`password`): checked first, and when it's wrong nothing
    /// is done at all. Giving it doesn't open those items' secrets
    /// (`may_use` stays as it was).
    ///
    /// Favorites and shared folders go item by item, with the calls a
    /// single item's action makes; the server's refusal of one leaves the
    /// rest to try, and anything else (no connection, signed out) stops
    /// there. The trash and a folder are one call for all. What went and
    /// what didn't is in the outcome either way.
    pub fn bulk(
        &mut self,
        ids: &[String],
        action: &Bulk,
        password: Option<&str>,
    ) -> Result<Outcome> {
        if let Some(password) = password {
            self.check_master_password(password)?;
        }
        let (steps, skipped) = self.bulk_steps(ids, action, password.is_some())?;
        let mut out = Outcome {
            done: Vec::new(),
            skipped,
            failed: Vec::new(),
            error: None,
            synced: true,
        };
        if steps.iter().all(|(_, step)| matches!(step, Step::Already)) {
            out.done = steps.into_iter().map(|(id, _)| id).collect();
            return Ok(out);
        }
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        let mut together = Vec::new();
        let mut stopped = false;
        for (id, step) in steps {
            let call = match step {
                Step::Already => {
                    out.done.push(id);
                    continue;
                }
                Step::Together => {
                    together.push(id);
                    continue;
                }
                _ if stopped => {
                    out.failed.push(id);
                    continue;
                }
                Step::Favorite { folder, favorite } => {
                    server.set_favorite(session, &id, folder.as_deref(), favorite)
                }
                Step::Place(placed) => server.set_item_collections(session, &id, &placed),
            };
            match call {
                Ok(()) => out.done.push(id),
                Err(e) => {
                    stopped = !matches!(e, Error::Server(_) | Error::Status(_));
                    out.failed.push(id);
                    out.error.get_or_insert(e);
                }
            }
        }
        let sent = match action {
            _ if together.is_empty() => Ok(()),
            Bulk::Trash => server.trash_items(session, &together),
            Bulk::Folder(folder) => server.move_items(session, &together, folder.as_deref()),
            // Only those two go together.
            Bulk::Favorite(_) | Bulk::SharedFolder { .. } => Ok(()),
        };
        out.synced = self.sync().is_ok();
        match sent {
            Ok(()) => out.done.append(&mut together),
            Err(e) => {
                // The server stops at the first item it refuses, with the
                // ones before it done: the vault it sent after says which.
                // Without that vault, none is said to be done.
                let vault = self.vault.as_ref().filter(|_| out.synced);
                for id in together {
                    let item = vault.and_then(|vault| vault.item(&id));
                    let went = match action {
                        Bulk::Trash => vault.is_some() && item.is_none_or(|i| i.deleted),
                        Bulk::Folder(folder) => item.is_some_and(|i| i.folder_id == *folder),
                        Bulk::Favorite(_) | Bulk::SharedFolder { .. } => false,
                    };
                    if went {
                        out.done.push(id);
                    } else {
                        out.failed.push(id);
                    }
                }
                out.error.get_or_insert(e);
            }
        }
        Ok(out)
    }
}
