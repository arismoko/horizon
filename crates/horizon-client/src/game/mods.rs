//! Mods in the game (`docs/mods.md`): content mods' blocks, items, data and
//! assets (`content_root`), and the Mods page's rows. Code mods are native
//! (`native_hooks.rs`); executable components in content packages are refused.
use super::Game;
use horizon_client::mod_list::{self, Installed};
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct ModsState {
    /// `--dev-mod PATH`: always on.
    pub(super) dev: Vec<PathBuf>,
}

/// Freeze native component IDs before reading content or opening a connection.
/// With no mod registering any, nothing is installed, which every reader
/// treats as no custom components, and `registries.json` isn't read.
pub(super) fn register_components(root: &std::path::Path) -> horizon_client::Result<()> {
    let mut names = Vec::new();
    Game::native_component_types(&mut names);
    if names.is_empty() {
        return Ok(());
    }
    let path =
        horizon_protocol::registry::Builtin::find(root).ok_or("registries.json not found")?;
    let builtin = horizon_protocol::registry::Builtin::load(&path)?;
    let types = builtin
        .get("minecraft:data_component_type")
        .ok_or("missing data component registry")?;
    let first = i32::try_from(types.names.len())?;
    horizon_protocol::item::custom::install(first, names)
        .map_err(|error| format!("invalid native component registration: {error}"))?;
    Ok(())
}

/// The blocks and items native mods declare in code (`native_blocks`,
/// `native_items`): asked once, so every content root of this run, a pack
/// reload's included, has the same ones.
fn native_declared() -> &'static (
    Vec<horizon_mods::content::Block>,
    Vec<horizon_mods::content::Item>,
) {
    static DECLARED: std::sync::OnceLock<(
        Vec<horizon_mods::content::Block>,
        Vec<horizon_mods::content::Item>,
    )> = std::sync::OnceLock::new();
    DECLARED.get_or_init(|| {
        let (mut blocks, mut items) = (Vec::new(), Vec::new());
        Game::native_blocks(&mut blocks);
        Game::native_items(&mut items);
        (blocks, items)
    })
}
/// The native mods' standalone models (`native_standalone_models`), set once
/// before the resources first load.
pub(super) fn register_standalone_models() {
    let mut names = Vec::new();
    Game::native_standalone_models(&mut names);
    horizon_client::models::register_standalone(names);
}

/// `root` with the content mods that are on layered over it
/// (docs/modpacks.md): their blocks, items, data and assets. Read once at
/// start, before the resources load; a mod whose content is refused is left
/// out here and its row on the Mods page says why.
pub(super) fn content_root(
    root: &std::path::Path,
    enabled: &std::collections::BTreeSet<String>,
    dev: &[std::path::PathBuf],
) -> horizon_client::Result<std::path::PathBuf> {
    use horizon_client::mod_content::{self, ContentMod};
    use horizon_mods::Content;
    let here = std::env::current_dir().unwrap_or_default();
    let built = std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::read_to_string(exe.with_file_name(BUILD_INFO)).ok())
        .map(|info| built_ids(&info))
        .unwrap_or_default();
    let mut known = None;
    let mut mods: Vec<ContentMod> = Vec::new();
    let announce = !horizon_protocol::mod_list::is_set();
    let mut announced = Vec::new();
    for selected in horizon_mods::selection::packages(&here, enabled, &built, dev) {
        let path = &selected.path;
        let m = &selected.package.manifest;
        if mods.iter().any(|c| c.id == m.id) {
            continue;
        }
        // The items this mod's native code declares may be a file item's crafting remainder.
        let declared: Vec<String> = native_declared()
            .1
            .iter()
            .filter(|i| i.id.split_once(':').is_some_and(|(ns, _)| ns == m.id))
            .map(|i| i.id.clone())
            .collect();
        let content = match Content::read_declaring(path, m, &declared) {
            Ok(content) => content,
            Err(e) => {
                log::warn!("Horizon: mod {} content not loaded: {e}", m.id);
                continue;
            }
        };
        let known = match &mut known {
            Some(types) => types,
            None => match mod_content::known_components(root) {
                Ok(types) => known.insert(types),
                Err(e) => {
                    log::warn!("Horizon: content mods not loaded: {e}");
                    return Err(e);
                }
            },
        };
        let candidate = ContentMod {
            id: m.id.clone(),
            content,
        };
        match mod_content::accept_mod(&mut mods, candidate, known) {
            Ok(_) if announce => announced.push(announced_entry(m)),
            Ok(_) => {}
            Err(e) => log::warn!("Horizon: mod {} content not loaded: {e}", m.id),
        }
    }
    let (blocks, items) = native_declared();
    if !blocks.is_empty() || !items.is_empty() {
        if known.is_none() {
            match mod_content::known_components(root) {
                Ok(types) => known = Some(types),
                Err(e) => log::warn!("Horizon: native mods' blocks and items not loaded: {e}"),
            }
        }
        if let Some(known) = &known {
            for e in mod_content::accept_declared(&mut mods, blocks, items, known) {
                log::warn!("Horizon: {e}");
            }
            // A file item's remainder named a code-declared item that was refused: that mod
            // alone is left out, not every content mod through the overlay's check.
            for e in mod_content::drop_dangling_remainders(&mut mods) {
                log::warn!("Horizon: {e}");
            }
        }
    }
    let out = mod_content::resource_root(root, &mods, &here.join(".horizon-cache"))?;
    if announce {
        horizon_protocol::mod_list::set_local(announced);
    }
    if out != root {
        log::info!(
            "Horizon: content from {} mod(s) in {}",
            mods.len(),
            out.display()
        );
    }
    register_mob_effects(&mods);
    register_entity_types(&mods);
    register_menu_types(&mods);
    Ok(out)
}

