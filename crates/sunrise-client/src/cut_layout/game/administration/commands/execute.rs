#![cfg(test)]
#[cfg(test)]
#[path = "execute_facing_tests.rs"]
pub(crate) mod execute_facing_tests;

#[cfg(test)]
pub(crate) mod terminal_fork_tests {
    use std::sync::Arc;
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::chunk::Chunk;
    use {
        crate::game::{Settings, load_resources},
        sunrise_gameplay_common::game::Options,
        sunrise_gameplay_common::game::Progress,
    };

    #[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::commands_server::execute::terminal_fork_tests::capture;

    #[test]
    #[ignore = "daily"]
    fn terminal_conditions_keep_the_fork_state_and_callbacks() {
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
        options.game_dir = Some(std::env::current_dir().unwrap().join("game"));
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
        let mut chunk = Chunk::empty(0, 0);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 199, z, stone.0);
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("scoreboard objectives add h147 dummy"));
        assert!(game.run_command("scoreboard players set zero h147 0"));
        for n in [1, 2] {
            assert!(game.run_command(&format!("summon pig {} 200 4.5 {{UUID:[I;147,0,0,{n}],Health:10f,NoAI:1b,NoGravity:1b,Tags:[\"h147\"]}}", n as f64 + 0.5)));
            assert!(
                game.mobs
                    .iter()
                    .any(|m| game.persist.uuids.get(&m.id) == Some(&[147, 0, 0, n])
                        && m.kind.id == "minecraft:pig"
                        && m.health == 10.0)
            );
        }
        let time = game.world.game_time;
        let a = "00000093-0000-0000-0000-000000000001";
        let positive = capture(
            &mut game,
            &format!("execute as {a} if score zero h147 matches 0"),
        );
        assert_eq!(positive.0.len(), 1);
        assert_eq!(positive.1, vec![(true, 1)]);
        let direct = capture(&mut game, "execute if score zero h147 matches 1");
        assert_eq!(direct.0.len(), 1);
        assert_eq!(direct.1, vec![(false, 0)]);
        for tail in [
            format!("as {a} if score zero h147 matches 1"),
            format!("at {a} if block ~ ~-1 ~ air"),
            format!("as {a} if score zero h147_missing matches 1"),
        ] {
            assert!(game.run_command("scoreboard players set result h147 -99"));
            assert!(game.run_command("scoreboard players set success h147 -99"));
            let (messages, callbacks) = capture(
                &mut game,
                &format!(
                    "execute store result score result h147 store success score success h147 {tail}"
                ),
            );
            assert_eq!(callbacks, vec![(false, 0)]);
            assert_eq!(
                game.score.board.score("result", "h147").map(|s| s.value),
                Some(0)
            );
            assert_eq!(
                game.score.board.score("success", "h147").map(|s| s.value),
                Some(0)
            );
            assert!(messages.is_empty(), "forked terminal error: {messages:?}");
        }
        let many = capture(
            &mut game,
            "execute as @e[type=pig,tag=h147] if score zero h147 matches 1",
        );
        assert!(many.0.is_empty());
        assert_eq!(many.1, vec![(false, 0), (false, 0)]);
        let missing = capture(&mut game, "execute if score zero h147_missing matches 1");
        assert_eq!(missing.0.len(), 1);
        assert_eq!(missing.1, vec![(false, 0)]);
        let empty = capture(
            &mut game,
            "execute as @e[tag=h147_none] if score zero h147 matches 1",
        );
        assert!(empty.0.is_empty() && empty.1.is_empty());
        assert_eq!(game.world.game_time, time);
    }
}

#[cfg(test)]
pub(crate) mod score_condition_tests {
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;

    pub(crate) fn game(
        resources: &mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> sunrise_gameplay::game::ServerGame<'_> {
        let mut game = server_game(resources);
        game.build_registries();
        game.set_allow_commands(true);
        game
    }

    #[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::commands_server::execute::score_condition_tests::say;

    /// `checkScore` resolves the source holder and objective before it reads
    /// the target's score, so a terminal condition names the bad objective
    /// even when the target has no score at all.
    #[test]
    #[ignore = "daily"]
    fn an_unknown_source_objective_fails_a_terminal_condition() {
        let mut resources = None;
        let mut game = game(&mut resources);
        assert!(game.run_command("scoreboard objectives add a dummy"));
        for sign in ["if", "unless"] {
            let (ok, said) = say(
                &mut game,
                &format!("execute {sign} score zero a = zero nope"),
            );
            assert!(!ok, "{sign}: {said:?}");
            assert!(
                said.iter().any(|m| m.contains("nope")),
                "{sign}: the source objective is named: {said:?}"
            );
        }
    }

    /// A non-terminal `if`/`unless` is a fork: its error is silent and its
    /// source is dropped (`CommandSourceStack.handleError` with `forked`), so
    /// `unless` does not run the branch either.
    #[test]
    #[ignore = "daily"]
    fn an_unknown_source_objective_drops_a_forked_condition_silently() {
        let mut resources = None;
        let mut game = game(&mut resources);
        assert!(game.run_command("scoreboard objectives add a dummy"));
        for sign in ["if", "unless"] {
            let (ok, said) = say(
                &mut game,
                &format!("execute {sign} score zero a = zero nope run say hi"),
            );
            assert!(ok, "{sign}: {said:?}");
            assert!(said.is_empty(), "{sign}: {said:?}");
        }
    }
}
