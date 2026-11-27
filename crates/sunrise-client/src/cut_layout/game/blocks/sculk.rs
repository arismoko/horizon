#![cfg(test)]
use sunrise_core::position::BlockPos;
use sunrise_level::particle_options::ParticleType;
#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) mod shriek_capture_tests {
    use super::*;
    use crate::game::load_resources;
    use crate::settings::Settings;
    use std::sync::Arc;
    use sunrise_client_lib::audio::Audio;
    use sunrise_core::random::RandomSource;
    use sunrise_core::{position::ChunkPos, random::LegacyRandom};
    use sunrise_level::chunk::Chunk;
    use sunrise_world::level::LevelEvent;
    use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};

    #[test]
    #[ignore = "daily"]
    fn level_event_shriek_keeps_local_play_and_random_but_not_shared_capture() {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
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
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("setblock 4 200 4 sculk_shrieker[waterlogged=false] strict"));
        assert!(game.run_command("setblock 7 200 4 sculk_shrieker[waterlogged=true] strict"));
        assert_eq!(
            game.state_property(game.world.block(4, 200, 4), "waterlogged"),
            Some("false")
        );
        assert_eq!(
            game.state_property(game.world.block(7, 200, 4), "waterlogged"),
            Some("true")
        );
        let clock = game.world.game_time;
        let mut world_random = game.world.random.clone();
        let effect = "minecraft:block.sculk_shrieker.shriek";
        let before = "minecraft:block.note_block.pling";
        let after = "minecraft:block.amethyst_block.chime";
        let dry_state = game.world.block(4, 200, 4);
        let wet_state = game.world.block(7, 200, 4);
        // Server role: does not play 3007 locally or record it into pending_sounds.
        game.start_audio_capture();
        game.server_play_sound_on(4, before, [1.5, 200.0, 4.5], 1.0, 1.0);
        game.level_events.push(LevelEvent::LevelEvent {
            source: None,
            id: 3007,
            pos: BlockPos::new(4, 200, 4),
            data: 0,
        });
        game.level_events.push(LevelEvent::LevelEvent {
            source: None,
            id: 3007,
            pos: BlockPos::new(7, 200, 4),
            data: 0,
        });
        game.server_process_level_events().unwrap();
        game.server_play_sound_on(4, after, [2.5, 200.0, 4.5], 0.5, 1.25);
        assert_eq!(game.world.game_time, clock);
        assert_eq!(
            game.world.random.next_long(),
            world_random.next_long(),
            "level random continuation"
        );
        let shared = &game.server.pending_sounds;
        assert_eq!(
            shared.iter().map(|s| s.event.as_str()).collect::<Vec<_>>(),
            vec![before, after],
            "shriek level event must not enter pending_sounds"
        );
        for (sound, (position, volume, pitch)) in shared.iter().zip([
            ([1.5, 200.0, 4.5], 1.0, 1.0),
            ([2.5, 200.0, 4.5], 0.5, 1.25),
        ]) {
            assert_eq!(
                (sound.position, sound.volume, sound.pitch, sound.source),
                (position, volume, pitch, 4)
            );
            assert_eq!((sound.entity, sound.except_entity), (None, None));
        }
        // Client role: plays dry shriek locally with client particle RNG and unshared audio, silent when wet.
        let client_settings = Settings::default();
        let client_loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &client_settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut client_options = Options::parse(std::iter::empty()).unwrap().unwrap();
        client_options.resources = root.clone();
        client_options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        let mut client = sunrise_client_gameplay::game::ClientGame::new(
            &client_loaded.resources,
            client_loaded.client_res.unwrap(),
            client_loaded.pack,
            client_loaded.client_pack.unwrap(),
            client_loaded.gui.unwrap(),
            client_options,
            client_settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root,
            )),
            None,
            sunrise_gameplay::game::server_side::integrated::serve,
            sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
            crate::game::session_verifier(),
            crate::game::profile_resolver,
        )
        .unwrap();
        client
            .world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        client.world.set(4, 200, 4, dry_state).unwrap();
        client.world.set(7, 200, 4, wet_state).unwrap();
        client.client.particles.0.requested = Some(Vec::new());
        client.client.particles.0.seed_randoms(6500, 6501, 6502);
        client.client.audio.0.capture = Some(Vec::new());
        let mut random = LegacyRandom::new(6500);
        let mut client_world_random = client.world.random.clone();
        client.level_events.push(LevelEvent::LevelEvent {
            source: None,
            id: 3007,
            pos: BlockPos::new(4, 200, 4),
            data: 0,
        });
        client.client_process_level_events().unwrap();
        assert_eq!(
            client.client.audio.0.heard.get(effect),
            Some(&1),
            "dry effect must play locally on client"
        );
        random.next_float();
        assert_eq!(
            client.client.particles.0.random().next_long(),
            random.next_long(),
            "dry random continuation"
        );
        client.level_events.push(LevelEvent::LevelEvent {
            source: None,
            id: 3007,
            pos: BlockPos::new(7, 200, 4),
            data: 0,
        });
        client.client_process_level_events().unwrap();
        assert_eq!(
            client.client.audio.0.heard.get(effect),
            Some(&1),
            "waterlogged effect stays silent"
        );
        assert_eq!(
            client.client.particles.0.random().next_long(),
            random.next_long(),
            "wet branch consumes no pitch draw"
        );
        let requests = client.client.particles.0.requested.as_ref().unwrap();
        assert_eq!(requests.len(), 20);
        for (i, (options, position, velocity)) in requests.iter().enumerate() {
            assert_eq!(options.ty, ParticleType::Shriek);
            assert_eq!(options.scale, ((i % 10) * 5) as f32);
            assert_eq!(*position, [if i < 10 { 4.5 } else { 7.5 }, 200.5, 4.5]);
            assert_eq!(*velocity, [0.0; 3]);
        }
        assert_eq!(
            client.world.random.next_long(),
            client_world_random.next_long(),
            "client level random untouched"
        );
        assert!(
            client.client.audio.0.capture.as_ref().unwrap().is_empty(),
            "client local shriek must not enter capture"
        );
    }
}
#[cfg(feature = "client")]
#[cfg(test)]
#[path = "tests/farm_animal_interaction_tests.rs"]
pub(crate) mod farm_animal_interaction_tests;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
