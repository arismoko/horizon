//! Prepare local worlds without blocking the window's event loop.
use super::*;
use sunrise_core::identifier::Identifier;
use sunrise_gameplay_common::game::condition_reload_state::WorldBiomeRegistry;
use sunrise_gameplay_common::game::growth_features;
use sunrise_level::{chunk_streamer::ChunkStreamer, sim_settings::SimSettings};
use sunrise_sim::vanillagen::{TerrainResources, shared_biome_source_data, shared_resource_data};
use sunrise_world::worldgen::{
    biome_source::{BiomeSourceError, ResolvedBiomeSource},
    generator::data::{BiomeSourceData, GeneratorData},
    mod_generators::{ModChunkGenerator, ModChunkGenerators},
};

pub struct WorldRequest {
    pub save_root: PathBuf,
    pub folder: String,
    pub name: Option<String>,
    pub seed: Option<i64>,
    pub creative: bool,
    pub create: Option<sunrise_sim::world_options::WorldOptions>,
    pub rule_registry: Arc<sunrise_game_data::gamerules::RuleRegistry>,
    /// Where the server's region workers send their notices.
    pub chunk_reports: Arc<sunrise_sim::chunk_io::Reports>,
    /// The account the world opens under, for an integrated server
    /// (`IntegratedServer.getSingleplayerProfile`): its name and UUID.
    pub owner: Option<(String, u128)>,
    #[cfg(test)]
    pub(super) stem_order: Option<&'static [&'static str]>,
}

pub struct PreparedWorld {
    pub save_root: PathBuf,
    pub level: Level,
    pub session_lock: sunrise_sim::save::SessionLock,
    pub rand_sequences: sunrise_items::random_sequences::RandomSequences,
    /// The owner's offline UUID and the account's, when an older world moved its
    /// owner at open (`playerdata::migrate_owner`): entities migrate as they load.
    pub owner_migration: Option<(u128, u128)>,
    created: bool,
    save_reporter: Arc<sunrise_sim::chunk_io::SaveFailureReporter>,
    chunk_reports: Arc<sunrise_sim::chunk_io::Reports>,
    #[cfg(test)]
    stem_order: Option<&'static [&'static str]>,
}

pub struct PreparedTerrain {
    pub shapes: sunrise_level::chunk::LevelShapes,
    pub configured_levels: Vec<Dim>,
    pub world: World,
    pub streaming: ChunkStreamer,
    pub save: Arc<SaveDir>,
    pub locator: Option<Arc<VanillaTerrain>>,
}

/// The dedicated server's `Main` takes level.dat_old without asking
/// (`getUnfixedDataTagWithFallback`): there is no world list to recover from. The integrated
/// server's world opens from that list, which offers the recovery instead, whatever the launch
/// configuration says.
fn fall_back_to_level_dat_old_without_asking(
    role: execution_role::ExecutionRole,
    save_root: &std::path::Path,
    folder: &str,
) {
    if role.is_dedicated_server() && sunrise_sim::save::fall_back_to_level_dat_old(save_root) {
        log::warn!("Sunrise: level.dat of \"{folder}\" did not read; level.dat_old took its place");
    }
}

/// Settle level.dat before applying any on-disk choice for the world's generator.
fn prepare_level_data(
    role: execution_role::ExecutionRole,
    save_root: &std::path::Path,
    folder: &str,
    game_dir: &std::path::Path,
) -> Result<()> {
    // openWorldLoadLevelData: level data that does not read is RecoverWorldDataScreen's to
    // handle (the title screen shows it); a world opened another way must not become a
    // new one with a random seed under the same name.
    fall_back_to_level_dat_old_without_asking(role, save_root, folder);
    if let Some(issue) = sunrise_sim::save::level_data_needs_recovery(save_root) {
        let why = match issue {
            sunrise_sim::save::DatIssue::Missing => "level.dat is missing".to_owned(),
            sunrise_sim::save::DatIssue::Unreadable(why) => why,
        };
        return Err(format!(
            "Failed to load world \"{folder}\": its level data does not read ({why}). \
             Open it from the world list to try restoring level.dat_old."
        )
        .into());
    }
    // LevelSummary.isCompatible, which the dedicated server's `Main` checks before it fixes
    // a world: a world from another series is refused, never upgraded in place.
    if let Some(version) = sunrise_sim::save::incompatible_version(save_root) {
        let version = if version.is_empty() {
            "unknown".to_owned()
        } else {
            version
        };
        return Err(format!(
            "Failed to load world \"{folder}\": it was last played in version {version}, \
             which cannot be opened in this version."
        )
        .into());
    }
    if role.is_dedicated_server() && save_root.join("level.dat").exists() {
        super::dedicated::biome_regions::choose(save_root, &game_dir.join("server.toml"))?;
    }
    Ok(())
}

/// `MinecraftServer.getServerDirectory` and its integrated override.
fn chunk_report_root<'a>(
    role: execution_role::ExecutionRole,
    game_dir: Option<&'a std::path::Path>,
    working_dir: &'a std::path::Path,
) -> &'a std::path::Path {
    if role == execution_role::ExecutionRole::IntegratedServer {
        // `Options::read` may already have made the configured game directory the cwd.
        game_dir.unwrap_or(working_dir)
    } else {
        working_dir
    }
}

impl WorldRequest {
    pub fn new(
        folder: &str,
        name: Option<&str>,
        seed: Option<i64>,
        creative: bool,
        create: Option<&sunrise_sim::world_options::WorldOptions>,
        rule_registry: Arc<sunrise_game_data::gamerules::RuleRegistry>,
    ) -> Result<Self> {
        Ok(Self {
            save_root: std::env::current_dir()?.join("saves").join(folder),
            folder: folder.to_owned(),
            name: name.map(str::to_owned),
            seed,
            creative,
            create: create.cloned(),
            rule_registry,
            chunk_reports: sunrise_sim::chunk_io::PROCESS_REPORTS.clone(),
            owner: None,
            #[cfg(test)]
            stem_order: None,
        })
    }

    /// The account this world opens under (`IntegratedServer.getSingleplayerProfile`):
    /// a world older builds made under the offline profile moves its owner to it at
    /// open. A dedicated server has no single-player owner and leaves it unset.
    pub(in crate::game::server_side) fn owned_by(mut self, player: &str, owner: u128) -> Self {
        self.owner = Some((player.to_owned(), owner));
        self
    }

    pub fn prepare(
        self,
        res: &Resources,
        pack: &PackResources,
        options: &Options,
        upgrade: &sunrise_world::datafix::progress::UpgradeProgress,
    ) -> Result<PreparedWorld> {
        let _prepare_step = startup::step("world.prepare");
        #[cfg(test)]
        let stem_order = self.stem_order;
        let Self {
            save_root,
            folder,
            name,
            seed,
            creative,
            create,
            rule_registry,
            chunk_reports,
            owner,
            ..
        } = self;
        let name = name.as_deref();
        sunrise_level::biome::Biomes::forget_missing_warnings();
        let create = create.map(|mut create| {
            create.set_additional_data_packs(pack.additional_data.clone());
            create
        });
        let create = create.as_ref();
        prepare_level_data(
            options.execution_role,
            &save_root,
            &folder,
            options
                .game_dir
                .as_deref()
                .unwrap_or(std::path::Path::new(".")),
        )?;
        sunrise_sim::save::upgrade_older_world_with(&save_root, upgrade)?;
        sunrise_sim::save::migrate_legacy(&save_root)?;
        let session_lock = sunrise_sim::save::SessionLock::acquire(&save_root)?;
        // RandomSequences: the world's data/minecraft/random_sequences.dat.
        let rand_sequences = super::trading::read_random_sequences(&save_root);
        let existing = Level::read_with_registry(&save_root, rule_registry.clone());
        let created = existing.is_none();
        let level = existing.unwrap_or_else(|| Level {
            name: name.unwrap_or(&folder).to_owned(),
            // WorldOptions.randomSeed when no seed was given.
            seed: seed.unwrap_or_else(|| {
                let t = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64);
                (t ^ (t >> 29)).wrapping_mul(0x9E37_79B9_7F4A_7C15) as i64
            }),
            creative,
            day_time: 0,
            health: 20.0,
            food: 20,
            saturation: 5.0,
            rule_registry,
            ..Level::default()
        });
        let mut level = level;
        // `IntegratedServer.setSingleplayerProfile`: a world older Sunrise builds made
        // under the offline profile moves its owner to the account at open — the player
        // file, its `UUID` and level.dat's `singleplayer_uuid`. Its entities follow as
        // their chunks load (`owner_migration`, `entity_nbt::migrate_player_uuid`).
        let owner_migration = owner.and_then(|(player, account)| {
            if created {
                // The profile exists before `IntegratedServer.initServer` saves; the player does not.
                level.player_uuid = Some(sunrise_sim::lan::playerdata::uuid_ints(account));
            }
            let old = sunrise_protocol::crypto::offline_uuid(&player);
            (old != account).then(|| {
                if !created
                    && sunrise_sim::lan::playerdata::migrate_owner(
                        &save_root,
                        &mut level,
                        old,
                        account,
                        res.items(),
                    )
                {
                    log::info!(
                        "Sunrise: \"{folder}\" was made under the offline profile; \
                         its owner moved to the account's UUID"
                    );
                }
                (old, account)
            })
        });
        // CreateWorldScreen's choices, before streaming reads the generator.
        if created && let Some(o) = create {
            level.hardcore = o.hardcore;
            level.difficulty = sunrise_sim::world_options::DIFFICULTIES[o.difficulty()].into();
            level.allow_commands = Some(o.commands(creative));
            o.write_world_gen(&save_root, level.seed)?;
            // The DataPackConfig the screen chose: the picked packs on,
            // everything else disabled — the feature packs left out, and the
            // world packs the Data Packs screen held back (`install` copies
            // every pack of the folder, selected or not).
            let packs = sunrise_level::datapack::feature_packs(&options.resources);
            level.data_packs = Some({
                let mut disabled: Vec<String> = packs
                    .iter()
                    .map(|p| p.id.clone())
                    .filter(|f| !o.data_packs.contains(f))
                    .collect();
                if let Some(dir) = o.pack_dir.as_deref() {
                    disabled.extend(
                        sunrise_level::pack_meta::folder_meta(
                            std::path::Path::new(dir),
                            sunrise_core::version::DATA_PACK_FORMAT,
                        )
                        .into_iter()
                        .map(|p| p.id)
                        .filter(|id| !o.data_packs.contains(id)),
                    );
                }
                (o.data_packs.clone(), disabled)
            });
            // WorldDataConfiguration's `enabled_features`: the flags of the
            // picked feature packs over vanilla's (`FeatureFlags.REGISTRY`'s
            // codec writes the whole set, `minecraft:vanilla` included, and
            // omits the tag when it is the default).
            level.enabled_features = o.enabled_features(&options.resources).into();
            // CreateWorldScreen copies the edited rules with the final flags,
            // even when the player never opened the rules screen.
            level.game_rules = o
                .rules
                .copy_with_features(&level.enabled_features)
                .by_name();
        }
        // Worlds made without the Create New World screen (--world, the
        // control socket) take its default: commands on for Creative only.
        if created {
            level.allow_commands.get_or_insert(creative);
        }
        // The biome regions are fixed before any chunk generates: a new world records the
        // installed mods' regions; an existing one opens only with a record that reads and a
        // recorded choice for each missing mod's region.
        if created {
            sunrise_sim::save::biome_regions::create(&save_root)?;
        } else {
            sunrise_sim::save::biome_regions::open(&save_root)
                .map_err(|why| format!("Failed to load world \"{folder}\": {why}"))?;
        }
        if let Some(d) = &options.dimension {
            level.dimension = d.clone();
            level.position = None;
        }
        let integrated = options.execution_role == execution_role::ExecutionRole::IntegratedServer;
        // Resolving the report path must not prevent the world from opening.
        let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let server_root = chunk_report_root(
            options.execution_role,
            options.game_dir.as_deref(),
            &working_dir,
        );
        // Dedicated requests carry a storage path as `folder` and a level id as `name`.
        let level_id = if options.execution_role.is_dedicated_server() {
            name.unwrap_or(&folder)
        } else {
            &folder
        };
        let save_reporter = Arc::new(sunrise_sim::chunk_io::SaveFailureReporter::new(
            integrated,
            server_root,
            level_id,
        ));
        Ok(PreparedWorld {
            save_root,
            level,
            session_lock,
            rand_sequences,
            owner_migration,
            created,
            save_reporter,
            chunk_reports,
            #[cfg(test)]
            stem_order,
        })
    }
}

