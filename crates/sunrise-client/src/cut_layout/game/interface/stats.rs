#![cfg(test)]
#[cfg(test)]
#[path = "tests/phantom_spawn_tests.rs"]
mod phantom_spawn_tests;
#[cfg(test)]
#[path = "tests/player_stat_award_tests.rs"]
mod player_stat_award_tests;
#[cfg(test)]
#[path = "tests/stat_award_tests.rs"]
mod stat_award_tests;

#[cfg(test)]
pub(crate) mod tests {

    use std::path::Path;

    use sunrise_gameplay_common::stats::player_json;

    #[test]
    #[ignore = "daily"]
    fn fake_players_do_not_advance_phantom_rng() {
        use sunrise_core::random::RandomSource;
        super::super::native_entity_refs::tests::with_game(|game| {
            sunrise_gameplay::game::server_side::bossbars::audience_tests::add_real_and_fake(game);
            game.player = None;
            game.world.day_time = 18000;
            let host = game.server.lan.as_mut().unwrap();
            let guests = &mut host.hub.guests;
            let index = guests.iter().position(|g| g.uuid == 2).unwrap();
            guests.update(index, |g| g.position = [1000.0, 100.0, 0.0]);
            for guest in &host.hub.guests {
                let mut play =
                    sunrise_gameplay::game::server_side::lan_actions_server::GuestPlay::default();
                play.stats.custom("time_since_rest", 200_000);
                host.play.insert(guest.conn, play);
            }
            game.world.random = sunrise_core::random::LevelRandom::for_level(
                sunrise_core::random::LegacyRandom::new(12345),
            );
            game.spawn_phantoms();
            assert!(
                game.mobs
                    .iter()
                    .all(|mob| (mob.position.x - 1000.0).abs() > 20.0)
            );
            let with_fake = game.world.random.next_int();
            game.world.random = sunrise_core::random::LevelRandom::for_level(
                sunrise_core::random::LegacyRandom::new(12345),
            );
            game.stats.set_phantom_tick(0);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .retain(|g| g.uuid != 2);
            game.spawn_phantoms();
            assert_eq!(with_fake, game.world.random.next_int());
        });
    }
    /// A write that cannot finish leaves the last saved JSON of a player: the owner's stats and
    /// advancements and a network player's, which a crash save can reach mid-write. The new
    /// file is written beside it and renamed over, so blocking the beside file stops the write
    /// before the saved one is touched; unblocked, the same saves write.
    #[test]
    #[ignore = "daily"]
    fn a_player_json_write_that_cannot_finish_keeps_the_last_saved_file() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let world = std::env::temp_dir().join(format!(
                "sunrise-player-json-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&world);
            game.server.world_root = world.clone();
            game.level.player_uuid = Some([0, 0, 0, 5]);
            game.stats.loaded = true;
            game.adv.loaded = true;
            assert!(!game.advancement_tree().list.is_empty());
            let guest = sunrise_sim::lan::playerdata::uuid_ints(12);
            let files = [
                game.stats_path().unwrap(),
                player_json(&world, "advancements", [0, 0, 0, 5]),
                player_json(&world, "stats", guest),
                player_json(&world, "advancements", guest),
            ];
            let beside = |file: &Path| {
                let mut name = file.file_name().unwrap().to_owned();
                name.push("_new");
                file.with_file_name(name)
            };
            for file in &files {
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, "last saved").unwrap();
                std::fs::create_dir(beside(file)).unwrap();
            }
            let host = game.server.lan.as_mut().unwrap();
            host.play.insert(
                12,
                sunrise_gameplay::game::server_side::lan_actions_server::GuestPlay::default(),
            );
            let save = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.save_stats();
                game.save_advancements();
                let host = game.server.lan.as_ref().unwrap();
                let guest = host.hub.guests.iter().find(|g| g.conn == 12).unwrap();
                game.save_guest_records(host, guest);
            };
            save(game);
            for file in &files {
                assert_eq!(
                    std::fs::read_to_string(file).unwrap(),
                    "last saved",
                    "{}",
                    file.display()
                );
            }
            for file in &files {
                std::fs::remove_dir(beside(file)).unwrap();
            }
            save(game);
            for file in &files {
                let text = std::fs::read_to_string(file).unwrap();
                assert!(
                    serde_json::from_str::<serde_json::Value>(&text).is_ok(),
                    "{} is written",
                    file.display()
                );
            }
            std::fs::remove_dir_all(&world).unwrap();
        });
    }
}
