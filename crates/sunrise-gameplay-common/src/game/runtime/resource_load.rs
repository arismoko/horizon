//! The startup's resource load: the stages the loading screen shows, the
//! scoped threads each step runs on, and the core, gameplay and pack tables
//! read from the prepared resources.
use super::{resource_tables::PackParts, *};
pub use std::path::Path;
pub use sunrise_assets::{item_model::ItemModels, resource_source::ResourceRoot};
use sunrise_world::{
    block_render::FaceVisibility,
    item_defaults::{ItemData, ItemDefaults},
    registry::BlockRegistry,
};

pub trait CorePresentation {
    type Early: Send;
    type Visual: Send;
    type Output: Send;

    fn alongside_assets<A>(
        resources: &ResourceRoot,
        parallel: bool,
        load_assets: impl FnOnce() -> A,
    ) -> (A, Self::Early);
    fn render(
        resources: &ResourceRoot,
        assets: &Assets,
        early: &mut Self::Early,
        emissive_sprites: &[mod_api::Answered<mod_api::EmissiveSpriteRegistration>],
    ) -> std::result::Result<Self::Visual, String>;
    fn visibility(
        root: &Path,
        registry: &'static BlockRegistry,
    ) -> std::result::Result<FaceVisibility, String>;
    fn biomes(resources: &ResourceRoot) -> std::result::Result<Biomes, String>;
    fn finish(
        resources: &ResourceRoot,
        early: Self::Early,
        visual: Self::Visual,
        item_data: &Arc<ItemData>,
    ) -> std::result::Result<Self::Output, String>;
    fn item_tables(
        root: &Path,
        item_data: &Arc<ItemData>,
        output: &Self::Output,
    ) -> std::result::Result<(Arc<ItemModels>, Arc<HashMap<String, i32>>), String>;
}

pub struct ServerCorePresentation;

impl CorePresentation for ServerCorePresentation {
    type Early = ();
    type Visual = ();
    type Output = ();

    fn alongside_assets<A>(_: &ResourceRoot, _: bool, load_assets: impl FnOnce() -> A) -> (A, ()) {
        (load_assets(), ())
    }
    fn render(
        _: &ResourceRoot,
        _: &Assets,
        _: &mut (),
        _: &[mod_api::Answered<mod_api::EmissiveSpriteRegistration>],
    ) -> std::result::Result<(), String> {
        Ok(())
    }
    fn visibility(
        _: &Path,
        _: &'static BlockRegistry,
    ) -> std::result::Result<FaceVisibility, String> {
        Ok(FaceVisibility::empty())
    }
    fn biomes(resources: &ResourceRoot) -> std::result::Result<Biomes, String> {
        Biomes::load_for_server(resources).map_err(|e| e.to_string())
    }
    fn finish(
        _: &ResourceRoot,
        (): (),
        (): (),
        _: &Arc<ItemData>,
    ) -> std::result::Result<(), String> {
        Ok(())
    }
    fn item_tables(
        root: &Path,
        item_data: &Arc<ItemData>,
        _: &(),
    ) -> std::result::Result<(Arc<ItemModels>, Arc<HashMap<String, i32>>), String> {
        let models =
            sunrise_sim::item_models::load_for_server(item_data).map_err(|e| e.to_string())?;
        let ids = Arc::new(sunrise_sim::item_models::load_item_ids(root));
        Ok((models, ids))
    }
}

#[cfg(test)]
#[path = "incoming_tests/resource_load_scope.rs"]
pub(in crate::game) mod scope_tests;

#[cfg(test)]
#[path = "resource_load/facts_memo.rs"]
pub(in crate::game) mod facts_memo;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub(in crate::game) static SKIP_FACTS_MEMO: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_skip_facts_memo<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            SKIP_FACTS_MEMO.set(self.0);
        }
    }
    let _restore = Restore(SKIP_FACTS_MEMO.replace(true));
    f()
}

pub(in crate::game) const STAGE_LINES: [&str; 10] = [
    "Loading menu",
    "Loading assets",
    "Loading block facts",
    "Loading biomes",
    "Loading pack tables",
    "Loading collision",
    "Loading items",
    "Loading recipes",
    "Loading world resources",
    "World resources ready",
];

/// The background loader's checkpoint, read by the loading screen each frame.
#[derive(Clone)]
pub struct Progress {
    pub(in crate::game) value: Arc<AtomicU32>,
    pub(in crate::game) lines: &'static [&'static str],
}

