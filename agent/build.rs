// Bakes the brand (brand/brand.json, shared with the other apps) and the
// update key into the binary, so nothing the agent shows or trusts comes from
// a file someone could change after install.

use std::{env, fs, path::Path};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let brand_path = root.join("../brand/brand.json");
    let key_path = root.join("update-key.pub");
    println!("cargo:rerun-if-changed={}", brand_path.display());
    println!("cargo:rerun-if-changed={}", key_path.display());

    let brand: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&brand_path).expect("brand/brand.json"))
            .expect("brand/brand.json is JSON");
    let text = |pointer: &str| -> String {
        brand
            .pointer(pointer)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("brand/brand.json has no {pointer}"))
            .to_owned()
    };

    // The public half of the offline update key, in minisign's format. Without
    // it the build can't update itself, which is right for local builds.
    let update_key = fs::read_to_string(&key_path).unwrap_or_default();

    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("brand.rs");
    fs::write(
        out,
        format!(
            "pub const NAME: &str = {:?};\n\
             pub const DOMAIN: &str = {:?};\n\
             pub const WEB_VAULT_URL: &str = {:?};\n\
             pub const NATIVE_MESSAGING_HOST: &str = {:?};\n\
             pub const UPDATE_KEY: &str = {:?};\n",
            text("/name"),
            text("/domain"),
            text("/urls/webVault"),
            text("/nativeMessagingHost"),
            update_key.trim(),
        ),
    )
    .unwrap();
}
