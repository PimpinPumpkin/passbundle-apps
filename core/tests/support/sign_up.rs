// Signing up as an app would, for the server tests (tests/server.rs) and
// test setup (examples/test_account.rs): the master key and login hash from
// the password, a new user key sealed with the stretched master key, and a
// new RSA key pair with its private key under the user key.

use base64::{engine::general_purpose::STANDARD, Engine};
use passbundle_core::crypto::{PrivateKey, SymmetricKey};
use passbundle_core::kdf::{Kdf, MasterKey};
use passbundle_core::server::Server;
use serde_json::json;

pub fn sign_up(server: &Server, email: &str, password: &str) {
    let master = MasterKey::derive(
        password,
        email,
        &Kdf::Pbkdf2 {
            iterations: 600_000,
        },
    )
    .unwrap();
    let user_key = SymmetricKey::generate();
    let private = PrivateKey::generate().unwrap();
    let token = server
        .raw_post(
            "/identity/accounts/register/send-verification-email",
            json!({ "email": email, "name": "Core test", "receiveMarketingEmails": false }),
        )
        .unwrap();
    server
        .raw_post(
            "/identity/accounts/register/finish",
            json!({
                "email": email,
                "emailVerificationToken": token,
                "masterPasswordHash": master.login_hash(password),
                "masterPasswordHint": null,
                "userSymmetricKey": master.stretch().encrypt(&user_key.to_bytes()).to_string(),
                "userAsymmetricKeys": {
                    "publicKey": STANDARD.encode(private.public_key_der()),
                    "encryptedPrivateKey": user_key.encrypt(&private.to_pkcs8_der()).to_string(),
                },
                "kdf": 0,
                "kdfIterations": 600000,
            }),
        )
        .unwrap();
}
