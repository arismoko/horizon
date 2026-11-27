use mod_api::{BlockPos, Direction, ModEntry};
use std::sync::Arc;
use sunrise_client::particles::{ParticleLevel, requested_count};
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource as _},
};
use sunrise_gameplay_common::game::native_hooks::block_particle_effects_tests::Calls;
use sunrise_gameplay_common::game::native_hooks::block_particle_effects_tests::Register;
use sunrise_gameplay_common::game::native_hooks::block_particle_effects_tests::fresh;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;
use {
    crate::game::client_side::native_client_session::tests::logged_client,
    crate::game::native_entity_refs::tests::fixture_root,
    crate::game::native_entity_refs::tests::with_game_role_and_facts_client,
    sunrise_gameplay_common::game::execution_role::ExecutionRole,
};

pub(in crate::game::client_side) fn admit(game: &mut crate::game::ClientGame<'_>) -> usize {
    game.client.particles.0.tick(&ParticleLevel {
        world: &game.common.world,
        facts: game.common.res.facts(),
        collision: game.common.collision,
        player: None,
        cached_chunk: std::cell::Cell::new(None),
    });
    game.client.particles.0.len()
}

#[test]
#[ignore = "daily"]
fn break_event_uses_the_destroyed_state_and_keeps_its_sound() {
    if fresh("break_event_uses_the_destroyed_state_and_keeps_its_sound") {
        return;
    }
    with_effects(|game, calls| {
        let glass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:glass")
            .unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let barrier = game
            .pack
            .assets
            .registry
            .default_state("minecraft:barrier")
            .unwrap();
        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        assert!(game.res.facts().get(stone).spawn_terrain_particles);
        assert!(!game.res.facts().get(barrier).spawn_terrain_particles);
        game.world.set(4, 64, 4, air).unwrap();
        game.side.particles.0.requested = Some(Vec::new());
        let before_part = requested_count();
        game.client_level_event_at(2001, [4, 64, 4], glass.0 as i32);
        assert_eq!(game.side.particles.0.requested.take().unwrap().len(), 1);
        assert_eq!(admit(game), 1, "the part replaces the entire vanilla burst");
        let [sound] = game.side.audio.0.capture.as_deref().unwrap() else {
            panic!("expected exactly one break sound");
        };
        assert_eq!(sound.event, "minecraft:block.glass.break");
        assert_eq!(
            sound.particles_before, before_part,
            "the sound precedes the Part's request"
        );
        assert_eq!(game.world.block(4, 64, 4), air);
        game.side.particles.0.clear();
        game.client_level_event_at(2001, [4, 64, 4], stone.0 as i32);
        assert_eq!(
            admit(game),
            64,
            "an unhandled full block gives the vanilla burst"
        );
        game.side.particles.0.clear();
        let cobblestone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cobblestone")
            .unwrap();
        assert!(!game.res.facts().get(cobblestone).spawn_terrain_particles);
        assert!(game.particle_sprite(cobblestone, [4, 64, 4]).is_some());
        assert!(
            game.res
                .facts()
                .get(cobblestone)
                .outline
                .iter()
                .any(|b| b[4] > b[1])
        );
        game.destroy_particles([4, 64, 4], cobblestone);
        assert_eq!(
            admit(game),
            0,
            "an unhandled false flag suppresses debris even on a drawable state"
        );
        game.destroy_particles([4, 64, 4], barrier);
        assert_eq!(
            admit(game),
            0,
            "an invisible non-air state can handle destruction"
        );
        game.destroy_particles([4, 64, 4], air);
        assert_eq!(admit(game), 0);
        let pos = BlockPos::new(4, 64, 4);
        assert_eq!(
            *calls.lock().unwrap(),
            [
                ("minecraft:glass".into(), pos, None),
                ("minecraft:barrier".into(), pos, None)
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn hit_effect_replaces_the_crumb_but_not_the_sound_or_level_random() {
    if fresh("hit_effect_replaces_the_crumb_but_not_the_sound_or_level_random") {
        return;
    }
    with_effects(|game, calls| {
        for (name, expected) in [
            ("glass", 1),
            ("dirt", 1),
            ("stone", 1),
            ("barrier", 0),
            ("air", 0),
        ] {
            let state = game
                .pack
                .assets
                .registry
                .default_state(&format!("minecraft:{name}"))
                .unwrap();
            game.world.set(4, 64, 4, state).unwrap();
            game.side.particles.0.clear();
            game.side.audio.0.heard.clear();
            game.side.particles.0.seed_randoms(10, 11, 12);
            game.breaking_block_effects([4, 64, 4], [-1, 0, 0], true);
            assert_eq!(admit(game), expected, "{name}");
            if name == "glass" {
                // The client-only hit sound intentionally bypasses the server's effect capture.
                assert_eq!(
                    game.side.audio.0.heard,
                    [("minecraft:block.glass.hit".into(), 1)].into()
                );
                let mut expected_random = LegacyRandom::new(11);
                expected_random.next_int();
                for _ in 0..3 {
                    expected_random.next_double();
                }
                assert_eq!(
                    game.side.particles.0.level_random().next_long(),
                    expected_random.next_long()
                );
                let mut quads = Vec::new();
                game.side.particles.0.render(
                    glam::DVec3::ZERO,
                    glam::Quat::IDENTITY,
                    0.0,
                    &|_| 0xff,
                    &mut quads,
                );
                assert_eq!(quads.len(), 4);
                assert!(quads.iter().all(|vertex| vertex.pos[0] > 7.0));
            } else if name == "air" {
                assert!(game.side.audio.0.heard.is_empty());
            }
        }
        let pos = BlockPos::new(4, 64, 4);
        assert_eq!(
            *calls.lock().unwrap(),
            [
                ("minecraft:glass".into(), pos, Some(Direction::West)),
                ("minecraft:dirt".into(), pos, Some(Direction::West))
            ]
        );

        let level = game.native_client_level().unwrap();
        let grass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:short_grass")
            .unwrap();
        game.side.streaming.biome_blend = 2;
        let biomes = world_biomes!(game);
        let tint = biomes.grass(biomes.id("minecraft:plains"), 0.0, 0.0);
        let c = |shift: u32| 0.6 * ((tint >> shift) & 255) as f32 / 255.0;
        for edge in [[i32::MIN, 64, i32::MAX], [i32::MAX, 64, i32::MIN]] {
            game.side.particles.0.clear();
            assert_eq!(
                game.level_add_breaking_particles(
                    level,
                    edge,
                    grass,
                    sunrise_core::geometry::Direction::West
                ),
                Ok(true)
            );
            assert_eq!(
                game.level_add_terrain_particle(level, grass, edge, [4.5, 64.5, 4.5], [0.0; 3]),
                Ok(true)
            );
            assert_eq!(admit(game), 2);
            for particle in game.side.particles.0.live() {
                assert_eq!(particle.rgba, [c(16), c(8), c(0), 1.0]);
            }
        }
    });
}

pub(in crate::game::client_side) fn with_effects(
    test: impl FnOnce(&mut crate::game::ClientGame<'_>, &Calls),
) {
    let calls = Calls::default();
    let mods = vec![ModEntry::new("debris", Box::new(Register(calls.clone())))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_game_role_and_facts_client(
            fixture_root(),
            ExecutionRole::Client,
            Some(|registry, facts| {
                // A visible full cube with a sprite, but no subscribed destroy Part.
                let state = registry.default_state("minecraft:cobblestone").unwrap();
                facts.states[state.0 as usize].spawn_terrain_particles = false;
            }),
            |game| {
                let _server = logged_client(game);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 63.0, 4.5)).unwrap());
                game.side.audio.0.capture = Some(Vec::new());
                test(game, &calls);
            },
        );
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
