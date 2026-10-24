//! The Windows and Linux updater's manifests against the meta repo's test
//! vectors (docs/vectors/desktop-update-v1.json, copied into tests/vectors;
//! DESIGN.md, "Desktop updates"): a fixed key, manifests signed with it as
//! the release workflows sign them, and changes each that must be refused.
#![cfg(not(any(target_os = "android", target_os = "ios")))]

use passbundle_core::updates::manifest::{self, Expect, Refused, PREFIX};
use serde_json::Value;

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors/desktop-update-v1.json")).unwrap()
}

fn key(value: &Value) -> [u8; 32] {
    manifest::sha256_of(value.as_str().unwrap()).expect("32 bytes of hex")
}

fn code(refused: &Refused) -> &'static str {
    match refused {
        Refused::Signature => "signature",
        Refused::Tag { .. } => "tag",
        Refused::Version { .. } => "version",
        Refused::NotNewer { .. } => "notNewer",
        Refused::TooLarge(_) => "tooLarge",
        Refused::NoFile { .. } => "noFile",
        Refused::Name(_) => "name",
        Refused::App => "app",
        Refused::Malformed(_) => "malformed",
    }
}

#[test]
fn the_prefix_is_the_vectors() {
    assert_eq!(vectors()["prefix"].as_str().unwrap().as_bytes(), PREFIX);
}

#[test]
fn the_fixed_keys_public_halves_are_the_vectors() {
    use ed25519_dalek::SigningKey;
    let v = vectors();
    for name in ["key", "otherKey"] {
        let seed = key(&v[name]["seed"]);
        assert_eq!(
            SigningKey::from_bytes(&seed).verifying_key().to_bytes(),
            key(&v[name]["public"]),
            "{name}"
        );
    }
}

#[test]
fn each_case_is_taken_or_refused_as_the_vectors_say() {
    let v = vectors();
    let keys = [key(&v["key"]["public"])];
    for case in v["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let expect = &case["expect"];
        let expect = Expect {
            tag: expect["tag"].as_str().unwrap(),
            os: expect["os"].as_str().unwrap(),
            platform: expect["platform"].as_str().unwrap(),
            running_build: expect["runningBuild"].as_u64().unwrap() as u32,
        };
        let got = manifest::read(
            case["manifest"].as_str().unwrap().as_bytes(),
            case["signature"].as_str().unwrap(),
            &keys,
            &expect,
        );
        match (&got, case["refused"].as_str()) {
            (Ok(release), None) => {
                let file = &case["file"];
                assert_eq!(release.file.name, file["name"].as_str().unwrap(), "{name}");
                assert_eq!(release.file.size, file["size"].as_u64().unwrap(), "{name}");
                assert_eq!(
                    manifest::hex(&release.file.sha256),
                    file["sha256"].as_str().unwrap(),
                    "{name}"
                );
                assert_eq!(release.tag, expect.tag, "{name}");
            }
            (Err(refused), Some(why)) => assert_eq!(code(refused), why, "{name}"),
            _ => panic!("{name}: {got:?}"),
        }
    }
}

#[test]
fn only_the_pinned_key_counts() {
    // Every case signed with the fixed key, checked with the other key alone,
    // or with no key: none is taken.
    let v = vectors();
    let other = [key(&v["otherKey"]["public"])];
    for case in v["cases"].as_array().unwrap() {
        let manifest = case["manifest"].as_str().unwrap().as_bytes();
        let signature = case["signature"].as_str().unwrap();
        let name = case["name"].as_str().unwrap();
        if name == "signed with another key" {
            assert_eq!(manifest::verify(manifest, signature, &other), Ok(()));
            continue;
        }
        assert_eq!(
            manifest::verify(manifest, signature, &other),
            Err(Refused::Signature),
            "{name}"
        );
        assert_eq!(
            manifest::verify(manifest, signature, &[]),
            Err(Refused::Signature),
            "{name}"
        );
    }
}
