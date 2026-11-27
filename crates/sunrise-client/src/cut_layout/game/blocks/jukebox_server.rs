#![cfg(test)]
use sunrise_gameplay_common::game::jukebox_common::jukebox_signals;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_level::containers::Jukebox;
#[cfg(test)]
pub(crate) mod jukebox_play_tests {
    use super::*;

    fn playing(ticks: i32) -> Option<Vec<String>> {
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES")?);
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
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        let jukebox = game
            .pack
            .assets
            .registry
            .default_state("minecraft:jukebox")
            .unwrap();
        game.world.set(0, 64, 0, jukebox).unwrap();
        let disc = ItemStack::new("minecraft:music_disc_cat", 1);
        sunrise_gameplay::game::dispense_fixture::start_recording();
        assert_eq!(
            game.use_jukebox(
                [0, 64, 0],
                jukebox,
                Some(&disc),
                sunrise_world::level::UseBranch::ItemOn,
            ),
            Some(mod_api::InteractionResult::Success),
            "the disc goes in"
        );
        let insertion = sunrise_gameplay::game::dispense_fixture::stop_recording();
        assert!(
            insertion
                .iter()
                .any(|line| line == "G 0,64,0 minecraft:block_change"),
            "using the jukebox must publish the insertion event: {insertion:?}"
        );
        sunrise_gameplay::game::dispense_fixture::start_recording();
        for _ in 0..ticks {
            game.tick_jukebox((0, 64, 0));
        }
        Some(sunrise_gameplay::game::dispense_fixture::stop_recording())
    }

    #[test]
    #[ignore = "daily"]
    fn a_playing_jukebox_posts_jukebox_play_every_twenty_ticks() {
        let Some(lines) = playing(41) else { return };
        let plays: Vec<&String> = lines
            .iter()
            .filter(|l| l.ends_with("minecraft:jukebox_play"))
            .collect();
        assert_eq!(
            plays,
            vec![
                &"G 0,64,0 minecraft:jukebox_play".to_owned(),
                &"G 0,64,0 minecraft:jukebox_play".to_owned(),
                &"G 0,64,0 minecraft:jukebox_play".to_owned(),
            ],
            "{lines:?}"
        );
        assert!(
            !lines
                .iter()
                .any(|l| l.ends_with("minecraft:jukebox_stop_play")),
            "{lines:?}"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_stopped_jukebox_posts_no_play_event() {
        let Some(lines) = playing(0) else { return };
        assert!(
            lines.iter().all(|l| l.ends_with("minecraft:block_change")),
            "{lines:?}"
        );
    }
}

#[cfg(test)]
pub(crate) mod saved_song_clock_tests {
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;

    #[test]
    #[ignore = "daily"]
    fn actual_start_tick_stop_and_loaded_record_use_the_modeled_clock() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("forceload add 0 0 15 15"));
            assert!(game.world.loaded(4, 4));
            assert!(game.run_command("setblock 4 199 4 stone"));
            assert!(game.run_command("setblock 4 200 4 jukebox"));
            let clock = game.world.game_time;
            let mut failures = Vec::new();
            let mut check = |label: &str, tag: &sunrise_core::nbt::Tag, want: Option<i64>| {
                let got = tag.get("ticks_since_song_started").cloned();
                let want = want.map(sunrise_core::nbt::Tag::Long);
                if got != want {
                    failures.push(format!("{label}: {got:?}, expected {want:?}"));
                }
            };
            let pos = [4, 200, 4];
            check(
                "actual empty Game writer",
                &game.block_entity_nbt(pos).unwrap(),
                None,
            );
            assert!(game.run_command("item replace block 4 200 4 container.0 with music_disc_cat[minecraft:custom_data={h032:1}]"));
            let j = game.jukebox(pos).unwrap();
            assert!(j.playing);
            assert_eq!(j.ticks, 0);
            assert_eq!(j.record().unwrap().count, 1);
            check(
                "actual start keeps zero Long",
                &game.block_entity_nbt(pos).unwrap(),
                Some(0),
            );
            game.tick_jukebox((4, 200, 4));
            assert_eq!(game.jukebox(pos).unwrap().ticks, 1);
            check(
                "actual tick keeps sole counter",
                &game.block_entity_nbt(pos).unwrap(),
                Some(1),
            );
            let before = game
                .block_entity_nbt(pos)
                .unwrap()
                .get("RecordItem")
                .cloned();
            assert!(before.is_some());
            game.jukebox_set_item(pos, None);
            let j = game.jukebox(pos).unwrap();
            assert!(!j.playing);
            assert_eq!(j.ticks, 0);
            assert!(j.record().is_none());
            let saved = game.block_entity_nbt(pos).unwrap();
            assert!(saved.get("RecordItem").is_none());
            check("actual stop removes clock", &saved, None);
            assert!(game.run_command("setblock 8 199 4 stone"));
            assert!(game.run_command("setblock 8 200 4 jukebox[has_record=true]{RecordItem:{id:\"minecraft:music_disc_cat\",count:1,components:{\"minecraft:custom_data\":{h032:1}}}}"));
            let j = game.jukebox([8, 200, 4]).unwrap();
            assert!(!j.playing, "a record without a clock does not play on load");
            assert_eq!(j.ticks, 0);
            let saved = game.block_entity_nbt([8, 200, 4]).unwrap();
            assert_eq!(
                saved.get("RecordItem"),
                before.as_ref(),
                "actual record/components preserved"
            );
            check("actual loaded record without clock", &saved, None);
            assert_eq!(
                game.world.game_time, clock,
                "no scene ticks in this caller test; registered runtime progress is separately paired"
            );
            assert!(
                failures.is_empty(),
                "actual Game saved song-clock mismatches: {failures:?}"
            );
        });
    }
}

