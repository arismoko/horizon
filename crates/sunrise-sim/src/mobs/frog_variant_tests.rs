//! Data-pack frog variants: spawn selection, tadpole growth conversion, save round trip,
//! entity-data network id -> client texture resolution, and loot view reporting.
use super::*;
use crate::mobs::variants::{FrogVariant, VariantTables};
use crate::{
    enchant::Enchantments,
    entity_nbt::{self, EntityTagWriter, MobSave},
    items::Items,
    lan::data::{PackRegistryDefinitions, RegistryRef, SyncData},
    mobs::{
        Mob, MobType, Species, drops,
        finalize::{self, Ctx, FinalizeLevel, SpawnData},
        spawn_rules::Reason,
        spawning::LocalDifficulty,
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

fn frog_type() -> Arc<MobType> {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let k = Arc::make_mut(&mut kind);
    k.id = "minecraft:frog".into();
    k.species = Species::Frog;
    kind
}

fn frog_variant_json(asset_id: &str, spawn_conditions: Value) -> Vec<u8> {
    json!({
        "asset_id": asset_id,
        "spawn_conditions": spawn_conditions,
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

/// (1) A pack frog variant spawns where its condition applies, and a tadpole
/// grown there (`Tadpole.ageUp` -> `Frog.finalizeSpawn(Conversion)`) becomes it.
#[test]
fn pack_frog_variant_spawns_and_tadpole_growth_becomes_it() {
    let files = [
        (
            "frog_variant/minecraft:temperate".to_owned(),
            vec![frog_variant_json(
                "minecraft:entity/frog/frog_temperate",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "frog_variant/test:glow".to_owned(),
            vec![frog_variant_json(
                "test:entity/frog/glow",
                json!([{"priority": 1, "condition": {"type": "minecraft:biome", "biomes": "minecraft:lush_caves"}}]),
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let variants = packed.variant_tables();
    let glow = variants.frog.lookup("test:glow").unwrap();
    let temperate = variants.frog.lookup("minecraft:temperate").unwrap();

    // Natural spawn in lush_caves selects `test:glow`.
    let mut level = test_level("minecraft:lush_caves", &variants);
    let mut random = LegacyRandom::new(42);
    let mut frog = Mob::new(1, frog_type(), Vec3::new(0.5, 64.0, 0.5), 0.0);
    let mut cx = Ctx {
        level: &mut level,
        random: &mut random,
        difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
        created: Vec::new(),
        jockey_rider: None,
    };
    finalize::finalize(&mut cx, &mut frog, Reason::Natural, None);
    assert_eq!(frog.extra.frog_variant, Some(glow));
    assert_eq!(variants.frog.id(glow), Some("test:glow"));

    // Tadpole growth (`Tadpole.ageUp` -> `Frog.finalizeSpawn` with `Conversion`)
    // in lush_caves becomes `test:glow` too.
    let mut grown = Mob::new(2, frog_type(), Vec3::new(0.5, 64.0, 0.5), 0.0);
    finalize::finalize(&mut cx, &mut grown, Reason::Conversion, None);
    assert_eq!(grown.extra.frog_variant, Some(glow));

    // Outside lush_caves, fallback priority 0 picks `minecraft:temperate`.
    let mut plains = test_level("minecraft:plains", &variants);
    let mut plain_frog = Mob::new(3, frog_type(), Vec3::new(0.5, 64.0, 0.5), 0.0);
    let mut plains_cx = Ctx {
        level: &mut plains,
        random: &mut random,
        difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
        created: Vec::new(),
        jockey_rider: None,
    };
    finalize::finalize(&mut plains_cx, &mut plain_frog, Reason::Conversion, None);
    assert_eq!(plain_frog.extra.frog_variant, Some(temperate));
}

/// (2) Save round trip keeps `"test:glow"`, and loading an unknown variant id
/// behaves as vanilla's `VariantUtils.readVariant` (falls back to `minecraft:temperate`).
#[test]
fn frog_variant_save_round_trip_keeps_pack_id_and_unknown_falls_back_to_default() {
    let files = [
        (
            "frog_variant/minecraft:temperate".to_owned(),
            vec![frog_variant_json(
                "minecraft:entity/frog/frog_temperate",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "frog_variant/test:glow".to_owned(),
            vec![frog_variant_json(
                "test:entity/frog/glow",
                json!([{"priority": 1}]),
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let variants = packed.variant_tables();
    let glow = variants.frog.lookup("test:glow").unwrap();
    let temperate = variants.frog.lookup("minecraft:temperate").unwrap();
    let kind = frog_type();
    let mut types = HashMap::new();
    types.insert("minecraft:frog".to_owned(), kind.clone());
    let mut frog = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    frog.extra.frog_variant = Some(glow);

    let mut writer = EntityTagWriter::from_saved(Tag::compound(Vec::<(&str, Tag)>::new()));
    entity_nbt::write_mob(
        &frog,
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
        Some(&Tag::String(JavaString::from("test:glow")))
    );

    let loaded =
        entity_nbt::mob_from_nbt_with_variants(&saved, &types, 2, None, &variants).unwrap();
    assert_eq!(loaded.extra.frog_variant, Some(glow));

    let mut unknown = saved;
    if let Tag::Compound(map) = &mut unknown {
        map.insert("variant".into(), Tag::String(JavaString::from("test:gone")));
    }
    let fallback =
        entity_nbt::mob_from_nbt_with_variants(&unknown, &types, 3, None, &variants).unwrap();
    assert_eq!(fallback.extra.frog_variant, Some(temperate));
}

/// (3) Entity data carries the network id and a client given the synced entries
/// resolves `test:glow`'s `asset_id` in the server's sync order, with no fallback
/// to a built-in variant when an id is out of range.
#[test]
fn synced_frog_variant_network_id_resolves_to_pack_asset_id_on_client() {
    let files = [
        (
            "frog_variant/minecraft:temperate".to_owned(),
            vec![frog_variant_json(
                "minecraft:entity/frog/frog_temperate",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "frog_variant/test:glow".to_owned(),
            vec![frog_variant_json(
                "test:entity/frog/glow",
                json!([{"priority": 1}]),
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let server_variants = packed.variant_tables();
    let glow = server_variants.frog.lookup("test:glow").unwrap();
    let temperate = server_variants.frog.lookup("minecraft:temperate").unwrap();

    let mut wire = Vec::new();
    packed.append_encoded("minecraft:frog_variant", &mut wire);
    wire.sort_by_cached_key(|((ns, path), ..)| (format!("{path}.json"), ns.clone()));
    let mut dynamic = Dynamic::default();
    dynamic.add(RegistryData {
        registry: "minecraft:frog_variant".to_owned(),
        entries: wire
            .into_iter()
            .map(|((ns, path), data, _)| RegistryEntry {
                id: format!("{ns}:{path}"),
                data: Some(data),
            })
            .collect(),
    });
    let client_variants = VariantTables::from_synced(&dynamic, &VariantTables::default()).unwrap();

    let mut client_frog = Mob::new(1, frog_type(), Vec3::ZERO, 0.0);
    let resolve = |mob: &Mob| {
        mob.frog_variant(&client_variants.frog)
            .and_then(|r| client_variants.frog.get(r))
            .map(FrogVariant::texture)
            .unwrap_or("")
    };
    client_frog.extra.frog_variant = Some(RegistryRef::new(glow.network_id()));
    assert_eq!(resolve(&client_frog), "test:entity/frog/glow");

    client_frog.extra.frog_variant = Some(RegistryRef::new(temperate.network_id()));
    assert_eq!(resolve(&client_frog), "entity/frog/frog_temperate");

    // Out-of-range id never falls back to a built-in frog texture.
    client_frog.extra.frog_variant = Some(RegistryRef::new(99));
    assert_eq!(resolve(&client_frog), "");
}

/// (4) `LootEntityView::frog_variant` returns the pack variant's id from the
/// world registry (and the default variant id when unset).
#[test]
fn loot_view_returns_pack_frog_variant_id() {
    let files = [
        (
            "frog_variant/minecraft:temperate".to_owned(),
            vec![frog_variant_json(
                "minecraft:entity/frog/frog_temperate",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "frog_variant/test:glow".to_owned(),
            vec![frog_variant_json(
                "test:entity/frog/glow",
                json!([{"priority": 1}]),
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let variants = packed.variant_tables();
    let glow = variants.frog.lookup("test:glow").unwrap();

    let mut frog = Mob::new(1, frog_type(), Vec3::ZERO, 0.0);
    frog.extra.frog_variant = Some(glow);
    let entity = drops::loot_entity_with_variants(&frog, None, Some(&variants));
    assert_eq!(entity.frog_variant.as_deref(), Some("test:glow"));
    assert_eq!(
        mod_api::__private::LootEntityAccess::frog_variant(&entity),
        Some("test:glow")
    );

    // Unset frog variant resolves to `minecraft:temperate`.
    let default_frog = Mob::new(2, frog_type(), Vec3::ZERO, 0.0);
    let default_entity = drops::loot_entity_with_variants(&default_frog, None, Some(&variants));
    assert_eq!(
        mod_api::__private::LootEntityAccess::frog_variant(&default_entity),
        Some("minecraft:temperate")
    );
}
