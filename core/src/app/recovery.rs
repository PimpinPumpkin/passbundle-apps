//! Enrolling in account recovery at unlock (DESIGN.md in the meta repo,
//! "Enrolling at unlock", "Our apps"): once per unlock or sign-in, after the
//! app's own sync, every organization the person is due in (recovery.rs),
//! only to the pinned organization key, and silently. Which memberships, the
//! pin, the check, and the wrap are `crate::recovery`'s.

use std::io::Write;

use super::App;
use crate::error::Error;
use crate::recovery::{self, Due, Outcome, Tried};
use crate::server::{Server, Session};
use crate::vault::Vault;

impl App {
    /// Enrolls the person in account recovery where they're due, once per
    /// unlock: going by what this device has (the kept sync, which the
    /// unlock's own sync brought up to date), with no sync of its own
    /// before, and one after an enrollment goes through so the
    /// organization's options show it.
    ///
    /// `pin` is the organization key fingerprint an administrator set
    /// (managed.rs, or a phone's managed configuration). Without one that's
    /// a pin (`recovery::pin`), nothing is sent, not even a request for the
    /// key. With one, each organization's key from the server is taken only
    /// if it's the pinned one, and only then is the user key wrapped to it
    /// and sent. Nothing is shown; what didn't go through is said on
    /// standard error, never with a key or a fingerprint, and waits for the
    /// next unlock, as everything here does: a second call this unlock does
    /// nothing. Locked, or beside the app (iOS's AutoFill extension),
    /// nothing, and the next unlock still has its try.
    pub fn recovery_at_unlock(&mut self, pin: Option<&str>) -> Vec<Tried> {
        if self.beside || self.recovery_tried {
            return Vec::new();
        }
        let Some(vault) = self.vault.as_ref() else {
            return Vec::new();
        };
        // Marked first, so nothing makes this unlock try twice.
        self.recovery_tried = true;
        let due = self
            .store
            .sync()
            .and_then(|text| serde_json::from_str(&text).ok())
            .map(|sync| recovery::due(&sync))
            .unwrap_or_default();
        if due.is_empty() {
            return Vec::new();
        }
        let tried: Vec<Tried> = match (recovery::pin(pin), &self.server, self.session.as_mut()) {
            // Not even the organization's key is asked for.
            (None, _, _) => due.into_iter().map(|d| tried(d, Outcome::NoPin)).collect(),
            (Some(pin), Some(server), Some(session)) => due
                .into_iter()
                .map(|d| {
                    let outcome = enroll(server, session, vault, &pin, &d.organization_id);
                    tried(d, outcome)
                })
                .collect(),
            _ => due
                .into_iter()
                .map(|d| tried(d, Outcome::Failed(Error::SignedOut.to_string())))
                .collect(),
        };
        for t in &tried {
            if let Some(why) = t.outcome.why() {
                // The organization's name as the server gave it, on one line.
                let name: String = t.name.chars().filter(|c| !c.is_control()).collect();
                let _ = writeln!(
                    std::io::stderr(),
                    "PassBundle: account recovery at unlock in {name}: {why}"
                );
            }
        }
        if tried.iter().any(|t| t.outcome == Outcome::Enrolled) {
            // So the organization's options show it, as after enrolling by
            // hand; offline by now, the next sync brings it.
            let _ = self.sync();
        }
        tried
    }
}

fn tried(due: Due, outcome: Outcome) -> Tried {
    Tried {
        organization_id: due.organization_id,
        name: due.name,
        outcome,
    }
}

/// One organization: its public key from the server, taken only if it's
/// the pinned one, then the user key wrapped to those same bytes and sent.
fn enroll(
    server: &Server,
    session: &mut Session,
    vault: &Vault,
    pin: &str,
    organization_id: &str,
) -> Outcome {
    let answer = match server.organization_keys(session, organization_id) {
        Ok(answer) => answer,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    // Nothing is encrypted, and nothing sent.
    let Some(key) = recovery::pinned_key(&answer, pin) else {
        return Outcome::NotThePinnedKey;
    };
    // The key checked is the key encrypted to.
    let sent = recovery::wrap(&key, vault.user_key()).and_then(|wrapped| {
        server.enroll_in_recovery(session, organization_id, &vault.user_id, &wrapped)
    });
    match sent {
        Ok(()) => Outcome::Enrolled,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}
