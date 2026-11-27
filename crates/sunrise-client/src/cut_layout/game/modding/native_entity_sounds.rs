#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_resident_game;
    use mod_api::{EntityRef, SoundSource, WorldEpoch};
    use sunrise_command::args::GameMode;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::server_side::dispense_fixture;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
    use sunrise_sim::mobs::Mob;
    use sunrise_sim::motion::PlayerMotion;

    fn spawn(game: &mut sunrise_gameplay::game::ServerGame<'_>, id: u32, kind: &str) -> EntityRef {
        let kind = game.res.mob_types().get(kind).unwrap().clone();
        game.mobs
            .push(Mob::new(id, kind, Vec3::new(1.5, 64.0, 2.5), 0.0));
        game.entity(id).unwrap()
    }

    fn acting_player(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 7, 7);
    }

    #[test]
    #[ignore = "daily"]
    fn sound_sources_follow_the_entity_class() {
        with_resident_game(|game| {
            let cow = spawn(game, 10, "minecraft:cow");
            let zombie = spawn(game, 11, "minecraft:zombie");
            let ghast = spawn(game, 12, "minecraft:ghast");
            let slime = spawn(game, 13, "minecraft:slime");
            let sulfur = spawn(game, 14, "minecraft:sulfur_cube");
            let bunny = spawn(game, 15, "minecraft:rabbit");
            let source =
                |game: &sunrise_gameplay::game::ServerGame<'_>, e| game.entity_sound_source(e);
            assert_eq!(source(game, cow), Some(SoundSource::Neutral));
            assert_eq!(source(game, zombie), Some(SoundSource::Hostile));
            assert_eq!(source(game, ghast), Some(SoundSource::Hostile));
            assert_eq!(source(game, slime), Some(SoundSource::Hostile));
            assert_eq!(source(game, sulfur), Some(SoundSource::Neutral));
            assert_eq!(source(game, bunny), Some(SoundSource::Neutral));
            game.mobs
                .iter_mut()
                .find(|m| m.id == 15)
                .unwrap()
                .extra
                .variant = sunrise_sim::mobs::rabbit::EVIL;
            assert_eq!(source(game, bunny), Some(SoundSource::Hostile));
            acting_player(game);
            let player = game.entity(7).unwrap();
            assert_eq!(source(game, player), Some(SoundSource::Players));
            let stale = EntityRef::new(WorldEpoch::new(cow.epoch().get() + 1), 10);
            assert_eq!(source(game, stale), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn play_sound_queues_at_the_entity_in_its_source() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            let cow = spawn(game, 10, "minecraft:cow");
            assert!(game.entity_play_sound(cow, "minecraft:entity.cow.ambient", 0.5, 1.5));
            let sound = &game.server.pending_native_sounds[0];
            assert_eq!(sound.sound, "minecraft:entity.cow.ambient");
            assert_eq!(sound.at, [1.5, 64.0, 2.5]);
            assert_eq!(sound.source as usize, SoundSource::Neutral.index());
            assert_eq!((sound.volume, sound.pitch, sound.except), (0.5, 1.5, None));
            // A silent mob plays nothing; a malformed event key is refused.
            game.mobs[0].silent = true;
            assert!(!game.entity_play_sound(cow, "minecraft:entity.cow.ambient", 1.0, 1.0));
            game.mobs[0].silent = false;
            assert!(!game.entity_play_sound(cow, "invalid key", 1.0, 1.0));
            assert_eq!(game.server.pending_native_sounds.len(), 1);
            // A player doesn't hear their own sound, silent or not.
            game.as_guest(12, |game| {
                let player = game.entity(12).unwrap();
                assert!(game.entity_play_sound(player, "minecraft:entity.player.burp", 1.0, 1.0));
                let sound = &game.server.pending_native_sounds[1];
                assert_eq!(sound.source as usize, SoundSource::Players.index());
                assert_eq!(sound.except, Some(12));
            })
            .expect("the sound's real guest turn runs");
            let stale = EntityRef::new(WorldEpoch::new(cow.epoch().get() + 1), 10);
            assert!(!game.entity_play_sound(stale, "minecraft:entity.cow.ambient", 1.0, 1.0));
            assert_eq!(game.server.pending_native_sounds.len(), 2);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn game_and_entity_events_come_from_the_entity() {
        with_resident_game(|game| {
            let cow = spawn(game, 10, "minecraft:cow");
            dispense_fixture::start_recording();
            assert!(game.entity_game_event(cow, "minecraft:step"));
            assert!(!game.entity_game_event(cow, "minecraft:no_such_event"));
            assert_eq!(
                dispense_fixture::stop_recording(),
                ["G 1,64,2 minecraft:step"]
            );
            game.start_audio_capture();
            assert!(game.entity_broadcast_event(cow, 3));
            assert_eq!(game.server.lan_entity_events, [(10, 3)]);
            let stale = EntityRef::new(WorldEpoch::new(cow.epoch().get() + 1), 10);
            assert!(!game.entity_broadcast_event(stale, 3));
            assert!(!game.entity_game_event(stale, "minecraft:step"));
            assert_eq!(game.server.lan_entity_events.len(), 1);
            game.stop_audio_capture();
        });
    }

    #[test]
    #[ignore = "daily"]
    fn only_a_spectator_player_is_a_spectator() {
        with_resident_game(|game| {
            let cow = spawn(game, 10, "minecraft:cow");
            assert_eq!(game.entity_spectator(cow), Some(false));
            acting_player(game);
            let player = game.entity(7).unwrap();
            assert_eq!(game.entity_spectator(player), Some(false));
            game.mode = GameMode::Spectator;
            assert_eq!(game.entity_spectator(player), Some(true));
            assert_eq!(game.entity_spectator(cow), Some(false));
        });
    }
}