/// Harness and dedicated runs keep the fully-synchronous startup so their
/// output stays byte-identical; only an ordinary launch shows the window first.
pub fn synchronous_startup(options: &Options) -> bool {
    // A headless game has no window for the loading screen.
    options.headless
        || options.frames.is_some()
        || options.world_frames.is_some()
        || options.benchmark.is_some()
        || options.control.is_some()
        || !options.execution_role.is_client()
        || std::env::var_os("SUNRISE_SCREEN_BUILD_FIXTURE").is_some()
        || std::env::var_os("SUNRISE_DISPENSE_FIXTURE").is_some()
        || std::env::var_os("SUNRISE_SCULK_FIXTURE").is_some()
        || std::env::var_os("SUNRISE_HOT_PATHS").is_some()
        || std::env::var_os("SUNRISE_COMMAND_SCRIPT").is_some()
        || std::env::var_os("SUNRISE_NET_REPORT").is_some()
}

/// The local world a client's launch opens at once (`--world`,
/// `--quickPlaySingleplayer`), whose generator data can load with the game's.
pub fn launch_world(options: &Options) -> Option<&str> {
    (options.execution_role.is_client() && options.server.is_none() && !options.test_terrain)
        .then_some(options.world.as_deref())
        .flatten()
}

/// A scoped thread with a name, for one step of the resource load.
///
/// Named because the heap census and the startup trace both fall back to the
/// OS thread name: unnamed, every loader's retained memory and every span it
/// ran landed in one `unnamed thread` bucket. That bucket was 330 MiB of a
/// playing game's resident set -- the largest of all of them -- with nothing
/// to say which loader held it, and 13 of the trace's 17 lanes were
/// indistinguishable.
pub fn named_scope<'scope, T: Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    name: &str,
    f: impl FnOnce() -> T + Send + 'scope,
) -> std::thread::ScopedJoinHandle<'scope, T> {
    #[cfg(any(test, feature = "test-support"))]
    let inherited = mod_api::testing::inherit_mods(move |_: &()| f());
    #[cfg(any(test, feature = "test-support"))]
    let f = move || inherited(&());
    std::thread::Builder::new()
        .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
        // Linux keeps 15 bytes of a thread name: a short prefix keeps them apart
        // ("sunrise-load-core" and "sunrise-load-collision" would both read
        // "sunrise-load-co").
        .name(format!("load-{name}"))
        .spawn_scoped(scope, f)
        // `Scope::spawn`, which this replaces, panics when the OS refuses a
        // thread. Keeping that: a resource load cannot go on without it.
        .expect("resource loader thread")
}

/// A step of the resource load: on its own scoped thread, or, when the
/// settings allow a single worker, run where it is joined.
pub enum Step<'scope, T> {
    Thread(std::thread::ScopedJoinHandle<'scope, T>),
    Later(Box<dyn FnOnce() -> T + Send + 'scope>),
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        self.value.load(Ordering::Relaxed) as f32 / (self.lines.len() - 1) as f32
    }
    pub fn line(&self) -> &'static str {
        self.lines[(self.value.load(Ordering::Relaxed) as usize).min(self.lines.len() - 1)]
    }
    pub fn new() -> Self {
        Self::with(&STAGE_LINES)
    }
    pub fn set(&self, stage: u32) {
        self.value.store(stage, Ordering::Relaxed);
    }
    pub fn with(lines: &'static [&'static str]) -> Self {
        Self {
            value: Arc::new(AtomicU32::new(0)),
            lines,
        }
    }
}

impl<'scope, T: Send + 'scope> Step<'scope, T> {
    pub fn join(self) -> T {
        match self {
            Self::Thread(handle) => handle.join().unwrap(),
            Self::Later(f) => f(),
        }
    }
    /// `name` names the thread when one is spawned ([`named_scope`]).
    pub fn spawn(
        scope: &'scope std::thread::Scope<'scope, '_>,
        parallel: bool,
        name: &str,
        f: impl FnOnce() -> T + Send + 'scope,
    ) -> Self {
        if parallel {
            Self::Thread(named_scope(scope, name, f))
        } else {
            Self::Later(Box::new(f))
        }
    }
}

pub struct WorldRestLoader {
    pub progress: Progress,
    pub handle: JoinHandle<std::result::Result<(), String>>,
}