/// Manifest identity for the configuration-phase mod check.
fn announced_entry(m: &horizon_mods::Manifest) -> horizon_protocol::mod_list::Entry {
    use horizon_mods::Side;
    use horizon_protocol::mod_list;
    mod_list::Entry {
        id: m.id.clone(),
        version: m.version.clone(),
        side: match m.side {
            Side::Both => mod_list::Side::Both,
            Side::Client => mod_list::Side::Client,
        },
    }
}

/// The mods' mob effects join vanilla's (`brewing::effect`), once the root
/// holding their network ids is built. The startup root's effects, none
/// included, stay until restart: a pack reload after a Mods switch builds a
/// root with others.
fn register_mob_effects(mods: &[horizon_client::mod_content::ContentMod]) {
    use horizon_client::brewing::{self, Category};
    use horizon_mods::content::EffectCategory;
    let effects = horizon_client::mod_content::mob_effects(mods);
    let ids: Vec<String> = effects.iter().map(|e| e.id.clone()).collect();
    let set = brewing::register_mod_effects(effects.into_iter().map(|e| {
        let category = match e.category {
            EffectCategory::Beneficial => Category::Beneficial,
            EffectCategory::Harmful => Category::Harmful,
            EffectCategory::Neutral => Category::Neutral,
        };
        (e.id.clone(), e.color, category)
    }));
    if !set
        && brewing::mod_effects()
            .iter()
            .map(|e| e.id)
            .ne(ids.iter().map(String::as_str))
    {
        log::warn!("Horizon: the mods' mob effects changed; restart the game to use them");
    }
}

/// the mods' thrown entity types, as their mob effects: set once, from the
/// startup root.
fn register_entity_types(mods: &[horizon_client::mod_content::ContentMod]) {
    use horizon_client::projectiles;
    let types = horizon_client::mod_content::entity_types(mods);
    let set = projectiles::register_thrown_types(
        types
            .iter()
            .map(|t| (t.id.clone(), t.item.clone(), t.damage)),
    );
    if !set
        && projectiles::thrown_types()
            .iter()
            .map(|t| t.id)
            .ne(types.iter().map(|t| t.id.as_str()))
    {
        log::warn!("Horizon: the mods' entity types changed; restart the game to use them");
    }
}

