use crate::source;
use mod_source::{
    index::Index,
    modules::{ModuleVariant, cfg_atom, cfg_conditions, evaluate_cfg, walk_module_variants},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
    rc::Rc,
};
use syn::Item;

/// The non-test builds that still agree on every predicate the check has needed so far.
/// Target facts stay together; feature and custom cfgs split independently when encountered.
#[derive(Clone, Default)]
pub(super) struct Build {
    targets: Vec<Rc<HashSet<String>>>,
    values: BTreeMap<String, bool>,
    unknown: RefCell<Option<String>>,
}

impl Build {
    pub(super) fn from_targets(targets: &[String]) -> Result<Self, String> {
        if targets.is_empty() {
            return Err("expected at least one supported target's rustc --print cfg output".into());
        }
        let targets = targets
            .iter()
            .map(|output| {
                output
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(|line| {
                        cfg_atom(line)
                            .map_err(|error| format!("invalid target cfg {line:?}: {error}"))
                    })
                    .collect::<Result<HashSet<_>, _>>()
                    .map(Rc::new)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            targets,
            ..Self::default()
        })
    }

    pub(super) fn split(&self) -> Option<[Self; 2]> {
        let atom = self.unknown.take()?;
        Some([true, false].map(|value| {
            let mut build = self.clone();
            if is_target_atom(&atom) {
                build
                    .targets
                    .retain(|target| target.contains(&atom) == value);
            } else {
                build.values.insert(atom.clone(), value);
            }
            build
        }))
    }

    pub(super) fn evaluate(&self, predicate: &str) -> Option<bool> {
        let mut missing = None;
        let value = evaluate_cfg(predicate, |atom| {
            let known = if atom == "test" {
                Some(false)
            } else if is_target_atom(atom) {
                let mut values = self.targets.iter().map(|target| target.contains(atom));
                values
                    .next()
                    .and_then(|first| values.all(|value| value == first).then_some(first))
            } else {
                self.values.get(atom).copied()
            };
            if known.is_none() {
                missing.get_or_insert_with(|| atom.to_owned());
            }
            known
        });
        if value.is_none()
            && let Some(missing) = missing
        {
            self.unknown.borrow_mut().get_or_insert(missing);
        }
        value
    }

    fn permits(&self, conditions: &[String]) -> bool {
        !conditions
            .iter()
            .any(|condition| self.evaluate(condition) == Some(false))
    }

    pub(super) fn enabled(&self, attrs: &[syn::Attribute]) -> bool {
        self.permits(&cfg_conditions(attrs))
    }

    pub(super) fn is_disabled(&self, atom: &str) -> bool {
        self.values.get(atom) == Some(&false)
    }
}

fn is_target_atom(atom: &str) -> bool {
    matches!(atom, "unix" | "windows") || atom.starts_with("target_")
}

type ModuleItems = Vec<(Vec<String>, Vec<Item>)>;

const CRATES: &[(&str, &str)] = &[
    ("sunrise_client", ""),
    ("sunrise_client_gameplay", ""),
    ("sunrise_gameplay", ""),
    ("sunrise_gameplay_common", ""),
    ("sunrise_client_lib", ""),
    ("sunrise_client_render", ""),
    ("sunrise_mod_render", ""),
    ("sunrise_mod_vocab", ""),
    ("sunrise_sim", ""),
    ("sunrise_world", ""),
];

/// Index packages by Cargo's library roots (including client-lib's out-of-package root).
/// Cache parsed bodies only for modules the check inspects.
pub(super) struct Sources<'a> {
    root: &'a Path,
    files: HashMap<String, Result<Vec<ModuleVariant>, String>>,
    modules: HashMap<(String, String), ModuleItems>,
}

impl<'a> Sources<'a> {
    pub(super) fn new(root: &'a Path) -> Self {
        Self {
            root,
            files: HashMap::new(),
            modules: HashMap::new(),
        }
    }

    pub(super) fn index<'s>(&'s mut self, build: &'s Build) -> Index<'s> {
        self.binding_index(build, false)
    }

