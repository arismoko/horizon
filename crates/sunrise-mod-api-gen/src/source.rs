//! The game's source tree as the generator walks it: each crate's files with their module paths
//! and the `cfg`s they are compiled under, following `mod` declarations, `#[path]`s and inline
//! modules.

use mod_source::modules::{ModuleFile, test_only, walk_module_files};
use quote::ToTokens;
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::{Attribute, Expr, Item, Type};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GameDomain {
    pub package: &'static str,
    pub key: &'static str,
}

impl GameDomain {
    pub fn package_path(self, root: &Path) -> PathBuf {
        root.join("crates").join(self.package)
    }

    pub fn library(self, root: &Path) -> Result<PathBuf, String> {
        let package = self.package_path(root);
        if package.join("Cargo.toml").is_file() {
            library_root(&package)
        } else {
            Ok(package.join("src/lib.rs"))
        }
    }

    pub(crate) fn files(self, root: &Path, every_build: bool) -> Result<Vec<ModuleFile>, String> {
        let lib = self.library(root)?;
        let src = lib.parent().expect("a source file has a folder");
        if crate::GameplayPackage::from_library(self.key).is_some() {
            let game = format!("{}::game", self.key);
            let prefix = format!("{game}::");
            let dir = src.to_owned();
            let mut files = module_files(root, self.key.into(), lib, dir, every_build)?;
            files.retain(|file| file.module == game || file.module.starts_with(&prefix));
            if files.is_empty() {
                return Err(format!("{game}: no mounted gameplay source files"));
            }
            return Ok(files);
        }
        module_files(
            root,
            format!("{}::game", self.key),
            src.join("game.rs"),
            src.join("game"),
            every_build,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceLayout {
    pub(crate) runtime: GameDomain,
    pub(crate) domains: &'static [GameDomain],
    pub(crate) entry: Option<&'static str>,
}

pub(crate) const MARKER: &str = "crates/sunrise-mod-api-gen/source-layout";

pub(crate) const ENTRY_DOMAIN: GameDomain = GameDomain {
    package: "sunrise-client",
    key: "sunrise_client",
};

pub(crate) const LEGACY_LAYOUT: SourceLayout = SourceLayout {
    runtime: ENTRY_DOMAIN,
    domains: &[ENTRY_DOMAIN],
    entry: Some("src/entry.rs"),
};

impl Default for SourceLayout {
    fn default() -> Self {
        #[cfg(test)]
        let layout = LEGACY_LAYOUT;
        #[cfg(not(test))]
        let layout = crate::GAMEPLAY_LAYOUT;
        layout
    }
}

impl SourceLayout {
    /// Read the source tree’s split layout marker. Older trees require their own generator.
    pub fn read(root: &Path) -> Result<Self, String> {
        let path = root.join(MARKER);
        match fs::read_to_string(&path) {
            Ok(text) => match text.trim() {
                #[cfg(test)]
                "legacy" => Ok(LEGACY_LAYOUT),
                #[cfg(not(test))]
                "legacy" => Err("legacy source requires the generator from its revision".into()),
                "split" => Ok(crate::gameplay::GAMEPLAY_LAYOUT),
                other => Err(format!(
                    "{}: unknown source layout {other:?}",
                    path.display()
                )),
            },
            #[cfg(test)]
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(LEGACY_LAYOUT),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    pub(crate) fn is_split(self) -> bool {
        self == crate::gameplay::GAMEPLAY_LAYOUT
    }

    pub(crate) fn domain(self, key: &str) -> Option<GameDomain> {
        self.domains
            .iter()
            .copied()
            .find(|domain| domain.key == key)
    }

    pub(crate) fn owns(self, path: &str) -> bool {
        self.domain(path.split("::").next().unwrap_or_default())
            .is_some()
    }

    pub(crate) fn files(self, root: &Path, every_build: bool) -> Result<Vec<ModuleFile>, String> {
        let mut files = Vec::new();
        for domain in self.domains {
            files.extend(domain.files(root, every_build)?);
        }
        Ok(files)
    }
}

/// Production inline declarations of `name` inside the parent module's items.
pub(crate) fn inline_module_items<'a>(
    items: &'a [Item],
    name: &'a str,
) -> impl Iterator<Item = &'a [Item]> {
    items.iter().filter_map(move |item| match item {
        Item::Mod(module) if module.ident == name && !test_only(&module.attrs) => {
            module.content.as_ref().map(|(_, items)| items.as_slice())
        }
        _ => None,
    })
}

/// Syn puts attributes on the concrete expression nodes, not on the enclosing enum.
pub(crate) fn expr_attrs(expr: &Expr) -> &[Attribute] {
    macro_rules! attrs {
        ($($variant:ident),+ $(,)?) => {
            match expr {
                $(Expr::$variant(node) => &node.attrs,)+
                _ => &[],
            }
        };
    }
    attrs!(
        Array, Assign, Async, Await, Binary, Block, Break, Call, Cast, Closure, Const, Continue,
        Field, ForLoop, Group, If, Index, Infer, Let, Lit, Loop, Macro, Match, MethodCall, Paren,
        Path, Range, RawAddr, Reference, Repeat, Return, Struct, Try, TryBlock, Tuple, Unary,
        Unsafe, While, Yield,
    )
}

/// The files of the game's module `module` ([`walk_module_files`]), each parsed with the game's
/// generated module lists expanded. With `every_build`, the replaceable functions: those every
/// build compiles; without, the internals, listed with the builds they exist in.
pub(crate) fn module_files(
    root: &Path,
    module: String,
    path: PathBuf,
    dir: PathBuf,
    every_build: bool,
) -> Result<Vec<ModuleFile>, String> {
    walk_module_files(module, path, dir, every_build, false, |path, source| {
        crate::game_modules::parse(root, path, source)
    })
}

/// The crate `krate`'s package folder and library root, from its manifest.
pub(crate) fn crate_root(root: &Path, krate: &str) -> Result<(PathBuf, PathBuf), String> {
    let package = root.join("crates").join(krate.replace('_', "-"));
    let lib = library_root(&package)?;
    Ok((package, lib))
}

fn library_root(package: &Path) -> Result<PathBuf, String> {
    let manifest = package.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let manifest: toml::Value = toml::from_str(&text).map_err(|e| e.to_string())?;
    let lib = manifest
        .get("lib")
        .and_then(|lib| lib.get("path"))
        .and_then(toml::Value::as_str)
        .unwrap_or("src/lib.rs");
    Ok(package.join(lib))
}

/// The files of the crate `krate` that every build compiles. Each must resolve inside `root`:
/// the index never names a file the staged copy doesn't hold.
pub(crate) fn crate_files(root: &Path, krate: &str) -> Result<Vec<ModuleFile>, String> {
    let (_, lib) = crate_root(root, krate)?;
    let dir = lib.parent().expect("a source file has a folder").to_owned();
    let files = module_files(root, krate.to_owned(), lib, dir, true)?;
    let canonical = root
        .canonicalize()
        .map_err(|e| format!("{}: {e}", root.display()))?;
    for file in &files {
        let physical = file
            .path
            .canonicalize()
            .map_err(|e| format!("{}: {e}", file.path.display()))?;
        if !physical.starts_with(&canonical) {
            return Err(format!(
                "{}: outside the source at {}",
                file.path.display(),
                root.display()
            ));
        }
    }
    Ok(files)
}

/// A type or path without generic arguments or spaces, as listed paths, owners and found ones
/// are compared (`Game<'_>` is `Game`).
pub(crate) fn plain_path(path: &str) -> String {
    let mut depth = 0usize;
    path.chars()
        .filter(|&c| match c {
            '<' => {
                depth += 1;
                false
            }
            '>' => {
                depth = depth.saturating_sub(1);
                false
            }
            c => depth == 0 && !c.is_whitespace(),
        })
        .collect()
}

/// `r#type` as `type`.
pub(crate) fn unraw(ident: &syn::Ident) -> String {
    ident.to_string().trim_start_matches("r#").to_owned()
}

/// A type as a target names it: lifetimes as `'_`, no spaces.
pub(crate) fn owner(ty: &Type) -> String {
    struct EraseLifetimes;
    impl syn::visit_mut::VisitMut for EraseLifetimes {
        fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
            *lifetime = syn::Lifetime::new("'_", lifetime.span());
        }
    }
    let mut ty = ty.clone();
    syn::visit_mut::VisitMut::visit_type_mut(&mut EraseLifetimes, &mut ty);
    ty.to_token_stream().to_string().replace(' ', "")
}

/// The byte offset of `at` in `source`, whose line starts are `lines`.
pub(crate) fn offset(source: &str, lines: &[usize], at: proc_macro2::LineColumn) -> usize {
    let line = lines[at.line - 1];
    source[line..]
        .char_indices()
        .nth(at.column)
        .map_or(source.len(), |(i, _)| line + i)
}

/// A function marker shared by source-discovery passes.
pub(crate) fn marker<'a>(attrs: &'a [syn::Attribute], name: &str) -> Option<&'a syn::Attribute> {
    attrs
        .iter()
        .find(|attr| attr.path().segments.last().is_some_and(|s| s.ident == name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_layout_marker_selects_the_layout_and_rejects_typos() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let path = root.join(MARKER);
        assert_eq!(SourceLayout::read(root).unwrap(), LEGACY_LAYOUT);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "legacy\r\n").unwrap();
        assert_eq!(SourceLayout::read(root).unwrap(), LEGACY_LAYOUT);
        fs::write(&path, "split\n").unwrap();
        assert_eq!(SourceLayout::read(root).unwrap(), crate::GAMEPLAY_LAYOUT);
        fs::write(&path, "splti\n").unwrap();
        assert!(
            SourceLayout::read(root)
                .unwrap_err()
                .contains("unknown source layout")
        );
    }

