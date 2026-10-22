//! Making an account (docs/core.md, "Making an account"), as upstream's apps
//! make one, so it opens anywhere: the master key from the master password
//! and the email (PBKDF2-SHA256, 600,000 iterations, upstream's default), a
//! new user key sealed with the stretched master key, and a new RSA key pair
//! with its private key under the user key. The server gets the login hash,
//! never the password or a key that opens the vault.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use zeroize::Zeroizing;

use crate::crypto::{EncString, PrivateKey, SymmetricKey};
use crate::error::{Error, Result};
use crate::kdf::{normalize_email, Kdf, MasterKey};

/// The key derivation a new account uses: upstream's default for new accounts.
pub const KDF: Kdf = Kdf::Pbkdf2 {
    iterations: 600_000,
};

/// The shortest master password a new account takes, as upstream's apps.
pub const SHORTEST: usize = 12;

/// A new account, ready for the server: everything in it is safe to send.
pub struct NewAccount {
    pub email: String,
    pub name: Option<String>,
    pub hint: Option<String>,
    /// What signing in sends: the server's credential, wiped when dropped.
    pub login_hash: Zeroizing<String>,
    pub kdf: Kdf,
    /// The user key, sealed with the stretched master key.
    pub user_key: EncString,
    /// The public key, DER (SubjectPublicKeyInfo).
    pub public_key: Vec<u8>,
    /// The private key, PKCS#8 DER under the user key.
    pub private_key: EncString,
}

impl NewAccount {
    /// A new account's keys for this email and master password, all new.
    /// Slow: the master key's 600,000 iterations, and an RSA key pair.
    pub fn make(
        email: &str,
        name: Option<&str>,
        password: &str,
        hint: Option<&str>,
    ) -> Result<NewAccount> {
        check(email, password, hint)?;
        let user_key = SymmetricKey::generate();
        let private = PrivateKey::generate()?;
        Self::with(email, name, password, hint, &KDF, &user_key, &private, None)
    }

    /// The same from given keys, and IVs for the two sealed values (the
    /// vectors' fixed ones; None picks new ones).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with(
        email: &str,
        name: Option<&str>,
        password: &str,
        hint: Option<&str>,
        kdf: &Kdf,
        user_key: &SymmetricKey,
        private: &PrivateKey,
        ivs: Option<([u8; 16], [u8; 16])>,
    ) -> Result<NewAccount> {
        let master = MasterKey::derive(password, email, kdf)?;
        let stretched = master.stretch();
        let user_bytes = user_key.to_bytes();
        let private_der = private.to_pkcs8_der();
        let (sealed_user_key, sealed_private_key) = match ivs {
            Some((user_iv, private_iv)) => (
                stretched.encrypt_with_iv(&user_bytes, user_iv),
                user_key.encrypt_with_iv(&private_der, private_iv),
            ),
            None => (
                stretched.encrypt(&user_bytes),
                user_key.encrypt(&private_der),
            ),
        };
        let text = |t: Option<&str>| {
            t.map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_owned)
        };
        Ok(NewAccount {
            email: normalize_email(email),
            name: text(name),
            hint: text(hint),
            login_hash: Zeroizing::new(master.login_hash(password)),
            kdf: *kdf,
            user_key: sealed_user_key,
            public_key: private.public_key_der(),
            private_key: sealed_private_key,
        })
    }

    /// What upstream's `register/finish` takes, with the server's token for
    /// this email.
    pub fn finish_body(&self, token: &str) -> Value {
        let (kdf, iterations, memory, parallelism) = match self.kdf {
            Kdf::Pbkdf2 { iterations } => (0, iterations, None, None),
            Kdf::Argon2id {
                iterations,
                memory_mib,
                parallelism,
            } => (1, iterations, Some(memory_mib), Some(parallelism)),
        };
        json!({
            "email": self.email,
            "emailVerificationToken": token,
            "masterPasswordHash": self.login_hash.as_str(),
            "masterPasswordHint": self.hint,
            "userSymmetricKey": self.user_key.to_string(),
            "userAsymmetricKeys": {
                "publicKey": STANDARD.encode(&self.public_key),
                "encryptedPrivateKey": self.private_key.to_string(),
            },
            "kdf": kdf,
            "kdfIterations": iterations,
            "kdfMemory": memory,
            "kdfParallelism": parallelism,
        })
    }
}

/// What a new account takes: an email, a master password of 12 characters
/// or more, and a hint that isn't the master password.
fn check(email: &str, password: &str, hint: Option<&str>) -> Result<()> {
    let email = normalize_email(email);
    let (local, domain) = email.split_once('@').unwrap_or_default();
    if local.is_empty() || domain.is_empty() || email.contains(char::is_whitespace) {
        return Err(Error::Refused("that isn't an email address"));
    }
    if password.chars().count() < SHORTEST {
        return Err(Error::Refused(
            "a master password needs at least 12 characters",
        ));
    }
    // Case and spaces aside: "Correct Horse Battery" gives away "correct horse battery".
    let said = |t: &str| t.trim().to_lowercase();
    if hint.is_some_and(|h| said(h).contains(&said(password))) {
        return Err(Error::Refused(
            "the hint can't give away the master password",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_account_needs_an_email_a_long_password_and_a_hint_that_isnt_it() {
        assert!(check("ana@example.test", "twelve chars", None).is_ok());
        assert!(check("ana@example.test", "eleven char", None).is_err());
        assert!(check("not an email", "long enough password", None).is_err());
        assert!(check("@example.test", "long enough password", None).is_err());
        assert!(check(
            "ana@example.test",
            "long enough password",
            Some("long enough password")
        )
        .is_err());
        assert!(check(
            "ana@example.test",
            "long enough password",
            Some("it's long enough password!")
        )
        .is_err());
        assert!(check(
            "ana@example.test",
            "Long Enough Password",
            Some("long enough PASSWORD ")
        )
        .is_err());
        assert!(check(
            "ana@example.test",
            "long enough password",
            Some("the usual")
        )
        .is_ok());
    }

    #[test]
    fn a_made_account_opens_with_its_own_password() {
        let account = NewAccount::make(
            " Ana@Example.TEST ",
            Some(" Ana "),
            "correct horse battery staple",
            Some(""),
        )
        .unwrap();
        assert_eq!(account.email, "ana@example.test");
        assert_eq!(account.name.as_deref(), Some("Ana"));
        assert_eq!(account.hint, None);
        let master =
            MasterKey::derive("correct horse battery staple", "ana@example.test", &KDF).unwrap();
        assert_eq!(
            *account.login_hash,
            master.login_hash("correct horse battery staple")
        );
        let user_key = account.user_key.decrypt_key(&master.stretch()).unwrap();
        let private =
            PrivateKey::from_pkcs8_der(&account.private_key.decrypt(&user_key).unwrap()).unwrap();
        assert_eq!(private.public_key_der(), account.public_key);
        let body = account.finish_body("token");
        assert_eq!(body["kdf"], 0);
        assert_eq!(body["kdfIterations"], 600_000);
        assert!(body["kdfMemory"].is_null());
        assert_eq!(body["emailVerificationToken"], "token");
        assert!(body["masterPasswordHint"].is_null());
    }
}
