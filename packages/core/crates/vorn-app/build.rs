use std::path::PathBuf;

fn main() {
    let icon = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../resources/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let rc = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("vorn.rc");
    let path = icon.display().to_string().replace('\\', "/");
    std::fs::write(&rc, format!("1 ICON \"{path}\"\n")).unwrap();
    embed_resource::compile(&rc, embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
