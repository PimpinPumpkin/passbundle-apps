//! Making a file Send (DESIGN.md in the meta repo, "Send in the apps";
//! docs/core.md, "Send"): the file and its name encrypted on this device
//! under the Send's key, as the web vault encrypts them, so the link opens
//! on the web vault's access page.
//!
//! A file goes up as an attachment does (files.rs), in steps, and only three
//! of them need the app: checking the draft and making the Send's seed and
//! its encrypted fields (`file_send_to_make`), making the Send on the
//! server, which answers where its file goes (`start_file_send`), and the
//! sync once the file is there (`finish_file_send`). Reading the file,
//! encrypting it, and sending it happen between them without the app
//! (`send_file`), so a long upload doesn't hold up its other calls.
//!
//! The server makes the Send before it has the file. Until the whole file
//! is there, the app doesn't list it (`hides_send`); one whose upload failed
//! or was canceled is deleted on the server, as the web vault deletes one,
//! now or before the next sync.

use std::path::Path;

use serde_json::Value;
use zeroize::Zeroizing;

use super::files::{ask, read_file, FileUse, Phase, Progress, Reach};
use super::sends::{SavedSend, DISABLED, NOT_AS_SENT};
use super::{forget_refresh_token, keep_refresh_token, App};
use crate::crypto::SymmetricKey;
use crate::error::{Error, Result};
use crate::send::{self, Seed, Send, SendDraft, SendKind};
use crate::server::Upload;

/// The longest name a file is sent under, as for an attachment.
const MAX_NAME: usize = 255;

/// A file Send about to be made (`App::file_send_to_make`): its seed and
/// the key from it, which the file is encrypted with, and what the server
/// gets, every field encrypted already. The seed leaves only wrapped under
/// the user key, inside `body`.
pub struct MakingFileSend {
    seed: Seed,
    key: SymmetricKey,
    name: String,
    file_name: String,
    body: Value,
}

impl std::fmt::Debug for MakingFileSend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MakingFileSend").finish_non_exhaustive()
    }
}

/// A file Send this app made on the server that the list doesn't show: its
/// file is still on its way, or (`delete`) its upload failed or was
/// canceled and it's to be deleted there. The Send as the server answered
/// it waits in `made` until its file is there. Never the seed alone.
pub(super) struct UnfinishedSend {
    id: String,
    delete: bool,
    made: Option<Send>,
}

/// Makes a file Send of the file at `path`, named `name` (the file's own
/// name when empty), with the options in `draft` and, if given, a
/// password: every step, with the app asked only for the three that need
/// it. `progress` says how far it is and hears Cancel.
///
/// Nothing is left half made: a file that can't be read, or is too large,
/// never reaches the server; a Send whose upload fails or is canceled is
/// deleted (`App::finish_file_send`).
pub fn send_file(
    on_app: Reach<'_>,
    draft: &SendDraft,
    path: &Path,
    name: &str,
    password: Option<Zeroizing<String>>,
    progress: &Progress,
) -> Result<SavedSend> {
    let own_name;
    let name = if name.is_empty() {
        own_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        own_name.as_str()
    } else {
        name
    };
    let making = ask(on_app, |app| app.file_send_to_make(draft, name, password))?;
    let file = read_file(path, progress, FileUse::Sending)?;
    progress.go_on()?;
    progress.enter(Phase::Encrypting, 0);
    // In place, under the Send's own key, as the web vault encrypts a
    // Send's file: after this the buffer holds the encrypted file only.
    let encrypted = file.encrypt(&making.key);
    progress.go_on()?;
    let size = encrypted.len() as u64;
    let (id, upload) = ask(on_app, |app| app.start_file_send(&making, size))?;
    progress.enter(Phase::Sending, size);
    let sent = upload.send(&encrypted, &mut |sent| {
        progress.set(sent);
        !progress.canceled()
    });
    drop(encrypted);
    drop(upload);
    progress.enter(Phase::Finishing, 0);
    ask(on_app, |app| app.finish_file_send(&id, sent))
}

impl App {
    /// The first step of a file Send: checks the policies and the draft,
    /// and makes the Send's seed, the key from it, and every field
    /// encrypted under it, the file's name too (`SendDraft::body`). A
    /// password, if one is given, is hashed with the seed and wiped.
    /// Nothing goes to the server.
    pub fn file_send_to_make(
        &self,
        draft: &SendDraft,
        file_name: &str,
        password: Option<Zeroizing<String>>,
    ) -> Result<MakingFileSend> {
        if draft.kind == Some(SendKind::Text) {
            return Err(Error::Refused("a text Send has no file"));
        }
        self.may_save_send(draft)?;
        if file_name.trim().is_empty() {
            return Err(Error::Refused("a file needs a name to be sent"));
        }
        if file_name.chars().count() > MAX_NAME {
            return Err(Error::Refused(
                "that file's name is longer than 255 characters. Rename it, then send it",
            ));
        }
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let seed = send::new_seed();
        let proof = password
            .filter(|p| !p.is_empty())
            .map(|p| send::password_proof(&p, &*seed));
        let body = draft.body(
            SendKind::File,
            &*seed,
            vault.user_key(),
            proof,
            Some(file_name),
        );
        Ok(MakingFileSend {
            key: send::key_from_seed(&*seed),
            seed,
            name: draft.name.clone(),
            file_name: file_name.to_owned(),
            body,
        })
    }

