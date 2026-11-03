//! The package candidates the game loads, before content validation and conflicts.
mod context;
pub use context::{Argument, LaunchArguments, MissingValue, ModSelectionContext};

use crate::{Package, find};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// A selected package, read and validated once at the filesystem boundary.
pub struct SelectedPackage {
    pub path: PathBuf,
    pub package: Package,
}

/// Native content follows the compiled build, not the next build's toggles.
/// Content-only packages follow the player's toggles. Explicit development
/// paths take precedence over discovery and are enabled independently of both.
/// Content validation and duplicate-ID acceptance remain the caller's job.
pub fn packages<'a>(
    home: &Path,
    enabled: &'a BTreeSet<String>,
    built: &'a BTreeSet<String>,
    dev: &'a [PathBuf],
) -> impl Iterator<Item = SelectedPackage> + 'a {
    let mut paths = find(&home.join("horizon-mods"));
    paths.extend(find(&home.join("horizon-native-mods")));
    paths.retain(|path| !dev.contains(path));
    paths.extend(dev.iter().cloned());
    paths.into_iter().filter_map(move |path| {
        let package = Package::read(&path).ok()?;
        let selected = if package.manifest.native.is_some() {
            built
        } else {
            enabled
        };
        (selected.contains(&package.manifest.id) || dev.contains(&path))
            .then_some(SelectedPackage { path, package })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let home = std::env::temp_dir()
                .join(format!("package-selection-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&home).unwrap();
            Self(home)
        }
        fn package(&self, folder: &str, id: &str, code: bool) -> PathBuf {
            let mut manifest = format!(
                "id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n"
            );
            let source: &[(&str, &[u8])] = if code {
                manifest += &format!(
                    "[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
                    "a".repeat(40),
                    "b".repeat(40)
                );
                &[
                    ("Cargo.toml", b"[package]\nname='example'\nversion='1'\n"),
                    ("src/lib.rs", b"pub fn hooks() {}"),
                ]
            } else {
                &[]
            };
            let directory = self.0.join(folder);
            fs::create_dir_all(&directory).unwrap();
            let path = directory.join(format!("{id}.hmod"));
            fs::write(&path, Package::write_files(&manifest, source).unwrap()).unwrap();
            path
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn built_native_ids_and_content_toggles_apply_in_both_folders() {
        let home = Scratch::new();
        // Deliberately put both kinds in the opposite folders too.
        let built = home.package("horizon-mods", "compiled", true);
        home.package("horizon-mods", "next_build", true);
        home.package("horizon-mods", "disabled", false);
        let content = home.package("horizon-native-mods", "plain", false);
        let native = home.package("horizon-native-mods", "other_compiled", true);
        let source = home.package("workspace", "directory_compiled", true);
        let directory = home.0.join("horizon-native-mods/source");
        zip::ZipArchive::new(fs::File::open(source).unwrap())
            .unwrap()
            .extract(&directory)
            .unwrap();
        let enabled = BTreeSet::from(["next_build".into(), "plain".into()]);
        let compiled = BTreeSet::from([
            "compiled".into(),
            "other_compiled".into(),
            "directory_compiled".into(),
        ]);
        let selected: Vec<_> = packages(&home.0, &enabled, &compiled, &[])
            .map(|p| p.path)
            .collect();
        assert_eq!(selected, [built, native, content.clone(), directory]);
        let rebuilt: Vec<_> = packages(&home.0, &enabled, &BTreeSet::new(), &[])
            .map(|p| p.path)
            .collect();
        assert_eq!(rebuilt, [content]);
    }

    #[test]
    fn development_paths_are_appended_once_and_invalid_packages_are_skipped() {
        let home = Scratch::new();
        let discovered = home.package("horizon-mods", "discovered", false);
        let external = home.package("workspace", "external", true);
        let invalid = home.0.join("horizon-mods/invalid.hmod");
        fs::write(&invalid, b"not a package").unwrap();
        let enabled = BTreeSet::from(["discovered".into(), "invalid".into()]);
        let dev = [discovered.clone(), external.clone(), invalid];
        let selected: Vec<_> = packages(&home.0, &enabled, &BTreeSet::new(), &dev)
            .map(|p| p.path)
            .collect();
        assert_eq!(selected, [discovered, external]);
    }
}