impl WorldRestLoader {
    pub fn ready(&self) -> Result<bool> {
        // No time limit: the title stays usable while this loads, and a slow
        // disk or a debug build can take minutes. A failure ends the thread
        // and is reported by `finish`.
        Ok(self.handle.is_finished())
    }

    pub fn finish(self) -> Result<()> {
        self.handle
            .join()
            .map_err(|_| "world resource loader panicked")?
            .map_err(Into::into)
    }
}

/// The world bundle's core tables from one resource root, before
/// `load_resources` binds them into the pack tables and `WorldCore`.
pub struct CoreTables {
    pub(in crate::game) item_data: Arc<sunrise_world::item_defaults::ItemData>,
    pub(in crate::game) item_defaults: Arc<sunrise_world::item_defaults::ItemDefaults>,
    pub(in crate::game) assets: Assets,
    pub(in crate::game) item_models: Arc<sunrise_assets::item_model::ItemModels>,
    pub(in crate::game) item_ids: Arc<HashMap<String, i32>>,
    pub(in crate::game) visibility: sunrise_world::block_render::FaceVisibility,
    pub(in crate::game) gameplay: GameplayTables,
    pub(in crate::game) biomes: Arc<Biomes>,
    pub(in crate::game) environment: Environment,
    pub(in crate::game) collision: Arc<BlockCollisionRegistry<'static>>,
}
// Distinguish a failed pack (which startup can roll back) from a failed
// load or mod prepare (neither may publish a partly prepared generation).
pub enum CoreLoadError {
    Pack(String),
    Fatal(String),
}
impl From<mod_api::__private::reload::ReloadStageError> for CoreLoadError {
    fn from(error: mod_api::__private::reload::ReloadStageError) -> Self {
        Self::Fatal(error.diagnostic())
    }
}

/// The block facts and the tables read from them.
pub(in crate::game) struct GameplayTables {
    pub(in crate::game) behaviour_json: serde_json::Value,
    pub(in crate::game) facts: BlockFacts,
    pub(in crate::game) gameplay_projection: Vec<sunrise_items::blocks::GameplayProjection>,
    pub(in crate::game) light: LightTables,
    pub(in crate::game) motion: MotionResources,
}

