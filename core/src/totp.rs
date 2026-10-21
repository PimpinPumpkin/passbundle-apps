//! One-time codes (RFC 6238), from what people store: a base32 secret, an
//! `otpauth://totp/` address, or a `steam://` secret.

use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Digits(u32),
    Steam,
}

pub struct Totp {
    secret: Zeroizing<Vec<u8>>,
    algorithm: Algorithm,
    period: u64,
    style: Style,
}

const STEAM_ALPHABET: &[u8] = b"23456789BCDFGHJKMNPQRTVWXY";

fn base32(text: &str) -> Result<Zeroizing<Vec<u8>>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = Zeroizing::new(Vec::new());
    let (mut buffer, mut bits) = (0u64, 0u32);
    for c in text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '=' && *c != '-')
    {
        let index = ALPHABET
            .iter()
            .position(|&a| a as char == c.to_ascii_uppercase())
            .ok_or(Error::Totp("it isn't base32"))?;
        buffer = (buffer << 5) | index as u64;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    if out.is_empty() {
        return Err(Error::Totp("it's empty"));
    }
    Ok(out)
}

/// Percent-decodes a query value ("+" is a space).
fn unescape(value: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(high), Some(low)) => {
                    out.push(high << 4 | low);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Totp {
    pub fn parse(text: &str) -> Result<Totp> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix("steam://") {
            return Ok(Totp {
                secret: base32(rest)?,
                algorithm: Algorithm::Sha1,
                period: 30,
                style: Style::Steam,
            });
        }
        let lower = text.to_ascii_lowercase();
        if lower.starts_with("otpauth://hotp/") {
            return Err(Error::Totp("counter-based (HOTP) codes aren't supported"));
        }
        if !lower.starts_with("otpauth://totp/") {
            return Ok(Totp {
                secret: base32(text)?,
                algorithm: Algorithm::Sha1,
                period: 30,
                style: Style::Digits(6),
            });
        }
        let query = text.split_once('?').map(|(_, q)| q).unwrap_or("");
        let (mut secret, mut algorithm, mut period, mut digits) =
            (None, Algorithm::Sha1, 30u64, 6u32);
        for pair in query.split('&') {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = unescape(value);
            match name.to_ascii_lowercase().as_str() {
                "secret" => secret = Some(base32(&value)?),
                "algorithm" => {
                    algorithm = match value.to_ascii_uppercase().as_str() {
                        "SHA1" => Algorithm::Sha1,
                        "SHA256" => Algorithm::Sha256,
                        "SHA512" => Algorithm::Sha512,
                        _ => {
                            return Err(Error::Totp(
                                "its algorithm isn't SHA-1, SHA-256, or SHA-512",
                            ))
                        }
                    }
                }
                "period" => {
                    period = value
                        .parse()
                        .ok()
                        .filter(|p| (1..=3600).contains(p))
                        .ok_or(Error::Totp("its period isn't a number of seconds"))?
                }
                "digits" => {
                    digits = value
                        .parse()
                        .ok()
                        .filter(|d| (1..=10).contains(d))
                        .ok_or(Error::Totp("it asks for more than 10 digits"))?
                }
                _ => {}
            }
        }
        let secret = secret.ok_or(Error::Totp("the address has no secret"))?;
        Ok(Totp {
            secret,
            algorithm,
            period,
            style: Style::Digits(digits),
        })
    }

    fn truncated(&self, counter: u64) -> u32 {
        let message = counter.to_be_bytes();
        let hash: Vec<u8> = match self.algorithm {
            Algorithm::Sha1 => hmac_digest::<Hmac<Sha1>>(&self.secret, &message),
            Algorithm::Sha256 => hmac_digest::<Hmac<Sha256>>(&self.secret, &message),
            Algorithm::Sha512 => hmac_digest::<Hmac<Sha512>>(&self.secret, &message),
        };
        let offset = (hash[hash.len() - 1] & 0xf) as usize;
        u32::from_be_bytes([
            hash[offset],
            hash[offset + 1],
            hash[offset + 2],
            hash[offset + 3],
        ]) & 0x7fff_ffff
    }

    /// The code at a time, in seconds since 1970.
    pub fn code_at(&self, unix_seconds: u64) -> String {
        let n = self.truncated(unix_seconds / self.period);
        match self.style {
            Style::Digits(digits) => {
                let modulus = 10u64.pow(digits);
                format!("{:0width$}", n as u64 % modulus, width = digits as usize)
            }
            Style::Steam => {
                let mut n = n as usize;
                (0..5)
                    .map(|_| {
                        let c = STEAM_ALPHABET[n % STEAM_ALPHABET.len()] as char;
                        n /= STEAM_ALPHABET.len();
                        c
                    })
                    .collect()
            }
        }
    }

    /// Seconds until the code changes.
    pub fn seconds_left(&self, unix_seconds: u64) -> u64 {
        self.period - unix_seconds % self.period
    }

    pub fn period(&self) -> u64 {
        self.period
    }
}

fn hmac_digest<M: Mac + hmac::digest::KeyInit>(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut mac =
        <M as hmac::digest::KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}
