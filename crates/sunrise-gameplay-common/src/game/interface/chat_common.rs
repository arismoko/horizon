//! Common command registries, resource-id caches, permissions, and shared helpers.
use crate::game::side_ops::tables::TablesSideGameExt as _;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use sunrise_command::Registries;
use sunrise_game_data::gamerules::GameRules;
use sunrise_gameplay_common::world_clock::WorldClock;
use sunrise_world::{
    heightmap::{HeightmapState, HeightmapType},
    registry::{BlockRegistry, BlockStateId},
    worldgen::terrain::TerrainBlockStates,
};

/// The local player's name (offline single player).
pub const PLAYER_NAME: &str = "Player";

#[allow(unused_imports)]
pub use sunrise_gameplay_common::world_clock::OVERWORLD_CLOCK;
#[allow(unused_imports)]
pub use sunrise_gameplay_common::world_clock::TimeMarker;

/// An owned command tree, initialized even before a world is opened.
pub struct SessionDispatcher(pub Arc<sunrise_command::Dispatcher>);
impl Default for SessionDispatcher {
    fn default() -> Self {
        let registry = sunrise_game_data::gamerules::RuleRegistry::vanilla();
        Self(Arc::new(sunrise_command::vanilla::session_dispatcher(
            &registry,
            &[],
        )))
    }
}

/// Server- and common-accessible command, rule, clock, difficulty, and death-cause state.
#[derive(Default)]
pub struct WorldCommandState {
    pub registries: Registries,
    pub rules: GameRules,
    /// The server session command tree.
    pub dispatcher: SessionDispatcher,
    pub clock: WorldClock,
    /// The totals and state of the clocks the world has met, the Overworld's total among them
    /// while another clock is the one in `active_clock`. The active clock's total is
    /// World.day_time; the Overworld's state stays in `clock`.
    pub command_clocks: BTreeMap<String, (i64, WorldClock)>,
    /// The clock `World.day_time` follows: the default clock of the level in place, the
    /// Overworld's while empty. The totals of the others are in `command_clocks`.
    pub active_clock: String,
    pub end_clock: WorldClock,
    /// Time markers by `(clock_id, marker_id)` and timelines by `timeline_id -> (clock_id, period)`
    /// from the world's `minecraft:timeline` registry.
    pub markers: BTreeMap<(String, String), TimeMarker>,
    pub timelines: BTreeMap<String, (String, Option<i64>)>,
    /// 0 peaceful .. 3 hard.
    pub difficulty: u8,
    /// Death message key set by the cause of death (e.g. /kill).
    pub death_cause: Option<String>,
    /// The prepared resources before resource packs are layered on.
    pub vanilla_root: std::path::PathBuf,
}

/// The ids `build_registries` lists from the prepared resources' folders (cached per root).
pub struct ResourceIds {
    pub recipes: Vec<String>,
    pub(in crate::game) templates: Vec<String>,
    /// `entity-types.json`'s keys, sorted (empty when the file is missing).
    pub(in crate::game) entity_types: Vec<String>,
    pub(in crate::game) enchantments: Vec<String>,
    pub(in crate::game) structures: Vec<String>,
    pub(in crate::game) biomes: Vec<String>,
}

pub(in crate::game) static RESOURCE_IDS: Mutex<Vec<(PathBuf, Arc<ResourceIds>)>> =
    Mutex::new(Vec::new());

/// The `minecraft:` ids of the `.json` files directly in `dir`, sorted.
pub(in crate::game) fn json_ids_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".json")
                .map(|n| format!("minecraft:{n}"))
        })
        .collect();
    out.sort();
    out
}

pub fn resource_ids(root: &Path) -> Arc<ResourceIds> {
    let mut cache = RESOURCE_IDS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, ids)) = cache.iter().find(|(cached, _)| cached == root) {
        return ids.clone();
    }
    let data = root.join("data/minecraft");
    let entity_types = super::entity_type_rows::entity_type_rows(root).ids.clone();
    let ids = Arc::new(ResourceIds {
        // `/recipe <id>` must suggest a content mod's recipes too.
        recipes: json_ids_of_every_namespace(root, "recipe"),
        templates: templates_under(&root.join("worldgen/structure")),
        entity_types,
        // A content mod's enchantments too (`/enchant`, `/give` components).
        enchantments: json_ids_of_every_namespace(root, "enchantment"),
        structures: json_ids_in(&data.join("worldgen/structure")),
        biomes: json_ids_in(&data.join("worldgen/biome")),
    });
    cache.push((root.to_owned(), ids.clone()));
    ids
}