#[cfg(test)]
pub(crate) mod jukebox_component_tests {
    use super::*;
    use sunrise_core::nbt::Tag;
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay::game::game_tests::tests::server_game;

    fn comparator(game: &mut sunrise_gameplay::game::ServerGame<'_>, stack: ItemStack) -> i32 {
        let key = (8, 64, 8);
        game.world.block_entities.insert(
            key,
            BlockEntity::Jukebox(Jukebox {
                items: [Some(stack)],
                ticks: 0,
                song_clock_present: true,
                playing: true,
                loaded_clock: false,
            }),
        );
        jukebox_signals(
            &game.world,
            game.items(),
            BlockPos::new(key.0, key.1, key.2),
        )
        .unwrap()
        .1
    }

    #[test]
    #[ignore = "daily"]
    fn a_jukebox_reads_the_discs_own_jukebox_component() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let cat = game.items().songs.get("minecraft:cat").unwrap().comparator;
        assert_eq!(
            comparator(&mut game, ItemStack::new("minecraft:music_disc_cat", 1)),
            cat,
            "the item's default song"
        );

        let mut removed = ItemStack::new("minecraft:music_disc_cat", 1);
        removed.data_mut().set_other(
            "!minecraft:jukebox_playable",
            Some(Tag::Compound(Default::default())),
        );
        assert_eq!(
            comparator(&mut game, removed),
            0,
            "a removed playable component names no song"
        );

        let mut other = ItemStack::new("minecraft:stone", 1);
        other.data_mut().set_other(
            "minecraft:jukebox_playable",
            Some(Tag::String("minecraft:cat".into())),
        );
        assert_eq!(
            comparator(&mut game, other),
            cat,
            "the stack's own component names the song"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_jukebox_takes_what_the_stacks_component_makes_playable() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:jukebox")
            .unwrap();
        game.world.set(8, 64, 8, state).unwrap();
        let takes = |game: &sunrise_gameplay::game::ServerGame<'_>, stack: ItemStack| {
            game.jukebox_takes([8, 64, 8], Some(&stack))
        };
        assert!(takes(&game, ItemStack::new("minecraft:music_disc_cat", 1)));
        assert!(!takes(&game, ItemStack::new("minecraft:stone", 1)));
        let mut removed = ItemStack::new("minecraft:music_disc_cat", 1);
        removed.data_mut().set_other(
            "!minecraft:jukebox_playable",
            Some(Tag::Compound(Default::default())),
        );
        assert!(
            !takes(&game, removed),
            "a removed component is not playable"
        );
        let mut stone = ItemStack::new("minecraft:stone", 1);
        stone.data_mut().set_other(
            "minecraft:jukebox_playable",
            Some(Tag::String("minecraft:cat".into())),
        );
        assert!(
            takes(&game, stone),
            "the stack's own component makes it playable"
        );
    }
}

#[cfg(test)]
#[path = "jukebox/tests.rs"]
pub(crate) mod jukebox_tests;

#[cfg(test)]
pub(crate) mod block_change_tests {

    use std::sync::Arc;
    use sunrise_level::chunk::Chunk;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn a_disc_change_posts_block_change() {
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
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let jukebox = game
            .pack
            .assets
            .registry
            .default_state("minecraft:jukebox")
            .unwrap();
        game.world.set(2, 4, 3, jukebox).unwrap();
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|events| *events.borrow_mut() = Some(Vec::new()));
        game.set_has_record([2, 4, 3], true);
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|events| {
            assert!(
                events.borrow().as_ref().unwrap().contains(&(
                    "minecraft:block_change",
                    [2.5, 4.5, 3.5],
                    None
                )),
                "{:?}",
                events.borrow()
            );
        });
        game.world.set(2, 4, 5, jukebox).unwrap();
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|events| *events.borrow_mut() = Some(Vec::new()));
        game.set_has_record_notify([2, 4, 5], true, false);
        let with_record = game.world.block(2, 4, 5);
        assert_ne!(with_record, jukebox, "the state still turns has_record on");
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|events| {
            assert_eq!(events.borrow().as_ref().unwrap(), &[], "nothing posted");
        });
    }
}
use sunrise_gameplay_common::game::tables::GameItems as _;
