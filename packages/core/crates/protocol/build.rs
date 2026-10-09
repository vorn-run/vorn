//! Generates the protocol types from `schema/store.json`, which
//! `scripts/gen-store-schema.mjs` writes from the TypeScript in
//! `packages/shared`.

use std::path::Path;

fn main() {
    let schema_path = Path::new("schema/store.json");
    println!("cargo:rerun-if-changed={}", schema_path.display());
    let text = std::fs::read_to_string(schema_path).expect("read schema/store.json");
    let schema: schemars::schema::RootSchema =
        serde_json::from_str(&text).expect("schema/store.json is a JSON Schema");

    let mut settings = typify::TypeSpaceSettings::default();
    settings.with_struct_builder(false);
    let mut space = typify::TypeSpace::new(&settings);
    space
        .add_root_schema(schema)
        .expect("schema/store.json converts to Rust types");

    let file: syn::File = syn::parse2(space.to_stream()).expect("generated Rust parses");
    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("store.rs");
    std::fs::write(out, prettyplease::unparse(&file)).expect("write the generated types");
}
