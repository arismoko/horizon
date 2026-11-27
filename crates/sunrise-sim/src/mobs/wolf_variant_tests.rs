//! Data-pack wolf variants and wolf sound variants: spawn selection, save round trip,
//! entity-data network id -> client texture resolution, and bark sound playback.
use super::*;
use crate::mobs::variants::VariantTables;
use crate::{
    enchant::Enchantments,
    entity_nbt::{self, EntityTagWriter, MobSave},
    items::Items,
    lan::data::{PackRegistryDefinitions, RegistryRef, SyncData},
    mobs::{
        Damage, DamageKind, Mob, MobEvent, MobType, Species, ai,
        finalize::{self, Ctx, FinalizeLevel, SpawnData},
        spawn_rules::Reason,
        spawning::LocalDifficulty,
        tame,
    },
};
use serde_json::json;
use std::{collections::HashMap, sync::Arc};
use sunrise_core::{
    geometry::{Aabb, Vec3},
    nbt::{JavaString, Tag},
    random::LegacyRandom,
};
use sunrise_protocol::{
    packets::configuration::{RegistryData, RegistryEntry},
    registry::Dynamic,
};
use sunrise_world::loot::PackOverlay;

fn wolf_type() -> Arc<MobType> {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let k = Arc::make_mut(&mut kind);
    k.id = "minecraft:wolf".into();
    k.species = Species::Wolf;
    kind
}

fn wolf_variant_json(prefix: &str, spawn_conditions: Value) -> Vec<u8> {
    json!({
        "assets": {
            "wild": prefix,
            "tame": format!("{prefix}_tame"),
            "angry": format!("{prefix}_angry"),
        },
        "baby_assets": {
            "wild": format!("{prefix}_baby"),
            "tame": format!("{prefix}_tame_baby"),
            "angry": format!("{prefix}_angry_baby"),
        },
        "spawn_conditions": spawn_conditions,
    })
    .to_string()
    .into_bytes()
}

