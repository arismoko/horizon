//! Data-pack pig, cow and chicken variants and sound variants: spawn selection,
//! save round trip, synced client model/texture resolution, sound playback,
//! egg hatching variant, and `LootEntityView::chicken_variant`.
use super::*;
use crate::mobs::variants::VariantTables;
use crate::{
    enchant::Enchantments,
    entity_nbt::{self, EntityTagWriter, MobSave},
    items::{ItemStack, Items},
    lan::data::{PackRegistryDefinitions, RegistryRef, SyncData},
    mobs::{
        Mob, MobEvent, MobType, Species, ai, drops,
        finalize::{self, Ctx, FinalizeLevel, SpawnData},
        spawn_rules::Reason,
        spawning::LocalDifficulty,
    },
    projectiles::{Projectile, ProjectileKind},
};
use serde_json::json;
use std::{collections::HashMap, sync::Arc};
use sunrise_core::{
    geometry::{Aabb, Vec3},
    nbt::Tag,
    random::LegacyRandom,
};
use sunrise_protocol::{
    packets::configuration::{RegistryData, RegistryEntry},
    registry::Dynamic,
};
use sunrise_world::loot::PackOverlay;

fn mob_type(id: &str, species: Species) -> Arc<MobType> {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let k = Arc::make_mut(&mut kind);
    k.id = id.into();
    k.species = species;
    kind
}

fn animal_variant_json(
    model: Option<&str>,
    asset_id: &str,
    baby_asset_id: &str,
    spawn_conditions: Value,
) -> Vec<u8> {
    let mut map = serde_json::Map::new();
    if let Some(m) = model {
        map.insert("model".into(), json!(m));
    }
    map.insert("asset_id".into(), json!(asset_id));
    map.insert("baby_asset_id".into(), json!(baby_asset_id));
    map.insert("spawn_conditions".into(), spawn_conditions);
    Value::Object(map).to_string().into_bytes()
}

fn pig_sound_json(adult_prefix: &str, baby_prefix: &str) -> Vec<u8> {
    let set = |p: &str| {
        json!({
            "ambient_sound": {"sound_id": format!("{p}.ambient")},
            "hurt_sound": {"sound_id": format!("{p}.hurt")},
            "death_sound": {"sound_id": format!("{p}.death")},
            "step_sound": {"sound_id": format!("{p}.step")},
            "eat_sound": {"sound_id": format!("{p}.eat")},
        })
    };
    json!({
        "adult_sounds": set(adult_prefix),
        "baby_sounds": set(baby_prefix),
    })
    .to_string()
    .into_bytes()
}

fn cow_sound_json(prefix: &str) -> Vec<u8> {
    json!({
        "ambient_sound": {"sound_id": format!("{prefix}.ambient")},
        "hurt_sound": {"sound_id": format!("{prefix}.hurt")},
        "death_sound": {"sound_id": format!("{prefix}.death")},
        "step_sound": {"sound_id": format!("{prefix}.step")},
    })
    .to_string()
    .into_bytes()
}

fn chicken_sound_json(adult_prefix: &str, baby_prefix: &str) -> Vec<u8> {
    let set = |p: &str| {
        json!({
            "ambient_sound": {"sound_id": format!("{p}.ambient")},
            "hurt_sound": {"sound_id": format!("{p}.hurt")},
            "death_sound": {"sound_id": format!("{p}.death")},
            "step_sound": {"sound_id": format!("{p}.step")},
        })
    };
    json!({
        "adult_sounds": set(adult_prefix),
        "baby_sounds": set(baby_prefix),
    })
    .to_string()
    .into_bytes()
}

fn stage_pack(files: &[(String, Vec<Vec<u8>>)]) -> PackRegistryDefinitions {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), b"{}").unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let (packed, admitted) = SyncData::stage_registry_definitions(
        root.path(),
        &defaults,
        &PackOverlay::new(files),
        &Default::default(),
        &Default::default(),
        |_| true,
    );
    admitted.unwrap();
    packed
}

