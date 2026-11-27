//! Villager registry indices shared by matching mod loads; no additional packet format.
pub use sunrise_protocol::{entity_data::Value, registry::Builtin};
use sunrise_sim::mobs::villager::VILLAGER_TYPES;
pub use sunrise_sim::mobs::{Mob, Species, villager::ResolvedProfessions};
/// VillagerData as the network sends it: the type and the profession by
/// their built-in registry ids, then the level.
pub fn villager_data(m: &mut Mob, data: &[i32], builtin: &Builtin) {
    let &[kind, profession, level] = data else {
        return;
    };
    let index = |registry: &str, id: i32, names: &[&str]| {
        let name = builtin
            .get(registry)?
            .name(id)?
            .strip_prefix("minecraft:")?;
        names.iter().position(|n| *n == name).map(|i| i as u8)
    };
    if let Some(t) = index("minecraft:villager_type", kind, &VILLAGER_TYPES) {
        m.extra.variant = t;
    }
    if let Some(p) = builtin
        .get("minecraft:villager_profession")
        .and_then(|r| r.name(profession))
        .and_then(|name| name.strip_prefix("minecraft:"))
        .and_then(sunrise_sim::mobs::villager::profession_index)
    {
        m.extra.profession = sunrise_sim::mobs::villager::ProfessionState::from_index(p);
    }
    m.extra.villager_level = level.max(1);
}

pub fn server_meta(
    m: &Mob,
    hub: &sunrise_sim::lan::hub::Hub,
    player: Option<i32>,
    time: i64,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
    table: &ResolvedProfessions,
) {
    crate::game::lan_mob_metadata::mob_meta(m, hub, player, time, meta, defaults);
    let index = match m.species() {
        Species::Villager => 19,
        Species::ZombieVillager => 20,
        _ => return,
    };
    for (field, _, value) in meta {
        if *field == index
            && let Value::Ids(data) = value
            && data.len() == 3
        {
            // Unknown saved names keep the existing none projection, without changing the save.
            data[1] = table
                .state_index(&m.extra.profession)
                .and_then(|slot| i32::try_from(slot).ok())
                .unwrap_or(0);
        }
    }
}

pub use std::sync::Arc;
pub(in crate::game) type BuiltinCache = Vec<(std::path::PathBuf, Arc<Builtin>)>;
pub(in crate::game) static BUILTIN: std::sync::Mutex<BuiltinCache> =
    std::sync::Mutex::new(Vec::new());

/// The prepared resources' `registries.json`, parsed once per root: the
/// file never changes while the game runs, and parsing it took about 3 ms
/// of the frame that joins a server (or opens a single-player world).
/// `None` when it is missing or invalid (not cached: a fixed install can
/// join again).
pub fn builtin_registries(root: &std::path::Path) -> Option<Arc<Builtin>> {
    let mut cache = BUILTIN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, builtin)) = cache.iter().find(|(cached, _)| cached == root) {
        return Some(builtin.clone());
    }
    let builtin = Arc::new(sunrise_sim::lan::data::with_custom_stats(
        Builtin::find(root).and_then(|p| Builtin::load(&p).ok())?,
    ));
    cache.push((root.to_owned(), builtin.clone()));
    Some(builtin)
}
