//! Opening a browser's SQLite files read-only, as they are on disk: no lock
//! taken, nothing written (no journal, no WAL), so an open browser is never
//! disturbed. A file the browser is changing at that moment can read as
//! damaged; closing the browser and trying again reads it whole.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

pub(crate) fn open(path: &Path) -> rusqlite::Result<Connection> {
    let uri = format!("{}?mode=ro&immutable=1", file_uri(path));
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

/// A table's columns, to read only the ones a browser's version has.
pub(crate) fn columns(conn: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = conn.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let names = statement
        .query_map([table], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names)
}

/// A path as a SQLite `file:` URI: forward slashes, and every byte that isn't
/// plainly safe percent-encoded (spaces, `?`, `#`, `%`, and the rest).
pub(crate) fn file_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from("file:");
    // A Windows path starts with its drive ("C:/..."): SQLite wants "///C:/".
    if !text.starts_with('/') {
        uri.push_str("///");
    } else {
        uri.push_str("//");
    }
    for byte in text.bytes() {
        let safe = byte.is_ascii_alphanumeric() || b"-._~/:".contains(&byte);
        if safe {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_become_file_uris_with_awkward_characters_encoded() {
        assert_eq!(
            file_uri(Path::new(
                "/home/ana/.config/google-chrome/Default/Login Data"
            )),
            "file:///home/ana/.config/google-chrome/Default/Login%20Data"
        );
        assert_eq!(
            file_uri(Path::new(
                r"C:\Users\Ana\AppData\Local\Google\Chrome\User Data\Default\Login Data"
            )),
            "file:///C:/Users/Ana/AppData/Local/Google/Chrome/User%20Data/Default/Login%20Data"
        );
        assert_eq!(file_uri(Path::new("/a/b?c#d%e")), "file:///a/b%3Fc%23d%25e");
    }
}