    /// The second step: makes the Send on the server, telling it a file of
    /// `size` bytes (its encrypted length) is coming, after checking the
    /// policies again (the vault may have locked, or a sync changed them,
    /// while the file was read). The server's answer must open, under the
    /// user key, to the seed, the name, and the file name sent; one that
    /// doesn't never gets its file, and is deleted. Answers the Send's id
    /// and its file's upload. The list doesn't show it yet (`hides_send`).
    pub fn start_file_send(
        &mut self,
        making: &MakingFileSend,
        size: u64,
    ) -> Result<(String, Upload)> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        if self.send_policy.disabled {
            return Err(DISABLED);
        }
        let user = vault.user_key().clone();
        let (Some(server), Some(settings), Some(session)) =
            (&self.server, &self.settings, self.session.as_mut())
        else {
            return Err(Error::SignedOut);
        };
        let started = server.start_file_send(session, making.body.clone(), size);
        let (answer, upload) = match started {
            Err(Error::SignedOut) => {
                self.session = None;
                forget_refresh_token(&*self.secrets, &self.store, settings);
                return Err(Error::SignedOut);
            }
            other => other?,
        };
        // Asking may have refreshed the session, as for a sync.
        keep_refresh_token(&*self.secrets, &self.store, settings, session);
        let made = Send::open(&answer, &user);
        let as_sent = made.opens
            && made.kind == SendKind::File
            && made.has_seed(&*making.seed)
            && made.name == making.name
            && made.file_name.as_deref() == Some(making.file_name.as_str());
        if !as_sent {
            self.unfinished_sends.push(UnfinishedSend {
                id: answer.id,
                delete: true,
                made: None,
            });
            let _ = self.sync();
            return Err(NOT_AS_SENT);
        }
        self.unfinished_sends.push(UnfinishedSend {
            id: answer.id.clone(),
            delete: false,
            made: Some(made),
        });
        Ok((answer.id, upload))
    }

    /// The last step, with how the upload went.
    ///
    /// Sent whole: the Send is listed from here on, and the app syncs; if
    /// the sync doesn't come through, it's listed as the server answered
    /// it until the next one, so its link can be copied
    /// (`SavedSend::synced`).
    ///
    /// Failed or canceled: the Send is deleted on the server. If that
    /// can't reach the server now, it's tried again before each sync, and
    /// the list doesn't show the Send meanwhile.
    pub fn finish_file_send(&mut self, id: &str, sent: Result<()>) -> Result<SavedSend> {
        if let Err(e) = sent {
            for unfinished in self.unfinished_sends.iter_mut().filter(|u| u.id == id) {
                unfinished.delete = true;
                unfinished.made = None;
            }
            let _ = self.sync();
            return Err(e);
        }
        let made = self
            .unfinished_sends
            .iter_mut()
            .find(|u| u.id == id)
            .and_then(|u| u.made.take());
        self.unfinished_sends.retain(|u| u.id != id);
        let synced = match made {
            Some(made) => self.synced_with(made),
            None => self.sync().is_ok(),
        };
        Ok(SavedSend {
            id: id.to_owned(),
            synced,
        })
    }

    /// Makes a file Send, start to finish, holding the app throughout
    /// (tests and tools; the bridge uses `send_file`, which lets go of the
    /// app while the file travels).
    pub fn send_file(
        &mut self,
        draft: &SendDraft,
        path: &Path,
        name: &str,
        password: Option<Zeroizing<String>>,
    ) -> Result<SavedSend> {
        send_file(
            &mut |call| call(self),
            draft,
            path,
            name,
            password,
            &Progress::new(),
        )
    }

    /// Whether a Send is one the list doesn't show: its file is on its way,
    /// or it's to be deleted on the server.
    pub fn hides_send(&self, id: &str) -> bool {
        self.unfinished_sends.iter().any(|u| u.id == id)
    }

    /// Before a sync: deletes on the server the file Sends this app made
    /// and didn't finish. What can't be deleted now stays for the next sync.
    pub(super) fn delete_unfinished_sends(&mut self) {
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return;
        };
        for unfinished in self.unfinished_sends.iter().filter(|u| u.delete) {
            let _ = server.delete_send(session, &unfinished.id);
        }
    }

    /// After a sync: a Send to delete that the vault doesn't list anymore is
    /// gone, and forgotten. One it still lists stays hidden, and is tried
    /// again before the next sync.
    pub(super) fn forget_deleted_sends(&mut self) {
        let Some(vault) = &self.vault else {
            return;
        };
        self.unfinished_sends
            .retain(|u| !u.delete || vault.send(&u.id).is_some());
    }

    /// Signing out: nothing of this account's is asked of a server again.
    pub(super) fn forget_unfinished_sends(&mut self) {
        self.unfinished_sends.clear();
    }
}
