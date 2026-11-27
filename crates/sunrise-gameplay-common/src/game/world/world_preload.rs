use crate::game::{pack_discovery::data_repository, startup};
use std::sync::Arc;
use sunrise_sim::save::Level;

/// How long the preloaded world data outlives the world screens: a player
/// who goes back to the title or to Multiplayer and opens no world for this
/// long gets the memory back.
const PRELOAD_KEPT: std::time::Duration = std::time::Duration::from_secs(60);

enum Preload {
    Idle,
    /// Read, or being read, on the "World data preload" thread. `left`: when
    /// the player last left the world screens.
    Started {
        thread: std::thread::JoinHandle<()>,
        left: Option<std::time::Instant>,
    },
    /// A world opened: the data is the world's now, and stays.
    Used,
}

static PRELOAD: std::sync::Mutex<Preload> = std::sync::Mutex::new(Preload::Idle);

/// Each frame of the title: preload the shared worldgen bundle on a helper
/// thread while the player is on the world screens
/// (`on_world_screens`: Singleplayer, Create World and their
/// dialogs), so it overlaps the time spent choosing a world. A world opened
/// meanwhile waits for the part being read instead of reading it again. Not
/// at the title screen itself: many players only go to Multiplayer, and
/// small machines need the memory. Dropped after `PRELOAD_KEPT` away from
/// the world screens without a world opening.
pub fn preload_world_data(
    on_world_screens: bool,
    resources: &std::path::Path,
    data_source: &sunrise_assets::resource_source::ResourceSource,
    registry: &'static sunrise_world::registry::BlockRegistry,
) {
    let mut preload = PRELOAD.lock().unwrap_or_else(|e| e.into_inner());
    match &mut *preload {
        Preload::Used => {}
        Preload::Idle if on_world_screens => {
            let resources = resources.to_owned();
            let data_source = data_source.clone();
            let spawned = std::thread::Builder::new()
                .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
                .name("World data preload".into())
                .spawn(move || {
                    sunrise_threads::behind_server();
                    // Errors are left for the world opening to report.
                    if let Ok(data) = sunrise_sim::vanillagen::shared_data_from(
                        &resources.join("worldgen"),
                        &Default::default(),
                        registry,
                        &data_source,
                    ) {
                        // The Overworld's feature decoration, which every
                        // seed shares: about 140 ms off opening the world.
                        let _ = data.prepare_decoration("minecraft:normal", "minecraft:overworld");
                    }
                });
            match spawned {
                Ok(thread) => *preload = Preload::Started { thread, left: None },
                Err(e) => log::warn!("Sunrise: world data preload: {e}"),
            }
        }
        Preload::Idle => {}
        Preload::Started { left, .. } if on_world_screens => *left = None,
        Preload::Started { thread, left } => {
            let left = *left.get_or_insert_with(std::time::Instant::now);
            if left.elapsed() >= PRELOAD_KEPT && thread.is_finished() {
                sunrise_sim::vanillagen::forget_shared_data();
                crate::game::lan_common::sync::forget_sync_data();
                crate::game::lan_common::sync::forget_server_sources();
                sunrise_threads::release_free_memory();
                *preload = Preload::Idle;
                log::info!("Sunrise: preloaded world data dropped, no world opened");
            }
        }
    }
}

/// A launch that names a world (`--world`, `--quickPlaySingleplayer`): the
/// same preload, started as soon as the block registry and the biomes are
/// loaded, so it overlaps the rest of the startup instead of the world's
/// opening. Read for the world's own data packs and preset, the bundle and
/// decoration `make_streaming` asks for; a world they don't fit (one its
/// upgrade rewrites) reads its own when it opens.
pub(in crate::game) fn preload_launch_world(
    folder: &str,
    resources: &std::path::Path,
    builtin_source: &sunrise_assets::resource_source::ResourceSource,
    data_source: &sunrise_assets::resource_source::ResourceSource,
    registry: &'static sunrise_world::registry::BlockRegistry,
    additional: &[Arc<sunrise_level::datapack::DataPack>],
    base_conditions: sunrise_world::conditions::LoadConditions,
) {
    let mut preload = PRELOAD.lock().unwrap_or_else(|e| e.into_inner());
    if !matches!(*preload, Preload::Idle) {
        return;
    }
    let Ok(dir) = std::env::current_dir() else {
        return;
    };
    let world_root = dir.join("saves").join(folder);
    let resources = resources.to_owned();
    let builtin_source = builtin_source.clone();
    let data_source = data_source.clone();
    let additional = additional.to_vec();
    let spawned = std::thread::Builder::new()
        .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
        .name("World data preload".into())
        .spawn(move || {
            // Not behind the server: the world's opening waits for it.
            let _step = startup::step("world.preload");
            let started = std::time::Instant::now();
            // Errors are left for the world opening to report. The world's
            // flags from its level.dat: the feature packs a flag forces on.
            let level = Level::read(&world_root);
            let enabled_features = level
                .as_ref()
                .map_or(&[][..], |l| l.enabled_features.as_ref());
            let packs = data_repository(&world_root, &resources, &additional);
            let enabled =
                sunrise_level::datapack::enabled_ids(&world_root, &packs, enabled_features);
            let stack = crate::game::function_pack_stack::PackStack::new(
                &builtin_source,
                &data_source,
                false,
                packs,
                enabled,
                enabled_features,
            );
            let enabled: Vec<_> = stack.enabled_packs().collect();
            let conditions = crate::game::load_conditions::worldgen_load_conditions(
                &base_conditions,
                &resources,
                &enabled,
            );
            let Ok((overrides, _)) = preload_overrides(&stack.data_packs(Vec::new()), &conditions)
            else {
                return;
            };
            if let Ok(data) = sunrise_sim::vanillagen::shared_data_from(
                &resources.join("worldgen"),
                &overrides,
                registry,
                &data_source,
            ) {
                let generator = level
                    .as_ref()
                    .and_then(|l| l.generator.as_ref())
                    .map(|g| g.settings());
                let preset = sunrise_sim::save::WorldGen::noise_preset(generator);
                let _ = data.prepare_decoration(preset, "minecraft:overworld");
            }
            log::info!(
                "Sunrise: world data preloaded for the launch's world in {} ms",
                started.elapsed().as_millis()
            );
        });
    match spawned {
        Ok(thread) => *preload = Preload::Started { thread, left: None },
        Err(e) => log::warn!("Sunrise: world data preload: {e}"),
    }
}

fn preload_overrides(
    packs: &sunrise_level::datapack::DataPacks,
    conditions: &sunrise_world::conditions::LoadConditions,
) -> Result<
    (
        sunrise_world::worldgen::overrides::WorldgenOverrides,
        Vec<String>,
    ),
    String,
> {
    let result = sunrise_level::datapack::worldgen_overrides(packs, conditions);
    conditions.check_errors()?;
    Ok(result)
}

/// A world opens: what `preload_world_data` read is the world's from now on.
pub fn world_data_used() {
    *PRELOAD.lock().unwrap_or_else(|e| e.into_inner()) = Preload::Used;
}
