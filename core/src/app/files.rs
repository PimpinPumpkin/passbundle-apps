//! Adding a file to an item, and deleting one (docs/core.md, "Attachments";
//! DESIGN.md in the meta repo, "Attachments in the apps").
//!
//! A file goes up in steps, and only three of them need the app: checking
//! the item and making the attachment's key (`attachment_to_add`), telling
//! the server a file is coming (`start_attachment`), and looking at what
//! the server kept (`finish_attachment`). Reading the file, encrypting it,
//! and sending it happen between them without the app (`attach`), so a long
//! upload doesn't hold up its other calls, as a download doesn't.
//!
//! The server makes an attachment's record before it has the file. One the
//! app started is kept out of the item's attachments (`hides_attachment`)
//! until the server has the whole file and a sync lists it with the key and
//! the name made here; one that failed or was canceled is deleted there,
//! now or before the next sync.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use super::{forget_refresh_token, keep_refresh_token, App};
use crate::crypto::FileToEncrypt;
use crate::error::{Error, Result};
use crate::server::Upload;
use crate::vault::{Item, NewAttachment, Vault};

/// The largest file the apps attach. The whole file is in memory once to
/// encrypt it (its format has one integrity check, over all of it), so a
/// phone takes less than a computer. Both are under what the server takes.
pub const MAX_FILE: u64 = if cfg!(phone) { 100 } else { 500 } * 1024 * 1024;

/// The longest name a file is attached under.
const MAX_NAME: usize = 255;

/// Where an upload is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Reading,
    Encrypting,
    Sending,
    /// The server has the file; the vault is being brought up to date.
    Finishing,
}

impl Phase {
    /// As the app hears it.
    pub fn word(self) -> &'static str {
        match self {
            Phase::Reading => "reading",
            Phase::Encrypting => "encrypting",
            Phase::Sending => "sending",
            Phase::Finishing => "finishing",
        }
    }
}

/// How far an upload is, for the screen to ask while it runs, and the
/// person's Cancel. Shared between the thread that uploads and the ones
/// that ask, so nothing here waits for the app.
#[derive(Default)]
pub struct Progress {
    /// 0 when nothing is uploading, or a `Phase`'s place, counted from 1.
    phase: AtomicU8,
    done: AtomicU64,
    total: AtomicU64,
    canceled: AtomicBool,
}

impl Progress {
    pub const fn new() -> Progress {
        Progress {
            phase: AtomicU8::new(0),
            done: AtomicU64::new(0),
            total: AtomicU64::new(0),
            canceled: AtomicBool::new(false),
        }
    }

    /// A new upload: nothing done, not canceled.
    pub fn begin(&self) {
        self.canceled.store(false, Ordering::SeqCst);
        self.enter(Phase::Reading, 0);
    }

    /// No upload anymore.
    pub fn end(&self) {
        self.phase.store(0, Ordering::SeqCst);
        self.done.store(0, Ordering::SeqCst);
        self.total.store(0, Ordering::SeqCst);
    }

    /// The person's Cancel. It stops a file being read, encrypted, or
    /// sent; once the server has all of it, the file is attached.
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::SeqCst);
    }

    pub fn canceled(&self) -> bool {
        self.canceled.load(Ordering::SeqCst)
    }

    /// Where the upload is, or None when there's none.
    pub fn phase(&self) -> Option<Phase> {
        match self.phase.load(Ordering::SeqCst) {
            1 => Some(Phase::Reading),
            2 => Some(Phase::Encrypting),
            3 => Some(Phase::Sending),
            4 => Some(Phase::Finishing),
            _ => None,
        }
    }

    /// How many bytes of this phase are done, and of how many (0 when the
    /// phase has no amount to show).
    pub fn amounts(&self) -> (u64, u64) {
        (
            self.done.load(Ordering::SeqCst),
            self.total.load(Ordering::SeqCst),
        )
    }

    fn enter(&self, phase: Phase, total: u64) {
        self.done.store(0, Ordering::SeqCst);
        self.total.store(total, Ordering::SeqCst);
        self.phase.store(phase as u8 + 1, Ordering::SeqCst);
    }

    fn set(&self, done: u64) {
        self.done.store(done, Ordering::SeqCst);
    }

    /// Stops here if the person canceled.
    fn go_on(&self) -> Result<()> {
        if self.canceled() {
            Err(Error::Canceled)
        } else {
            Ok(())
        }
    }
}