/// [`CoreTables`] at `root`. Every table reads the block registry; past it,
/// the block models and sprites (then the item models, which share their
/// parsed models), the face visibility, the collisions, the biomes and the
/// environment need no block facts and load beside them. The outer error
/// fails the load; the inner one is the packs' (the registry or the block
/// models), which the client entry can roll back from.
pub fn load_core<P: CorePresentation>(
    resources: &sunrise_assets::resource_source::ResourceRoot,
    additional_data: &[Arc<sunrise_level::datapack::DataPack>],
    conditions: &sunrise_world::conditions::LoadConditions,
    emissive_sprites: &[mod_api::Answered<mod_api::EmissiveSpriteRegistration>],
    load_role: mods::LoadRole,
    captured: sunrise_world::native_load_context::SettingsCapture,
    parallel: bool,
    progress: &Progress,
    launch_world: Option<&str>,
) -> std::result::Result<std::result::Result<(CoreTables, P::Output), String>, String> {
    let role = load_role.execution;
    let root = resources.prepared();
    let (assets, mut early) = P::alongside_assets(resources, parallel, || {
        startup::time("core.assets", || Assets::load(root))
    });
    let assets = match assets {
        Ok(assets) => assets,
        Err(e) => return Ok(Err(e.to_string())),
    };
    progress.set(1);
    let item_data =
        Arc::new(mods::load_item_data(root, role, captured).map_err(|e| e.to_string())?);
    let registry = assets.registry;
    let (render, gameplay, visibility, biomes, environment, collision) =
        std::thread::scope(|scope| {
            let assets = &assets;
            let item_data = &item_data;
            let render = Step::spawn(scope, parallel, "render", move || {
                let visual = P::render(resources, assets, &mut early, emissive_sprites)?;
                let item_defaults = startup::time("core.item_defaults", || {
                    sunrise_world::item_defaults::shared(root)
                })
                .map_err(|e| e.to_string())?;
                Ok::<_, String>((
                    P::finish(resources, early, visual, item_data),
                    item_defaults,
                ))
            });
            let visibility = Step::spawn(scope, parallel, "visibility", || {
                P::visibility(root, registry)
            });
            let biomes = Step::spawn(scope, parallel, "biomes", || {
                let biomes = Arc::new(startup::time("core.biomes", || P::biomes(resources))?);
                // A launch that names a world: its data preload needs only the
                // block registry and the biomes, so it starts here rather than
                // after the block models and facts (~250 ms later).
                if let Some(folder) = launch_world {
                    crate::game::world_preload::preload_launch_world(
                        folder,
                        root,
                        resources.data(),
                        resources.data_changes(),
                        registry,
                        additional_data,
                        conditions.clone(),
                    );
                }
                Ok::<_, String>(biomes)
            });
            let environment = Step::spawn(scope, parallel, "environment", || {
                startup::time("core.environment", || Environment::load(resources.source()))
                    .map_err(|e| e.to_string())
            });
            let collision = Step::spawn(scope, parallel, "collision", || {
                startup::time("core.collision", || collisions::load(root, registry))
                    .map_err(|e| e.to_string())
            });
            let gameplay = gameplay_tables(root, assets, conditions, parallel, progress);
            (
                render.join(),
                gameplay,
                visibility.join(),
                biomes.join(),
                environment.join(),
                collision.join(),
            )
        });
    // The packs' failure first: it rolls back whatever else failed.
    let (client, item_defaults) = match render {
        Ok(render) => render,
        Err(e) => return Ok(Err(e)),
    };
    let gameplay = gameplay?;
    progress.set(3);
    let biomes = biomes?;
    let environment = environment?;
    let visibility = visibility?;
    let client = client?;
    let (item_models, item_ids) = P::item_tables(root, &item_data, &client)?;
    Ok(Ok((
        CoreTables {
            item_data,
            item_defaults,
            gameplay,
            biomes,
            environment,
            visibility,
            item_models,
            item_ids,
            collision: Arc::new(collision?),
            assets,
        },
        client,
    )))
}
/// The block facts, then the light tables beside the motion tables, which
/// both read them.
pub(in crate::game) fn gameplay_tables(
    root: &std::path::Path,
    assets: &Assets,
    conditions: &sunrise_world::conditions::LoadConditions,
    parallel: bool,
    progress: &Progress,
) -> std::result::Result<GameplayTables, String> {
    let facts_step = startup::step("core.block_facts");
    // Read whole, then parsed from the buffer: `serde_json::from_reader`
    // pulls through a `BufReader` and is measurably slower on a file this
    // size than one read plus `from_slice`, which is how `BlockFacts` next
    // door already reads its own. Same value either way.
    let behaviour_json: serde_json::Value =
        sunrise_world::json_file::read(&root.join("block-behaviours.json"))
            .map_err(|e| e.to_string())?;
    #[cfg(not(test))]
    let (facts, gameplay_projection) =
        BlockFacts::load_with_projection(root, assets.registry, &behaviour_json, parallel)
            .map_err(|e| e.to_string())?;
    #[cfg(test)]
    let (facts, gameplay_projection) = if SKIP_FACTS_MEMO.get() {
        BlockFacts::load_with_projection(root, assets.registry, &behaviour_json, parallel)
            .map_err(|e| e.to_string())?
    } else {
        facts_memo::load(root, assets.registry, &behaviour_json, parallel)?
    };
    drop(facts_step);
    progress.set(2);
    let (light, motion) = std::thread::scope(|scope| {
        let light = Step::spawn(scope, parallel, "light", || {
            startup::time("core.light", || {
                LightTables::load(root, assets.registry, &facts)
            })
            .map_err(|e| e.to_string())
        });
        let motion = startup::time("core.motion", || {
            let mut motion =
                MotionResources::load_with_conditions(root, assets, &facts, conditions)?;
            // #812: the attribute types mods declare come first, appended
            // after every vanilla attribute in the sorted catalog order, so
            // the rows below may name a mod's own new attribute and the
            // network ids stay the same on every side running these mods.
            let declared = mod_api::__private::attribute_declarations();
            if !declared.is_empty() {
                motion.attributes =
                    std::sync::Arc::new(with_mod_attributes(&motion.attributes, declared)?);
            }
            // #3728: then the bounds mods widen, on vanilla's attributes and
            // the declared ones alike, before anything reads a definition.
            let widenings = mod_api::__private::attribute_bounds_widenings();
            if !widenings.is_empty() {
                motion.attributes =
                    std::sync::Arc::new(with_mod_widened_bounds(&motion.attributes, widenings)?);
            }
            // #845: a mod's declared attribute sets
            // (`EntityAttributeCreationEvent`) become their entity types' rows
            // in the live attribute map's catalog — the one record the mob
            // logic and the attribute sync read (#1493) — built from the
            // template's row, as the type's content row is.
            for declaration in mod_api::__private::entity_attribute_declarations() {
                motion.attributes = std::sync::Arc::new(
                    motion
                        .attributes
                        .with_entity_row(
                            &declaration.id,
                            &declaration.template,
                            &declaration.attributes,
                        )
                        .map_err(|error| {
                            format!("mod entity attributes of {}: {error}", declaration.id)
                        })?,
                );
            }
            // #846: then the attributes mods add to existing types
            // (`EntityAttributeModificationEvent`, fired after the creation
            // event), on the same catalog rows.
            let modifications = mod_api::__private::entity_attribute_modifications();
            if !modifications.is_empty() {
                motion.attributes = std::sync::Arc::new(with_mod_attribute_additions(
                    &motion.attributes,
                    modifications,
                )?);
            }
            // #4473: then the attributes the content mods' effects declare
            // modifiers on, which the complete catalog knows.
            check_effect_modifier_attributes(
                &motion.attributes,
                sunrise_game_data::potions::mod_effects(),
            )?;
            Ok::<_, Box<dyn std::error::Error>>(motion)
        })
        .map_err(|e| e.to_string());
        (light.join(), motion)
    });
    Ok(GameplayTables {
        light: light?,
        motion: motion?,
        behaviour_json,
        facts,
        gameplay_projection,
    })
}

