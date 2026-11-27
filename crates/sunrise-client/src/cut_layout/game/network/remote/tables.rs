#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use std::{collections::HashSet, sync::Arc};
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_items::items::ItemStack;
    use sunrise_protocol::{
        connection::Pipe,
        packets::configuration::{RegistryTags, TagEntry, UpdateTags},
    };
    use sunrise_sim::mobs::equip::equippable_in_slot;
    use {
        crate::game::load_resources, crate::settings::Settings, sunrise_client_lib::audio::Audio,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
    };

    fn client<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> sunrise_client_gameplay::game::ClientGame<'a> {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            &Progress::new(),
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::Client;
        let res = &*resources.insert(loaded.resources);
        let mut game = sunrise_client_gameplay::game::ClientGame::new(
            res,
            loaded.client_res.unwrap(),
            loaded.pack,
            loaded.client_pack.unwrap(),
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
            sunrise_gameplay::game::server_side::integrated::serve,
            sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
            crate::game::session_verifier(),
            crate::game::profile_resolver,
        )
        .unwrap();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "TagTest").unwrap();
        game
    }

    #[test]
    #[ignore = "daily"]
    fn default_and_patched_equippables_follow_the_servers_entity_tags() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::remote::tables::tests::default_and_patched_equippables_follow_the_servers_entity_tags",
            || {
                let mut resources = None;
                let mut game = client(&mut resources);
                let builtin = game.client.remote.as_ref().unwrap().builtin.clone();
                let registry = "minecraft:entity_type";
                let tag = "minecraft:can_equip_saddle";
                let update = |entity: &str| UpdateTags {
                    registries: vec![RegistryTags {
                        registry: registry.into(),
                        tags: vec![TagEntry {
                            name: tag.into(),
                            ids: vec![builtin.get(registry).unwrap().id(entity).unwrap()],
                        }],
                    }],
                };
                let horse = "minecraft:horse";
                let camel = "minecraft:camel";
                let saddle = ItemStack::new("minecraft:saddle", 1);
                let mut patched = ItemStack::new("minecraft:stick", 1);
                patched.data_mut().set_other(
                    "minecraft:equippable",
                    Some(Tag::compound([
                        ("slot", Tag::String("saddle".into())),
                        ("allowed_entities", Tag::String("#can_equip_saddle".into())),
                    ])),
                );
                let assert_equips =
                    |game: &sunrise_client_gameplay::game::ClientGame<'_>, allowed, denied| {
                        for stack in [&saddle, &patched] {
                            assert!(equippable_in_slot(game.items(), stack, "saddle", allowed));
                            assert!(!equippable_in_slot(game.items(), stack, "saddle", denied));
                        }
                    };

                // Configuration stores the tags before the client installs the server tables.
                Arc::make_mut(&mut game.client.remote.as_mut().unwrap().registries)
                    .add_tags(&update(horse));
                game.install_server_items();
                assert_equips(&game, horse, camel);

                game.remote_update_tags(&update(camel));
                assert_equips(&game, camel, horse);
            },
        );
    }

    /// A server whose data pack makes stone climbable and sticks planks: the
    /// client reads its tags, not the game's own (#7843).
    #[test]
    #[ignore = "daily"]
    fn a_servers_block_and_item_tags_replace_the_games() {
        let mut resources = None;
        let mut game = client(&mut resources);
        assert!(!game.items().block_tags["minecraft:climbable"].contains("minecraft:stone"));
        let builtin = game.client.remote.as_ref().unwrap().builtin.clone();
        let id = |registry: &str, name: &str| builtin.get(registry).unwrap().id(name).unwrap();
        let tag = |registry: &str, name: &str, members: &[&str]| RegistryTags {
            registry: registry.into(),
            tags: vec![TagEntry {
                name: name.into(),
                ids: members.iter().map(|m| id(registry, m)).collect(),
            }],
        };
        game.remote_update_tags(&UpdateTags {
            registries: vec![
                tag(
                    "minecraft:block",
                    "minecraft:climbable",
                    &["minecraft:ladder", "minecraft:stone"],
                ),
                tag(
                    "minecraft:item",
                    "minecraft:planks",
                    &["minecraft:stick", "minecraft:oak_planks"],
                ),
            ],
        });

        let items = game.items();
        let set = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<HashSet<_>>();
        assert_eq!(
            items.block_tags["minecraft:climbable"],
            set(&["minecraft:ladder", "minecraft:stone"])
        );
        // The server's tags replace the game's: a tag it didn't send is gone.
        assert!(!items.block_tags.contains_key("minecraft:fences"));
        assert_eq!(
            items.item_tag_order["minecraft:planks"],
            ["minecraft:stick", "minecraft:oak_planks"]
        );
        assert!(items.item_tags["minecraft:planks"].contains("minecraft:stick"));
    }
}
