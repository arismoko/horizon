//! Vanilla 26.3 `SynchedEntityData` serializer layouts by `minecraft:entity_type` id.

/// Base `Entity` data entries (`0..8`) shared by every entity type.
pub const ENTITY_BASE: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1];

const BOAT: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 1, 1, 3, 8, 8, 1];
const ITEM_PROJECTILE: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 7];
const MINECART: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 1, 1, 3, 15, 1];
const MOB: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0];
const MOB_BYTE: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 0];
const MOB_INT: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 1];
const MOB_BOOL: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8];
const SPELLCASTER: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 0];
const MOB_BOOL_INT: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 1];
const AGEABLE: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8];
const AGEABLE_BYTE: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 0];
const AGEABLE_INT: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 1];
const AGEABLE_BOOL: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 8];
const AGEABLE_BOOL2: &[i32] = &[
    0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 8, 8,
];
const CHESTED_HORSE: &[i32] = &[
    0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 0, 8,
];
const CAMEL: &[i32] = &[
    0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 0, 8, 2,
];
const LLAMA: &[i32] = &[
    0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 0, 8, 1, 1,
];
const ZOMBIE: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 1, 8];
const ITEM_FRAME: &[i32] = &[0, 1, 6, 8, 8, 8, 20, 1, 12, 7, 1];