/// Publishes `catalog`'s entity type suppliers, the ones mods declared and added to included,
/// as `ItemsHost::entity_type_default_attribute` reads them (#3944): each attribute's supplier
/// value (`AttributeSupplier.getValue`, the base inside its range; the spawn and reset bases
/// stay the raw ones). The read answers only a registered entity type's: a declaration for a
/// type no mod registered stays unread.
pub fn publish_entity_type_defaults(catalog: &sunrise_world::attributes::entity::Catalog) {
    mod_api::__private::publish_entity_type_defaults(
        catalog
            .entities()
            .filter_map(|entity| Some((entity.to_owned(), catalog.supplier_values(entity)?))),
    );
}

/// The catalog with the attribute types mods declared (#812,
/// `Registrar::declare_attribute`), appended after every vanilla attribute:
/// `Catalog::with_attributes` sorts them by declaring mod id, then attribute
/// id, and refuses a duplicate, naming it.
pub(in crate::game) fn with_mod_attributes(
    catalog: &sunrise_world::attributes::entity::Catalog,
    declared: &[mod_api::__private::AttributeDeclaration],
) -> std::result::Result<sunrise_world::attributes::entity::Catalog, String> {
    let added = declared
        .iter()
        .map(|declared| {
            Ok(sunrise_world::attributes::entity::AddedAttribute {
                mod_id: declared.mod_id.clone(),
                id: declared.id.clone(),
                definition: sunrise_world::attributes::Definition::ranged(
                    declared.default,
                    declared.min,
                    declared.max,
                )
                .map_err(|error| format!("mod attribute {}: {error}", declared.id))?,
                syncable: declared.syncable,
                tooltip: sunrise_world::attributes::Tooltip {
                    sentiment: match declared.sentiment {
                        mod_api::Sentiment::Positive => {
                            sunrise_world::attributes::Sentiment::Positive
                        }
                        mod_api::Sentiment::Negative => {
                            sunrise_world::attributes::Sentiment::Negative
                        }
                        mod_api::Sentiment::Neutral => {
                            sunrise_world::attributes::Sentiment::Neutral
                        }
                    },
                    display: match declared.tooltip.display {
                        mod_api::AttributeDisplay::Ranged => {
                            sunrise_world::attributes::Display::Ranged
                        }
                        mod_api::AttributeDisplay::Percentage => {
                            sunrise_world::attributes::Display::Percentage
                        }
                        mod_api::AttributeDisplay::Boolean => {
                            sunrise_world::attributes::Display::Boolean
                        }
                    },
                    base_modifier: declared.tooltip.base_modifier.clone(),
                },
            })
        })
        .collect::<std::result::Result<Vec<_>, String>>()?;
    catalog
        .with_attributes(&added)
        .map_err(|error| format!("mod attribute declarations: {error}"))
}

