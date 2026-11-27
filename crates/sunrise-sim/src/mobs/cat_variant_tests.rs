//! Data-pack cat variants and cat sound variants: spawn selection, save round trip,
//! entity-data network id -> client texture resolution, and ambient sound playback.
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

fn cat_type() -> Arc<MobType> {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let k = Arc::make_mut(&mut kind);
    k.id = "minecraft:cat".into();
    k.species = Species::Cat;
    kind
}

fn cat_variant_json(asset_id: &str, baby_asset_id: &str, spawn_conditions: Value) -> Vec<u8> {
    json!({
        "asset_id": asset_id,
        "baby_asset_id": baby_asset_id,
        "spawn_conditions": spawn_conditions,
    })
    .to_string()
    .into_bytes()
}

fn cat_sound_json(adult_prefix: &str, baby_prefix: &str) -> Vec<u8> {
    let set = |p: &str| {
        json!({
            "ambient_sound": {"sound_id": format!("{p}.ambient")},
            "stray_ambient_sound": {"sound_id": format!("{p}.stray_ambient")},
            "hiss_sound": {"sound_id": format!("{p}.hiss")},
            "hurt_sound": {"sound_id": format!("{p}.hurt")},
            "death_sound": {"sound_id": format!("{p}.death")},
            "eat_sound": {"sound_id": format!("{p}.eat")},
            "beg_for_food_sound": {"sound_id": format!("{p}.beg_for_food")},
            "purr_sound": {"sound_id": format!("{p}.purr")},
            "purreow_sound": {"sound_id": format!("{p}.purreow")},
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
    structures: Vec<String>,
    black_cat_structures: Vec<&'a str>,
    moon_phase: usize,
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
        self.structures.clone()
    }
    fn structure_tag(&self, tag: &str, structure: &str) -> bool {
        tag == "minecraft:cats_spawn_as_black" && self.black_cat_structures.contains(&structure)
    }
    fn moon_phase(&self, _: [i32; 3]) -> usize {
        self.moon_phase
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
        structures: Vec::new(),
        black_cat_structures: vec!["minecraft:swamp_hut"],
        moon_phase: 4,
        variants,
        enchantments: Enchantments::default(),
        items: Items::empty(),
    }
}

/// (1) A pack cat variant with a structure spawn condition wins in that structure
/// (including structure spawns such as witch huts), while fallback wins elsewhere.
#[test]
fn pack_cat_variant_with_spawn_condition_wins_where_it_applies() {
    let files = [
        (
            "cat_variant/minecraft:black".to_owned(),
            vec![cat_variant_json(
                "minecraft:entity/cat/cat_black",
                "minecraft:entity/cat/cat_black_baby",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "cat_variant/test:midnight".to_owned(),
            vec![cat_variant_json(
                "test:entity/cat/midnight",
                "test:entity/cat/midnight_baby",
                json!([{"priority": 2, "condition": {"type": "minecraft:structure", "structures": "#minecraft:cats_spawn_as_black"}}]),
            )],
        ),
        (
            "cat_sound_variant/minecraft:classic".to_owned(),
            vec![cat_sound_json(
                "minecraft:entity.cat",
                "minecraft:entity.baby_cat",
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let variants = packed.variant_tables();
    let midnight = variants.cat.lookup("test:midnight").unwrap();
    let black = variants.cat.lookup("minecraft:black").unwrap();

    // In a swamp hut (StructureReason): `test:midnight` wins at priority 2.
    let mut hut_level = test_level("minecraft:swamp", &variants);
    hut_level.structures = vec!["minecraft:swamp_hut".to_owned()];
    let mut random = LegacyRandom::new(42);
    let mut hut_cat = Mob::new(1, cat_type(), Vec3::new(0.5, 64.0, 0.5), 0.0);
    let mut cx = Ctx {
        level: &mut hut_level,
        random: &mut random,
        difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
        created: Vec::new(),
        jockey_rider: None,
    };
    finalize::finalize(&mut cx, &mut hut_cat, Reason::Structure, None);
    assert_eq!(hut_cat.extra.cat_variant, Some(midnight));
    assert_eq!(variants.cat.id(midnight), Some("test:midnight"));

    // Outside the structure: the priority-0 fallback (`minecraft:black`) wins.
    let mut plains_level = test_level("minecraft:plains", &variants);
    let mut plains_cat = Mob::new(2, cat_type(), Vec3::new(0.5, 64.0, 0.5), 0.0);
    let mut cx = Ctx {
        level: &mut plains_level,
        random: &mut random,
        difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
        created: Vec::new(),
        jockey_rider: None,
    };
    finalize::finalize(&mut cx, &mut plains_cat, Reason::Natural, None);
    assert_eq!(plains_cat.extra.cat_variant, Some(black));
}

/// (2) Saving a cat with a pack variant and sound variant writes their entry ids;
/// reloading resolves the same holders, and an unknown id falls back to vanilla's
/// default (`minecraft:black` / `minecraft:classic`).
#[test]
fn pack_cat_variant_save_round_trip_and_unknown_id_fallback() {
    let files = [
        (
            "cat_variant/minecraft:black".to_owned(),
            vec![cat_variant_json(
                "minecraft:entity/cat/cat_black",
                "minecraft:entity/cat/cat_black_baby",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "cat_variant/test:midnight".to_owned(),
            vec![cat_variant_json(
                "test:entity/cat/midnight",
                "test:entity/cat/midnight_baby",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "cat_sound_variant/minecraft:classic".to_owned(),
            vec![cat_sound_json(
                "minecraft:entity.cat",
                "minecraft:entity.baby_cat",
            )],
        ),
        (
            "cat_sound_variant/test:lynx".to_owned(),
            vec![cat_sound_json("test:cat.lynx", "test:cat.lynx_baby")],
        ),
    ];
    let packed = stage_pack(&files);
    let variants = packed.variant_tables();
    let midnight = variants.cat.lookup("test:midnight").unwrap();
    let black = variants.cat.lookup("minecraft:black").unwrap();
    let lynx = variants.cat_sound.lookup("test:lynx").unwrap();
    let classic = variants.cat_sound.lookup("minecraft:classic").unwrap();

    let kind = cat_type();
    let mut cat = Mob::new(1, kind.clone(), Vec3::new(1.0, 64.0, 2.0), 0.0);
    cat.extra.cat_variant = Some(midnight);
    cat.extra.cat_sound_variant = Some(lynx);

    let mut out = EntityTagWriter::from_saved(Tag::compound(Vec::<(&str, Tag)>::new()));
    entity_nbt::write_mob(
        &cat,
        [1, 2, 3, 4],
        &mut out,
        None,
        |_| None,
        &Items::empty(),
        MobSave {
            variants: Some(&variants),
            ..Default::default()
        },
    );
    let saved = out.finish();
    assert_eq!(
        saved.get("variant"),
        Some(&Tag::String(JavaString::from("test:midnight")))
    );
    assert_eq!(
        saved.get("sound_variant"),
        Some(&Tag::String(JavaString::from("test:lynx")))
    );

    let types = HashMap::from([("minecraft:cat".to_owned(), kind)]);
    let loaded =
        entity_nbt::mob_from_nbt_with_variants(&saved, &types, 2, None, &variants).unwrap();
    assert_eq!(loaded.extra.cat_variant, Some(midnight));
    assert_eq!(loaded.extra.cat_sound_variant, Some(lynx));

    // Unknown ids on load behave as vanilla's codec: stay at default (`black` / `classic`).
    let mut bad = saved.clone();
    if let Tag::Compound(map) = &mut bad {
        map.insert(
            "variant".into(),
            Tag::String(JavaString::from("test:deleted")),
        );
        map.insert(
            "sound_variant".into(),
            Tag::String(JavaString::from("test:deleted")),
        );
    }
    let fallback =
        entity_nbt::mob_from_nbt_with_variants(&bad, &types, 3, None, &variants).unwrap();
    assert_eq!(fallback.extra.cat_variant, Some(black));
    assert_eq!(fallback.extra.cat_sound_variant, Some(classic));
}

/// (3) Synced `minecraft:cat_variant` reaches the client in the server's network id
/// order, and the client resolves its `asset_id` / `baby_asset_id` without falling
/// back to a built-in variant.
#[test]
fn client_resolves_synced_cat_variant_asset_id_by_network_id() {
    let files = [
        (
            "cat_variant/minecraft:black".to_owned(),
            vec![cat_variant_json(
                "minecraft:entity/cat/cat_black",
                "minecraft:entity/cat/cat_black_baby",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "cat_variant/test:midnight".to_owned(),
            vec![cat_variant_json(
                "test:entity/cat/midnight",
                "test:entity/cat/midnight_baby",
                json!([{"priority": 0}]),
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let server_variants = packed.variant_tables();
    let midnight = server_variants.cat.lookup("test:midnight").unwrap();

    // Sync the server's encoded entries in the server's file_order.
    let mut wire = Vec::new();
    packed.append_encoded("minecraft:cat_variant", &mut wire);
    wire.sort_by_cached_key(|((ns, path), _, _)| (format!("{path}.json"), ns.clone()));
    let mut dynamic = Dynamic::default();
    dynamic.add(RegistryData {
        registry: "minecraft:cat_variant".to_owned(),
        entries: wire
            .into_iter()
            .map(|((ns, path), data, _)| RegistryEntry {
                id: format!("{ns}:{path}"),
                data: Some(data),
            })
            .collect(),
    });
    let client_variants = VariantTables::from_synced(&dynamic, &VariantTables::default()).unwrap();

    // Entity data carries `midnight.network_id()` directly.
    let mut client_cat = Mob::new(1, cat_type(), Vec3::ZERO, 0.0);
    client_cat.extra.cat_variant = Some(RegistryRef::new(midnight.network_id()));
    assert_eq!(
        tame::cat_texture(&client_cat, Some(&client_variants)),
        "test:entity/cat/midnight"
    );
    client_cat.extra.age = -24000;
    assert_eq!(
        tame::cat_texture(&client_cat, Some(&client_variants)),
        "test:entity/cat/midnight_baby"
    );
    // Out-of-range id never falls back to a built-in cat texture.
    client_cat.extra.cat_variant = Some(RegistryRef::new(99));
    assert_eq!(tame::cat_texture(&client_cat, Some(&client_variants)), "");
}

/// (4) A pack cat sound variant is chosen on spawn and its sounds are played for ambient.
#[test]
fn pack_cat_sound_variant_is_chosen_and_played_for_ambient() {
    let files = [
        (
            "cat_variant/minecraft:black".to_owned(),
            vec![cat_variant_json(
                "minecraft:entity/cat/cat_black",
                "minecraft:entity/cat/cat_black_baby",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "cat_sound_variant/test:lynx".to_owned(),
            vec![cat_sound_json("test:cat.lynx", "test:cat.lynx_baby")],
        ),
    ];
    let packed = stage_pack(&files);
    let variants = packed.variant_tables();
    let lynx = variants.cat_sound.lookup("test:lynx").unwrap();

    let mut level = test_level("minecraft:plains", &variants);
    let mut random = LegacyRandom::new(7);
    let mut cat = Mob::new(1, cat_type(), Vec3::new(0.5, 64.0, 0.5), 0.0);
    let mut cx = Ctx {
        level: &mut level,
        random: &mut random,
        difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
        created: Vec::new(),
        jockey_rider: None,
    };
    finalize::finalize(&mut cx, &mut cat, Reason::Natural, None);
    assert_eq!(cat.extra.cat_sound_variant, Some(lynx));

    // Wild cat plays `stray_ambient_sound`.
    let mut events = Vec::new();
    ai::play_cat_ambient(&mut cat, Some(&variants), &mut events);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, MobEvent::SoundId(id, ..) if id == "test:cat.lynx.stray_ambient")),
        "wild cat should play the pack sound variant's stray_ambient"
    );

    // Tame baby cat in love plays baby `purr_sound`.
    cat.extra.tame = true;
    cat.extra.in_love = 100;
    cat.extra.age = -24000;
    events.clear();
    ai::play_cat_ambient(&mut cat, Some(&variants), &mut events);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, MobEvent::SoundId(id, ..) if id == "test:cat.lynx_baby.purr")),
        "tame baby cat in love should play the baby pack sound variant's purr"
    );

    // Cat.getHurtSound and Cat.getDeathSound read the world's sound variant table.
    let mut hurt_cat = cat.clone();
    hurt_cat.extra.age = 0;
    let mut dmg = crate::health::TestLevel::new(0);
    assert!(hurt_cat.damage(
        &mut dmg.level().with_variants(&variants),
        1.0,
        Damage::of(DamageKind::Generic),
    ));
    assert!(
        hurt_cat
            .outbox
            .iter()
            .any(|e| matches!(e, MobEvent::SoundId(id, ..) if id == "test:cat.lynx.hurt")),
        "adult cat hurt should play the pack sound variant's hurt sound: {:?}",
        hurt_cat.outbox
    );

    let mut baby_death_cat = cat.clone();
    baby_death_cat.extra.age = -24000;
    let mut dmg = crate::health::TestLevel::new(0);
    assert!(baby_death_cat.damage(
        &mut dmg.level().with_variants(&variants),
        1000.0,
        Damage::of(DamageKind::Generic),
    ));
    assert!(
        baby_death_cat
            .outbox
            .iter()
            .any(|e| matches!(e, MobEvent::SoundId(id, ..) if id == "test:cat.lynx_baby.death")),
        "baby cat death should play the baby pack sound variant's death sound: {:?}",
        baby_death_cat.outbox
    );
}
