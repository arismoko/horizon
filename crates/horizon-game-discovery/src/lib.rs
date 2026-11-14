//! Directory-derived game module declarations.
#![forbid(unsafe_code)]

use std::{collections::BTreeMap, fs, path::Path};

/// Discover area-root modules in deterministic filename order, preserving their logical scope.
/// `explicit` lists root files or directories declared separately by the caller. Every other
/// directory needs a `.game-area` marker. Headers select visibility, cfg and an optional logical
/// name; `_tests` modules default to `cfg(test)`, unless their cfg header says `always`.
pub fn declarations(game: &Path, explicit: &[&str]) -> Result<String, String> {
    let mut modules = BTreeMap::new();
    for entry in fs::read_dir(game).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            if path
                .file_stem()
                .is_some_and(|name| explicit.iter().any(|item| name == *item))
            {
                continue;
            }
            return Err(format!(
                "{}: put game module roots in an area folder",
                path.display()
            ));
        }
        if !path.join(".game-area").is_file() {
            if path.is_dir()
                && path
                    .file_name()
                    .is_none_or(|name| !explicit.iter().any(|item| name == *item))
            {
                return Err(format!("{}: missing .game-area marker", path.display()));
            }
            continue;
        }
        let mut roots: Vec<_> = module_files(&path)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|name| (name.clone(), path.join(format!("{name}.rs"))))
            .collect();
        for child in fs::read_dir(&path).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            let file = child.path().join("mod.rs");
            if file.is_file() {
                let name = child.file_name().to_string_lossy().into_owned();
                syn::parse_str::<syn::Ident>(&name).map_err(|e| e.to_string())?;
                roots.push((name, file));
            }
        }
        for (name, file) in roots {
            let source = fs::read_to_string(&file).map_err(|e| e.to_string())?;
            let visibility = source
                .lines()
                .find_map(|line| line.strip_prefix("// game-module-visibility: "))
                .unwrap_or("");
            if !matches!(visibility, "" | "pub" | "pub(crate)") {
                return Err(format!(
                    "{}: invalid game module visibility",
                    file.display()
                ));
            }
            let condition = source
                .lines()
                .find_map(|line| line.strip_prefix("// game-module-cfg: "));
            let cfg = match condition {
                Some("always") => String::new(),
                Some(condition) => {
                    syn::parse_str::<syn::Meta>(condition)
                        .map_err(|e| format!("{}: invalid game module cfg: {e}", file.display()))?;
                    format!("#[cfg({condition})]\n")
                }
                None if name.ends_with("_tests") => "#[cfg(test)]\n".to_owned(),
                None => String::new(),
            };
            let absolute = file.canonicalize().map_err(|e| e.to_string())?;
            let declaration_name = source
                .lines()
                .find_map(|line| line.strip_prefix("// game-module-name: "))
                .unwrap_or(&name);
            syn::parse_str::<syn::Ident>(declaration_name).map_err(|e| e.to_string())?;
            let declaration = format!(
                "{cfg}#[path = {:?}]\n{visibility} mod {declaration_name};\n",
                absolute.to_string_lossy()
            );
            if modules.insert(name.clone(), declaration).is_some() {
                return Err(format!("duplicate game module {name}"));
            }
        }
    }
    Ok(modules.into_values().collect())
}

/// Declare every `*.rs` file of `directory` as a module named after its file, in file-name order, for a test
/// target's root to `include!`: a new file there needs no hand-written `mod` line. A subfolder or a `mod.rs` is
/// refused, since the flat folder would silently skip it.
pub fn folder_modules(directory: &Path) -> Result<String, String> {
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() || path.file_name().is_some_and(|name| name == "mod.rs") {
            return Err(format!(
                "{}: put each module in a file of its own here, not a folder or mod.rs",
                path.display()
            ));
        }
    }
    let mut code = String::new();
    for name in module_files(directory)? {
        let absolute = directory
            .join(format!("{name}.rs"))
            .canonicalize()
            .map_err(|e| e.to_string())?;
        code.push_str(&format!(
            "#[path = {:?}]\nmod {name};\n",
            absolute.to_string_lossy()
        ));
    }
    Ok(code)
}

fn module_files(directory: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            if name != "mod" {
                syn::parse_str::<syn::Ident>(&name).map_err(|_| {
                    format!("{}: the file name is not a Rust identifier", path.display())
                })?;
                names.push(name);
            }
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_headers_and_explicit_roots_preserve_the_module_contract() {
        let root = std::env::temp_dir().join(format!("engine-discovery-{}", std::process::id()));
        let area = root.join("runtime");
        fs::create_dir_all(&area).unwrap();
        fs::write(area.join(".game-area"), "").unwrap();
        fs::write(root.join("separate.rs"), "").unwrap();
        fs::write(area.join("service.rs"), "// game-module-cfg: all(feature = \"network\", not(test))\n// game-module-name: transport\n// game-module-visibility: pub(crate)\n").unwrap();
        fs::write(area.join("probe_tests.rs"), "").unwrap();
        fs::write(area.join("shared_tests.rs"), "// game-module-cfg: always\n").unwrap();
        let code = declarations(&root, &["separate"]).unwrap();
        assert!(code.contains("#[cfg(all(feature = \"network\", not(test)))]"));
        assert!(code.contains("pub(crate) mod transport;"));
        let parsed = syn::parse_file(&code).unwrap();
        let attributes: Vec<_> = parsed
            .items
            .iter()
            .map(|item| match item {
                syn::Item::Mod(item) => (
                    item.ident.to_string(),
                    item.attrs.iter().any(|attr| attr.path().is_ident("cfg")),
                ),
                _ => panic!("unexpected item"),
            })
            .collect();
        assert_eq!(
            attributes,
            [
                ("probe_tests".into(), true),
                ("transport".into(), true),
                ("shared_tests".into(), false)
            ]
        );
        assert!(
            declarations(&root, &[])
                .unwrap_err()
                .contains("area folder")
        );
        fs::write(area.join("service.rs"), "// game-module-cfg: !\n").unwrap();
        assert!(
            declarations(&root, &["separate"])
                .unwrap_err()
                .contains("invalid game module cfg")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_modules_declare_each_file_in_name_order_and_refuse_what_they_cannot_declare() {
        let root = std::env::temp_dir().join(format!("folder-modules-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("zebra.rs"), "").unwrap();
        fs::write(root.join("apple.rs"), "").unwrap();
        fs::write(root.join("notes.txt"), "").unwrap();
        let absolute = root.canonicalize().unwrap();
        assert_eq!(
            folder_modules(&root).unwrap(),
            format!(
                "#[path = {:?}]\nmod apple;\n#[path = {:?}]\nmod zebra;\n",
                absolute.join("apple.rs").to_string_lossy(),
                absolute.join("zebra.rs").to_string_lossy()
            )
        );
        for refused in ["loot-tags.rs", "mod.rs", "nested"] {
            let path = root.join(refused);
            if refused == "nested" {
                fs::create_dir(&path).unwrap();
            } else {
                fs::write(&path, "").unwrap();
            }
            let error = folder_modules(&root).unwrap_err();
            assert!(error.contains(refused), "{error}");
            if refused == "nested" {
                fs::remove_dir(&path).unwrap();
            } else {
                fs::remove_file(&path).unwrap();
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}
