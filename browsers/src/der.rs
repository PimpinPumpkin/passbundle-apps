//! Just enough DER for Firefox's encrypted values: SEQUENCE, OCTET STRING,
//! OBJECT IDENTIFIER, INTEGER, and NULL, with definite lengths. Anything
//! else, or anything that runs past its end, is refused.

pub(crate) const SEQUENCE: u8 = 0x30;
pub(crate) const OCTET_STRING: u8 = 0x04;
pub(crate) const OID: u8 = 0x06;
pub(crate) const INTEGER: u8 = 0x02;
pub(crate) const NULL: u8 = 0x05;

/// Reads one value after another from a slice.
pub(crate) struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Reader { rest: bytes }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }

    /// The next value: its tag and contents.
    pub(crate) fn any(&mut self) -> Option<(u8, &'a [u8])> {
        let (&tag, rest) = self.rest.split_first()?;
        let (&first, mut rest) = rest.split_first()?;
        let length = if first < 0x80 {
            first as usize
        } else {
            let count = (first & 0x7f) as usize;
            if count == 0 || count > 4 || rest.len() < count {
                return None;
            }
            let (bytes, after) = rest.split_at(count);
            rest = after;
            bytes.iter().fold(0usize, |n, &b| (n << 8) | b as usize)
        };
        if rest.len() < length {
            return None;
        }
        let (contents, after) = rest.split_at(length);
        self.rest = after;
        Some((tag, contents))
    }

    /// The next value, which must have this tag.
    pub(crate) fn take(&mut self, tag: u8) -> Option<&'a [u8]> {
        match self.any()? {
            (t, contents) if t == tag => Some(contents),
            _ => None,
        }
    }

    /// The next value, a SEQUENCE, to read inside.
    pub(crate) fn sequence(&mut self) -> Option<Reader<'a>> {
        self.take(SEQUENCE).map(Reader::new)
    }

    /// The next value, a small non-negative INTEGER.
    pub(crate) fn integer(&mut self) -> Option<u32> {
        uint(self.take(INTEGER)?)
    }
}

/// An INTEGER's contents, when it's small and not negative.
pub(crate) fn uint(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || bytes[0] & 0x80 != 0 {
        return None;
    }
    let bytes = match bytes {
        [0, rest @ ..] => rest,
        other => other,
    };
    if bytes.len() > 4 {
        return None;
    }
    Some(bytes.iter().fold(0u32, |n, &b| (n << 8) | b as u32))
}

/// Writes DER, for tests that make Firefox profiles.
#[cfg(any(test, feature = "fixtures"))]
pub(crate) fn tlv(tag: u8, contents: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = contents.len();
    if n < 0x80 {
        out.push(n as u8);
    } else if n < 0x100 {
        out.extend([0x81, n as u8]);
    } else {
        out.extend([0x82, (n >> 8) as u8, n as u8]);
    }
    out.extend_from_slice(contents);
    out
}

#[cfg(any(test, feature = "fixtures"))]
pub(crate) fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(SEQUENCE, &parts.concat())
}

#[cfg(any(test, feature = "fixtures"))]
pub(crate) fn int(n: u32) -> Vec<u8> {
    let mut bytes: Vec<u8> = n
        .to_be_bytes()
        .into_iter()
        .skip_while(|&b| b == 0)
        .collect();
    if bytes.first().is_none_or(|b| b & 0x80 != 0) {
        bytes.insert(0, 0);
    }
    tlv(INTEGER, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_it_writes_and_refuses_what_runs_past_its_end() {
        let long = vec![7u8; 300];
        let bytes = seq(&[
            tlv(OCTET_STRING, &long),
            int(10_000),
            int(0),
            tlv(NULL, &[]),
        ]);
        let mut outer = Reader::new(&bytes);
        let mut inner = outer.sequence().unwrap();
        assert!(outer.is_empty());
        assert_eq!(inner.take(OCTET_STRING).unwrap(), &long[..]);
        assert_eq!(inner.integer(), Some(10_000));
        assert_eq!(inner.integer(), Some(0));
        assert_eq!(inner.take(NULL), Some(&[][..]));
        assert!(inner.is_empty());

        assert!(Reader::new(&[0x30, 0x05, 0x04, 0x01]).any().is_none());
        assert!(Reader::new(&[0x04, 0x85, 0, 0, 0, 0, 1]).any().is_none());
        assert!(Reader::new(&[0x04, 0x01, 0x00]).take(OID).is_none());
    }
}