/// Sorted `(minecraft:entity_type, serializer_ids)` table matching vanilla 26.3 `SynchedEntityData`.
pub(in crate::game) const ENTITY_SERIALIZERS: &[(&str, &[i32])] = &[
    ("minecraft:acacia_boat", BOAT),
    ("minecraft:acacia_chest_boat", BOAT),
    ("minecraft:allay", AGEABLE),
    (
        "minecraft:area_effect_cloud",
        &[0, 1, 6, 8, 8, 8, 20, 1, 3, 8, 16],
    ),
    (
        "minecraft:armadillo",
        &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 36],
    ),
    (
        "minecraft:armor_stand",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 9, 9, 9, 9, 9, 9,
        ],
    ),
    ("minecraft:arrow", &[0, 1, 6, 8, 8, 8, 20, 1, 0, 0, 8, 1]),
    (
        "minecraft:axolotl",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 1, 8, 8,
        ],
    ),
    ("minecraft:bamboo_chest_raft", BOAT),
    ("minecraft:bamboo_raft", BOAT),
    ("minecraft:bat", MOB_BYTE),
    (
        "minecraft:bee",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 0, 2,
        ],
    ),
    ("minecraft:birch_boat", BOAT),
    ("minecraft:birch_chest_boat", BOAT),
    ("minecraft:blaze", MOB_BYTE),
    (
        "minecraft:block_display",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 1, 1, 1, 39, 39, 40, 40, 0, 1, 3, 3, 3, 3, 3, 1, 14,
        ],
    ),
    ("minecraft:bogged", MOB_BOOL),
    ("minecraft:breeze", MOB),
    ("minecraft:breeze_wind_charge", ENTITY_BASE),
    ("minecraft:camel", CAMEL),
    ("minecraft:camel_husk", CAMEL),
    (
        "minecraft:cat",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 0, 13, 21, 8, 8, 1, 22,
        ],
    ),
    ("minecraft:cave_spider", MOB_BYTE),
    ("minecraft:cherry_boat", BOAT),
    ("minecraft:cherry_chest_boat", BOAT),
    ("minecraft:chest_minecart", MINECART),
    (
        "minecraft:chicken",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 30, 31,
        ],
    ),
    ("minecraft:cod", MOB_BOOL),
    (
        "minecraft:command_block_minecart",
        &[0, 1, 6, 8, 8, 8, 20, 1, 1, 1, 3, 15, 1, 4, 5],
    ),
    (
        "minecraft:copper_golem",
        &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 38, 37],
    ),
    (
        "minecraft:cow",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 23, 24,
        ],
    ),
    (
        "minecraft:creaking",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 8, 11,
        ],
    ),
    (
        "minecraft:creeper",
        &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 1, 8, 8],
    ),
    ("minecraft:cushion", &[0, 1, 6, 8, 8, 8, 20, 1, 43]),
    ("minecraft:dark_oak_boat", BOAT),
    ("minecraft:dark_oak_chest_boat", BOAT),
    (
        "minecraft:dolphin",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 8, 1,
        ],
    ),
    ("minecraft:donkey", CHESTED_HORSE),
    ("minecraft:dragon_fireball", ENTITY_BASE),
    ("minecraft:drowned", ZOMBIE),
    ("minecraft:egg", ITEM_PROJECTILE),
    ("minecraft:elder_guardian", MOB_BOOL_INT),
    ("minecraft:end_crystal", &[0, 1, 6, 8, 8, 8, 20, 1, 11, 8]),
    ("minecraft:ender_dragon", MOB_INT),
    ("minecraft:ender_pearl", ITEM_PROJECTILE),
    (
        "minecraft:enderman",
        &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 15, 8, 8],
    ),
    ("minecraft:endermite", MOB),
    ("minecraft:evoker", SPELLCASTER),
    ("minecraft:evoker_fangs", ENTITY_BASE),
    ("minecraft:experience_bottle", ITEM_PROJECTILE),
    ("minecraft:experience_orb", &[0, 1, 6, 8, 8, 8, 20, 1, 1]),
    ("minecraft:eye_of_ender", ITEM_PROJECTILE),
    ("minecraft:falling_block", &[0, 1, 6, 8, 8, 8, 20, 1, 10]),
    ("minecraft:fireball", ITEM_PROJECTILE),
    (
        "minecraft:firework_rocket",
        &[0, 1, 6, 8, 8, 8, 20, 1, 7, 19, 8],
    ),
    ("minecraft:fishing_bobber", &[0, 1, 6, 8, 8, 8, 20, 1, 1, 8]),
    (
        "minecraft:fox",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 1, 0, 13, 13,
        ],
    ),
    (
        "minecraft:frog",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 27, 19,
        ],
    ),
    (
        "minecraft:furnace_minecart",
        &[0, 1, 6, 8, 8, 8, 20, 1, 1, 1, 3, 15, 1, 8],
    ),
    ("minecraft:ghast", MOB_BOOL),
    ("minecraft:giant", MOB),
    ("minecraft:glow_item_frame", ITEM_FRAME),
    ("minecraft:glow_squid", AGEABLE_INT),
    (
        "minecraft:goat",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 8, 8, 8,
        ],
    ),
    ("minecraft:guardian", MOB_BOOL_INT),
    ("minecraft:happy_ghast", AGEABLE_BOOL2),
    ("minecraft:hoglin", AGEABLE_BOOL),
    ("minecraft:hopper_minecart", MINECART),
    (
        "minecraft:horse",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 0, 1,
        ],
    ),
    ("minecraft:husk", ZOMBIE),
    ("minecraft:illusioner", SPELLCASTER),
    ("minecraft:interaction", &[0, 1, 6, 8, 8, 8, 20, 1, 3, 3, 8]),
    ("minecraft:iron_golem", MOB_BYTE),
    ("minecraft:item", ITEM_PROJECTILE),
    (
        "minecraft:item_display",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 1, 1, 1, 39, 39, 40, 40, 0, 1, 3, 3, 3, 3, 3, 1, 7, 0,
        ],
    ),
    ("minecraft:item_frame", ITEM_FRAME),
    ("minecraft:jungle_boat", BOAT),
    ("minecraft:jungle_chest_boat", BOAT),
    ("minecraft:leash_knot", ENTITY_BASE),
    ("minecraft:lightning_bolt", ENTITY_BASE),
    ("minecraft:lingering_potion", ITEM_PROJECTILE),
    ("minecraft:llama", LLAMA),
    ("minecraft:llama_spit", ENTITY_BASE),
    ("minecraft:magma_cube", AGEABLE_INT),
    ("minecraft:mangrove_boat", BOAT),
    ("minecraft:mangrove_chest_boat", BOAT),
    (
        "minecraft:mannequin",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 42, 0, 41, 8, 6,
        ],
    ),
    ("minecraft:marker", ENTITY_BASE),
    ("minecraft:minecart", MINECART),
    ("minecraft:mooshroom", AGEABLE_INT),
    ("minecraft:mule", CHESTED_HORSE),
    (
        "minecraft:nautilus",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 0, 13, 8,
        ],
    ),
    ("minecraft:oak_boat", BOAT),
    ("minecraft:oak_chest_boat", BOAT),
    ("minecraft:ocelot", AGEABLE_BOOL),
    ("minecraft:ominous_item_spawner", ITEM_PROJECTILE),
    ("minecraft:painting", &[0, 1, 6, 8, 8, 8, 20, 1, 12, 34]),
    ("minecraft:pale_oak_boat", BOAT),
    ("minecraft:pale_oak_chest_boat", BOAT),
    (
        "minecraft:panda",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 1, 1, 1, 0, 0, 0,
        ],
    ),
    ("minecraft:parched", MOB),
    (
        "minecraft:parrot",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 0, 13, 1,
        ],
    ),
    ("minecraft:phantom", MOB_INT),
    (
        "minecraft:pig",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 1, 28, 29,
        ],
    ),
    ("minecraft:piglin", AGEABLE_BOOL2),
    ("minecraft:piglin_brute", MOB_BOOL),
    ("minecraft:pillager", AGEABLE),
    (
        "minecraft:player",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 42, 0, 3, 1, 19, 19,
        ],
    ),
    ("minecraft:polar_bear", AGEABLE_BOOL),
    ("minecraft:poplar_boat", BOAT),
    ("minecraft:poplar_chest_boat", BOAT),
    ("minecraft:pufferfish", MOB_BOOL_INT),
    ("minecraft:rabbit", AGEABLE_INT),
    ("minecraft:ravager", MOB_BOOL),
    ("minecraft:salmon", MOB_BOOL_INT),
    ("minecraft:sheep", AGEABLE_BYTE),
    (
        "minecraft:shulker",
        &[0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 12, 0, 0],
    ),
    ("minecraft:shulker_bullet", ENTITY_BASE),
    ("minecraft:silverfish", MOB),
    ("minecraft:skeleton", MOB_BOOL),
    ("minecraft:skeleton_horse", AGEABLE_BYTE),
    ("minecraft:slime", AGEABLE_INT),
    ("minecraft:small_fireball", ITEM_PROJECTILE),
    (
        "minecraft:sniffer",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 35, 1,
        ],
    ),
    ("minecraft:snow_golem", MOB_BYTE),
    ("minecraft:snowball", ITEM_PROJECTILE),
    ("minecraft:spawner_minecart", MINECART),
    (
        "minecraft:spectral_arrow",
        &[0, 1, 6, 8, 8, 8, 20, 1, 0, 0, 8],
    ),
    ("minecraft:spider", MOB_BYTE),
    ("minecraft:splash_potion", ITEM_PROJECTILE),
    ("minecraft:spruce_boat", BOAT),
    ("minecraft:spruce_chest_boat", BOAT),
    ("minecraft:squid", AGEABLE),
    ("minecraft:stray", MOB),
    (
        "minecraft:strider",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 1, 8,
        ],
    ),
    (
        "minecraft:sulfur_cube",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 1, 1, 8,
        ],
    ),
    ("minecraft:tadpole", AGEABLE),
    (
        "minecraft:text_display",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 1, 1, 1, 39, 39, 40, 40, 0, 1, 3, 3, 3, 3, 3, 1, 5, 1, 1, 0, 0,
        ],
    ),
    ("minecraft:tnt", &[0, 1, 6, 8, 8, 8, 20, 1, 1, 14]),
    ("minecraft:tnt_minecart", MINECART),
    ("minecraft:trader_llama", LLAMA),
    (
        "minecraft:trident",
        &[0, 1, 6, 8, 8, 8, 20, 1, 0, 0, 8, 0, 8],
    ),
    ("minecraft:tropical_fish", MOB_BOOL_INT),
    ("minecraft:turtle", AGEABLE_BOOL2),
    ("minecraft:vex", MOB_BYTE),
    (
        "minecraft:villager",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 1, 18, 8,
        ],
    ),
    ("minecraft:vindicator", MOB_BOOL),
    ("minecraft:wandering_trader", AGEABLE_INT),
    ("minecraft:warden", MOB_INT),
    ("minecraft:wind_charge", ENTITY_BASE),
    ("minecraft:witch", AGEABLE),
    (
        "minecraft:wither",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 1, 1, 1, 1,
        ],
    ),
    ("minecraft:wither_skeleton", MOB),
    ("minecraft:wither_skull", &[0, 1, 6, 8, 8, 8, 20, 1, 8]),
    (
        "minecraft:wolf",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 0, 13, 8, 1, 2, 25, 26,
        ],
    ),
    ("minecraft:zoglin", MOB_BOOL),
    ("minecraft:zombie", ZOMBIE),
    ("minecraft:zombie_horse", AGEABLE_BYTE),
    (
        "minecraft:zombie_nautilus",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 8, 0, 13, 8, 32,
        ],
    ),
    (
        "minecraft:zombie_villager",
        &[
            0, 1, 6, 8, 8, 8, 20, 1, 0, 3, 17, 8, 1, 1, 11, 0, 8, 1, 8, 8, 18, 8,
        ],
    ),
    ("minecraft:zombified_piglin", ZOMBIE),
];

