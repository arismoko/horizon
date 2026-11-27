#![cfg(test)]
#[cfg(test)]
#[path = "attribute_missing_tests.rs"]
pub(crate) mod attribute_missing_tests;

#[cfg(test)]
#[path = "registered_damage_tests.rs"]
pub(crate) mod registered_damage_tests;

#[cfg(test)]
pub(crate) mod datapack_damage_tests {

    use sunrise_gameplay::game::server_side::commands::world_cmds::datapack_damage_tests::with_pack;

    use sunrise_gameplay::game::server_side::commands_server::world_cmds::datapack_damage_tests::pig;

    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::commands_common::Target;

    use sunrise_core::geometry::Vec3;

    pub(crate) use crate::game::server_side::commands_server::world_cmds::datapack_damage_tests::run_a_datapack_explosion_type_removes_a_crystal_quietly_and_hurts_the_dragon;

    /// #2647: a damage type a world's datapack declares is in the world's frozen
    /// registry, and `/damage` with it hurts a mob, as `DamageCommand.damage`
    /// does with the `Holder` it resolved there. Every hurt path reads the
    /// world's registry, not the startup catalog, which holds no pack.
    #[test]
    #[ignore = "daily"]
    fn a_datapack_damage_type_hurts_a_mob() {
        run_a_datapack_damage_type_hurts_a_mob();
    }
    pub(crate) fn run_a_datapack_damage_type_hurts_a_mob() {
        with_game(|game| {
            with_pack(game);
            pig(game, 1, Vec3::new(0.5, 64.0, 0.5));
            assert!(game.hurt_target(Target::Mob(1), 1.0, "dmgtest:zap", None, None, None));
            assert_eq!(game.mobs[0].health, 9.0);
            // A type neither registry holds is still refused.
            assert!(!game.hurt_target(Target::Mob(1), 1.0, "dmgtest:missing", None, None, None));
            assert_eq!(game.mobs[0].health, 9.0);
        });
    }
    /// A datapack type's tags in the world's registry decide as vanilla's do:
    /// `#bypasses_cooldown` lets an equal repeat hit land inside the hurt
    /// cooldown, which the untagged type's is refused; `#no_anger` records no
    /// attacker on the mob.
    #[test]
    #[ignore = "daily"]
    fn a_datapack_types_tags_reach_the_mob_hurt_path() {
        run_a_datapack_types_tags_reach_the_mob_hurt_path();
    }
    pub(crate) fn run_a_datapack_types_tags_reach_the_mob_hurt_path() {
        with_game(|game| {
            with_pack(game);
            pig(game, 1, Vec3::new(0.5, 64.0, 0.5));
            pig(game, 2, Vec3::new(4.5, 64.0, 0.5));
            assert!(game.hurt_target(Target::Mob(1), 1.0, "dmgtest:zap", None, None, None));
            assert!(!game.hurt_target(Target::Mob(1), 1.0, "dmgtest:zap", None, None, None));
            assert_eq!(game.mobs[0].health, 9.0);
            assert!(game.hurt_target(Target::Mob(1), 1.0, "dmgtest:rush", None, None, None));
            assert_eq!(game.mobs[0].health, 8.0);
            // A mob's hit names its attacker, unless the type is in `#no_anger`.
            pig(game, 3, Vec3::new(8.5, 64.0, 0.5));
            let attacker = Some(Target::Mob(1));
            assert!(game.hurt_target(
                Target::Mob(2),
                1.0,
                "dmgtest:calm",
                attacker,
                attacker,
                None
            ));
            assert!(game.hurt_target(Target::Mob(3), 1.0, "dmgtest:zap", attacker, attacker, None));
            let last_hurt_by = |game: &sunrise_gameplay::game::ServerGame<'_>, id: u32| {
                game.mobs
                    .iter()
                    .find(|m| m.id == id)
                    .unwrap()
                    .ai
                    .last_hurt_by
            };
            assert_eq!(last_hurt_by(game, 2), None, "#no_anger records no attacker");
            assert!(last_hurt_by(game, 3).is_some(), "the untagged type does");
        });
    }
    /// A datapack type in `#is_explosion` takes the End crystal away without
    /// the crystal's own blast, so a pig beside it is untouched; the untagged
    /// type sets the blast off. With no player behind it, it still hurts the
    /// dragon.
    #[test]
    #[ignore = "daily"]
    fn a_datapack_explosion_type_removes_a_crystal_quietly_and_hurts_the_dragon() {
        run_a_datapack_explosion_type_removes_a_crystal_quietly_and_hurts_the_dragon();
    }
}

