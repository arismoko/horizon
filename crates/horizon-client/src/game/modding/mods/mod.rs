// game-module-visibility: pub(crate)
//! Mods in the game (`docs/modding/src/how-to/content-mods.md`): content mods' blocks, items, data and
//! assets (`content_root`), and the Mods page's rows. Code mods are native (`native_hooks.rs`)
//! or wasm (`horizon-mod-wasm`, loaded at startup); any other executable file is refused.
use super::{Game, execution_role::ExecutionRole};
use horizon_client::{
    lang,
    mod_list::{self, Installed},
};
use horizon_mod_package::manifest::{MissingJar, collect_jar_requirements, missing_jars};
use horizon_world::{
    item_defaults::ItemData,
    native_load_context::{LoadContext, LoadInputs},
};
use mod_api::__private::game::*;
use mod_api::RegistrationContext;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub(super) struct CodeSwitches {
    disabled: std::collections::BTreeSet<String>,
    applied: std::sync::atomic::AtomicBool,
}

#[derive(Clone, Default)]
pub(super) struct ModsState {
    /// Saved switches gate the first data inventory while registration keeps
    /// its existing startup ordering. Runtime dispatch owns later switches.
    pub(super) code_switches: Option<Arc<CodeSwitches>>,
    /// The original prepared input is fixed for this engine load, shared by
    /// the window and its server. A later data reload must not overlay an
    /// already generated root recursively.
    pub(super) content_base: Arc<std::sync::OnceLock<PathBuf>>,
    /// `--dev-mod PATH`: content mods always on.
    pub(super) dev: Vec<PathBuf>,
    pub(super) native: std::sync::Arc<horizon_mod_package::runtime::Runtime>,
    pub(super) selected: std::sync::Arc<[horizon_mod_package::selection::SelectedPackage]>,
    /// The content mods the loaded resources were built with: a switch the
    /// game can apply live moves it to the player's choice (`switch_mods`).
    pub(super) applied: Option<std::collections::BTreeSet<String>>,
    /// A live switch's resource reload is running: when it applied, the server
    /// reloads its data from the new root. Its start, for the log.
    pub(super) live: Option<std::time::Instant>,
    /// The content mods installed, and those of them with only assets and
    /// data (`mod_content::live_switch`): they switch with a world open.
    pub(super) content_ids: std::collections::BTreeSet<String>,
    pub(super) live_ids: std::collections::BTreeSet<String>,
    /// The server's data comes from a root a live switch built, not the one
    /// `res` was loaded from: the world tables are built from it even with no
    /// data pack (`build_world_tables`).
    pub(super) data_switched: bool,
    /// Structured load refusals, rendered only at the existing Mods-page boundary.
    pub(super) refused: Arc<Mutex<Vec<MissingJar>>>,
}

impl ModsState {
    pub(super) fn capture_code_switches(&mut self, disabled: &std::collections::BTreeSet<String>) {
        self.code_switches = Some(Arc::new(CodeSwitches {
            disabled: disabled.clone(),
            applied: Default::default(),
        }));
    }

    fn apply_code_switches(&self, disabled: &std::collections::BTreeSet<String>) {
        for id in mod_api::__private::installed_code_ids() {
            mod_api::__private::set_mod_enabled(id, !disabled.contains(id));
        }
        if let Some(switches) = &self.code_switches {
            switches
                .applied
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }

    /// One admission rule for initial composition and later code-only toggles.
    /// Content acceptance belongs to the generation; code admission belongs to
    /// the installed dispatch entries, including their live enabled switches.
    fn enabled_inventory(
        &self,
        accepted_content: impl Fn(&str) -> bool,
    ) -> horizon_mod_package::enabled::EnabledMods {
        let native = self
            .native
            .packages()
            .iter()
            .map(|package| (&package.manifest, true));
        let selected = self.selected.iter().map(|selected| {
            (
                &selected.package.manifest,
                selected.package.manifest.wasm.is_some(),
            )
        });
        horizon_mod_package::enabled::EnabledMods::new(native.chain(selected).filter_map(
            |(manifest, code)| {
                let enabled = if code {
                    let initially_disabled = self.code_switches.as_ref().is_some_and(|switches| {
                        !switches.applied.load(std::sync::atomic::Ordering::Acquire)
                            && switches.disabled.contains(&manifest.id)
                    });
                    !initially_disabled
                        && mod_api::__private::mod_index(&manifest.id)
                            .is_some_and(mod_api::__private::mod_index_on)
                } else {
                    accepted_content(&manifest.id)
                };
                enabled.then(|| (manifest.id.clone(), manifest.side))
            },
        ))
    }

    pub(super) fn select(
        &mut self,
        game_dir: &std::path::Path,
        enabled: &std::collections::BTreeSet<String>,
    ) {
        self.selected =
            horizon_mod_package::selection::content_packages(game_dir, enabled, &self.dev)
                .filter(|selected| {
                    let id = &selected.package.manifest.id;
                    // A refused wasm mod's content stays out with its code.
                    !self.native.built().iter().any(|built| built.id == *id)
                        && !horizon_mod_wasm::refused()
                            .iter()
                            .any(|(refused, _)| refused == id)
                })
                .collect();
        self.refused = Default::default();
    }

    fn report_refusals(&self, rows: &mut [Installed]) {
        let refused = self.refused.lock().unwrap();
        for row in rows {
            let problems: Vec<_> = refused
                .iter()
                .filter(|missing| missing.owners.contains(&row.id))
                .map(ToString::to_string)
                .collect();
            if !problems.is_empty() {
                row.problem = Some(problems.join("\n"));
            }
        }
    }
}

/// Capture load inputs at the resource-generation boundary, before workers share them.
pub(super) fn load_item_data(root: &Path, role: ExecutionRole) -> horizon_client::Result<ItemData> {
    let phase = mod_api::__private::PhaseOwner::scope(if role.is_client() {
        mod_api::__private::ModPhase::Setup
    } else {
        mod_api::__private::ModPhase::World
    });
    let _reload = phase.reloading()?;
    let mut registries: BTreeMap<_, _> = builtin_registries(root)?
        .registries
        .into_iter()
        .map(|(name, ids)| (name, ids.names))
        .collect();
    if let Some(types) = registries.get_mut("minecraft:data_component_type") {
        types.extend(horizon_protocol::item::custom::entries().map(|(_, name)| name.to_owned()));
    }
    let inputs = LoadInputs::new(registries, Game::mod_data_root());
    let inventory = horizon_mod_package::enabled::EnabledMods::read(root)?;
    let ids: Vec<_> = if role.is_logical_server() {
        inventory.server_ids().collect()
    } else {
        inventory.client_ids().collect()
    };
    let context = LoadContext::new(role.data_load_origin(), Arc::new(inputs), ids);

    Ok(ItemData::load(root, context)?)
}

fn builtin_registries(root: &Path) -> horizon_client::Result<horizon_protocol::registry::Builtin> {
    let path = horizon_protocol::registry::Builtin::find(root).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "registries.json not found")
    })?;
    Ok(horizon_protocol::registry::Builtin::load(&path)?)
}

/// Freeze native component IDs before reading content or opening a connection.
/// With no mod registering any, nothing is installed, which every reader
/// treats as no custom components, and `registries.json` isn't read.
pub(super) fn register_components(root: &std::path::Path) -> horizon_client::Result<()> {
    let mut names = Vec::new();
    Game::native_component_types(&RegistrationContext::new(), &mut names);
    if names.is_empty() {
        return Ok(());
    }
    let builtin = builtin_registries(root)?;
    let types = builtin
        .get("minecraft:data_component_type")
        .ok_or("missing data component registry")?;
    let first = i32::try_from(types.names.len())?;
    horizon_protocol::item::custom::install(first, names)
        .map_err(|error| format!("invalid native component registration: {error}"))?;
    Ok(())
}

