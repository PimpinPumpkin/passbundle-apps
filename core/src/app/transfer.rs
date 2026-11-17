//! Exporting and importing a file (meta repo DESIGN.md, "Export and import
//! in the apps"; export.rs and import.rs for the formats).
//!
//! - **Exporting takes the master password itself** (`confirm_export`),
//!   never a PIN or biometrics ("Unlock with a PIN": "exporting ... take the
//!   master password itself"), and it's good for one export, within
//!   `EXPORT_CONFIRMATION_LASTS`. An organization's "Remove individual vault
//!   export" policy refuses it, where a server offers that policy.
//! - **Importing reads first and saves later:** `read_import` reads and sorts
//!   the file and keeps what it found here, in memory, for the person to see;
//!   `save_import` encrypts it with the keys of where it goes and sends it.
//!   Where an organization keeps items out of Personal, Personal isn't
//!   offered, and a shared folder the person may add to is.

use std::time::{Duration, Instant};

use super::App;
use crate::error::{Error, Result};
use crate::export::{self, Format as ExportFormat};
use crate::import::{self, Format as ImportFormat, Parsed, Read};
use crate::kdf::Kdf;

/// How long the master password, given to export, lasts: time to pick
/// where the file goes.
pub const EXPORT_CONFIRMATION_LASTS: Duration = Duration::from_secs(10 * 60);

/// The largest file PassBundle imports.
pub const MAX_IMPORT_BYTES: u64 = 100 * 1024 * 1024;

/// At most this many items go to the server in one request.
const BATCH: usize = 250;

/// An export, written: how many items are in it, and how many of the
/// person's own aren't.
#[derive(Debug, PartialEq, Eq)]
pub struct Exported {
    pub items: usize,
    pub left_out: usize,
}

/// Where an import's items go.
#[derive(Debug, PartialEq, Eq)]
pub enum Destination {
    /// The person's own items, with the file's folders.
    Personal,
    /// One of an organization's shared folders, by its id: every item goes
    /// in it, and the file's folders aren't kept.
    SharedFolder(String),
}

/// An import, saved: how many items, and whether the sync after it came
/// through (the vault shows them either way, at the next sync if not now).
#[derive(Debug, PartialEq, Eq)]
pub struct ImportSaved {
    pub added: usize,
    pub synced: bool,
}

/// What reading a file to import came to.
#[derive(Debug, PartialEq, Eq)]
pub enum ImportRead {
    /// Read: what it holds waits here (`App::pending_import`).
    Read,
    /// A password-protected file: its password, please.
    NeedsPassword,
    /// That isn't the file's password.
    WrongPassword,
}

/// Importing into Personal, where an organization keeps items out of it.
const KEEPS_ITEMS_OUT: Error = Error::Refused(
    "your organization keeps items out of Personal, so PassBundle can't import there. Choose a shared folder you can add to, or ask an admin for one",
);

const FORBIDDEN: Error =
    Error::Refused("an organization you're in doesn't let you export your own items");
const CONFIRM_FIRST: Error = Error::Refused("type your master password first, then export");
const NOTHING_READ: Error = Error::Refused("choose a file to import first");
const TOO_LARGE: Error = Error::Refused("that file is larger than PassBundle imports (100 MB)");
const UNREADABLE: Error = Error::Refused(
    "PassBundle couldn't read that file. Check that it's still there, then choose it again",
);
const NOT_THERE: Error = Error::Refused(
    "that shared folder isn't one you can add items to. Choose another, or sync to see what's there now",
);

impl App {
    /// The organization whose policy doesn't let this person export their
    /// own items, by name, from the kept sync; None when they may.
    pub fn export_forbidden_by(&self) -> Option<String> {
        import::forbids_export(self.store.sync().as_deref())
    }

    /// The master password, given to export: checked the way unlocking
    /// checks it, never a PIN's or biometrics' session standing in. Good for
    /// one export (`export_to`) within `EXPORT_CONFIRMATION_LASTS`.
    pub fn confirm_export(&mut self, password: &str) -> Result<()> {
        self.export_confirmed = None;
        if self.export_forbidden_by().is_some() {
            return Err(FORBIDDEN);
        }
        self.check_master_password(password)?;
        self.export_confirmed = Some(Instant::now());
        Ok(())
    }

