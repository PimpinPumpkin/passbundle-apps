//! Websites' icons for logins' tiles, as the web vault and the extension
//! show them (passbundle-clients, build-cipher-icon.ts and item-tile): from
//! the server's icon service, with the login's letter where there's none.
//! The person can turn them off (each one tells the server a website they
//! have a login for).
//!
//! They're kept in memory only, while the vault is open (`Cache`): which
//! sites a vault has logins for is the vault's to say, so nothing of it is
//! written to the device. An image whose header says it's larger than an
//! icon (`MAX_SIDE`), or that isn't a kind tiles draw, counts as none, so a
//! small file that unpacks to a huge picture never reaches the screens.

use std::collections::{HashMap, VecDeque};
use std::path::Path;

/// The most a side of an icon may be, in pixels: the icon service's are
/// 32 to 256.
pub const MAX_SIDE: u32 = 256;

/// How many hosts' answers the cache keeps, and how many bytes of icons.
const MAX_KEPT: usize = 1000;
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// The website a login's icon is for, as upstream's apps decide it: its
/// first address's host, for a website (an http(s) address, or a bare one
/// with a dot in it), not an app's (androidapp://, iosapp://), nor an onion
/// or I2P site. Only plain ASCII host names: anything else shows the letter.
pub fn host_of(uri: &str) -> Option<String> {
    let uri = uri.trim();
    if uri.is_empty() || uri.starts_with("androidapp://") || uri.starts_with("iosapp://") {
        return None;
    }
    let rest = match uri.split_once("://") {
        None if uri.contains('.') => uri,
        None => return None,
        Some((scheme, rest)) => {
            let scheme = scheme.to_ascii_lowercase();
            if (scheme != "http" && scheme != "https") || !rest.contains('.') {
                return None;
            }
            rest
        }
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    if host.starts_with('[') {
        return None;
    }
    let host = host
        .split(':')
        .next()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let valid = !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !host.starts_with('.')
        && !host.contains("..");
    if !valid || host.ends_with(".onion") || host.ends_with(".i2p") {
        return None;
    }
    Some(host)
}

/// The server's own small globe for a site with no icon (Vaultwarden's
/// fallback, a 19 by 19 pixel PNG): tiles show the letter instead.
pub fn is_fallback(image: &[u8]) -> bool {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if image.len() < 24 || !image.starts_with(PNG) || &image[12..16] != b"IHDR" {
        return false;
    }
    let width = u32::from_be_bytes([image[16], image[17], image[18], image[19]]);
    let height = u32::from_be_bytes([image[20], image[21], image[22], image[23]]);
    width == 19 && height == 19
}

/// Whether an icon from the server is one the tiles draw: a PNG, GIF,
/// JPEG, WebP, ICO, or BMP whose header says each side, of every frame or
/// image in it, is from 1 to `MAX_SIDE` pixels.
pub fn acceptable(image: &[u8]) -> bool {
    size_of(image).is_some_and(|(w, h)| (1..=MAX_SIDE).contains(&w) && (1..=MAX_SIDE).contains(&h))
}

/// The largest width and height an image's header says it has (of any of
/// its frames or images), for the kinds the tiles draw; None for anything
/// else, or a header too short to say.
pub fn size_of(image: &[u8]) -> Option<(u32, u32)> {
    if image.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png(image);
    }
    if image.starts_with(b"GIF87a") || image.starts_with(b"GIF89a") {
        return gif(image);
    }
    if image.starts_with(&[0xFF, 0xD8]) {
        return jpeg(image);
    }
    if image.len() >= 12 && &image[0..4] == b"RIFF" && &image[8..12] == b"WEBP" {
        return webp(image);
    }
    if image.starts_with(&[0, 0, 1, 0]) || image.starts_with(&[0, 0, 2, 0]) {
        return ico(image);
    }
    if image.starts_with(b"BM") {
        return dib(image.get(14..)?);
    }
    None
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn le16(b: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le24(b: &[u8], at: usize) -> Option<u32> {
    let b = b.get(at..at + 3)?;
    Some(b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16)
}

/// A PNG's IHDR (an animated one's frames fit inside it).
fn png(b: &[u8]) -> Option<(u32, u32)> {
    if b.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((be32(b, 16)?, be32(b, 20)?))
}

/// A GIF's screen, and each of its frames, which may claim to reach past it.
fn gif(b: &[u8]) -> Option<(u32, u32)> {
    let (mut w, mut h) = (le16(b, 6)?, le16(b, 8)?);
    let packed = *b.get(10)?;
    let mut at = 13
        + if packed & 0x80 != 0 {
            3 << ((packed & 7) + 1)
        } else {
            0
        };
    // Skips data sub-blocks: each a length byte and that many bytes, to a 0.
    let skip_blocks = |mut at: usize| -> Option<usize> {
        loop {
            let len = *b.get(at)? as usize;
            at += 1;
            if len == 0 {
                return Some(at);
            }
            at += len;
        }
    };
    while let Some(&block) = b.get(at) {
        match block {
            0x21 => at = skip_blocks(at + 2)?,
            0x2C => {
                let (left, top) = (le16(b, at + 1)?, le16(b, at + 3)?);
                let (fw, fh) = (le16(b, at + 5)?, le16(b, at + 7)?);
                w = w.max(left + fw);
                h = h.max(top + fh);
                let flags = *b.get(at + 9)?;
                at += 10;
                if flags & 0x80 != 0 {
                    at += 3 << ((flags & 7) + 1);
                }
                at = skip_blocks(at + 1)?;
            }
            _ => break,
        }
    }
    Some((w, h))
}

/// A JPEG's frame header (SOF0 to SOF15, but for DHT, JPG, and DAC),
/// found segment by segment before the image data starts.
fn jpeg(b: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        while *b.get(at)? != 0xFF {
            at += 1;
        }
        while *b.get(at)? == 0xFF {
            at += 1;
        }
        let marker = *b.get(at)?;
        at += 1;
        match marker {
            0x01 | 0xD0..=0xD7 => continue,
            0xD9 | 0xDA => return None,
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                return Some((be16(b, at + 5)?, be16(b, at + 3)?));
            }
            _ => at += be16(b, at)? as usize,
        }
    }
}