/// Every block's properties and their values, as `/setblock` completes them.
pub(in crate::game) type BlockProperties = BTreeMap<String, Vec<(String, Vec<String>)>>;

pub(in crate::game) static BLOCK_PROPERTIES: Mutex<Vec<(usize, Arc<BlockProperties>)>> =
    Mutex::new(Vec::new());

/// `BlockProperties` of a block registry, cached once per registry identity.
pub fn block_properties(registry: &BlockRegistry) -> Arc<BlockProperties> {
    // The payload is retained for the process, so equal data shares one table whatever borrows it.
    let key = registry.payload_identity();
    let mut cache = BLOCK_PROPERTIES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, properties)) = cache.iter().find(|(cached, _)| *cached == key) {
        return properties.clone();
    }
    let mut blocks: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for id in 0..registry.len() {
        let Some(state) = registry.state(BlockStateId(id as u32)) else {
            continue;
        };
        let props = blocks.entry(state.name.clone()).or_default();
        for (k, v) in &state.properties {
            let values = props.entry(k.clone()).or_default();
            if !values.contains(v) {
                values.push(v.clone());
            }
        }
    }
    let properties: Arc<BlockProperties> = Arc::new(
        blocks
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().collect()))
            .collect(),
    );
    cache.push((key, properties.clone()));
    properties
}

pub(in crate::game) static COMMAND_EXPORTS: Mutex<Vec<(PathBuf, Option<Arc<Registries>>)>> =
    Mutex::new(Vec::new());

/// Built-in registries from `command-registries.json`, cached once per root when `keep` is set.
pub fn command_export(dir: &Path, keep: bool) -> Option<Arc<Registries>> {
    let mut cache = COMMAND_EXPORTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, export)) = cache.iter().find(|(cached, _)| cached == dir) {
        return export.clone();
    }
    let export = std::fs::read(dir.join("command-registries.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .map(|export| Arc::new(Registries::from_export(&export)));
    if keep {
        cache.push((dir.to_owned(), export.clone()));
    }
    export
}

/// The ids of the `.json` files in `data/<namespace>/<kind>` of every namespace, sorted.
pub(in crate::game) fn json_ids_of_every_namespace(root: &Path, kind: &str) -> Vec<String> {
    let data = root.join("data");
    let mut namespaces: std::collections::BTreeSet<String> = std::fs::read_dir(&data)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    // A prepared root's namespaces live in its data file, not on the disk.
    if let Some(files) = sunrise_core::prepared_files::files_under(&data) {
        namespaces.extend(
            files
                .iter()
                .filter_map(|f| Some(f.split_once('/')?.0.to_owned())),
        );
    }
    let mut out: Vec<String> = namespaces
        .iter()
        .flat_map(|ns| ids_in(&data.join(ns).join(kind), "json", ns))
        .collect();
    out.sort();
    out
}

/// The templates of every namespace folder under `root`, as `namespace:path`, sorted.
pub(in crate::game) fn templates_under(root: &Path) -> Vec<String> {
    let Ok(namespaces) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<String> = namespaces
        .flatten()
        .filter(|e| e.path().is_dir())
        .flat_map(|e| ids_in(&e.path(), "nbt", &e.file_name().to_string_lossy()))
        .collect();
    out.sort();
    out
}

/// The `namespace:` ids of the `extension` files under `root`, sorted.
pub(in crate::game) fn ids_in(root: &Path, extension: &str, namespace: &str) -> Vec<String> {
    let suffix = format!(".{extension}");
    // A prepared root's folder: from its data file, without listing the disk.
    if let Some(files) = sunrise_core::prepared_files::files_under(root) {
        let mut out: Vec<String> = files
            .iter()
            .filter_map(|file| file.strip_suffix(&suffix))
            .filter(|path| !path.is_empty() && !path.ends_with('/'))
            .map(|path| format!("{namespace}:{path}"))
            .collect();
        out.sort();
        return out;
    }
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            // Follow symlinks like `Path::is_dir` while reusing the directory entry's type.
            let is_dir = e
                .file_type()
                .is_ok_and(|t| t.is_dir() || (t.is_symlink() && path.is_dir()));
            if is_dir {
                stack.push(path);
            } else if path.extension().and_then(|x| x.to_str()) == Some(extension)
                && let Ok(rel) = path.strip_prefix(root)
            {
                out.push(format!(
                    "{namespace}:{}",
                    rel.with_extension("").to_string_lossy().replace('\\', "/")
                ));
            }
        }
    }
    out.sort();
    out
}