impl PreparedWorld {
    pub fn prepare_terrain(
        &mut self,
        res: &Resources,
        pack: &PackResources,
        options: &Options,
        settings: &SimSettings,
        biomes: &WorldBiomeRegistry,
    ) -> Result<PreparedTerrain> {
        let save_root = &self.save_root;
        let level = &mut self.level;
        let mut overrides = biomes.overrides.clone();
        if self.created {
            level.generator = sunrise_sim::save::WorldGenerator::read_new(save_root);
        } else if options.execution_role.is_dedicated_server() {
            level.recover_world_generator(&overrides, || {
                sunrise_level::random_source::create().next_long()
            })?;
        }
        growth_features::install_live_features(res, pack, options, &overrides);
        let (shapes, configured_levels) = level_shapes(
            &mut overrides,
            &options.resources,
            level,
            #[cfg(test)]
            self.stem_order,
        );
        Dim::set_extra(shapes.extra.iter().map(|level| level.id));
        // The bounded registry-loader leaf slice: fail world preparation before streaming,
        // rather than letting the flat-generator fallback swallow a selected pack's codec error.
        overrides.validate_fill_layers(pack.assets.registry)?;
        // PrepareSpawnTask: resolve the player's dimension against this world's stems,
        // without changing the saved position or adding a stem for the player.
        let dim = configured_levels
            .iter()
            .find(|dim| dim.id() == level.dimension)
            .or_else(|| {
                let spawn = level.world_spawn_dimension.as_deref()?;
                configured_levels.iter().find(|dim| dim.id() == spawn)
            })
            .copied()
            .unwrap_or(Dim::Overworld);
        level.dimension = dim.id().to_owned();
        let shape = shapes.get(dim.id());
        let streaming_step = startup::step("world.make_streaming");
        let (mut streaming, save, locator, generator_type) = make_streaming(
            res,
            pack,
            options,
            settings,
            save_root,
            dim,
            shape,
            level.seed,
            level,
            biomes.table.clone(),
            &overrides,
            None,
            self.chunk_reports.clone(),
            self.save_reporter.clone(),
        )?;
        drop(streaming_step);
        streaming.radius = settings.render_distance;
        let mut world =
            sunrise_gameplay_common::game::authoritative_world_for_mods(res, level.seed);
        world.attachments = save.load_attachments();
        world.shape = shape;
        world.generator_type = Some(generator_type);
        world.fast_lava = sunrise_world::dimension_type::DimensionFacts::of_level(
            sunrise_core::LevelKey::parse(dim.id()).unwrap_or(sunrise_core::LevelKey::OVERWORLD),
        )
        .fast_lava;
        world.game_time = level.time;
        world.day_time = options.time.unwrap_or(level.day_time);
        world.end_clock = level.end_clock.unwrap_or(level.time);
        world.weather = level.weather;
        if let Some(w) = options.weather.as_deref() {
            // /weather: clear, rain or thunder for a long duration.
            let (rain, thunder) = (w != "clear", w == "thunder");
            world.weather = sunrise_level::world::Weather {
                clear_time: if w == "clear" { 6000 } else { 0 },
                rain_time: 6000,
                thunder_time: 6000,
                raining: rain,
                thundering: thunder,
                rain_level: if rain { (1.0, 1.0) } else { (0.0, 0.0) },
                thunder_level: if thunder { (1.0, 1.0) } else { (0.0, 0.0) },
            };
        }

        // IntegratedServer.initServer writes the first level.dat after preparing the levels.
        // The worker must not write an initialized world or the unplaced owner's player file.
        if options.execution_role != execution_role::ExecutionRole::IntegratedServer
            && !save_root.join("level.dat").exists()
        {
            level.write(save_root, res.items())?;
        }
        Ok(PreparedTerrain {
            shapes,
            configured_levels,
            world,
            streaming,
            save,
            locator,
        })
    }
}

pub struct WorldLoader {
    handle: JoinHandle<std::result::Result<PreparedWorld, String>>,
}

impl WorldLoader {
    /// Prepare the world on its own thread. `upgrade` is what its upgrade has done
    /// (`UpgradeProgress`), which the window's `FileFixerProgressScreen` shows while the
    /// file fixers run, and where its Cancel stops them.
    pub fn start(
        request: WorldRequest,
        res: Arc<Resources>,
        pack: PackResources,
        options: Options,
        upgrade: Arc<sunrise_world::datafix::progress::UpgradeProgress>,
    ) -> Result<Self> {
        let handle = std::thread::Builder::new()
            .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
            .name("World initialization".into())
            .spawn(move || {
                request
                    .prepare(&res, &pack, &options, &upgrade)
                    .map_err(|e| e.to_string())
            })?;
        Ok(Self { handle })
    }

    pub fn finish(self) -> Result<PreparedWorld> {
        self.handle
            .join()
            .map_err(|_| "world initialization worker panicked")?
            .map_err(Into::into)
    }
}

pub fn worldgen_overrides_of(
    data: &sunrise_level::datapack::DataPacks,
    conditions: &sunrise_world::conditions::LoadConditions,
) -> std::result::Result<
    (
        sunrise_world::worldgen::overrides::WorldgenOverrides,
        Vec<String>,
    ),
    String,
> {
    let loaded = sunrise_level::datapack::worldgen_overrides(data, conditions);
    conditions.check_errors()?;
    Ok(loaded)
}

/// [`worldgen_overrides_of`] over the view of the world a test's fixture folder holds.
#[cfg(any(test, feature = "test-support"))]
pub fn worldgen_overrides_in_world(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    enabled_features: &[String],
    additional: &[Arc<sunrise_level::datapack::DataPack>],
    base_conditions: &sunrise_world::conditions::LoadConditions,
) -> std::result::Result<
    (
        sunrise_world::worldgen::overrides::WorldgenOverrides,
        Vec<String>,
    ),
    String,
> {
    let empty = sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
    let stack = super::functions::pack_stack(
        world_root,
        resources,
        &empty,
        &empty,
        false,
        enabled_features,
        additional,
    );
    let enabled: Vec<_> = stack.enabled_packs().collect();
    let conditions = sunrise_gameplay_common::game::load_conditions::worldgen_load_conditions(
        base_conditions,
        resources,
        &enabled,
    );
    worldgen_overrides_of(&stack.data_packs(Vec::new()), &conditions)
}

fn level_shapes(
    overrides: &mut sunrise_world::worldgen::overrides::WorldgenOverrides,
    resources: &std::path::Path,
    level: &Level,
    #[cfg(test)] stem_order: Option<&[&str]>,
) -> (sunrise_level::chunk::LevelShapes, Vec<Dim>) {
    if let Some(generator) = &level.generator {
        generator.merge_dimensions(overrides);
    } else {
        overrides.include_implicit_level_stems();
    }
    let stems = overrides.level_stems();
    #[cfg(test)]
    let (shapes, stems) = match stem_order {
        Some(order) => {
            assert_eq!(order.len(), overrides.level_stems().count());
            shapes_of(
                overrides,
                order
                    .iter()
                    .map(|id| overrides.level_stems().find(|(key, _)| key == id).unwrap()),
            )
        }
        None => shapes_of(overrides, stems),
    };
    #[cfg(not(test))]
    let (shapes, stems) = shapes_of(overrides, stems);
    Dim::set_extra(shapes.extra.iter().map(|level| level.id));
    set_level_sea_levels(
        overrides,
        sunrise_world::worldgen::overrides::mod_dimensions(),
        resources,
    );
    (shapes, stems.into_iter().map(Dim::from_id).collect())
}

/// The sea levels of the levels whose stem the mods or the data packs give: a mod's own
/// (`DimensionSettings::sea_level`), else the `sea_level` of the noise settings their stem names,
/// the packs' over the game's. The game's spawning, weather and maps then use them. A level of a
/// generator that reads no noise settings has its built-in level's, and the three built-in levels
/// too unless the packs replace their stem or their noise settings.
fn set_level_sea_levels(
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
    mods: &[sunrise_world::worldgen::overrides::ModDimension],
    resources: &std::path::Path,
) {
    use sunrise_world::worldgen::overrides::{DIMENSIONS, NOISE_SETTINGS, stem_sea_level};
    let read = |id: &str| -> Option<serde_json::Value> {
        if let Some((_, json)) = overrides
            .entries(NOISE_SETTINGS)
            .find(|(entry, _)| *entry == id)
        {
            return serde_json::from_str(json).ok();
        }
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let file = resources.join(format!(
            "data/{namespace}/worldgen/noise_settings/{path}.json"
        ));
        serde_json::from_slice(&std::fs::read(file).ok()?).ok()
    };
    let mut levels: Vec<(sunrise_core::LevelKey, i32)> = Vec::new();
    for (id, stem) in overrides.entries(DIMENSIONS) {
        let own = mods.iter().find(|d| d.id == id).and_then(|d| d.sea_level);
        // The noise settings are what the terrain is built with; `sea_level` of the mod is for
        // a generator that has none.
        let stem = serde_json::from_str::<serde_json::Value>(stem).ok();
        // A superflat generator builds at -63 (`FlatLevelSource.getSeaLevel`) whatever its level
        // is told; a mod's explicit sea level still wins for it.
        let flat = stem
            .as_ref()
            .is_some_and(|stem| stem["generator"]["type"].as_str() == Some("minecraft:flat"));
        let sea = stem
            .and_then(|stem| stem_sea_level(&stem, &read))
            .or(own)
            .or(flat.then_some(-63));
        if let (Some(sea), Ok(key)) = (sea, sunrise_core::LevelKey::parse(id)) {
            levels.push((key, sea));
        }
    }
    // A built-in level the packs gave no stem: its default noise settings, which a pack may replace.
    for (id, settings) in [
        ("minecraft:overworld", "minecraft:overworld"),
        ("minecraft:the_nether", "minecraft:nether"),
        ("minecraft:the_end", "minecraft:end"),
    ] {
        let Ok(key) = sunrise_core::LevelKey::parse(id) else {
            continue;
        };
        if levels.iter().any(|(level, _)| *level == key) {
            continue;
        }
        let replaced = overrides
            .entries(NOISE_SETTINGS)
            .any(|(entry, _)| entry == settings);
        let sea = replaced
            .then(|| read(settings))
            .flatten()
            .and_then(|json| i32::try_from(json["sea_level"].as_i64()?).ok());
        if let Some(sea) = sea {
            levels.push((key, sea));
        }
    }
    sunrise_world::level::set_sea_levels(levels);
}

