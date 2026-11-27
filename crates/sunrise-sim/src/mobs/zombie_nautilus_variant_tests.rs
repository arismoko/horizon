//! Data-pack zombie nautilus variants: spawn selection, save round trip,
//! entity-data network id -> client texture and model resolution, and codec validation.
use super::*;
use crate::mobs::variants::{VariantTables, ZombieNautilusModelType, ZombieNautilusVariant};
use crate::{
    enchant::Enchantments,
    entity_nbt::{self, EntityTagWriter, MobSave},
    items::Items,
    lan::data::{PackRegistryDefinitions, RegistryRef, SyncData},
    mobs::{
        Mob, MobType, Species,
        finalize::{self, Ctx, FinalizeLevel, SpawnData},
        nautilus,
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

fn zombie_nautilus_type() -> Arc<MobType> {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let k = Arc::make_mut(&mut kind);
    k.id = "minecraft:zombie_nautilus".into();
    k.species = Species::ZombieNautilus;
    kind
}

fn zombie_nautilus_variant_json(
    model: Option<&str>,
    asset_id: &str,
    spawn_conditions: Value,
) -> Vec<u8> {
    let mut obj = serde_json::Map::new();
    if let Some(m) = model {
        obj.insert("model".into(), json!(m));
    }
    obj.insert("asset_id".into(), json!(asset_id));
    obj.insert("spawn_conditions".into(), spawn_conditions);
    Value::Object(obj).to_string().into_bytes()
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

/// A pack zombie nautilus variant spawns where its condition applies, falls back
/// to `minecraft:temperate` elsewhere, round-trips through the save, and an
/// unknown saved id falls back to `VariantUtils.getDefaultOrAny` (`temperate`).
#[test]
fn pack_zombie_nautilus_variant_spawns_where_condition_applies_and_round_trips_save() {
    let files = [
        (
            "zombie_nautilus_variant/minecraft:temperate".to_owned(),
            vec![zombie_nautilus_variant_json(
                None,
                "minecraft:entity/nautilus/zombie_nautilus",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "zombie_nautilus_variant/minecraft:warm".to_owned(),
            vec![zombie_nautilus_variant_json(
                Some("warm"),
                "minecraft:entity/nautilus/zombie_nautilus_coral",
                json!([{"priority": 1, "condition": {"type": "minecraft:biome", "biomes": "minecraft:warm_ocean"}}]),
            )],
        ),
        (
            "zombie_nautilus_variant/test:abyssal".to_owned(),
            vec![zombie_nautilus_variant_json(
                Some("warm"),
                "test:entity/nautilus/abyssal",
                json!([{"priority": 5, "condition": {"type": "minecraft:biome", "biomes": "minecraft:deep_lukewarm_ocean"}}]),
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let variants = packed.variant_tables();
    let abyssal = variants.zombie_nautilus.lookup("test:abyssal").unwrap();
    let temperate = variants
        .zombie_nautilus
        .lookup("minecraft:temperate")
        .unwrap();

    // Condition applies -> selects `test:abyssal`.
    let kind = zombie_nautilus_type();
    let mut mob = Mob::new(1, kind.clone(), Vec3::new(0.5, 40.0, 0.5), 0.0);
    {
        let mut level = test_level("minecraft:deep_lukewarm_ocean", &variants);
        let mut random = LegacyRandom::new(42);
        let mut cx = Ctx {
            level: &mut level,
            random: &mut random,
            difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
            created: Vec::new(),
            jockey_rider: None,
        };
        finalize::finalize(&mut cx, &mut mob, Reason::Natural, None);
    }
    assert_eq!(mob.extra.zombie_nautilus_variant, Some(abyssal));
    assert_eq!(variants.zombie_nautilus.id(abyssal), Some("test:abyssal"));

    // Condition does not apply -> selects fallback `minecraft:temperate`.
    let mut fallback_mob = Mob::new(2, kind.clone(), Vec3::new(0.5, 40.0, 0.5), 0.0);
    {
        let mut level = test_level("minecraft:ocean", &variants);
        let mut random = LegacyRandom::new(42);
        let mut cx = Ctx {
            level: &mut level,
            random: &mut random,
            difficulty: LocalDifficulty::new(2, 0, 0, 0.0),
            created: Vec::new(),
            jockey_rider: None,
        };
        finalize::finalize(&mut cx, &mut fallback_mob, Reason::Natural, None);
    }
    assert_eq!(fallback_mob.extra.zombie_nautilus_variant, Some(temperate));

    // Save round trip keeps `"test:abyssal"`.
    let mut writer = EntityTagWriter::from_saved(Tag::compound(Vec::<(&str, Tag)>::new()));
    entity_nbt::write_mob(
        &mob,
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
        Some(&Tag::String(JavaString::from("test:abyssal")))
    );

    let mut types = HashMap::new();
    types.insert("minecraft:zombie_nautilus".to_owned(), kind);
    let loaded =
        entity_nbt::mob_from_nbt_with_variants(&saved, &types, 3, None, &variants).unwrap();
    assert_eq!(loaded.extra.zombie_nautilus_variant, Some(abyssal));

    // Unknown id in save behaves as vanilla's `VariantUtils.readVariant`: keeps default (`temperate`).
    let mut unknown = saved;
    if let Tag::Compound(map) = &mut unknown {
        map.insert("variant".into(), Tag::String(JavaString::from("test:gone")));
    }
    let unknown_loaded =
        entity_nbt::mob_from_nbt_with_variants(&unknown, &types, 4, None, &variants).unwrap();
    assert_eq!(
        unknown_loaded.extra.zombie_nautilus_variant,
        Some(temperate)
    );
}

/// Entity data carries the network id and a client given the synced entries
/// resolves the pack variant's asset id and model in the server's sync order.
#[test]
fn synced_zombie_nautilus_variant_network_id_resolves_texture_and_model_on_client() {
    let files = [
        (
            "zombie_nautilus_variant/minecraft:temperate".to_owned(),
            vec![zombie_nautilus_variant_json(
                None,
                "minecraft:entity/nautilus/zombie_nautilus",
                json!([{"priority": 0}]),
            )],
        ),
        (
            "zombie_nautilus_variant/test:abyssal".to_owned(),
            vec![zombie_nautilus_variant_json(
                Some("warm"),
                "test:entity/nautilus/abyssal",
                json!([{"priority": 5}]),
            )],
        ),
    ];
    let packed = stage_pack(&files);
    let server_variants = packed.variant_tables();
    let abyssal = server_variants
        .zombie_nautilus
        .lookup("test:abyssal")
        .unwrap();
    let temperate = server_variants
        .zombie_nautilus
        .lookup("minecraft:temperate")
        .unwrap();

    // Build the exact RegistryData packet entries in `file_order`, matching `SyncData`.
    let mut wire = Vec::new();
    packed.append_encoded("minecraft:zombie_nautilus_variant", &mut wire);
    wire.sort_by_cached_key(|((ns, path), ..)| (format!("{path}.json"), ns.clone()));
    let mut dynamic = Dynamic::default();
    dynamic.add(RegistryData {
        registry: "minecraft:zombie_nautilus_variant".to_owned(),
        entries: wire
            .into_iter()
            .map(|((ns, path), data, _)| RegistryEntry {
                id: format!("{ns}:{path}"),
                data: Some(data),
            })
            .collect(),
    });
    let client_variants = VariantTables::from_synced(&dynamic, &VariantTables::default()).unwrap();

    let mut client_mob = Mob::new(1, zombie_nautilus_type(), Vec3::ZERO, 0.0);
    client_mob.extra.zombie_nautilus_variant = Some(RegistryRef::new(abyssal.network_id()));
    assert_eq!(
        nautilus::texture(&client_mob, Some(&client_variants)),
        "test:entity/nautilus/abyssal"
    );
    assert_eq!(
        nautilus::model(&client_mob, Some(&client_variants), false),
        "zombie_nautilus_coral#main"
    );

    client_mob.extra.zombie_nautilus_variant = Some(RegistryRef::new(temperate.network_id()));
    assert_eq!(
        nautilus::texture(&client_mob, Some(&client_variants)),
        "entity/nautilus/zombie_nautilus"
    );
    assert_eq!(
        nautilus::model(&client_mob, Some(&client_variants), false),
        "zombie_nautilus#main"
    );

    // Out-of-range id never falls back to a built-in texture.
    client_mob.extra.zombie_nautilus_variant = Some(RegistryRef::new(99));
    assert_eq!(nautilus::texture(&client_mob, Some(&client_variants)), "");
}

/// `ZombieNautilusVariant.DIRECT_CODEC` validates `model`, `asset_id` and `spawn_conditions`.
#[test]
fn zombie_nautilus_variant_codec_validates_model_and_required_fields() {
    let default_model = ZombieNautilusVariant::decode(&json!({
        "asset_id": "minecraft:entity/nautilus/zombie_nautilus",
        "spawn_conditions": [{"priority": 0}],
    }))
    .unwrap();
    assert_eq!(default_model.model, ZombieNautilusModelType::Normal);

    let warm = ZombieNautilusVariant::decode(&json!({
        "model": "warm",
        "asset_id": "minecraft:entity/nautilus/zombie_nautilus_coral",
        "spawn_conditions": [{"priority": 0}],
    }))
    .unwrap();
    assert_eq!(warm.model, ZombieNautilusModelType::Warm);

    assert!(
        ZombieNautilusVariant::decode(&json!({
            "model": "cold",
            "asset_id": "minecraft:entity/nautilus/zombie_nautilus",
            "spawn_conditions": [{"priority": 0}],
        }))
        .is_err()
    );
    assert!(
        ZombieNautilusVariant::decode(&json!({
            "spawn_conditions": [{"priority": 0}],
        }))
        .is_err()
    );
    assert!(
        ZombieNautilusVariant::decode(&json!({
            "asset_id": "minecraft:entity/nautilus/zombie_nautilus",
        }))
        .is_err()
    );
}
