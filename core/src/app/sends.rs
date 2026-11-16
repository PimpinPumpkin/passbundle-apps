//! The account's Sends (DESIGN.md, "Send in the apps"; send.rs for the
//! format): listed from the vault the last sync opened, made, edited, their
//! password taken away, deleted, and their link built, always with this
//! account's keys and server only.
//!
//! Nothing half-made: a Send made or edited here counts only once the
//! server's answer opens, under the user key, to the seed and the name the
//! app sent. One that doesn't is deleted (a new one) and refused.

use zeroize::Zeroizing;

use super::App;
use crate::error::{Error, Result};
use crate::send::{self, Send, SendDraft, SendKind, SendPolicy};

/// A Send the server took, and whether the sync after it came through
/// (without it, it's listed after the next).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedSend {
    pub id: String,
    pub synced: bool,
}

pub(super) const DISABLED: Error = Error::Refused(
    "an organization you're in doesn't let you make or edit Sends. You can still delete them",
);
const NO_HIDING_EMAIL: Error = Error::Refused(
    "an organization you're in doesn't let Sends hide your email. Turn off \"Hide my email\" to save it",
);
pub(super) const NOT_AS_SENT: Error =
    Error::Damaged("a Send the server didn't keep as it was sent");

impl App {
    /// The organizations' Send policies, from the kept sync.
    pub fn send_policy(&self) -> SendPolicy {
        self.send_policy
    }

    /// The account's Sends, as the last sync opened them; none while locked.
    pub fn sends(&self) -> &[Send] {
        self.vault.as_ref().map_or(&[], |v| v.sends.as_slice())
    }

    /// Makes a text Send (`id` None) or edits one, then syncs. A password,
    /// if one is given, is hashed with the Send's seed for the server
    /// (`send::password_proof`) and wiped; an edit without one leaves the
    /// Send's password as it is.
    pub fn save_send(
        &mut self,
        id: Option<&str>,
        draft: &SendDraft,
        password: Option<Zeroizing<String>>,
    ) -> Result<SavedSend> {
        self.may_save_send(draft)?;
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let user = vault.user_key().clone();
        let editing = match id {
            Some(id) => {
                let send = vault.send(id).ok_or(Error::Damaged("no such Send"))?;
                if send.asks_for_email {
                    return Err(Error::Refused(
                        "this Send asks for an email address, which PassBundle doesn't edit. You can delete it",
                    ));
                }
                let seed = send.seed().ok_or(Error::Refused(
                    "this Send doesn't open here, so it can only be deleted",
                ))?;
                Some((id.to_owned(), send.kind, seed.clone()))
            }
            None => None,
        };
        let (kind, seed) = match &editing {
            Some((_, kind, seed)) => (*kind, seed.clone()),
            None => match draft.kind.unwrap_or(SendKind::Text) {
                SendKind::Text => (SendKind::Text, send::new_seed()),
                SendKind::File => {
                    return Err(Error::Refused("a file Send is made with its file"));
                }
            },
        };
        let proof = password
            .filter(|p| !p.is_empty())
            .map(|p| send::password_proof(&p, &*seed));
        let body = draft.body(kind, &*seed, &user, proof, None);
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        let answer = match &editing {
            Some((id, _, _)) => server.update_send(session, id, body)?,
            None => server.create_send(session, body)?,
        };
        let kept = Send::open(&answer, &user);
        if !kept.opens || !kept.has_seed(&*seed) || kept.name != draft.name {
            // A new one the server made otherwise goes; an edit stays as
            // the server has it, and the sync shows that.
            if editing.is_none() {
                let _ = server.delete_send(session, &answer.id);
            }
            let _ = self.sync();
            return Err(NOT_AS_SENT);
        }
        let id = answer.id;
        Ok(SavedSend {
            id,
            synced: self.synced_with(kept),
        })
    }

    /// What the policies and the draft must allow before a Send is made or
    /// edited (the server checks again).
    pub(super) fn may_save_send(&self, draft: &SendDraft) -> Result<()> {
        if self.vault.is_none() {
            return Err(Error::SignedOut);
        }
        if self.send_policy.disabled {
            return Err(DISABLED);
        }
        draft.check()?;
        if draft.hide_email && self.send_policy.no_hiding_email {
            return Err(NO_HIDING_EMAIL);
        }
        Ok(())
    }

    /// Syncs after the server took a Send (`kept`, as it answered, opened
    /// and checked): whether the sync came through. When it didn't, the
    /// Send is listed as the server answered it until the next one, so its
    /// link can be copied at once.
    pub(super) fn synced_with(&mut self, kept: Send) -> bool {
        if self.sync().is_ok() {
            return true;
        }
        if let Some(vault) = self.vault.as_mut() {
            vault.sends.retain(|s| s.id != kept.id);
            vault.sends.push(kept);
        }
        false
    }

    /// Takes a Send's password away, then syncs: whether the sync came
    /// through.
    pub fn remove_send_password(&mut self, id: &str) -> Result<bool> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        if self.send_policy.disabled {
            return Err(DISABLED);
        }
        vault.send(id).ok_or(Error::Damaged("no such Send"))?;
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.remove_send_password(session, id)?;
        Ok(self.sync().is_ok())
    }

    /// Deletes a Send (the policies allow it always), then syncs: whether
    /// the sync came through. Once the server has said yes, it isn't
    /// listed, even if the sync after doesn't come through.
    pub fn delete_send(&mut self, id: &str) -> Result<bool> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        vault.send(id).ok_or(Error::Damaged("no such Send"))?;
        let (Some(server), Some(session)) = (&self.server, self.session.as_mut()) else {
            return Err(Error::SignedOut);
        };
        server.delete_send(session, id)?;
        if let Some(vault) = self.vault.as_mut() {
            vault.sends.retain(|s| s.id != id);
        }
        Ok(self.sync().is_ok())
    }

    /// A Send's link, on this account's server (`send::link`): copied as a
    /// secret, never shown.
    pub fn send_link(&self, id: &str) -> Result<Zeroizing<String>> {
        let vault = self.vault.as_ref().ok_or(Error::SignedOut)?;
        let settings = self.settings.as_ref().ok_or(Error::SignedOut)?;
        vault
            .send(id)
            .ok_or(Error::Damaged("no such Send"))?
            .link(&settings.server)
    }
}
