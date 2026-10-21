//! How long the master key takes here: `cargo run --release --example unlock_time`.
use passbundle_core::kdf::{Kdf, MasterKey};
use std::time::Instant;

fn main() {
    for (name, kdf) in [
        (
            "PBKDF2, 600,000 iterations",
            Kdf::Pbkdf2 {
                iterations: 600_000,
            },
        ),
        (
            "Argon2id, 64 MiB, 3 iterations, 4 lanes",
            Kdf::Argon2id {
                iterations: 3,
                memory_mib: 64,
                parallelism: 4,
            },
        ),
    ] {
        let started = Instant::now();
        let key =
            MasterKey::derive("correct horse battery staple 1", "ana@example.test", &kdf).unwrap();
        println!("{name}: {} ms", started.elapsed().as_millis());
        drop(key);
    }
}