/// The levels' shapes the mods' and packs' worldgen entries give.
fn shapes_of<'a>(
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
    stems: impl Iterator<Item = (&'a str, Option<&'a str>)>,
) -> (sunrise_level::chunk::LevelShapes, Vec<&'static str>) {
    use sunrise_level::chunk::{ExtraLevel, LevelShape, LevelShapes};
    let builtins = [Dim::Overworld.id(), Dim::Nether.id(), Dim::End.id()];
    let mut shapes = LevelShapes::VANILLA;
    let mut extra = Vec::new();
    let mut configured = Vec::new();
    for (level, stem) in stems {
        let shape = overrides.level_height(level).and_then(|h| {
            LevelShape::new(
                h.min_y,
                h.height,
                h.logical_height,
                h.has_skylight,
                h.has_ceiling,
            )
            .ok()
        });
        if let Some(id) = builtins.into_iter().find(|id| *id == level) {
            configured.push(id);
            if let Some(shape) = shape {
                *shapes.get_mut(id) = shape;
            } else {
                log::warn!(
                    "Sunrise: data pack worldgen: the dimension type of {id} is not one the \
                     game accepts; the level keeps the vanilla build height"
                );
            }
        } else if let Some(shape) = shape {
            // A stem's `type` is a dimension type's id, or the type itself.
            let Some(stem) = stem.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            else {
                continue;
            };
            let id: &'static str = level.to_owned().leak();
            let dimension_type = stem["type"].as_str().unwrap_or(level).to_owned().leak();
            extra.push(ExtraLevel {
                id,
                dimension_type,
                shape,
            });
            configured.push(id);
        } else {
            log::warn!(
                "Sunrise: data pack worldgen: the dimension type of {level} is not one the \
                 game accepts; the level is left out"
            );
        }
    }
    shapes.extra = sunrise_level::chunk::ExtraLevels::new(extra);
    set_level_types(overrides, &shapes);
    (shapes, configured)
}

/// The dimension types the mods and packs changed or added, for the levels `shapes` holds: what
/// `DimensionFacts::of_level` answers for them. A type the game refuses keeps the level's own.
fn set_level_types(
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
    shapes: &sunrise_level::chunk::LevelShapes,
) {
    use sunrise_core::LevelKey;
    let levels = [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ]
    .into_iter()
    .chain(shapes.extra.iter().map(|level| level.id));
    let mut facts = Vec::new();
    for level in levels {
        match overrides.level_facts(level) {
            Some(Ok(found)) => facts.extend(LevelKey::parse(level).ok().map(|key| (key, found))),
            Some(Err(why)) => log::warn!(
                "Sunrise: data pack worldgen: the dimension type of {level} sets a field the game                  refuses ({why}); the level keeps the type it would have without it"
            ),
            None => {}
        }
    }
    sunrise_world::dimension_type::DimensionFacts::set_levels(facts);
}

/// A registered generator paired with a biome source bound to the world's registries.
/// What generates a level whose stem names a generator a mod registered.
pub struct ModStem {
    /// Full terrain-stage worldgen resources, loaded only when `generator.vanilla_stages().any()`.
    data: Option<Arc<GeneratorData>>,
    json: serde_json::Value,
    generator_id: Identifier,
    generator: Arc<dyn ModChunkGenerator>,
    pub source: Arc<ResolvedBiomeSource>,
}

impl ModStem {
    pub fn bind(
        biomes: &BiomeSourceData,
        json: serde_json::Value,
        generator_id: Identifier,
        generator: Arc<dyn ModChunkGenerator>,
        seed: i64,
    ) -> std::result::Result<Self, BiomeSourceError> {
        let source = biomes
            .resolve_mod_biome_source(&json["generator"]["biome_source"].to_string(), seed)?;
        Ok(Self {
            data: None,
            json,
            generator_id,
            generator,
            source: Arc::new(source),
        })
    }

    pub fn with_stage_data(mut self, data: Arc<GeneratorData>) -> Self {
        self.data = Some(data);
        self
    }
}

/// What generates a level whose stem names a generator a mod registered.
enum ModLevel {
    /// The generator's terrain alone.
    Terrain(sunrise_sim::modgen::ModTerrain),
    /// Its terrain and the requested vanilla stages over the stem's noise settings.
    Staged(VanillaTerrain),
}

/// Resolve a packed, saved, or runtime stem and return the registered generator's ID.
fn mod_terrain(
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
    terrain_resources: TerrainResources<'_>,
    generators: &ModChunkGenerators,
    dim: Dim,
    shape: sunrise_level::chunk::LevelShape,
    seed: i64,
    structures: bool,
    runtime_stem: Option<&ModStem>,
    saved_stem: Option<&sunrise_core::nbt::Tag>,
) -> Option<std::result::Result<(ModLevel, Identifier), String>> {
    let bound;
    let stem = if let Some(stem) = runtime_stem {
        stem
    } else {
        if generators.is_empty() {
            return None;
        }
        let json: serde_json::Value = overrides
            .entries(sunrise_world::worldgen::overrides::DIMENSIONS)
            .find(|(level, _)| *level == dim.id())
            .and_then(|(_, stem)| serde_json::from_str(stem).ok())
            .or_else(|| saved_stem.map(sunrise_game_data::scoreboard::tag_to_json))?;
        let requested = Identifier::parse_namespaced(json["generator"]["type"].as_str()?)?;
        let (id, generator) = generators.get_key_value(&requested)?;
        let result = shared_biome_source_data(terrain_resources.dir, overrides)
            .map_err(|e| e.to_string())
            .and_then(|biomes| {
                ModStem::bind(&biomes, json, id.clone(), generator.clone(), seed)
                    .map_err(|e| format!("biome source: {e}"))
            })
            .and_then(|stem| {
                if generator.vanilla_stages().any() {
                    let data = shared_resource_data(terrain_resources, overrides)
                        .map_err(|e| e.to_string())?;
                    Ok(stem.with_stage_data(data))
                } else {
                    Ok(stem)
                }
            });
        bound = match result {
            Ok(stem) => stem,
            Err(e) => return Some(Err(format!("{}: {e}", dim.id()))),
        };
        &bound
    };
    let level = if stem.generator.vanilla_stages().any() {
        let Some(data) = stem.data.clone() else {
            return Some(Err(format!(
                "{}: missing worldgen data for vanilla stages",
                dim.id()
            )));
        };
        Some(
            VanillaTerrain::for_mod(
                terrain_resources,
                data,
                seed,
                dim.id(),
                &stem.json,
                stem.generator.clone(),
                stem.source.clone(),
                structures,
            )
            .map(ModLevel::Staged)
            .map_err(|e| format!("{}: {e}", dim.id())),
        )
    } else {
        let biomes = terrain_resources.biomes;
        Some(
            sunrise_sim::modgen::ModTerrain::new(
                dim.id(),
                stem.generator.clone(),
                stem.source.clone(),
                terrain_resources.registry,
                biomes.index.clone(),
                shape,
                seed,
            )
            .map(ModLevel::Terrain),
        )
    };
    level.map(|level| level.map(|level| (level, stem.generator_id.clone())))
}

/// The file beside the world that keeps the structure references of the level `id`. An id with a
/// `/` puts it in a folder of `data/`.
pub(in crate::game::server_side) fn structure_references_file(
    world_root: &std::path::Path,
    id: &str,
) -> std::path::PathBuf {
    world_root.join("data").join(format!(
        "sunrise_structure_references_{}.json",
        id.replace(':', "_")
    ))
}

/// Where `inner` keeps the structure references an explorer map skips, and how it reads a
/// saved chunk's.
fn keep_structure_references(
    inner: &VanillaTerrain,
    world_root: &std::path::Path,
    dim: Dim,
    save: &Arc<SaveDir>,
) {
    inner.set_reference_file(structure_references_file(world_root, dim.id()));
    let save = save.clone();
    inner.set_saved_references(Arc::new(move |name, x, z| {
        save.start_references(x, z, name)
    }));
}

