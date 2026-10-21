// Bakes the brand (../brand/brand.json, shared with the agent) into the core:
// the agent's socket address is named after it.

use std::{env, fs, path::Path};

fn main() {
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
            "pub const NAME: &str = {:?};\npub const DOMAIN: &str = {:?};\n",
            text("/name"),
            text("/domain")
        ),
    )
    .unwrap();
}
