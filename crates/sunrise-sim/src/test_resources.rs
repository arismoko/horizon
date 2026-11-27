//! Shared item-data producer for tests and standalone probes, not a runtime API.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use sunrise_world::{
    item_defaults::ItemData,
    native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopContext, LoopSide},
};

pub fn item_data(root: &Path, origin: DataLoadOrigin) -> sunrise_game_data::Result<Arc<ItemData>> {
    let inputs = LoadInputs::new(BTreeMap::new(), root.join("settings"));
    Ok(Arc::new(ItemData::load(
        root,
        LoadContext::new(origin, Arc::new(inputs), []),
    )?))
}

/// A server's data load with no mods and no settings, for loaders under test.
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not load recipes"
)]
pub fn server_load() -> LoadContext {
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(BTreeMap::new(), std::path::PathBuf::new())),
        [],
    )
}

/// A server's loop context with no mods and no settings, shared by tests that build a
/// motion or mob world.
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not build worlds"
)]
pub fn server_loop_context() -> &'static LoopContext {
    static CONTEXT: std::sync::OnceLock<LoopContext> = std::sync::OnceLock::new();
    CONTEXT.get_or_init(|| server_load().loop_context(LoopSide::Server, []))
}

/// A loop context of `side` with no mods and no settings, for menus under test.
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not build menus"
)]
pub fn loop_context(side: LoopSide) -> std::sync::Arc<LoopContext> {
    std::sync::Arc::new(server_load().loop_context(side, []))
}

/// Vanilla's villager foods (`DataComponents.VILLAGER_FOOD` nutrition), for the registries of
/// villager fixtures.
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not feed villagers"
)]
pub const VANILLA_VILLAGER_FOOD: [(&str, i32); 4] = [
    ("minecraft:bread", 4),
    ("minecraft:carrot", 1),
    ("minecraft:potato", 1),
    ("minecraft:beetroot", 1),
];

/// An item registry of only `food`, each item with its `villager_food` nutrition, catalogued
/// from a content root of its own: the registry a villager fixture lives with.
#[allow(
    dead_code,
    reason = "standalone probes that include this file may not feed villagers"
)]
pub fn villager_food_items(food: &[(&str, i32)]) -> sunrise_items::items::Items {
    let root = std::env::temp_dir().join(format!(
        "sunrise-villager-food-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
    let items: serde_json::Map<String, serde_json::Value> = food
        .iter()
        .map(|&(item, nutrition)| {
            let components =
                serde_json::json!({"minecraft:villager_food": {"nutrition": nutrition}});
            (item.to_owned(), components)
        })
        .collect();
    std::fs::write(
        root.join("items.json"),
        serde_json::Value::Object(items).to_string(),
    )
    .unwrap();
    let defaults = sunrise_world::item_defaults::read(&root).unwrap();
    let items = sunrise_items::items::Items::catalog(
        &root,
        &sunrise_world::loot::PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    items
}
