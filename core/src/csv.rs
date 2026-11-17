//! CSV, read and written as the web vault does (its importers and exporter
//! use papaparse, passbundle-clients `libs/importer/src/importers/
//! base-importer.ts`, `parseCsv`, and `libs/tools/export-vault-core`'s
//! `papa.unparse`), so a file from either one reads the same in the other.
//!
//! - **Reading:** every line ending (`\r\n`, `\r`, `\n`) becomes `\n`, even
//!   inside quotes, and the text is trimmed (which takes a byte order mark
//!   off too). A field in double quotes may hold commas, quotes (two of
//!   them), and line breaks; a quote in an unquoted field is part of it. A
//!   quote that's never closed would swallow every row after it into one
//!   value, so the file is refused (as our fork of the web vault refuses it).
//! - **Writing:** a header row, then a row for each record, joined with
//!   `\r\n` and nothing after the last. A value is quoted only when it has a
//!   comma, a quote, a line break, or a byte order mark, or starts or ends
//!   with a space, and a quote inside it is doubled. No rows, no text at all.

use std::collections::HashMap;

use zeroize::Zeroizing;

/// A quote that's never closed.
#[derive(Debug, PartialEq, Eq)]
pub struct Unclosed;

/// The rows of a CSV file, each a list of fields, after the line endings
/// are made `\n` and the text is trimmed.
pub fn rows(text: &str) -> Result<Vec<Vec<Zeroizing<String>>>, Unclosed> {
    let normalized = Zeroizing::new(text.replace("\r\n", "\n").replace('\r', "\n"));
    let data = normalized.trim_matches(js_space);
    let chars: Vec<char> = data.chars().collect();
    let mut rows = Vec::new();
    if chars.is_empty() {
        return Ok(rows);
    }
    let mut row: Vec<Zeroizing<String>> = Vec::new();
    let mut at = 0;
    loop {
        let mut field = Zeroizing::new(String::new());
        // Where the field ended: a comma (another field follows), a line
        // break (another row), or the end of the text.
        let ended;
        if chars.get(at) == Some(&'"') {
            at += 1;
            loop {
                let Some(&c) = chars.get(at) else {
                    return Err(Unclosed);
                };
                if c != '"' {
                    field.push(c);
                    at += 1;
                    continue;
                }
                // Two quotes are one, inside quotes.
                if chars.get(at + 1) == Some(&'"') {
                    field.push('"');
                    at += 2;
                    continue;
                }
                // A closing quote, maybe with spaces before the comma or
                // the line break.
                let mut next = at + 1;
                while chars.get(next) == Some(&' ') {
                    next += 1;
                }
                match chars.get(next) {
                    None => {
                        at = next;
                        ended = End::Text;
                        break;
                    }
                    Some(',') => {
                        at = next + 1;
                        ended = End::Field;
                        break;
                    }
                    Some('\n') => {
                        at = next + 1;
                        ended = End::Row;
                        break;
                    }
                    // A quote that doesn't close the field is part of it.
                    Some(_) => {
                        field.push('"');
                        at += 1;
                    }
                }
            }
        } else {
            loop {
                match chars.get(at) {
                    None => {
                        ended = End::Text;
                        break;
                    }
                    Some(',') => {
                        at += 1;
                        ended = End::Field;
                        break;
                    }
                    Some('\n') => {
                        at += 1;
                        ended = End::Row;
                        break;
                    }
                    Some(&c) => {
                        field.push(c);
                        at += 1;
                    }
                }
            }
        }
        row.push(field);
        match ended {
            End::Field => {}
            End::Row => rows.push(std::mem::take(&mut row)),
            End::Text => {
                rows.push(row);
                return Ok(rows);
            }
        }
    }
}

/// What JavaScript's `trim` takes off the ends: Unicode's spaces and line
/// breaks, the byte order mark, but not U+0085 (Rust's `trim` takes that one
/// and leaves the byte order mark).
pub(crate) fn js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

enum End {
    Field,
    Row,
    Text,
}

/// A CSV file with a header row, as upstream's importers read one: each
/// row by its column's name, as the header has it. A column named twice is
/// the second time "name_1", then "name_2", as papaparse names them. A row
/// with nothing in it is left out.
pub struct Table {
    pub columns: Vec<String>,
    pub records: Vec<HashMap<String, Zeroizing<String>>>,
}