/// the mods' menu types, as their entity types: set once, from the startup
/// root.
fn register_menu_types(mods: &[horizon_client::mod_content::ContentMod]) {
    use horizon_client::containers;
    let menus = horizon_client::mod_content::menu_types(mods);
    let set = containers::register_mod_menu_types(menus.iter().map(|m| {
        let results = m.results.iter().map(|&r| usize::from(r)).collect();
        (m.id.clone(), m.slots.into(), m.data_slots.into(), results)
    }));
    if !set
        && containers::mod_menu_types()
            .iter()
            .map(|m| m.id)
            .ne(menus.iter().map(|m| m.id.as_str()))
    {
        log::warn!("Horizon: the mods' menu types changed; restart the game to use them");
    }
}

/// What the launcher writes beside a game it built with native mods: their ids under `mods`.
const BUILD_INFO: &str = "native-mods.json";

/// IDs compiled into this executable, from the launcher's build record.
fn built_ids(built: &str) -> std::collections::BTreeSet<String> {
    let info: serde_json::Value = serde_json::from_str(built).unwrap_or_default();
    info["mods"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str().map(str::to_owned))
        .collect()
}

/// `native-mods.json`'s `mods` list is not empty: the game was built with mods.
fn build_lists_mods(built: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(built)
        .ok()
        .and_then(|info| info["mods"].as_array().map(|mods| !mods.is_empty()))
        .unwrap_or(false)
}

/// Whether the game playing was built with native mods: the launcher writes their ids into
/// [`BUILD_INFO`] beside the executable, and a release or Nightly has no such file.
pub(super) fn built_with_native_mods() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::read_to_string(exe.with_file_name(BUILD_INFO)).ok())
        .is_some_and(|built| build_lists_mods(&built))
}

/// The native mods the launcher built this game with, `id version, id version`, for the crash
/// report (`built`: its [`BUILD_INFO`]). None for a release or Nightly, which have no mods.
fn native_mods_detail(built: &str) -> Option<String> {
    let info: serde_json::Value = serde_json::from_str(built).unwrap_or_default();
    let list = info["mods"]
        .as_array()?
        .iter()
        .map(|m| {
            let text = |key: &str| m[key].as_str().unwrap_or("?").to_owned();
            format!("{} {}", text("id"), text("version"))
        })
        .collect::<Vec<_>>()
        .join(", ");
    (!list.is_empty()).then_some(list)
}

/// A row for each native mod the launcher installed in `dir` (`<id>.hmod`) that this game wasn't
/// built with (`built`: its [`BUILD_INFO`], empty for a release or Nightly) and that has no row
/// yet. The page said "No mods" before, and players copied the files into `horizon-mods` (#4770).
/// Its toggle is the `mod.<id>` line the launcher's next build follows.
fn not_built(dir: &std::path::Path, built: &str, rows: &[Installed]) -> Vec<Installed> {
    let in_build = built_ids(built);
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "hmod"));
    paths.sort();
    let mut out: Vec<Installed> = Vec::new();
    for path in paths {
        let Ok(bundle) = horizon_mods::index::native::Bundle::read(&path) else {
            continue;
        };
        let m = bundle.manifest;
        let listed = rows.iter().chain(&out).any(|row| row.id == m.id);
        if !listed && !in_build.contains(&m.id) {
            out.push(Installed {
                title: format!("{} {}", m.id, m.version),
                id: m.id,
                description: String::new(),
                problem: Some(mod_list::NOT_BUILT.to_owned()),
                dev: false,
                native: None,
            });
        }
    }
    out
}

