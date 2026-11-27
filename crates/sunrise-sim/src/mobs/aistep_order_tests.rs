//! #1760: `LivingEntity.aiStep` runs, after `travel`, `applyEffectsFromBlocks`
//! (the block walk), the freezing and its hurt, then `pushEntities`
//! (cramming) and the water's hurt; with AI and without.
use super::hurt_teleport_contact_tests::{build, enderman, steps};
use super::*;
use crate::mobs::Seen;
use crate::mobs::world::with_built_test_world;
use std::sync::Arc;

const START: Vec3 = Vec3::new(0.5, 64.0, 0.5);

/// Three pushable NoAI pigs over the enderman's spot, for `maxEntityCramming` 3.
fn crowd(types: &crate::mobs::MobTypes) -> Vec<Seen> {
    [(11, 0.6), (12, 0.7), (13, 0.8)]
        .into_iter()
        .map(|(id, x)| {
            let kind = Arc::new(types.types["minecraft:pig"].clone());
            let mut p = Mob::new(id, kind, Vec3::new(x, 64.0, 0.5), 0.0);
            p.no_ai = true;
            p.no_gravity = true;
            p.seen(None, &crate::items::Items::empty())
        })
        .collect()
}

/// A fully frozen enderman standing in powder snow on the 40-tick beat, its
/// AI paused (no goal draws) or off.
fn frozen_enderman(types: &crate::mobs::MobTypes, seed: i64, no_ai: bool) -> Mob {
    let mut m = enderman(types, seed * 104_729 + 1_234_567);
    m.no_ai = no_ai;
    m.ai.ai_paused = !no_ai;
    m.previous = m.position;
    m.ticks_frozen = 140;
    m.age = 40;
    m
}

/// A crammed, frozen enderman: the freeze hurt draws and teleports first, inside
/// the hurt; cramming's `nextInt(4)` follows only while it still stands in the
/// crowd, and its hurt then teleports in turn. Before #1760 the cramming drew
/// first and the freeze hurt came after it.
#[test]
#[ignore = "daily"]
fn a_crammed_frozen_enderman_is_hurt_by_the_freeze_first() {
    assert!(
        with_built_test_world(
            &[],
            build("minecraft:stone", "minecraft:powder_snow"),
            |w, types| {
                let seen = crowd(types);
                let crowded = MobWorld {
                    side: crate::motion::dry_side(),
                    others: &seen,
                    max_entity_cramming: 3,
                    // The crowd is read from `others`, not the world's empty index.
                    others_grid: None,
                    ..w.clone()
                };
                // Per path: the freeze hurt teleported it out of the crowd; it
                // stayed and the cramming hurt it too.
                let mut seen_cases = [[0; 2]; 2];
                for (path, no_ai) in [true, false].into_iter().enumerate() {
                    for seed in 0..40 {
                        let mut m = frozen_enderman(types, seed, no_ai);
                        let from = m.random.clone();
                        let mut expected = m.clone();
                        hurt(
                            &mut expected,
                            &crowded,
                            &mut Vec::new(),
                            1.0,
                            Damage::of(DamageKind::Freeze),
                        );
                        if expected.position != START {
                            seen_cases[path][0] += 1;
                        } else if expected.random.next_int_bound(4).unwrap() == 0 {
                            hurt(
                                &mut expected,
                                &crowded,
                                &mut Vec::new(),
                                6.0,
                                Damage::of(DamageKind::Cramming),
                            );
                            seen_cases[path][1] += 1;
                        }
                        ai_and_finish_tick(
                            &mut m,
                            &crowded,
                            &mut crate::health::TestLevel::new(0).level(),
                            &mut Vec::new(),
                            None,
                        );
                        let at = format!("no_ai {no_ai}, seed {seed}");
                        assert_eq!(m.position, expected.position, "{at}");
                        assert_eq!(
                            steps(&from, &m.random),
                            steps(&from, &expected.random),
                            "{at}: the hurts' draws"
                        );
                        assert_eq!(
                            m.last_damage.map(|(d, _)| d.kind),
                            expected.last_damage.map(|(d, _)| d.kind),
                            "{at}"
                        );
                    }
                }
                assert!(
                    seen_cases.iter().flatten().all(|n| *n > 0),
                    "teleported out, and crammed after: {seen_cases:?}"
                );
            },
        ),
        "vanilla data is missing"
    );
}

/// A wet, frozen enderman: the freeze hurt (1) lands first, so the water's
/// hurt (1) comes inside the hurt cooldown and is refused; the last damage is
/// the freeze. Before #1760 the water hurt first and the freeze was refused.
#[test]
#[ignore = "daily"]
fn a_wet_frozen_enderman_is_hurt_by_the_freeze_first() {
    assert!(
        with_built_test_world(
            &[],
            build("minecraft:stone", "minecraft:powder_snow"),
            |w, types| {
                for no_ai in [true, false] {
                    for seed in 0..10 {
                        let mut m = frozen_enderman(types, seed, no_ai);
                        m.fluids.in_water = true;
                        let health = m.health;
                        ai_and_finish_tick(
                            &mut m,
                            w,
                            &mut crate::health::TestLevel::new(0).level(),
                            &mut Vec::new(),
                            None,
                        );
                        let at = format!("no_ai {no_ai}, seed {seed}");
                        assert_eq!(
                            m.last_damage.map(|(d, _)| d.kind),
                            Some(DamageKind::Freeze),
                            "{at}"
                        );
                        assert_eq!(health - m.health, 1.0, "{at}: one hurt of 1");
                    }
                }
            },
        ),
        "vanilla data is missing"
    );
}

/// `Mob.isSunBurnTick` reads `wasInPowderSnow`: a zombie whose last tick's walk
/// found powder snow does not burn this tick in plain daylight, with AI (the
/// burn after the walk) or without (the burn before it), where the same draws
/// burn a zombie that was not in it.
#[test]
#[ignore = "daily"]
fn a_zombie_out_of_powder_snow_since_last_tick_does_not_burn_yet() {
    assert!(
        with_built_test_world(
            &[],
            build("minecraft:stone", "minecraft:air"),
            |w, types| {
                let mut day = w.clone();
                day.monsters_burn = true;
                for no_ai in [true, false] {
                    let mut burnt = 0;
                    for seed in 0..100 {
                        let kind = Arc::new(types.types["minecraft:zombie"].clone());
                        let mut m = Mob::new(5, kind, Vec3::new(3.5, 64.0, 3.5), 0.0);
                        register(&mut m);
                        m.random = sunrise_core::random::LegacyRandom::new(seed);
                        m.no_ai = no_ai;
                        m.ai.ai_paused = !no_ai;
                        m.ground.on_ground = true;
                        let mut was = m.clone();
                        was.in_powder_snow = true;
                        let mut level = crate::health::TestLevel::new(0);
                        tick(&mut m, &day, &mut level.level(), &mut Vec::new());
                        tick(&mut was, &day, &mut level.level(), &mut Vec::new());
                        assert!(
                            was.fire_ticks <= 0,
                            "no_ai {no_ai}, seed {seed}: was in powder snow"
                        );
                        burnt += usize::from(m.fire_ticks > 0);
                    }
                    assert!(burnt > 0, "no_ai {no_ai}: some draw burns");
                }
            },
        ),
        "vanilla data is missing"
    );
}