fn wolf_sound_json(adult_prefix: &str, baby_prefix: &str) -> Vec<u8> {
    let set = |p: &str| {
        json!({
            "ambient_sound": {"sound_id": format!("{p}.bark")},
            "death_sound": {"sound_id": format!("{p}.death")},
            "growl_sound": {"sound_id": format!("{p}.growl")},
            "hurt_sound": {"sound_id": format!("{p}.hurt")},
            "pant_sound": {"sound_id": format!("{p}.pant")},
            "whine_sound": {"sound_id": format!("{p}.whine")},
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

/// (1) A pack adds `test:mint` wolf variant whose biome spawn condition has higher
/// priority than vanilla's fallback: a wolf spawned in that biome gets `test:mint`.
#[test]
fn pack_wolf_variant_with_higher_priority_biome_condition_is_selected_on_spawn() {
    let files = [
        (
            "wolf_variant/minecraft:pale".to_owned(),
            vec![wolf_variant_json(
                "minecraft:entity/wolf/wolf",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "wolf_variant/test:mint".to_owned(),
            vec![wolf_variant_json(
                "test:entity/wolf/mint",
                json!([{"priority": 5, "condition": {"type": "minecraft:biome", "biomes": "minecraft:plains"}}]),
            )],
        ),
        (
            "wolf_sound_variant/minecraft:classic".to_owned(),
            vec![wolf_sound_json(
                "minecraft:entity.wolf",
                "minecraft:entity.baby_wolf",
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let variants = packed.variant_tables();
    let mint = variants.wolf.lookup("test:mint").unwrap();
    let mut level = test_level("minecraft:plains", &variants);
    let mut random = LegacyRandom::new(42);
    let mut wolf = Mob::new(1, wolf_type(), Vec3::new(0.5, 64.0, 0.5), 0.0);
    let mut cx = Ctx {
        level: &mut level,
        random: &mut random,
        difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
        created: Vec::new(),
        jockey_rider: None,
    };
    let group = finalize::finalize(&mut cx, &mut wolf, Reason::Natural, None);
    assert_eq!(wolf.extra.wolf_variant, Some(mint));
    assert_eq!(variants.wolf.id(mint), Some("test:mint"));
    assert!(matches!(group, Some(finalize::Group::Wolf(_, v)) if v == mint));
}

/// (2) Save round trip keeps `"test:mint"`, and loading an unknown variant id falls
/// back to `VariantUtils.getDefaultOrAny` (`minecraft:pale`).
#[test]
fn wolf_variant_save_round_trip_keeps_pack_id_and_unknown_falls_back_to_default() {
    let files = [
        (
            "wolf_variant/minecraft:pale".to_owned(),
            vec![wolf_variant_json(
                "minecraft:entity/wolf/wolf",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "wolf_variant/test:mint".to_owned(),
            vec![wolf_variant_json(
                "test:entity/wolf/mint",
                json!([{"priority": 5}]),
            )],
        ),
        (
            "wolf_sound_variant/minecraft:classic".to_owned(),
            vec![wolf_sound_json(
                "minecraft:entity.wolf",
                "minecraft:entity.baby_wolf",
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let variants = packed.variant_tables();
    let mint = variants.wolf.lookup("test:mint").unwrap();
    let pale = variants.wolf.lookup("minecraft:pale").unwrap();
    let kind = wolf_type();
    let mut types = HashMap::new();
    types.insert("minecraft:wolf".to_owned(), kind.clone());
    let mut wolf = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    wolf.extra.wolf_variant = Some(mint);

    let mut writer = EntityTagWriter::from_saved(Tag::compound(Vec::<(&str, Tag)>::new()));
    entity_nbt::write_mob(
        &wolf,
        [1, 2, 3, 4],
        &mut writer,
        None,
        |_| None,
        &Items::empty(),
        MobSave {
            variants: Some(&variants),
            ..Default::default()
        },
    );
    let saved = writer.finish();
    assert_eq!(
        saved.get("variant"),
        Some(&Tag::String(JavaString::from("test:mint")))
    );

    let loaded =
        entity_nbt::mob_from_nbt_with_variants(&saved, &types, 2, None, &variants).unwrap();
    assert_eq!(loaded.extra.wolf_variant, Some(mint));

    // Unknown id in save behaves as vanilla's `VariantUtils.readVariant`: keeps default (`pale`).
    let mut unknown = saved;
    if let Tag::Compound(map) = &mut unknown {
        map.insert("variant".into(), Tag::String(JavaString::from("test:gone")));
    }
    let fallback =
        entity_nbt::mob_from_nbt_with_variants(&unknown, &types, 3, None, &variants).unwrap();
    assert_eq!(fallback.extra.wolf_variant, Some(pale));
}

/// (3) Entity data carries the network id and a client given the synced entries
/// resolves `test:mint`'s asset id in the server's sync order.
#[test]
fn synced_wolf_variant_network_id_resolves_to_pack_asset_id_on_client() {
    let files = [
        (
            "wolf_variant/minecraft:pale".to_owned(),
            vec![wolf_variant_json(
                "minecraft:entity/wolf/wolf",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "wolf_variant/test:mint".to_owned(),
            vec![wolf_variant_json(
                "test:entity/wolf/mint",
                json!([{"priority": 5}]),
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let server_variants = packed.variant_tables();
    let mint = server_variants.wolf.lookup("test:mint").unwrap();

    // Build the exact RegistryData packet entries in `file_order`, matching `SyncData`.
    let mut wire = Vec::new();
    packed.append_encoded("minecraft:wolf_variant", &mut wire);
    wire.sort_by_cached_key(|((ns, path), ..)| (format!("{path}.json"), ns.clone()));
    let mut dynamic = Dynamic::default();
    dynamic.add(RegistryData {
        registry: "minecraft:wolf_variant".to_owned(),
        entries: wire
            .into_iter()
            .map(|((ns, path), data, _)| RegistryEntry {
                id: format!("{ns}:{path}"),
                data: Some(data),
            })
            .collect(),
    });
    let client_variants = VariantTables::from_synced(&dynamic, &VariantTables::default()).unwrap();

    // Entity data carries `mint.network_id()` directly.
    let mut client_wolf = Mob::new(1, wolf_type(), Vec3::ZERO, 0.0);
    client_wolf.extra.wolf_variant = Some(RegistryRef::new(mint.network_id()));
    assert_eq!(
        tame::texture(&client_wolf, Some(&client_variants)),
        "test:entity/wolf/mint"
    );
    client_wolf.extra.tame = true;
    assert_eq!(
        tame::texture(&client_wolf, Some(&client_variants)),
        "test:entity/wolf/mint_tame"
    );
    client_wolf.extra.age = -24000;
    assert_eq!(
        tame::texture(&client_wolf, Some(&client_variants)),
        "test:entity/wolf/mint_tame_baby"
    );
    // Out-of-range id never falls back to a built-in wolf texture.
    client_wolf.extra.wolf_variant = Some(RegistryRef::new(99));
    assert_eq!(tame::texture(&client_wolf, Some(&client_variants)), "");
}

/// A malformed synced wolf variant (no `baby_assets`) refuses the registry data.
#[test]
fn malformed_synced_wolf_variant_refuses_the_registry_data() {
    let asset = |name: &str| {
        Tag::compound([
            ("wild", Tag::String(JavaString::from(name))),
            ("tame", Tag::String(JavaString::from(name))),
            ("angry", Tag::String(JavaString::from(name))),
        ])
    };
    let mut dynamic = Dynamic::default();
    dynamic.add(RegistryData {
        registry: "minecraft:wolf_variant".to_owned(),
        entries: vec![RegistryEntry {
            id: "test:bad".to_owned(),
            data: Some(Tag::compound([("assets", asset("test:entity/wolf/bad"))])),
        }],
    });
    let error = VariantTables::from_synced(&dynamic, &VariantTables::default()).unwrap_err();
    assert!(error.contains("minecraft:wolf_variant/test:bad"), "{error}");
}

/// (4) A pack wolf sound variant is chosen on spawn and its sounds are played for bark.
#[test]
fn pack_wolf_sound_variant_is_chosen_and_played_for_bark() {
    let files = [
        (
            "wolf_variant/minecraft:pale".to_owned(),
            vec![wolf_variant_json(
                "minecraft:entity/wolf/wolf",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "wolf_sound_variant/test:husky".to_owned(),
            vec![wolf_sound_json("test:wolf.husky", "test:wolf.husky_baby")],
        ),
    ];
    let packed = stage_pack(&files);
    let variants = packed.variant_tables();
    let husky = variants.wolf_sound.lookup("test:husky").unwrap();

    let mut level = test_level("minecraft:plains", &variants);
    let mut random = LegacyRandom::new(7);
    let mut wolf = Mob::new(1, wolf_type(), Vec3::new(0.5, 64.0, 0.5), 0.0);
    let mut cx = Ctx {
        level: &mut level,
        random: &mut random,
        difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
        created: Vec::new(),
        jockey_rider: None,
    };
    finalize::finalize(&mut cx, &mut wolf, Reason::Natural, None);
    assert_eq!(wolf.extra.wolf_sound_variant, Some(husky));

    // Find a seed whose `nextInt(3) != 0` so `Wolf.getAmbientSound` picks bark (`ambient_sound`).
    let mut bark_played = false;
    for seed in 0..16i64 {
        wolf.random = LegacyRandom::new(seed);
        let mut events = Vec::new();
        ai::play_wolf_ambient(&mut wolf, Some(&variants), &mut events);
        if events
            .iter()
            .any(|e| matches!(e, MobEvent::SoundId(id, ..) if id == "test:wolf.husky.bark"))
        {
            bark_played = true;
            break;
        }
    }
    assert!(
        bark_played,
        "wolf should play the pack sound variant's bark"
    );

    // Wolf.getHurtSound and Wolf.getDeathSound read the world's sound variant table.
    let mut hurt_wolf = wolf.clone();
    let mut dmg = crate::health::TestLevel::new(0);
    assert!(hurt_wolf.damage(
        &mut dmg.level().with_variants(&variants),
        1.0,
        Damage::of(DamageKind::Generic),
    ));
    assert!(
        hurt_wolf
            .outbox
            .iter()
            .any(|e| matches!(e, MobEvent::SoundId(id, ..) if id == "test:wolf.husky.hurt")),
        "adult wolf hurt should play the pack sound variant's hurt sound: {:?}",
        hurt_wolf.outbox
    );

    let mut baby_death_wolf = wolf.clone();
    baby_death_wolf.extra.age = -24000;
    let mut dmg = crate::health::TestLevel::new(0);
    assert!(baby_death_wolf.damage(
        &mut dmg.level().with_variants(&variants),
        1000.0,
        Damage::of(DamageKind::Generic),
    ));
    assert!(
        baby_death_wolf
            .outbox
            .iter()
            .any(|e| matches!(e, MobEvent::SoundId(id, ..) if id == "test:wolf.husky_baby.death")),
        "baby wolf death should play the baby pack sound variant's death sound: {:?}",
        baby_death_wolf.outbox
    );
}

/// `Wolf.getAmbientSound`: a tame wolf under 20 health whines, whatever its maximum health.
#[test]
fn tame_wolf_under_twenty_health_whines() {
    let files = [(
        "wolf_sound_variant/test:husky".to_owned(),
        vec![wolf_sound_json("test:wolf.husky", "test:wolf.husky_baby")],
    )];
    let packed = stage_pack(&files);
    let variants = packed.variant_tables();
    let mut wolf = Mob::new(1, wolf_type(), Vec3::new(0.5, 64.0, 0.5), 0.0);
    wolf.extra.wolf_sound_variant = variants.wolf_sound.lookup("test:husky");
    wolf.extra.tame = true;
    wolf.attrs.max_health = 10.0;
    wolf.health = 10.0;
    let mut whined = false;
    for seed in 0..16i64 {
        wolf.random = LegacyRandom::new(seed);
        let mut events = Vec::new();
        ai::play_wolf_ambient(&mut wolf, Some(&variants), &mut events);
        for event in &events {
            if let MobEvent::SoundId(id, ..) = event {
                assert_ne!(id, "test:wolf.husky.pant", "seed {seed}");
                whined |= id == "test:wolf.husky.whine";
            }
        }
    }
    assert!(whined, "a tame wolf at 10 health should whine");
}
