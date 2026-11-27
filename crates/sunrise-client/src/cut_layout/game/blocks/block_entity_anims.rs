#![cfg(test)]
use sunrise_level::containers::BlockEntity;
#[cfg(test)]
pub(crate) mod tests {
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    #[test]
    #[ignore = "daily"]
    fn replacing_a_gateway_discards_its_old_cooldown() {
        run_replacing_a_gateway_discards_its_old_cooldown();
    }

    pub(crate) fn run_replacing_a_gateway_discards_its_old_cooldown() {
        use {crate::game::*, sunrise_gameplay_common::game::*};
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("set SUNRISE_TEST_RESOURCES"),
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
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        let res = loaded.resources;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &res,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.options.dedicated = Some(Vec::new());
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        game.server.streaming.forced.insert((0, 0));
        game.build_registries();
        fn command(game: &mut sunrise_gameplay::game::ServerGame<'_>, line: &str) -> bool {
            game.as_console(|game| game.run_command(line))
        }
        let portal = game
            .pack
            .assets
            .registry
            .default_state("minecraft:end_gateway")
            .unwrap();
        for x in [2, 4] {
            assert!(command(
                &mut game,
                &format!(
                    "setblock {x} 200 2 end_gateway{{Age:200L,exit_portal:[I;8,210,8],ExactTeleport:1b}}"
                )
            ));
            assert_eq!(game.world.block(x, 200, 2), portal);
            assert!(
                game.gateway_enter([x, 200, 2]),
                "fresh gateway admits a traveller"
            );
            assert!(
                !game.gateway_available([x, 200, 2]),
                "entry starts its cooldown"
            );
        }
        assert!(command(&mut game, "setblock 2 200 2 air"));
        assert!(game.res.facts().is_air(game.world.block(2, 200, 2)));
        assert!(command(
            &mut game,
            "setblock 2 200 2 end_gateway{Age:200L,exit_portal:[I;8,210,8],ExactTeleport:1b}"
        ));
        assert_eq!(game.world.block(2, 200, 2), portal);
        assert!(
            game.gateway_enter([2, 200, 2]),
            "replacement is a fresh block entity, not the cooling predecessor"
        );
        assert!(
            !game.gateway_available([4, 200, 2]),
            "the untouched gateway stays on cooldown"
        );
        assert!(
            !game.gateway_enter([2, 200, 2]),
            "the replacement starts its own cooldown"
        );
    }
}

#[cfg(test)]
pub(crate) mod conduit_target_tests {

    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::server_side::block_entity_anims::conduit_target_tests::CONDUIT;

    use sunrise_gameplay::game::server_side::block_entity_anims::conduit_target_tests::setup;

    use sunrise_gameplay::game::server_side::block_entity_anims::conduit_target_tests::summon;

    use crate::game::{Options, Progress, Settings, execution_role, load_resources};

    use std::sync::Arc;

    use sunrise_level::chunk::Chunk;

    fn game_with(
        resources: &mut Option<Arc<sunrise_gameplay_common::game::Resources>>,
        role: execution_role::ExecutionRole,
    ) -> sunrise_gameplay::game::ServerGame<'_> {
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
            role,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = role;
        let res = resources.insert(loaded.resources);
        let mut game = sunrise_gameplay::game::ServerGame::new(
            res,
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
        game
    }
    /// A level of a server.
    pub(crate) fn game(
        resources: &mut Option<Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> sunrise_gameplay::game::ServerGame<'_> {
        let mut game = game_with(resources, execution_role::ExecutionRole::IntegratedServer);
        setup(&mut game);
        game
    }
    /// `selectNewTarget`'s `instanceof Enemy`: the monster-category animals
    /// (a zombie horse, camel husk, zombie nautilus) are not enemies, so a
    /// conduit beside them picks nothing; a zombie is.
    #[test]
    #[ignore = "daily"]
    fn a_conduit_hunts_only_enemies() {
        let mut resources = None;
        let mut game = game(&mut resources);
        for kind in ["zombie_horse", "camel_husk", "zombie_nautilus"] {
            summon(&mut game, kind);
        }
        assert_eq!(
            game.conduit_new_target(CONDUIT),
            None,
            "monster-category animals are not Enemy"
        );
        let zombie = summon(&mut game, "zombie");
        assert_eq!(game.conduit_new_target(CONDUIT), Some(zombie));
    }
}

#[cfg(test)]
pub(crate) mod gateway_age_tests {
    use super::*;
    use sunrise_gameplay::game::game_tests::tests::server_game;

    /// `TheEndGatewayBlockEntity.portalTick`: an age that moved is a change
    /// to save.
    #[test]
    #[ignore = "daily"]
    fn a_gateway_that_ages_marks_the_chunk() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:end_gateway")
            .unwrap();
        let pos = (2, 4, 3);
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        game.world.block_entities.insert(
            pos,
            BlockEntity::EndGateway(sunrise_level::containers::EndGateway::default()),
        );
        game.world.modified.clear();

        game.tick_end_gateway(pos);

        let Some(BlockEntity::EndGateway(g)) = game.world.block_entities.get(&pos) else {
            panic!("no gateway");
        };
        assert_eq!(g.age, 1, "the tick aged the gateway");
        assert!(
            game.world.modified.contains(&(0, 0)),
            "the aged gateway left the chunk unmarked"
        );
    }
}