pub fn make_streaming(
    res: &Resources,
    pack: &PackResources,
    options: &Options,
    settings: &SimSettings,
    world_root: &std::path::Path,
    dim: Dim,
    shape: sunrise_level::chunk::LevelShape,
    seed: i64,
    level: &Level,
    biomes: Arc<Biomes>,
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
    runtime: Option<&super::runtime_levels::Generate>,
    chunk_reports: Arc<sunrise_sim::chunk_io::Reports>,
    save_reporter: Arc<sunrise_sim::chunk_io::SaveFailureReporter>,
) -> Result<(
    ChunkStreamer,
    Arc<SaveDir>,
    Option<Arc<VanillaTerrain>>,
    Identifier,
)> {
    use sunrise_sim::save::WorldGen;
    let generator = level.generator.as_ref().map(|g| g.settings());
    let saved_stem = generator.and_then(|s| s.get("dimensions")?.get(dim.id()));
    let mut save = SaveDir::new(
        sunrise_sim::save::dimension_dir(world_root, dim.id()),
        shape,
        Some(res.shared_items()),
    );
    save.mark_server_owned();
    save.set_save_reporting(
        world_root.to_path_buf(),
        dim.id(),
        save_reporter,
        chunk_reports,
    );
    let save = Arc::new(save);
    if dim == Dim::End {
        // Migrate before any chunk reader can cache the old block entities. A side file it
        // can't read stays where it is and costs only its old exits: the End still opens.
        if let Err(e) = save.migrate_end_gateway_exits() {
            log::warn!("Sunrise: End gateway exits not migrated: {e}");
        }
    }
    let worldgen_dir = options.resources.join("worldgen");
    let terrain_resources = TerrainResources {
        data: Some(&pack.data_changes),
        dir: &worldgen_dir,
        registry: pack.assets.registry,
        biomes: &biomes,
        item_data: &res.core.item_data,
    };
    let mut locator = None;
    // Record the codec of the generator that actually builds the level.
    let generator_type;
    let overworld_settings = generator.filter(|_| dim == Dim::Overworld);
    use sunrise_sim::flatgen::FlatTerrain;
    let flat = if runtime.is_none() {
        // Enabled packs own the resolved stem; saved presets fill in levels they don't define.
        let packed_stem = overrides
            .entries(sunrise_world::worldgen::overrides::DIMENSIONS)
            .find(|(id, _)| *id == dim.id())
            .map(|(_, json)| serde_json::from_str::<serde_json::Value>(json))
            .transpose()
            .map_err(|e| format!("{}: cannot decode dimension stem: {e}", dim.id()))?
            .as_ref()
            .map(sunrise_game_data::scoreboard::json_to_tag);
        packed_stem
            .as_ref()
            .or(saved_stem)
            .and_then(|stem| stem.get("generator"))
            .map(FlatTerrain::decode_generator)
            .transpose()
            .map_err(|e| format!("{}: cannot decode flat dimension: {e}", dim.id()))?
            .flatten()
    } else {
        None
    };
    // `DebugLevelSource`: the overworld of the Debug preset.
    let debug = overworld_settings.and_then(|s| {
        sunrise_sim::debugworld::DebugTerrain::from_settings(
            s,
            pack.assets.registry,
            &biomes,
            shape,
        )
    });
    // PlaceCommand.placeJigsaw/placeStructure generate directly, independently
    // of WorldOptions.generateStructures. Keep their flat locator available
    // even when automatic structure starts are disabled.
    let automatic_structures = generator.is_none_or(|s| WorldGen::from_settings(s).structures);
    // The sea level the level's rules and the mods read is the one its generator builds with
    // (`ChunkGenerator.getSeaLevel`), so a pack's noise settings or a superflat world are seen:
    // the superflat generators' is -63 (`FlatLevelSource.getSeaLevel`).
    let register_sea_level = |sea_level: i32| {
        if let Ok(key) = sunrise_core::LevelKey::parse(dim.id()) {
            sunrise_world::level::register_sea_level(key, sea_level);
        }
    };
    let mut streaming = if let Some(super::runtime_levels::Generate::Layers { flat, biome }) =
        runtime
    {
        // A level a mod opened with the default layers or with air alone.
        let inner = sunrise_sim::flatgen::FlatTerrain::of_layers(
            pack.assets.registry,
            shape,
            *flat,
            *biome,
        )
        .ok_or("a runtime level's layers use a block the game lacks")?;
        register_sea_level(-63);
        generator_type = Identifier::parse("minecraft:flat").unwrap();
        ChunkStreamer::new(
            Arc::new(Persistent {
                inner: Arc::new(inner),
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.chunk_context_with(biomes.clone()),
            res.threads(),
        )
    } else if let Some(flat) = flat {
        // `adjustGenerationSettings` decorates independently of generateStructures.
        let staged = FlatTerrain::uses_pipeline(&flat)
            .then(|| {
                VanillaTerrain::for_flat_level(
                    terrain_resources,
                    overrides,
                    seed,
                    &flat,
                    automatic_structures,
                    dim.id(),
                    Some(shape),
                )
                .map(|mut terrain| {
                    terrain.add_mod_signs(res.facts());
                    Arc::new(terrain)
                })
            })
            .transpose()
            .map_err(|e| format!("{}: cannot build flat dimension: {e}", dim.id()))?;
        register_sea_level(-63);
        generator_type = Identifier::parse("minecraft:flat").unwrap();
        locator = staged.clone();
        if let Some(inner) = staged.filter(|_| automatic_structures || flat.features || flat.lakes)
        {
            if automatic_structures {
                inner.search_rings_behind();
            }
            keep_structure_references(&inner, world_root, dim, &save);
            ChunkStreamer::new(
                Arc::new(Persistent {
                    inner,
                    save: save.clone(),
                    registry: pack.assets.clone(),
                    biomes: biomes.clone(),
                }),
                pack.chunk_context_with(biomes.clone()),
                res.threads(),
            )
        } else {
            let inner = FlatTerrain::from_decoded(&flat, pack.assets.registry, &biomes, shape)
                .ok_or_else(|| format!("{}: flat layers use an unregistered block", dim.id()))?;
            ChunkStreamer::new(
                Arc::new(Persistent {
                    inner: Arc::new(inner),
                    save: save.clone(),
                    registry: pack.assets.clone(),
                    biomes: biomes.clone(),
                }),
                pack.chunk_context_with(biomes.clone()),
                res.threads(),
            )
        }
    } else if let Some(inner) = debug {
        generator_type = Identifier::parse("minecraft:debug").unwrap();
        ChunkStreamer::new(
            Arc::new(Persistent {
                inner: Arc::new(inner),
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.chunk_context_with(biomes.clone()),
            res.threads(),
        )
    } else if options.test_terrain && dim == Dim::Overworld {
        // The test terrain stands in for the noise generator its stem names.
        generator_type = Identifier::parse("minecraft:noise").unwrap();
        let inner = Arc::new(TestTerrain::new(pack.assets.registry, &biomes)?);
        ChunkStreamer::new(
            Arc::new(Persistent {
                inner,
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.chunk_context_with(biomes.clone()),
            res.threads(),
        )
    } else if let Some((level, id)) = mod_terrain(
        overrides,
        terrain_resources,
        sunrise_world::worldgen::mod_generators::mod_chunk_generators(),
        dim,
        shape,
        seed,
        automatic_structures,
        match runtime {
            Some(super::runtime_levels::Generate::Stem(stem)) => Some(stem),
            _ => None,
        },
        saved_stem,
    )
    .transpose()?
    {
        generator_type = id;
        match level {
            ModLevel::Terrain(inner) => ChunkStreamer::new(
                Arc::new(Persistent {
                    inner: Arc::new(inner),
                    save: save.clone(),
                    registry: pack.assets.clone(),
                    biomes: biomes.clone(),
                }),
                pack.chunk_context_with(biomes.clone()),
                res.threads(),
            ),
            ModLevel::Staged(mut inner) => {
                inner.add_mod_signs(res.facts());
                register_sea_level(inner.sea_level());
                let inner = Arc::new(inner);
                inner.load_structure_mobs_behind();
                keep_structure_references(&inner, world_root, dim, &save);
                locator = Some(inner.clone());
                ChunkStreamer::new(
                    Arc::new(Persistent {
                        inner,
                        save: save.clone(),
                        registry: pack.assets.clone(),
                        biomes: biomes.clone(),
                    }),
                    pack.chunk_context_with(biomes.clone()),
                    res.threads(),
                )
            }
        }
    } else {
        if runtime.is_some() {
            return Err(format!("{}: no generator builds this runtime level", dim.id()).into());
        }
        generator_type = Identifier::parse("minecraft:noise").unwrap();
        // WorldOptions.generateStructures, from the world's settings.
        let structures = automatic_structures;
        let preset = WorldGen::noise_preset(generator);
        let saved_stem = saved_stem.map(sunrise_game_data::scoreboard::tag_to_json);
        let regions = sunrise_sim::save::biome_regions::layout(world_root, dim.id())
            .map_err(|e| format!("Failed to read the world's biome regions: {e}"))?;
        let mut inner = VanillaTerrain::for_world_with_settings(terrain_resources, overrides, seed, preset, dim.id(), structures, saved_stem.as_ref(), regions).map_err(|e| {
                if overrides.is_empty() {
                    format!("{e}\nPrepare the worldgen bundle with the launcher, or: sunrise-prepare --prepare")
                } else {
                    format!("{e}\nThe world's data packs change its world generation: one of them may not be made for this version")
                }
            })?;
        inner.add_mod_signs(res.facts());
        register_sea_level(inner.sea_level());
        let inner = Arc::new(inner);
        // The first mob a fresh chunk generates needs the mob data: read it now,
        // off the server's tick (#13857). The End's chunks generate none.
        if dim != Dim::End {
            inner.load_structure_mobs_behind();
        }
        if dim == Dim::Overworld && structures {
            inner.search_rings_behind();
        }
        // Chunks an older version saved: the new chunks around them blend in.
        if dim == Dim::Overworld {
            let old = Arc::new(sunrise_sim::old_world::SavedOldWorld::new(
                save.clone(),
                pack.assets.clone(),
                res.biomes().clone(),
                inner.generator_data().clone(),
            ));
            // Saves of this folder take the old chunks' calculated blending data when
            // they are requested; a weak link, as the old world holds the folder.
            let weak = Arc::downgrade(&old);
            save.set_blending_lookup(Box::new(move |x, z| {
                weak.upgrade().and_then(|old| old.calculated(x, z))
            }));
            inner.set_old_world(old);
        }
        keep_structure_references(&inner, world_root, dim, &save);
        locator = Some(inner.clone());
        ChunkStreamer::new(
            Arc::new(Persistent {
                inner,
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.chunk_context_with(biomes.clone()),
            res.threads(),
        )
    };
    streaming.radius = settings.render_distance;
    streaming.biome_zoom = BiomeZoom::new(obfuscate_seed(seed));
    let facts = sunrise_world::dimension_type::DimensionFacts::of_builtin_level(
        sunrise_core::LevelKey::builtin_or_overworld(dim.id()),
    );
    streaming.sky_light = facts.height.has_skylight;
    Ok((streaming, save, locator, generator_type))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_gameplay_common::game::server_side::world_loading::tests::PACK_BIOME;

    #[test]
    fn report_root_follows_the_server_role() {
        use execution_role::ExecutionRole::{DedicatedServer, IntegratedServer};
        use std::path::Path;

        let cwd = Path::new("server");
        let game = Path::new("game");
        assert_eq!(chunk_report_root(DedicatedServer, Some(game), cwd), cwd);
        assert_eq!(chunk_report_root(IntegratedServer, Some(game), cwd), game);
        assert_eq!(chunk_report_root(IntegratedServer, None, cwd), cwd);
    }

    #[test]
    fn unreadable_level_backup_refuses_before_biome_adoption() {
        use sunrise_core::nbt::Tag;
        use sunrise_sim::save;
        use sunrise_world::worldgen::biome_source::regions::SelectorParams;

        let game = tempfile::tempdir().unwrap();
        let world = game.path().join("world");
        let record = world.join("data/sunrise/biome_regions.dat");
        let backup = world.join("level.dat_old");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        std::fs::write(&backup, b"unreadable backup").unwrap();
        std::fs::write(
            game.path().join("server.toml"),
            "[biome_regions]\nadopt_installed = true\n",
        )
        .unwrap();
        let selector = SelectorParams::current();
        let layout = Tag::compound([
            (
                "selector",
                Tag::compound([
                    ("version", Tag::Int(selector.version as i32)),
                    ("cell", Tag::Int(selector.cell as i32)),
                    ("salt", Tag::String(selector.salt.as_str().into())),
                    ("vanilla_weight", Tag::Int(selector.vanilla_weight as i32)),
                ]),
            ),
            ("regions", Tag::List(Vec::new())),
        ]);
        save::write_dat(
            &record,
            Tag::compound([
                ("version", Tag::Int(1)),
                (
                    "dimensions",
                    Tag::compound([("minecraft:overworld", layout)]),
                ),
            ]),
        )
        .unwrap();
        assert!(save::biome_regions::adoptable(&world));
        let before = std::fs::read(&record).unwrap();
        let role = execution_role::ExecutionRole::DedicatedServer;
        assert!(prepare_level_data(role, &world, "world", game.path()).is_err());
        assert!(!world.join("level.dat").exists());
        assert_eq!(std::fs::read(&record).unwrap(), before);
        assert_eq!(std::fs::read(&backup).unwrap(), b"unreadable backup");

        // The explicit opt-in does change this layout when recovery can proceed.
        save::write_dat(
            &backup,
            Tag::compound([("Data", Tag::Compound(Default::default()))]),
        )
        .unwrap();
        prepare_level_data(role, &world, "world", game.path()).unwrap();
        assert_ne!(std::fs::read(&record).unwrap(), before);
    }

    #[test]
    #[ignore = "daily"]
    fn custom_flat_stems_validate_and_generate_pack_decoration() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use std::fs;
        use sunrise_level::chunk::LevelShape;

        const CASE: &str = "SUNRISE_CUSTOM_FLAT_STEM_CASE";
        if run_fresh_cases(
            "game::server_side::world_loading::tests::custom_flat_stems_validate_and_generate_pack_decoration",
            CASE,
            &["invalid", "decorated"],
        ) {
            return;
        }
        let invalid = std::env::var(CASE).unwrap() == "invalid";
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pack = root.join("datapacks/flat");
        let write = |path: &str, text: &str| {
            let path = pack.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "pack.mcmeta",
            r#"{"pack":{"description":"Flat stem fixture","min_format":[121,0],"max_format":[121,0]}}"#,
        );
        fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/flat"],"disabled":[]}"#,
        )
        .unwrap();
        // FlatLevelGeneratorSettings.validateHeight rejects one above the limit.
        let height = if invalid { 4065 } else { 1 };
        write(
            "data/test/dimension/flat.json",
            &format!(
                r#"{{"type":"minecraft:the_nether","generator":{{"type":"minecraft:flat","settings":{{
            "biome":"test:glade","features":true,"lakes":false,"structure_overrides":[],
            "layers":[{{"height":{height},"block":"minecraft:bedrock"}}]}}}}}}"#
            ),
        );
        write(
            "data/test/worldgen/biome/glade.json",
            r#"{"temperature":0.8,"downfall":0.4,"has_precipitation":true,"effects":{},"carvers":[],"features":[["test:decoration"]]}"#,
        );
        write(
            "data/test/worldgen/feature/decoration.json",
            r#"{"type":"minecraft:fill_layer","height":4,"state":{"id":"minecraft:gold_block"}}"#,
        );
        write(
            "data/test/worldgen/placed_feature/decoration.json",
            r#"{"feature":"test:decoration","placement":[]}"#,
        );

        with_game(|game| {
            let shape = LevelShape::NETHER;
            let base_conditions = sunrise_world::conditions::LoadConditions::vanilla();
            let empty =
                sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
            let stack = crate::game::functions::pack_stack(
                root,
                &game.options.resources,
                &empty,
                &empty,
                false,
                &game.level.enabled_features,
                &game.pack.additional_data,
            );
            let data = stack.data_packs(Vec::new());
            let enabled: Vec<_> = stack.enabled_packs().collect();
            let conditions =
                sunrise_gameplay_common::game::load_conditions::worldgen_load_conditions(
                    &base_conditions,
                    &game.options.resources,
                    &enabled,
                );
            let (overrides, _) = worldgen_overrides_of(&data, &conditions).unwrap();
            let packed: Vec<(String, serde_json::Value)> = overrides
                .entries("biomes")
                .filter_map(|(id, json)| Some((id.to_owned(), serde_json::from_str(json).ok()?)))
                .collect();
            let biomes = if packed.is_empty() {
                game.res.biomes().clone()
            } else {
                Arc::new(
                    game.res
                        .biomes()
                        .with_packs(packed.iter().map(|(i, v)| (i.as_str(), v)))
                        .unwrap(),
                )
            };
            let built = make_streaming(
                &game.res,
                &game.pack,
                &game.options,
                &game.server.settings,
                root,
                Dim::Extra(mod_api::__private::dimension_key("test:flat")),
                shape,
                29,
                &game.level,
                biomes.clone(),
                &overrides,
                None,
                game.server.save.0.chunk_reports().clone(),
                game.server.save.0.save_failure_reporter().clone(),
            );
            if invalid {
                let Err(error) = built else {
                    panic!("a height outside the flat codec must reject this stem");
                };
                assert!(
                    error
                        .to_string()
                        .starts_with("test:flat: cannot decode flat dimension:")
                );
                return;
            }
            let (mut streaming, _, locator, generator_type) = built.unwrap();
            assert_eq!(generator_type.as_str(), "minecraft:flat");
            assert_eq!(
                locator.unwrap().noise_biomes(&[[0, 0, 0]]),
                [Some("test:glade".into())]
            );
            let mut world = World::new(game.res.facts().air, 29);
            world.shape = shape;
            assert!(streaming.set_forced_loaded(&mut world, 0, 0, true));
            let chunk = &world.chunks[&(0, 0)];
            let gold = game
                .pack
                .assets
                .registry
                .default_state("minecraft:gold_block")
                .unwrap();
            let bedrock = game
                .pack
                .assets
                .registry
                .default_state("minecraft:bedrock")
                .unwrap();
            assert_eq!(chunk.get(0, 0, 0), bedrock.0);
            assert_eq!(chunk.get(0, 4, 0), gold.0);
            assert_eq!(biomes.saved_name(chunk.biome(0, 4, 0)), "test:glade");
        });
    }

    /// Report the generator selected for saved, packed, and runtime stems.
    #[test]
    #[ignore = "daily"]
    fn a_level_reports_the_generator_it_was_built_with() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use std::fs;
        use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
        use sunrise_level::chunk::LevelShape;
        use sunrise_sim::save::{WorldGen, WorldGenerator, WorldPreset};

        const CASE: &str = "SUNRISE_LEVEL_GENERATOR_TYPE_CASE";
        if run_fresh_cases(
            "game::server_side::world_loading::tests::a_level_reports_the_generator_it_was_built_with",
            CASE,
            &["debug", "unregistered", "runtime"],
        ) {
            return;
        }
        let case = std::env::var(CASE).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        if case == "runtime" {
            use mod_api::{RuntimeGenerator, RuntimeLevel, RuntimeLevelError};
            use sunrise_world::worldgen::mod_generators::ChunkAccess;

            struct Floor;
            impl ModChunkGenerator for Floor {
                fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                    let pos = chunk.pos();
                    let gold = chunk.state("minecraft:gold_block");
                    assert!(chunk.set_block([pos.x * 16, chunk.min_y(), pos.z * 16], gold));
                }
            }

            let id = Identifier::parse("minecraft:fixture_floor").unwrap();
            let generators = ModChunkGenerators::from([(
                id.clone(),
                Arc::new(Floor) as Arc<dyn ModChunkGenerator>,
            )]);
            with_game(|game| {
                game.server.world_root = root.to_owned();
                let open = |game: &mut crate::game::ServerGame<'_>, name: &str| {
                    game.runtime_level_queue_open(
                        "test:runtime_floor",
                        RuntimeLevel::new(
                            "minecraft:overworld",
                            RuntimeGenerator::Mod(name.into()),
                            29,
                        ),
                        &generators,
                    )
                };
                for name in [
                    "fixture_floor",
                    ":fixture_floor",
                    "minecraft:BAD",
                    "test:unregistered",
                ] {
                    assert_eq!(
                        open(game, name),
                        Err(RuntimeLevelError::UnknownGenerator),
                        "{name}"
                    );
                }
                let level = open(game, id.as_str()).unwrap();
                game.apply_runtime_levels();
                assert_eq!(level.generator_type(game), Some(id));
                level.load_chunk(game, [0, 0]).unwrap();
                let world = game.native_level_world(level).unwrap();
                let gold = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:gold_block")
                    .unwrap();
                assert_eq!(world.block(0, world.shape.min_y, 0), gold);
            });
            return;
        }
        let debug = case == "debug";
        let pack = root.join("datapacks/stems");
        let write = |path: &str, text: &str| {
            let path = pack.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "pack.mcmeta",
            r#"{"pack":{"description":"Generator stem fixture","min_format":[121,0],"max_format":[121,0]}}"#,
        );
        fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/stems"],"disabled":[]}"#,
        )
        .unwrap();
        write(
            "data/minecraft/dimension/overworld.json",
            r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:noise","settings":"minecraft:overworld",
            "biome_source":{"type":"minecraft:fixed","biome":"minecraft:plains"}}}"#,
        );
        write(
            "data/test/dimension/odd.json",
            r#"{"type":"minecraft:overworld","generator":{"type":"test:missing",
            "biome_source":{"type":"minecraft:fixed","biome":"minecraft:plains"}}}"#,
        );

        with_game(|game| {
            let base_conditions = sunrise_world::conditions::LoadConditions::vanilla();
            let empty =
                sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
            let stack = crate::game::functions::pack_stack(
                root,
                &game.options.resources,
                &empty,
                &empty,
                false,
                &game.level.enabled_features,
                &game.pack.additional_data,
            );
            let data = stack.data_packs(Vec::new());
            let enabled: Vec<_> = stack.enabled_packs().collect();
            let conditions =
                sunrise_gameplay_common::game::load_conditions::worldgen_load_conditions(
                    &base_conditions,
                    &game.options.resources,
                    &enabled,
                );
            let (overrides, _) = worldgen_overrides_of(&data, &conditions).unwrap();
            let settings = tempfile::tempdir().unwrap();
            let preset = if debug {
                WorldPreset::Debug
            } else {
                WorldPreset::Normal
            };
            let options = WorldGen {
                preset,
                ..WorldGen::default()
            };
            sunrise_sim::save::write_world_gen_settings(settings.path(), 1, options).unwrap();
            game.common.level.generator = WorldGenerator::read_new(settings.path());
            assert!(game.level.generator.is_some());
            let dim = if debug {
                Dim::Overworld
            } else {
                Dim::Extra(mod_api::__private::dimension_key("test:odd"))
            };
            let built = make_streaming(
                &game.res,
                &game.pack,
                &game.options,
                &game.server.settings,
                root,
                dim,
                LevelShape::OVERWORLD,
                29,
                &game.level,
                game.res.biomes().clone(),
                &overrides,
                None,
                game.server.save.0.chunk_reports().clone(),
                game.server.save.0.save_failure_reporter().clone(),
            );
            if debug {
                let (_, _, _, generator_type) = built.unwrap();
                assert_eq!(generator_type.as_str(), "minecraft:debug");
            } else {
                let Err(error) = built else {
                    panic!("no registered generator builds test:missing");
                };
                assert!(
                    error.to_string().contains("is not a noise generator"),
                    "{error}"
                );
            }
        });
    }

    /// A new pack biome receives an ID after the built-ins and generates in the overworld.
    #[test]
    #[ignore = "daily"]
    fn a_pack_biome_no_built_in_has_generates_in_the_overworld() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use std::fs;
        use sunrise_level::chunk::LevelShape;

        const CASE: &str = "SUNRISE_NEW_BIOME_WORLD_CASE";
        if run_fresh_cases(
            "game::server_side::world_loading::tests::a_pack_biome_no_built_in_has_generates_in_the_overworld",
            CASE,
            &["overworld"],
        ) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pack = root.join("datapacks/biome");
        let write = |path: &str, text: &str| {
            let path = pack.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "pack.mcmeta",
            r#"{"pack":{"description":"New biome fixture","min_format":[121,0],"max_format":[121,0]}}"#,
        );
        fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/biome"],"disabled":[]}"#,
        )
        .unwrap();
        write(
            "data/minecraft/dimension/overworld.json",
            r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:noise","settings":"minecraft:overworld",
            "biome_source":{"type":"minecraft:fixed","biome":"test:new_biome"}}}"#,
        );
        with_game(|game| {
            // A copy of the game's plains with its own water colour.
            let mut file = game
                .res
                .biomes()
                .builtin_file("minecraft:plains")
                .unwrap()
                .clone();
            file["effects"]["water_color"] = serde_json::json!("#123456");
            write("data/test/worldgen/biome/new_biome.json", &file.to_string());

            let builtin = game.res.biomes();
            assert!(!builtin.contains("test:new_biome"));
            let empty =
                sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
            let stack = crate::game::functions::pack_stack(
                root,
                &game.options.resources,
                &empty,
                &empty,
                false,
                &game.level.enabled_features,
                &game.pack.additional_data,
            );
            let data = stack.data_packs(Vec::new());
            let (overrides, _) =
                worldgen_overrides_of(&data, &sunrise_world::conditions::LoadConditions::vanilla())
                    .unwrap();
            let packed: Vec<(String, serde_json::Value)> = overrides
                .entries("biomes")
                .filter_map(|(id, json)| Some((id.to_owned(), serde_json::from_str(json).ok()?)))
                .collect();
            let biomes = Arc::new(
                builtin
                    .with_packs(packed.iter().map(|(i, v)| (i.as_str(), v)))
                    .unwrap(),
            );
            let id = biomes
                .try_id("test:new_biome")
                .expect("the packed table knows the biome");
            assert!(usize::from(id) >= builtin.registry_order().len());
            assert_eq!(biomes.name(id), "test:new_biome");

            let shape = LevelShape::OVERWORLD;
            let (mut streaming, _, locator, _) = make_streaming(
                &game.res,
                &game.pack,
                &game.options,
                &game.server.settings,
                root,
                Dim::Overworld,
                shape,
                7,
                &game.level,
                biomes.clone(),
                &overrides,
                None,
                game.server.save.0.chunk_reports().clone(),
                game.server.save.0.save_failure_reporter().clone(),
            )
            .unwrap();
            if let Some(locator) = locator {
                assert_eq!(
                    locator.noise_biomes(&[[0, 0, 0]]),
                    [Some("test:new_biome".into())]
                );
            }
            let mut world = World::new(game.res.facts().air, 7);
            world.shape = shape;
            assert!(streaming.set_forced_loaded(&mut world, 0, 0, true));
            let chunk = &world.chunks[&(0, 0)];
            for y in [shape.min_y, 0, 64, 200] {
                assert_eq!(
                    biomes.saved_name(chunk.biome(0, y, 0)),
                    "test:new_biome",
                    "y {y}"
                );
            }
        });
    }

    fn overworld_biomes_with_pack(files: &[(&str, &str)]) -> Vec<String> {
        use crate::game::native_entity_refs::tests::with_game;
        use std::fs;
        use sunrise_level::chunk::LevelShape;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pack = root.join("datapacks/preset");
        let write = |path: &str, text: &str| {
            let path = pack.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "pack.mcmeta",
            r#"{"pack":{"description":"Preset fixture","min_format":[121,0],"max_format":[121,0]}}"#,
        );
        fs::write(
            root.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/preset"],"disabled":[]}"#,
        )
        .unwrap();
        for (path, text) in files {
            write(path, text);
        }
        let mut names = Vec::new();
        with_game(|game| {
            let empty =
                sunrise_assets::resource_source::ResourceSource::indexed(Default::default());
            let stack = crate::game::functions::pack_stack(
                root,
                &game.options.resources,
                &empty,
                &empty,
                false,
                &game.level.enabled_features,
                &game.pack.additional_data,
            );
            let data = stack.data_packs(Vec::new());
            let (overrides, _) =
                worldgen_overrides_of(&data, &sunrise_world::conditions::LoadConditions::vanilla())
                    .unwrap();
            let packed: Vec<(String, serde_json::Value)> = overrides
                .entries("biomes")
                .filter_map(|(id, json)| Some((id.to_owned(), serde_json::from_str(json).ok()?)))
                .collect();
            let biomes = Arc::new(
                game.res
                    .biomes()
                    .with_packs(packed.iter().map(|(i, v)| (i.as_str(), v)))
                    .unwrap(),
            );
            let shape = LevelShape::OVERWORLD;
            let (mut streaming, _, _, _) = make_streaming(
                &game.res,
                &game.pack,
                &game.options,
                &game.server.settings,
                root,
                Dim::Overworld,
                shape,
                7,
                &game.level,
                biomes.clone(),
                &overrides,
                None,
                game.server.save.0.chunk_reports().clone(),
                game.server.save.0.save_failure_reporter().clone(),
            )
            .unwrap();
            let mut world = World::new(game.res.facts().air, 7);
            world.shape = shape;
            assert!(streaming.set_forced_loaded(&mut world, 0, 0, true));
            let chunk = &world.chunks[&(0, 0)];
            names = [0, 5, 10, 15]
                .into_iter()
                .map(|x| biomes.saved_name(chunk.biome(x, 64, x)).into_owned())
                .collect();
        });
        names
    }

    #[test]
    #[ignore = "daily"]
    fn a_pack_world_preset_replaces_the_default_presets_overworld() {
        const CASE: &str = "SUNRISE_PRESET_WORLD_CASE";
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::server_side::world_loading::tests::a_pack_world_preset_replaces_the_default_presets_overworld",
            CASE,
            &["overworld"],
        ) {
            return;
        }
        let names = overworld_biomes_with_pack(&[(
            "data/minecraft/worldgen/world_preset/normal.json",
            r#"{"dimensions":{"minecraft:overworld":{"type":"minecraft:overworld","generator":{
            "type":"minecraft:noise","settings":"minecraft:overworld",
            "biome_source":{"type":"minecraft:fixed","biome":"minecraft:cherry_grove"}}}}}"#,
        )]);
        assert_eq!(names, ["minecraft:cherry_grove"; 4]);
    }

    /// The pack's parameter list points to the Nether preset, even when the overworld uses it.
    #[test]
    #[ignore = "daily"]
    fn a_level_stem_naming_a_pack_parameter_list_generates_per_that_list() {
        const CASE: &str = "SUNRISE_PARAMETER_LIST_WORLD_CASE";
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "game::server_side::world_loading::tests::a_level_stem_naming_a_pack_parameter_list_generates_per_that_list",
            CASE,
            &["overworld"],
        ) {
            return;
        }
        let names = overworld_biomes_with_pack(&[
            (
                "data/test/worldgen/multi_noise_biome_source_parameter_list/mine.json",
                r#"{"preset":"minecraft:nether"}"#,
            ),
            (
                "data/minecraft/dimension/overworld.json",
                r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:noise",
                "settings":"minecraft:overworld",
                "biome_source":{"type":"minecraft:multi_noise","preset":"test:mine"}}}"#,
            ),
        ]);
        let nether = [
            "minecraft:nether_wastes",
            "minecraft:soul_sand_valley",
            "minecraft:crimson_forest",
            "minecraft:warped_forest",
            "minecraft:basalt_deltas",
        ];
        assert!(
            names.iter().all(|name| nether.contains(&name.as_str())),
            "{names:?}"
        );
    }

    /// A world whose level.dat does not read but whose level.dat_old does: only the
    /// dedicated server restores it unasked. Every role here carries the launch configuration
    /// (`options.dedicated`) a dedicated server and the integrated server thread both have, so
    /// it cannot be what decides.
    #[test]
    fn only_the_dedicated_role_takes_level_dat_old_unasked() {
        use execution_role::ExecutionRole::{Client, DedicatedServer, IntegratedServer};
        for (role, restored) in [
            (DedicatedServer, true),
            (IntegratedServer, false),
            (Client, false),
        ] {
            let dir = std::env::temp_dir().join(format!(
                "sunrise-level-dat-old-{role:?}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            Level {
                name: "Recovered".into(),
                ..Level::default()
            }
            .write(&dir, &Default::default())
            .unwrap();
            std::fs::copy(dir.join("level.dat"), dir.join("level.dat_old")).unwrap();
            std::fs::write(dir.join("level.dat"), b"not a level").unwrap();
            let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
            options.dedicated = Some(Vec::new());
            options.execution_role = role;

            fall_back_to_level_dat_old_without_asking(options.execution_role, &dir, "world");

            assert_eq!(
                sunrise_sim::save::level_dat_issue(&dir, false).is_none(),
                restored,
                "{role:?}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_levels_sea_level_comes_from_its_mod_or_its_noise_settings() {
        use sunrise_level::datapack::worldgen_overrides_over;
        use sunrise_world::worldgen::overrides::ModDimension;
        let level = |id: &str, stem: &str, sea_level| ModDimension {
            id: id.into(),
            stem: stem.into(),
            dimension_type: "{}".into(),
            sea_level,
        };
        let mods = [
            level(
                "test:own",
                r#"{"type":"test:own","generator":{"type":"test:floor"}}"#,
                Some(10),
            ),
            level(
                "test:inline",
                r#"{"type":"test:inline","generator":{"type":"minecraft:noise","settings":{"sea_level":20}}}"#,
                None,
            ),
            level(
                "test:plain",
                r#"{"type":"test:plain","generator":{"type":"test:floor"}}"#,
                None,
            ),
        ];
        let (overrides, _) = worldgen_overrides_over(
            &mods,
            &Default::default(),
            &sunrise_world::conditions::LoadConditions::vanilla(),
        );
        set_level_sea_levels(&overrides, &mods, std::path::Path::new("."));
        let sea = |id: &str| {
            sunrise_world::level::sea_level_of(sunrise_core::LevelKey::parse(id).unwrap())
        };
        assert_eq!(sea("test:own"), 10);
        assert_eq!(sea("test:inline"), 20);
        assert_eq!(sea("test:plain"), 63);
        assert_eq!(sea("minecraft:the_nether"), 32);
        sunrise_world::level::set_sea_levels(Vec::new());
    }

    #[test]
    #[ignore = "daily"]
    fn creation_rule_selection_reaches_the_saved_world() {
        use super::super::native_entity_refs::tests::with_game;
        use sunrise_game_data::gamerules::{GameRules, Rule, RuleRegistry, Value};
        let rule = Rule {
            name: "example:enabled".into(),
            category: "example:machines".into(),
            default: Value::Bool(true),
            min: 0,
            max: 0,
        };
        let registry =
            Arc::new(RuleRegistry::with_mod_rules(&["example:machines".into()], &[rule]).unwrap());
        let mut create = sunrise_sim::world_options::WorldOptions {
            rules: GameRules::with_registry(registry.clone(), &[]),
            ..Default::default()
        };
        assert!(create.rules.set("example:enabled", Value::Bool(false)));
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("creation-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        with_game(|game| {
            let request = WorldRequest {
                owner: None,
                chunk_reports: sunrise_sim::chunk_io::PROCESS_REPORTS.clone(),
                stem_order: None,
                save_root: dir.clone(),
                folder: "test".into(),
                name: Some("test".into()),
                seed: Some(1),
                creative: true,
                create: Some(create.clone()),
                rule_registry: registry.clone(),
            };
            assert!(Arc::ptr_eq(
                request.create.as_ref().unwrap().rules.registry(),
                &request.rule_registry
            ));
            game.load_world(request).unwrap();
            assert_eq!(
                game.ui.rules.get("example:enabled"),
                Some(Value::Bool(false))
            );
            assert!(Arc::ptr_eq(&game.level.rule_registry, &registry));
            game.save_game_with(true).unwrap();
        });
        assert_eq!(
            GameRules::load_with_registry(&dir, &[], registry).get("example:enabled"),
            Some(Value::Bool(false))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn biome_registry_fixture() -> (
        tempfile::TempDir,
        Arc<Biomes>,
        &'static sunrise_world::registry::BlockRegistry,
        crate::game::functions::PackStack,
    ) {
        use sunrise_assets::resource_source::ResourceRoot;
        use sunrise_level::datapack::DataPack;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("data/minecraft/worldgen/biome");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(root.path().join("worldgen")).unwrap();
        for name in ["fixture", "plains"] {
            std::fs::write(dir.join(format!("{name}.json")), PACK_BIOME).unwrap();
        }
        let resources = ResourceRoot::directory(root.path().to_owned());
        let base = Arc::new(Biomes::load_for_server(&resources).unwrap());
        let blocks: &'static _ = Box::leak(Box::new(sunrise_world::registry::BlockRegistry::from_facts_json(
            br#"{"defaults":{"minecraft:air":0,"minecraft:cave_air":1,"minecraft:void_air":2},"states":[["a",true,false,"","minecraft:air"],["c",true,false,"","minecraft:cave_air"],["v",true,false,"","minecraft:void_air"]]}"#.as_slice(),
        ).unwrap()));
        let stack = crate::game::functions::PackStack {
            builtin: None,
            content: DataPack::from_source("sunrise-content".into(), resources.data().clone()),
            packs: Vec::new(),
            enabled: Vec::new(),
            features: Vec::new(),
        };
        (root, base, blocks, stack)
    }

    #[test]
    fn biome_loader_checks_winning_conditions_and_decodes_passing_payloads() {
        use serde_json::{Value, json};
        use sunrise_world::{conditions::LoadConditions, loot::PackOverlay};
        let (root, base, blocks, stack) = biome_registry_fixture();
        std::fs::write(
            root.path().join("worldgen/placed_features.json"),
            json!({"demo:malformed": "{}", "demo:dangling": r#"{"feature":"demo:missing","placement":[]}"#}).to_string(),
        ).unwrap();
        let tags = root.path().join("data/demo/tags/worldgen/placed_feature");
        std::fs::create_dir_all(&tags).unwrap();
        std::fs::write(
            tags.join("malformed.json"),
            r#"{"values":["demo:malformed"]}"#,
        )
        .unwrap();
        let unknown = |condition| json!({"neoforge:conditions":[{"type":condition}]});
        let load = |winner: Value, conditions: &LoadConditions| {
            let files = [(
                "worldgen/biome/minecraft:fixture".into(),
                vec![
                    unknown("absent:loser").to_string().into_bytes(),
                    winner.to_string().into_bytes(),
                ],
            )];
            WorldBiomeRegistry::load(
                &base,
                root.path(),
                blocks,
                &stack,
                &stack.data_packs(Vec::new()),
                &PackOverlay {
                    files: &files,
                    conditions,
                    ..PackOverlay::default()
                },
                &Default::default(),
            )
        };

        let conditions = LoadConditions::new([]);
        let refused = load(unknown("absent:wire_biome"), &conditions).unwrap();
        assert!(!refused.table.contains("minecraft:fixture"));
        assert_eq!(
            conditions.check_errors().unwrap_err(),
            "data/minecraft/worldgen/biome/fixture.json: unknown load conditions [absent:wire_biome]"
        );

        for (field, invalid, diagnostic) in [
            (
                "effects",
                Value::Null,
                "invalid world generation field: effects",
            ),
            (
                "attributes",
                json!({"minecraft:visual/ambient_particles":1}),
                "minecraft:visual/ambient_particles",
            ),
            ("carvers", json!({}), "carvers:"),
            (
                "carvers",
                json!({"type":"neoforge:and","values":[[{"type":"minecraft:missing"}],[]]}),
                "world carver type",
            ),
            (
                "features",
                json!([["demo:unknown"]]),
                "unknown placed feature demo:unknown",
            ),
            (
                "features",
                json!([["demo:malformed"]]),
                "placed feature demo:malformed",
            ),
            (
                "features",
                json!(["#demo:malformed"]),
                "placed feature demo:malformed",
            ),
            (
                "features",
                json!([["demo:dangling"]]),
                "unknown feature reference demo:missing",
            ),
            (
                "features",
                json!([{"type":"neoforge:any"}]),
                "placed feature demo:dangling",
            ),
            (
                "features",
                json!([{"type":"neoforge:and","values":[["demo:malformed"],[]]}]),
                "placed feature demo:malformed",
            ),
        ] {
            let mut payload: Value = serde_json::from_str(PACK_BIOME).unwrap();
            payload[field] = invalid;
            let conditions = LoadConditions::new([]);
            let error = load(
                json!({"neoforge:conditions":[{"type":"neoforge:always"}],"neoforge:value":payload}),
                &conditions,
            )
            .err()
            .expect("a passing condition does not excuse an invalid biome");
            let error = error.to_string();
            assert!(error.starts_with("biome minecraft:fixture:"), "{error}");
            assert!(error.contains(diagnostic), "{error}");
            assert_eq!(conditions.check_errors(), Ok(()));
        }
    }

    #[test]
    fn wrapped_biome_resolves_custom_holder_sets_and_inline_registry_predicates() {
        use serde_json::{Value, json};
        use sunrise_world::{conditions::LoadConditions, loot::PackOverlay};

        let (root, base, blocks, stack) = biome_registry_fixture();
        std::fs::write(
            root.path().join("worldgen/fluid_tags.json"),
            json!({"demo:wet": r#"{"values":["minecraft:water"]}"#}).to_string(),
        )
        .unwrap();
        let feature = json!({"type":"minecraft:no_op", "config": {}});
        std::fs::write(
            root.path().join("worldgen/placed_features.json"),
            json!({"demo:no_op": json!({"feature":feature.clone(), "placement":[]}).to_string()})
                .to_string(),
        )
        .unwrap();
        let mut payload: Value = serde_json::from_str(PACK_BIOME).unwrap();
        payload["features"] = json!([
            {"type":"neoforge:any"},
            [{"feature":feature,"placement":[
                {"type":"minecraft:block_predicate_filter", "predicate": {
                    "type":"minecraft:matching_biomes", "biomes":["minecraft:plains", "demo:incoming"]
                }},
                {"type":"minecraft:block_predicate_filter", "predicate": {
                    "type":"minecraft:matching_fluids", "fluids":"#demo:wet"
                }}
            ]}]
        ]);
        payload["carvers"] = json!({"type":"neoforge:any"});
        let files = [(
            "worldgen/biome/demo:incoming".into(),
            vec![
                json!({
                    "neoforge:conditions":[{"type":"neoforge:always"}], "neoforge:value":payload
                })
                .to_string()
                .into_bytes(),
            ],
        )];
        let conditions = LoadConditions::new([]);
        let admitted = WorldBiomeRegistry::load(
            &base,
            root.path(),
            blocks,
            &stack,
            &stack.data_packs(Vec::new()),
            &PackOverlay {
                files: &files,
                conditions: &conditions,
                ..PackOverlay::default()
            },
            &Default::default(),
        )
        .unwrap();
        assert!(admitted.table.contains("demo:incoming"));
        assert_eq!(conditions.check_errors(), Ok(()));
    }

    #[test]
    fn the_saved_biome_tests_pack_biome_decodes() {
        let climate =
            sunrise_world::worldgen::biome_temperature::BiomeTemperatureSettings::from_json(
                PACK_BIOME,
            )
            .unwrap();
        assert!(climate.has_precipitation);
        assert_eq!(climate.base_temperature, 0.8);
        assert_eq!(climate.downfall, 0.4);
        let value: serde_json::Value = serde_json::from_str(PACK_BIOME).unwrap();
        assert_eq!(value["carvers"], serde_json::json!([]));
    }

    #[test]
    #[ignore = "daily"]
    fn a_loaded_missing_biome_survives_a_game_save_with_an_enabled_biome_pack() {
        use super::super::native_entity_refs::tests::with_game;
        let root = std::env::temp_dir().join(format!(
            "saved-biome-pack-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let pack = root.join("datapacks/biomes");
        let entries = pack.join("data/test/worldgen/biome");
        std::fs::create_dir_all(&entries).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Biome fixture","min_format":[121,0],"max_format":[121,0]}}"#,
        )
        .unwrap();
        std::fs::write(entries.join("present.json"), PACK_BIOME).unwrap();
        Level {
            data_packs: Some((vec!["vanilla".into(), "file/biomes".into()], vec![])),
            ..Level::default()
        }
        .write(&root, &Default::default())
        .unwrap();
        with_game(|game| {
            let before = Arc::new(game.res.biomes().with_packs([]).unwrap());
            let mut chunk = sunrise_level::chunk::Chunk::empty(8, 8);
            chunk.set_biome_cell(1, 20, 2, before.saved_id("gone_mod:highland"));
            let save = SaveDir::new(
                root.clone(),
                sunrise_level::chunk::LevelShape::OVERWORLD,
                Some(Default::default()),
            );
            save.save(&[Arc::new(chunk)], game.pack.assets.registry, &before)
                .unwrap();
            drop(save);
            let mut request = WorldRequest::new(
                "test",
                Some("test"),
                Some(1),
                true,
                None,
                game.level.rule_registry.clone(),
            )
            .unwrap();
            request.save_root = root.clone();
            game.load_world(request).unwrap();
            assert!(world_biomes!(game).contains("test:present"));
            game.server
                .streaming
                .set_forced_loaded(&mut game.common.world, 8, 8, true);
            let chunk = &game.world.chunks[&(8, 8)];
            let id = chunk.biome(4, 16, 8);
            assert_eq!(world_biomes!(game).saved_name(id), "gone_mod:highland");
            game.world.modified.insert((8, 8));
            game.save_level(true).unwrap();
            let reloaded = game
                .server
                .save
                .0
                .load(8, 8, game.pack.assets.registry, world_biomes!(game))
                .unwrap();
            assert_eq!(
                world_biomes!(game).saved_name(reloaded.biome(4, 16, 8)),
                "gone_mod:highland"
            );
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    /// NeoForge's `ModMismatchEvent`: a world saved with version 1 of a mod and
    /// opened with version 2 fires `mod_mismatch` once with both versions, a
    /// mod no longer loaded with no current version; the mod that marks its
    /// own mismatch resolved gets no warning while the missing one does. The
    /// save records the loaded list, so the reopened world fires nothing, as a
    /// world without the list doesn't.
    #[test]
    #[ignore = "daily"]
    fn a_world_saved_with_other_mod_versions_fires_mod_mismatch() {
        use super::super::native_entity_refs::tests::with_game;
        use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
        // The warning is read back from the process's logger and its last 50 lines. In the shared
        // test process another test may own the global logger, or flood those lines: this runs
        // in a process of its own.
        if run_fresh_cases(
            "game::server_side::world_loading::tests::a_world_saved_with_other_mod_versions_fires_mod_mismatch",
            "SUNRISE_MOD_MISMATCH_CHILD",
            &["alone"],
        ) {
            return;
        }
        use mod_api::{Hooks, HostSide, ModEntry, ModMismatch, ModMismatchEvent};
        use std::sync::{Arc, Mutex};
        use sunrise_sim::save::{
            Level,
            level_mods::{ModRecord, set_loaded},
        };

        struct Migrating(Arc<Mutex<Vec<Vec<ModMismatch>>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Migrating {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                self.0.lock().unwrap().push(event.mismatches.to_vec());
                event.mark_resolved("mismatch_test");
            }
        }
        struct Duplicate;
        #[mod_api::__subscriptions]
        impl Hooks for Duplicate {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                assert!(event.was_resolved("mismatch_test"));
                assert_eq!(event.resolver("mismatch_test"), Some("mismatch_test"));
                event.mark_resolved("mismatch_test");
            }
        }
        struct Observing;
        #[mod_api::__subscriptions]
        impl Hooks for Observing {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                assert_eq!(event.resolver("mismatch_test"), Some("mismatch_test"));
                assert_eq!(event.resolved()[0].resolver, "mismatch_test");
            }
        }
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("mod-mismatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sunrise_level::logging::init(&dir, "main");
        let record = |id: &str, version: &str| ModRecord {
            id: id.into(),
            version: version.into(),
        };
        let mismatch = |mod_id: &str, saved: &str, current: Option<&str>| ModMismatch {
            mod_id: mod_id.into(),
            saved_version: saved.into(),
            current_version: current.map(str::to_owned),
        };
        let request = |root: &std::path::Path| {
            let mut request = WorldRequest::new(
                "test",
                Some("test"),
                Some(1),
                true,
                None,
                Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
            )
            .unwrap();
            request.save_root = root.to_owned();
            request
        };
        let warnings = || -> Vec<String> {
            sunrise_level::logging::recent_lines()
                .into_iter()
                .filter(|line| line.contains("Sunrise: mod "))
                .collect()
        };
        let heard = Arc::new(Mutex::new(Vec::new()));
        let level = || Level {
            name: "test".into(),
            ..Level::default()
        };
        // The loaded list is the whole process's: the test's goes away when it ends, even
        // when it fails, so the tests after it save no list.
        struct Restore(Option<Vec<ModRecord>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                set_loaded(self.0.take());
            }
        }
        // Before the loaded mods are known: a world without the list.
        let _restore = Restore(set_loaded(None));
        let plain = dir.join("plain");
        level().write(&plain, &Default::default()).unwrap();
        assert_eq!(Level::read(&plain).unwrap().saved_mods, None);
        // Saved with version 1 of the mod, and with one since gone.
        let changed = dir.join("changed");
        set_loaded(Some(vec![
            record("mismatch_test", "1.0.0"),
            record("gone", "0.5.0"),
        ]));
        level().write(&changed, &Default::default()).unwrap();
        set_loaded(Some(vec![record("mismatch_test", "2.0.0")]));
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new("mismatch_test", Box::new(Migrating(heard.clone()))),
                ModEntry::new("duplicate", Box::new(Duplicate)),
                ModEntry::new("observing", Box::new(Observing)),
            ]
            .into(),
            || {
                with_game(|game| {
                    game.load_world(request(&changed)).unwrap();
                    let lines = warnings();
                    assert_eq!(
                        *heard.lock().unwrap(),
                        vec![vec![
                            mismatch("mismatch_test", "1.0.0", Some("2.0.0")),
                            mismatch("gone", "0.5.0", None),
                        ]]
                    );
                    assert!(
                        lines.iter().any(|line| line.contains(
                            "Sunrise: mod gone was 0.5.0 when the world was saved, now missing"
                        )),
                        "{lines:?}"
                    );
                    assert!(
                        !lines.iter().any(|line| line.contains("mod mismatch_test")),
                        "{lines:?}"
                    );
                    game.save_game_with(true).unwrap();
                });
                assert_eq!(
                    Level::read(&changed).unwrap().saved_mods,
                    Some(vec![record("mismatch_test", "2.0.0")])
                );
                // An unchanged list still fires.
                with_game(|game| {
                    game.load_world(request(&changed)).unwrap();
                    assert_eq!(heard.lock().unwrap().len(), 2);
                });
                // A world without the list fires nothing and gets the list on its save.
                with_game(|game| {
                    game.load_world(request(&plain)).unwrap();
                    assert_eq!(heard.lock().unwrap().len(), 2);
                    game.save_game_with(true).unwrap();
                });
                assert_eq!(
                    Level::read(&plain).unwrap().saved_mods,
                    Some(vec![record("mismatch_test", "2.0.0")])
                );
            },
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_mods_level_opens_with_no_data_packs() {
        use sunrise_level::chunk::{LevelShape, LevelShapes};
        use sunrise_level::datapack::worldgen_overrides_over;
        let (mut none, _) = worldgen_overrides_over(
            &[],
            &Default::default(),
            &sunrise_world::conditions::LoadConditions::vanilla(),
        );
        none.include_implicit_level_stems();
        assert_eq!(shapes_of(&none, none.level_stems()).0, LevelShapes::VANILLA);

        let pocket = sunrise_world::worldgen::overrides::ModDimension {
            id: "test:pocket".into(),
            stem: r#"{"type":"test:pocket","generator":{"type":"minecraft:noise"}}"#.into(),
            dimension_type:
                r#"{"min_y":-128,"height":448,"logical_height":320,"has_skylight":false,"has_ceiling":true}"#
                    .into(),
            sea_level: None,
        };
        let (mut overrides, _) = worldgen_overrides_over(
            &[pocket],
            &Default::default(),
            &sunrise_world::conditions::LoadConditions::vanilla(),
        );
        overrides.include_implicit_level_stems();
        let registry: std::collections::HashMap<_, _> = overrides.level_stems().collect();
        let traversal = [
            "minecraft:the_nether",
            "test:pocket",
            "minecraft:overworld",
            "minecraft:the_end",
        ];
        assert_eq!(registry.len(), traversal.len());
        let (shapes, configured) = shapes_of(
            &overrides,
            traversal.into_iter().map(|id| (id, registry[id])),
        );
        assert_eq!(configured, traversal);
        assert!(shapes.has("test:pocket"));
        assert_eq!(
            shapes.get("test:pocket"),
            LevelShape::new(-128, 448, 320, false, true).unwrap()
        );
        assert_eq!(shapes.dimension_type("test:pocket"), "test:pocket");
        assert_eq!(shapes.overworld, LevelShape::OVERWORLD);
    }

    /// A pack stem can pair a mod generator with a biome holder set without noise settings.
    #[test]
    #[ignore = "daily"]
    fn a_stem_naming_a_mods_generator_opens_on_mod_terrain() {
        use std::fs;
        use sunrise_level::chunk_streamer::ChunkGenerator as _;
        use sunrise_world::{
            native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
            worldgen::mod_generators::ChunkAccess,
        };
        struct Floor;
        impl ModChunkGenerator for Floor {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                let stone = chunk.state("stone");
                chunk.set_block([0, chunk.min_y(), 0], stone);
            }
        }
        let generators = ModChunkGenerators::from([(
            Identifier::parse("test:floor").unwrap(),
            Arc::new(Floor) as Arc<dyn ModChunkGenerator>,
        )]);
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let biomes = sunrise_level::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        )
        .unwrap();
        let registry = sunrise_level::assets::Assets::load(&root).unwrap().registry;
        let scratch = tempfile::tempdir().unwrap();
        let world = scratch.path();
        let pack = world.join("datapacks/checkerboard");
        let dimensions = pack.join("data/test/dimension");
        fs::create_dir_all(&dimensions).unwrap();
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Checkerboard test","min_format":[121,0],"max_format":[121,0]}}"#,
        ).unwrap();
        fs::write(
            world.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/checkerboard"],"disabled":[]}"#,
        )
        .unwrap();
        fs::write(
            dimensions.join("pocket.json"),
            r##"{
            "type":"minecraft:overworld", "generator":{"type":"test:floor", "biome_source":{
                "type":"minecraft:checkerboard", "biomes":"#minecraft:is_forest", "scale":0
            }}
        }"##,
        )
        .unwrap();
        let inputs = LoadInputs::new(Default::default(), root.join("settings"));
        let item_data = Arc::new(
            sunrise_world::item_defaults::ItemData::load(
                &root,
                LoadContext::new(DataLoadOrigin::ServerDataLoad, Arc::new(inputs), []),
            )
            .unwrap(),
        );
        let worldgen = root.join("worldgen");
        let resources = TerrainResources {
            data: None,
            dir: &worldgen,
            registry,
            biomes: &biomes,
            item_data: &item_data,
        };
        let base_conditions = sunrise_world::conditions::LoadConditions::vanilla();
        let (overrides, _) =
            worldgen_overrides_in_world(world, &root, &[], &[], &base_conditions).unwrap();
        let open = |dim, overrides: &_, saved: Option<&_>| {
            mod_terrain(
                overrides,
                resources,
                &generators,
                dim,
                sunrise_level::chunk::LevelShape::OVERWORLD,
                0,
                true,
                None,
                saved,
            )
        };
        let pocket = Dim::Extra(mod_api::__private::dimension_key("test:pocket"));
        let Some(Ok((ModLevel::Terrain(terrain), generator))) = open(pocket, &overrides, None)
        else {
            panic!("the pack's checkerboard needs no noise settings");
        };
        assert_eq!(
            generator.as_str(),
            "test:floor",
            "the read names the generator that built it"
        );
        let chunk = terrain.finish(terrain.proto(&mut (), 0, 0).unwrap());
        assert_eq!(
            chunk.block(0, -64, 0),
            registry.default_state("minecraft:stone").unwrap()
        );
        assert_eq!(biomes.name(chunk.biome(0, 0, 0)), "minecraft:forest");
        assert!(open(Dim::Overworld, &overrides, None).is_none());

        let empty = Default::default();
        let mut stem = serde_json::json!({
            "type": "minecraft:overworld",
            "generator": {
                "type": "test:floor",
                "biome_source": {"type": "minecraft:fixed", "biome": "minecraft:plains"},
            },
        });
        let saved = sunrise_game_data::scoreboard::json_to_tag(&stem);
        let Some(Ok((_, generator))) = open(pocket, &empty, Some(&saved)) else {
            panic!("the registered generator should also bind from a saved stem");
        };
        assert_eq!(generator.as_str(), "test:floor");

        for name in ["test:BAD", "test:unregistered"] {
            stem["generator"]["type"] = name.into();
            let invalid = sunrise_game_data::scoreboard::json_to_tag(&stem);
            let mut packed = empty.clone();
            packed.set(
                sunrise_world::worldgen::overrides::DIMENSIONS,
                pocket.id(),
                stem.to_string(),
            );
            assert!(
                open(pocket, &empty, Some(&invalid)).is_none(),
                "saved {name}"
            );
            // A valid pack stem wins before lookup even when its name does not bind.
            assert!(open(pocket, &packed, Some(&saved)).is_none(), "pack {name}");
        }
    }

    #[test]
    fn no_stage_checkerboard_opens_without_noise_settings_bundle() {
        use std::fs;
        use sunrise_level::chunk_streamer::ChunkGenerator as _;
        use sunrise_world::{
            native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
            registry::BlockRegistry,
            worldgen::{
                generator::data::GeneratorData, mod_generators::ChunkAccess,
                overrides::WorldgenOverrides,
            },
        };
        struct Floor;
        impl ModChunkGenerator for Floor {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                let stone = chunk.state("stone");
                chunk.set_block([0, chunk.min_y(), 0], stone);
            }
        }
        let generators = ModChunkGenerators::from([(
            Identifier::parse("test:floor").unwrap(),
            Arc::new(Floor) as Arc<dyn ModChunkGenerator>,
        )]);
        let scratch = tempfile::tempdir().unwrap();
        let resources_root = scratch.path().join("resources");
        let biome_dir = resources_root.join("data/minecraft/worldgen/biome");
        let worldgen_dir = resources_root.join("worldgen");
        fs::create_dir_all(&biome_dir).unwrap();
        fs::create_dir_all(&worldgen_dir).unwrap();
        fs::write(resources_root.join("items.json"), "{}").unwrap();
        fs::write(biome_dir.join("plains.json"), "{}").unwrap();
        fs::write(biome_dir.join("desert.json"), "{}").unwrap();
        fs::write(
            worldgen_dir.join("biomes.json"),
            r#"{"minecraft:plains":"{}","minecraft:desert":"{}"}"#,
        )
        .unwrap();
        fs::write(
            worldgen_dir.join("biome_tags.json"),
            r#"{"minecraft:is_dry":"{\"values\":[\"minecraft:desert\",\"minecraft:plains\"]}"}"#,
        )
        .unwrap();
        let registry: &'static BlockRegistry = Box::leak(Box::new(
            BlockRegistry::from_facts_json(
                &br#"{"states":[["minecraft:air",true,false,"","minecraft:air"],["minecraft:stone",false,false,"","minecraft:stone"]],"defaults":{"minecraft:air":0,"minecraft:stone":1}}"#[..],
            )
            .unwrap(),
        ));
        assert!(
            GeneratorData::load_with(&worldgen_dir, registry, &WorldgenOverrides::default())
                .is_err(),
            "no noise-settings or terrain-stage bundles exist in this fixture"
        );
        let biomes = sunrise_level::biome::Biomes::load_for_server(
            &sunrise_assets::resource_source::ResourceRoot::directory(resources_root.clone()),
        )
        .unwrap();
        let inputs = LoadInputs::new(Default::default(), resources_root.join("settings"));
        let item_data = Arc::new(
            sunrise_world::item_defaults::ItemData::load(
                &resources_root,
                LoadContext::new(DataLoadOrigin::ServerDataLoad, Arc::new(inputs), []),
            )
            .unwrap(),
        );
        let world = scratch.path().join("world");
        let pack = world.join("datapacks/checkerboard");
        let dimensions = pack.join("data/test/dimension");
        fs::create_dir_all(&dimensions).unwrap();
        fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"Checkerboard test","min_format":[121,0],"max_format":[121,0]}}"#,
        )
        .unwrap();
        fs::write(
            world.join("datapacks.json"),
            r#"{"enabled":["vanilla","file/checkerboard"],"disabled":[]}"#,
        )
        .unwrap();
        fs::write(
            dimensions.join("pocket.json"),
            r##"{
                "type":"minecraft:overworld",
                "generator":{
                    "type":"test:floor",
                    "biome_source":{
                        "type":"minecraft:checkerboard",
                        "biomes":"#minecraft:is_dry",
                        "scale":0
                    }
                }
            }"##,
        )
        .unwrap();
        let resources = TerrainResources {
            data: None,
            dir: &worldgen_dir,
            registry,
            biomes: &biomes,
            item_data: &item_data,
        };
        let pocket = Dim::Extra(mod_api::__private::dimension_key("test:pocket"));
        let base_conditions = sunrise_world::conditions::LoadConditions::vanilla();
        let (overrides, _) =
            worldgen_overrides_in_world(&world, &resources_root, &[], &[], &base_conditions)
                .unwrap();
        let Some(Ok((ModLevel::Terrain(terrain), generator))) = mod_terrain(
            &overrides,
            resources,
            &generators,
            pocket,
            sunrise_level::chunk::LevelShape::OVERWORLD,
            0,
            true,
            None,
            None,
        ) else {
            panic!("no-stage checkerboard must open without noise-settings bundle");
        };
        assert_eq!(generator.as_str(), "test:floor");
        let chunk0 = terrain.finish(terrain.proto(&mut (), 0, 0).unwrap());
        assert_eq!(
            chunk0.block(0, -64, 0),
            registry.default_state("minecraft:stone").unwrap()
        );
        assert_eq!(biomes.name(chunk0.biome(0, 0, 0)), "minecraft:desert");
        let chunk1 = terrain.finish(terrain.proto(&mut (), 1, 0).unwrap());
        assert_eq!(biomes.name(chunk1.biome(0, 0, 0)), "minecraft:plains");
    }

    #[test]
    fn worker_panics_reach_the_client() {
        let loader = WorldLoader {
            handle: std::thread::spawn(|| panic!("broken world initialization")),
        };
        assert_eq!(
            loader.finish().err().unwrap().to_string(),
            "world initialization worker panicked"
        );
    }

    #[test]
    fn prepared_world_can_cross_threads() {
        fn send<T: Send>() {}
        send::<PreparedWorld>();
    }
}
