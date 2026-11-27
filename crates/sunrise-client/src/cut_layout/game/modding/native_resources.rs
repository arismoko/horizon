#![cfg(test)]
use mod_api::Identifier;
use mod_api::PackType;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::Host;
    use std::collections::BTreeMap;
    use std::io::Read as _;
    use sunrise_assets::resource_source::{Resource, ResourceSource};
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_resources::NativeResourceReads as _;

    fn read(host: &dyn Host, pack_type: PackType, name: &str) -> Option<Vec<u8>> {
        let mut reader = host
            .open_resource(pack_type, &Identifier::parse(name).unwrap())
            .unwrap()?;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        Some(bytes)
    }

    #[cfg(feature = "client")]
    fn id(value: &str) -> Identifier {
        Identifier::parse(value).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn server_resource_uses_the_higher_of_two_selected_world_packs() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let scratch = tempfile::tempdir().unwrap();
            game.server.world_root = scratch.path().to_owned();
            for (name, bytes) in [("low", "low"), ("high", "high")] {
                let pack = game.server.world_root.join("datapacks").join(name);
                std::fs::create_dir_all(pack.join("data/demo/value")).unwrap();
                std::fs::write(
                    pack.join("pack.mcmeta"),
                    r#"{"pack":{"description":"test","pack_format":121,"min_format":[121,0],"max_format":[121,0]}}"#,
                )
                .unwrap();
                std::fs::write(pack.join("data/demo/value/order.json"), bytes).unwrap();
            }
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("reload"));
            assert!(game.run_command("datapack disable \"file/high\""));
            assert!(game.run_command("datapack disable \"file/low\""));
            assert!(game.run_command("datapack enable \"file/low\" first"));
            assert!(game.run_command("datapack enable \"file/high\" last"));
            assert_eq!(
                read(game, PackType::ServerData, "demo:value/order.json").as_deref(),
                Some(b"high".as_slice())
            );
            assert!(game.run_command("datapack disable \"file/low\""));
            assert!(game.run_command("datapack enable \"file/low\" last"));
            assert_eq!(
                read(game, PackType::ServerData, "demo:value/order.json").as_deref(),
                Some(b"low".as_slice())
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn server_data_reads_vanilla_files_and_refuses_missing_ones() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let bytes = read(
                game,
                PackType::ServerData,
                "minecraft:enchantment/sharpness.json",
            )
            .expect("the game's own data pack holds sharpness");
            assert!(bytes.starts_with(b"{"));
            assert!(
                read(
                    game,
                    PackType::ServerData,
                    "minecraft:enchantment/not_a_real_enchantment.json"
                )
                .is_none()
            );
            // A server has no selected client asset stack to read.
            assert!(
                read(
                    game,
                    PackType::ClientResources,
                    "minecraft:models/block/stone.json"
                )
                .is_none()
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn server_data_reads_mod_content_and_lets_it_override_the_game() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let vanilla = read(
                game,
                PackType::ServerData,
                "minecraft:enchantment/sharpness.json",
            )
            .expect("the game's own data pack holds sharpness");
            game.pack.data_changes = ResourceSource::indexed(BTreeMap::from([
                (
                    "data/demo/machines/furnace.json".to_owned(),
                    Resource::bytes(b"{\"mod\":true}".to_vec()),
                ),
                (
                    "data/minecraft/enchantment/sharpness.json".to_owned(),
                    Resource::bytes(b"{\"pack\":true}".to_vec()),
                ),
            ]));
            // A mod's own data file resolves through its content layer.
            assert_eq!(
                read(game, PackType::ServerData, "demo:machines/furnace.json").as_deref(),
                Some(b"{\"mod\":true}".as_slice())
            );
            // A later layer in the stack overrides the game's own data.
            assert_eq!(
                read(
                    game,
                    PackType::ServerData,
                    "minecraft:enchantment/sharpness.json"
                )
                .as_deref(),
                Some(b"{\"pack\":true}".as_slice())
            );
            assert_ne!(vanilla, b"{\"pack\":true}");
        });
    }

    /// `getResourceStack` gives each copy lowest first, and `listResources` the names under a
    /// folder of every namespace with the suffix, path first, without metadata files.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn resource_stacks_and_listings_cover_every_pack_and_namespace() {
        let name = "cut_layout::game::native_resources::tests::resource_stacks_and_listings_cover_every_pack_and_namespace";
        let flag = "SUNRISE_RESOURCE_STACK_CASE";
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            name,
            flag,
            &["server", "client"],
        ) {
            return;
        }
        let files = |top: &str| -> BTreeMap<String, Resource> {
            [
                ("demo/machines/a.json", "a"),
                ("demo/machines/a.json.mcmeta", "meta"),
                ("demo/machines/sub/b.json", "b"),
                ("demo/machines/e.txt", "e"),
                ("demo/other/d.json", "d"),
                ("other/machines/c.json", "c"),
            ]
            .into_iter()
            .map(|(path, bytes)| (format!("{top}/{path}"), Resource::bytes(bytes.into())))
            .collect()
        };
        let listed = [
            id("demo:machines/a.json"),
            id("other:machines/c.json"),
            id("demo:machines/sub/b.json"),
        ];
        match std::env::var(flag).unwrap().as_str() {
            "server" => sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let sharpness = id("minecraft:enchantment/sharpness.json");
                let vanilla = read(
                    game,
                    PackType::ServerData,
                    "minecraft:enchantment/sharpness.json",
                )
                .unwrap();
                let mut changes = files("data");
                changes.insert(
                    "data/minecraft/enchantment/sharpness.json".to_owned(),
                    Resource::bytes(b"{\"pack\":true}".to_vec()),
                );
                game.pack.data_changes = ResourceSource::indexed(changes);
                assert_eq!(
                    game.resource_stack(PackType::ServerData, &sharpness),
                    [vanilla, b"{\"pack\":true}".to_vec()]
                );
                assert_eq!(
                    game.resource_stack(PackType::ServerData, &id("demo:machines/none.json")),
                    Vec::<Vec<u8>>::new()
                );
                assert_eq!(
                    game.list_resources(PackType::ServerData, "machines", ".json"),
                    listed
                );
                assert!(
                    game.list_resources(PackType::ServerData, "../machines", "")
                        .is_empty()
                );
                // A server has no client resource stack.
                assert!(
                    game.resource_stack(PackType::ClientResources, &listed[0])
                        .is_empty()
                );
                assert!(
                    game.list_resources(PackType::ClientResources, "", "")
                        .is_empty()
                );
            }),
            "client" => {
                crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                    let mut index = files("assets");
                    let model = "assets/demo/models/gem.json".to_owned();
                    let copies = vec![
                        Resource::bytes(b"low".to_vec()),
                        Resource::bytes(b"high".to_vec()),
                    ];
                    index.insert(model.clone(), copies[1].clone());
                    game.pack.source = ResourceSource::indexed_with_layers(
                        index,
                        BTreeMap::from([(model, copies)]),
                        None,
                    );
                    assert_eq!(
                        game.resource_stack(PackType::ClientResources, &id("demo:models/gem.json")),
                        [b"low".to_vec(), b"high".to_vec()]
                    );
                    assert_eq!(
                        game.list_resources(PackType::ClientResources, "machines", ".json"),
                        listed
                    );
                })
            }
            case => unreachable!("{case}"),
        }
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn client_assets_read_the_winning_pack_file() {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let bytes = read(
                game,
                PackType::ClientResources,
                "minecraft:models/block/stone.json",
            )
            .expect("the vanilla asset pack holds stone's model");
            assert!(bytes.starts_with(b"{"));
            assert!(
                read(
                    game,
                    PackType::ClientResources,
                    "minecraft:models/block/not_a_real_model.json"
                )
                .is_none()
            );
        });
    }
}
