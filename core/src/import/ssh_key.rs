//! An SSH private key from another app, as the vault keeps one: OpenSSH's
//! own format (`-----BEGIN OPENSSH PRIVATE KEY-----`, OpenSSH's PROTOCOL.key,
//! unencrypted), its public key as an `authorized_keys` line without a
//! comment, and its SHA-256 fingerprint as `ssh-keygen -l` prints it. The web
//! vault turns 1Password's keys into these with upstream's SDK, which the
//! apps never link (docs/core.md); this does the same from the formats'
//! public definitions.
//!
//! - **What it reads:** PKCS#8 (`-----BEGIN PRIVATE KEY-----`, RFC 5208 and
//!   RFC 5958, as 1Password keeps its keys) for Ed25519 (RFC 8410) and RSA;
//!   PKCS#1 (`-----BEGIN RSA PRIVATE KEY-----`, RFC 8017); and OpenSSH's own,
//!   kept as it is. Anything else (another kind of key, or one under a
//!   password) isn't read, and the caller leaves the item out and says so.
//! - **Checked:** an Ed25519 key's public half is worked out from its private
//!   one (and must match the file's, when the file has it); an RSA key must
//!   be a whole one (the `rsa` crate checks its numbers), with two primes, as
//!   OpenSSH's format holds.

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::der::asn1::OctetStringRef;
use rsa::pkcs8::der::Decode;
use rsa::pkcs8::{DecodePrivateKey, ObjectIdentifier, PrivateKeyInfo};
use rsa::traits::{PrivateKeyParts, PublicKeyParts};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// id-Ed25519 (RFC 8410).
const ED25519: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.101.112");
/// rsaEncryption (RFC 8017).
const RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");

/// An SSH key as the vault keeps it.
pub(super) struct Converted {
    pub private_key: Zeroizing<String>,
    pub public_key: String,
    pub fingerprint: String,
}

/// The key in `pem`, in OpenSSH's format, with its public key and
/// fingerprint; None for one this doesn't read.
pub(super) fn convert(pem: &str) -> Option<Converted> {
    if let Some(der) = pem_body(pem, "PRIVATE KEY") {
        let info = PrivateKeyInfo::from_der(&der).ok()?;
        if info.algorithm.oid == ED25519 {
            let seed = OctetStringRef::from_der(info.private_key).ok()?;
            let seed: [u8; 32] = seed.as_bytes().try_into().ok()?;
            return ed25519(Zeroizing::new(seed), info.public_key);
        }
        if info.algorithm.oid == RSA {
            return rsa_key(&rsa::RsaPrivateKey::from_pkcs8_der(&der).ok()?);
        }
        return None;
    }
    if let Some(der) = pem_body(pem, "RSA PRIVATE KEY") {
        return rsa_key(&rsa::RsaPrivateKey::from_pkcs1_der(&der).ok()?);
    }
    if let Some(body) = pem_body(pem, "OPENSSH PRIVATE KEY") {
        return openssh(pem, &body);
    }
    None
}

/// What's between a PEM block's lines, decoded; None if there's no such
/// block (`-----BEGIN <label>-----`) or what's in it isn't base64.
fn pem_body(pem: &str, label: &str) -> Option<Zeroizing<Vec<u8>>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let start = pem.find(&begin)? + begin.len();
    let stop = start + pem[start..].find(&end)?;
    let text = Zeroizing::new(
        pem[start..stop]
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect::<String>(),
    );
    STANDARD.decode(text.as_bytes()).ok().map(Zeroizing::new)
}

/// An SSH string: its length, then its bytes.
fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// An SSH mpint of a number that isn't negative, from its big-endian bytes:
/// no leading zeros, and one zero byte in front when the top bit is set.
fn put_mpint(out: &mut Vec<u8>, big_endian: &[u8]) {
    let first = big_endian
        .iter()
        .position(|b| *b != 0)
        .unwrap_or(big_endian.len());
    let digits = &big_endian[first..];
    let pad = digits.first().is_some_and(|b| b & 0x80 != 0);
    let len = digits.len() + usize::from(pad);
    out.extend_from_slice(&(len as u32).to_be_bytes());
    if pad {
        out.push(0);
    }
    out.extend_from_slice(digits);
}

