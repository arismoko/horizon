//! Data maps for native server mods (docs/modding/src/how-to/native-mods.md): values a data pack
//! attaches to the entries of a registry, under the types a mod registered
//! (`native_data_map_types`). Vanilla has no data maps; the contract is
//! NeoForge's `DataMapType`: files at
//! `data/<map namespace>/data_maps/<registry folder>/<map path>.json` hold
//! `replace`, `values` (entry ids and `#tags` to values, each value optionally
//! guarded by its own `neoforge:conditions`) and `remove`, and a newer value
//! overrides an older one for the same entry. A registry's folder is its path
//! for a `minecraft` registry and `<namespace>/<path>` otherwise. Any registry
//! the game names takes maps: built in, the packs' and content mods', and the
//! datapack registries mods declare. The files are the world's data packs' and
//! what mods contribute through `native_data_pack_files` (key
//! `data_maps/<ns>:<registry folder>/<map path>`), merged each time the
//! server's tables are built: a world's first open and every `/reload`.
use crate::game::ServerGame;

/// The winning JSON by registry, then map id, then entry id in sorted order.
#[cfg(test)]
pub(in crate::game) type MergedDataMaps = LoadedMaps<Value>;

#[cfg(test)]
use mod_api::game::DataMapType;
#[cfg(test)]
use serde_json::Value;
#[cfg(test)]
use sunrise_gameplay_common::game::native_data_maps_common::{
    LoadedMaps, MapRegistries, load_maps,
};

/// Merge the types' raw JSON values in unit tests that do not test a specific codec.
#[cfg(test)]
fn load_json_maps(
    types: &[DataMapType],
    overlay: &sunrise_world::loot::PackOverlay<'_>,
    registries: &dyn MapRegistries,
) -> MergedDataMaps {
    load_maps(types, overlay, registries, |_, value| Ok(value.clone()))
}

impl ServerGame<'_> {
    /// Tell the mods the data maps of every registry with a type were merged
    /// (`DataMapsUpdatedEvent`), once the server's tables hold them.
    pub(in crate::game::server_side) fn announce_data_maps_updated(
        &mut self,
        cause: mod_api::DataMapsUpdateCause,
    ) {
        for registry in self.server.data_map_registries.clone() {
            self.native_data_maps_updated(&registry, cause);
        }
    }
}

