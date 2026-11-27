#![cfg(all(test, feature = "client"))]
pub(crate) mod tests {
    use crate::game::client_side::remote_tags::tests::login;
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{Hooks, Identifier, ModEntry, TagMembers};
    use std::sync::Arc;
    use sunrise_gameplay_common::game::native_registry_entries::NativeRegistryReads;
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_gameplay_common::game::tables::WorldTablesExt;
    use sunrise_protocol::packets::play::Clientbound as C;

    #[test]
    #[ignore = "daily"]
    fn reload_publishes_retained_and_bound_empty_tags_to_loot_and_clients() {
        use std::path::Path;
        use sunrise_protocol::{
            buf::{Codec, Reader},
            item::Stack,
            packets::recipes::{HolderSet, SlotDisplay, UpdateRecipes},
        };
        use sunrise_world::{level::classes::is_generated_tag, loot::LootTags};
        use {
            sunrise_gameplay::game::lan::sync,
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
            sunrise_gameplay::game::native_entity_refs::tests::with_game,
        };

        const FIXTURE: &str = "SUNRISE_TAG_RELOAD_FIXTURE";
        if run_fresh_cases(
            "cut_layout::game::server_side::remote_tags_server::tests::reload_publishes_retained_and_bound_empty_tags_to_loot_and_clients",
            FIXTURE,
            &["tags", "packed-damage"],
        ) {
            return;
        }
        let packed_damage = std::env::var(FIXTURE).unwrap() == "packed-damage";
        let damage_type = if packed_damage {
            "tag_reload:impact"
        } else {
            "minecraft:stalagmite"
        };
        let save = tempfile::tempdir().unwrap();
        let pack = save.path().join("datapacks/test");
        let write = |path: &Path, bytes: &[u8]| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        write(
            &save.path().join("datapacks.json"),
            br#"{"enabled":["vanilla","file/test"],"disabled":[]}"#,
        );
        write(
            &pack.join("pack.mcmeta"),
            br#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
        );
        let write_tag = |registry: &str, name: &str, bytes: &[u8]| {
            let (ns, path) = name.split_once(':').unwrap();
            write(
                &pack.join(format!("data/{ns}/tags/{registry}/{path}.json")),
                bytes,
            );
        };
        let members = [
            ("item", ["minecraft:potion", "minecraft:stick"]),
            ("block", ["minecraft:stone", "minecraft:dirt"]),
            ("fluid", ["minecraft:water", "minecraft:lava"]),
            ("damage_type", [damage_type, "minecraft:fall"]),
        ];
        if packed_damage {
            write(
                &pack.join("data/tag_reload/damage_type/impact.json"),
                br#"{"message_id":"impact","scaling":"never","exhaustion":0.0}"#,
            );
        }
        write(
            &pack.join("data/tag_reload/advancement/damage_probe.json"),
            br##"{"criteria":{"probe":{"trigger":"minecraft:entity_hurt_player","conditions":{"damage":{"type":{"tags":[{"id":"#tag_reload:probe","expected":true}]}}}}}}"##,
        );
        for (registry, values) in members {
            let json = serde_json::json!({ "values": values });
            write_tag(registry, "tag_reload:probe", json.to_string().as_bytes());
        }
        write_tag(
            "item",
            "tag_reload:old",
            br#"{"values":["minecraft:stick"]}"#,
        );
        write(
            &pack.join("data/tag_reload/loot_table/check.json"),
            br##"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:tag","items":"#tag_reload:probe","expand":false}]}]}"##,
        );
        write(
            &pack.join("data/tag_reload/recipe/retained.json"),
            br##"{"type":"minecraft:stonecutting","ingredient":"#tag_reload:probe","result":{"id":"minecraft:diamond"}}"##,
        );
        let key =
            |name: &str| mod_api::__private::tag_key(&Identifier::parse(name).unwrap()).unwrap();
        fn resolved<K>(ids: &[&str], resolve: impl Fn(&str) -> Option<K>) -> TagMembers<K> {
            TagMembers::Members(ids.iter().map(|id| resolve(id).unwrap()).collect())
        }
        let check_host = |game: &dyn mod_api::Tags, keep: usize| {
            let probe = key("tag_reload:probe");
            assert_eq!(
                mod_api::Tags::items(game, probe.typed()),
                resolved(&members[0].1[..keep], mod_api::__private::item_key)
            );
            assert_eq!(
                mod_api::Tags::blocks(game, probe.typed()),
                resolved(&members[1].1[..keep], mod_api::__private::block_key)
            );
            assert_eq!(
                mod_api::Tags::fluids(game, probe.typed()),
                resolved(&members[2].1[..keep], mod_api::__private::fluid_key)
            );
            assert_eq!(
                mod_api::Tags::items(game, key("tag_reload:old").typed()),
                if keep == 0 {
                    TagMembers::Missing
                } else {
                    resolved(&["minecraft:stick"], mod_api::__private::item_key)
                }
            );
        };
        let mut snapshots = Vec::new();
        with_game(|game| {
            let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
                "tag-reload",
                Some("test"),
                Some(1),
                true,
                None,
                std::sync::Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
            )
            .unwrap();
            request.save_root = save.path().to_owned();
            game.load_world(request).unwrap();
            let rejected = br#"{"replace":true,"values":["tag_reload:missing_member"]}"#;
            for (registry, tags) in game.items().ordered_tags() {
                for tag in tags
                    .keys()
                    .filter(|tag| registry != "block" || !is_generated_tag(tag))
                {
                    write_tag(registry, tag, rejected);
                }
            }
            for tag in game.health_resources.damage.tags() {
                write_tag("damage_type", tag, rejected);
            }
            // Definitions force an eager wire load; a tag-only world still exercises
            // reload before the first wire snapshot. Neither case has a joined client.
            assert_eq!(sync::cached_reload_sync().is_some(), packed_damage);
            for expect_empty in [false, true] {
                if expect_empty {
                    // These recipes name item tags that this stage deliberately removes.
                    let recipes = sunrise_world::json_folder::read(
                        &game.options.resources.join("data/minecraft/recipe"),
                    )
                    .unwrap();
                    for file in &recipes.files {
                        write(
                            &pack.join(format!("data/minecraft/recipe/{}.json", file.name)),
                            br#"{"neoforge:conditions":[{"type":"neoforge:never"}]}"#,
                        );
                    }
                    for (registry, _) in members {
                        write_tag(
                            registry,
                            "tag_reload:probe",
                            br#"{"replace":true,"values":[]}"#,
                        );
                    }
                }
                game.reload_datapacks(true, &game.level.enabled_features.clone())
                    .unwrap();
                let keep = if expect_empty { 0 } else { 2 };
                check_host(game, keep);
                let entry = game
                    .registry_entry("minecraft:damage_type", damage_type)
                    .unwrap();
                assert_eq!(
                    game.registry_entry_in_tag(&entry, "tag_reload:probe"),
                    Some(!expect_empty)
                );
                game.trigger_entity_hurt_player(&sunrise_sim::advancements::DamageView {
                    kind: damage_type.into(),
                    ..Default::default()
                });
                assert_eq!(
                    game.adv.progress.revoke("tag_reload:damage_probe", "probe"),
                    !expect_empty
                );
                let loot = game.server.server_loot.0.get().unwrap();
                for (registry, values) in members {
                    let expected: Vec<_> = values[..keep].iter().map(|id| id.to_string()).collect();
                    assert_eq!(loot.members(registry, "tag_reload:probe"), Some(expected));
                }
                let drops = loot.roll(
                    "tag_reload:check",
                    &mut sunrise_core::random::LegacyRandom::new(0),
                    0.0,
                    None,
                    None,
                );
                assert_eq!(
                    drops
                        .iter()
                        .map(|drop| drop.item.as_str())
                        .collect::<Vec<_>>(),
                    members[0].1[..keep]
                );
                let wire = sync::sync_data(
                    &game.options.resources,
                    &game.res.core.item_data,
                    game.items(),
                    &game.pack_files(),
                )
                .unwrap();
                if !expect_empty {
                    let recipe = UpdateRecipes::read(&mut Reader::new(&wire.recipes)).unwrap();
                    let diamond = wire.id("minecraft:item", "minecraft:diamond").unwrap();
                    assert!(recipe.stonecutter.contains(&(
                        HolderSet::Tag("tag_reload:probe".into()),
                        SlotDisplay::ItemStack(Stack::new(diamond, 1)),
                    )));
                    for item in members[0].1 {
                        assert_eq!(
                            wire.stonecutter_buttons.recipe(item, 0),
                            Some("tag_reload:retained")
                        );
                    }
                }
                snapshots.push(wire);
            }
        });
        // The client only installs tag tables for Host reads when there is a mod.
        struct TagReader;
        #[mod_api::__subscriptions]
        impl Hooks for TagReader {}
        mod_platform::testing::with_mods(
            vec![ModEntry::new("tag_reload", Box::new(TagReader))].into(),
            || {
                with_client_game(|game| {
                    let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(pipe, "Steve").unwrap();
                    let remote = game.client.remote.as_mut().unwrap();
                    remote.registries = Arc::new(snapshots[0].dynamic.clone());
                    Arc::make_mut(&mut remote.registries).add_tags(&snapshots[0].tags);
                    game.handle_packet(login()).unwrap();
                    check_host(game, 2);
                    let ids = members[3]
                        .1
                        .map(|id| snapshots[0].id("minecraft:damage_type", id).unwrap());
                    assert_eq!(
                        game.client
                            .remote
                            .as_ref()
                            .unwrap()
                            .registries
                            .tag("minecraft:damage_type", "tag_reload:probe"),
                        Some(&ids[..])
                    );
                    game.handle_packet(C::UpdateTags(snapshots[1].tags.clone()))
                        .unwrap();
                    check_host(game, 0);
                    assert_eq!(
                        game.client
                            .remote
                            .as_ref()
                            .unwrap()
                            .registries
                            .tag("minecraft:damage_type", "tag_reload:probe"),
                        Some(&[][..])
                    );
                });
            },
        );
    }
}
