use crate::game::server_side::native_entity_refs::tests::with_client_game;
use serde_json::json;
use std::sync::Arc;
use sunrise_client::sound_engine::{SoundInstance, TickContext};
use sunrise_core::{geometry::Vec3, random::LegacyRandom};
use sunrise_level::chunk::Chunk;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{Clientbound, EntityEvent},
};
use sunrise_sim::{mobs::Mob, motion::PlayerMotion};
use {
    sunrise_gameplay_common::game::Dim, sunrise_gameplay_common::game::in_own_process,
    sunrise_gameplay_common::game::minecarts::CartKind,
    sunrise_gameplay_common::game::minecarts::Minecart,
};

#[test]
#[ignore = "daily"]
fn minecart_loops_gate_both_gains_on_live_interpolated_rails() {
    in_own_process(
        "cut_layout::game::client_side::music::audio_music_tests::minecart_loops_gate_both_gains_on_live_interpolated_rails",
        || {
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "CartAudioTest").unwrap();
                game.client.remote.as_mut().unwrap().features =
                    Arc::from(["minecraft:minecart_improvements".to_owned()]);

                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let rail = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:rail")
                    .unwrap();
                game.world.set(1, 63, 1, rail).unwrap();
                game.world.set(3, 64, 1, rail).unwrap();

                // At this height the below probe crosses into y=63 only with the
                // f32 epsilon, while a rail in y=64 can still match the fallback.
                let position = |x| Vec3::new(x, 64.100005, 1.5);
                let mut cart = Minecart::new(42, CartKind::Rideable, position(0.5));
                cart.velocity = Vec3::new(0.4, 0.0, 0.0);
                game.carts.push(cart);
                game.riding = Some(42);
                let ctx = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                    let (vehicle, carts) = game.start_cart_sounds();
                    TickContext {
                        vehicle,
                        carts,
                        ..TickContext::default()
                    }
                };
                let start = ctx(game);
                let mut ext = SoundInstance::minecart(&start.carts[0], LegacyRandom::new(1));
                let mut inside =
                    SoundInstance::riding_minecart(&start.carts[0], false, LegacyRandom::new(2));
                let exact_gain = [0x3e8f5c29, 0x3e99999a];
                for (x, on_rails, expected) in [
                    (1.5, true, exact_gain),
                    (2.5, false, [0; 2]),
                    (3.5, true, exact_gain),
                ] {
                    game.tick_remote_objects(
                        |_, at| at,
                        |_, yaw| yaw,
                        |cart| {
                            cart.position = position(x);
                            true
                        },
                    );
                    let inputs = ctx(game);
                    assert!(inputs.carts[0].new_behavior);
                    assert_eq!(inputs.carts[0].on_rails, on_rails);
                    ext.tick(&inputs);
                    inside.tick(&inputs);
                    assert_eq!([ext.volume.to_bits(), inside.volume.to_bits()], expected);
                    assert!(!ext.stopped && !inside.stopped);
                }

                game.client.remote.as_mut().unwrap().features = Arc::from([]);
                game.carts.clear();
                let mut old_cart = Minecart::new(43, CartKind::Rideable, position(2.5));
                old_cart.velocity = Vec3::new(0.4, 0.0, 0.0);
                game.carts.push(old_cart);
                game.riding = Some(43);
                let inputs = ctx(game);
                let old_cart = &inputs.carts[0];
                assert!(!old_cart.new_behavior && !old_cart.on_rails);
                let mut ext = SoundInstance::minecart(old_cart, LegacyRandom::new(1));
                let mut inside =
                    SoundInstance::riding_minecart(old_cart, false, LegacyRandom::new(2));
                ext.tick(&inputs);
                inside.tick(&inputs);
                assert_eq!([ext.volume.to_bits(), inside.volume.to_bits()], exact_gain);
                assert!(!ext.stopped && !inside.stopped);
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn guardian_and_elder_events_admit_two_independent_loops_each() {
    std::thread_local! {
        static STARTS: std::cell::RefCell<Vec<(u64, [f64; 3])>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    in_own_process(
        "cut_layout::game::client_side::music::audio_music_tests::guardian_and_elder_events_admit_two_independent_loops_each",
        || {
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "GuardianAudioTest").unwrap();
                game.client.audio.0.engine.loaded = true;
                game.client.audio.0.engine.registry.add_file(
                    "minecraft",
                    br#"{"entity.guardian.attack":{"sounds":["fixture/guardian"]}}"#,
                    &|_, _| true,
                );
                game.client
                    .audio
                    .0
                    .engine
                    .set_test_playback_hook(|sound, _, _, _| {
                        if sound.starting && sound.identifier == "minecraft:entity.guardian.attack"
                        {
                            STARTS.with_borrow_mut(|starts| {
                                starts.push((sound.instance, sound.position))
                            });
                        }
                    });
                for (id, kind, x, silent) in [
                    (101, "minecraft:guardian", 1.0, false),
                    (102, "minecraft:elder_guardian", 3.0, false),
                    (103, "minecraft:guardian", 5.0, true),
                ] {
                    let mut mob = Mob::new(
                        id as u32,
                        game.res.mob_types()[kind].clone(),
                        Vec3::new(x, 64.0, 0.0),
                        0.0,
                    );
                    mob.silent = silent;
                    game.mobs.push(mob);
                    for _ in 0..2 {
                        game.handle_packet(Clientbound::EntityEvent(EntityEvent { id, event: 21 }))
                            .unwrap();
                    }
                }
                let starts = STARTS.take();
                assert_eq!(starts.len(), 4);
                assert!(starts.windows(2).all(|w| w[0].0 < w[1].0));
                assert_eq!(
                    starts.iter().map(|(_, pos)| *pos).collect::<Vec<_>>(),
                    [[0.0; 3]; 4]
                );

                game.tick_sound_engine();
                let ticked: Vec<[f64; 3]> = starts
                    .iter()
                    .map(|(id, _)| game.client.audio.0.engine.instance(*id).unwrap().position)
                    .collect();
                assert_eq!(
                    ticked,
                    [
                        [1.0, 64.0, 0.0],
                        [1.0, 64.0, 0.0],
                        [3.0, 64.0, 0.0],
                        [3.0, 64.0, 0.0],
                    ]
                );
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn music_selection_and_gain_follow_the_camera_across_a_biome_boundary() {
    in_own_process(
        "cut_layout::game::client_side::music::audio_music_tests::music_selection_and_gain_follow_the_camera_across_a_biome_boundary",
        || {
            with_client_game(|game| {
                game.client.title.0 = None;
                game.dim = Dim::Overworld;
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(-20.0, 64.0, 0.0)).unwrap());

                let biome = |sound, volume, min, max, replace| {
                    json!({"attributes": {
                        "minecraft:audio/background_music": {"default": {
                            "sound": sound, "min_delay": min, "max_delay": max, "replace_current_music": replace
                        }},
                        "minecraft:audio/music_volume": volume
                    }})
                };
                let a = biome("minecraft:music.game", 0.25, 100, 200, false);
                let b = biome(
                    "minecraft:music.nether.crimson_forest",
                    0.75,
                    300,
                    400,
                    true,
                );
                let biomes = game
                    .res
                    .biomes()
                    .with_packs([("test:biome_a", &a), ("test:biome_b", &b)])
                    .unwrap();
                for (x, name) in [(-20, "test:biome_a"), (20, "test:biome_b")] {
                    let mut chunk = Chunk::empty(x >> 4, 0);
                    chunk.set_biome_cell(
                        (x >> 2) & 3,
                        (64 - chunk.min_y()) >> 2,
                        0,
                        biomes.id(name),
                    );
                    game.world.chunks.insert((x >> 4, 0), Arc::new(chunk));
                }
                game.world_biomes = Some(Arc::new(biomes));
                game.client.view.position = [-20.0, 64.0, 0.0];
                let (left, volume) = game.situational_music();
                assert_eq!(left.unwrap().sound, "minecraft:music.game");
                assert_eq!(volume.to_bits(), 0x3e800000);

                game.client.view.position = [20.0, 64.0, 0.0];
                let (right, volume) = game.situational_music();
                let right = right.unwrap();
                assert_eq!(right.sound, "minecraft:music.nether.crimson_forest");
                assert_eq!(
                    (
                        right.min_delay,
                        right.max_delay,
                        right.replace_current_music
                    ),
                    (300, 400, true)
                );
                assert_eq!(volume.to_bits(), 0x3f400000);
            })
        },
    );
}
