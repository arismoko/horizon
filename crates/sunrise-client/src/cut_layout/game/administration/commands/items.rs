#![cfg(test)]
use sunrise_items::items::ItemStack;
#[cfg(test)]
pub(crate) mod stand_equip_event_tests {
    use std::sync::Arc;
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::{chunk::Chunk, containers::BlockEntity, sculk::Info};
    use sunrise_sim::mobs::Who;
    use {
        crate::game::{Settings, load_resources},
        sunrise_gameplay_common::game::{Options, Progress},
    };

    fn candidate(game: &sunrise_gameplay::game::ServerGame<'_>) -> Option<Info> {
        let Some(BlockEntity::Sculk(sensor)) = game.world.block_entities.get(&(9, 200, 10)) else {
            panic!("actual sensor must be registered")
        };
        sensor.vibration.selector.current.map(|(info, _)| info)
    }

    #[test]
    #[ignore = "daily"]
    fn commands_post_stand_equipment_events_after_its_first_tick() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir =
            Some(std::env::temp_dir().join(format!("sunrise-h130-unit-{}", std::process::id())));
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let registry = game.pack.assets.registry;
        let sensor = registry.default_state("minecraft:sculk_sensor").unwrap();
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(9, 200, 10, sensor.0);
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("summon armor_stand 6.5 200 10.5 {UUID:[I;130,0,0,1],NoGravity:1b,attributes:[{id:\"minecraft:movement_speed\",base:0.1d}]}"));
        let id = game.armor_stands[0].id;
        assert_eq!(game.uuid_for(id), [130, 0, 0, 1]);
        assert!(
            game.armor_stands[0].speed_instance,
            "a loaded attribute is not firstTick"
        );
        game.sculk_prepare();
        game.sculk_sync_index();
        assert!(candidate(&game).is_none());
        let prefix = "item replace entity 00000082-0000-0000-0000-000000000001 ";
        for item in ["iron_helmet", "air"] {
            assert!(game.run_command(&format!("{prefix}armor.head with {item}")));
            assert!(candidate(&game).is_none(), "first-tick suppression");
        }
        game.tick_armor_stands().unwrap();
        assert!(
            game.run_command(
                "data merge entity 00000082-0000-0000-0000-000000000001 {ShowArms:1b}"
            )
        );
        for (slot, item, event) in [
            ("armor.head", "iron_helmet", Some("minecraft:equip")),
            ("armor.head", "iron_helmet", None),
            ("armor.head", "air", Some("minecraft:unequip")),
            ("armor.head", "air", None),
            ("weapon.mainhand", "stick", Some("minecraft:unequip")),
        ] {
            let Some(BlockEntity::Sculk(sensor)) = game.world.block_entities.get_mut(&(9, 200, 10))
            else {
                unreachable!()
            };
            sensor.vibration = Default::default();
            assert!(game.run_command(&format!("{prefix}{slot} with {item}")));
            let heard = candidate(&game);
            assert_eq!(heard.map(|info| info.event), event, "{slot} -> {item}");
            if let Some(info) = heard {
                assert_eq!(info.source, Some(Who::Mob(id).into()));
                assert_eq!(info.pos, [6.5, 200.0, 10.5]);
                assert!(info.owner.is_none());
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod mob_equipment_command_tests {
    use std::sync::Arc;
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_level::chunk::Chunk;
    use {
        crate::game::{Settings, load_resources},
        sunrise_gameplay_common::game::{Options, Progress},
    };
    use {
        sunrise_gameplay::game::commands::data::DataRef,
        sunrise_gameplay_common::game::commands_common::Target,
    };

    pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h025-{:?}", std::thread::current().id())),
        );
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("difficulty normal"));
        assert!(game.run_command("forceload add 0 0"));
        test(&mut game);
    }

    pub(crate) fn actor(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        kind: &str,
        n: i32,
    ) -> (String, u32) {
        let uuid = format!("00000019-0000-0000-0000-{n:012x}");
        let prev = game.server.chat_capture.replace(Vec::new());
        let ok = game.run_command(&format!(
            "summon {kind} 4.5 200 5.5 {{UUID:[I;25,0,0,{n}],NoAI:1b,NoGravity:1b,Silent:1b,h025_raw:7,Tags:[h025]}}"
        ));
        let lines = std::mem::replace(&mut game.server.chat_capture, prev).unwrap_or_default();
        assert!(ok, "{kind}/{n}: {lines:?}");
        let mob = game
            .mobs
            .iter()
            .find(|m| game.persist.uuid_of(m.id) == Some([25, 0, 0, n]))
            .unwrap();
        assert!(mob.no_ai);
        (uuid, mob.id)
    }

    #[test]
    #[ignore = "daily"]
    fn body_commands_reject_wrong_items_without_changing_head() {
        with_game(|game| {
            for (n, kind, slot, item) in [
                (1, "horse", "armor.body", "iron_horse_armor"),
                (2, "wolf", "armor.body", "wolf_armor"),
                (3, "husk", "armor.head", "iron_helmet"),
            ] {
                let (uuid, id) = actor(game, kind, n);
                assert!(
                    game.run_command(&format!("item replace entity {uuid} {slot} with {item}"))
                );
                let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
                let actual = if slot == "armor.body" {
                    &mob.extra.equipment[sunrise_items::equipment::BODY]
                } else {
                    &mob.extra.equipment[sunrise_items::equipment::HEAD]
                };
                assert_eq!(
                    actual.as_ref().map(|s| &*s.item),
                    Some(format!("minecraft:{item}").as_str())
                );
            }
            for (n, kind) in [(4, "husk"), (5, "pig")] {
                let (uuid, id) = actor(game, kind, n);
                assert!(
                    game.run_command(&format!("item replace entity {uuid} armor.head with stone"))
                );
                assert!(
                    !game.run_command(&format!(
                        "item replace entity {uuid} armor.body with diamond_chestplate"
                    )),
                    "body must reject a chest item"
                );
                let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
                assert!(
                    mob.extra
                        .slot_stack(sunrise_items::equipment::BODY)
                        .is_none()
                );
                assert_eq!(mob.extra.head_id(), Some("minecraft:stone"));
                let saved = game.data_of(&DataRef::Entity(Target::Mob(id))).unwrap();
                assert!(saved.get("equipment").unwrap().get("body").is_none());
                assert_eq!(
                    saved
                        .get("equipment")
                        .unwrap()
                        .get("head")
                        .unwrap()
                        .get("id"),
                    Some(&sunrise_core::nbt::Tag::String("minecraft:stone".into()))
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn saddle_commands_round_trip_existing_model_and_saved_equipment() {
        use sunrise_core::nbt::Tag;
        use sunrise_gameplay::game::server_side::commands::items::Slot;
        with_game(|game| {
            // Pig is the existing species saver; Husk needs the common projection.
            for (n, kind) in [(6, "pig"), (7, "husk")] {
                let (uuid, id) = actor(game, kind, n);
                assert!(game.slot_get(&Slot::Mob(id, "saddle")).is_none());
                assert!(
                    game.run_command(&format!("item replace entity {uuid} armor.head with stone"))
                );
                assert!(
                    game.run_command(&format!("item replace entity {uuid} saddle with saddle")),
                    "command must reach the existing saddle model"
                );
                assert_eq!(
                    game.slot_get(&Slot::Mob(id, "saddle"))
                        .unwrap()
                        .item
                        .as_ref(),
                    "minecraft:saddle"
                );
                let saved = game.data_of(&DataRef::Entity(Target::Mob(id))).unwrap();
                assert_eq!(
                    saved
                        .get("equipment")
                        .unwrap()
                        .get("saddle")
                        .unwrap()
                        .get("id"),
                    Some(&Tag::String("minecraft:saddle".into()))
                );
                assert_eq!(
                    saved
                        .get("equipment")
                        .unwrap()
                        .get("head")
                        .unwrap()
                        .get("id"),
                    Some(&Tag::String("minecraft:stone".into()))
                );
                assert_eq!(saved.get("h025_raw"), Some(&Tag::Int(7)));
                assert_eq!(saved.get("Silent"), Some(&Tag::Byte(1)));
                assert_eq!(
                    saved.get("Tags"),
                    Some(&Tag::List(vec![Tag::String("h025".into())]))
                );
                assert!(
                    !game.run_command(&format!("item replace entity {uuid} saddle with stick"))
                );
                assert_eq!(
                    game.slot_get(&Slot::Mob(id, "saddle"))
                        .unwrap()
                        .item
                        .as_ref(),
                    "minecraft:saddle"
                );
                assert!(game.run_command(&format!("item replace entity {uuid} saddle with air")));
                assert!(game.slot_get(&Slot::Mob(id, "saddle")).is_none());
            }
        });
    }
    /// `SulfurCube.getEquipmentSlotForItem`: a `#sulfur_cube_swallowable`
    /// item is a body item, so the body's `SlotAccess` takes it (#12038).
    #[test]
    #[ignore = "daily"]
    fn a_sulfur_cubes_body_takes_a_swallowable_block() {
        with_game(|game| {
            assert!(game.run_command("summon sulfur_cube 4.5 200 4.5 {NoAI:1b,Tags:[\"cube\"]}"));
            let body = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let m = game
                    .mobs
                    .iter()
                    .find(|m| m.kind.id == "minecraft:sulfur_cube");
                m.and_then(|m| {
                    m.extra.equipment[sunrise_items::equipment::BODY]
                        .as_ref()
                        .map(|stack| stack.item.clone())
                })
            };
            let replace = "item replace entity @e[tag=cube,limit=1] armor.body with";
            assert!(game.run_command(&format!("{replace} white_wool")));
            assert_eq!(body(game).as_deref(), Some("minecraft:white_wool"));
            // An item outside the tag stays out, and the body stays on.
            assert!(!game.run_command(&format!("{replace} diamond")));
            assert_eq!(body(game).as_deref(), Some("minecraft:white_wool"));
            // An empty stack always fits: the body comes off.
            assert!(game.run_command(&format!("{replace} air")));
            assert_eq!(body(game), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn effective_item_composition_rejects_before_destination_mutation() {
        use sunrise_gameplay::game::server_side::commands::items::Slot;
        with_game(|game| {
            assert!(game.run_command("setblock 4 200 4 chest"));
            let slot = Slot::Block([4, 200, 4], 0);
            for item in [
                "iron_helmet[max_stack_size=2]",
                "iron_helmet[max_stack_size=99]",
                "stone[max_damage=10,max_stack_size=2]",
            ] {
                assert!(game.run_command("item replace block 4 200 4 container.0 with emerald[custom_data={sentinel:86}] 4"));
                let before = game.slot_get(&slot).unwrap();
                assert!(
                    !game.run_command(&format!(
                        "item replace block 4 200 4 container.0 with {item}"
                    )),
                    "invalid effective composition must reject {item}"
                );
                assert_eq!(game.slot_get(&slot), Some(before));
            }
            for item in [
                "iron_helmet",
                "stone[max_stack_size=2]",
                "stone[max_stack_size=99]",
                "iron_helmet[!max_damage,max_stack_size=2]",
            ] {
                assert!(
                    game.run_command(&format!(
                        "item replace block 4 200 4 container.0 with {item}"
                    )),
                    "valid effective composition must accept {item}"
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn effective_item_validation_view_uses_loaded_prototypes_and_typed_removals() {
        use sunrise_core::nbt::Tag;
        with_game(|game| {
            let damage = "minecraft:max_damage".to_owned();
            let size = "minecraft:max_stack_size".to_owned();
            let helmet = game.items().get("minecraft:iron_helmet").unwrap();
            assert!(helmet.components.contains(&damage));
            let plain = game
                .item_validation_components("minecraft:iron_helmet", None)
                .unwrap();
            assert_eq!(
                plain.get(&damage),
                Some(&Tag::Int(helmet.max_damage as i32))
            );
            assert_eq!(plain.get(&size), Some(&Tag::Int(helmet.max_stack as i32)));
            let removed = game
                .item_validation_components(
                    "minecraft:iron_helmet",
                    Some("[!max_damage,max_stack_size=2,custom_data={keep:86}]"),
                )
                .unwrap();
            assert!(removed.get(&damage).is_none());
            assert_eq!(removed.get(&size), Some(&Tag::Int(2)));
            assert!(removed.get(&"minecraft:custom_data".to_owned()).is_some());
            let fallback = game
                .item_validation_components("minecraft:iron_helmet", Some("[!max_stack_size]"))
                .unwrap();
            assert!(fallback.get(&damage).is_some());
            assert_eq!(
                sunrise_world::datafix::dynamic::as_int(fallback.get(&size), 1),
                1
            );
            let stone = game
                .item_validation_components("minecraft:stone", None)
                .unwrap();
            assert!(stone.get(&damage).is_none());
            assert_eq!(
                stone.get(&size),
                Some(&Tag::Int(
                    game.items().get("minecraft:stone").unwrap().max_stack as i32
                ))
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod mob_equipment_stack_tests {
    use std::sync::Arc;
    use sunrise_core::nbt::Tag;
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::mobs::finalize;
    use {
        crate::game::{Settings, load_resources},
        sunrise_gameplay_common::game::{Options, Progress},
    };
    use {
        sunrise_gameplay::game::commands::data::DataRef,
        sunrise_gameplay_common::game::commands_common::Target,
    };
    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir = Some(
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
                .join(format!("h023-{:?}", std::thread::current().id())),
        );
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("difficulty normal"));
        assert!(game.run_command("forceload add 0 0"));
        test(&mut game);
    }

    fn actor(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        kind: &str,
        n: i32,
    ) -> (String, u32) {
        let uuid = format!("00000017-0000-0000-0000-{n:012x}");
        let prev = game.server.chat_capture.replace(Vec::new());
        let ok = game.run_command(&format!(
            "summon {kind} 4.5 200 5.5 {{UUID:[I;23,0,0,{n}],NoAI:1b,NoGravity:1b,Silent:1b,h023_raw:7,Tags:[h023]}}"
        ));
        let lines = std::mem::replace(&mut game.server.chat_capture, prev).unwrap_or_default();
        assert!(ok, "{kind}/{n}: {lines:?}");
        let mob = game
            .mobs
            .iter()
            .find(|m| game.persist.uuid_of(m.id) == Some([23, 0, 0, n]))
            .unwrap();
        assert!(mob.no_ai);
        (uuid, mob.id)
    }

    fn saved(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32) -> Tag {
        game.data_of(&DataRef::Entity(Target::Mob(id))).unwrap()
    }
    fn audit(stack: &sunrise_items::items::ItemStack, value: i32) {
        assert_eq!(
            stack.custom_data().and_then(|c| c.get(&"audit".into())),
            Some(&Tag::Int(value))
        );
    }
    #[test]
    #[ignore = "daily"]
    fn offhand_commands_adopt_and_copy_the_whole_stack() {
        use sunrise_gameplay::game::server_side::commands::items::Slot;
        with_game(|game| {
            let (uuid, id) = actor(game, "husk", 1);
            assert!(game.run_command("setblock 4 200 4 chest"));
            assert!(game.slot_get(&Slot::Mob(id, "offhand")).is_none());
            assert!(game.run_command(&format!(
                "item replace entity {uuid} weapon.mainhand with stone"
            )));
            assert!(game.run_command(&format!(
                "item replace entity {uuid} weapon.offhand with stick[custom_data={{audit:7}}] 3"
            )));
            let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
            assert_eq!(mob.extra.off_hand_id(), Some("minecraft:stick"));
            let adopted = finalize::item_stack(mob, finalize::OFFHAND).unwrap();
            assert_eq!(
                adopted.count, 3,
                "actual command adoption must retain count"
            );
            audit(&adopted, 7);
            let tag = saved(game, id);
            assert_eq!(
                tag.get("equipment")
                    .unwrap()
                    .get("offhand")
                    .unwrap()
                    .get("count"),
                Some(&Tag::Int(3))
            );
            assert_eq!(
                tag.get("equipment")
                    .unwrap()
                    .get("mainhand")
                    .unwrap()
                    .get("id"),
                Some(&Tag::String("minecraft:stone".into()))
            );
            assert!(game.run_command(&format!(
                "item replace block 4 200 4 container.0 from entity {uuid} weapon.offhand"
            )));
            let copied = game.slot_get(&Slot::Block([4, 200, 4], 0)).unwrap();
            assert_eq!(copied.count, 3);
            audit(&copied, 7);
            assert!(game.run_command(&format!(
                "item replace entity {uuid} weapon.offhand with air"
            )));
            assert!(game.slot_get(&Slot::Mob(id, "offhand")).is_none());
            assert!(game.run_command(&format!(
                "item replace entity {uuid} weapon.offhand with stick"
            )));
            let plain = game.slot_get(&Slot::Mob(id, "offhand")).unwrap();
            assert_eq!(plain.count, 1);
            assert!(
                plain.custom_data().is_none(),
                "stale components must not return for the same id"
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn armor_command_copy_keeps_the_already_adopted_stack() {
        use sunrise_gameplay::game::server_side::commands::items::Slot;
        with_game(|game| {
            let (uuid, id) = actor(game, "husk", 2);
            assert!(game.run_command("setblock 4 200 4 chest"));
            assert!(game.run_command(&format!("item replace entity {uuid} armor.head with iron_helmet[damage=3,custom_data={{audit:8}}]")));
            let mob = game.mobs.iter().find(|m| m.id == id).unwrap();
            let stacks = mob.equipment_stacks();
            let adopted = stacks[5].as_ref().unwrap();
            assert_eq!(
                adopted.damage, 3,
                "existing armor state already adopts the full stack"
            );
            audit(adopted, 8);
            assert!(game.run_command(&format!(
                "item replace block 4 200 4 container.0 from entity {uuid} armor.head"
            )));
            let copied = game.slot_get(&Slot::Block([4, 200, 4], 0)).unwrap();
            assert_eq!(
                copied.damage, 3,
                "actual command copy must use the full stack"
            );
            audit(&copied, 8);
            let tag = saved(game, id);
            let components = tag
                .get("equipment")
                .unwrap()
                .get("head")
                .unwrap()
                .get("components")
                .unwrap();
            assert_eq!(components.get("minecraft:damage"), Some(&Tag::Int(3)));
            assert_eq!(
                components
                    .get("minecraft:custom_data")
                    .unwrap()
                    .get("audit"),
                Some(&Tag::Int(8))
            );
            assert!(game.run_command(&format!(
                "item replace entity {uuid} armor.head with iron_helmet"
            )));
            let plain = game.slot_get(&Slot::Mob(id, "head")).unwrap();
            assert_eq!(plain.damage, 0);
            assert!(plain.custom_data().is_none());
            assert!(game.run_command(&format!("item replace entity {uuid} armor.head with air")));
            assert!(game.slot_get(&Slot::Mob(id, "head")).is_none());
        });
    }
}

#[cfg(test)]
pub(crate) mod builtin_component_delivery_tests {
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    use sunrise_core::nbt::Tag;
    use sunrise_core::position::ChunkPos;
    use sunrise_core::vanilla_bytes::Writer;
    use sunrise_gameplay::game::lan::LanHost;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_protocol::{
        buf::{Codec, Reader},
        item::{Slot, component},
        packets::{configuration::ClientInformation, play},
    };
    use sunrise_sim::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};

    #[test]
    #[ignore = "daily"]
    fn h283_actual_command_inventory_bridge_encodes_four_builtin_components() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.build_registries();
            game.set_allow_commands(true);
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            assert!(game.world.loaded(8, 8));
            game.lan_host(
                sunrise_gameplay::game::lan::LanSettings {
                    port: 0,
                    guest_commands: true,
                    force_game_mode: false,
                },
                NetConfig {
                    announce: false,
                    ..NetConfig::loopback(0, "component-wire-test".into(), 2)
                },
                HubConfig {
                    default_game_mode: 0,
                    guest_commands: true,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: "minecraft:overworld".into(),
                    allow_flight: false,
                    seed: 0,
                    max_players: 2,
                    integrated: false,
                    levels: Default::default(),
                    level_seeds: Default::default(),
                },
            )
            .unwrap();
            let (tx, rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 2,
                    uuid: 283,
                    name: "ComponentGuest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                2,
                [8.5, 200.0, 8.5],
                2,
            );
            guest.dimension = "minecraft:overworld".into();
            guest.placed = true;
            guest.loaded = true;
            guest.selected = 1;
            guest.permission = 2;
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
            let clock = game.world.game_time;
            let root =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let registries: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.join("registries.json")).unwrap())
                    .unwrap();
            let id = |registry: &str, name: &str| {
                registries[registry]["entries"][name]["protocol_id"]
                    .as_i64()
                    .unwrap() as i32
            };
            let command = |game: &mut sunrise_gameplay::game::ServerGame<'_>, input: &str| {
                assert!(
                    game.as_guest(2, |g| g.run_command(input)).unwrap(),
                    "{input}"
                );
            };
            let capture = |host: &mut LanHost| {
                while rx.try_recv().is_ok() {}
                host.hub.send_inventory(0);
                let mut packets = Vec::new();
                for out in rx.try_iter() {
                    match out {
                        Out::Packet(body) => packets.push(body),
                        Out::Packets(bodies) => packets.extend(bodies),
                        Out::Disconnect(_) => panic!("unexpected disconnect"),
                    }
                }
                let mut contents = Vec::new();
                for body in packets {
                    let mut r = Reader::new(&body);
                    let packet = r.varint().unwrap();
                    if packet == sunrise_protocol::ids::play::clientbound::CONTAINER_SET_CONTENT {
                        let content = play::ContainerSetContent::read(&mut r).unwrap();
                        assert_eq!(r.remaining(), 0, "complete enclosing content payload");
                        assert_eq!(content.container, 0);
                        assert_eq!(content.items.len(), 46);
                        assert!(content.carried.is_none());
                        contents.push(content);
                    }
                }
                assert_eq!(contents.len(), 1, "one actual inventory content packet");
                contents.pop().unwrap().items[9].clone().unwrap()
            };
            command(
                game,
                "item replace entity @s inventory.0 with stone[minecraft:custom_name=\"H283 control\"] 3",
            );
            let control = capture(game.server.lan.as_mut().unwrap());
            assert_eq!(control.item, id("minecraft:item", "minecraft:stone"));
            assert_eq!(control.count, 3);
            assert!(control.component(component::CUSTOM_NAME).is_some());
            command(game, "item replace entity @s inventory.0 with gold_ingot");
            let plain = capture(game.server.lan.as_mut().unwrap());
            assert!(
                plain
                    .component(id(
                        "minecraft:data_component_type",
                        "minecraft:provides_trim_material"
                    ))
                    .is_none()
            );
            assert!(plain.removed.is_empty());
            let mut failures = Vec::new();
            for (name, item, value, expected) in [
                (
                    "minecraft:provides_trim_material",
                    "minecraft:diamond",
                    "\"minecraft:gold\"",
                    Tag::String("minecraft:gold".into()),
                ),
                (
                    "minecraft:provides_banner_patterns",
                    "minecraft:paper",
                    "\"#minecraft:pattern_item/globe\"",
                    Tag::String("#minecraft:pattern_item/globe".into()),
                ),
                (
                    "minecraft:can_place_on",
                    "minecraft:stone",
                    "{blocks:\"minecraft:dirt\"}",
                    Tag::compound([("blocks", Tag::String("minecraft:dirt".into()))]),
                ),
                (
                    "minecraft:can_break",
                    "minecraft:diamond_pickaxe",
                    "{blocks:\"minecraft:stone\",state:{}}",
                    Tag::compound([
                        ("blocks", Tag::String("minecraft:stone".into())),
                        ("state", Tag::compound(std::iter::empty::<(&str, Tag)>())),
                    ]),
                ),
            ] {
                command(
                    game,
                    &format!("item replace entity @s inventory.0 with {item}[{name}={value}]"),
                );
                let source: ItemStack = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(2)
                    .unwrap()
                    .inventory[9]
                    .clone()
                    .unwrap();
                assert_eq!(source.item.as_ref(), item);
                assert_eq!(source.count, 1);
                assert_eq!(
                    source.data().unwrap().other(name),
                    Some(&expected),
                    "actual registered command adoption"
                );
                assert!(source.wire_patch().is_none(), "no cached wire fallback");
                let received = capture(game.server.lan.as_mut().unwrap());
                assert_eq!(received.item, id("minecraft:item", item));
                assert_eq!(received.count, 1);
                assert!(received.removed.is_empty());
                let kind = id("minecraft:data_component_type", name);
                let mut w = Writer::new();
                Slot::write(&Some(received.clone()), &mut w);
                let mut r = Reader::new(&w.data);
                let reread = Slot::read(&mut r).unwrap().unwrap();
                assert_eq!(r.remaining(), 0, "complete nested slot payload");
                assert_eq!(reread.component(kind), received.component(kind));
                if received.component(kind).is_none() {
                    failures.push((name, received));
                }
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guest(2).unwrap().uuid,
                    283
                );
                assert_eq!(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(2)
                        .unwrap()
                        .health,
                    20.0
                );
                assert_eq!(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(2)
                        .unwrap()
                        .position,
                    [8.5, 200.0, 8.5]
                );
                assert_eq!(game.world.game_time, clock);
            }
            assert!(
                failures.is_empty(),
                "actual command -> network_stack -> full protocol packet missing builtin components: {failures:?}"
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod component_modifier_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn item_modify_removal_hides_the_prototype_and_a_set_clears_the_marker() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let stack = ItemStack::new("minecraft:bread", 1);
            assert_eq!(stack.villager_food(game.items().get(&stack.item)), Some(4));
            let mut random = sunrise_core::random::LegacyRandom::new(1);
            let remove = serde_json::json!({"type":"minecraft:set_components", "components":{"!villager_food":{}}});
            let removed = game
                .apply_item_modifier(&remove, Some(stack), &mut random, None, [0.0; 3], 0)
                .unwrap();
            assert_eq!(removed.villager_food(game.items().get(&removed.item)), None);
            let set = serde_json::json!({"type":"minecraft:set_components", "components":{"minecraft:villager_food":{"nutrition":7}}});
            let result = game
                .apply_item_modifier(&set, Some(removed), &mut random, None, [0.0; 3], 0)
                .unwrap();
            assert_eq!(
                result.villager_food(game.items().get(&result.item)),
                Some(7)
            );
            assert!(
                result
                    .data()
                    .unwrap()
                    .other("!minecraft:villager_food")
                    .is_none()
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod registered_slot_source_preservation_tests {
    use sunrise_gameplay_common::game::commands_common::Slot;

    use sunrise_gameplay::game::server_side::commands::items::registered_slot_source_preservation_tests::count;

    pub(crate) use crate::game::server_side::commands_server::items::registered_slot_source_preservation_tests::scene;

    #[test]
    #[ignore = "daily"]
    fn loaded_snapshot_changes_only_on_existing_reload_with_pack_precedence() {
        let mut resources = None;
        let directory = tempfile::tempdir().unwrap();
        let (mut game, file) = scene(
            &mut resources,
            directory.path().join("slot-source-snapshot-world"),
        );
        let before = game.block_entity_nbt([0, 100, 0]).unwrap();
        let clock = game.world.game_time;
        count(&mut game, 3); // Higher-priority pack overrides the same id.
        std::fs::write(&file, r#"{"type":"minecraft:limit_slots","limit":2,"slot_source":{"type":"minecraft:slot_range","slots":"container.*"}}"#).unwrap();
        count(&mut game, 3); // Disk edits do not change the loaded snapshot.
        game.reload_datapacks(true, &game.level.enabled_features.clone())
            .unwrap();
        count(&mut game, 2);
        std::fs::write(file, "{").unwrap();
        assert!(
            game.reload_datapacks(true, &game.level.enabled_features.clone())
                .is_err()
        );
        count(&mut game, 2); // A refused reload keeps the loaded data (ReloadableServerRegistries).
        assert!(!game.run_command("execute if slots block 0 100 0 first_three"));
        assert_eq!(game.block_entity_nbt([0, 100, 0]).as_ref(), Some(&before));
        assert_eq!(game.world.game_time, clock);
    }
    /// SlotSources.CODEC is a holder codec: a quoted id is the same reference
    /// as an unquoted one, and so is a group's single quoted term.
    /// LimitSlotSource's limit is ExtraCodecs.POSITIVE_INT: a string or 0
    /// fails to parse, and the command changes nothing.
    #[test]
    #[ignore = "daily"]
    fn quoted_holders_resolve_and_a_non_int_limit_fails_to_parse() {
        let mut resources = None;
        let directory = tempfile::tempdir().unwrap();
        let (mut game, _) = scene(
            &mut resources,
            directory.path().join("slot-source-quoted-world"),
        );
        let before = game.block_entity_nbt([0, 100, 0]).unwrap();
        let clock = game.world.game_time;
        count(&mut game, 3);
        for source in [
            r#""parity:first_three""#,
            r#"{type:"group",terms:"parity:first_three"}"#,
        ] {
            assert!(game.run_command("scoreboard players set #q slotprobe -99"));
            assert!(game.run_command(&format!(
                "execute store result score #q slotprobe run execute if slots block 0 100 0 {source}"
            )));
            assert_eq!(
                game.score.board.score("#q", "slotprobe").unwrap().value,
                3,
                "{source}"
            );
        }
        for limit in [r#""x""#, "0"] {
            assert!(game.run_command("scoreboard players set #q slotprobe -99"));
            let command = format!(
                "execute store result score #q slotprobe run execute if slots block 0 100 0 {{type:\"limit_slots\",limit:{limit},slot_source:{{type:\"slot_range\",slots:\"container.*\"}}}}"
            );
            assert!(!game.run_command(&command), "{command}");
            assert_eq!(
                game.score.board.score("#q", "slotprobe").unwrap().value,
                -99,
                "{command}"
            );
        }
        assert_eq!(game.block_entity_nbt([0, 100, 0]).as_ref(), Some(&before));
        assert_eq!(game.world.game_time, clock);
    }
    /// `Codec.INT` gives `limit:1b` and `limit:2.5d` their `Number.intValue()`:
    /// a read counts that many slots and a write changes that many.
    #[test]
    #[ignore = "daily"]
    fn a_byte_or_fractional_limit_selects_its_int_value() {
        let mut resources = None;
        let directory = tempfile::tempdir().unwrap();
        let (mut game, _) = scene(
            &mut resources,
            directory.path().join("slot-source-numeric-limit-world"),
        );
        let source = |limit: &str| {
            format!(
                "{{type:\"limit_slots\",limit:{limit},slot_source:{{type:\"slot_range\",slots:\"container.*\"}}}}"
            )
        };
        for (limit, slots) in [("1b", 1), ("2.5d", 2)] {
            assert!(game.run_command("scoreboard players set #q slotprobe -99"));
            assert!(game.run_command(&format!(
                "execute store result score #q slotprobe run execute if slots block 0 100 0 {}",
                source(limit)
            )));
            assert_eq!(
                game.score.board.score("#q", "slotprobe").unwrap().value,
                slots,
                "{limit}"
            );
        }
        // `/item override` writes the stack over the selected slots and no further:
        // one slot for `1b`, two for `2.5d`, the stone in the fourth slot never.
        let held = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            [0, 1, 3].map(|slot| {
                game.slot_get(&Slot::Block([0, 100, 0], slot))
                    .map(|stack| (stack.item.to_string(), stack.count))
            })
        };
        let stack = |item: &str, count| Some((format!("minecraft:{item}"), count));
        assert!(game.run_command(&format!(
            "item override block 0 100 0 {} with minecraft:diamond 2",
            source("1b")
        )));
        assert_eq!(
            held(&mut game),
            [stack("diamond", 2), stack("dirt", 2), stack("stone", 7)]
        );
        assert!(game.run_command(&format!(
            "item override block 0 100 0 {} with minecraft:emerald 2",
            source("2.5d")
        )));
        assert_eq!(
            held(&mut game),
            [stack("emerald", 2), None, stack("stone", 7)]
        );
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
