// Bakes the brand (brand/brand.json, shared with the other apps) and the
// update key into the binary, so nothing the agent shows or trusts comes from
// a file someone could change after install.

use std::{env, fs, path::Path};

fn main() {
    // Where Cargo runs this script from now, not where it was compiled: a
    // target folder shared by two checkouts reuses one build script.
    let root = std::path::PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets it"));
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
    // it the build can't update itself, which is right for local builds. The
    // update site is the brand's. A test build (the test-update feature, which
    // release builds refuse) takes both from its environment instead, for the
    // update task's end-to-end test: a throwaway key, and a stand-in for the
    // site on 127.0.0.1.
    println!("cargo:rerun-if-env-changed=PB_TEST_UPDATE_KEY");
    println!("cargo:rerun-if-env-changed=PB_TEST_UPDATE_SOURCE");
    let site = format!("https://{}/agent", text("/domain"));
    let (update_key, update_source) = if env::var_os("CARGO_FEATURE_TEST_UPDATE").is_some() {
        (
            env::var("PB_TEST_UPDATE_KEY").unwrap_or_default(),
            env::var("PB_TEST_UPDATE_SOURCE")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or(site),
        )
    } else {
        (fs::read_to_string(&key_path).unwrap_or_default(), site)
    };

    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("brand.rs");
    fs::write(
        out,
        format!(
            "pub const NAME: &str = {:?};\n\
             pub const DOMAIN: &str = {:?};\n\
             pub const WEB_VAULT_URL: &str = {:?};\n\
             pub const NATIVE_MESSAGING_HOST: &str = {:?};\n\
             pub const UPDATE_KEY: &str = {:?};\n\
             pub const UPDATE_SOURCE: &str = {:?};\n",
            text("/name"),
            text("/domain"),
            text("/urls/webVault"),
            text("/nativeMessagingHost"),
            update_key.trim(),
            update_source.trim_end_matches('/'),
        ),
    )
    .unwrap();
}