    /// Makes the export and writes it at `path` (where the person chose to
    /// save it, or a phone's own folder that hands it on): the master
    /// password must have been given just now (`confirm_export`), and that
    /// confirmation is used up. A password-protected one is under
    /// `file_password`, with this account's own KDF settings.
    pub fn export_to(
        &mut self,
        format: ExportFormat,
        path: &std::path::Path,
        file_password: Option<&str>,
    ) -> Result<Exported> {
        let confirmed = self
            .export_confirmed
            .take()
            .is_some_and(|at| at.elapsed() < EXPORT_CONFIRMATION_LASTS);
        if !confirmed {
            return Err(CONFIRM_FIRST);
        }
        if self.export_forbidden_by().is_some() {
            return Err(FORBIDDEN);
        }
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let made = match format {
            ExportFormat::Json => export::json(vault)?,
            ExportFormat::Csv => export::csv(vault)?,
            ExportFormat::PasswordProtected => {
                let password = file_password
                    .filter(|p| !p.is_empty())
                    .ok_or(Error::Refused(
                        "a password-protected export needs a file password",
                    ))?;
                let settings = self.settings.as_ref().ok_or(Error::SignedOut)?;
                let kdf = Kdf::from_prelogin(&settings.prelogin())?;
                let plain = export::json(vault)?;
                export::Made {
                    text: export::protect(&plain.text, password, &kdf)?,
                    items: plain.items,
                    left_out: plain.left_out,
                }
            }
        };
        export::write(path, &made.text)?;
        Ok(Exported {
            items: made.items,
            left_out: made.left_out,
        })
    }