/// The catalog with the attribute bounds mods widened (#3728,
/// `Registrar::widen_attribute_bounds`): the union of every ask, whatever the
/// load order. An unknown attribute, or a range without the attribute's
/// default, refuses the load, naming the mod.
pub(in crate::game) fn with_mod_widened_bounds(
    catalog: &sunrise_world::attributes::entity::Catalog,
    widenings: &[mod_api::__private::AttributeBoundsWidening],
) -> std::result::Result<sunrise_world::attributes::entity::Catalog, String> {
    let asks: Vec<_> = widenings
        .iter()
        .map(|ask| sunrise_world::attributes::entity::WidenedBounds {
            mod_id: ask.mod_id.clone(),
            id: ask.id.clone(),
            min: ask.min,
            max: ask.max,
        })
        .collect();
    catalog
        .with_widened_bounds(&asks)
        .map_err(|error| format!("mod attribute bounds: {error}"))
}

/// The catalog with the attributes mods add to existing entity types
/// (#846, `EntityAttributeModificationEvent`), in registration order, so a
/// later addition of the same attribute wins. An unknown attribute or a type
/// without a row refuses the load, naming the type.
pub fn with_mod_attribute_additions(
    catalog: &sunrise_world::attributes::entity::Catalog,
    modifications: &[mod_api::__private::EntityAttributeModification],
) -> std::result::Result<sunrise_world::attributes::entity::Catalog, String> {
    let mut catalog = catalog.clone();
    for modification in modifications {
        catalog = catalog
            .with_entity_additions(&modification.entity, &modification.attributes)
            .map_err(|error| {
                format!(
                    "mod entity attribute additions to {}: {error}",
                    modification.entity
                )
            })?;
    }
    Ok(catalog)
}

/// Refuses the load when a content mod's effect declares a modifier on an
/// attribute the catalog does not have, vanilla's and the mods' declared ones
/// included (#4473), naming the effect and the attribute.
pub fn check_effect_modifier_attributes(
    catalog: &sunrise_world::attributes::entity::Catalog,
    effects: &[sunrise_game_data::potions::EffectDef],
) -> std::result::Result<(), String> {
    for effect in effects {
        for &(attribute, ..) in effect.modifiers {
            if catalog.id(attribute).is_none() {
                return Err(format!(
                    "mob effect {}: attribute_modifiers: unknown attribute {attribute}",
                    effect.id
                ));
            }
        }
    }
    Ok(())
}

/// Each mob type mods added attributes to takes its grown catalog row
/// (`with_mod_attribute_additions`) as its attribute map and spawn fields, as
/// a declared set's type does (#845), so a new mob of the type spawns with
/// what the live map answers. A type with no mob type (the player) builds
/// its map from the catalog alone.
pub fn take_modified_attribute_rows(
    types: &mut HashMap<String, sunrise_sim::mobs::MobType>,
    catalog: &Arc<sunrise_world::attributes::entity::Catalog>,
    modifications: &[mod_api::__private::EntityAttributeModification],
) {
    let modified: std::collections::BTreeSet<&str> = modifications
        .iter()
        .map(|modification| modification.entity.as_str())
        .collect();
    for entity in modified {
        if let (Some(kind), Some(row)) = (types.get_mut(entity), catalog.entity_row(entity)) {
            kind.take_attribute_row(Arc::new(row), catalog);
        }
    }
}

/// Write the prepared folder's `sunrise-data.bin` when it has none the game
/// can use: on a thread of its own, at idle priority, once the start is done.
/// This start has just read the folder's files (they are in the disk cache);
/// the next starts read them from one file (#1961).
pub fn write_prepared_files(root: PathBuf) {
    use sunrise_core::prepared_files;
    if prepared_files::open(&root).is_some() || !root.join("sunrise-resources.json").is_file() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
        .name("Prepared data file".into())
        .spawn(move || {
            sunrise_threads::idle_current();
            let started = Instant::now();
            match prepared_files::write(&root) {
                Ok(()) => log::info!(
                    "Sunrise: wrote {} in {} ms",
                    prepared_files::FILE_NAME,
                    started.elapsed().as_millis()
                ),
                Err(e) => log::info!("Sunrise: {} not written: {e}", prepared_files::FILE_NAME),
            }
        });
    if let Err(e) = spawned {
        log::info!(
            "Sunrise: {} not written: {e}",
            sunrise_core::prepared_files::FILE_NAME
        );
    }
}

