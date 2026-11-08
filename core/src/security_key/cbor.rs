//! The little CBOR (RFC 8949) a security key speaks (CTAP 2.1, "Message
//! Encoding"): unsigned and negative integers, byte and text strings, arrays,
//! maps, true, false, and null, each with a definite length. What the core
//! sends is in CTAP2's canonical form (shortest lengths, map keys sorted
//! shortest first, then bytewise); what a key answers is read with limits on
//! size and depth, and anything else (floats, tags, indefinite lengths) is
//! refused rather than guessed at.

use crate::error::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Unsigned(u64),
    /// A negative integer, -1 - n as CBOR stores it.
    Negative(u64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    /// Pairs in the order read, or for writing, in any order: `encode` sorts them.
    Map(Vec<(Value, Value)>),
    Bool(bool),
    Null,
}

impl Value {
    pub fn text(s: &str) -> Value {
        Value::Text(s.to_owned())
    }

    /// A map's value for an integer key (CTAP's numbered members).
    pub fn get(&self, key: u64) -> Option<&Value> {
        self.get_by(&Value::Unsigned(key))
    }

    /// A map's value for a text key ("id", "type", "up").
    pub fn get_text(&self, key: &str) -> Option<&Value> {
        self.get_by(&Value::text(key))
    }

    fn get_by(&self, key: &Value) -> Option<&Value> {
        match self {
            Value::Map(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_unsigned(&self) -> Option<u64> {
        match self {
            Value::Unsigned(n) => Some(*n),
            _ => None,
        }
    }
}

fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let major = major << 5;
    match n {
        0..=23 => out.push(major | n as u8),
        24..=0xff => out.extend([major | 24, n as u8]),
        0x100..=0xffff => {
            out.push(major | 25);
            out.extend((n as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(major | 26);
            out.extend((n as u32).to_be_bytes());
        }
        _ => {
            out.push(major | 27);
            out.extend(n.to_be_bytes());
        }
    }
}

/// The value in CTAP2's canonical CBOR.
pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write(&mut out, value);
    out
}

fn write(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Unsigned(n) => head(out, 0, *n),
        Value::Negative(n) => head(out, 1, *n),
        Value::Bytes(b) => {
            head(out, 2, b.len() as u64);
            out.extend(b);
        }
        Value::Text(t) => {
            head(out, 3, t.len() as u64);
            out.extend(t.as_bytes());
        }
        Value::Array(items) => {
            head(out, 4, items.len() as u64);
            for item in items {
                write(out, item);
            }
        }
        Value::Map(pairs) => {
            // Keys sorted by their encoding: shorter first, then bytewise
            // (CTAP 2.1, "CTAP2 canonical CBOR encoding form").
            let mut encoded: Vec<(Vec<u8>, &Value)> =
                pairs.iter().map(|(k, v)| (encode(k), v)).collect();
            encoded.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
            head(out, 5, encoded.len() as u64);
            for (key, value) in encoded {
                out.extend(key);
                write(out, value);
            }
        }
        Value::Bool(false) => out.push(0xf4),
        Value::Bool(true) => out.push(0xf5),
        Value::Null => out.push(0xf6),
    }
}

/// How deep a key's answer may nest: CTAP's answers go four deep at most.
const DEEPEST: usize = 8;

/// Reads one value that takes up all of `bytes`.
pub fn decode(bytes: &[u8]) -> Result<Value> {
    let mut reader = Reader { bytes, at: 0 };
    let value = reader.value(0)?;
    if reader.at != bytes.len() {
        return Err(unreadable());
    }
    Ok(value)
}

fn unreadable() -> Error {
    Error::Malformed("a security key answer the core can't read".into())
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.at.checked_add(n).ok_or_else(unreadable)?;
        let taken = self.bytes.get(self.at..end).ok_or_else(unreadable)?;
        self.at = end;
        Ok(taken)
    }

    fn length(&mut self, info: u8) -> Result<u64> {
        Ok(match info {
            0..=23 => info as u64,
            24 => self.take(1)?[0] as u64,
            25 => u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as u64,
            26 => u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as u64,
            27 => u64::from_be_bytes(self.take(8)?.try_into().unwrap()),
            // Indefinite lengths and the reserved values.
            _ => return Err(unreadable()),
        })
    }

    /// A count of items still to read, which can't be more than the bytes left.
    fn count(&mut self, info: u8) -> Result<usize> {
        let n = self.length(info)?;
        let left = (self.bytes.len() - self.at) as u64;
        if n > left {
            return Err(unreadable());
        }
        Ok(n as usize)
    }

    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > DEEPEST {
            return Err(unreadable());
        }
        let first = self.take(1)?[0];
        let (major, info) = (first >> 5, first & 0x1f);
        Ok(match major {
            0 => Value::Unsigned(self.length(info)?),
            1 => Value::Negative(self.length(info)?),
            2 => {
                let n = self.count(info)?;
                Value::Bytes(self.take(n)?.to_vec())
            }
            3 => {
                let n = self.count(info)?;
                let text = std::str::from_utf8(self.take(n)?).map_err(|_| unreadable())?;
                Value::Text(text.to_owned())
            }
            4 => {
                let n = self.count(info)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.value(depth + 1)?);
                }
                Value::Array(items)
            }
            5 => {
                let n = self.count(info)?;
                let mut pairs = Vec::with_capacity(n);
                for _ in 0..n {
                    let key = self.value(depth + 1)?;
                    let value = self.value(depth + 1)?;
                    if pairs.iter().any(|(k, _)| *k == key) {
                        return Err(unreadable());
                    }
                    pairs.push((key, value));
                }
                Value::Map(pairs)
            }
            7 => match info {
                20 => Value::Bool(false),
                21 => Value::Bool(true),
                22 => Value::Null,
                _ => return Err(unreadable()),
            },
            // Tags.
            _ => return Err(unreadable()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn writes_rfc_8949s_examples() {
        // RFC 8949, Appendix A.
        for (value, expected) in [
            (Value::Unsigned(0), "00"),
            (Value::Unsigned(23), "17"),
            (Value::Unsigned(24), "1818"),
            (Value::Unsigned(1000), "1903e8"),
            (Value::Unsigned(1_000_000), "1a000f4240"),
            (Value::Unsigned(1_000_000_000_000), "1b000000e8d4a51000"),
            (Value::Negative(0), "20"),
            (Value::Negative(99), "3863"),
            (Value::Bytes(vec![1, 2, 3, 4]), "4401020304"),
            (Value::text("IETF"), "6449455446"),
            (Value::text("\u{fc}"), "62c3bc"),
            (Value::Bool(false), "f4"),
            (Value::Bool(true), "f5"),
            (Value::Null, "f6"),
            (
                Value::Array(vec![
                    Value::Unsigned(1),
                    Value::Array(vec![Value::Unsigned(2), Value::Unsigned(3)]),
                ]),
                "8201820203",
            ),
        ] {
            assert_eq!(encode(&value), hex(expected), "{value:?}");
            assert_eq!(decode(&hex(expected)).unwrap(), value);
        }
    }

    #[test]
    fn sorts_map_keys_as_ctap2_does() {
        // Integers before text, shorter text first: CTAP 2.1's own example order.
        let map = Value::Map(vec![
            (Value::text("type"), Value::text("public-key")),
            (Value::text("id"), Value::Bytes(vec![7])),
            (Value::Unsigned(2), Value::Null),
            (Value::Unsigned(1), Value::Null),
        ]);
        let encoded = encode(&map);
        assert_eq!(
            encoded,
            [
                &hex("a4")[..],
                &hex("01f6"),
                &hex("02f6"),
                &hex("626964"),
                &hex("4107"),
                &hex("6474797065"),
                &hex("6a7075626c69632d6b6579"),
            ]
            .concat()
        );
        let read = decode(&encoded).unwrap();
        assert_eq!(
            read.get_text("id").and_then(Value::as_bytes),
            Some(&[7u8][..])
        );
        assert_eq!(read.get(1), Some(&Value::Null));
    }

    #[test]
    fn refuses_what_a_key_never_sends() {
        for bad in [
            "",                   // nothing
            "18",                 // a length that isn't there
            "5f4101ff",           // an indefinite byte string
            "c11a514b67b0",       // a tag
            "f97c00",             // a float
            "a20101",             // a map one pair short
            "a2010101f6",         // the same key twice
            "0001",               // more after the value
            "62c328",             // text that isn't UTF-8
            "5b00000000ffffffff", // a length longer than what's left
        ] {
            assert!(decode(&hex(bad)).is_err(), "{bad}");
        }
        // Nesting past what CTAP uses.
        let deep = [vec![0x81u8; DEEPEST + 2], vec![0x00]].concat();
        assert!(decode(&deep).is_err());
    }
}
