//! Lists the libraries Windows programs import, as their PE import tables
//! name them (delay-loaded ones too), and fails if any is the Visual C++
//! runtime's or the C runtime's: `VCRUNTIME*.dll`, `MSVCP*.dll`, or
//! `api-ms-win-crt-*.dll`. The agent and its host are built with the C
//! runtime linked in (`.cargo/config.toml`), so they need only Windows' own
//! libraries, which come from System32 (DESIGN.md, "The update task
//! (Windows)"). CI checks the release builds with this:
//!
//!   cargo run --example imports -- <program>...
//!
//! It only reads the files, so it runs anywhere.

use std::{fs, process::ExitCode};

/// Names of the C and C++ runtimes' libraries, lowercase.
const RUNTIMES: &[&str] = &["vcruntime", "msvcp", "api-ms-win-crt-"];

fn main() -> ExitCode {
    let programs: Vec<String> = std::env::args().skip(1).collect();
    if programs.is_empty() {
        eprintln!("usage: imports <program>...");
        return ExitCode::from(2);
    }
    let mut ok = true;
    for program in &programs {
        let imports = match fs::read(program)
            .map_err(|e| e.to_string())
            .and_then(|bytes| imports(&bytes))
        {
            Ok(imports) => imports,
            Err(e) => {
                eprintln!("{program}: {e}");
                ok = false;
                continue;
            }
        };
        let runtimes: Vec<&String> = imports
            .iter()
            .filter(|name| {
                let name = name.to_ascii_lowercase();
                RUNTIMES.iter().any(|runtime| name.starts_with(runtime))
            })
            .collect();
        println!("{program} imports {}", imports.join(", "));
        if !runtimes.is_empty() {
            let names: Vec<&str> = runtimes.iter().map(|s| s.as_str()).collect();
            eprintln!(
                "{program} needs the C or C++ runtime installed ({}): it isn't built with the runtime linked in.",
                names.join(", ")
            );
            ok = false;
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, String> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| "it ends early".to_owned())
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| "it ends early".to_owned())
}

fn u64_at(bytes: &[u8], at: usize) -> Result<u64, String> {
    Ok(u64::from(u32_at(bytes, at)?) | u64::from(u32_at(bytes, at + 4)?) << 32)
}

struct Section {
    address: u32,
    size: u32,
    offset: u32,
}

/// The names of the libraries a PE file imports, in its import table's
/// order, then its delay-load table's.
fn imports(bytes: &[u8]) -> Result<Vec<String>, String> {
    if bytes.get(..2) != Some(b"MZ") {
        return Err("it isn't a Windows program".into());
    }
    let pe = u32_at(bytes, 0x3c)? as usize;
    if bytes.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("it isn't a Windows program".into());
    }
    let coff = pe + 4;
    let sections = u16_at(bytes, coff + 2)? as usize;
    let optional = coff + 20;
    let optional_size = u16_at(bytes, coff + 16)? as usize;
    // PE32 or PE32+: where the image base and the data directories are.
    let (base, directories, count) = match u16_at(bytes, optional)? {
        0x10b => (
            u64::from(u32_at(bytes, optional + 28)?),
            optional + 96,
            u32_at(bytes, optional + 92)?,
        ),
        0x20b => (
            u64_at(bytes, optional + 24)?,
            optional + 112,
            u32_at(bytes, optional + 108)?,
        ),
        other => {
            return Err(format!(
                "its optional header is of an unknown kind ({other:#x})"
            ))
        }
    };
    let table: Vec<Section> = (0..sections)
        .map(|i| {
            let at = optional + optional_size + 40 * i;
            Ok(Section {
                size: u32_at(bytes, at + 8)?.max(u32_at(bytes, at + 16)?),
                address: u32_at(bytes, at + 12)?,
                offset: u32_at(bytes, at + 20)?,
            })
        })
        .collect::<Result<_, String>>()?;
    let file_offset = |rva: u32| -> Result<usize, String> {
        table
            .iter()
            .find(|s| rva >= s.address && rva - s.address < s.size)
            .map(|s| (s.offset + (rva - s.address)) as usize)
            .ok_or_else(|| format!("address {rva:#x} is in no section"))
    };
    let name_at = |rva: u32| -> Result<String, String> {
        let start = file_offset(rva)?;
        let rest = bytes.get(start..).ok_or("a name is past its end")?;
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or("a name doesn't end")?;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    };
    let directory = |index: u32| -> Result<Option<u32>, String> {
        if index >= count {
            return Ok(None);
        }
        let rva = u32_at(bytes, directories + 8 * index as usize)?;
        Ok((rva != 0).then_some(rva))
    };

    let mut names = Vec::new();
    // The import table: 20-byte entries, the library's name at 12, ending
    // with an empty one.
    if let Some(rva) = directory(1)? {
        let mut at = file_offset(rva)?;
        loop {
            let name = u32_at(bytes, at + 12)?;
            if name == 0 && u32_at(bytes, at + 16)? == 0 {
                break;
            }
            names.push(name_at(name)?);
            at += 20;
        }
    }
    // The delay-load table: 32-byte entries, attributes at 0 and the name
    // at 4: an address relative to the image, or in old ones a whole one.
    if let Some(rva) = directory(13)? {
        let mut at = file_offset(rva)?;
        loop {
            let attributes = u32_at(bytes, at)?;
            let name = u32_at(bytes, at + 4)?;
            if attributes == 0 && name == 0 {
                break;
            }
            let name = if attributes & 1 == 1 {
                name
            } else {
                u32::try_from(u64::from(name).saturating_sub(base))
                    .map_err(|_| "a delay-loaded library's name is out of range")?
            };
            names.push(name_at(name)?);
            at += 32;
        }
    }
    Ok(names)
}
