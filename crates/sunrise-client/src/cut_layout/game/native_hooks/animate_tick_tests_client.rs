use crate::game::native_hooks::animate_tick_tests::with_hearts;
use mod_api::BlockPos;
use std::sync::Arc;
use sunrise_core::random::{LegacyRandom, RandomSource as _};
use sunrise_level::particle_options::ParticleType;
use sunrise_world::registry::BlockStateId;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
};

pub(crate) fn level_random(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
) -> LegacyRandom {
    LegacyRandom::from(<(u64, Option<f64>)>::from(
        &*game.client.particles.0.level_random(),
    ))
}

#[test]
#[ignore = "daily"]
fn sampled_picks_reach_the_parts_of_their_block_only() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            assert!(
                !game.mod_animate_parts(),
                "with no part answering, the sampler asks nothing"
            );
        })
    });
    with_hearts(|game, calls| {
        assert!(game.mod_animate_parts());
        let registry = &game.pack.assets.registry;
        let glass = registry.default_state("minecraft:glass").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        assert!(sunrise_client_gameplay::game::ClientGame::animate_part_listens(glass));
        assert!(!sunrise_client_gameplay::game::ClientGame::animate_part_listens(stone));
        // Under All the limiter draws nothing: the part's two draws are the level random's next.
        game.client.particles.0.particle_level = 0;
        let mut expected = level_random(game);
        game.mod_animate_tick(glass, [1, 64, 2]);
        let first = expected.next_float();
        let second = expected.next_float();
        let glass_id = "minecraft:glass".to_owned();
        assert_eq!(
            *calls.lock().unwrap(),
            [(
                BlockPos::new(1, 64, 2),
                glass_id.clone(),
                first,
                second,
                true
            )]
        );
        assert_eq!(
            game.client.particles.0.level_random().next_long(),
            expected.next_long(),
            "the part drew the level's own random"
        );
        // A client tick's real sampling: a floor of glass under the player with a stone row;
        // only glass positions reach the part.
        calls.lock().unwrap().clear();
        for x in -8..24 {
            for z in -8..24 {
                let state = if x == 8 { stone } else { glass };
                game.world.set(x, 64, z, state).unwrap();
            }
        }
        for _ in 0..4 {
            game.tick_particles();
        }
        let calls = calls.lock().unwrap();
        assert!(!calls.is_empty(), "the sampling picked a glass block");
        for (pos, block, ..) in calls.iter() {
            assert_eq!(pos.y, 64);
            assert_ne!(pos.x, 8, "the stone row has no part");
            assert_eq!(*block, glass_id);
        }
    });
}

/// Review of #2656: `event.random` and `add_particle` advance one level random in call order.
/// Under Decreased the limiter rolls the level random for each particle (one in three
/// dropped): the part's second draw and the level random after the call both come after that
/// roll.
#[test]
#[ignore = "daily"]
fn the_limiter_roll_of_add_particle_moves_the_parts_random() {
    with_hearts(|game, calls| {
        let glass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:glass")
            .unwrap();
        game.client.settings.particles = 1;
        game.client.particles.0.particle_level = 1;
        let before = level_random(game);
        let mut expected = before.clone();
        game.mod_animate_tick(glass, [8, 64, 8]);
        // A draw, the limiter's `nextInt(3)`, a draw.
        let first = expected.next_float();
        expected.next_int_bound(3).unwrap();
        let second = expected.next_float();
        assert_eq!(
            *calls.lock().unwrap(),
            [(
                BlockPos::new(8, 64, 8),
                "minecraft:glass".to_owned(),
                first,
                second,
                true
            )]
        );
        assert_eq!(
            game.client.particles.0.level_random().next_long(),
            expected.next_long(),
            "the level random after the call counts the limiter roll"
        );
        // Without the roll in between, the second draw would have been another value.
        let mut skipped = before;
        skipped.next_float();
        assert_ne!(skipped.next_float(), second);
    });
}

/// ClientLevel.tickWeatherEffects precedes animateTick, whose block part runs before the pick's fluid.
#[test]
#[ignore = "daily"]
fn rain_precedes_animation_and_the_part_runs_before_its_fluid() {
    with_hearts(|game, calls| {
        let registry = &game.pack.assets.registry;
        let rod = registry.default_state("minecraft:lightning_rod").unwrap();
        let mut descriptor = registry.state(rod).unwrap().clone();
        descriptor
            .properties
            .insert("waterlogged".into(), "true".into());
        let rod: BlockStateId = registry.resolve(&descriptor).unwrap();
        for x in -8..24 {
            for z in -8..24 {
                game.world.set(x, 64, z, rod).unwrap();
            }
        }
        // Thundering, and late enough in the 200-tick cycle that every topmost rod sparks.
        game.world.weather.rain_level = (1.0, 1.0);
        game.world.weather.thunder_level = (1.0, 1.0);
        game.world.game_time = 199;
        // The empty chunks' biome, made a rainy one.
        let mut biomes = sunrise_level::biome::Biomes::clone(game.res.biomes());
        biomes.list[0].precipitation = true;
        biomes.list[0].temperature = 0.8;
        game.world_biomes = Some(Arc::new(biomes));
        game.client.settings.particles = 0;
        game.client.settings.sunrise.weather = true;
        // Run after a camera frame: tickWeatherEffects uses the camera position,
        // which is still the origin until the camera has aligned with its entity.
        game.camera_view(1.0).expect("camera over the rod floor");
        game.client.particles.0.requested = Some(Vec::new());
        game.tick_particles();
        let log: Vec<(ParticleType, [i32; 3])> = game
            .client
            .particles
            .0
            .requested
            .take()
            .unwrap()
            .into_iter()
            .map(|(options, at, _)| (options.ty, at.map(|v| v.floor() as i32)))
            .collect();
        let count = |ty| log.iter().filter(|(t, _)| *t == ty).count();
        let hearts = count(ParticleType::Heart);
        assert_eq!(hearts, calls.lock().unwrap().len());
        assert!(hearts > 0, "the sampling picked a rod");
        assert!(count(ParticleType::Underwater) > 0, "a rod's water bubbled");
        assert!(count(ParticleType::Rain) > 0, "the rain splashed");
        for (i, &(ty, at)) in log.iter().enumerate() {
            match ty {
                // The rod's sparks are followed by more of its sparks or its part's heart:
                // never by its water, the next pick or the rain.
                ParticleType::ElectricSpark => {
                    let (next, next_at) = log[i + 1];
                    assert!(
                        next == ParticleType::ElectricSpark
                            || (next == ParticleType::Heart && next_at == at),
                        "spark {i} at {at:?} followed by {next:?} at {next_at:?}"
                    );
                }
                // The heart comes right after its own rod's sparks.
                ParticleType::Heart => {
                    assert!(i > 0 && log[i - 1] == (ParticleType::ElectricSpark, at));
                }
                // The water's bubble comes right after the part of its own pick.
                ParticleType::Underwater => {
                    assert_eq!(log[i - 1], (ParticleType::Heart, at), "bubble {i}");
                }
                _ => {}
            }
        }
        let first_heart = log
            .iter()
            .position(|(t, _)| *t == ParticleType::Heart)
            .unwrap();
        let last_rain = log
            .iter()
            .rposition(|(t, _)| *t == ParticleType::Rain)
            .unwrap();
        assert!(
            last_rain < first_heart,
            "the level tick splashes rain before animation"
        );
    });
}
