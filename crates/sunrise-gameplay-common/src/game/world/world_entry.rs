//! Entering a world without blocking the window: the deferred entry, world creation, the world
//! resources' background load, and the world's own content tables.
use std::sync::atomic::{AtomicU64, Ordering};

pub(in crate::game) static BIOME_KEYS_SESSION: AtomicU64 = AtomicU64::new(1);

pub fn next_biome_keys_session() -> u64 {
    BIOME_KEYS_SESSION.fetch_add(1, Ordering::Relaxed)
}

use sunrise_game_data::Result;
use sunrise_items::items::Items;
use sunrise_level::biome::Biomes;

pub fn bind_world_registry_tags(
    items: &mut Items,
    root: &std::path::Path,
    tag_files: &sunrise_world::loot::PackOverlay<'_>,
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    biomes: &Biomes,
) -> Result<()> {
    items.bind_enchantment_tags(root, tag_files)?;
    items.bind_registry_tags(
        root,
        tag_files,
        sunrise_items::items::TagRegistry::Biome,
        "worldgen/biome",
        |id| biomes.contains(id),
    )?;
    let mut structures: std::collections::HashSet<String> = items
        .component_registry
        .ids("minecraft:worldgen/structure")
        .iter()
        .cloned()
        .collect();
    for (id, _) in overlay.kind("worldgen/structure") {
        structures.insert(id.to_owned());
    }
    items.bind_registry_tags(
        root,
        tag_files,
        sunrise_items::items::TagRegistry::Structure,
        "worldgen/structure",
        |id| structures.contains(id),
    )?;
    let mut clocks: std::collections::HashSet<String> = items
        .component_registry
        .ids("minecraft:world_clock")
        .iter()
        .cloned()
        .collect();
    for id in mod_api::__private::world_clock_declarations() {
        clocks.insert(id.to_owned());
    }
    for (id, _) in overlay.kind("world_clock") {
        clocks.insert(id.to_owned());
    }
    items.bind_registry_tags(
        root,
        tag_files,
        sunrise_items::items::TagRegistry::WorldClock,
        "world_clock",
        |id| clocks.contains(id),
    )?;
    Ok(())
}