fn sync_tables(packed: &PackRegistryDefinitions, registries: &[&str]) -> VariantTables {
    let mut synced = Dynamic::default();
    for &registry in registries {
        let mut encoded = Vec::new();
        packed.append_encoded(registry, &mut encoded);
        encoded.sort_by_cached_key(|((ns, path), _, _)| (format!("{path}.json"), ns.clone()));
        let packet = RegistryData {
            registry: registry.to_owned(),
            entries: encoded
                .into_iter()
                .map(|((ns, path), tag, _)| RegistryEntry {
                    id: format!("{ns}:{path}"),
                    data: Some(tag),
                })
                .collect(),
        };
        synced.add(packet);
    }
    VariantTables::from_synced(&synced, &VariantTables::default()).unwrap()
}

struct TestLevel<'a> {
    biome: &'a str,
    variants: &'a VariantTables,
    enchantments: Enchantments,
    items: Items,
}

impl FinalizeLevel for TestLevel<'_> {
    fn biome(&self, _: [i32; 3]) -> String {
        self.biome.to_owned()
    }
    fn biome_tag(&self, _: &str, _: [i32; 3]) -> bool {
        false
    }
    fn structures_with_piece_at(&self, _: [i32; 3]) -> Vec<String> {
        Vec::new()
    }
    fn moon_phase(&self, _: [i32; 3]) -> usize {
        0
    }
    fn spawning_monsters(&self) -> bool {
        true
    }
    fn no_collision(&self, _: Aabb) -> bool {
        true
    }
    fn free_chicken(&self, _: Aabb) -> Option<u32> {
        None
    }
    fn ride_chicken(&mut self, _: u32, _: u32) {}
    fn game_time(&self) -> i64 {
        0
    }
    fn halloween(&self) -> bool {
        false
    }
    fn create(&mut self, _: &str, _: Reason) -> Option<Mob> {
        None
    }
    fn spawn_data(&self) -> SpawnData<'_> {
        SpawnData {
            variants: self.variants,
            enchantments: &self.enchantments,
            items: &self.items,
            camel_husk_size: (1.7, 2.375),
        }
    }
}

fn test_level<'a>(biome: &'a str, variants: &'a VariantTables) -> TestLevel<'a> {
    TestLevel {
        biome,
        variants,
        enchantments: Enchantments::default(),
        items: Items::empty(),
    }
}

fn save_mob(mob: &Mob, variants: &VariantTables) -> Tag {
    let mut writer = EntityTagWriter::from_saved(Tag::compound(Vec::<(&str, Tag)>::new()));
    entity_nbt::write_mob(
        mob,
        [1, 2, 3, 4],
        &mut writer,
        None,
        |_| None,
        &Items::empty(),
        MobSave {
            variants: Some(variants),
            ..Default::default()
        },
    );
    writer.finish()
}