    /// A module whose file each target names with `cfg_attr(…, path = …)` is read from each
    /// target's file under its condition; it has no default file, and none is needed.
    #[test]
    fn per_target_module_paths_are_followed_and_need_no_default_file() {
        let dir = std::env::temp_dir().join(format!("module-cfg-attr-{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("control")).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "#[cfg_attr(unix, path = \"control/unix.rs\")]\n#[cfg_attr(windows, path = \"control/windows.rs\")]\nmod platform;\n",
        )
        .unwrap();
        std::fs::write(src.join("control/unix.rs"), "pub fn a() {}").unwrap();
        std::fs::write(src.join("control/windows.rs"), "pub fn b() {}").unwrap();
        let files = module_files(&dir, "k".into(), src.join("lib.rs"), src.clone(), false).unwrap();
        let mut found: Vec<_> = files
            .iter()
            .map(|f| {
                (
                    f.module.as_str(),
                    f.path.strip_prefix(&src).unwrap().to_owned(),
                    f.cfg.clone(),
                )
            })
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                ("k", "lib.rs".into(), vec![]),
                (
                    "k::platform",
                    "control/unix.rs".into(),
                    vec!["unix".to_owned()]
                ),
                (
                    "k::platform",
                    "control/windows.rs".into(),
                    vec!["windows".to_owned()]
                ),
            ]
        );
        // Every build compiles neither, so the replaceable-function walk leaves the module out.
        let every = module_files(&dir, "k".into(), src.join("lib.rs"), src, true).unwrap();
        assert_eq!(every.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