use sunrise_gameplay_common::game::native_hooks::data_maps_updated::GameHooks as _;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::native_data_maps_common::{DataMaps, WorldRegistries};

    #[test]
    fn data_map_read_refuses_lossy_payloads_instead_of_successful_substitutes() {
        let registries =
            Fixture::with(&[("minecraft:item", "minecraft:logs", &["minecraft:oak_log"])]);
        for value in [
            json!(null),
            json!({"nested": [null]}),
            json!(9223372036854775809_u64),
            json!(u64::MAX),
        ] {
            let layers = files(&[(
                "data_maps/test:item/map",
                &[
                    json!({"values": {"minecraft:stick": 7, "minecraft:coal": 3}}),
                    json!({
                        "replace": true,
                        "values": {"minecraft:oak_log": 4, "minecraft:stone": value.clone()},
                        "remove": ["minecraft:coal"]
                    }),
                    json!({
                        "replace": true,
                        "values": {"#minecraft:logs": value}
                    }),
                    json!({"values": {"minecraft:birch_log": 6}}),
                ],
            )]);
            let maps = DataMaps::load(
                &[ty("test:map", "minecraft:item")],
                &sunrise_world::loot::PackOverlay {
                    files: &layers,
                    ..sunrise_world::loot::PackOverlay::default()
                },
                &registries,
            );
            assert_eq!(
                maps.value("minecraft:item", "test:map", "minecraft:stick"),
                Ok(Some(&Tag::Int(7)))
            );
            assert_eq!(
                maps.value("minecraft:item", "test:map", "minecraft:coal"),
                Ok(Some(&Tag::Int(3)))
            );
            assert_eq!(
                maps.value("minecraft:item", "test:map", "minecraft:oak_log"),
                Ok(None)
            );
            assert_eq!(
                maps.value("minecraft:item", "test:map", "minecraft:stone"),
                Ok(None)
            );
            assert_eq!(
                maps.value("minecraft:item", "test:map", "minecraft:birch_log"),
                Ok(Some(&Tag::Int(6)))
            );
            let expected = [
                ("minecraft:birch_log".to_owned(), Tag::Int(6)),
                ("minecraft:coal".to_owned(), Tag::Int(3)),
                ("minecraft:stick".to_owned(), Tag::Int(7)),
            ];
            assert_eq!(
                maps.entries("minecraft:item", "test:map"),
                Ok(expected.as_slice())
            );
        }
    }

    #[test]
    fn builtin_store_is_registered_without_mod_types() {
        let builtin = super::super::neoforge_mob_maps::load(
            &sunrise_world::loot::PackOverlay::NONE,
            |_| None,
            |_, _| true,
        )
        .unwrap();
        let maps = DataMaps::default().with_builtins(builtin);
        assert_eq!(maps.registries().len(), 5);
        assert_eq!(
            maps.value(
                "minecraft:item",
                "neoforge:villager_compostables",
                "minecraft:wheat_seeds"
            ),
            Ok(Some(&Tag::compound([(
                "can_villager_compost",
                Tag::Byte(1)
            )])))
        );
    }

    /// Registries for tests: items, blocks and entity types; entity type ids
    /// listed, item and block ids not, and the tags given by registry.
    #[derive(Default)]
    struct Fixture {
        tags: HashMap<(&'static str, &'static str), Vec<&'static str>>,
    }
    impl Fixture {
        fn with(tags: &[(&'static str, &'static str, &[&'static str])]) -> Self {
            Self {
                tags: tags
                    .iter()
                    .map(|(registry, tag, members)| ((*registry, *tag), members.to_vec()))
                    .collect(),
            }
        }
    }
    impl MapRegistries for Fixture {
        fn known(&self, registry: &str) -> bool {
            ["minecraft:item", "minecraft:block", "minecraft:entity_type"].contains(&registry)
        }
        fn contains(&self, registry: &str, entry: &str) -> bool {
            registry != "minecraft:entity_type"
                || [
                    "minecraft:pig",
                    "minecraft:stray",
                    "minecraft:skeleton",
                    "minecraft:cow",
                ]
                .contains(&entry)
        }
        fn tag(&self, registry: &str, tag: &str) -> Vec<String> {
            self.tags
                .iter()
                .filter(|((r, t), _)| *r == registry && *t == tag)
                .flat_map(|(_, members)| members.iter().map(|m| (*m).to_owned()))
                .collect()
        }
    }

    fn ty(id: &str, registry: &str) -> DataMapType {
        DataMapType {
            id: id.to_owned(),
            registry: registry.to_owned(),
        }
    }

    fn files(list: &[(&str, &[Value])]) -> Vec<(String, Vec<Vec<u8>>)> {
        list.iter()
            .map(|(key, versions)| {
                (
                    (*key).to_owned(),
                    versions
                        .iter()
                        .map(|v| v.to_string().into_bytes())
                        .collect(),
                )
            })
            .collect()
    }

    fn load(files: &[(String, Vec<Vec<u8>>)]) -> MergedDataMaps {
        let registries = Fixture::with(&[(
            "minecraft:item",
            "minecraft:logs",
            &["minecraft:oak_log", "minecraft:birch_log"],
        )]);
        load_json_maps(
            &[ty("mymod:fuel", "minecraft:item")],
            &sunrise_world::loot::PackOverlay {
                files,
                ..sunrise_world::loot::PackOverlay::default()
            },
            &registries,
        )
    }

    fn get<'a>(maps: &'a MergedDataMaps, entry: &str) -> Option<&'a Value> {
        maps["minecraft:item"]["mymod:fuel"].get(entry)
    }

    #[test]
    fn a_value_reads_as_nbt_and_a_map_no_mod_registered_is_refused() {
        let f = files(&[
            (
                "data_maps/mymod:item/fuel",
                &[json!({"values": {
                    "minecraft:coal": 1600,
                    "minecraft:stick": {"burn": 3_000_000_000_i64}
                }})],
            ),
            (
                "data_maps/minecraft:item/fuel",
                &[json!({"values": {
                    "minecraft:coal": 800
                }})],
            ),
        ]);
        let maps = DataMaps::load(
            &[
                ty("mymod:fuel", "minecraft:item"),
                ty("minecraft:fuel", "minecraft:item"),
            ],
            &sunrise_world::loot::PackOverlay {
                files: &f,
                ..sunrise_world::loot::PackOverlay::default()
            },
            &Fixture::default(),
        );
        let value =
            |registry: &str, type_id: &str, entry: &str| maps.value(registry, type_id, entry);
        assert_eq!(
            value("minecraft:item", "mymod:fuel", "minecraft:coal"),
            Ok(Some(&Tag::Int(1600)))
        );
        assert_eq!(
            value("minecraft:item", "mymod:fuel", "minecraft:stick"),
            Ok(Some(&Tag::Compound(
                [("burn".into(), Tag::Long(3_000_000_000))]
                    .into_iter()
                    .collect()
            )))
        );
        assert_eq!(
            value("minecraft:item", "mymod:fuel", "minecraft:stone"),
            Ok(None)
        );
        assert_eq!(
            value("minecraft:item", "minecraft:fuel", "minecraft:coal"),
            Ok(Some(&Tag::Int(800)))
        );
        assert_eq!(
            value("minecraft:block", "mymod:fuel", "minecraft:coal"),
            Err(mod_api::DataMapError::UnknownType)
        );
        assert_eq!(
            value("minecraft:item", "mymod:other", "minecraft:coal"),
            Err(mod_api::DataMapError::UnknownType)
        );
    }

    #[test]
    fn registries_with_a_type_are_listed_once_in_id_order() {
        let maps = DataMaps::load(
            &[
                ty("mymod:b", "minecraft:block"),
                ty("mymod:a", "minecraft:item"),
                ty("mymod:c", "minecraft:block"),
                ty("mymod:d", "test:nowhere"),
            ],
            &sunrise_world::loot::PackOverlay::NONE,
            &Fixture::default(),
        );
        assert_eq!(maps.registries(), ["minecraft:block", "minecraft:item"]);
    }

    #[test]
    fn a_map_on_any_registry_resolves_ids_tags_and_lists_whole() {
        // `minecraft:ghost` is not in the fixture's entity types: a tag naming
        // an id the registry doesn't hold publishes nothing for it.
        let registries = Fixture::with(&[(
            "minecraft:entity_type",
            "minecraft:skeletons",
            &["minecraft:skeleton", "minecraft:stray", "minecraft:ghost"],
        )]);
        let f = files(&[(
            "data_maps/test:entity_type/weight",
            &[json!({"values": {
                "pig": 3,
                "#skeletons": 5,
                "minecraft:missing": 9
            }})],
        )]);
        let maps = DataMaps::load(
            &[ty("test:weight", "minecraft:entity_type")],
            &sunrise_world::loot::PackOverlay {
                files: &f,
                ..sunrise_world::loot::PackOverlay::default()
            },
            &registries,
        );
        let value = |entry| maps.value("minecraft:entity_type", "test:weight", entry);
        let pig = value("minecraft:pig").unwrap().unwrap();
        assert_eq!(pig, &Tag::Int(3));
        assert_eq!(value("minecraft:stray"), Ok(Some(&Tag::Int(5))));
        assert_eq!(value("minecraft:cow"), Ok(None));
        // An id the registry doesn't hold is skipped.
        assert_eq!(value("minecraft:missing"), Ok(None));
        // A tag member the registry doesn't hold is skipped with the plain ids.
        assert_eq!(value("minecraft:ghost"), Ok(None));
        let entries = maps
            .entries("minecraft:entity_type", "test:weight")
            .unwrap();
        assert_eq!(
            entries,
            &[
                ("minecraft:pig".to_owned(), Tag::Int(3)),
                ("minecraft:skeleton".to_owned(), Tag::Int(5)),
                ("minecraft:stray".to_owned(), Tag::Int(5)),
            ]
        );
        assert!(std::ptr::eq(pig, &entries[0].1));
        assert_eq!(maps.registries(), ["minecraft:entity_type"]);
    }

    /// A mod-declared datapack registry the command ids do not list is checked
    /// against the world's loaded entries; a registry with no source of ids
    /// still accepts an entry, as before.
    #[test]
    fn a_declared_datapack_registry_rejects_unknown_ids() {
        // The declaration the process reads, as a registering mod's would be.
        mod_api::__private::install_process_datapack_registry_declarations(vec![
            mod_api::DatapackRegistryDeclaration {
                mod_id: "example".into(),
                key: "example:spell".into(),
                fields: vec![mod_api::DatapackField::required(
                    "power",
                    mod_api::DatapackFieldKind::Int { min: 0, max: 100 },
                )],
                sync: mod_api::DatapackRegistrySync::Required,
            },
        ]);
        let set = mod_api::DatapackRegistrySet::with_entries_for_testing(
            "example:spell",
            &["example:fire"],
        );
        let items = sunrise_items::items::Items::empty();
        let registries = WorldRegistries::new(
            [],
            Some(&set),
            &items,
            std::path::Path::new("unused"),
            &sunrise_world::loot::PackOverlay::NONE,
        );
        assert!(
            !registries.contains("example:spell", "example:ice"),
            "an id the world never loaded is refused"
        );
        assert!(registries.contains("example:spell", "example:fire"));
        // No source of ids, no refusal: a typed registry the sync did not
        // list (the ids branch above covers the listed ones).
        assert!(registries.contains("minecraft:some_typed", "whatever"));
    }

    #[test]
    fn a_value_counts_only_when_its_own_conditions_hold() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": 1,
                "minecraft:stick": {"neoforge:conditions": [
                    {"type": "neoforge:mod_loaded", "modid": "absent"}], "value": 2},
                "minecraft:planks": {"neoforge:conditions": [
                    {"type": "neoforge:always"}], "value": 3},
                "minecraft:coal_block": {"neoforge:conditions": [
                    {"type": "neoforge:always"}], "neoforge:value": 9},
                "minecraft:kelp": {"neoforge:conditions": [
                    {"type": "neoforge:always"}], "burn": 2}
            }})],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(1)));
        assert_eq!(get(&maps, "minecraft:stick"), None);
        assert_eq!(get(&maps, "minecraft:planks"), Some(&json!(3)));
        assert_eq!(get(&maps, "minecraft:coal_block"), Some(&json!(9)));
        assert_eq!(get(&maps, "minecraft:kelp"), Some(&json!({"burn": 2})));
    }

    /// A value's own conditions are NeoForge's element conditions: a Fabric key there is
    /// ordinary data of the value, never evaluated (Fabric has no data maps).
    #[test]
    fn a_values_fabric_key_is_ordinary_data() {
        let value = json!({"fabric:load_conditions": {"condition": "fabric:false"}, "burn": 4});
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {"minecraft:cod": value}})],
        )]);
        assert_eq!(get(&load(&f), "minecraft:cod"), Some(&value));
    }

    #[test]
    fn values_attach_to_entries_and_to_every_member_of_a_tag() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": {"burn": 1600},
                "stick": 100,
                "#minecraft:logs": {"burn": 300}
            }})],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!({"burn": 1600})));
        // An id without a namespace is a `minecraft` one.
        assert_eq!(get(&maps, "minecraft:stick"), Some(&json!(100)));
        assert_eq!(get(&maps, "minecraft:oak_log"), Some(&json!({"burn": 300})));
        assert_eq!(
            get(&maps, "minecraft:birch_log"),
            Some(&json!({"burn": 300}))
        );
        assert_eq!(get(&maps, "minecraft:stone"), None);
    }

    #[test]
    fn a_bad_value_condition_does_not_replace_or_apply_any_values_in_its_layer() {
        let layers = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values":{"minecraft:stick":2,"minecraft:coal":3}}),
                json!({"replace":true,"values":{"minecraft:oak_log":4,"minecraft:planks":{
                "value":5,"neoforge:conditions":[{"type":"neoforge:not","value":{"type":"minecraft:missing"}}]
            }},"remove":["minecraft:coal"]}),
                json!({"values":{"minecraft:birch_log":6}}),
            ],
        )]);
        let maps = load(&layers);
        assert_eq!(get(&maps, "minecraft:stick"), Some(&json!(2)));
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(3)));
        assert_eq!(get(&maps, "minecraft:oak_log"), None);
        assert_eq!(get(&maps, "minecraft:planks"), None);
        assert_eq!(get(&maps, "minecraft:birch_log"), Some(&json!(6)));
    }

    #[test]
    fn a_malformed_layer_envelope_preserves_earlier_values() {
        let lower = json!({"values": {"minecraft:coal": 10}});
        for invalid in [
            json!({"replace": true, "values": {}, "remove": false}),
            json!({"replace": "true", "values": {"minecraft:coal": 20}}),
            json!({"values": {"minecraft:coal": 20}, "remove": ["minecraft:coal", {"key": false}]}),
        ] {
            let files = files(&[("data_maps/mymod:item/fuel", &[lower.clone(), invalid])]);
            assert_eq!(get(&load(&files), "minecraft:coal"), Some(&json!(10)));
        }
    }

    #[test]
    fn a_newer_file_overrides_and_replace_clears_what_came_before() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"minecraft:coal": 1, "minecraft:stick": 2}}),
                json!({"values": {"minecraft:coal": 10}}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(10)));
        assert_eq!(get(&maps, "minecraft:stick"), Some(&json!(2)));
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"minecraft:coal": 1, "minecraft:stick": 2}}),
                json!({"replace": true, "values": {"minecraft:coal": 10}}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(10)));
        assert_eq!(get(&maps, "minecraft:stick"), None);
    }

    #[test]
    fn remove_takes_entries_and_tags_away_after_the_files_own_values() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"#minecraft:logs": 5, "minecraft:coal": 1}}),
                // The file's own value for coal is attached, then removed.
                json!({"values": {"minecraft:coal": 9}, "remove": ["minecraft:coal", "#minecraft:logs"]}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), None);
        assert_eq!(get(&maps, "minecraft:oak_log"), None);
    }

    #[test]
    fn the_wrapper_form_gives_its_value_and_a_plain_object_is_a_value() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": {"value": {"burn": 3}, "replace": true},
                "minecraft:stick": {"value": 1, "other": 2}
            }})],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!({"burn": 3})));
        // More than `value` and `replace`: the object is the data.
        assert_eq!(
            get(&maps, "minecraft:stick"),
            Some(&json!({"value": 1, "other": 2}))
        );
    }

    #[test]
    fn a_bad_file_is_skipped_alone_and_bad_types_are_left_out() {
        let mut f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {"minecraft:coal": 1}})],
        )]);
        f[0].1.insert(0, b"{nope".to_vec());
        f[0].1
            .push(json!({"nothing": true}).to_string().into_bytes());
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(1)));
        let mc_files = files(&[(
            "data_maps/minecraft:item/fuel",
            &[json!({"values": {"minecraft:coal": 2}})],
        )]);
        let maps = load_json_maps(
            &[
                ty("minecraft:fuel", "minecraft:item"),
                ty("mymod:x", "test:nowhere"),
                ty("nonamespace", "minecraft:item"),
                ty("Bad:Caps", "minecraft:item"),
                ty("a/b:c", "minecraft:item"),
                ty("..:x", "minecraft:item"),
                ty("mymod:a", "minecraft:block"),
                ty("mymod:a", "minecraft:block"),
            ],
            &sunrise_world::loot::PackOverlay {
                files: &mc_files,
                ..sunrise_world::loot::PackOverlay::default()
            },
            &Fixture::default(),
        );
        // Both valid types are accepted (minecraft:fuel on item, mymod:a once on block);
        // unknown registries and malformed identifiers are left out.
        assert_eq!(maps.len(), 2);
        let items = &maps["minecraft:item"];
        assert_eq!(items.len(), 1);
        assert_eq!(
            items["minecraft:fuel"].get("minecraft:coal"),
            Some(&json!(2))
        );
        let blocks = &maps["minecraft:block"];
        assert_eq!(blocks.len(), 1);
        assert!(blocks["mymod:a"].is_empty());
    }

    #[test]
    fn top_level_conditions_do_not_suppress_map_layers() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values":{"minecraft:coal":1}}),
                json!({"neoforge:conditions":[{"type":"neoforge:never"}],"replace":true,"values":{
                    "minecraft:stick":2,
                    "minecraft:coal":{"neoforge:conditions":[{"type":"neoforge:never"}],"value":3}
                }}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:stick"), Some(&json!(2)));
        assert!(get(&maps, "minecraft:coal").is_none());
    }

    #[test]
    fn block_maps_read_the_block_tags() {
        let registries =
            Fixture::with(&[("minecraft:block", "minecraft:logs", &["minecraft:oak_log"])]);
        let f = files(&[(
            "data_maps/mymod:block/hardness",
            &[json!({"values": {"#minecraft:logs": 2}})],
        )]);
        let maps = load_json_maps(
            &[ty("mymod:hardness", "minecraft:block")],
            &sunrise_world::loot::PackOverlay {
                files: &f,
                ..sunrise_world::loot::PackOverlay::default()
            },
            &registries,
        );
        let map = &maps["minecraft:block"]["mymod:hardness"];
        assert_eq!(map.get("minecraft:oak_log"), Some(&json!(2)));
    }

    #[test]
    fn empty_loaded_biome_registry_rejects_data_map_targets() {
        let biome_registry = sunrise_sim::lan::data::BIOME_REGISTRY;
        let command_biomes = [String::from("minecraft:plains")];
        let items = sunrise_items::items::Items::empty();
        let f = files(&[(
            "data_maps/mymod:worldgen/biome/climate",
            &[json!({"values": {
                "minecraft:plains": 1,
                "test:marsh": 2
            }})],
        )]);
        let overlay = sunrise_world::loot::PackOverlay {
            files: &f,
            ..sunrise_world::loot::PackOverlay::default()
        };

        // An explicitly empty loaded biome table must reject every target even when
        // the command registry still lists biome ids.
        let empty = WorldRegistries::new(
            [(biome_registry, command_biomes.as_slice())],
            None,
            &items,
            std::path::Path::new("unused"),
            &overlay,
        )
        .with_biome_keys(&[]);
        assert!(!empty.contains(biome_registry, "minecraft:plains"));
        let maps = DataMaps::load(&[ty("mymod:climate", biome_registry)], &overlay, &empty);
        assert_eq!(
            maps.value(biome_registry, "mymod:climate", "minecraft:plains"),
            Ok(None)
        );
        assert_eq!(maps.entries(biome_registry, "mymod:climate"), Ok(&[][..]));

        // Authoritative loaded biome keys override the command registry for populated tables too.
        let loaded_keys = [mod_api::BiomeKey::parse("test:marsh").unwrap()];
        let loaded = WorldRegistries::new(
            [(biome_registry, command_biomes.as_slice())],
            None,
            &items,
            std::path::Path::new("unused"),
            &overlay,
        )
        .with_biome_keys(&loaded_keys);
        let maps = DataMaps::load(&[ty("mymod:climate", biome_registry)], &overlay, &loaded);
        assert_eq!(
            maps.value(biome_registry, "mymod:climate", "minecraft:plains"),
            Ok(None)
        );
        assert_eq!(
            maps.value(biome_registry, "mymod:climate", "test:marsh"),
            Ok(Some(&Tag::Int(2)))
        );
    }
}
