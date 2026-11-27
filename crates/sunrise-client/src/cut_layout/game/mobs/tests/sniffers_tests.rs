//! Sniffer digging effects: the `entity_action` a dig posts every ten ticks
//! (`Sniffer.emitDiggingParticles`), the local sounds silence does not mute
//! (`playSearchingSound`, the block's hit sound) and the egg's own-random push
//! (`Sniffer.spawnChildFromBreeding`'s `new ItemEntity(..)`).
use crate::game::server_side::kill_die_vibration_tests::game as sculk_game;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_gameplay_common::game::PlayerMotion;
use sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS;
use sunrise_sim::mobs::Who;
use sunrise_sim::mobs::{Mob, sniffer};

fn game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    sculk_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 100.0, 8.5)).unwrap());
        game.mobs.clear();
        test(game);
    });
}

/// A digging sniffer at `(8.5, 100, 8.5)`, facing +z, its head above the stone.
fn digging_sniffer<'g>(
    game: &mut impl sunrise_gameplay_common::game::SideGame<'g>,
) -> (u32, [i32; 3]) {
    let kind = game
        .res
        .mob_types()
        .get("minecraft:sniffer")
        .unwrap()
        .clone();
    let mut m = Mob::new(42, kind, Vec3::new(8.5, 100.0, 8.5), 0.0);
    sniffer::set_state(&mut m, sniffer::DIGGING);
    let head = sniffer::head_block(&m);
    game.mobs.push(m);
    (42, head)
}

fn posted() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    TEST_POSTED_EVENTS.with(|events| events.borrow().clone().unwrap_or_default())
}

#[test]
#[ignore = "daily"]
fn digging_posts_entity_action_every_ten_ticks_outside_the_dust_window() {
    game(|game| {
        let (id, head) = digging_sniffer(game);
        let at = [
            head[0] as f64 + 0.5,
            head[1] as f64 + 0.5,
            head[2] as f64 + 0.5,
        ];
        // 0 is the first dig tick, 20 is before the window, 40 inside it and 130
        // after it; 25 is not a tenth tick.
        for age in [0_u32, 20, 40, 130, 25] {
            let m = &mut game.mobs[0];
            m.age = age;
            m.anim.states[3].start(0.0);
            TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = Some(Vec::new()));
            game.tick_sniffer_server();
            let events = posted();
            let action = events
                .iter()
                .filter(|(event, ..)| *event == "minecraft:entity_action")
                .collect::<Vec<_>>();
            if age.is_multiple_of(10) {
                assert_eq!(
                    action,
                    vec![&("minecraft:entity_action", at, Some(Who::Mob(id)))],
                    "age {age}"
                );
            } else {
                assert!(action.is_empty(), "age {age} posted {events:?}");
            }
        }
        TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = None);
    });
}

#[test]
#[ignore = "daily"]
fn a_silent_sniffer_still_plays_the_local_searching_and_hit_sounds() {
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in 6..=10 {
            for z in 6..=12 {
                game.world.set(x, 99, z, stone).unwrap();
            }
        }
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 100.0, 8.5)).unwrap());
        game.mobs.clear();
        let _ = digging_sniffer(game);
        game.mobs[0].silent = true;
        game.mobs[0].age = 20;
        sniffer::set_state(&mut game.mobs[0], sniffer::SEARCHING);
        game.client.audio.0.heard.clear();
        game.tick_sniffer_client();
        assert_eq!(
            game.client
                .audio
                .0
                .heard
                .get("minecraft:entity.sniffer.searching"),
            Some(&1)
        );
        sniffer::set_state(&mut game.mobs[0], sniffer::DIGGING);
        game.mobs[0].age = 40;
        game.mobs[0].anim.states[3].start(0.0);
        game.client.audio.0.heard.clear();
        game.tick_sniffer_client();
        assert_eq!(
            game.client.audio.0.heard.get("minecraft:block.stone.hit"),
            Some(&1)
        );
    });
}

#[test]
#[ignore = "daily"]
fn the_egg_push_comes_from_the_items_own_random() {
    game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:sniffer")
            .unwrap()
            .clone();
        let parent = Mob::new(43, kind, Vec3::new(8.5, 100.0, 8.5), 0.0);
        let mut level = game.world.random.clone();
        game.lay_sniffer_egg(&parent);
        let egg = game.dropped.last().unwrap();
        assert_eq!(&*egg.stack.item, "minecraft:sniffer_egg");
        assert_eq!(egg.pickup_delay, 10);
        // `new ItemEntity(level, x, y, z, stack)` draws its motion from the item's
        // random: the level's stream does not move.
        assert_eq!(game.world.random.next_double(), level.next_double());
    });
}