#[cfg(test)]
pub(crate) mod vehicle_damage_command_tests {
    use std::sync::Arc;
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::chunk::Chunk;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
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
                .join(format!("h050-{:?}", std::thread::current().id())),
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

    #[test]
    #[ignore = "daily"]
    fn generic_commands_reach_actual_vehicle_damage_and_drop() {
        with_game(|game| {
            game.creative = true; // issuer creative is not an attacker
            let clock = game.world.game_time;
            for (kind, n) in [("oak_boat", 1), ("minecart", 2)] {
                let uuid = format!("000001f4-0000-0000-0000-{n:012x}");
                assert!(game.run_command(&format!(
                    "summon {kind} 4.5 200 5.5 {{UUID:[I;500,0,0,{n}],NoGravity:1b}}"
                )));
                let id = game
                    .boats
                    .iter()
                    .map(|b| b.id)
                    .chain(game.carts.iter().map(|c| c.id))
                    .find(|id| game.persist.uuid_of(*id) == Some([500, 0, 0, n]))
                    .unwrap();
                assert!(
                    game.run_command(&format!("damage {uuid} 1 minecraft:generic")),
                    "accepted small hit must reach vehicle"
                );
                let damage = if kind == "oak_boat" {
                    game.boats.iter().find(|b| b.id == id).unwrap().damage
                } else {
                    game.carts.iter().find(|c| c.id == id).unwrap().damage
                };
                assert_eq!(damage, 10.0);
                assert!(game.run_command(&format!("damage {uuid} 4 minecraft:generic")));
                assert!(!game.boats.iter().any(|b| b.id == id));
                assert!(!game.carts.iter().any(|c| c.id == id));
                assert_eq!(
                    game.dropped
                        .iter()
                        .filter(|d| d.stack.item.as_ref() == format!("minecraft:{kind}")
                            && d.stack.count == 1)
                        .count(),
                    1
                );
                let n = n + 2;
                let uuid = format!("000001f4-0000-0000-0000-{n:012x}");
                assert!(game.run_command(&format!(
                    "summon {kind} 8.5 200 5.5 {{UUID:[I;500,0,0,{n}],NoGravity:1b}}"
                )));
                let id = game
                    .boats
                    .iter()
                    .map(|b| b.id)
                    .chain(game.carts.iter().map(|c| c.id))
                    .find(|id| game.persist.uuid_of(*id) == Some([500, 0, 0, n]))
                    .unwrap();
                assert!(game.run_command(&format!("damage {uuid} 4 minecraft:generic")));
                let damage = if kind == "oak_boat" {
                    game.boats.iter().find(|b| b.id == id).unwrap().damage
                } else {
                    game.carts.iter().find(|c| c.id == id).unwrap().damage
                };
                assert_eq!(damage, 40.0, "exact threshold survives");
            }
            // Fresh rejected subjects avoid relying on vehicle data adoption.
            for (kind, n) in [("oak_boat", 5), ("minecart", 6)] {
                let uuid = format!("000001f4-0000-0000-0000-{n:012x}");
                assert!(game.run_command(&format!("summon {kind} 11.5 200 5.5 {{UUID:[I;500,0,0,{n}],NoGravity:1b,Invulnerable:1b}}")));
                let id = game
                    .boats
                    .iter()
                    .map(|b| b.id)
                    .chain(game.carts.iter().map(|c| c.id))
                    .find(|id| game.persist.uuid_of(*id) == Some([500, 0, 0, n]))
                    .unwrap();
                assert!(!game.run_command(&format!("damage {uuid} 4 minecraft:generic")));
                let damage = if kind == "oak_boat" {
                    game.boats.iter().find(|b| b.id == id).unwrap().damage
                } else {
                    game.carts.iter().find(|c| c.id == id).unwrap().damage
                };
                assert_eq!(damage, 0.0);
            }
            assert_eq!(game.world.game_time, clock);
        });
    }
    pub(crate) fn attribute_chat_count(
        rx: &std::sync::mpsc::Receiver<sunrise_sim::lan::Out>,
    ) -> usize {
        use sunrise_protocol::packets::play::Clientbound;
        use sunrise_sim::lan::Out;
        let mut bodies = Vec::new();
        for out in rx.try_iter() {
            match out {
                Out::Packet(body) => bodies.push(body),
                Out::Packets(more) => bodies.extend(more),
                Out::Disconnect(_) => panic!("actual registered guest disconnected"),
            }
        }
        bodies
            .iter()
            .filter(|body| {
                let mut r = sunrise_protocol::buf::Reader::new(body);
                let id = r.varint().unwrap();
                matches!(
                    Clientbound::decode(id, r.rest()).unwrap(),
                    Clientbound::SystemChat(_)
                )
            })
            .count()
    }
}