pub struct PreparedCore {
    pub pack: PackResources,
    pub facts: BlockFacts,
    pub biomes: Arc<Biomes>,
    pub item_data: Arc<ItemData>,
    pub item_defaults: Arc<ItemDefaults>,
    pub assets: Arc<Assets>,
    pub visibility: FaceVisibility,
    pub behaviour_json: serde_json::Value,
    pub gameplay_projection: Vec<sunrise_items::blocks::GameplayProjection>,
    pub motion: MotionResources,
    pub environment: Environment,
    pub collision: Arc<BlockCollisionRegistry<'static>>,
    pub spawner_types: HashMap<String, sunrise_sim::mobs::spawner::SpawnerTypeFacts>,
}

pub fn prepare_core(
    core: CoreTables,
    root: &Path,
    resolved: &ResourceRoot,
    additional_data: &[Arc<sunrise_level::datapack::DataPack>],
    conditions: &sunrise_world::conditions::LoadConditions,
    role: crate::game::execution_role::ExecutionRole,
    launch_world: Option<&str>,
    started: Instant,
) -> Result<(PreparedCore, Option<sunrise_level::spans::startup::Step>)> {
    // The same prepared gameplay root supplies entity keys and spawner facts,
    // regardless of which client resource layers loaded successfully.
    let spawner_types = mods::register_iterated_registries(root)?;
    log::info!("load core tables: {:.2}s", started.elapsed().as_secs_f32());
    let CoreTables {
        item_data,
        item_defaults,
        assets,
        visibility,
        item_models,
        item_ids,
        gameplay:
            GameplayTables {
                behaviour_json,
                facts,
                gameplay_projection,
                light,
                motion,
            },
        biomes,
        environment,
        collision,
    } = core;
    // The accepted suppliers and resolved entity keys belong to the same Parts installation.
    publish_entity_type_defaults(&motion.attributes);
    let assets = Arc::new(assets);
    // The namespaces of the mod block states registered now: what every level.dat saved from
    // here on records, and what a world is checked against before it opens (`level_mods`). A
    // launch world's data preload, which may already have started, reads no mod records; its
    // open, which checks them, waits for these resources.
    {
        use sunrise_sim::save::level_mods;
        let registry = assets.registry;
        let names = (0..registry.len() as u32)
            .filter_map(|id| registry.state(sunrise_world::registry::BlockStateId(id)))
            .map(|state| state.name.as_str());
        level_mods::set_block_namespaces(Some(level_mods::mod_namespaces(names)));
    }
    if let Some(folder) = launch_world {
        crate::game::world_preload::preload_launch_world(
            folder,
            root,
            resolved.data(),
            resolved.data_changes(),
            assets.registry,
            additional_data,
            conditions.clone(),
        );
    }
    api_values::resolve_block_definitions_for_role(assets.registry, role)?;
    mods::extend_poi_types(assets.registry);
    mods::register_cauldron_fluid_contents(assets.registry);
    let pack_step = startup::step("pack.assemble");
    let pack = PackResources::assemble(
        resolved,
        PackParts {
            assets: assets.clone(),
            biomes: biomes.clone(),
            light: Arc::new(light),
            item_models,
            item_ids,
        },
    )?;
    Ok((
        PreparedCore {
            pack,
            facts,
            biomes,
            item_data,
            item_defaults,
            assets,
            visibility,
            behaviour_json,
            gameplay_projection,
            motion,
            environment,
            collision,
            spawner_types,
        },
        pack_step,
    ))
}

pub fn finish_core(
    root: &Path,
    item_data: Arc<ItemData>,
    facts: BlockFacts,
    biomes: Arc<Biomes>,
    environment: Environment,
    collision: Arc<BlockCollisionRegistry<'static>>,
    motion: MotionResources,
    threads: usize,
) -> Result<Arc<Resources>> {
    let suffocation =
        sunrise_sim::mobs::suffocation::bind(collision.clone(), &motion.causes_suffocation)?;
    let resources = Resources {
        core: WorldCore {
            item_data,
            facts,
            biomes,
            environment,
            collision,
            motion,
            suffocation,
            variants: Arc::new(sunrise_sim::mobs::variants::VariantTables::load_prepared(
                root,
            )),
            threads,
        },
        rest: std::sync::OnceLock::new(),
    };
    Ok(Arc::new(resources))
}
