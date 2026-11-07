use horizon_mod_api_gen::internals_view::{self, Declaration, Injection};
use std::{env, fs, path::PathBuf};

fn generate() -> Result<String, String> {
    const MANIFEST: &str = "HORIZON_MOD_INTERNALS_MANIFEST";
    const CATALOG: &str = "HORIZON_MOD_INTERNALS_CATALOG";
    println!("cargo:rerun-if-env-changed={MANIFEST}");
    println!("cargo:rerun-if-env-changed={CATALOG}");
    let (manifest, catalog) = match (env::var_os(MANIFEST), env::var_os(CATALOG)) {
        (None, None) => return Ok("pub mod view {}".into()),
        (Some(m), Some(c)) => (PathBuf::from(m), PathBuf::from(c)),
        _ => return Err(format!("{MANIFEST} and {CATALOG} must both be set")),
    };
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", catalog.display());
    let text = fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let manifest = horizon_mod_package::manifest::Manifest::parse(&text)?;
    let catalog =
        fs::read_to_string(&catalog).map_err(|e| format!("{}: {e}", catalog.display()))?;
    let entries = internals_view::catalog(&catalog)?;
    let declared = manifest
        .wasm
        .as_ref()
        .map(|w| w.internals())
        .or_else(|| manifest.native.as_ref().map(|n| n.internals.as_slice()))
        .unwrap_or(&[])
        .iter()
        .map(|i| Declaration {
            path: i.path.clone(),
            hash: i.hash.clone(),
            kind: i.kind.clone(),
        })
        .collect::<Vec<_>>();
    let injections = manifest
        .inject
        .iter()
        .filter(|i| i.access == horizon_mod_package::manifest::inject::Access::Internals)
        .map(|i| {
            Ok(Injection {
                target: i.at.clone(),
                at_return: match i.kind {
                    horizon_mod_package::manifest::inject::Kind::Head => false,
                    horizon_mod_package::manifest::inject::Kind::Return => true,
                    _ => return Err(format!("{}: E_INJECT_NO_SLOT: wasm sites pending", i.id)),
                },
                listener: i.with.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let points = horizon_mod_api_gen::points::game_points(&root)?;
    // The shared selector may read a function body or point declaration as well as the catalog.
    for path in horizon_mod_api_gen::points::tracked(&root)? {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    internals_view::generate(&entries, &declared, &injections, &points)
}

fn main() {
    let code = generate().expect("generating the mod's declared internals view");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("view.rs"),
        code,
    )
    .unwrap();
}
