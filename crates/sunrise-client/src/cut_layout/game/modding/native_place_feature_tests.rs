//! `level.place_feature` through the public handle: a tree placed as `/place feature` places it,
//! a feature with no room, and a literal test for each refusal.

use mod_api::{LevelRef, PlaceFeatureRefusal, WorldEpoch};
use std::sync::Arc;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_level::chunk::Chunk;

/// The 3x3 chunks around (100, 100): a dirt floor at y 63 under open air, or, with `buried`,
/// stone from y 63 up to y 90.
fn ground(game: &mut sunrise_gameplay::game::ServerGame<'_>, buried: bool) {
    // As a world opens, its live features are installed (`prepare_world`).
    game.install_live_features();
    let registry = &game.pack.assets.registry;
    let dirt = registry.default_state("minecraft:dirt").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    for dz in -1..=1 {
        for dx in -1..=1 {
            let mut chunk = Chunk::empty(100 + dx, 100 + dz);
            for (x, z) in (0..16).flat_map(|x| (0..16).map(move |z| (x, z))) {
                chunk.set(x, 63, z, dirt.0);
                if buried {
                    for y in 63..=90 {
                        chunk.set(x, y, z, stone.0);
                    }
                }
            }
            game.world
                .chunks
                .insert((100 + dx, 100 + dz), Arc::new(chunk));
        }
    }
}

/// The middle of chunk (100, 100), one above the floor.
const AT: [i32; 3] = [1608, 64, 1608];

fn stale(level: LevelRef) -> LevelRef {
    LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension())
}

/// An oak placed on open ground puts its log at the position, as `/place feature minecraft:oak`.
#[test]
#[ignore = "daily"]
fn a_tree_is_placed_as_the_command_places_it() {
    with_game(|game| {
        ground(game, false);
        let level = game.level(game.dim.id()).unwrap();
        assert_eq!(level.place_feature(game, "minecraft:oak", AT), Ok(true));
        let log = game.world.block(AT[0], AT[1], AT[2]);
        assert_eq!(game.res.facts().get(log).name.as_ref(), "minecraft:oak_log");
    });
}

/// A tree buried in stone reports failure, `Ok(false)`, and this one set no block: the stone is
/// untouched.
#[test]
#[ignore = "daily"]
fn a_feature_with_no_room_places_nothing() {
    with_game(|game| {
        ground(game, true);
        let level = game.level(game.dim.id()).unwrap();
        let before = game.world.block(AT[0], AT[1], AT[2]);
        assert_eq!(level.place_feature(game, "minecraft:oak", AT), Ok(false));
        assert_eq!(game.world.block(AT[0], AT[1], AT[2]), before);
    });
}

/// An id that is not a feature, or has no namespace, is `UnknownFeature`.
#[test]
#[ignore = "daily"]
fn an_unknown_feature_is_refused_by_name() {
    with_game(|game| {
        ground(game, false);
        let level = game.level(game.dim.id()).unwrap();
        for id in ["mymod:no_such_tree", "oak", "minecraft:oak_log"] {
            assert_eq!(
                level.place_feature(game, id, AT),
                Err(PlaceFeatureRefusal::UnknownFeature),
                "{id}"
            );
        }
        assert_eq!(
            game.res
                .facts()
                .get(game.world.block(AT[0], AT[1], AT[2]))
                .name
                .as_ref(),
            "minecraft:air"
        );
    });
}

/// A position whose chunk ring is not all loaded, or out of height, is `NotLoaded`.
#[test]
#[ignore = "daily"]
fn unloaded_chunks_are_refused_by_name() {
    with_game(|game| {
        ground(game, false);
        let level = game.level(game.dim.id()).unwrap();
        // Chunk (101, 100) is loaded, but (102, 100) of its ring is not.
        assert_eq!(
            level.place_feature(game, "minecraft:oak", [1624, 64, 1608]),
            Err(PlaceFeatureRefusal::NotLoaded)
        );
        assert_eq!(
            level.place_feature(game, "minecraft:oak", [AT[0], 10_000, AT[2]]),
            Err(PlaceFeatureRefusal::NotLoaded)
        );
    });
}