/// A file about to be attached (`App::attachment_to_add`): the item, the
/// file's name, and the key and encrypted name made for it.
pub struct Adding {
    item: String,
    name: String,
    new: NewAttachment,
}

/// A file attached (`App::finish_attachment`): its attachment's id, and
/// whether the sync after it came through. When it didn't, the server has
/// the file all the same, and the item shows it at the next sync.
#[derive(Debug, PartialEq, Eq)]
pub struct Attached {
    pub id: String,
    pub synced: bool,
}

/// An attachment this app started on the server, which the item doesn't
/// show: one whose file is still on its way, or (`delete`) one to delete
/// there, because its upload failed or was canceled, or because it was
/// deleted and no sync has come through since. Ids only, never a key.
pub(super) struct Unfinished {
    item: String,
    attachment: String,
    delete: bool,
}

/// What the person hears for a file larger than `MAX_FILE`.
pub fn too_large() -> Error {
    Error::Refused(if cfg!(phone) {
        "that file is larger than 100 MB, the most PassBundle attaches on a phone. Attach it from a computer"
    } else {
        "that file is larger than 500 MB, the most PassBundle attaches"
    })
}

/// Whether the person may change an item's attachments now: the rule the
/// item's Edit follows, and the editor's for one that asks for the master
/// password again. The server checks again.
fn may_change(vault: &Vault, item: &Item, may_use: bool) -> Result<()> {
    if item.deleted {
        return Err(Error::Refused(
            "this item is in the trash. Restore it to change its attachments",
        ));
    }
    if !may_use {
        return Err(Error::MasterPasswordFirst);
    }
    if !vault.grid(item).edit {
        return Err(Error::Refused(
            "you can view this item but not change it, so you can't change its attachments",
        ));
    }
    Ok(())
}

/// Reads the file at `path`, whole, into the room its encrypted form needs
/// (`FileToEncrypt`), a piece at a time, so it's in memory once. Its length
/// is checked before any of it is read.
pub fn read_file(path: &Path, progress: &Progress) -> Result<FileToEncrypt> {
    use std::io::Read;
    /// Read at a time, so progress is said, and Cancel heard, often.
    const PIECE: usize = 1024 * 1024;
    let unread = Error::Refused(
        "PassBundle couldn't read that file. Check that it's still there and that you may open it",
    );
    let mut file = std::fs::File::open(path).map_err(|_| unread.clone())?;
    let about = file.metadata().map_err(|_| unread.clone())?;
    if !about.is_file() {
        return Err(Error::Refused("that isn't a file PassBundle can attach"));
    }
    let len = about.len();
    if len > MAX_FILE {
        return Err(too_large());
    }
    progress.enter(Phase::Reading, len);
    let mut room = FileToEncrypt::with_room(len as usize)?;
    loop {
        progress.go_on()?;
        let read = room
            .read_from(&mut file, PIECE)
            .map_err(|_| unread.clone())?;
        if read == 0 {
            break;
        }
        progress.set(room.len() as u64);
    }
    // One that grew or shrank meanwhile isn't the file the person chose.
    let more = file.read(&mut [0u8; 1]).map_err(|_| unread)? != 0;
    if more || room.len() as u64 != len {
        return Err(Error::Refused(
            "that file changed while PassBundle was reading it. Try again",
        ));
    }
    Ok(room)
}

