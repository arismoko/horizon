//! Validated flat packages and launcher-owned horizon-mixins input.
use super::{Bundle, Injection, InjectionSite, MANIFEST};
use horizon_mods::{manifest, Package};
use std::{fs, path::Path};

pub(super) fn read(dir: &Path) -> Result<Option<Bundle>, String> {
    for old in [
        "horizon-native.json",
        "horizon-mixin.toml",
        "native",
        "rust",
    ] {
        if dir.join(old).exists() {
            return Err(format!("{}: unsupported package path {old}", dir.display()));
        }
    }
    if !dir.join(manifest::MANIFEST).is_file() {
        return Ok(None);
    }
    let package = Package::read(dir)?;
    let Some(native) = &package.manifest.native else {
        return Ok(None);
    };
    let injections = native
        .injection
        .iter()
        .map(|entry| {
            let site = match (&entry.hook, &entry.target) {
                (Some(hook), _) => InjectionSite::Hook(hook.clone()),
                (_, Some(target)) => InjectionSite::Target(target.clone()),
                _ => unreachable!("validated injection site"),
            };
            let kind = serde_json::to_value(entry.kind).expect("serializable injection kind");
            Injection {
                id: entry.id.clone(),
                site,
                kind: Some(kind.as_str().expect("kind is a string").to_owned()),
                selector: entry.selector.clone(),
                ordinal: entry.ordinal,
                priority: entry.priority.unwrap_or_default(),
                cancellable: entry.cancellable,
            }
        })
        .collect();
    package.directory_files(dir)?;
    Ok(Some(Bundle {
        manifest: package.manifest,
        dir: dir.to_owned(),
        injections,
    }))
}

/// Generate tool input in the build's incoming copy, never in a modder's project.
pub(super) fn tool_manifest(bundle: &Bundle) -> Result<(), String> {
    let mut manifest = toml::Table::new();
    manifest.insert(
        "mod".into(),
        toml::Value::Table(toml::Table::from_iter([
            ("id".into(), bundle.manifest.id.clone().into()),
            ("version".into(), bundle.manifest.version.clone().into()),
        ])),
    );
    manifest.insert(
        "compatibility".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "horizon_revision".into(),
            bundle.native().horizon.clone().into(),
        )])),
    );
    if !bundle.native().injection.is_empty() {
        manifest.insert(
            "injection".into(),
            toml::Value::try_from(&bundle.native().injection).map_err(|e| e.to_string())?,
        );
    }
    fs::write(
        bundle.dir.join(MANIFEST),
        toml::to_string(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn checked_files(bundle: &Bundle) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
    Package::read(&bundle.dir)?.directory_files(&bundle.dir)
}

pub(super) fn files(bundle: &Bundle) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    Ok(checked_files(bundle)?
        .into_keys()
        .map(|name| {
            let path = bundle.dir.join(&name);
            (name, path)
        })
        .collect())
}

pub(super) fn copy(bundle: &Bundle, to: &Path) -> Result<(), String> {
    copy_files(to, checked_files(bundle)?)
}

/// Only code enters the tool; runtime resource packs stay with the package.
pub(super) fn copy_tool_input(bundle: &Bundle, to: &Path) -> Result<(), String> {
    copy_files(
        to,
        checked_files(bundle)?
            .into_iter()
            .filter(|(name, _)| manifest::source_file(name)),
    )
}

fn copy_files(to: &Path, files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Result<(), String> {
    for (name, bytes) in files {
        let path = to.join(name);
        fs::create_dir_all(path.parent().ok_or("invalid destination")?)
            .map_err(|e| e.to_string())?;
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}
