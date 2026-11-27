#![cfg(feature = "client")]
#![cfg(test)]
use mod_api::{BiomeKey, BiomeKeyError, DataMap, DataMapError, Tag};

struct VillagerTypes;
impl DataMap for VillagerTypes {
    type Entry = BiomeKey;
    type Value = Tag;
    fn id(&self) -> &str {
        "neoforge:villager_types"
    }
    fn decode(&self, value: &Tag) -> Option<Tag> {
        Some(value.clone())
    }
}

#[test]
#[ignore = "daily"]
fn biome_key_world_install_reload_and_unavailable_sides() {
    mod_platform::testing::with_mods(Default::default(), || {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let root = tempfile::tempdir().unwrap();
            game.server.world_root = root.path().to_owned();
            game.options.dedicated = Some(Vec::new());

            let plains = BiomeKey::parse("minecraft:plains").unwrap();
            let desert = BiomeKey::parse("minecraft:desert").unwrap();
            let marsh = BiomeKey::parse("test:marsh").unwrap();
            let unknown = BiomeKey::parse("test:nope").unwrap();
            game.common.world_biomes = Some(std::sync::Arc::new(
                game.common
                    .res
                    .biomes()
                    .with_packs([("test:marsh", &serde_json::json!({}))])
                    .unwrap(),
            ));

            assert_eq!(
                game.biome("minecraft:plains"),
                Err(BiomeKeyError::Unavailable)
            );
            assert_eq!(plains.view(game), Err(BiomeKeyError::Unavailable));
            assert_eq!(
                plains.data(game, &VillagerTypes),
                Err(DataMapError::Unavailable)
            );

            let pack = root.path().join("datapacks/biomes");
            let file = pack.join("data/neoforge/data_maps/worldgen/biome/villager_types.json");
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(
                pack.join("pack.mcmeta"),
                r#"{"pack":{"description":"Biome fixture"}}"#,
            )
            .unwrap();
            let definition = pack.join("data/test/worldgen/biome/marsh.json");
            std::fs::create_dir_all(definition.parent().unwrap()).unwrap();
            let plains_file = game
                .common
                .res
                .biomes()
                .builtin_file("minecraft:plains")
                .unwrap();
            std::fs::write(definition, plains_file.to_string()).unwrap();
            std::fs::write(
                root.path().join("datapacks.json"),
                r#"{"enabled":["file/biomes"],"disabled":[]}"#,
            )
            .unwrap();

            let foreign_keys = mod_api::__private::BiomeKeys::new(u64::MAX, [plains.clone()]);
            let foreign_view = foreign_keys.view("minecraft:plains").unwrap();

            for (index, value) in ["minecraft:plains", "minecraft:savanna"]
                .into_iter()
                .enumerate()
            {
                std::fs::write(
                    &file,
                    format!(
                        r#"{{"replace":true,"values":{{"test:marsh":{{"villager_type":"{value}"}}}}}}"#
                    ),
                )
                .unwrap();
                let packs = game.reload_datapacks(false, &[]).unwrap();
                let resources = game.options.resources.clone();
                game.server.world_tables.0 = game
                    .stage_world_tables(&packs, index > 0, &resources)
                    .unwrap()
                    .tables;

                let view = game.biome("minecraft:plains").unwrap();
                assert_eq!(view.id(), "minecraft:plains");
                assert_eq!(view.key(), &plains);
                assert!(view.is(&plains));
                assert_eq!(plains.view(game), Ok(view));
                assert_eq!(game.biome("test:nope"), Err(BiomeKeyError::UnknownBiome));
                assert_eq!(
                    game.biome("Bad Name"),
                    Err(BiomeKeyError::InvalidIdentifier)
                );
                assert_eq!(unknown.view(game), Err(BiomeKeyError::UnknownBiome));

                let expected = Ok(Some(Tag::Compound(sunrise_core::nbt::Compound::from([(
                    "villager_type".into(),
                    Tag::String(value.into()),
                )]))));
                let marsh_view = game.biome("test:marsh").unwrap();
                assert_eq!(marsh_view.key(), &marsh);
                assert_eq!(marsh_view.data(game, &VillagerTypes), expected);
                assert_eq!(marsh.data(game, &VillagerTypes), expected);
                let reload: &dyn mod_api::ReloadContext = game;
                assert_eq!(reload.biome_key_id(&marsh), Ok("test:marsh"));
                assert_eq!(reload.data_map(&marsh, &VillagerTypes), expected);
                assert_eq!(desert.data(game, &VillagerTypes), Ok(None));
                assert_eq!(
                    unknown.data(game, &VillagerTypes),
                    Err(DataMapError::InvalidKey)
                );
                assert_eq!(
                    game.biome_view_id(foreign_view),
                    Err(BiomeKeyError::WrongGeneration)
                );
                assert_eq!(
                    foreign_view.data(game, &VillagerTypes),
                    Err(DataMapError::InvalidKey)
                );
            }

            super::native_entity_refs::tests::with_client_game(|game| {
                assert_eq!(
                    game.biome("minecraft:plains"),
                    Err(BiomeKeyError::Unavailable)
                );
                assert_eq!(plains.view(game), Err(BiomeKeyError::Unavailable));
                assert_eq!(
                    plains.data(game, &VillagerTypes),
                    Err(DataMapError::Unavailable)
                );
            });

            game.server.world_tables.0 = None;
            assert_eq!(
                game.biome("minecraft:plains"),
                Err(BiomeKeyError::Unavailable)
            );
            assert_eq!(
                plains.data(game, &VillagerTypes),
                Err(DataMapError::Unavailable)
            );
        });
    });
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