/// Finish every once-only content declaration on the registration thread.
pub(super) fn freeze_declarations() {
    let _ = native_declared();
}

struct NativeDeclared {
    blocks: Vec<horizon_mod_package::content::Block>,
    items: Vec<horizon_mod_package::content::Item>,
    types: Vec<horizon_mod_package::content::BlockEntityType>,
}

/// The same native declarations serve startup and every pack reload.
fn native_declared() -> &'static NativeDeclared {
    static DECLARED: std::sync::OnceLock<NativeDeclared> = std::sync::OnceLock::new();
    DECLARED.get_or_init(|| {
        let (mut blocks, mut items) = (Vec::new(), Vec::new());
        Game::native_blocks(&RegistrationContext::new(), &mut blocks);
        Game::native_items(&RegistrationContext::new(), &mut items);
        let types = super::api_values::block_entity_declarations();
        NativeDeclared {
            blocks,
            items,
            types,
        }
    })
}
/// The native mods' standalone models (`native_standalone_models`), set once
/// before the resources first load.
pub(super) fn register_standalone_models(role: ExecutionRole) {
    if let Some(names) = collect_standalone_models(role) {
        horizon_client::models::register_standalone(names);
    }
}

fn collect_standalone_models(role: ExecutionRole) -> Option<Vec<String>> {
    if !role.is_client() {
        return None;
    }
    let mut names = Vec::new();
    Game::native_standalone_models(&RegistrationContext::new(), &mut names);
    Some(names)
}
pub(super) fn collect_key_mappings<'declared>(
    role: ExecutionRole,
    declared: &'declared [(&'static str, mod_api::NativeKeyRegistration)],
) -> &'declared [(&'static str, mod_api::NativeKeyRegistration)] {
    if role.is_client() { declared } else { &[] }
}

/// The native mods' entity model layers (`native_layer_definitions`), kept
/// once at start for `bake_layer`.
pub(super) fn register_layer_definitions(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut layers = Vec::new();
    Game::native_layer_definitions(&RegistrationContext::new(), &mut layers);
    horizon_client::mobs::model_layers::register(layers);
}
/// The native mods' F3 debug entries (`native_register_debug_entries`), kept once at start.
pub(super) fn register_debug_entries(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut entries = Vec::new();
    Game::native_register_debug_entries(&RegistrationContext::new(), &mut entries);
    horizon_client::debug_entries::register(entries);
}
/// The native mods' debug renderers now (`native_register_debug_renderers`), asked at each
/// refresh of the list (`Game::refresh_debug_renderers`).
pub(super) fn debug_renderers(role: ExecutionRole) -> Vec<mod_api::DebugRenderer> {
    let mut renderers = Vec::new();
    if role.is_client() {
        Game::native_register_debug_renderers(&RegistrationContext::new(), &mut renderers);
    }
    renderers
}
/// The native mods' numeric item model properties
/// (`native_register_range_select_item_model_property`), kept once at start, before
/// resources load, so `range_dispatch` models on them read the mod's number.
pub(super) fn register_range_select_item_model_properties(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut properties = Vec::new();
    Game::native_register_range_select_item_model_property(
        &RegistrationContext::new(),
        &mut properties,
    );
    horizon_assets::item_model::register_range_properties(properties);
}
/// The native mods' categorical item model properties
/// (`native_register_select_item_model_property`), kept once at start, before resources
/// load, so `select` models on them read the mod's value.
pub(super) fn register_select_item_model_properties(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut properties = Vec::new();
    Game::native_register_select_item_model_property(&RegistrationContext::new(), &mut properties);
    horizon_assets::item_model::register_select_properties(properties);
}
/// The native mods' item tint sources (`native_register_item_tint_sources`), kept once at
/// start, before resources load, so `tints` entries on them read the mod's colour.
pub(super) fn register_item_tint_sources(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut sources = Vec::new();
    Game::native_register_item_tint_sources(&RegistrationContext::new(), &mut sources);
    horizon_assets::item_model::register_tint_sources(sources);
}
/// The native mods' block tint sources (`native_register_block_tint_sources`), kept once at
/// start, before resources load; every resource load applies them to its mesh tables.
pub(super) fn register_block_tint_sources(role: ExecutionRole) {
    if !role.is_client() {
        return;
    }
    let mut registry = mod_api::BlockTintRegistry::default();
    Game::native_register_block_tint_sources(&RegistrationContext::new(), &mut registry);
    horizon_client::mesher::register_block_tint_sources(registry.into_layers());
}

/// `root` with the content mods that are on layered over it
/// (docs/modpacks.md): their blocks, items, data and assets. Read once at
/// start, before the resources load; a mod whose content is refused is left
/// out here and its row on the Mods page says why.
/// The merged content root and the accepted content mods behind it, before
/// any process-wide registration. A caller that only reads the overlay (the
/// reload tests) must not freeze the mod registries for the rest of the
/// process.
pub(super) struct ContentOverlay {
    /// The root with the mods' content merged in.
    pub(super) root: horizon_assets::resource_source::ResourceRoot,
    /// The accepted content mods, in overlay order.
    pub(super) mods: Vec<horizon_client::mod_content::ContentMod>,
    /// The protocol entries of the accepted mods.
    pub(super) announced: Vec<horizon_protocol::mod_list::Entry>,
    /// Whether the protocol's local mod list is still unset.
    pub(super) announce: bool,
}