/// `SuggestionProviders.SUMMONABLE_ENTITIES`: entity types available for `/summon`.
pub fn summonable_entities(
    entity_types: &[String],
    mob_types: impl IntoIterator<Item = impl AsRef<str>>,
) -> Vec<String> {
    let mut summonable: Vec<String> = if !entity_types.is_empty() {
        entity_types
            .iter()
            .filter(|id| !matches!(id.as_str(), "minecraft:player" | "minecraft:fishing_bobber"))
            .cloned()
            .collect()
    } else {
        let mut list: Vec<String> = mob_types
            .into_iter()
            .map(|k| sunrise_command::args::normalize(k.as_ref()))
            .filter(|id| !matches!(id.as_str(), "minecraft:player" | "minecraft:fishing_bobber"))
            .collect();
        list.extend(
            [
                "minecraft:item",
                "minecraft:experience_orb",
                "minecraft:falling_block",
                "minecraft:arrow",
                "minecraft:lightning_bolt",
                "minecraft:minecart",
                "minecraft:chest_minecart",
                "minecraft:hopper_minecart",
                "minecraft:furnace_minecart",
                "minecraft:tnt_minecart",
                "minecraft:wind_charge",
                "minecraft:breeze_wind_charge",
            ]
            .map(String::from),
        );
        list
    };
    // content mods' thrown types.
    let thrown = sunrise_sim::projectiles::thrown_types().iter();
    summonable.extend(thrown.map(|t| t.id.to_owned()));
    summonable.sort();
    summonable.dedup();
    summonable
}

/// `IntegratedServer.getProfilePermissions` for the owner: `OWNER` (4) or `ALL` (0).
pub fn owner_permission(allow_commands: bool) -> u8 {
    if allow_commands { 4 } else { 0 }
}

impl crate::game::CommonGame<'_> {
    /// `PlayerList.isOp` for the single-player owner: the world's Allow Commands.
    pub fn allow_commands(&self) -> bool {
        self.level.allow_commands == Some(true)
    }

    /// The canonical loaded inventory, with code targets resolved from the native runtime.
    pub fn perf_mods(&self) -> Vec<sunrise_perf_format::v1::Mod> {
        self.mods
            .perf_metadata(mod_api::__private::loaded_mods().iter())
    }

    /// `Heightmap.getFirstAvailable`: one above the highest block the heightmap counts.
    pub fn column_height(&self, kind: HeightmapType, x: i32, z: i32) -> i32 {
        let properties = &self.pack.assets.properties;
        let surface = self
            .world
            .world_surface(x, z, |s| properties.heightmap_state(s).is_air());
        if kind == HeightmapType::WorldSurface {
            return surface;
        }
        (self.world.bottom_y()..surface)
            .rev()
            .find(|&y| kind.is_opaque(properties.heightmap_state(self.world.block(x, y, z))))
            .map_or(self.world.bottom_y(), |y| y + 1)
    }

    /// The engine-side rule values (block behaviour and health).
    pub fn level_rules(&self) -> sunrise_world::level::GameRules {
        let r = &self.ui.rules;
        sunrise_world::level::GameRules {
            water_source_conversion: r
                .bool_at(sunrise_game_data::gamerules::vanilla::WATER_SOURCE_CONVERSION),
            lava_source_conversion: r
                .bool_at(sunrise_game_data::gamerules::vanilla::LAVA_SOURCE_CONVERSION),
            random_tick_speed: r.int_at(sunrise_game_data::gamerules::vanilla::RANDOM_TICK_SPEED),
            block_drops: r.bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS),
            mob_griefing: r.bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING),
            max_snow_accumulation_height: r
                .int_at(sunrise_game_data::gamerules::vanilla::MAX_SNOW_ACCUMULATION_HEIGHT),
            spawn_mobs: r.bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MOBS),
            spawn_monsters: r.bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MONSTERS),
            fire_spread_radius_around_player: r
                .int_at(sunrise_game_data::gamerules::vanilla::FIRE_SPREAD_RADIUS_AROUND_PLAYER),
            spread_vines: r.bool_at(sunrise_game_data::gamerules::vanilla::SPREAD_VINES),
        }
    }

    /// Apply rules mirrored in other state (natural regeneration, difficulty).
    pub fn sync_rules(&mut self) {
        self.world.health_rules.natural_regeneration = self
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::NATURAL_HEALTH_REGENERATION);
        // Player.isInvulnerableTo: the damage-type rules.
        self.world.health_rules.drowning_damage = self
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::DROWNING_DAMAGE);
        self.world.health_rules.fall_damage = self
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::FALL_DAMAGE);
        self.world.health_rules.fire_damage = self
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::FIRE_DAMAGE);
        self.world.health_rules.freeze_damage = self
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::FREEZE_DAMAGE);
        sunrise_sim::mobs::sulfur_cube::set_tnt_explodes(
            self.ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES),
        );
        use sunrise_world::food::Difficulty;
        self.world.health_rules.difficulty = match self.ui.difficulty {
            0 => Difficulty::Peaceful,
            1 => Difficulty::Easy,
            3 => Difficulty::Hard,
            _ => Difficulty::Normal,
        };
    }
}

