// Bakes the brand (../brand/brand.json, shared with the agent) into the core:
// the agent's socket address is named after it. Also sets `cfg(phone)` for
// Android and iOS, where the OS's own APIs stand in for the desktop's
// clipboard and secret store (docs/phones.md).

use std::{env, fs, path::Path};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(phone)");
    if matches!(
        env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("android" | "ios")
    ) {
        println!("cargo::rustc-cfg=phone");
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let brand_path = root.join("../brand/brand.json");
    println!("cargo:rerun-if-changed={}", brand_path.display());
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
    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("brand.rs");
    fs::write(
        out,
        format!(
            "pub const NAME: &str = {:?};\npub const DOMAIN: &str = {:?};\n/// PassBundle's own server: where people sign in unless they host their own.\npub const SERVER: &str = {:?};\n",
            text("/name"),
            text("/domain"),
            text("/urls/webVault")
        ),
    )
    .unwrap();
}