/// A WebP's canvas: VP8X's, or a still lossy (VP8) or lossless (VP8L) one's.
fn webp(b: &[u8]) -> Option<(u32, u32)> {
    match b.get(12..16)? {
        b"VP8X" => Some((le24(b, 24)? + 1, le24(b, 27)? + 1)),
        b"VP8 " => {
            if b.get(23..26)? != [0x9D, 0x01, 0x2A] {
                return None;
            }
            Some((le16(b, 26)? & 0x3FFF, le16(b, 28)? & 0x3FFF))
        }
        b"VP8L" => {
            if *b.get(20)? != 0x2F {
                return None;
            }
            let bits = le32(b, 21)?;
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        _ => None,
    }
}

/// An ICO's (or CUR's) images: each entry's size, and the size its image
/// says it is (a PNG, or a bitmap), which may be larger.
fn ico(b: &[u8]) -> Option<(u32, u32)> {
    let count = le16(b, 4)? as usize;
    if count == 0 {
        return None;
    }
    let (mut w, mut h) = (0, 0);
    for i in 0..count {
        let entry = 6 + 16 * i;
        let side = |v: u8| if v == 0 { 256 } else { v as u32 };
        w = w.max(side(*b.get(entry)?));
        h = h.max(side(*b.get(entry + 1)?));
        let offset = le32(b, entry + 12)? as usize;
        let image = b.get(offset..)?;
        let (iw, ih) = if image.starts_with(b"\x89PNG\r\n\x1a\n") {
            png(image)?
        } else {
            // A bitmap's height counts its mask too: twice the icon's.
            let (iw, ih) = dib(image)?;
            (iw, ih.div_ceil(2))
        };
        w = w.max(iw);
        h = h.max(ih);
    }
    Some((w, h))
}

/// A bitmap's header (BITMAPCOREHEADER, or BITMAPINFOHEADER and later).
fn dib(b: &[u8]) -> Option<(u32, u32)> {
    if le32(b, 0)? == 12 {
        return Some((le16(b, 4)?, le16(b, 6)?));
    }
    let w = i32::from_le_bytes(b.get(4..8)?.try_into().ok()?);
    let h = i32::from_le_bytes(b.get(8..12)?.try_into().ok()?);
    Some((w.unsigned_abs(), h.unsigned_abs()))
}

/// The icons the server gave while the vault is open: each host's, or that
/// it has none, the most recently used kept, at most `MAX_KEPT` hosts and
/// `MAX_BYTES` of icons. In memory only; the app empties it when the vault
/// locks, when the person signs out, and when icons are turned off.
#[derive(Default)]
pub struct Cache {
    answers: HashMap<String, Option<Vec<u8>>>,
    /// Hosts, the least recently used first.
    order: VecDeque<String>,
    bytes: usize,
}

impl Cache {
    /// What's known for a host: Some(Some(icon)), Some(None) when the
    /// server had none, None when it's to be asked.
    pub fn get(&mut self, host: &str) -> Option<Option<&[u8]>> {
        if !self.answers.contains_key(host) {
            return None;
        }
        self.touch(host);
        self.answers.get(host).map(|a| a.as_deref())
    }

    /// Keeps what the server answered for a host, making room as needed.
    pub fn put(&mut self, host: &str, icon: Option<Vec<u8>>) {
        self.remove(host);
        self.bytes += icon.as_ref().map_or(0, Vec::len);
        self.answers.insert(host.to_owned(), icon);
        self.order.push_back(host.to_owned());
        while self.answers.len() > MAX_KEPT || self.bytes > MAX_BYTES {
            let Some(oldest) = self.order.front().cloned() else {
                break;
            };
            self.remove(&oldest);
        }
    }

    pub fn clear(&mut self) {
        *self = Cache::default();
    }

    pub fn len(&self) -> usize {
        self.answers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
    }

    fn touch(&mut self, host: &str) {
        if let Some(at) = self.order.iter().position(|h| h == host) {
            if let Some(h) = self.order.remove(at) {
                self.order.push_back(h);
            }
        }
    }

    fn remove(&mut self, host: &str) {
        if let Some(icon) = self.answers.remove(host) {
            self.bytes -= icon.as_ref().map_or(0, Vec::len);
            self.order.retain(|h| h != host);
        }
    }
}

/// Deletes the folder earlier builds kept icons in, on this device next to
/// the vault (`icons`), if it's there: icons are in memory only now.
pub fn forget_kept(store: &Path) {
    let folder = store.join("icons");
    if folder.is_dir() {
        let _ = std::fs::remove_dir_all(folder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logins_icon_is_for_its_websites_host() {
        for (uri, host) in [
            ("https://mail.example.test/login", Some("mail.example.test")),
            ("http://Example.COM:8080/path?q=1#x", Some("example.com")),
            ("example.com", Some("example.com")),
            (
                "https://user:pw@accounts.example.com",
                Some("accounts.example.com"),
            ),
            ("androidapp://com.example.app", None),
            ("iosapp://com.example.app", None),
            ("https://localhost:8443", None),
            ("ftp://files.example.com", None),
            ("https://hidden.onion", None),
            ("https://[::1]/", None),
            ("https://bücher.example", None),
            ("", None),
            ("just words", None),
        ] {
            assert_eq!(host_of(uri).as_deref(), host, "{uri}");
        }
    }

    #[test]
    fn the_servers_fallback_globe_counts_as_no_icon() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&19u32.to_be_bytes());
        png.extend_from_slice(&19u32.to_be_bytes());
        assert!(is_fallback(&png));
        png[19] = 32;
        png[23] = 32;
        assert!(!is_fallback(&png));
        assert!(!is_fallback(b"<svg/>"));
    }

    fn png_of(w: u32, h: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&w.to_be_bytes());
        png.extend_from_slice(&h.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        png
    }

    #[test]
    fn an_icon_is_drawn_only_when_its_header_says_its_icon_sized() {
        // A small file whose header asks for 12000 by 12000 pixels (576 MB
        // once drawn) isn't an icon.
        assert_eq!(size_of(&png_of(12000, 12000)), Some((12000, 12000)));
        assert!(!acceptable(&png_of(12000, 12000)));
        assert!(!acceptable(&png_of(257, 16)));
        assert!(!acceptable(&png_of(0, 16)));
        assert!(acceptable(&png_of(256, 256)));
        assert!(acceptable(&png_of(32, 32)));

        // GIF: the screen, and a frame that claims to reach past it.
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[32, 0, 32, 0, 0, 0, 0]);
        assert_eq!(size_of(&gif), Some((32, 32)));
        gif.extend_from_slice(&[0x21, 0xF9, 4, 0, 0, 0, 0, 0]);
        gif.extend_from_slice(&[0x2C, 0, 0, 0, 0, 0x10, 0x27, 0x10, 0x27, 0, 2, 1, 0, 0]);
        gif.push(0x3B);
        assert_eq!(size_of(&gif), Some((10000, 10000)));
        assert!(!acceptable(&gif));

        // JPEG: an APP0 segment, then the frame header (SOF0).
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0];
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0, 48, 0, 64, 3]);
        assert_eq!(size_of(&jpeg), Some((64, 48)));
        jpeg[13] = 0x40;
        assert!(!acceptable(&jpeg));

        // WebP, lossy, lossless, and extended.
        let riff = |chunk: &[u8], data: &[u8]| {
            let mut b = b"RIFF\0\0\0\0WEBP".to_vec();
            b.extend_from_slice(chunk);
            b.extend_from_slice(&[0, 0, 0, 0]);
            b.extend_from_slice(data);
            b
        };
        let lossy = riff(b"VP8 ", &[0, 0, 0, 0x9D, 0x01, 0x2A, 64, 0, 32, 0]);
        assert_eq!(size_of(&lossy), Some((64, 32)));
        let bits: u32 = 63 | (31 << 14);
        let mut data = vec![0x2F];
        data.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(size_of(&riff(b"VP8L", &data)), Some((64, 32)));
        let extended = riff(b"VP8X", &[0, 0, 0, 0, 0x0F, 0x27, 0, 0x0F, 0x27, 0]);
        assert_eq!(size_of(&extended), Some((10000, 10000)));
        assert!(!acceptable(&extended));

        // ICO: its entry says 32 by 32, but the PNG inside is larger.
        let inner = png_of(4096, 4096);
        let mut ico = vec![0, 0, 1, 0, 1, 0, 32, 32, 0, 0, 1, 0, 32, 0];
        ico.extend_from_slice(&(inner.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(&inner);
        assert_eq!(size_of(&ico), Some((4096, 4096)));
        assert!(!acceptable(&ico));
        // And a bitmap inside, its height counting its mask.
        let mut ico = vec![
            0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0, 40, 0, 0, 0, 22, 0, 0, 0,
        ];
        ico.extend_from_slice(&40u32.to_le_bytes());
        ico.extend_from_slice(&16i32.to_le_bytes());
        ico.extend_from_slice(&32i32.to_le_bytes());
        assert_eq!(size_of(&ico), Some((16, 16)));
        assert!(acceptable(&ico));

        // BMP, drawn bottom up or top down.
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0; 12]);
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&48i32.to_le_bytes());
        bmp.extend_from_slice(&(-48i32).to_le_bytes());
        assert_eq!(size_of(&bmp), Some((48, 48)));

        // Not a kind the tiles draw, or too short to say.
        assert!(!acceptable(b"<svg xmlns='http://www.w3.org/2000/svg'/>"));
        assert!(!acceptable(b"\x89PNG\r\n\x1a\n"));
        assert!(!acceptable(&[0xFF, 0xD8, 0xFF]));
        assert!(!acceptable(b""));
    }

    #[test]
    fn the_cache_keeps_the_most_recently_used() {
        let mut cache = Cache::default();
        assert_eq!(cache.get("a.example"), None);
        cache.put("a.example", Some(vec![1; 10]));
        cache.put("none.example", None);
        assert_eq!(cache.get("a.example"), Some(Some(&[1u8; 10][..])));
        assert_eq!(cache.get("none.example"), Some(None));
        // Past the limit, the least recently used go first.
        for i in 0..MAX_KEPT {
            cache.put(&format!("{i}.example"), None);
            if i == 0 {
                cache.get("a.example");
            }
        }
        assert_eq!(cache.len(), MAX_KEPT);
        assert_eq!(cache.get("none.example"), None);
        // And no more than MAX_BYTES of icons.
        cache.put("big.example", Some(vec![0; MAX_BYTES]));
        cache.put("bigger.example", Some(vec![0; MAX_BYTES / 2 + 1]));
        assert_eq!(cache.get("big.example"), None);
        assert!(cache.get("bigger.example").is_some());
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn the_folder_earlier_builds_kept_icons_in_is_deleted() {
        let dir = std::env::temp_dir().join(format!("pb-icons-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(dir.join("icons")).unwrap();
        std::fs::write(dir.join("icons").join("example.com.png"), b"icon").unwrap();
        forget_kept(&dir);
        assert!(!dir.join("icons").exists());
        forget_kept(&dir);
        let _ = std::fs::remove_dir_all(dir);
    }
}