    /// Reads a file to import, in the format the person chose, with the
    /// password a password-protected `.json` is under once they've given it
    /// (`Read::NeedsPassword` asks for it). What's in it waits here
    /// (`pending_import`) until it's saved or given up; nothing is sent.
    pub fn read_import(
        &mut self,
        format: ImportFormat,
        path: &std::path::Path,
        file_password: Option<&str>,
    ) -> Result<ImportRead> {
        self.pending_import = None;
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let size = std::fs::metadata(path).map_err(|_| UNREADABLE)?.len();
        if size > MAX_IMPORT_BYTES {
            return Err(TOO_LARGE);
        }
        let bytes = zeroize::Zeroizing::new(std::fs::read(path).map_err(|_| UNREADABLE)?);
        // As the web vault reads a file: UTF-8, with anything that isn't
        // read as the replacement character.
        let text = zeroize::Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned());
        Ok(match import::read(format, &text, file_password, vault)? {
            Read::Parsed(parsed) => {
                self.pending_import = Some(parsed);
                ImportRead::Read
            }
            Read::NeedsPassword => ImportRead::NeedsPassword,
            Read::WrongPassword => ImportRead::WrongPassword,
        })
    }

    /// What the file read last holds, waiting to be saved.
    pub fn pending_import(&self) -> Option<&Parsed> {
        self.pending_import.as_ref()
    }

    /// Gives up the file read last, wiping what it held.
    pub fn cancel_import(&mut self) {
        self.pending_import = None;
    }

    /// The organization that keeps this person's items out of Personal, by
    /// name, from the kept sync.
    pub fn keeps_items_out(&self) -> Option<String> {
        import::keeps_items_out(self.store.sync().as_deref())
    }

    /// The shared folders an import may go to: those the person may add
    /// items to, whose organization's key they have, by name.
    pub fn import_shared_folders(&self) -> Vec<(String, String, String, String)> {
        let Some(vault) = self.vault.as_ref() else {
            return Vec::new();
        };
        let mut folders: Vec<(String, String, String, String)> = vault
            .collections
            .iter()
            .filter(|(id, _)| vault.collection_grids.get(*id).is_some_and(|g| g.create))
            .filter_map(|(id, name)| {
                let org = vault.collection_orgs.get(id)?;
                let org_name = vault.organizations.get(org)?;
                Some((id.clone(), name.clone(), org.clone(), org_name.clone()))
            })
            .collect();
        folders.sort_by(|a, b| {
            (a.3.to_lowercase(), a.1.to_lowercase()).cmp(&(b.3.to_lowercase(), b.1.to_lowercase()))
        });
        folders
    }

    /// Saves the file read last where the person chose, then syncs: each
    /// item encrypted on this device with the keys of where it goes, at most
    /// `BATCH` to a request. Personal gets the file's folders (one already
    /// there of the same name is used) and, for a browser's export, its
    /// "Check these" folder. A request the server refuses partway leaves the
    /// ones before it saved (`Parsed::saved` counts them), and saving again
    /// sends only the rest.
    pub fn save_import(&mut self, to: &Destination) -> Result<ImportSaved> {
        let parsed = self.pending_import.as_ref().ok_or(NOTHING_READ)?;
        let start = parsed.saved;
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        if self.server.is_none() || self.session.is_none() {
            return Err(Error::SignedOut);
        }
        let sent = match to {
            Destination::Personal => {
                if self.keeps_items_out().is_some() {
                    return Err(KEEPS_ITEMS_OUT);
                }
                let ciphers = parsed.items[start..]
                    .iter()
                    .map(|item| import::encrypt(vault, item, None))
                    .collect::<Result<Vec<_>>>()?;
                // Each folder the items go in, as the file names them, then
                // the one to compare in.
                let mut names: Vec<String> = parsed.folders.clone();
                if let Some(check) = &parsed.check_folder {
                    names.push(check.clone());
                }
                let mut folder_of_item: Vec<Option<usize>> =
                    parsed.items.iter().map(|i| i.folder).collect();
                if parsed.check_folder.is_some() {
                    for at in &parsed.to_check {
                        folder_of_item[*at] = Some(names.len() - 1);
                    }
                }
                let folder_of_item = folder_of_item.split_off(start);
                let encrypted: Vec<String> = names
                    .iter()
                    .map(|n| vault.user_key().encrypt_str(n).to_string())
                    .collect();
                let existing: Vec<Option<String>> = names
                    .iter()
                    .map(|n| {
                        vault
                            .folders
                            .iter()
                            .find(|(_, name)| *name == n)
                            .map(|(id, _)| id.clone())
                    })
                    .collect();
                let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
                    return Err(Error::SignedOut);
                };
                // The folders first, so every request puts its items in the
                // same ones; only those an item goes in.
                let mut ids = existing;
                for (at, id) in ids.iter_mut().enumerate() {
                    let used = folder_of_item.contains(&Some(at));
                    if id.is_none() && used {
                        *id = Some(server.create_folder(session, &encrypted[at])?);
                    }
                }
                let folders: Vec<(Option<String>, String)> =
                    ids.into_iter().zip(encrypted).collect();
                let mut sent: std::result::Result<usize, (Error, usize)> = Ok(0);
                for (n, batch) in ciphers.chunks(BATCH).enumerate() {
                    let first = n * BATCH;
                    let folder_of: Vec<(usize, usize)> = folder_of_item[first..first + batch.len()]
                        .iter()
                        .enumerate()
                        .filter_map(|(i, f)| f.map(|f| (i, f)))
                        .collect();
                    match server.import_items(session, batch, &folders, &folder_of) {
                        Ok(()) => sent = sent.map(|done| done + batch.len()),
                        Err(e) => {
                            sent = Err((e, n * BATCH));
                            break;
                        }
                    }
                }
                sent
            }
            Destination::SharedFolder(id) => {
                let may = vault.collection_grids.get(id).is_some_and(|g| g.create);
                let (Some(org), Some(name), true) = (
                    vault.collection_orgs.get(id),
                    vault.collections.get(id),
                    may,
                ) else {
                    return Err(NOT_THERE);
                };
                let encrypted_name = vault.encrypt_collection_name(org, name)?;
                let ciphers = parsed.items[start..]
                    .iter()
                    .map(|item| import::encrypt(vault, item, Some(org)))
                    .collect::<Result<Vec<_>>>()?;
                let org = org.clone();
                let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
                    return Err(Error::SignedOut);
                };
                let mut sent: std::result::Result<usize, (Error, usize)> = Ok(0);
                for (n, batch) in ciphers.chunks(BATCH).enumerate() {
                    match server.import_organization_items(
                        session,
                        &org,
                        batch,
                        id,
                        &encrypted_name,
                    ) {
                        Ok(()) => sent = sent.map(|done| done + batch.len()),
                        Err(e) => {
                            sent = Err((e, n * BATCH));
                            break;
                        }
                    }
                }
                sent
            }
        };
        let synced = self.sync().is_ok();
        match sent {
            Ok(added) => {
                self.pending_import = None;
                Ok(ImportSaved { added, synced })
            }
            Err((e, done)) => {
                if let Some(parsed) = self.pending_import.as_mut() {
                    parsed.saved += done;
                }
                Err(e)
            }
        }
    }
}