#[cfg(test)]
#[path = "particle_command_tests.rs"]
pub(crate) mod particle_command_tests;

#[cfg(test)]
#[path = "attribute_tests.rs"]
pub(crate) mod attribute_tests;

#[cfg(test)]
#[path = "effect_attribute_tests.rs"]
pub(crate) mod effect_attribute_tests;

#[cfg(test)]
pub(crate) mod attribute_modifier_command_tests {
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_core::nbt::Tag;
    use sunrise_core::position::ChunkPos;
    use sunrise_game_data::nbt_value::text;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
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
            std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join(format!(
                "attribute-modifier-{:?}",
                std::thread::current().id()
            )),
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

    /// `AttributeCommand.addModifier` uses `AttributeInstance.addPermanentModifier`;
    /// `AttributeInstance.pack` saves only permanent modifiers. A command
    /// modifier must therefore be in the player's saved list and survive a reload.
    #[test]
    #[ignore = "daily"]
    fn a_command_modifier_is_saved_with_the_player() {
        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            game.reset_entity_residency();
            game.native_lifecycle
                .begin_open(Some(mod_api::Authority::IntegratedServer));
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 1, 1);
            assert!(game.run_command(
                "attribute @s minecraft:step_height modifier add test:a 0.4 add_value"
            ));
            assert_eq!(
                game.player
                    .as_mut()
                    .unwrap()
                    .attribute("minecraft:step_height")
                    .unwrap(),
                1.0
            );
            let saved = game.player.as_ref().unwrap().saved_attributes();
            let holds = match &saved {
                Tag::List(instances) => instances.iter().any(|entry| {
                    text(entry.get("id")).as_deref() == Some("minecraft:step_height")
                        && matches!(
                            entry.get("modifiers"),
                            Some(Tag::List(modifiers))
                                if modifiers
                                    .iter()
                                    .any(|m| text(m.get("id")).as_deref() == Some("test:a"))
                        )
                }),
                _ => false,
            };
            assert!(holds, "saved attributes: {saved:?}");
            let mut back = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            back.apply_saved_attributes(Some(&saved)).unwrap();
            assert_eq!(
                back.attribute("minecraft:step_height").unwrap(),
                1.0,
                "saved attributes: {saved:?}"
            );
        });
    }
}