/// Something to do with the app.
pub type OnApp<'a> = &'a mut dyn FnMut(&mut App);

/// How an upload reaches the item's account's app for each step that needs
/// it: it runs what it's given on that app, however the caller gets to it
/// (the bridge takes its lock for that moment only).
pub type Reach<'a> = &'a mut dyn FnMut(OnApp<'_>);

/// One step of an upload that needs the app.
fn ask<T>(on_app: Reach<'_>, call: impl FnOnce(&mut App) -> Result<T>) -> Result<T> {
    let mut call = Some(call);
    let mut answer = Err(Error::SignedOut);
    on_app(&mut |app| {
        if let Some(call) = call.take() {
            answer = call(app);
        }
    });
    answer
}

/// Attaches the file at `path` to an item, under `name` (the file's own
/// name when empty): every step, with the app asked only for the three
/// that need it. `progress` says how far it is and hears Cancel.
///
/// Nothing is left half made: a file that can't be read, or is too large,
/// never reaches the server; an upload that fails or is canceled has its
/// record deleted (`App::finish_attachment`).
pub fn attach(
    on_app: Reach<'_>,
    item: &str,
    path: &Path,
    name: &str,
    progress: &Progress,
) -> Result<Attached> {
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
    let adding = ask(on_app, |app| app.attachment_to_add(item, name))?;
    let file = read_file(path, progress)?;
    progress.go_on()?;
    progress.enter(Phase::Encrypting, 0);
    // In place: after this the buffer holds the encrypted file only.
    let encrypted = file.encrypt(&adding.new.key);
    progress.go_on()?;
    let size = encrypted.len() as u64;
    let (id, upload) = ask(on_app, |app| app.start_attachment(&adding, size))?;
    progress.enter(Phase::Sending, size);
    let sent = upload.send(&encrypted, &mut |sent| {
        progress.set(sent);
        !progress.canceled()
    });
    drop(encrypted);
    drop(upload);
    progress.enter(Phase::Finishing, 0);
    ask(on_app, |app| app.finish_attachment(&adding, &id, sent))
}

impl App {
    /// The first step of attaching a file: checks the person may change
    /// the item's attachments now, and makes the attachment's key and its
    /// encrypted name under the item's key (`Vault::new_attachment`).
    /// Nothing goes to the server.
    pub fn attachment_to_add(&self, id: &str, name: &str) -> Result<Adding> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let item = vault.item(id).ok_or(Error::Damaged("no such item"))?;
        may_change(vault, item, self.may_use(item))?;
        if name.trim().is_empty() {
            return Err(Error::Refused("a file needs a name to be attached"));
        }
        if name.chars().count() > MAX_NAME {
            return Err(Error::Refused(
                "that file's name is longer than 255 characters. Rename it, then attach it",
            ));
        }
        Ok(Adding {
            item: id.to_owned(),
            name: name.to_owned(),
            new: vault.new_attachment(item, name)?,
        })
    }

    /// The second step: tells the server a file of `size` bytes (its
    /// encrypted length) is coming, after checking the item again (the
    /// vault may have locked, or the item changed, while the file was
    /// read). Answers the attachment's id and its upload. The item doesn't
    /// show the attachment yet (`hides_attachment`).
    pub fn start_attachment(&mut self, adding: &Adding, size: u64) -> Result<(String, Upload)> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let item = vault
            .item(&adding.item)
            .ok_or(Error::Damaged("no such item"))?;
        may_change(vault, item, self.may_use(item))?;
        let (Some(server), Some(settings), Some(session)) =
            (&self.server, &self.settings, self.session.as_mut())
        else {
            return Err(Error::SignedOut);
        };
        let started = server.start_attachment(
            session,
            &adding.item,
            &adding.new.wrapped_key,
            &adding.new.file_name,
            size,
        );
        let (id, upload) = match started {
            Err(Error::SignedOut) => {
                self.session = None;
                forget_refresh_token(&*self.secrets, &self.store, settings);
                return Err(Error::SignedOut);
            }
            other => other?,
        };
        // Asking may have refreshed the session, as for a sync.
        keep_refresh_token(&*self.secrets, &self.store, settings, session);
        self.uploads.push(Unfinished {
            item: adding.item.clone(),
            attachment: id.clone(),
            delete: false,
        });
        Ok((id, upload))
    }

    /// The last step, with how the upload went.
    ///
    /// Sent whole: syncs, and the file is attached only if the item now
    /// lists it with the key and the name made here. If it lists anything
    /// else under that id, the attachment is deleted and this fails. If the
    /// sync doesn't come through (or the vault locked meanwhile), the file
    /// is on the server and shows at the next sync (`Attached::synced`).
    ///
    /// Failed or canceled: the record the server made is deleted. If that
    /// can't reach the server now, it's tried again before each sync, and
    /// the item doesn't show the attachment meanwhile.
    pub fn finish_attachment(
        &mut self,
        adding: &Adding,
        id: &str,
        sent: Result<()>,
    ) -> Result<Attached> {
        if let Err(e) = sent {
            self.delete_unfinished_after(id);
            return Err(e);
        }
        if self.sync().is_err() {
            self.uploads.retain(|u| u.attachment != id);
            return Ok(Attached {
                id: id.to_owned(),
                synced: false,
            });
        }
        let kept = self.vault.as_ref().is_some_and(|vault| {
            vault.has_attachment(&adding.item, id, &adding.new.key, &adding.name)
        });
        if !kept {
            self.delete_unfinished_after(id);
            return Err(Error::Damaged(
                "an attachment the server didn't keep as it was sent",
            ));
        }
        self.uploads.retain(|u| u.attachment != id);
        Ok(Attached {
            id: id.to_owned(),
            synced: true,
        })
    }

    /// Marks an attachment this app started to be deleted on the server,
    /// and syncs, which deletes it first (`delete_unfinished`).
    fn delete_unfinished_after(&mut self, id: &str) {
        for upload in self.uploads.iter_mut().filter(|u| u.attachment == id) {
            upload.delete = true;
        }
        let _ = self.sync();
    }

    /// Before a sync: deletes on the server what this app started and
    /// didn't finish. What can't be deleted now stays for the next sync.
    pub(super) fn delete_unfinished(&mut self) {
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return;
        };
        for upload in self.uploads.iter().filter(|u| u.delete) {
            let _ = server.delete_attachment(session, &upload.item, &upload.attachment);
        }
    }

    /// After a sync: an attachment to delete that the vault doesn't list
    /// anymore is gone, and forgotten. One it still lists stays hidden, and
    /// is tried again before the next sync.
    pub(super) fn forget_deleted(&mut self) {
        let Some(vault) = &self.vault else {
            return;
        };
        self.uploads.retain(|u| {
            !u.delete
                || vault
                    .item(&u.item)
                    .is_some_and(|item| item.attachments.iter().any(|a| a.id == u.attachment))
        });
    }

    /// Signing out: nothing of this account's is asked of a server again.
    pub(super) fn forget_uploads(&mut self) {
        self.uploads.clear();
    }

    /// Whether an attachment is one the item doesn't show: its file is on
    /// its way, or it's to be deleted on the server.
    pub fn hides_attachment(&self, attachment_id: &str) -> bool {
        self.uploads.iter().any(|u| u.attachment == attachment_id)
    }

    /// Attaches the file at `path` to an item, start to finish, holding
    /// the app throughout (tests and tools; the bridge uses `attach`, which
    /// lets go of the app while the file travels).
    pub fn attach(&mut self, id: &str, path: &Path, name: &str) -> Result<Attached> {
        attach(&mut |call| call(self), id, path, name, &Progress::new())
    }

    /// Deletes one of an item's attachments, then syncs; answers whether
    /// the sync came through. Once the server has deleted it, it's deleted:
    /// the item stops showing it even if the sync after doesn't come
    /// through.
    pub fn delete_attachment(&mut self, id: &str, attachment_id: &str) -> Result<bool> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let item = vault.item(id).ok_or(Error::Damaged("no such item"))?;
        may_change(vault, item, self.may_use(item))?;
        let listed = item.attachments.iter().any(|a| a.id == attachment_id);
        if !listed || self.hides_attachment(attachment_id) {
            return Err(Error::Damaged("no such attachment"));
        }
        let (Some(server), Some(settings), Some(session)) =
            (&self.server, &self.settings, self.session.as_mut())
        else {
            return Err(Error::SignedOut);
        };
        match server.delete_attachment(session, id, attachment_id) {
            Err(Error::SignedOut) => {
                self.session = None;
                forget_refresh_token(&*self.secrets, &self.store, settings);
                return Err(Error::SignedOut);
            }
            other => other?,
        }
        keep_refresh_token(&*self.secrets, &self.store, settings, session);
        // Hidden from here on; the sync that no longer lists it forgets it.
        self.uploads.push(Unfinished {
            item: id.to_owned(),
            attachment: attachment_id.to_owned(),
            delete: true,
        });
        Ok(self.sync().is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pb-attach-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_is_read_whole_with_its_progress_said() {
        let dir = new_dir();
        let path = dir.join("codes.txt");
        let contents: Vec<u8> = (0..3_000_000u32).map(|i| (i % 253) as u8).collect();
        std::fs::write(&path, &contents).unwrap();
        let progress = Progress::new();
        progress.begin();
        let file = read_file(&path, &progress).unwrap();
        assert_eq!(file.len(), contents.len());
        assert_eq!(progress.phase(), Some(Phase::Reading));
        assert_eq!(progress.amounts(), (3_000_000, 3_000_000));
        // It opens again as it was.
        let key = crate::crypto::SymmetricKey::generate();
        let opened = crate::crypto::decrypt_file(file.encrypt(&key), &key).unwrap();
        assert_eq!(opened.as_slice(), contents.as_slice());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_a_missing_file_and_a_canceled_read_are_refused() {
        let dir = new_dir();
        let progress = Progress::new();
        assert_eq!(
            read_file(&dir, &progress).err(),
            Some(Error::Refused("that isn't a file PassBundle can attach"))
        );
        assert!(matches!(
            read_file(&dir.join("gone.txt"), &progress),
            Err(Error::Refused(why)) if why.starts_with("PassBundle couldn't read that file")
        ));
        let path = dir.join("codes.txt");
        std::fs::write(&path, b"backup codes").unwrap();
        progress.cancel();
        assert_eq!(read_file(&path, &progress).err(), Some(Error::Canceled));
        // A new upload isn't canceled by the last one's Cancel.
        progress.begin();
        assert!(read_file(&path, &progress).is_ok());
        progress.end();
        assert_eq!(progress.phase(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A file over the cap is refused by its length alone: a sparse file
    /// that large takes no room, and none of it is read.
    #[test]
    fn a_file_over_the_cap_is_refused_before_any_of_it_is_read() {
        let dir = new_dir();
        let path = dir.join("large.bin");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_FILE + 1).unwrap();
        drop(file);
        let progress = Progress::new();
        progress.begin();
        assert_eq!(read_file(&path, &progress).err(), Some(too_large()));
        // Nothing was read: the phase never got an amount.
        assert_eq!(progress.amounts(), (0, 0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_cap_is_under_what_the_server_and_a_download_take() {
        let encrypted = crate::crypto::encrypted_file_len(MAX_FILE);
        // Vaultwarden refuses more than 525 MiB.
        assert!(encrypted <= 525 * 1024 * 1024);
        // And the apps download up to 501 MiB (server.rs).
        assert!(encrypted <= 501 * 1024 * 1024);
    }
}