/// A handle of an earlier open is `Stale`; a level open but not being ticked is `NotInPlace`.
#[test]
#[ignore = "daily"]
fn stale_and_other_levels_are_refused_by_name() {
    with_game(|game| {
        ground(game, false);
        let overworld = game.level("minecraft:overworld").unwrap();
        assert_eq!(
            stale(overworld).place_feature(game, "minecraft:oak", AT),
            Err(PlaceFeatureRefusal::Stale)
        );
        game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
            .unwrap();
        assert_eq!(
            overworld.place_feature(game, "minecraft:oak", AT),
            Err(PlaceFeatureRefusal::NotInPlace)
        );
    });
}

/// `oak_checked` is `oak` behind a `would_survive` filter for an oak sapling: on dirt it places, and
/// on stone the filter leaves no position, so it answers `Ok(false)` and writes nothing.
#[test]
#[ignore = "daily"]
fn a_placed_feature_runs_its_modifiers_before_the_feature() {
    with_game(|game| {
        ground(game, false);
        let level = game.level(game.dim.id()).unwrap();
        assert_eq!(
            level.place_placed_feature(game, "minecraft:oak_checked", AT),
            Ok(true)
        );
        let log = game.world.block(AT[0], AT[1], AT[2]);
        assert_eq!(game.res.facts().get(log).name.as_ref(), "minecraft:oak_log");
    });
    with_game(|game| {
        ground(game, true);
        let level = game.level(game.dim.id()).unwrap();
        let before = game.world.block(AT[0], AT[1], AT[2]);
        assert_eq!(
            level.place_placed_feature(game, "minecraft:oak_checked", AT),
            Ok(false)
        );
        assert_eq!(game.world.block(AT[0], AT[1], AT[2]), before);
    });
}

/// Iceberg blue has rarity 200, in-square, biome: seed 5133 passes rarity;
/// seed 0 rejects it. Only the reached filter refuses, after the expected draws.
#[test]
#[ignore = "daily"]
fn a_biome_filter_refuses_only_when_reached() {
    use sunrise_core::random::{LegacyRandom, RandomSource};
    for (seed, reached) in [(5133, true), (0, false)] {
        with_game(|game| {
            ground(game, false);
            let level = game.level(game.dim.id()).unwrap();
            *game.world.random = LegacyRandom::new(seed);
            let before = game.world.chunks.clone();
            assert_eq!(
                level.place_placed_feature(game, "minecraft:iceberg_blue", AT),
                if reached {
                    Err(PlaceFeatureRefusal::BiomeFilter)
                } else {
                    Ok(false)
                }
            );
            // Compare every resident block: no feature writes preceded the filter.
            assert_eq!(game.world.chunks.len(), before.len());
            for (pos, chunk) in &before {
                let after = &game.world.chunks[pos];
                for y in sunrise_level::chunk::MIN_Y
                    ..sunrise_level::chunk::MIN_Y + sunrise_level::chunk::HEIGHT
                {
                    for z in 0..16 {
                        for x in 0..16 {
                            assert_eq!(after.get(x, y, z), chunk.get(x, y, z));
                        }
                    }
                }
            }
            let mut expected = LegacyRandom::new(seed);
            let rarity = expected.next_float();
            assert_eq!(rarity < 1.0 / 200.0, reached);
            if reached {
                expected.next_int_bound(16).unwrap();
                expected.next_int_bound(16).unwrap();
            }
            assert_eq!(
                <(u64, Option<f64>)>::from(&*game.world.random),
                <(u64, Option<f64>)>::from(&expected)
            );
        });
    }
}

/// Registry-exclusive feature ids must not resolve through the other registry.
#[test]
#[ignore = "daily"]
fn a_placed_feature_id_is_checked_in_its_own_registry() {
    with_game(|game| {
        ground(game, false);
        let level = game.level(game.dim.id()).unwrap();
        for id in ["mymod:no_such_tree", "oak_checked", "minecraft:azalea_tree"] {
            assert_eq!(
                level.place_placed_feature(game, id, AT),
                Err(PlaceFeatureRefusal::UnknownFeature),
                "{id}"
            );
        }
        assert_eq!(
            level.place_feature(game, "minecraft:oak_checked", AT),
            Err(PlaceFeatureRefusal::UnknownFeature)
        );
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