impl Game<'_> {
    /// Lists the installed mods on the Mods page. Content mods load with the resources
    /// (`content_root`); native ones are built into the game (`native_hooks.rs`). Unsupported
    /// files get the same unreadable-file row as any other invalid package.
    pub(super) fn start_mods(&mut self) {
        let dir = Self::game_dir().join(mod_list::DIR);
        let mut paths = horizon_mods::find(&dir);
        paths.retain(|p| !self.mods.dev.contains(p));
        paths.extend(self.mods.dev.iter().cloned());
        self.settings.mods.installed = paths
            .iter()
            .map(|path| {
                let file = path.file_name().unwrap_or_default().to_string_lossy();
                let package = match horizon_mods::Package::read(path) {
                    Ok(package) => package,
                    Err(e) => {
                        return Installed {
                            id: String::new(),
                            title: file.into_owned(),
                            description: String::new(),
                            problem: Some(format!("Can't be read: {e}")),
                            dev: false,
                            native: None,
                        };
                    }
                };
                let m = &package.manifest;
                Installed {
                    id: m.id.clone(),
                    title: format!("{} {}", m.name, m.version),
                    description: m.description.clone(),
                    problem: None,
                    dev: self.mods.dev.contains(path),
                    native: None,
                }
            })
            .collect();
        // A mod with a native version built in says so instead.
        self.list_native_mods();
        let built = std::env::current_exe()
            .ok()
            .and_then(|exe| std::fs::read_to_string(exe.with_file_name(BUILD_INFO)).ok())
            .unwrap_or_default();
        let built_mods = native_mods_detail(&built);
        let built_with_mods = built_mods.is_some();
        if let Some(list) = built_mods {
            horizon_client::crash::set_detail("Native Mods", list);
        }
        horizon_client::gfx::native_render::set_mods_built_in(
            built_with_mods
                || self
                    .settings
                    .mods
                    .installed
                    .iter()
                    .any(|m| m.native.is_some()),
        );
        let native_dir = Self::game_dir().join(mod_list::NATIVE_DIR);
        let unbuilt = not_built(&native_dir, &built, &self.settings.mods.installed);
        self.settings.mods.installed.extend(unbuilt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Only a build whose `native-mods.json` lists a mod counts as built with mods.
    #[test]
    fn only_a_listed_mod_makes_a_build_a_mod_build() {
        assert!(!build_lists_mods(""));
        assert!(!build_lists_mods("{}"));
        assert!(!build_lists_mods(r#"{"mods":[]}"#));
        assert!(!build_lists_mods(r#"{"mods":"worldmap"}"#));
        assert!(build_lists_mods(r#"{"mods":[{"id":"worldmap"}]}"#));
    }

    /// A minimal flat native `.hmod`.
    fn hmod(dir: &std::path::Path, id: &str) {
        let manifest = format!(
            "id = '{id}'\nname = '{id}'\nversion = '0.3.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let files: &[(&str, &[u8])] = &[
            ("Cargo.toml", b"[package]\nname='example'\nversion='1'\n"),
            ("src/lib.rs", b"pub fn hooks() {}"),
        ];
        let bytes = horizon_mods::Package::write_files(&manifest, files).unwrap();
        std::fs::write(dir.join(format!("{id}.hmod")), bytes).unwrap();
    }

    #[test]
    fn the_crash_report_lists_the_builds_native_mods() {
        let built = r#"{"mods":[{"id":"machine_sample","version":"0.1.0","sha256":null},{"id":"pipes","version":"2.0"}]}"#;
        assert_eq!(
            native_mods_detail(built).as_deref(),
            Some("machine_sample 0.1.0, pipes 2.0")
        );
        // A release has no file; a build without mods has an empty list.
        assert_eq!(native_mods_detail(""), None);
        assert_eq!(native_mods_detail(r#"{"mods":[]}"#), None);
    }

    #[test]
    fn native_mods_installed_but_not_built_get_a_row_saying_so() {
        let dir = std::env::temp_dir().join(format!("horizon-not-built-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for id in ["veinminer", "zoom", "minimap"] {
            hmod(&dir, id);
        }
        std::fs::write(dir.join("notes.txt"), "not a mod").unwrap();
        std::fs::write(dir.join("broken.hmod"), "not a zip").unwrap();
        // A release: none is built in; the page lists them, sorted, instead of "No mods".
        let rows = not_built(&dir, "", &[]);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["minimap", "veinminer", "zoom"]);
        assert_eq!(rows[1].title, "veinminer 0.3.0");
        assert_eq!(rows[1].problem.as_deref(), Some(mod_list::NOT_BUILT));
        assert!(rows.iter().all(|r| r.native.is_none() && !r.dev));
        // A native build with zoom, and minimap's row already listed by its own hook.
        let built =
            r#"{"base":"nightly-2026-10-30-1da354b","mods":[{"id":"zoom","version":"0.3.0"}]}"#;
        let listed = Installed {
            id: "minimap".into(),
            ..rows[0].clone()
        };
        let rows = not_built(&dir, built, &[listed]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "veinminer");
        // No folder: no rows.
        assert!(not_built(&dir.join("missing"), "", &[]).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
