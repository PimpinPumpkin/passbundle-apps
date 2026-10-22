//! Uploads an attachment the way upstream's apps do, for tests (tests/server.rs)
//! and the test account (examples/test_account.rs): the file encrypted with a
//! key of its own, and that key and the file's name under the user key
//! (docs/core.md, "Attachments"). Vaultwarden's two steps: say what's coming,
//! then send the file as a form upload. Answers the attachment's id.

use passbundle_core::crypto::{EncString, SymmetricKey};
use passbundle_core::server::{Server, Session};
use serde_json::json;

pub fn upload(
    server: &Server,
    url: &str,
    session: &mut Session,
    cipher_id: &str,
    name: &str,
    contents: &[u8],
) -> String {
    let user_key = session.user_key().clone();
    let file_key = SymmetricKey::generate();
    let EncString::Aes { iv, data, mac } = file_key.encrypt(contents) else {
        unreachable!()
    };
    let mut blob = vec![2u8];
    blob.extend_from_slice(&iv);
    blob.extend_from_slice(&mac);
    blob.extend_from_slice(&data);
    let file_name = user_key.encrypt_str(name).to_string();
    let started = server
        .raw(
            session,
            "POST",
            &format!("/api/ciphers/{cipher_id}/attachment/v2"),
            Some(json!({
                "key": user_key.encrypt(&file_key.to_bytes()).to_string(),
                "fileName": file_name,
                "fileSize": blob.len(),
                "adminRequest": false,
            })),
        )
        .expect("starting the upload");
    let attachment_id = started["attachmentId"].as_str().unwrap().to_owned();
    let boundary = "pb-test-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"data\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&blob);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    ureq::post(&format!(
        "{url}/api/ciphers/{cipher_id}/attachment/{attachment_id}"
    ))
    .header(
        "Authorization",
        &format!("Bearer {}", session.access_token.as_str()),
    )
    .header(
        "Content-Type",
        &format!("multipart/form-data; boundary={boundary}"),
    )
    .send(&body[..])
    .expect("uploading the file");
    attachment_id
}
