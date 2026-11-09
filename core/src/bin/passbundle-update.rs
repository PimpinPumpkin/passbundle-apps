//! Finishes a Windows or Linux update once the app has quit
//! (src/updates/swap.rs): `passbundle-update finish <work folder>`, run by
//! the app from its own copy in that folder. A program of its own, with no
//! window: Windows doesn't let a program, nor the folder it runs from, be
//! replaced while it runs.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        #[cfg(not(phone))]
        [command, work] if command == "finish" => {
            match passbundle_core::updates::swap::finish(std::path::Path::new(work)) {
                passbundle_core::updates::swap::Outcome::Updated => {}
                passbundle_core::updates::swap::Outcome::Kept(_) => std::process::exit(1),
            }
        }
        [flag] if flag == "--version" => {
            println!("passbundle-update {}", passbundle_core::version());
        }
        _ => {
            eprintln!("PassBundle runs this itself, to finish an update.");
            std::process::exit(2);
        }
    }
}
