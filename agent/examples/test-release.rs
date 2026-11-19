//! A release for the update task's end-to-end test (the meta repo's tier 2
//! on Windows, `installer/windows/tests/Test-AgentUpdate.ps1`), signed with a
//! throwaway key that only that test's build of the agent has compiled in
//! (the test-update feature). Never for a real release: those are signed
//! offline with minisign (README, "The update key").
//!
//!   cargo run --example test-release -- key <dir>
//!       a throwaway key: <dir>/key.pub, the public key on one line, for
//!       PB_TEST_UPDATE_KEY, and <dir>/key.secret
//!   cargo run --example test-release -- sign <dir> <zip> <version> <site> <out>
//!       <out>/windows-x86_64.json, naming <site>/<the zip's name> at
//!       <version> with its size and SHA-256, and its signature with
//!       <dir>/key.secret, <out>/windows-x86_64.json.minisig

use std::{error::Error, fs, io, path::Path, process::ExitCode};

use sha2::{Digest, Sha256};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let done = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["key", dir] => key(Path::new(dir)),
        ["sign", dir, zip, version, site, out] => sign(
            Path::new(dir),
            Path::new(zip),
            version,
            site,
            Path::new(out),
        ),
        _ => {
            eprintln!("usage: test-release key <dir> | sign <dir> <zip> <version> <site> <out>");
            return ExitCode::from(2);
        }
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn key(dir: &Path) -> Result<(), Box<dyn Error>> {
    let pair = minisign::KeyPair::generate_unencrypted_keypair()?;
    fs::create_dir_all(dir)?;
    fs::write(dir.join("key.pub"), pair.pk.to_base64())?;
    fs::write(dir.join("key.secret"), pair.sk.to_box(None)?.into_string())?;
    Ok(())
}

fn sign(
    dir: &Path,
    zip: &Path,
    version: &str,
    site: &str,
    out: &Path,
) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(zip)?;
    let name = zip
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the zip has no name")?;
    let sha256: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let manifest = serde_json::json!({
        "product": "passbundle-agent",
        "version": version,
        "target": "windows-x86_64",
        "url": format!("{}/{name}", site.trim_end_matches('/')),
        "size": bytes.len(),
        "sha256": sha256,
    })
    .to_string();
    let secret = minisign::SecretKeyBox::from_string(&fs::read_to_string(dir.join("key.secret"))?)?
        .into_unencrypted_secret_key()?;
    let signature = minisign::sign(
        None,
        &secret,
        io::Cursor::new(manifest.as_bytes()),
        None,
        None,
    )?;
    fs::create_dir_all(out)?;
    fs::write(out.join("windows-x86_64.json"), &manifest)?;
    fs::write(
        out.join("windows-x86_64.json.minisig"),
        signature.to_string(),
    )?;
    println!("{manifest}");
    Ok(())
}