/// An Ed25519 key from its 32-byte seed.
fn ed25519(seed: Zeroizing<[u8; 32]>, said_public: Option<&[u8]>) -> Option<Converted> {
    let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
    let public = signing.verifying_key().to_bytes();
    if said_public.is_some_and(|said| said != public) {
        return None;
    }
    let mut blob = Vec::new();
    put_string(&mut blob, b"ssh-ed25519");
    put_string(&mut blob, &public);
    let mut private = Zeroizing::new(Vec::new());
    put_string(&mut private, b"ssh-ed25519");
    put_string(&mut private, &public);
    // OpenSSH keeps the seed, then the public key, as its private key.
    let mut both = Zeroizing::new(Vec::with_capacity(64));
    both.extend_from_slice(&*seed);
    both.extend_from_slice(&public);
    put_string(&mut private, &both);
    Some(Converted {
        private_key: container(&blob, &private),
        public_key: format!("ssh-ed25519 {}", STANDARD.encode(&blob)),
        fingerprint: fingerprint(&blob),
    })
}

/// An RSA key, which OpenSSH keeps as n, e, d, q⁻¹ mod p, p, and q.
fn rsa_key(key: &rsa::RsaPrivateKey) -> Option<Converted> {
    let [p, q] = key.primes() else {
        return None;
    };
    let iqmp = key.crt_coefficient()?;
    let mut blob = Vec::new();
    put_string(&mut blob, b"ssh-rsa");
    put_mpint(&mut blob, &key.e().to_bytes_be());
    put_mpint(&mut blob, &key.n().to_bytes_be());
    let mut private = Zeroizing::new(Vec::new());
    put_string(&mut private, b"ssh-rsa");
    put_mpint(&mut private, &key.n().to_bytes_be());
    put_mpint(&mut private, &key.e().to_bytes_be());
    put_mpint(&mut private, &Zeroizing::new(key.d().to_bytes_be()));
    put_mpint(&mut private, &Zeroizing::new(iqmp.to_bytes_be()));
    put_mpint(&mut private, &Zeroizing::new(p.to_bytes_be()));
    put_mpint(&mut private, &Zeroizing::new(q.to_bytes_be()));
    Some(Converted {
        private_key: container(&blob, &private),
        public_key: format!("ssh-rsa {}", STANDARD.encode(&blob)),
        fingerprint: fingerprint(&blob),
    })
}

/// "SHA256:" and the public key's SHA-256 in base64 without padding.
fn fingerprint(blob: &[u8]) -> String {
    format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(blob)))
}

/// OpenSSH's file for one unencrypted key (PROTOCOL.key): the public key,
/// then the private part (its check number twice, the key, an empty
/// comment, and padding to 8 bytes), in base64 lines of 70 characters.
fn container(blob: &[u8], key: &[u8]) -> Zeroizing<String> {
    // The check number is for telling a wrong passphrase, and there's none:
    // taken from the public key, so the same key comes out the same.
    let check = &Sha256::digest(blob)[..4];
    let mut private = Zeroizing::new(Vec::with_capacity(key.len() + 24));
    private.extend_from_slice(check);
    private.extend_from_slice(check);
    private.extend_from_slice(key);
    put_string(&mut private, b"");
    let mut pad = 1u8;
    while private.len() % 8 != 0 {
        private.push(pad);
        pad += 1;
    }
    let mut file = Zeroizing::new(b"openssh-key-v1\0".to_vec());
    put_string(&mut file, b"none");
    put_string(&mut file, b"none");
    put_string(&mut file, b"");
    file.extend_from_slice(&1u32.to_be_bytes());
    put_string(&mut file, blob);
    put_string(&mut file, &private);
    let encoded = Zeroizing::new(STANDARD.encode(&*file));
    let mut out = Zeroizing::new(String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n"));
    for line in encoded.as_bytes().chunks(70) {
        // Base64 is ASCII.
        out.push_str(std::str::from_utf8(line).unwrap_or_default());
        out.push('\n');
    }
    out.push_str("-----END OPENSSH PRIVATE KEY-----\n");
    out
}

/// Reads an SSH string at `at`, moving past it.
fn take_string<'a>(bytes: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    let len = u32::from_be_bytes(bytes.get(*at..*at + 4)?.try_into().ok()?) as usize;
    let start = *at + 4;
    let value = bytes.get(start..start.checked_add(len)?)?;
    *at = start + len;
    Some(value)
}

