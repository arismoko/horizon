//! Where the game puts the mob a tick conversion makes (#4876): a conversion vanilla makes
//! before the tick's travel (`AbstractPiglin.customServerAiStep`, inside `Mob.serverAiStep`;
//! `Skeleton.tick`'s `ConversionTracker.tick`, before `super.tick()`) takes the place the mob had
//! then (`game::piglin_conversion_tests`); one vanilla makes after it (`Zombie.tick`'s
//! `ConversionTracker.tick`, after `super.tick()`; `Tadpole.aiStep`'s ageing, after
//! `super.aiStep()`) takes the place travel moved the mob to.
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::ChunkPos;
use sunrise_sim::mobs::Species;
use {
    sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

/// A frozen serving level with a stone floor at y 99 and water from y 100 to 102 around the
/// origin; the mob `summon` adds there, walking, set up by `setup`. One tick later: where the
/// mob stood before it, and where the mob of `into` that replaced it stands.
fn converted_place(
    summon: &str,
    into: Species,
    setup: impl FnOnce(&mut sunrise_sim::mobs::Mob),
) -> (Vec3, Vec3) {
    let mut outcome = None;
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            for command in [
                "tick freeze",
                "forceload add -32 -32 31 31",
                "fill -31 99 -31 31 99 31 stone",
                "fill -8 100 -8 8 102 8 water",
                summon,
            ] {
                assert!(game.run_command(command), "{command}");
            }
            game.creative = false;
            let m = game
                .mobs
                .iter_mut()
                .find(|m| m.species() != into)
                .expect("the mob was summoned");
            let id = m.id;
            m.velocity = Vec3::new(0.3, 0.0, 0.0);
            setup(m);
            let at = m.position;
            assert!(game.run_command("tick step 1"));
            game.tick_server(false).unwrap();
            assert!(
                !game.mobs.iter().any(|m| m.id == id && !m.removed),
                "the mob converted"
            );
            let new = game
                .mobs
                .iter()
                .find(|m| m.species() == into)
                .expect("the new mob joined");
            outcome = Some((at, new.position));
        })
    });
    outcome.unwrap()
}

/// `Zombie.tick` runs `super.tick()`, travel included, before its `ConversionTracker.tick`
/// converts it: the drowned stands where the zombie's travel took it, not where it stood.
#[test]
#[ignore = "daily"]
fn a_drowned_stands_where_the_zombies_travel_took_it() {
    let (before, drowned) = converted_place(
        "summon minecraft:zombie 0.5 100 0.5 {PersistenceRequired:1b}",
        Species::Drowned,
        |m| {
            // `DrownedConversionTime` at 1: it converts this tick.
            m.extra.counter2 = 1;
        },
    );
    assert!(drowned.x > before.x, "{drowned:?} moved on from {before:?}");
}

/// `Tadpole.aiStep` ages it after `super.aiStep()`: the frog stands where the tadpole's travel
/// took it.
#[test]
#[ignore = "daily"]
fn a_tadpoles_frog_stands_where_the_tadpoles_travel_took_it() {
    let (before, frog) = converted_place(
        "summon minecraft:tadpole 0.5 100.5 0.5 {PersistenceRequired:1b,Age:23999}",
        Species::Frog,
        |_| {},
    );
    assert!(frog.x > before.x, "{frog:?} moved on from {before:?}");
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