impl Table {
    pub fn parse(text: &str) -> Result<Table, Unclosed> {
        let mut rows = rows(text)?.into_iter();
        let Some(header) = rows.next() else {
            return Ok(Table {
                columns: Vec::new(),
                records: Vec::new(),
            });
        };
        let mut columns: Vec<String> = Vec::new();
        for name in header {
            let mut unique = name.as_str().to_owned();
            let mut n = 0;
            while columns.contains(&unique) {
                n += 1;
                unique = format!("{}_{n}", name.as_str());
            }
            columns.push(unique);
        }
        let records = rows
            .filter(|row| row.iter().any(|f| !f.is_empty()))
            .map(|row| columns.iter().cloned().zip(row).collect())
            .collect();
        Ok(Table { columns, records })
    }

    /// Whether a column of this name (ignoring case) is in the header.
    pub fn has(&self, name: &str) -> bool {
        self.columns.iter().any(|c| c.eq_ignore_ascii_case(name))
    }
}

/// One value, as papaparse writes it (`safe`).
fn write_value(out: &mut String, value: &str) {
    let quote = value.contains([',', '"', '\r', '\n', '\u{feff}'])
        || value.starts_with(' ')
        || value.ends_with(' ');
    if quote {
        out.push('"');
        out.push_str(&value.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(value);
    }
}

/// A CSV file: the header, then each row, as papaparse writes them. No rows
/// is no text at all, as papaparse writes an empty list.
pub fn write(header: &[&str], rows: &[Vec<Zeroizing<String>>]) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::new());
    if rows.is_empty() {
        return out;
    }
    for (i, name) in header.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_value(&mut out, name);
    }
    for row in rows {
        out.push_str("\r\n");
        for (i, value) in row.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_value(&mut out, value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(rows: Vec<Vec<Zeroizing<String>>>) -> Vec<Vec<String>> {
        rows.into_iter()
            .map(|r| r.into_iter().map(|f| f.as_str().to_owned()).collect())
            .collect()
    }

    #[test]
    fn quotes_commas_and_line_breaks_inside_quotes() {
        let text = "\u{feff}name,notes\r\n\"a, b\",\"say \"\"hi\"\"\nthen\r\nbye\"\r\nplain,un\"quoted\n\n";
        assert_eq!(
            plain(rows(text).unwrap()),
            [
                vec!["name", "notes"],
                vec!["a, b", "say \"hi\"\nthen\nbye"],
                vec!["plain", "un\"quoted"],
            ]
        );
    }

    #[test]
    fn a_quote_never_closed_is_refused() {
        assert_eq!(rows("a,b\n\"never closed,x\ny,z"), Err(Unclosed));
    }

    #[test]
    fn empty_fields_and_a_quote_that_doesnt_close_its_field() {
        assert_eq!(
            plain(rows("a,,c\n\"x\"y\",z").unwrap()),
            [vec!["a", "", "c"], vec!["x\"y", "z"]]
        );
        assert!(rows("  \r\n ").unwrap().is_empty());
    }

    #[test]
    fn a_table_by_column_names_with_blank_rows_left_out() {
        let table = Table::parse("url,name,url\nhttps://a.test,A,x\n,,\nhttps://b.test").unwrap();
        assert_eq!(table.columns, ["url", "name", "url_1"]);
        assert_eq!(table.records.len(), 2);
        assert_eq!(table.records[0]["url_1"].as_str(), "x");
        // A short row has only the columns it reaches.
        assert!(!table.records[1].contains_key("name"));
        assert!(table.has("URL"));
    }

    #[test]
    fn written_as_papaparse_writes_it() {
        let row = |v: &[&str]| v.iter().map(|s| Zeroizing::new((*s).to_owned())).collect();
        let text = write(
            &["a", "b", "c"],
            &[
                row(&["plain", "with, comma", "with \"quote\""]),
                row(&[" lead", "trail ", "two\nlines"]),
                row(&["", "", ""]),
            ],
        );
        assert_eq!(
            text.as_str(),
            "a,b,c\r\nplain,\"with, comma\",\"with \"\"quote\"\"\"\r\n\" lead\",\"trail \",\"two\nlines\"\r\n,,"
        );
        assert_eq!(write(&["a"], &[]).as_str(), "");
        // What's written reads back the same.
        let read = Table::parse(&text).unwrap();
        assert_eq!(read.records[0]["c"].as_str(), "with \"quote\"");
        assert_eq!(read.records[1]["a"].as_str(), " lead");
    }
}
