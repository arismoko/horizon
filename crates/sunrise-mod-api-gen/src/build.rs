//! Cargo-facing generation: the mod API's and the game's generated files.
use super::{Catalog, Target, host, sources};
use std::{fs, path::Path};
mod vocab_boundary;

fn write_vocab_output(
    out_dir: &Path,
    name: &str,
    code: &str,
    inventory: &vocab_boundary::Inventory,
    sources: &std::collections::BTreeSet<std::path::PathBuf>,
) -> Result<(), String> {
    vocab_boundary::validate_with_modules(name, code, inventory, sources)
        .map_err(|e| format!("{name}: {e}"))?;
    write_if_changed(out_dir.join(name), code).map_err(|e| e.to_string())
}

/// Preserve output timestamps when generation produces identical bytes.
pub fn write_if_changed(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    let path = path.as_ref();
    let bytes = bytes.as_ref();
    match fs::read(path) {
        Ok(previous) if previous == bytes => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::write(path, bytes)
}

/// Generate only this crate's files, tracking every source read by the generator.
/// `root` is supplied at run time by the consuming crate's build script.
pub fn generate(root: &Path, out_dir: &Path, target: Target) -> Result<(), String> {
    generate_with_vocab(root, out_dir, target, None)
}

pub fn generate_gameplay_vocab(root: &Path, out_dir: &Path) -> Result<(), String> {
    let catalog = super::gameplay::Routing::load(root)?.catalog(root)?;
    generate_with_vocab(root, out_dir, Target::Vocab, Some(catalog))
}

fn generate_with_vocab(
    root: &Path,
    out_dir: &Path,
    target: Target,
    catalog: Option<Catalog>,
) -> Result<(), String> {
    track(root)?;
    if let Some(package) = gameplay_package(&target) {
        crate::rerun_if_staged(root);
        for (name, code) in super::gameplay::generate(root, package)? {
            let code = generated_source(root, &target, name, code)?;
            syn::parse_file(&code).map_err(|e| format!("generated Rust: {name}: {e}"))?;
            write_if_changed(out_dir.join(name), code).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let inventory = if matches!(target, Target::Vocab) {
        let path = root.join("tools/layering/mod_vocab_state.json");
        crate::rerun_if_changed(&path.display());
        vocab_boundary::inventory(root)?
    } else {
        vocab_boundary::Inventory::new()
    };
    let vocab_sources = if matches!(target, Target::Vocab) {
        vocab_boundary::modules::sources(root)?
    } else {
        std::collections::BTreeSet::new()
    };
    let outputs = match target {
        Target::GameplayCommon | Target::GameplayServer | Target::GameplayClient => unreachable!(),
        Target::Vocab => {
            let generated = match catalog {
                Some(catalog) => super::sources_from_catalog(root, &catalog)?,
                None => sources(root)?,
            };
            write_vocab_output(
                out_dir,
                super::parts::OUTPUT,
                &super::parts::source(root)?,
                &inventory,
                &vocab_sources,
            )?;
            for (name, code) in super::exports::sources(root)? {
                write_vocab_output(out_dir, &name, &code, &inventory, &vocab_sources)?;
            }
            let included = [
                (host::AREAS_OUTPUT, generated.areas.as_str()),
                ("mod_api_generated.rs", generated.hooks.as_str()),
                ("mod_api_generated_host.rs", generated.host.as_str()),
            ];
            let (keys, paths) = super::capability_keys::generate(root, &included)?;
            for path in paths {
                crate::rerun_if_changed(&path.display());
            }
            vec![
                (host::AREAS_OUTPUT, generated.areas),
                ("mod_api_generated.rs", generated.hooks),
                ("mod_api_generated_host.rs", generated.host),
                (super::capability_keys::OUTPUT, keys),
            ]
        }
    };
    for (name, code) in outputs {
        let code = generated_source(root, &target, name, code)?;
        if matches!(target, Target::Vocab) {
            write_vocab_output(out_dir, name, &code, &inventory, &vocab_sources)?;
        } else {
            write_if_changed(out_dir.join(name), &code).map_err(|e| e.to_string())?;
        }
        syn::parse_file(&code).map_err(|e| format!("generated Rust: {name}: {e}"))?;
    }
    Ok(())
}

/// Generate the mod API's Part registration modules.
pub fn generate_api_parts(root: &Path, out_dir: &Path) -> Result<(), String> {
    crate::rerun_if_changed(&root.join("crates/sunrise-mod-api/src/parts").display());
    write_if_changed(
        out_dir.join("parts_registration_modules.rs"),
        super::parts::registration_source(root)?,
    )
    .map_err(|e| e.to_string())
}

fn gameplay_package(target: &Target) -> Option<super::GameplayPackage> {
    use super::GameplayPackage::*;
    match target {
        Target::GameplayCommon => Some(Common),
        Target::GameplayServer => Some(Server),
        Target::GameplayClient => Some(Client),
        Target::Vocab => None,
    }
}

fn generated_source(
    root: &Path,
    target: &Target,
    name: &str,
    code: String,
) -> Result<String, String> {
    if gameplay_package(target).is_some()
        && matches!(
            name,
            "mod_api_fire_generated.rs" | "mod_api_goal_view_generated.rs"
        )
        && root.join(".sunrise-source-staged").is_file()
    {
        super::internals::widen_generated(Path::new(name), code)
    } else {
        Ok(code)
    }
}

#[cfg(test)]
mod generated_visibility_tests {
    use super::*;

    #[test]
    fn staged_game_helpers_are_widened_but_source_helpers_are_unchanged() {
        let root =
            std::env::temp_dir().join(format!("fnr-generated-visibility-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let input = "impl Game<'_> { pub(super) fn fire(&mut self) {} }";
        for name in [
            "mod_api_fire_generated.rs",
            "mod_api_goal_view_generated.rs",
        ] {
            assert_eq!(
                generated_source(&root, &Target::GameplayClient, name, input.into()).unwrap(),
                input
            );
        }
        fs::write(root.join(".sunrise-source-staged"), "").unwrap();
        for name in [
            "mod_api_fire_generated.rs",
            "mod_api_goal_view_generated.rs",
        ] {
            assert_eq!(
                generated_source(&root, &Target::GameplayClient, name, input.into()).unwrap(),
                "impl Game<'_> { pub fn fire(&mut self) {} }"
            );
            assert_eq!(
                generated_source(&root, &Target::Vocab, name, input.into()).unwrap(),
                input
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
}

/// The game's side of the host traits, which `game/host/mod.rs` includes.
pub(crate) const HOST_OUTPUT: &str = "game_host_generated.rs";

/// The generated forwarders from each host area's trait to the inherent `Game` functions.
pub(crate) fn game_host_generated(root: &Path) -> Result<String, String> {
    Ok(format!(
        "mod generated {{\n{}\n}}\n",
        host::game_forwarders(root)?
    ))
}

/// Generate the native-only raw render records, without trait dispatch or subscriptions.
pub fn generate_render(root: &Path, out_dir: &Path) -> Result<(), String> {
    let declarations = root.join(super::catalog::DECLARATIONS);
    crate::rerun_if_changed(&declarations.display());
    for module in sunrise_game_discovery::module_files(&declarations)? {
        crate::rerun_if_changed(&declarations.join(format!("{module}.rs")).display());
    }
    let generated = super::dispatch::render(&Catalog::load(root)?)?;
    let source = generated.render_dispatch;
    use syn::spanned::Spanned;
    let file = syn::parse_file(&source).map_err(|error| error.to_string())?;
    let lines: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    let offset = |point: proc_macro2::LineColumn| lines[point.line - 1] + point.column;
    let mut records = String::from("use super::*;\n");
    for item in file.items {
        let syn::Item::Struct(record) = item else {
            continue;
        };
        let start = record
            .attrs
            .first()
            .map_or_else(|| record.vis.span().start(), |attr| attr.span().start());
        records.push_str(&source[offset(start)..offset(record.span().end())]);
        records.push('\n');
    }
    syn::parse_file(&records).map_err(|error| error.to_string())?;
    write_if_changed(out_dir.join("native_render_generated.rs"), records)
        .and_then(|()| {
            write_if_changed(
                out_dir.join("native_render_fire_generated.rs"),
                generated.render_fire,
            )
        })
        .map_err(|error| error.to_string())
}

#[test]
fn render_records_need_no_game_sources() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = workspace.join(format!("target/render-records-only-{}", std::process::id()));
    let declarations = root.join(super::catalog::DECLARATIONS);
    fs::create_dir_all(&declarations).unwrap();
    for entry in fs::read_dir(workspace.join(super::catalog::DECLARATIONS)).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            fs::copy(&path, declarations.join(path.file_name().unwrap())).unwrap();
        }
    }
    assert!(!root.join("crates/sunrise-client").exists());
    generate_render(&root, &root).unwrap();
    let generated = fs::read_to_string(root.join("native_render_generated.rs")).unwrap();
    let file = syn::parse_file(&generated).unwrap();
    assert_eq!(
        file.items
            .iter()
            .filter(|item| matches!(item, syn::Item::Struct(_)))
            .count(),
        5
    );
    fs::remove_dir_all(root).unwrap();
}

pub(super) fn track(root: &Path) -> Result<(), String> {
    fn directory(path: &Path, extension: &str, recursive: bool) -> Result<(), String> {
        // Track the directory as well: adding a helper source or fragment changes the inputs.
        crate::rerun_if_changed(&path.display());
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() && recursive {
                directory(&path, extension, recursive)?;
            } else if path.extension().is_some_and(|ext| ext == extension) {
                crate::rerun_if_changed(&path.display());
            }
        }
        Ok(())
    }
    crate::rerun_if_changed(&root.join("crates/sunrise-mod-vocab/src").display());
    directory(&root.join(super::catalog::DECLARATIONS), "rs", false)?;
    directory(&root.join(super::exports::DIRECTORY), "rs", false)?;
    directory(
        &root.join("crates/sunrise-mod-vocab/src/parts"),
        "rs",
        false,
    )?;
    Ok(())
}

#[test]
fn unchanged_outputs_keep_their_timestamp_but_changed_outputs_replace_bytes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    fs::create_dir_all(&root).unwrap();
    let output = root.join(format!("write-if-changed-test-{}", std::process::id()));
    write_if_changed(&output, b"first").unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
    fs::File::options()
        .write(true)
        .open(&output)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old))
        .unwrap();
    write_if_changed(&output, b"first").unwrap();
    assert_eq!(fs::metadata(&output).unwrap().modified().unwrap(), old);
    write_if_changed(&output, b"second").unwrap();
    assert_eq!(fs::read(&output).unwrap(), b"second");
    fs::remove_file(output).unwrap();
}
