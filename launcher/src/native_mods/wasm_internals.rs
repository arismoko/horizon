//! Only installed, enabled wasm mods opt a from-source game into an internals table.
use crate::i18n::trf;
use horizon_mod_package::{manifest::Package, mod_list};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) type Declarations = BTreeMap<String, BTreeSet<String>>;

/// Server exports have no instance. Their empty table can be replaced by the daily prebuilt.
pub(super) fn installed(home: Option<&Path>) -> Result<Declarations, String> {
    let mut declared = Declarations::new();
    let Some(home) = home else {
        return Ok(declared);
    };
    let directory = home.join(mod_list::DIR);
    let files = match fs::read_dir(&directory) {
        Ok(files) => files,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(declared),
        Err(e) => {
            return Err(trf(
                "Could not read wasm mods in {path}: {error}",
                &[
                    ("path", &directory.display().to_string()),
                    ("error", &e.to_string()),
                ],
            ));
        }
    };
    for file in files {
        let path = file.map_err(|e| e.to_string())?.path();
        if path.extension().is_none_or(|ext| ext != "hmod") {
            continue;
        }
        let package = Package::read(&path)?;
        let manifest = &package.manifest;
        if !crate::browser::java::is_enabled(home, &manifest.id) {
            continue;
        }
        let Some(wasm) = &manifest.wasm else { continue };
        if wasm.internals().is_empty() {
            continue;
        }
        let paths = declared.entry(manifest.id.clone()).or_default();
        paths.extend(wasm.internals().iter().map(|i| i.path.clone()));
        paths.extend(
            manifest
                .inject
                .iter()
                .filter(|i| i.access == horizon_mod_package::manifest::inject::Access::Internals)
                .map(|i| i.at.clone()),
        );
    }
    Ok(declared)
}

/// Include table selection in the cached build identity, including the empty-table gate.
pub(super) fn version(base: String, declared: &Declarations) -> String {
    let mut files = vec![("wasm-internals-installed".to_owned(), base.clone())];
    files.extend(declared.iter().map(|(id, paths)| {
        (
            id.clone(),
            paths.iter().cloned().collect::<Vec<_>>().join("\n"),
        )
    }));
    let digest = super::digest(
        files
            .iter()
            .map(|(id, paths)| (id.as_str(), paths.as_bytes())),
    );
    let prefix = base.rsplit_once('-').expect("native build identity").0;
    format!("{prefix}-{}", &digest[..12])
}

/// Never use the checkout's seed declarations in a player's from-source build.
pub(super) fn stage(
    root: &Path,
    declared: &Declarations,
    pass: &mut Command,
) -> Result<(), String> {
    if declared.is_empty() {
        pass.arg("--no-wasm-internals");
        return Ok(());
    }
    let directory: PathBuf = root.join("wasm-internals.installed");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    for (id, paths) in declared {
        let text = paths.iter().cloned().collect::<Vec<_>>().join("\n") + "\n";
        fs::write(directory.join(format!("{id}.txt")), text).map_err(|e| e.to_string())?;
    }
    pass.arg("--wasm-internals").arg(directory);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(home: &Path, id: &str, internals: &str) {
        let text = format!(
            "id = '{id}'\nname = 'Fixture'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n[wasm]\nsha512 = '{}'\ntoolchain = 'nightly'\n{internals}",
            horizon_mod_package::manifest::sha512_of(b"\0asmfixture")
        );
        let bytes = Package::write_files(
            &text,
            &[
                ("mod.wasm", b"\0asmfixture".as_slice()),
                (
                    "Cargo.toml",
                    b"[package]\nname = 'fixture'\nversion = '0.1.0'\n".as_slice(),
                ),
                ("src/lib.rs", b"".as_slice()),
            ],
        )
        .unwrap();
        let directory = home.join(mod_list::DIR);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(format!("{id}.hmod")), bytes).unwrap();
    }

    #[test]
    fn wasm_only_pipeline_reaches_installed_table_selection() {
        let root = super::super::tests::dir("wasm-only-pipeline");
        let native = root.join("native");
        fs::create_dir_all(&native).unwrap();
        install(
            &root,
            "wasm_only",
            "\n[[native.internals]]\npath = 'horizon_client::game::Game'\nhash = '1234'\n",
        );
        fs::write(root.join(mod_list::FILE), "mod.wasm_only:true\n").unwrap();
        let native = super::super::stage_installed(&root, native).unwrap();
        let (bundles, declared) = super::super::build_inputs(&native, Some(&root))
            .map_err(String::from)
            .unwrap();
        assert!(bundles.is_empty());
        assert!(super::super::has_wasm_internals(&root));
        let mut pass = Command::new("horizon-stage");
        stage(&root, &declared, &mut pass).unwrap();
        assert_eq!(pass.get_args().next().unwrap(), "--wasm-internals");
        assert_eq!(
            fs::read_to_string(root.join("wasm-internals.installed/wasm_only.txt")).unwrap(),
            "horizon_client::game::Game\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn from_source_table_is_gated_by_enabled_wasm_declarations() {
        let root = super::super::tests::dir("wasm-internals-gate");
        let stage = root.join("stage");
        let seed = stage.join("crates/horizon-mod-api-gen/wasm-internals");
        fs::create_dir_all(&seed).unwrap();
        fs::write(seed.join("uninstalled.txt"), "Never::include").unwrap();
        install(&root, "plain", "");
        let internal = "\n[[native.internals]]\npath = 'horizon_client::game::Game'\nhash = '1234'\n[[inject]]\nid = 'tick'\nat = \"horizon_client::game::Game<'_>::tick\"\nkind = 'head'\naccess = 'internals'\nhash = 'signature'\nwith = 'tick'\n";
        install(&root, "example", internal);
        fs::write(
            root.join(mod_list::FILE),
            "mod.plain:true\nmod.example:false\n",
        )
        .unwrap();
        let empty = installed(Some(&root)).unwrap();
        assert!(empty.is_empty());
        let mut pass = Command::new("horizon-stage");
        super::stage(&stage, &empty, &mut pass).unwrap();
        assert_eq!(pass.get_args().collect::<Vec<_>>(), ["--no-wasm-internals"]);
        assert!(!stage.join("wasm-internals.installed").exists());

        fs::write(
            root.join(mod_list::FILE),
            "mod.plain:true\nmod.example:true\n",
        )
        .unwrap();
        let declared = installed(Some(&root)).unwrap();
        assert_eq!(declared.len(), 1);
        assert_eq!(
            declared["example"]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "horizon_client::game::Game",
                "horizon_client::game::Game<'_>::tick"
            ]
        );
        let mut pass = Command::new("horizon-stage");
        super::stage(&stage, &declared, &mut pass).unwrap();
        assert_eq!(pass.get_args().next().unwrap(), "--wasm-internals");
        let directory = PathBuf::from(pass.get_args().nth(1).unwrap());
        assert_eq!(
            fs::read_to_string(directory.join("example.txt")).unwrap(),
            "horizon_client::game::Game\nhorizon_client::game::Game<'_>::tick\n"
        );
        assert_eq!(fs::read_dir(directory).unwrap().count(), 1);
        let base = "native-1234567-0123456789ab".to_owned();
        assert_ne!(
            version(base.clone(), &empty),
            version(base.clone(), &declared)
        );
        assert_eq!(version(base.clone(), &declared), version(base, &declared));
        assert!(installed(None).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