/// A key in OpenSSH's own format, kept as it is: its public key from the
/// file, which must hold one key with no passphrase.
fn openssh(pem: &str, body: &[u8]) -> Option<Converted> {
    let mut at = b"openssh-key-v1\0".len();
    if !body.starts_with(b"openssh-key-v1\0") {
        return None;
    }
    let cipher = take_string(body, &mut at)?;
    let kdf = take_string(body, &mut at)?;
    take_string(body, &mut at)?;
    let count = u32::from_be_bytes(body.get(at..at + 4)?.try_into().ok()?);
    at += 4;
    if cipher != b"none" || kdf != b"none" || count != 1 {
        return None;
    }
    let blob = take_string(body, &mut at)?;
    let mut inside = 0;
    let kind = std::str::from_utf8(take_string(blob, &mut inside)?).ok()?;
    if kind.is_empty() || !kind.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    let pem = pem.trim();
    Some(Converted {
        private_key: Zeroizing::new(format!("{pem}\n")),
        public_key: format!("{kind} {}", STANDARD.encode(blob)),
        fingerprint: fingerprint(blob),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::pkcs8::EncodePrivateKey;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    fn pem(label: &str, der: &[u8]) -> String {
        let body = STANDARD.encode(der);
        let lines: Vec<&str> = body
            .as_bytes()
            .chunks(64)
            .map(|l| std::str::from_utf8(l).unwrap())
            .collect();
        format!(
            "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
            lines.join("\n")
        )
    }

    /// Reads OpenSSH's file back: the public key, and the private part's
    /// check numbers and key, as `ssh-keygen` would.
    fn read_back(file: &str) -> (Vec<u8>, Vec<u8>) {
        let body = pem_body(file, "OPENSSH PRIVATE KEY").unwrap();
        let mut at = 15;
        assert_eq!(take_string(&body, &mut at).unwrap(), b"none");
        assert_eq!(take_string(&body, &mut at).unwrap(), b"none");
        assert_eq!(take_string(&body, &mut at).unwrap(), b"");
        assert_eq!(&body[at..at + 4], &1u32.to_be_bytes());
        at += 4;
        let blob = take_string(&body, &mut at).unwrap().to_vec();
        let private = take_string(&body, &mut at).unwrap().to_vec();
        assert_eq!(at, body.len());
        assert_eq!(private.len() % 8, 0);
        assert_eq!(private[..4], private[4..8]);
        (blob, private)
    }

    // RFC 8032's first test vector (7.1, TEST 1), as PKCS#8 with and
    // without its public key. OpenSSH's `ssh-keygen` reads the file this
    // makes, prints this public key and fingerprint for it (`-y`, `-l`), and
    // signs with it (`-Y sign`, checked with `-Y verify`).
    const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn ed25519_pkcs8(with_public: bool) -> Vec<u8> {
        let mut der = hex(if with_public {
            "3051020101300506032b657004220420"
        } else {
            "302e020100300506032b657004220420"
        });
        der.extend(hex(SEED));
        if with_public {
            der.extend(hex("812100"));
            der.extend(hex(PUBLIC));
        }
        der
    }

    #[test]
    fn an_ed25519_key_in_pkcs8_comes_out_as_opensshs() {
        for with_public in [false, true] {
            let key = convert(&pem("PRIVATE KEY", &ed25519_pkcs8(with_public))).unwrap();
            assert_eq!(
                key.public_key,
                "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINdamAGCsQq31Uv+08lkBzoO4XLz2qYjJa8CGmj3B1Ea"
            );
            assert_eq!(
                key.fingerprint,
                "SHA256:bbXpuKG6zhzdmnxq256TlqzFBzRl2f6OOg722cYNbU8"
            );
            assert!(key
                .private_key
                .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB"));
            assert!(key
                .private_key
                .ends_with("\n-----END OPENSSH PRIVATE KEY-----\n"));
            assert!(key
                .private_key
                .lines()
                .all(|l| l.len() <= 70 || l.starts_with("-----")));
            let (blob, private) = read_back(&key.private_key);
            assert_eq!(
                format!("ssh-ed25519 {}", STANDARD.encode(&blob)),
                key.public_key
            );
            let mut at = 8;
            assert_eq!(take_string(&private, &mut at).unwrap(), b"ssh-ed25519");
            assert_eq!(take_string(&private, &mut at).unwrap(), hex(PUBLIC));
            let both = take_string(&private, &mut at).unwrap();
            assert_eq!(both, [hex(SEED), hex(PUBLIC)].concat());
            assert_eq!(take_string(&private, &mut at).unwrap(), b"");
            // Then the padding, 1, 2, 3, ...
            for (i, b) in private[at..].iter().enumerate() {
                assert_eq!(*b as usize, i + 1);
            }
        }
    }

    #[test]
    fn a_key_whose_public_half_isnt_its_own_isnt_read() {
        let mut der = ed25519_pkcs8(true);
        let last = der.len() - 1;
        der[last] ^= 1;
        assert!(convert(&pem("PRIVATE KEY", &der)).is_none());
    }

    #[test]
    fn an_rsa_key_in_pkcs8_or_pkcs1_comes_out_as_opensshs() {
        use rand::SeedableRng;
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pkcs8 = pem("PRIVATE KEY", key.to_pkcs8_der().unwrap().as_bytes());
        let pkcs1 = pem("RSA PRIVATE KEY", key.to_pkcs1_der().unwrap().as_bytes());
        let from8 = convert(&pkcs8).unwrap();
        let from1 = convert(&pkcs1).unwrap();
        assert_eq!(from8.private_key.as_str(), from1.private_key.as_str());
        assert_eq!(from8.public_key, from1.public_key);

        // The public key: "ssh-rsa", e, then n, as mpints.
        let mut want = Vec::new();
        put_string(&mut want, b"ssh-rsa");
        put_mpint(&mut want, &key.e().to_bytes_be());
        put_mpint(&mut want, &key.n().to_bytes_be());
        assert_eq!(
            from8.public_key,
            format!("ssh-rsa {}", STANDARD.encode(&want))
        );
        assert_eq!(from8.fingerprint, fingerprint(&want));
        let (blob, private) = read_back(&from8.private_key);
        assert_eq!(blob, want);
        let mut at = 8;
        assert_eq!(take_string(&private, &mut at).unwrap(), b"ssh-rsa");
        let n = take_string(&private, &mut at).unwrap();
        // 2048 bits, its top bit set: a zero byte in front.
        assert_eq!(n.len(), 257);
        assert_eq!(n[0], 0);
        assert_eq!(&n[1..], key.n().to_bytes_be());
        let numbers: Vec<Vec<u8>> = (0..5)
            .map(|_| take_string(&private, &mut at).unwrap().to_vec())
            .collect();
        let strip = |v: &[u8]| {
            v.iter()
                .skip_while(|b| **b == 0)
                .copied()
                .collect::<Vec<u8>>()
        };
        assert_eq!(strip(&numbers[0]), key.e().to_bytes_be());
        assert_eq!(strip(&numbers[1]), key.d().to_bytes_be());
        assert_eq!(
            strip(&numbers[2]),
            key.crt_coefficient().unwrap().to_bytes_be()
        );
        assert_eq!(strip(&numbers[3]), key.primes()[0].to_bytes_be());
        assert_eq!(strip(&numbers[4]), key.primes()[1].to_bytes_be());
        assert_eq!(take_string(&private, &mut at).unwrap(), b"");
    }

    #[test]
    fn opensshs_own_is_kept_and_the_rest_isnt_read() {
        let made = convert(&pem("PRIVATE KEY", &ed25519_pkcs8(false))).unwrap();
        let kept = convert(&format!("\n{}\n\n", made.private_key.as_str())).unwrap();
        assert_eq!(kept.private_key.as_str(), made.private_key.as_str());
        assert_eq!(kept.public_key, made.public_key);
        assert_eq!(kept.fingerprint, made.fingerprint);

        // A P-256 key (another kind), one under a password, and no key at all.
        let p256 = "308187020100301306072a8648ce3d020106082a8648ce3d030107046d306b0201010420";
        let mut der = hex(p256);
        der.extend([7u8; 32]);
        der.extend(hex("a144034200"));
        der.extend([4u8; 65]);
        assert!(convert(&pem("PRIVATE KEY", &der)).is_none());
        assert!(convert(&pem("ENCRYPTED PRIVATE KEY", &ed25519_pkcs8(false))).is_none());
        assert!(convert("ssh-ed25519 AAAA").is_none());
        assert!(
            convert("-----BEGIN PRIVATE KEY-----\nnot base64!\n-----END PRIVATE KEY-----")
                .is_none()
        );
        let mut encrypted = b"openssh-key-v1\0".to_vec();
        put_string(&mut encrypted, b"aes256-ctr");
        put_string(&mut encrypted, b"bcrypt");
        put_string(&mut encrypted, b"");
        encrypted.extend(1u32.to_be_bytes());
        assert!(convert(&pem("OPENSSH PRIVATE KEY", &encrypted)).is_none());
    }
}