pub trait ChatCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Registry contents for argument parsing (built when a world opens).
    fn build_registries(&mut self) {
        let permission = owner_permission(self.allow_commands());
        let reg = &mut self.ui.registries;
        reg.permission = permission;
        let ids = resource_ids(&self.options.resources);
        self.ui.registries.recipes = ids.recipes.clone();
        self.ui.registries.damage_types = self
            .health_resources
            .damage
            .entries()
            .map(|(_, d)| d.name.clone())
            .collect();
        self.sync_loot_table_ids();
        // ResourceOrIdArgument decodes an inline feature with the world's live feature codec.
        self.ui.registries.inline_feature = self.res.behaviours().live_features().map(|features| {
            sunrise_game_data::registries::InlineFeatureCheck::new(move |tag| {
                let json = sunrise_game_data::scoreboard::tag_to_json(tag).to_string();
                features
                    .decode_inline(&json)
                    .map(drop)
                    .map_err(|e| e.to_string())
            })
        });
        self.ui.registries.templates = ids.templates.clone();
        self.ui.registries.sounds = self.side_sound_event_names();
        self.ui.registries.attributes = [
            "armor",
            "armor_toughness",
            "attack_damage",
            "attack_knockback",
            "attack_speed",
            "block_break_speed",
            "block_interaction_range",
            "burning_time",
            "camera_distance",
            "explosion_knockback_resistance",
            "entity_interaction_range",
            "fall_damage_multiplier",
            "flying_speed",
            "follow_range",
            "gravity",
            "jump_strength",
            "knockback_resistance",
            "luck",
            "max_absorption",
            "max_health",
            "mining_efficiency",
            "movement_efficiency",
            "movement_speed",
            "oxygen_bonus",
            "safe_fall_distance",
            "scale",
            "sneaking_speed",
            "spawn_reinforcements",
            "step_height",
            "submerged_mining_speed",
            "sweeping_damage_ratio",
            "tempt_range",
            "water_movement_efficiency",
        ]
        .iter()
        .map(|a| format!("minecraft:{a}"))
        .collect();
        let common = self.common_mut();
        let reg = &mut common.ui.registries;
        let mut items: Vec<String> = common
            .res
            .items()
            .order
            .iter()
            .map(|s| s.to_string())
            .collect();
        items.sort();
        reg.items = items;
        reg.item_tags = common
            .res
            .items()
            .item_tags
            .keys()
            .map(|k| sunrise_command::args::normalize(k))
            .collect();
        reg.item_tags.sort();
        reg.block_tags = common
            .res
            .items()
            .block_tags
            .keys()
            .map(|k| sunrise_command::args::normalize(k))
            .collect();
        reg.block_tags.sort();
        reg.blocks = BlockProperties::clone(&block_properties(common.pack.assets.registry));
        reg.summonable = summonable_entities(&ids.entity_types, common.res.mob_types().keys());
        reg.entity_types.clone_from(&ids.entity_types);
        // The mob effect registry for `/effect` and `mob_effect` resource arguments.
        reg.effects = sunrise_level::brewing::all_effects()
            .map(|e| sunrise_level::brewing::namespaced(e.id).to_owned())
            .collect();
        reg.effects.sort();
        reg.enchantments = ids.enchantments.clone();
        reg.structures = ids.structures.clone();
        reg.biomes = ids.biomes.clone();
        // The mods' clocks (`register_world_clock`) for `/time of`; the world's own
        // join them when it opens (`sync_timeline_registries`).
        reg.clocks = Vec::new();
        reg.clocks
            .extend(mod_api::__private::world_clock_declarations());
        reg.clocks.sort();
        reg.dimensions = vec![
            "minecraft:overworld".into(),
            "minecraft:the_nether".into(),
            "minecraft:the_end".into(),
        ];
        reg.players = vec![PLAYER_NAME.into()];
        // Vanilla's built-in registries (mob effects, components, dialogs...).
        for dir in [&common.options.resources, &common.ui.vanilla_root] {
            if let Some(export) = command_export(dir, common.options.dedicated.is_none()) {
                common.ui.registries.fill_from_exported(&export);
                break;
            }
        }
        let reg = &mut common.ui.registries;
        let particles: Vec<_> = mod_api::__private::particle_ids()
            .map(str::to_owned)
            .collect();
        if !particles.is_empty() {
            reg.by_registry
                .entry("minecraft:particle_type".into())
                .or_default()
                .ids = particles;
            reg.particle_options = Some(|name, fields| {
                if let Some(result) = mod_api::__private::decode_particle_map(name, fields) {
                    return Some(result.map(mod_api::ParticleOptions::from_typed));
                }
                let key = mod_api::__private::particle_named_key(name)?;
                (mod_api::__private::particle_kind(key)
                    == Some(mod_api::__private::ParticleKind::Custom))
                .then(|| Ok(mod_api::ParticleOptions::simple(key)))
            });
        }
        if reg.entity_types.is_empty() {
            reg.entity_types.clone_from(&reg.summonable);
            reg.entity_types.push("minecraft:player".into());
        }
        reg.entity_types.extend(
            crate::game::general::general_type_catalog()
                .keys()
                .map(str::to_owned),
        );
        reg.entity_types.sort();
        reg.entity_types.dedup();
        // Replace the exported entity-type IDs with the live list while keeping its tags.
        if let Some(entry) = reg.by_registry.get_mut("minecraft:entity_type") {
            entry.ids.clone_from(&reg.entity_types);
        }
        // Native component names supply both wire IDs and command components.
        for (_, name) in sunrise_protocol::item::custom::entries() {
            if !reg.persistent_components.iter().any(|entry| entry == name) {
                reg.persistent_components.push(name.to_owned());
            }
        }
        self.sync_timeline_registries();
        self.sync_enchantment_registries();
        // After the export, which keys the registry with vanilla's tags.
        self.sync_stat_registries();
        self.side_sync_pack_tag_ids();
        self.side_sync_test_registry_ids();
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> ChatCommonGameExt1<'__sg> for G {}

impl crate::game::CommonGame<'_> {
    /// Every world clock's total ticks by id, for the loot conditions that read one.
    pub fn loot_clock_ticks(&self) -> std::collections::BTreeMap<String, i64> {
        let mut clocks: std::collections::BTreeMap<String, i64> = self
            .ui
            .command_clocks
            .iter()
            .map(|(name, state)| (name.clone(), state.0))
            .collect();
        for name in [self.active_clock(), OVERWORLD_CLOCK, "minecraft:the_end"] {
            clocks.insert(name.to_owned(), self.command_clock_time(name));
        }
        clocks
    }
}

use crate::game::side_ops::presentation::PresentationSideGameExt as _;

use crate::game::command_registries::CommandRegistriesGameExt1 as _;
