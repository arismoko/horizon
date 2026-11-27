//! `Game::biome_facts` over what it reads, so a helper thread can ask too.
use sunrise_sim::mobs::{farm::Climate, spawning::BiomeFacts};

/// The facts of the biome at a position, for `finalizeSpawn` and the variant rules.
pub(in crate::game) fn biome_facts_in(
    world: &sunrise_level::world::World,
    biomes: &sunrise_level::biome::Biomes,
    tags: &sunrise_sim::mobs::MobData,
    x: i32,
    y: i32,
    z: i32,
) -> BiomeFacts {
    let Some(chunk) = world.chunks.get(&(x >> 4, z >> 4)) else {
        return BiomeFacts::default();
    };
    let b = biomes.get(chunk.biome(x, y, z));
    let has = |t: &str| tags.biome_has(t, &b.name);
    let climate = if has("minecraft:spawns_warm_variant_farm_animals") {
        Climate::Warm
    } else if has("minecraft:spawns_cold_variant_farm_animals") {
        Climate::Cold
    } else {
        Climate::Temperate
    };
    BiomeFacts {
        white_rabbit: has("minecraft:spawns_white_rabbits"),
        gold_rabbit: has("minecraft:spawns_gold_rabbits"),
        climate,
        temperature: b.temperature,
        snowy: has("minecraft:spawns_snow_foxes")
            || b.name.contains("snow")
            || b.name.contains("frozen"),
        snow_fox: has("minecraft:spawns_snow_foxes"),
        swamp: b.name.contains("swamp"),
        ocean: has("minecraft:is_ocean"),
        mushroom: b.name == "minecraft:mushroom_fields",
        desert: b.name == "minecraft:desert",
        jungle: has("minecraft:is_jungle"),
    }
}
