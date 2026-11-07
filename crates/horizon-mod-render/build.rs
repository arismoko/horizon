fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        return;
    }
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    horizon_mod_api_gen::generate_render(&root, &out).expect("generating raw render records");
}