/// Builds the content overlay: every enabled or dev content mod's files
/// merged over `root`. Registration of what the mods declare is
/// [`content_root`]'s step.
pub(super) fn content_overlay(
    root: &std::path::Path,
    sources: &ModsState,
    jar_store: &std::path::Path,
) -> horizon_client::Result<ContentOverlay> {
    use horizon_client::mod_content::{self, ContentMod};
    use horizon_mod_package::Content;
    let here = std::env::current_dir().unwrap_or_default();
    let original_base = root.to_owned();

    let requirements = collect_jar_requirements(
        sources
            .native
            .packages()
            .iter()
            .map(|p| &p.manifest)
            .chain(sources.selected.iter().map(|p| &p.package.manifest))
            .map(|m| (m.id.as_str(), m.jar.as_slice())),
    )?;
    let refused = missing_jars(jar_store, &requirements)?;
    *sources.refused.lock().unwrap() = refused.clone();
    if let Some(missing) = refused.iter().find(|missing| {
        missing
            .owners
            .iter()
            .any(|id| sources.native.built().iter().any(|built| built.id == id))
    }) {
        return Err(missing.clone().into());
    }

    let mut known = None;
    let mut mods: Vec<ContentMod> = Vec::new();
    let announce = !horizon_protocol::mod_list::is_set();
    let mut announced: Vec<_> = if announce {
        sources
            .native
            .built()
            .iter()
            .map(|m| announced_entry(m.id, m.version, m.side))
            .collect()
    } else {
        Vec::new()
    };
    struct PackageSource<'a> {
        path: &'a std::path::Path,
        manifest: &'a horizon_mod_package::Manifest,
        content_only: bool,
    }
    let native = sources.native.packages().iter().map(|p| PackageSource {
        path: &p.path,
        manifest: &p.manifest,
        content_only: false,
    });
    let content = sources.selected.iter().map(|p| PackageSource {
        path: &p.path,
        manifest: &p.package.manifest,
        content_only: true,
    });
    let mut seen = std::collections::HashSet::new();
    let packages: Vec<_> = native
        .chain(content)
        .filter(|source| {
            !refused
                .iter()
                .any(|missing| missing.owners.contains(&source.manifest.id))
        })
        .filter(|source| seen.insert(source.manifest.id.as_str()))
        .collect();
    let dependencies: Vec<_> = packages
        .iter()
        .map(|source| {
            horizon_mod_package::dependencies::from_manifest(&source.manifest.dependencies)
        })
        .collect();
    let inputs: Vec<_> = packages
        .iter()
        .zip(&dependencies)
        .map(
            |(source, dependencies)| horizon_mod_package::dependencies::Mod {
                id: &source.manifest.id,
                version: &source.manifest.version,
                has_api: false,
                dependencies,
            },
        )
        .collect();
    let order = horizon_mod_package::dependencies::resolve_content(&inputs)
        .map_err(|error| error.to_string())?
        .order;
    for index in order {
        let source = &packages[index];
        let m = source.manifest;
        // The items this mod's native code declares may be a file item's crafting remainder.
        let declared: Vec<String> = native_declared()
            .items
            .iter()
            .filter(|i| i.id.split_once(':').is_some_and(|(ns, _)| ns == m.id))
            .map(|i| i.id.clone())
            .collect();
        let content = match Content::read_declaring(source.path, m, &declared) {
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
            jars: m.jar.clone(),
        };
        match mod_content::accept_mod(&mut mods, candidate, known) {
            Ok(_) if announce && source.content_only => {
                announced.push(announced_entry(&m.id, &m.version, m.side))
            }
            Ok(_) => {}
            Err(e) => log::warn!("Horizon: mod {} content not loaded: {e}", m.id),
        }
    }
    let NativeDeclared {
        blocks,
        items,
        types,
    } = native_declared();
    if !blocks.is_empty() || !items.is_empty() || !types.is_empty() {
        let known = match &mut known {
            Some(types) => types,
            None => known.insert(mod_content::known_components(root)?),
        };
        let errors = mod_content::accept_declared(&mut mods, blocks, items, types, known);
        if !errors.is_empty() {
            return Err(errors.join("\n").into());
        }
        // A file item's remainder named a code-declared item that was refused: that mod
        // alone is left out, not every content mod through the overlay's check.
        for e in mod_content::drop_dangling_remainders(&mut mods) {
            log::warn!("Horizon: {e}");
        }
    }
    // Content validation may refuse a package. Revalidate against what really
    // loaded so dependents cannot start with a missing required content mod.
    let loaded: Vec<_> = inputs
        .into_iter()
        .filter(|package| {
            mods.iter().any(|content| content.id == package.id)
                || sources
                    .native
                    .built()
                    .iter()
                    .any(|native| native.id == package.id)
        })
        .collect();
    horizon_mod_package::dependencies::resolve_content(&loaded)
        .map_err(|error| error.to_string())?;
    let cache = here.join(".horizon-cache");
    // Content admission and the live code-dispatch boundary jointly determine
    // what loaded. Namespaces and raw startup metadata cannot answer this.
    let inventory = sources.enabled_inventory(|id| mods.iter().any(|loaded| loaded.id == id));
    let prepared = mod_content::resource_root(root, jar_store, &mods, &cache, &inventory)?;
    let root = mod_content::resolved_resources(root, prepared, jar_store, &mods, cache)?;
    let _ = sources.content_base.set(original_base);
    Ok(ContentOverlay {
        root,
        mods,
        announced,
        announce,
    })
}

/// The content root with everything the mods declare registered against the
/// process: block entities, mob effects, entity and menu types, fluid looks,
/// and the protocol's local mod list, each set once.
pub(super) fn content_root(
    root: &std::path::Path,
    sources: &ModsState,
) -> horizon_client::Result<horizon_assets::resource_source::ResourceRoot> {
    let jar_store = horizon_client::logging::game_home().join("jars");
    let overlay = content_overlay(root, sources, &jar_store)?;
    register_block_entity_types(overlay.root.prepared(), &overlay.mods)?;
    if overlay.announce {
        horizon_protocol::mod_list::set_local(overlay.announced);
    }
    if overlay.root.prepared() != root {
        log::info!(
            "Horizon: content from {} mod(s) in {}",
            overlay.mods.len(),
            overlay.root.prepared().display()
        );
    }
    register_mob_effects(&overlay.mods);
    mod_api::__private::resolve_effects(
        horizon_client_core::potions::all_effects()
            .map(|def| horizon_client_core::potions::namespaced(def.id)),
        |id| horizon_client_core::potions::effect(id).map(|def| def.index),
    )
    .map_err(|errors| errors.join("\n"))?;
    let behavior: serde_json::Value =
        horizon_world::json_file::read(&overlay.root.prepared().join("block-behaviours.json"))?;
    let fluids = behavior["fluids"]
        .as_array()
        .ok_or("missing fluid registry")?;
    let fluids = fluids
        .iter()
        .map(|value| value.as_str().ok_or("invalid fluid registry entry"))
        .collect::<Result<Vec<_>, _>>()?;
    mod_api::__private::resolve_fluids(&fluids).map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    register_entity_types(&overlay.mods);
    register_menu_types(&overlay.mods);
    horizon_client::mod_content::set_fluid_looks(&overlay.mods);
    Ok(overlay.root)
}

fn register_block_entity_types(
    root: &std::path::Path,
    mods: &[horizon_client::mod_content::ContentMod],
) -> horizon_client::Result<()> {
    use horizon_client::block_entity_types::{self, BlockEntityTypes};
    use horizon_protocol::registry::Builtin;
    let path = Builtin::find(root).ok_or("registries.json not found")?;
    let builtin = Builtin::load(&path)?;
    let names = &builtin
        .get("minecraft:block_entity_type")
        .ok_or("missing block entity type registry")?
        .names;
    let membership = mods
        .iter()
        .flat_map(|m| &m.content.block_entity_types)
        .flat_map(|kind| {
            kind.blocks
                .iter()
                .map(|block| (block.clone(), kind.id.clone()))
        })
        .collect();
    let types = BlockEntityTypes::new(membership, names.clone());
    mod_api::__private::resolve_block_entity_types(types.names(), |id| types.key(id))
        .map_err(|errors| errors.join("\n"))?;
    block_entity_types::install(types)?;
    Ok(())
}

/// Manifest identity for the configuration-phase mod check.
fn announced_entry(
    id: &str,
    version: &str,
    side: horizon_mod_package::Side,
) -> horizon_protocol::mod_list::Entry {
    use horizon_mod_package::Side;
    use horizon_protocol::mod_list;
    mod_list::Entry {
        id: id.to_owned(),
        version: version.to_owned(),
        side: match side {
            Side::Both => mod_list::Side::Both,
            Side::Client => mod_list::Side::Client,
        },
    }
}