/// Expected `EntityDataSerializers` id at each accessor index for vanilla 26.3 entity types.
pub fn serializers(kind: &str) -> Option<&'static [i32]> {
    ENTITY_SERIALIZERS
        .binary_search_by_key(&kind, |&(id, _)| id)
        .ok()
        .map(|idx| ENTITY_SERIALIZERS[idx].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_serializers_table_is_sorted_and_lookup_matches() {
        for pair in ENTITY_SERIALIZERS.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "ENTITY_SERIALIZERS must be strictly sorted: {} >= {}",
                pair[0].0,
                pair[1].0
            );
        }
        for &(kind, expected) in ENTITY_SERIALIZERS {
            assert_eq!(serializers(kind), Some(expected), "{kind}");
        }
    }

    #[test]
    #[ignore = "daily"]
    fn entity_data_schema_matches_26_3() {
        let dir = std::env::var("SUNRISE_NETWORK_WITNESS_DIR").expect(
            "SUNRISE_NETWORK_WITNESS_DIR must be set (run via tools/test_network_parity.py)",
        );
        let path = std::path::Path::new(&dir).join("entity_data.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        let witness: serde_json::Value = serde_json::from_str(&text).unwrap();
        let recorded = witness["entity_type_serializers"]
            .as_object()
            .expect("entity_type_serializers object in entity_data.json");

        assert_eq!(
            ENTITY_SERIALIZERS.len(),
            recorded.len(),
            "entity type count mismatch between Rust table and 26.3 recording"
        );
        for &(kind, actual) in ENTITY_SERIALIZERS {
            let expected: Vec<i32> = recorded
                .get(kind)
                .unwrap_or_else(|| panic!("extra entity type in Rust table: {kind}"))
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as i32)
                .collect();
            assert_eq!(actual, expected.as_slice(), "{kind}: serializer ids");
        }
        for (kind, value) in recorded {
            let expected: Vec<i32> = value
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as i32)
                .collect();
            assert_eq!(
                serializers(kind),
                Some(expected.as_slice()),
                "missing or mismatched entity type from 26.3 recording: {kind}"
            );
        }
    }
}