    pub(super) fn binding_index<'s>(&'s mut self, build: &'s Build, types: bool) -> Index<'s> {
        Index::new(self.root, &[], CRATES)
            .loading_functions(move |krate, path| {
                let path = path.join("::");
                let items = self.items(build, krate, &path)?.ok_or_else(|| {
                    format!("module {krate}::{path} was not declared in this build")
                })?;
                Ok(items
                    .filter_map(|item| match item {
                        Item::Fn(item) => Some(Item::Fn(item.clone())),
                        Item::Struct(item) => Some(Item::Struct(item.clone())),
                        Item::Enum(_) | Item::Union(_) | Item::Type(_) | Item::Trait(_)
                            if types =>
                        {
                            Some(item.clone())
                        }
                        Item::Use(item) => Some(Item::Use(item.clone())),
                        Item::Mod(item) => Some(Item::Mod(syn::ItemMod {
                            vis: item.vis.clone(),
                            ident: item.ident.clone(),
                            content: None,
                            attrs: item.attrs.clone(),
                            unsafety: item.unsafety,
                            mod_token: item.mod_token,
                            semi: item.semi,
                        })),
                        _ => None,
                    })
                    .collect())
            })
            .filtering_items(move |item| match item {
                Item::Fn(item) => build.enabled(&item.attrs),
                Item::Struct(item) => build.enabled(&item.attrs),
                Item::Enum(item) => build.enabled(&item.attrs),
                Item::Union(item) => build.enabled(&item.attrs),
                Item::Type(item) => build.enabled(&item.attrs),
                Item::Trait(item) => build.enabled(&item.attrs),
                Item::Use(item) => build.enabled(&item.attrs),
                Item::Mod(item) => build.enabled(&item.attrs),
                _ => true,
            })
    }

    pub(super) fn variants(&mut self, package: &str) -> Result<&[ModuleVariant], String> {
        let root = self.root;
        let parse = |path: &Path, text: &str| {
            let parsed = if package == "sunrise_mod_vocab" {
                syn::parse_file(&crate::exports::expand(root, text)?)
            } else {
                syn::parse_file(text)
            };
            parsed.map_err(|error| format!("{}: {error}", path.display()))
        };
        self.files
            .entry(package.to_owned())
            .or_insert_with(|| {
                let (_, lib) = source::crate_root(root, package)?;
                let dir = lib
                    .parent()
                    .expect("a library file has a folder")
                    .to_owned();
                walk_module_variants(package.to_owned(), lib, dir, false, false, parse)
            })
            .as_ref()
            .map(Vec::as_slice)
            .map_err(Clone::clone)
    }

    pub(super) fn items<'s>(
        &'s mut self,
        build: &'s Build,
        package: &str,
        module: &str,
    ) -> Result<Option<impl Iterator<Item = &'s Item>>, String> {
        let key = (package.to_owned(), module.to_owned());
        if !self.modules.contains_key(&key) {
            let root = self.root;
            let parse = |path: &Path, text: &str| {
                let parsed = if package == "sunrise_mod_vocab" {
                    syn::parse_file(&crate::exports::expand(root, text)?)
                } else {
                    syn::parse_file(text)
                };
                parsed.map_err(|error| format!("{}: {error}", path.display()))
            };
            let files = self.variants(package)?;
            let path = if module.is_empty() {
                package.to_owned()
            } else {
                format!("{package}::{module}")
            };
            let mut variants = Vec::new();
            for variant in files.iter().filter(|v| v.file.module == path) {
                let path = &variant.file.path;
                let text = fs::read_to_string(path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                let mut items = parse(path, &text)?.items;
                for &index in &variant.inline {
                    // The walker recorded these indices in the same parsed file.
                    let Item::Mod(declaration) = items.swap_remove(index) else {
                        unreachable!()
                    };
                    items = declaration.content.expect("recorded inline module").1;
                }
                variants.push((variant.file.cfg.clone(), items));
            }
            self.modules.insert(key.clone(), variants);
        }
        let variants = &self.modules[&key];
        // Resolve aliases and globs only if there isn't a declared module at this path.
        if !variants.iter().any(|(cfg, _)| build.permits(cfg)) {
            return Ok(None);
        }
        Ok(Some(
            variants
                .iter()
                .filter(|(cfg, _)| build.permits(cfg))
                .flat_map(|(_, items)| items),
        ))
    }
}