/// The mods' mob effects join vanilla's (`brewing::effect`), once the root
/// holding their network ids is built. The first root that declares effects
/// sets them, until restart: a pack reload after a Mods switch builds a root
/// with others. A root declaring none freezes nothing, and a game without
/// mob effects reads the same empty list either way.
fn register_mob_effects(mods: &[horizon_client::mod_content::ContentMod]) {
    use horizon_client::brewing::{self, Category};
    use horizon_mod_package::content::EffectCategory;
    let effects = horizon_client::mod_content::mob_effects(mods);
    if effects.is_empty() {
        return;
    }
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
/// startup root. Living types are mobs, loaded with vanilla's from the
/// content root's `entity-types.json`.
fn register_entity_types(mods: &[horizon_client::mod_content::ContentMod]) {
    use horizon_client::projectiles;
    let mut types = horizon_client::mod_content::entity_types(mods);
    types.retain(|t| t.item.is_some());
    let set = projectiles::register_thrown_types(
        types
            .iter()
            .filter_map(|t| Some((t.id.clone(), t.item.clone()?, t.damage))),
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

/// The embedded inventory, formatted for the crash report.
fn native_mods_detail(built: &[horizon_mod_package::runtime::BuiltMod]) -> Option<String> {
    let list = built
        .iter()
        .map(|m| format!("{} {}", m.id, m.version))
        .collect::<Vec<_>>()
        .join(", ");
    (!list.is_empty()).then_some(list)
}

/// Notes a content mod's id, and whether its files can switch with a world
/// open (`mod_content::live_switch`).
fn live_switch_of(
    path: &std::path::Path,
    package: &horizon_mod_package::Package,
    content_ids: &mut std::collections::BTreeSet<String>,
    live_ids: &mut std::collections::BTreeSet<String>,
) {
    if package.is_native() || package.manifest.wasm.is_some() {
        return;
    }
    let id = &package.manifest.id;
    content_ids.insert(id.clone());
    let content = horizon_mod_package::Content::read(path, &package.manifest);
    match content
        .as_ref()
        .map(horizon_client::mod_content::live_switch)
    {
        // Jar layers can declare worldgen that Content.files does not list.
        Ok(Ok(())) if package.manifest.jar.is_empty() => {
            live_ids.insert(id.clone());
        }
        Ok(Err(why)) => log::debug!("Horizon: mod {id} switches on a restart: {why}"),
        _ => {}
    }
}

/// Installed native mods not compiled into this executable get an explanatory row.
fn not_built(
    dir: &std::path::Path,
    built: &[horizon_mod_package::runtime::BuiltMod],
    rows: &[Installed],
) -> Vec<Installed> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "hmod"));
    paths.sort();
    let mut out: Vec<Installed> = Vec::new();
    for path in paths {
        let Ok(bundle) = horizon_mod_package::index::native::Bundle::read(&path) else {
            continue;
        };
        let m = bundle.manifest;
        let listed = rows.iter().chain(&out).any(|row| row.id == m.id);
        if !listed && !built.iter().any(|b| b.id == m.id) {
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

pub(super) struct PreparedLoopContext {
    game: Arc<horizon_world::native_load_context::LoopContext>,
    audio: Option<horizon_world::native_load_context::LoopContext>,
}

impl Game<'_> {
    /// Build this game's loop context over the loaded registries and the mods'
    /// current settings. Called where a resource generation starts or reloads: a
    /// client at window start and after a resource reload, a server when it starts
    /// its mods and on `/reload`. A client hands a second one to its audio.
    pub(super) fn refresh_loop_context(&mut self) -> horizon_client::Result<()> {
        let context = self.prepare_loop_context(&self.options.resources)?;
        self.install_loop_context(context);
        Ok(())
    }

    pub(super) fn prepare_loop_context(
        &self,
        resources: &std::path::Path,
    ) -> horizon_client::Result<PreparedLoopContext> {
        let side = self.options.execution_role.loop_side();
        let inventory = horizon_mod_package::enabled::EnabledMods::read(resources)?;
        let ids: Vec<_> = if self.is_logical_server() {
            inventory.server_ids().collect()
        } else {
            inventory.client_ids().collect()
        };
        let item_data = self.res.core.item_data.context();
        let audio = self
            .options
            .execution_role
            .is_client()
            .then(|| item_data.loop_context(side, ids.iter().copied()));
        Ok(PreparedLoopContext {
            game: Arc::new(item_data.loop_context(side, ids.iter().copied())),
            audio,
        })
    }

    pub(super) fn install_loop_context(&mut self, context: PreparedLoopContext) {
        self.loop_context = context.game;
        if let Some(host) = self.lan.as_mut() {
            host.hub.loop_context = self.loop_context.clone();
        }
        if let Some(audio) = context.audio {
            self.audio.set_loop_context(audio);
        }
    }

    /// Stage a fresh root only when live code admission changes the inventory.
    /// The caller publishes it after every server reload listener prepared.
    pub(super) fn stage_enabled_inventory(
        &self,
    ) -> horizon_client::Result<Option<horizon_assets::resource_source::ResourceRoot>> {
        let previous = horizon_mod_package::enabled::EnabledMods::read(&self.options.resources)?;
        let next = self
            .mods
            .enabled_inventory(|id| previous.client_ids().any(|loaded| loaded == id));
        if previous == next {
            return Ok(None);
        }
        let base = self
            .mods
            .content_base
            .get()
            .cloned()
            .unwrap_or_else(|| self.vanilla_root());
        let jars = horizon_client::logging::game_home().join("jars");
        Ok(Some(content_overlay(&base, &self.mods, &jars)?.root))
    }

    /// Lists the installed mods on the Mods page. Content mods load with the resources
    /// (`content_root`); native ones are built into the game (`native_hooks.rs`). Unsupported
    /// files get the same unreadable-file row as any other invalid package.
    pub(super) fn start_mods(&mut self) {
        let dir = Self::game_dir().join(mod_list::DIR);
        let mut paths = horizon_mod_package::find(&dir);
        paths.retain(|p| !self.mods.dev.contains(p));
        paths.extend(self.mods.dev.iter().cloned());
        let (mut content_ids, mut live_ids) = Default::default();
        self.settings.mods.installed = paths
            .iter()
            .map(|path| {
                let file = path.file_name().unwrap_or_default().to_string_lossy();
                let package = match horizon_mod_package::Package::read(path) {
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
                let refused = horizon_mod_wasm::refused()
                    .iter()
                    .find(|(id, _)| *id == m.id);
                live_switch_of(path, &package, &mut content_ids, &mut live_ids);
                Installed {
                    id: m.id.clone(),
                    title: format!("{} {}", m.name, m.version),
                    description: m.description.clone(),
                    problem: refused.map(|(_, why)| format!("Not loaded: {why}")),
                    dev: self.mods.dev.contains(path),
                    native: None,
                }
            })
            .collect();
        (self.mods.content_ids, self.mods.live_ids) = (content_ids, live_ids);
        self.ui.options.mods_live = self.mods_live_now();
        if self.mods.applied.is_none() {
            self.mods.applied = Some(self.settings.mods.enabled.clone());
        }
        // A mod with a native version built in says so instead, and isn't news.
        self.list_native_mods();
        self.sync_native_switches();
        let built = self.mods.native.built();
        let built_mods = native_mods_detail(built);
        if let Some(list) = built_mods {
            mod_api::crash::set_detail("Native Mods", list);
        }

        let native_dir = Self::game_dir().join(mod_list::NATIVE_DIR);
        let unbuilt = not_built(&native_dir, built, &self.settings.mods.installed);
        self.settings.mods.installed.extend(unbuilt);
        self.mods.report_refusals(&mut self.settings.mods.installed);
    }
    /// The content mods the Mods page lets the player switch with a world
    /// open: in single player and on an open LAN world, those with only
    /// assets and data; none on another server, which owns its data.
    pub(super) fn mods_live_now(&self) -> std::collections::BTreeSet<String> {
        match self.integrated.is_some() || self.remote.is_none() {
            true => self.mods.live_ids.clone(),
            false => Default::default(),
        }
    }
    /// The content mods the resources load with: the ones applied, which a
    /// switch that needs a restart doesn't change.
    pub(super) fn applied_mods(&self) -> std::collections::BTreeSet<String> {
        self.mods
            .applied
            .clone()
            .unwrap_or_else(|| self.settings.mods.enabled.clone())
    }
    /// The player turned content mods on or off (Options > Mods, or an
    /// install from Browse Mods): those with only assets and data apply now
    /// (docs/design/mod-hotload.md), through the resource reload and then the
    /// server's data reload (`mods_reloaded`); those that add blocks, items or
    /// worldgen wait for a restart, and the Mods page says so.
    pub(super) fn switch_mods(&mut self) {
        let Some(applied) = self.mods.applied.clone() else {
            return;
        };
        let wanted = self.settings.mods.enabled.clone();
        if applied == wanted {
            return;
        }
        // Native mods' `mod.<id>` lines follow the player's choice as they are
        // (`sync_native_switches`); a content mod with blocks, items or
        // worldgen keeps the state it loaded with until a restart.
        let content = &self.mods.content_ids;
        let mut next = wanted.clone();
        let mut waiting = Vec::new();
        for id in applied.symmetric_difference(&wanted) {
            if !content.contains(id) || self.mods.live_ids.contains(id) {
                continue;
            }
            log::info!("Horizon: mod {id} needs a restart to switch");
            waiting.push(id.clone());
            if applied.contains(id) {
                next.insert(id.clone());
            } else {
                next.remove(id);
            }
        }
        self.ui.options.mods_note = (!waiting.is_empty())
            .then(|| lang::tr("horizon.mods.switch.restart", &[&waiting.join(", ")]));
        let changed = applied
            .symmetric_difference(&next)
            .any(|id| content.contains(id));
        if !changed {
            self.mods.applied = Some(next);
            return;
        }
        log::info!(
            "Horizon: content mods switched live: {:?} -> {:?}",
            applied,
            next
        );
        self.mods.applied = Some(next);
        self.mods.live = Some(std::time::Instant::now());
        self.reload_packs();
    }
    /// A live switch's resources applied: the server reloads its data packs
    /// from the new root, as `/reload` does (recipes, tags, loot tables,
    /// advancements, functions), and sends them to its players.
    pub(super) fn mods_reloaded(&mut self, applied: bool) {
        let Some(started) = self.mods.live.take() else {
            return;
        };
        let resources_ms = started.elapsed().as_millis();
        if !applied {
            return;
        }
        let root = self.options.resources.clone();
        let data_source = self.pack.data_source.clone();
        let data_changes = self.pack.data_changes.clone();
        let additional_data = self.pack.additional_data.clone();
        let enabled = self.applied_mods();
        let selected = self.mods.selected.clone();
        let refused = self.mods.refused.clone();
        let reload = move |g: &mut Game<'_>| {
            let at = std::time::Instant::now();
            g.options.resources = root;
            g.pack.data_source = data_source;
            g.pack.data_changes = data_changes;
            g.pack.additional_data = additional_data;
            g.settings.mods.enabled = enabled.clone();
            g.mods.applied = Some(enabled);
            g.mods.selected = selected;
            g.mods.refused = refused;
            g.mods.data_switched = true;
            if g.world_root.as_os_str().is_empty() {
                return;
            }
            if let Err(error) = g.reload_datapacks(true, &g.level.enabled_features.clone()) {
                log::warn!("Horizon: content mods data reload failed: {error}");
                return;
            }
            log::info!(
                "Horizon: content mods switched live: server data reloaded in {} ms",
                at.elapsed().as_millis()
            );
        };
        log::info!("Horizon: content mods switched live: resources reloaded in {resources_ms} ms");
        if let Some(server) = &self.integrated {
            server.post(reload);
        } else if self.remote.is_none() {
            reload(self);
        }
    }
    /// Apply the player's switches at the canonical native and wasm dispatch boundary.
    pub(super) fn sync_native_switches(&self) {
        self.mods.apply_code_switches(&self.settings.mods.disabled);
    }
}

/// Mob types loaded from a scratch root whose `entity-types.json` has a
/// zombie, a villager, and the rows a content root writes for a mod's types
/// (`mod_content`): `ruby:ghoul` (a zombie with 40 health), `ruby:settler` (a
/// villager as it is) and `ruby:bolt` (a thrown snowball, not a mob).
#[cfg(test)]
pub(super) fn living_mod_types()
-> std::collections::HashMap<String, std::sync::Arc<horizon_client::mobs::MobType>> {
    let zombie =
        r#""width":0.6,"height":1.95,"eye_height":1.74,"category":"monster","fire_immune":false"#;
    let villager =
        r#""width":0.6,"height":1.95,"eye_height":1.62,"category":"misc","fire_immune":false"#;
    let trader =
        r#""width":0.6,"height":1.95,"eye_height":1.62,"category":"creature","fire_immune":false"#;
    let table = format!(
        r#"{{"minecraft:zombie":{{{zombie},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.23000000417232513}}}},
        "minecraft:villager":{{{villager},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.5}}}},
        "minecraft:wandering_trader":{{{trader},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.7}}}},
        "minecraft:snowball":{{"width":0.25,"height":0.25,"eye_height":0.0,"category":"misc","fire_immune":false}},
        "ruby:ghoul":{{{zombie},"attributes":{{"minecraft:max_health":40.0,"minecraft:movement_speed":0.23000000417232513}},"template":"minecraft:zombie"}},
        "ruby:settler":{{{villager},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.5}},"template":"minecraft:villager"}},
        "ruby:merchant":{{{trader},"attributes":{{"minecraft:max_health":20.0,"minecraft:movement_speed":0.7}},"template":"minecraft:wandering_trader"}},
        "ruby:bolt":{{"width":0.25,"height":0.25,"eye_height":0.0,"category":"misc","fire_immune":false,"template":"minecraft:snowball"}}}}"#
    );
    // A folder per call: tests run in parallel in one process, and each removes its own.
    static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "horizon-living-mod-types-{}-{call}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("entity-types.json"), table).unwrap();
    let types = horizon_client::mobs::MobTypes::load_for_server(&root).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    types
        .types
        .into_iter()
        .map(|(id, t)| (id, std::sync::Arc::new(t)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_mod_package::runtime::{BuiltMod, Runtime};

    #[test]
    fn only_clients_collect_registered_keys() {
        let declared = [(
            "sample",
            mod_api::NativeKeyRegistration::new("zoom", "key.keyboard.z"),
        )];
        assert_eq!(
            collect_key_mappings(ExecutionRole::Client, &declared).len(),
            1
        );
        assert!(collect_key_mappings(ExecutionRole::IntegratedServer, &declared).is_empty());
        assert!(collect_key_mappings(ExecutionRole::DedicatedServer, &declared).is_empty());
    }

    mod inventory {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/game/modding/mods/mod_inventory_tests.rs"
        ));
    }

    const BUILT: &[BuiltMod] = &[BuiltMod {
        id: "machine_sample",
        version: "0.3.0",
        side: horizon_mod_package::Side::Both,
    }];

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("horizon-game-mods-{}-{label}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn runtime(&self) -> Runtime {
            let dir = self.0.join("compiled");
            let package = dir.join(mod_list::NATIVE_DIR).join("machine_sample");
            let file = std::fs::File::open(self.0.join("machine_sample.hmod")).unwrap();
            zip::ZipArchive::new(file)
                .unwrap()
                .extract(&package)
                .unwrap();
            Runtime::load(&dir.join("horizon-engine"), BUILT).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    /// Runs `test` again in a child process whose working directory is its own
    /// empty folder: the overlay's `.horizon-cache` lives under the working
    /// directory, so children sharing one (the temp dir) race on that cache.
    fn run_in_child(test: &str, mode: &str) {
        let cwd = std::env::temp_dir().join(format!(
            "horizon-game-mods-cwd-{}-{mode}",
            std::process::id()
        ));
        std::fs::create_dir_all(&cwd).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .current_dir(&cwd)
            .args(["--exact", test, "--nocapture"])
            .env(mode, "1")
            .status()
            .unwrap();
        std::fs::remove_dir_all(&cwd).unwrap();
        assert!(status.success());
    }

    /// A content mod's living type is a mob of its template's species under
    /// its own id, with its own attributes: it saves its id with every field
    /// its template's class saves, loads back as itself, and rolls its own
    /// loot table. A thrown type stays a projectile.
    #[test]
    fn a_mod_living_type_is_its_templates_mob_with_its_own_attributes() {
        use horizon_client::mobs::{Mob, Species, drops::death_loot_table};
        use horizon_core::geometry::Vec3;
        let types = living_mod_types();
        let (zombie, ghoul) = (&types["minecraft:zombie"], &types["ruby:ghoul"]);
        assert_eq!(ghoul.species, Species::Zombie);
        assert_eq!(ghoul.class_id(), "minecraft:zombie");
        assert_eq!(types["ruby:settler"].species, Species::Villager);
        assert!(!types.contains_key("ruby:bolt"), "a thrown type is no mob");
        assert_eq!((zombie.max_health, ghoul.max_health), (20.0, 40.0));
        assert_eq!(ghoul.speed.to_bits(), zombie.speed.to_bits());
        assert_eq!(
            (ghoul.sounds, ghoul.texture.as_str()),
            (zombie.sounds, zombie.texture.as_str())
        );

        let at = Vec3::new(0.5, 64.0, 0.5);
        let (zombie, ghoul) = (
            Mob::new(1, zombie.clone(), at, 0.0),
            Mob::new(2, ghoul.clone(), at, 0.0),
        );
        assert_eq!(ghoul.health, 40.0);
        let saved =
            |m: &Mob| horizon_client::entity_nbt::mob_to_nbt(m, [0, 0, 0, 2], None, None, |_| None);
        let keys = |tag: &horizon_core::nbt::Tag| -> Vec<String> {
            let horizon_core::nbt::Tag::Compound(map) = tag else {
                panic!("a mob saves a compound");
            };
            map.keys().map(|k| k.to_string_lossy()).collect()
        };
        let (zombie_tag, ghoul_tag) = (saved(&zombie), saved(&ghoul));
        let Some(horizon_core::nbt::Tag::String(id)) = ghoul_tag.get("id") else {
            panic!("a saved mob has its id");
        };
        assert_eq!(id.to_string_lossy(), "ruby:ghoul");
        assert_eq!(
            keys(&ghoul_tag),
            keys(&zombie_tag),
            "it saves what a zombie saves"
        );
        for key in ["IsBaby", "CanBreakDoors", "DrownedConversionTime"] {
            assert!(keys(&ghoul_tag).iter().any(|k| k == key), "{key}");
        }
        let loaded = horizon_client::entity_nbt::mob_from_nbt(&ghoul_tag, &types, 3, None)
            .expect("it loads back");
        assert_eq!(
            (loaded.kind.id.as_str(), loaded.health),
            ("ruby:ghoul", 40.0)
        );
        assert_eq!(
            death_loot_table(&ghoul, true).as_deref(),
            Some("ruby:entities/ghoul")
        );
        assert_eq!(
            death_loot_table(&zombie, true).as_deref(),
            Some("minecraft:entities/zombie")
        );
    }

    /// A mod's declared attribute set (#845, `EntityAttributeCreationEvent`)
    /// reaches everything a mob of the type is: the fields it spawns with,
    /// the finalize defaults and the live attribute map's catalog row —
    /// built from the template's, so the reads of #1469 and #1493 answer the
    /// declared values with no second copy.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mods_declared_entity_attributes_reach_spawn_and_the_live_map() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        use horizon_world::attributes::entity::Catalog;

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &horizon_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        let declared = [
            ("minecraft:max_health".to_string(), 55.0),
            ("minecraft:movement_speed".to_string(), 0.4),
        ];
        let merged = catalog
            .with_entity_row("ruby:ghoul", "minecraft:zombie", &declared)
            .unwrap();
        let base = |catalog: &Catalog, entity: &str, name: &str| {
            catalog.default_base(entity, catalog.id(name).unwrap())
        };
        // The declared values, and the template's row under them.
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:max_health"),
            Some(55.0)
        );
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:movement_speed"),
            Some(0.4)
        );
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:attack_damage"),
            base(&catalog, "minecraft:zombie", "minecraft:attack_damage")
        );
        // The live map builds for the type and answers the declared values.
        let live = horizon_client::living_attributes::LivingAttributes::default();
        assert_eq!(
            live.read(
                &std::sync::Arc::new(merged.clone()),
                "ruby:ghoul",
                "minecraft:max_health",
                horizon_world::attribute_map::ValueKind::Computed,
                Vec::new
            ),
            Some(55.0)
        );
        // The type's fields spawn with them: the merged row takes the type's
        // attribute map and fields (`take_attribute_row`).
        let mut types = living_mod_types();
        let ghoul = types.get_mut("ruby:ghoul").unwrap();
        Arc::make_mut(ghoul)
            .take_attribute_row(Arc::new(merged.entity_row("ruby:ghoul").unwrap()), &merged);
        let ghoul = Mob::new(7, types["ruby:ghoul"].clone(), Vec3::ZERO, 0.0);
        assert_eq!(ghoul.health, 55.0, "the spawn's health");
        assert_eq!(ghoul.attrs.max_health, 55.0);
        assert_eq!(ghoul.attrs.speed, 0.4);
        assert_eq!(
            ghoul.kind.attributes.get("minecraft:max_health"),
            Some(&55.0),
            "the finalize defaults read the declared value"
        );
    }

    /// A declared set's merged row — the template's with the declared set on
    /// top — is the type's one record: a content row's own value (the
    /// ghoul's 40 health) gives way to the template's, so a mob of the type
    /// spawns with what the live attribute map answers (#845, review).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_declared_type_spawns_with_its_merged_row_not_its_content_row() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        use horizon_world::attributes::entity::Catalog;

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &horizon_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        // Only the speed is declared: the merged row keeps the template's
        // health (a zombie's 20), not the content row's 40.
        let declared = [("minecraft:movement_speed".to_string(), 0.4)];
        let merged = catalog
            .with_entity_row("ruby:ghoul", "minecraft:zombie", &declared)
            .unwrap();
        let mut types = living_mod_types();
        let ghoul = types.get_mut("ruby:ghoul").unwrap();
        assert_eq!(ghoul.max_health, 40.0, "the content row's own value");
        Arc::make_mut(ghoul)
            .take_attribute_row(Arc::new(merged.entity_row("ruby:ghoul").unwrap()), &merged);
        let ghoul = Mob::new(7, types["ruby:ghoul"].clone(), Vec3::ZERO, 0.0);
        assert_eq!(
            ghoul.health, 20.0,
            "the template's health through the merged row"
        );
        assert_eq!(ghoul.attrs.max_health, 20.0);
        assert_eq!(ghoul.attrs.speed, 0.4, "the declared speed");
        let live = horizon_client::living_attributes::LivingAttributes::default();
        assert_eq!(
            live.read(
                &std::sync::Arc::new(merged),
                "ruby:ghoul",
                "minecraft:max_health",
                horizon_world::attribute_map::ValueKind::Computed,
                Vec::new
            ),
            Some(20.0),
            "the live map answers the same row"
        );
    }

    /// A content mod's type from the wandering trader, the neutral template
    /// whose behaviour is all goals: a trader under its own id, with a
    /// trader's goals, that saves what a trader saves and loads back.
    #[test]
    fn a_mod_trader_type_is_a_wandering_trader_under_its_own_id() {
        use horizon_client::mobs::{Mob, Species};
        use horizon_core::nbt::Tag;
        let types = living_mod_types();
        let merchant = &types["ruby:merchant"];
        assert_eq!(merchant.species, Species::WanderingTrader);
        assert_eq!(merchant.class_id(), "minecraft:wandering_trader");
        let at = horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5);
        let trader = Mob::new(1, types["minecraft:wandering_trader"].clone(), at, 0.0);
        let merchant = Mob::new(2, merchant.clone(), at, 0.0);
        assert!(merchant.ai.goals.has("Float") && trader.ai.goals.has("Float"));
        let saved =
            |m: &Mob| horizon_client::entity_nbt::mob_to_nbt(m, [0, 0, 0, 2], None, None, |_| None);
        let keys = |tag: &Tag| -> Vec<String> {
            let Tag::Compound(map) = tag else {
                panic!("a mob saves a compound");
            };
            map.keys().map(|k| k.to_string_lossy()).collect()
        };
        let tag = saved(&merchant);
        assert_eq!(
            keys(&tag),
            keys(&saved(&trader)),
            "it saves what a trader saves"
        );
        let loaded =
            horizon_client::entity_nbt::mob_from_nbt(&tag, &types, 3, None).expect("it loads back");
        assert_eq!(
            (loaded.kind.id.as_str(), loaded.species()),
            ("ruby:merchant", Species::WanderingTrader)
        );
    }

    #[test]
    fn standalone_models_collect_only_on_the_client() {
        use mod_api::{self, Hooks, ModEntry, StandaloneModelsEvent, testing::with_mods};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        #[derive(Default)]
        struct Models(Arc<AtomicUsize>);
        #[mod_api::horizon_mod]
        impl Hooks for Models {
            fn standalone_models(
                &self,
                context: &RegistrationContext,
                event: &mut StandaloneModelsEvent<'_>,
            ) {
                assert_eq!(
                    context.mods().ids().collect::<Vec<_>>(),
                    ["registration_test"]
                );
                self.0.fetch_add(1, Ordering::Relaxed);
                event.models.push("registration_test:block/shaft".into());
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        with_mods(
            vec![ModEntry::new(
                "registration_test",
                Box::new(Models(calls.clone())),
            )]
            .into(),
            || {
                mod_api::__private::with_test_packages(&["registration_test"], || {
                    assert_eq!(
                        collect_standalone_models(ExecutionRole::IntegratedServer),
                        None
                    );
                    assert_eq!(
                        collect_standalone_models(ExecutionRole::DedicatedServer),
                        None
                    );
                    assert_eq!(calls.load(Ordering::Relaxed), 0);
                    let models = collect_standalone_models(ExecutionRole::Client).unwrap();
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                    assert_eq!(models, ["registration_test:block/shaft"]);
                })
            },
        );
    }

    fn hmod(dir: &std::path::Path, id: &str) {
        let manifest = format!(
            "id = '{id}'\nname = '{id}'\nversion = '0.3.0'\napi = '0.2'\nside = 'both'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let files: &[(&str, &[u8])] = &[
            (
                "Cargo.toml",
                b"[package]\nname='different_crate'\nversion='9.9.9'\n",
            ),
            ("src/lib.rs", b"pub fn hooks() {}"),
        ];
        let bytes = horizon_mod_package::Package::write_files(&manifest, files).unwrap();
        std::fs::write(dir.join(format!("{id}.hmod")), bytes).unwrap();
    }

    #[test]
    fn content_overlay_validates_dependencies_and_orders_layers() {
        const MODE: &str = "HORIZON_CONTENT_DEPENDENCY_TEST";
        if std::env::var_os(MODE).is_none() {
            run_in_child(
                "game::mods::tests::content_overlay_validates_dependencies_and_orders_layers",
                MODE,
            );
            return;
        }
        let f = Fixture::new("content-dependencies");
        let write = |id: &str, version: &str, dependencies: &str| {
            let dir = f.0.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("horizon-mod.toml"), format!(
                "id='{id}'\nname='{id}'\nversion='{version}'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{dependencies}"
            )).unwrap();
            dir
        };
        let addon = write("alpha", "1", "[dependencies]\nzbase='^1'");
        let base_mod = write("zbase", "1", "");
        let base = f.0.join("resources");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("horizon-resources.json"), "{}").unwrap();
        let mut sources = ModsState {
            dev: vec![addon.clone(), base_mod.clone()],
            ..Default::default()
        };
        let enabled = Default::default();
        sources.select(&f.0, &enabled);
        let overlay = content_overlay(&base, &sources, &f.0.join("jars")).unwrap();
        assert_eq!(
            overlay
                .mods
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["zbase", "alpha"]
        );
        sources.dev = vec![addon.clone()];
        sources.select(&f.0, &enabled);
        let error = content_overlay(&base, &sources, &f.0.join("jars"))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("alpha") && error.contains("zbase"));
        write(
            "alpha",
            "1",
            "[dependencies]\nzbase={version='^1', optional=true}",
        );
        sources.select(&f.0, &enabled);
        assert!(content_overlay(&base, &sources, &f.0.join("jars")).is_ok());
        write("zbase", "2", "");
        sources.dev.push(base_mod.clone());
        sources.select(&f.0, &enabled);
        let error = content_overlay(&base, &sources, &f.0.join("jars"))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("installed version is 2"));
        write("zbase", "1", "[dependencies]\nalpha='^1'");
        sources.select(&f.0, &enabled);
        assert!(
            content_overlay(&base, &sources, &f.0.join("jars"))
                .err()
                .unwrap()
                .to_string()
                .contains("cycle")
        );
    }

    #[test]
    fn native_content_follows_the_build_and_misplaced_content_follows_its_selection() {
        const MODE: &str = "HORIZON_MODS_SELECTION_TEST";
        let mode = std::env::var(MODE).ok();
        if mode.is_none() {
            // Inventory and content registries are process-wide, as in a real startup.
            for mode in ["compiled", "vanilla"] {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .current_dir(std::env::temp_dir())
                    .args(["--exact", "game::mods::tests::native_content_follows_the_build_and_misplaced_content_follows_its_selection", "--nocapture"])
                    .env(MODE, mode).status().unwrap();
                assert!(status.success());
            }
            return;
        }
        let f = Fixture::new(mode.as_deref().unwrap());
        hmod(&f.0, "machine_sample");
        let native = if mode.as_deref() == Some("compiled") {
            f.runtime()
        } else {
            Runtime::default()
        };
        let here = f.0.join("unrelated-game");
        let misplaced = here.join(mod_list::NATIVE_DIR);
        std::fs::create_dir_all(&misplaced).unwrap();
        hmod(&misplaced, "machine_sample");
        let manifest = "id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n";
        let plain = horizon_mod_package::Package::write_files(manifest, &[]).unwrap();
        std::fs::write(misplaced.join("plain.hmod"), plain).unwrap();
        let base = f.0.join("resources");
        std::fs::create_dir_all(&base).unwrap();
        // A prepared root's registry report: block entity registration reads it (B1).
        std::fs::write(
            base.join("registries.json"),
            "{\"minecraft:block_entity_type\":{\"entries\":{}}}",
        )
        .unwrap();
        std::fs::write(base.join("horizon-resources.json"), "{}").unwrap();
        std::fs::write(base.join("block-behaviours.json"), r#"{"fluids":[]}"#).unwrap();
        std::env::set_current_dir(&here).unwrap();
        let mut sources = ModsState {
            native: std::sync::Arc::new(native),
            ..Default::default()
        };
        let enabled = std::collections::BTreeSet::from(["plain".to_owned()]);
        sources.select(&here, &enabled);
        let generated = content_root(&base, &sources).unwrap();
        assert_ne!(generated.prepared(), base);
        assert_eq!(
            horizon_mod_package::enabled::EnabledMods::read(generated.prepared())
                .unwrap()
                .server_ids()
                .collect::<Vec<_>>(),
            ["plain"],
        );
        let local = horizon_protocol::mod_list::local();
        let ids: Vec<_> = local.iter().map(|m| m.id.as_str()).collect();
        if mode.as_deref() == Some("compiled") {
            assert_eq!(ids, ["machine_sample", "plain"]);
            assert_eq!(local[0].version, "0.3.0");
        } else {
            assert_eq!(ids, ["plain"]);
        }
        // Out of the fixture: Windows won't remove a process's working directory.
        std::env::set_current_dir(std::env::temp_dir()).unwrap();
    }

    #[test]
    fn a_shared_missing_jar_stops_native_load_and_refuses_each_content_mod_row() {
        const MODE: &str = "HORIZON_JAR_REFUSAL_TEST";
        if std::env::var_os(MODE).is_none() {
            // Native declarations, like the real startup, belong to one process.
            run_in_child(
                "game::mods::tests::a_shared_missing_jar_stops_native_load_and_refuses_each_content_mod_row",
                MODE,
            );
            return;
        }
        let f = Fixture::new("shared-jar-refusal");
        let jar = horizon_mod_package::manifest::JarRef::new(
            "original".into(),
            "1".into(),
            "9".repeat(128),
        )
        .unwrap();
        for id in ["machine_sample", "alpha", "beta", "gamma"] {
            let native = id == "machine_sample";
            let mut text = format!(
                "id='{id}'\nname='{id}'\nversion='0.3.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n"
            );
            if native {
                text.push_str(&format!(
                    "[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
                    "a".repeat(40),
                    "b".repeat(40)
                ));
            }
            if id != "gamma" {
                text.push_str(&format!(
                    "[[jar]]\nid='original'\nversion='1'\nsha512='{}'\n",
                    jar.sha512()
                ));
            }
            let code: &[(&str, &[u8])] = if native {
                &[
                    ("Cargo.toml", b"[package]\nname='example'\nversion='1'\n"),
                    ("src/lib.rs", b"pub fn hooks() {}"),
                ]
            } else {
                &[]
            };
            std::fs::write(
                f.0.join(format!("{id}.hmod")),
                horizon_mod_package::Package::write_files(&text, code).unwrap(),
            )
            .unwrap();
        }
        let selected: Arc<[horizon_mod_package::selection::SelectedPackage]> =
            ["alpha", "beta", "gamma"]
                .into_iter()
                .map(|id| {
                    let path = f.0.join(format!("{id}.hmod"));
                    horizon_mod_package::selection::SelectedPackage {
                        package: horizon_mod_package::Package::read(&path).unwrap(),
                        path,
                    }
                })
                .collect();
        let native = ModsState {
            native: Arc::new(f.runtime()),
            selected: selected.clone(),
            ..Default::default()
        };
        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("horizon-resources.json"), "{}").unwrap();
        let error = content_overlay(&base, &native, &store).err().unwrap();
        let missing = error.downcast_ref::<MissingJar>().unwrap();
        assert_eq!(missing.owners, ["machine_sample", "alpha", "beta"]);
        assert_eq!(missing.jar, jar);
        assert_eq!(missing.path, jar.cache_path(&store));
        assert!(!store.exists());

        let content = ModsState {
            selected,
            ..Default::default()
        };
        let overlay = content_overlay(&base, &content, &store).unwrap();
        assert_ne!(overlay.root.prepared(), base);
        assert_eq!(
            horizon_mod_package::enabled::EnabledMods::read(overlay.root.prepared())
                .unwrap()
                .server_ids()
                .collect::<Vec<_>>(),
            ["gamma"],
        );
        assert_eq!(
            overlay
                .mods
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["gamma"]
        );
        let mut rows: Vec<_> = ["alpha", "beta", "gamma"]
            .into_iter()
            .map(|id| Installed {
                id: id.into(),
                title: id.into(),
                description: String::new(),
                problem: None,
                dev: false,
                native: None,
            })
            .collect();
        content.report_refusals(&mut rows);
        for row in &rows[..2] {
            let problem = row.problem.as_deref().unwrap();
            assert!(problem.contains("alpha, beta: original 1:"));
            assert!(problem.contains(&jar.cache_path(&store).display().to_string()));
        }
        assert!(rows[2].problem.is_none());
    }

    #[test]
    fn the_crash_report_lists_the_embedded_native_mods() {
        let built = [
            BUILT[0],
            BuiltMod {
                id: "pipes",
                version: "2.0",
                side: horizon_mod_package::Side::Client,
            },
        ];
        assert_eq!(
            native_mods_detail(&built).as_deref(),
            Some("machine_sample 0.3.0, pipes 2.0")
        );
        assert_eq!(native_mods_detail(&[]), None);
    }

    #[test]
    fn native_mods_installed_but_not_built_get_a_row_saying_so() {
        let f = Fixture::new("not-built");
        for id in ["veinminer", "zoom", "minimap", "machine_sample"] {
            hmod(&f.0, id);
        }
        std::fs::write(f.0.join("notes.txt"), "not a mod").unwrap();
        std::fs::write(f.0.join("broken.hmod"), "not a zip").unwrap();
        let rows = not_built(&f.0, &[], &[]);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["machine_sample", "minimap", "veinminer", "zoom"]);
        assert_eq!(rows[2].title, "veinminer 0.3.0");
        assert_eq!(rows[2].problem.as_deref(), Some(mod_list::NOT_BUILT));
        assert!(rows.iter().all(|r| r.native.is_none() && !r.dev));
        let listed = Installed {
            id: "minimap".into(),
            ..rows[0].clone()
        };
        let rows = not_built(&f.0, BUILT, &[listed]);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["veinminer", "zoom"]);
        assert!(not_built(&f.0.join("missing"), &[], &[]).is_empty());
    }

    /// A root with mods that declare no mob effects leaves the registry open for the first one
    /// that does.
    #[test]
    fn a_root_without_mob_effects_does_not_freeze_the_registry() {
        use horizon_client::{brewing, mod_content::ContentMod};
        // Another test may have set the process-wide list already.
        if !brewing::mod_effects().is_empty() {
            return;
        }
        let f = Fixture::new("no-effects");
        std::fs::write(
            f.0.join("horizon-mod.toml"),
            "id = 'plain'\nname = 'plain'\nversion = '1.0'\napi = '0.2'\ncapabilities = ['registries']\n",
        )
        .unwrap();
        let package = horizon_mod_package::Package::read(&f.0).unwrap();
        let plain = ContentMod {
            id: "plain".into(),
            content: horizon_mod_package::content::Content::read(&f.0, &package.manifest).unwrap(),
            jars: Vec::new(),
        };
        register_mob_effects(&[plain]);
        // The list `potions::tests` registers, so it holds whichever test runs first.
        assert!(brewing::register_mod_effects([(
            "test:effect".to_owned(),
            0,
            brewing::Category::Neutral
        )]));
    }
}
