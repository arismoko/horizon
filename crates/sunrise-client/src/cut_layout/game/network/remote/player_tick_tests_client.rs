use crate::game::remote::player_tick_tests::Log;
use std::sync::{Arc, Mutex};

#[test]
#[ignore = "daily"]
fn client_player_ticks_on_the_death_screen_too() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "log",
        Box::new(Log(Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let (client, server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "tick-client").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.profile = 456;
            remote.entity_id = 42;
            game.player = sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::ZERO,
            )
            .ok();
            game.client
                .remote
                .as_mut()
                .unwrap()
                .level_players
                .insert(66, 789);
            let kind = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .player_type
                .as_ref()
                .unwrap()
                .clone();
            game.mobs.push(sunrise_sim::mobs::Mob::new(
                66,
                kind,
                sunrise_core::geometry::Vec3::ZERO,
                0.0,
            ));
            game.cooldowns.add("minecraft:ender_pearl".into(), 3);
            game.sleep_timer = 4;
            // LocalPlayer.tick waits for the client load: only
            // the other player's pair runs, and the level pass still ticks.
            game.tick_remote().unwrap();
            // Player.tick owns both counters, so they wait with it.
            assert_eq!(game.cooldowns.remaining().next().unwrap().1, 3);
            assert_eq!(game.sleep_timer, 4);
            let phases = || {
                std::mem::take(&mut *heard.lock().unwrap())
                    .iter()
                    .map(|s| (s.phase, s.side, s.uuid))
                    .collect::<Vec<_>>()
            };
            assert_eq!(phases(), [("pre", "client", 789), ("post", "client", 789)]);
            game.client.remote.as_mut().unwrap().loaded_sent = true;
            game.client.streaming.insert_remote(
                &mut game.common.world,
                sunrise_level::chunk::Chunk::empty(0, 0),
            );
            // The other player's tick ends before the local player's starts.
            game.tick_remote().unwrap();
            assert_eq!(game.cooldowns.remaining().next().unwrap().1, 2);
            assert_eq!(game.sleep_timer, 5);
            game.client.screen = sunrise_gameplay_common::game::Screen::Dead;
            game.player
                .as_mut()
                .unwrap()
                .set_health(0.0, false)
                .unwrap();
            game.tick_remote().unwrap();
            assert_eq!(
                phases(),
                [
                    ("pre", "client", 789),
                    ("post", "client", 789),
                    ("pre", "client", 456),
                    ("post", "client", 456),
                    ("pre", "client", 789),
                    ("post", "client", 789),
                    ("pre", "client", 456),
                    ("post", "client", 456)
                ]
            );
            server.shutdown();
        })
    });
}