/// Pig: a pack variant with `model: "cold"` and a pack sound variant spawn where
/// their condition applies, round-trip through save/load, sync to a client for
/// model/texture resolution, and play pack ambient and eating sounds.
#[test]
fn pack_pig_variant_spawns_round_trips_and_syncs() {
    let files = [
        (
            "pig_variant/minecraft:temperate".to_owned(),
            vec![animal_variant_json(
                None,
                "minecraft:entity/pig/pig_temperate",
                "minecraft:entity/pig/pig_temperate_baby",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "pig_variant/pack:truffle".to_owned(),
            vec![animal_variant_json(
                Some("cold"),
                "pack:entity/pig/truffle",
                "pack:entity/pig/truffle_baby",
                json!([{"priority": 1, "condition": {"type": "minecraft:biome", "biomes": "minecraft:taiga"}}]),
            )],
        ),
        (
            "pig_sound_variant/pack:snort".to_owned(),
            vec![pig_sound_json("pack:pig.snort", "pack:pig.snort_baby")],
        ),
    ];
    let packed = stage_pack(&files);
    let tables = packed.variant_tables();
    let truffle = tables.pig.lookup("pack:truffle").unwrap();
    let temperate = tables.pig.lookup("minecraft:temperate").unwrap();
    let snort = tables.pig_sound.lookup("pack:snort").unwrap();

    let kind = mob_type("minecraft:pig", Species::Pig);
    let mut pig = Mob::new(1, kind.clone(), Vec3::new(0.0, 64.0, 0.0), 0.0);
    let mut level = test_level("minecraft:taiga", &tables);
    let mut rng = LegacyRandom::new(11);
    let mut cx = Ctx {
        level: &mut level,
        random: &mut rng,
        difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
        created: Vec::new(),
        jockey_rider: None,
    };
    finalize::finalize(&mut cx, &mut pig, Reason::Natural, None);
    assert_eq!(pig.extra.pig_variant, Some(truffle));
    assert_eq!(pig.extra.pig_sound_variant, Some(snort));

    // Save and reload round-trip.
    let saved = save_mob(&pig, &tables);
    assert_eq!(
        saved.get("variant").and_then(|t| match t {
            Tag::String(s) => Some(s.to_string_lossy()),
            _ => None,
        }),
        Some("pack:truffle".to_owned())
    );
    let types = HashMap::from([("minecraft:pig".to_owned(), kind.clone())]);
    let loaded = entity_nbt::mob_from_nbt_with_variants(&saved, &types, 2, None, &tables).unwrap();
    assert_eq!(loaded.extra.pig_variant, Some(truffle));
    assert_eq!(loaded.extra.pig_sound_variant, Some(snort));

    // Unknown saved variant falls back to default (`minecraft:temperate`).
    let mut bad = saved.clone();
    if let Tag::Compound(map) = &mut bad {
        map.insert("variant".into(), Tag::String("pack:missing".into()));
    }
    let fallback = entity_nbt::mob_from_nbt_with_variants(&bad, &types, 3, None, &tables).unwrap();
    assert_eq!(fallback.extra.pig_variant, Some(temperate));

    // Client synced table resolves model and texture from the network id.
    let client_tables = sync_tables(
        &packed,
        &["minecraft:pig_variant", "minecraft:pig_sound_variant"],
    );
    let mut remote = Mob::new(4, kind, Vec3::ZERO, 0.0);
    remote.extra.pig_variant = Some(RegistryRef::new(truffle.network_id()));
    let pig_entry = remote
        .pig_variant(&client_tables.pig)
        .and_then(|r| client_tables.pig.get(r))
        .unwrap();
    assert_eq!(pig_entry.model_key(remote.is_baby()), "cold_pig#main");
    assert_eq!(
        pig_entry.texture(remote.is_baby()),
        "pack:entity/pig/truffle"
    );
    remote.extra.age = -24000;
    assert_eq!(pig_entry.model_key(remote.is_baby()), "pig_baby#main");
    assert_eq!(
        pig_entry.texture(remote.is_baby()),
        "pack:entity/pig/truffle_baby"
    );

    // Ambient and eating sounds resolve from the pack sound variant.
    let mut events = Vec::new();
    ai::play_farm_ambient(&mut pig, Some(&tables), &mut events);
    assert!(events.iter().any(|e| matches!(
        e,
        MobEvent::SoundId(id, ..) if id == "pack:pig.snort.ambient"
    )));
    let eat = ai::eating_sound_with_variants(&mut pig, &mut rng, Some(&tables));
    assert_eq!(
        eat.map(|(s, ..)| s.into_owned()).as_deref(),
        Some("pack:pig.snort.eat")
    );
}

/// Cow: a pack variant with `model: "warm"` and a flat pack sound variant spawn
/// where their condition applies, round-trip through save/load, and sync to a
/// client for warm adult/baby model and texture resolution.
#[test]
fn pack_cow_variant_spawns_round_trips_and_syncs() {
    let files = [
        (
            "cow_variant/minecraft:temperate".to_owned(),
            vec![animal_variant_json(
                None,
                "minecraft:entity/cow/cow_temperate",
                "minecraft:entity/cow/cow_temperate_baby",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "cow_variant/pack:highland".to_owned(),
            vec![animal_variant_json(
                Some("warm"),
                "pack:entity/cow/highland",
                "pack:entity/cow/highland_baby",
                json!([{"priority": 1, "condition": {"type": "minecraft:biome", "biomes": "minecraft:savanna"}}]),
            )],
        ),
        (
            "cow_sound_variant/pack:deep".to_owned(),
            vec![cow_sound_json("pack:cow.deep")],
        ),
    ];
    let packed = stage_pack(&files);
    let tables = packed.variant_tables();
    let highland = tables.cow.lookup("pack:highland").unwrap();
    let deep = tables.cow_sound.lookup("pack:deep").unwrap();

    let kind = mob_type("minecraft:cow", Species::Cow);
    let mut cow = Mob::new(1, kind.clone(), Vec3::new(0.0, 64.0, 0.0), 0.0);
    let mut level = test_level("minecraft:savanna", &tables);
    let mut rng = LegacyRandom::new(19);
    let mut cx = Ctx {
        level: &mut level,
        random: &mut rng,
        difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
        created: Vec::new(),
        jockey_rider: None,
    };
    finalize::finalize(&mut cx, &mut cow, Reason::Natural, None);
    assert_eq!(cow.extra.cow_variant, Some(highland));
    assert_eq!(cow.extra.cow_sound_variant, Some(deep));

    let saved = save_mob(&cow, &tables);
    let types = HashMap::from([("minecraft:cow".to_owned(), kind.clone())]);
    let loaded = entity_nbt::mob_from_nbt_with_variants(&saved, &types, 2, None, &tables).unwrap();
    assert_eq!(loaded.extra.cow_variant, Some(highland));
    assert_eq!(loaded.extra.cow_sound_variant, Some(deep));

    let client_tables = sync_tables(
        &packed,
        &["minecraft:cow_variant", "minecraft:cow_sound_variant"],
    );
    let mut remote = Mob::new(3, kind, Vec3::ZERO, 0.0);
    remote.extra.cow_variant = Some(RegistryRef::new(highland.network_id()));
    let cow_entry = remote
        .cow_variant(&client_tables.cow)
        .and_then(|r| client_tables.cow.get(r))
        .unwrap();
    assert_eq!(cow_entry.model_key(remote.is_baby()), "warm_cow#main");
    assert_eq!(
        cow_entry.texture(remote.is_baby()),
        "pack:entity/cow/highland"
    );
    remote.extra.age = -24000;
    assert_eq!(cow_entry.model_key(remote.is_baby()), "warm_cow_baby#main");
    assert_eq!(
        cow_entry.texture(remote.is_baby()),
        "pack:entity/cow/highland_baby"
    );

    let mut events = Vec::new();
    ai::play_farm_ambient(&mut cow, Some(&tables), &mut events);
    assert!(events.iter().any(|e| matches!(
        e,
        MobEvent::SoundId(id, ..) if id == "pack:cow.deep.ambient"
    )));
}

/// Chicken: a pack variant spawns where its condition applies, round-trips
/// through save/load, surfaces its pack id through `LootEntityView::chicken_variant`,
/// preserves its pack id through `ThrownEgg`, and resolves synced model/texture.
#[test]
fn pack_chicken_variant_spawns_round_trips_loot_view_and_egg() {
    let files = [
        (
            "chicken_variant/minecraft:temperate".to_owned(),
            vec![animal_variant_json(
                None,
                "minecraft:entity/chicken/chicken_temperate",
                "minecraft:entity/chicken/chicken_temperate_baby",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "chicken_variant/pack:golden_hen".to_owned(),
            vec![animal_variant_json(
                Some("cold"),
                "pack:entity/chicken/golden_hen",
                "pack:entity/chicken/golden_hen_baby",
                json!([{"priority": 1, "condition": {"type": "minecraft:biome", "biomes": "minecraft:meadow"}}]),
            )],
        ),
        (
            "chicken_sound_variant/pack:cluck".to_owned(),
            vec![chicken_sound_json(
                "pack:chicken.cluck",
                "pack:chicken.peep",
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let tables = packed.variant_tables();
    let golden = tables.chicken.lookup("pack:golden_hen").unwrap();
    let cluck = tables.chicken_sound.lookup("pack:cluck").unwrap();

    let kind = mob_type("minecraft:chicken", Species::Chicken);
    let mut hen = Mob::new(1, kind.clone(), Vec3::new(0.0, 64.0, 0.0), 0.0);
    let mut level = test_level("minecraft:meadow", &tables);
    let mut rng = LegacyRandom::new(29);
    let mut cx = Ctx {
        level: &mut level,
        random: &mut rng,
        difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
        created: Vec::new(),
        jockey_rider: None,
    };
    finalize::finalize(&mut cx, &mut hen, Reason::Natural, None);
    assert_eq!(hen.extra.chicken_variant, Some(golden));
    assert_eq!(hen.extra.chicken_sound_variant, Some(cluck));

    // Save and reload round-trip.
    let saved = save_mob(&hen, &tables);
    let types = HashMap::from([("minecraft:chicken".to_owned(), kind.clone())]);
    let loaded = entity_nbt::mob_from_nbt_with_variants(&saved, &types, 2, None, &tables).unwrap();
    assert_eq!(loaded.extra.chicken_variant, Some(golden));
    assert_eq!(loaded.extra.chicken_sound_variant, Some(cluck));

    // Mod API `LootEntityView::chicken_variant` returns the pack id.
    let loot_entity = drops::loot_entity_with_variants(&loaded, None, Some(&tables));
    assert_eq!(
        loot_entity.chicken_variant.as_deref(),
        Some("pack:golden_hen")
    );
    let params = mod_api::LootParamsView::new(
        mod_api::__private::LootParamsParts {
            entity: Some(&loot_entity),
            ..Default::default()
        },
        |_| Ok(mod_api::Text::literal("")),
    );
    assert_eq!(
        params.entity().and_then(|e| e.chicken_variant()),
        Some("pack:golden_hen")
    );

    // Thrown egg preserves the pack chicken variant id.
    let mut egg_stack = ItemStack::new("minecraft:egg", 1);
    egg_stack.data_mut().set_other(
        "minecraft:chicken/variant",
        Some(Tag::String("pack:golden_hen".into())),
    );
    let mut egg = Projectile::new(ProjectileKind::Egg, Vec3::ZERO, None);
    egg.set_egg_item(&egg_stack);
    assert_eq!(egg.egg_variant.as_deref(), Some("pack:golden_hen"));
    assert_eq!(
        egg.egg_variant
            .as_deref()
            .and_then(|id| tables.chicken.lookup(id)),
        Some(golden)
    );

    // Client synced table resolves cold model and pack texture.
    let client_tables = sync_tables(
        &packed,
        &[
            "minecraft:chicken_variant",
            "minecraft:chicken_sound_variant",
        ],
    );
    let mut remote = Mob::new(3, kind, Vec3::ZERO, 0.0);
    remote.extra.chicken_variant = Some(RegistryRef::new(golden.network_id()));
    let chicken_entry = remote
        .chicken_variant(&client_tables.chicken)
        .and_then(|r| client_tables.chicken.get(r))
        .unwrap();
    assert_eq!(
        chicken_entry.model_key(remote.is_baby()),
        "cold_chicken#main"
    );
    assert_eq!(
        chicken_entry.texture(remote.is_baby()),
        "pack:entity/chicken/golden_hen"
    );
    remote.extra.age = -24000;
    assert_eq!(
        chicken_entry.model_key(remote.is_baby()),
        "chicken_baby#main"
    );
    assert_eq!(
        chicken_entry.texture(remote.is_baby()),
        "pack:entity/chicken/golden_hen_baby"
    );
}
