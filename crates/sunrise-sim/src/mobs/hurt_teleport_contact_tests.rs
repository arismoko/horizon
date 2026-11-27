use super::*;
use crate::mobs::world::{with_built_contact_test_world, with_built_test_world};
use std::sync::Arc;
use sunrise_world::registry::BlockRegistry;

/// A stone floor over the loaded 2x2 chunks (so a teleport finds ground),
/// `under` below the mob's spot and `at` in it.
pub(super) fn build(
    under: &'static str,
    at: &'static str,
) -> impl FnOnce(&mut crate::world::World, &BlockRegistry) {
    move |world, registry| {
        let stone = registry.default_state("minecraft:stone").unwrap();
        for cx in -1..1 {
            for cz in -1..1 {
                world
                    .chunks
                    .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
            }
        }
        for x in -16..16 {
            for z in -16..16 {
                world.set(x, 63, z, stone).unwrap();
            }
        }
        world
            .set(0, 63, 0, registry.default_state(under).unwrap())
            .unwrap();
        world
            .set(0, 64, 0, registry.default_state(at).unwrap())
            .unwrap();
    }
}

pub(super) fn enderman(types: &crate::mobs::MobTypes, seed: i64) -> Mob {
    let kind = Arc::new(types.types["minecraft:enderman"].clone());
    let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    register(&mut m);
    m.random = sunrise_core::random::LegacyRandom::new(seed);
    m.ground.on_ground = true;
    m.stuck_multiplier = Vec3::ZERO;
    m
}

/// `Entity.applyEffectsFromBlocks` fixes the tick's movements before
/// `stepOn`: when the magma's hurt teleports an enderman, the scan still
/// walks the path it had, and the cobweb it stood in still holds it.
#[test]
#[ignore = "daily"]
fn a_magma_teleport_leaves_the_block_scan_on_the_old_path() {
    assert!(
        with_built_test_world(
            &[],
            build("minecraft:magma_block", "minecraft:cobweb"),
            |w, types| {
                let mut teleported = 0;
                for seed in 0..40 {
                    let mut m = enderman(types, seed);
                    let mut test_level = crate::health::TestLevel::new(42);
                    let mut random = test_level.level();
                    let mut events = Vec::new();
                    effects_from_blocks(&mut m, w, &mut random, &mut events);
                    if m.position != Vec3::new(0.5, 64.0, 0.5) {
                        teleported += 1;
                        assert_eq!(
                            m.stuck_multiplier,
                            Vec3::new(0.25, f64::from(0.05_f32), 0.25),
                            "seed {seed}: the cobweb on the old path"
                        );
                    }
                }
                assert!(teleported > 0, "some seed teleports");
            },
        ),
        "vanilla data is missing"
    );
}

/// A hurt in `baseTick` that teleports an enderman (`snapTo` resets the
/// old position) leaves aiStep's fire contact at the landing spot: no
/// fire there, so the fire timer only counts down (no re-ignite to 160).
#[test]
#[ignore = "daily"]
fn a_burning_teleport_does_not_take_the_old_spots_fire_along() {
    assert!(
        with_built_test_world(
            &[],
            build("minecraft:stone", "minecraft:fire"),
            |w, types| {
                let mut teleported = 0;
                for seed in 0..40 {
                    let mut m = enderman(types, seed);
                    m.fire_ticks = 20;
                    // Whether the burning hurt's own teleport lands (one in ten
                    // draws none, and a try can find no ground: the old spot's
                    // fire then ignites it before the second hurt teleports it).
                    let mut first = m.clone();
                    let mut events = Vec::new();
                    hurt(
                        &mut first,
                        w,
                        &mut events,
                        1.0,
                        Damage::of(DamageKind::OnFire),
                    );
                    base_tick(
                        &mut m,
                        w,
                        &mut crate::health::TestLevel::new(0).level(),
                        &mut events,
                    );
                    effects_from_blocks(
                        &mut m,
                        w,
                        &mut crate::health::TestLevel::new(0).level(),
                        &mut events,
                    );
                    if first.position != Vec3::new(0.5, 64.0, 0.5) {
                        assert_eq!(m.position, first.position, "seed {seed}: the same teleport");
                        teleported += 1;
                        assert_eq!(m.fire_ticks, 19, "seed {seed}: no fire where it landed");
                    }
                }
                assert!(teleported > 0, "some seed teleports");
            },
        ),
        "vanilla data is missing"
    );
}

/// How many generator steps (`next(bits)` calls: `next_double` is two)
/// `start` took to reach `end`.
pub(super) fn steps(
    start: &sunrise_core::random::LegacyRandom,
    end: &sunrise_core::random::LegacyRandom,
) -> u32 {
    use sunrise_core::random::RandomSource as _;
    let mut r = start.clone();
    let mut n = 0;
    while r.state() != end.state() {
        r.next_int();
        n += 1;
        assert!(n < 1000, "the draws never meet");
    }
    n
}

fn hurt_sounds(events: &[MobEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, MobEvent::Sound("entity.enderman.hurt", ..)))
        .count()
}

/// #1608: `LivingEntity.tickEffects` hurts an enderman twice in one tick
/// (Wither, then Poison inside the hurt cooldown). `Enderman.hurtServer`
/// draws and teleports inside each hurt (the oracle's trace, `mob_ai`
/// `enderman_two_hurts_a_tick` tick 1): the hurt sound's pitch (2 draws),
/// `nextInt(10)` and, unless it is 0, `teleport()`'s x, y and z (5 draws);
/// then the refused Poison hurt: no sound, `nextInt(10)` and its teleport.
#[test]
#[ignore = "daily"]
fn two_effect_hurts_in_a_tick_each_teleport_inside_the_hurt() {
    use sunrise_core::random::RandomSource as _;
    assert!(
        with_built_test_world(
            &[],
            build("minecraft:stone", "minecraft:air"),
            |w, types| {
                let mut teleported = 0;
                for seed in 0..40 {
                    let mut m = enderman(types, seed);
                    m.add_effect("minecraft:wither", 40, 0);
                    m.add_effect("minecraft:poison", 50, 0);
                    let start = m.random.clone();
                    let mut events = Vec::new();
                    tick_effects_hurting(
                        &mut m,
                        w.loop_context,
                        &mut crate::health::TestLevel::new(0).level(),
                        w.mod_effects,
                        &mut |m| hurt_teleports(m, w, &mut events),
                    );
                    // A hurt with no teleport to settle leaves its sound in the outbox,
                    // which the tick hands on after the effects (`base_tick`).
                    events.append(&mut m.outbox);
                    // Vanilla's draws, counted from the same random.
                    let mut vanilla = start.clone();
                    vanilla.next_float();
                    vanilla.next_float();
                    for _ in 0..2 {
                        if vanilla.next_int_bound(10).unwrap() != 0 {
                            vanilla.next_double();
                            vanilla.next_int_bound(64).unwrap();
                            vanilla.next_double();
                        }
                    }
                    assert_eq!(
                        steps(&start, &m.random),
                        steps(&start, &vanilla),
                        "seed {seed}: vanilla's generator steps"
                    );
                    assert_eq!(
                        hurt_sounds(&events),
                        1,
                        "seed {seed}: the refused hurt is silent"
                    );
                    // The hurt sound leaves the outbox before the teleport's.
                    assert!(
                        matches!(
                            events.iter().find(|e| matches!(e, MobEvent::Sound(..))),
                            Some(MobEvent::Sound("entity.enderman.hurt", ..))
                        ),
                        "seed {seed}: the hurt sound comes first"
                    );
                    assert_eq!(
                        m.health,
                        m.attrs.max_health - 1.0,
                        "seed {seed}: one hurt lands"
                    );
                    if m.position != Vec3::new(0.5, 64.0, 0.5) {
                        teleported += 1;
                    }
                }
                assert!(teleported > 0, "some seed teleports");
            },
        ),
        "vanilla data is missing"
    );
}

/// #1608: `base_tick` settles each of its hurts inside the hurt. An enderman
/// hurt in a wall, below the world (`LivingEntity.onBelowWorld`) or by
/// drowning, then by Wither and Poison, ends where those three hurts, each
/// with its teleport, dealt one after the other leave it, with the same draws.
/// Below the world no teleport finds ground, so the draws alone tell.
#[test]
#[ignore = "daily"]
fn a_base_tick_hurt_and_two_effect_hurts_each_teleport_inside_the_hurt() {
    let world = |world: &mut crate::world::World, registry: &BlockRegistry| {
        build("minecraft:stone", "minecraft:air")(world, registry);
        // A stone at the eyes of an enderman standing at (0, 64, 0), and water
        // at the eyes of one standing at (8, 64, 8).
        let stone = registry.default_state("minecraft:stone").unwrap();
        world.set(0, 66, 0, stone).unwrap();
        let water = registry.default_state("minecraft:water").unwrap();
        for y in 64..68 {
            world.set(8, y, 8, water).unwrap();
        }
    };
    assert!(
        with_built_test_world(&[], world, |w, types| {
            let below = f64::from(w.world.bottom_y() - 65);
            let cases = [
                (
                    "in a wall",
                    Vec3::new(0.5, 64.0, 0.5),
                    DamageKind::InWall,
                    1.0,
                ),
                (
                    "below the world",
                    Vec3::new(0.5, below, 0.5),
                    DamageKind::OutOfWorld,
                    4.0,
                ),
                (
                    "drowning",
                    Vec3::new(8.5, 64.0, 8.5),
                    DamageKind::Drown,
                    2.0,
                ),
            ];
            for (name, at, kind, amount) in cases {
                let mut landed = 0;
                for seed in 0..40 {
                    let mut m = enderman(types, seed);
                    m.position = at;
                    if kind == DamageKind::Drown {
                        m.air = -19;
                    }
                    m.add_effect("minecraft:wither", 40, 0);
                    m.add_effect("minecraft:poison", 50, 0);
                    let start = m.random.clone();
                    let mut expected = m.clone();
                    let mut expected_events = Vec::new();
                    hurt(
                        &mut expected,
                        w,
                        &mut expected_events,
                        amount,
                        Damage::of(kind),
                    );
                    for kind in [DamageKind::Wither, DamageKind::Magic] {
                        hurt(
                            &mut expected,
                            w,
                            &mut expected_events,
                            1.0,
                            Damage::of(kind),
                        );
                    }
                    let mut events = Vec::new();
                    base_tick(
                        &mut m,
                        w,
                        &mut crate::health::TestLevel::new(0).level(),
                        &mut events,
                    );
                    assert_eq!(m.position, expected.position, "{name}, seed {seed}");
                    // Plus the ambient sound's roll at the end of `Mob.baseTick` (line
                    // 324 in the oracle's trace), after the effects.
                    assert_eq!(
                        steps(&start, &m.random),
                        steps(&start, &expected.random) + 1,
                        "{name}, seed {seed}: the same generator steps"
                    );
                    assert_eq!(
                        hurt_sounds(&events),
                        1,
                        "{name}, seed {seed}: one hurt sound"
                    );
                    if m.position != at {
                        landed += 1;
                    }
                }
                // Below the world no teleport finds ground; elsewhere some land.
                assert_eq!(
                    landed > 0,
                    kind != DamageKind::OutOfWorld,
                    "{name}: landings"
                );
            }
        }),
        "vanilla data is missing"
    );
}

/// #1608: an enderman Wither kills still draws `Enderman.hurtServer`'s
/// `nextInt(10)` in the refused hurt that follows (instant damage), and
/// `teleport()` draws nothing for a dead mob.
#[test]
#[ignore = "daily"]
fn a_dead_endermans_refused_hurt_still_draws_its_chance() {
    assert!(
        with_built_test_world(
            &[],
            build("minecraft:stone", "minecraft:air"),
            |w, types| {
                for seed in 0..10 {
                    let run = |second: bool| {
                        let mut m = enderman(types, seed);
                        m.health = 1.0;
                        m.add_effect("minecraft:wither", 40, 0);
                        if second {
                            m.add_effect("minecraft:instant_damage", 1, 0);
                        }
                        let start = m.random.clone();
                        let mut events = Vec::new();
                        tick_effects_hurting(
                            &mut m,
                            w.loop_context,
                            &mut crate::health::TestLevel::new(0).level(),
                            w.mod_effects,
                            &mut |m| hurt_teleports(m, w, &mut events),
                        );
                        assert!(!m.alive(), "seed {seed}: Wither kills it");
                        assert_eq!(
                            m.position,
                            Vec3::new(0.5, 64.0, 0.5),
                            "seed {seed}: no teleport"
                        );
                        steps(&start, &m.random)
                    };
                    assert_eq!(run(true), run(false) + 1, "seed {seed}: one more draw");
                }
            },
        ),
        "vanilla data is missing"
    );
}

/// Contained water is not a neighbouring WATER node when its solid block
/// classifies as BLOCKED. Open water still contributes WATER_BORDER.
#[test]
#[ignore = "daily"]
fn a_solid_waterlogged_neighbor_does_not_penalize_a_dry_walk_node() {
    for waterlogged in [true, false] {
        assert!(
            with_built_test_world(
                &[],
                move |world, registry| {
                    build("minecraft:stone", "minecraft:air")(world, registry);
                    let neighbor = if waterlogged {
                        let default = registry.default_state("minecraft:oak_slab").unwrap();
                        let mut slab = registry.state(default).unwrap().clone();
                        slab.properties.insert("type".into(), "top".into());
                        slab.properties.insert("waterlogged".into(), "true".into());
                        registry.id(&slab).unwrap()
                    } else {
                        registry.default_state("minecraft:water").unwrap()
                    };
                    world.set(1, 63, 0, neighbor).unwrap();
                },
                |w, _| {
                    let expected = if waterlogged {
                        super::super::path::PathType::Walkable
                    } else {
                        super::super::path::PathType::WaterBorder
                    };
                    assert_eq!(w.path_type_static(0, 64, 0), expected);
                },
            ),
            "vanilla data is missing"
        );
    }
}

/// #1697: `Enderman.hurtServer` teleports inside every hurt. The freezing hurt
/// (`LivingEntity.aiStep`) and cramming (`pushEntities`) each settle it at
/// once: the enderman lands,
/// and draws, as the hurt dealt with its teleport.
#[test]
#[ignore = "daily"]
fn freezing_and_cramming_each_teleport_inside_the_hurt() {
    use crate::mobs::Seen;
    use sunrise_core::random::RandomSource as _;
    let world = |world: &mut crate::world::World, registry: &BlockRegistry| {
        build("minecraft:stone", "minecraft:powder_snow")(world, registry);
        let air = registry.default_state("minecraft:air").unwrap();
        world.set(8, 64, 8, air).unwrap();
    };
    assert!(
        with_built_contact_test_world(world, |w, types, _generic| {
            let mut landed = [0; 2];
            for seed in 0..40 {
                // Freezing: fully frozen in powder snow, on the 40-tick beat.
                let mut m = enderman(types, seed);
                m.ticks_frozen = crate::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE;
                m.age = 40;
                let mut expected = m.clone();
                hurt(
                    &mut expected,
                    w,
                    &mut Vec::new(),
                    1.0,
                    Damage::of(DamageKind::Freeze),
                );
                // The block walk's FREEZE effect for the snow it stands in.
                m.in_powder_snow = true;
                freeze_tick(
                    &mut m,
                    w,
                    &mut crate::health::TestLevel::new(0).level(),
                    &mut Vec::new(),
                );
                assert_eq!(m.position, expected.position, "freezing, seed {seed}");
                assert_eq!(
                    m.random.state(),
                    expected.random.state(),
                    "freezing, seed {seed}"
                );
                landed[0] += usize::from(m.position != Vec3::new(0.5, 64.0, 0.5));

                // Cramming: three pushable pigs around it, `maxEntityCramming` 3.
                let pig = |id: u32, x: f64| {
                    let kind = std::sync::Arc::new(types.types["minecraft:pig"].clone());
                    let mut p = Mob::new(id, kind, Vec3::new(x, 64.0, 8.5), 0.0);
                    p.no_ai = true;
                    p.no_gravity = true;
                    p
                };
                let crowd = [pig(11, 8.6), pig(12, 8.7), pig(13, 8.8)];
                let seen: Vec<Seen> = crowd
                    .iter()
                    .map(|o| o.seen(None, &crate::items::Items::empty()))
                    .collect();
                let crowded = MobWorld {
                    side: crate::motion::dry_side(),
                    others: &seen,
                    max_entity_cramming: 3,
                    // The crowd is read from `others`, not the world's empty index.
                    others_grid: None,
                    ..w.clone()
                };
                // Spread seeds: small consecutive ones share their first `nextInt(4)`.
                let mut m = enderman(types, seed * 104_729 + 1_234_567);
                m.position = Vec3::new(8.5, 64.0, 8.5);
                let mut expected = m.clone();
                // `nextInt(4) == 0`: the quarter that hurts.
                if expected.random.next_int_bound(4).unwrap() != 0 {
                    continue;
                }
                hurt(
                    &mut expected,
                    &crowded,
                    &mut Vec::new(),
                    6.0,
                    Damage::of(DamageKind::Cramming),
                );
                push_entities(
                    &mut m,
                    &crowded,
                    &mut crate::health::TestLevel::new(0).level(),
                    &mut Vec::new(),
                );
                assert_eq!(
                    m.last_damage.map(|(d, _)| d.kind),
                    Some(DamageKind::Cramming)
                );
                assert_eq!(m.position, expected.position, "cramming, seed {seed}");
                assert_eq!(
                    m.random.state(),
                    expected.random.state(),
                    "cramming, seed {seed}"
                );
                landed[1] += usize::from(m.position != Vec3::new(8.5, 64.0, 8.5));
            }
            assert!(
                landed.iter().all(|n| *n > 0),
                "each hurt lands a teleport: {landed:?}"
            );
        }),
        "vanilla data is missing"
    );
}

/// #1697: two hurts in one tick each teleport inside their own hurt, in the
/// order Sunrise deals them, as the two hurts dealt one after the other:
/// cramming then the water's hurt (`LivingEntity.aiStep` order), a mod part's
/// queued hurt then freezing (`applyEffectsFromBlocks`, then the freeze hurt).
/// Without each settle, the two marks would merge into one try.
#[test]
#[ignore = "daily"]
fn two_hurts_in_a_tick_each_teleport_inside_their_own_hurt() {
    use crate::mobs::Seen;
    use sunrise_core::random::RandomSource as _;
    let world = |world: &mut crate::world::World, registry: &BlockRegistry| {
        build("minecraft:stone", "minecraft:powder_snow")(world, registry);
        let air = registry.default_state("minecraft:air").unwrap();
        world.set(8, 64, 8, air).unwrap();
    };
    assert!(
        with_built_contact_test_world(world, |w, types, _generic| {
            let pig = |id: u32, x: f64| {
                let kind = std::sync::Arc::new(types.types["minecraft:pig"].clone());
                let mut p = Mob::new(id, kind, Vec3::new(x, 64.0, 8.5), 0.0);
                p.no_ai = true;
                p.no_gravity = true;
                p
            };
            let crowd = [pig(11, 8.6), pig(12, 8.7), pig(13, 8.8)];
            let seen: Vec<Seen> = crowd
                .iter()
                .map(|o| o.seen(None, &crate::items::Items::empty()))
                .collect();
            let crowded = MobWorld {
                side: crate::motion::dry_side(),
                others: &seen,
                max_entity_cramming: 3,
                others_grid: None,
                ..w.clone()
            };
            let mut landed = [0; 2];
            for seed in 0..40 {
                // Cramming, then the water's hurt, in a crowd and in water.
                let mut m = enderman(types, seed * 104_729 + 1_234_567);
                m.position = Vec3::new(8.5, 64.0, 8.5);
                m.fluids.in_water = true;
                let mut expected = m.clone();
                if expected.random.next_int_bound(4).unwrap() == 0 {
                    let kinds = [(6.0, DamageKind::Cramming), (1.0, DamageKind::Drown)];
                    for (amount, kind) in kinds {
                        hurt(
                            &mut expected,
                            &crowded,
                            &mut Vec::new(),
                            amount,
                            Damage::of(kind),
                        );
                    }
                    let mut level = crate::health::TestLevel::new(0);
                    let mut events = Vec::new();
                    push_entities(&mut m, &crowded, &mut level.level(), &mut events);
                    hurt_by_water(&mut m, &crowded, &mut level.level(), &mut events);
                    assert_eq!(
                        m.position, expected.position,
                        "cramming and water, seed {seed}"
                    );
                    assert_eq!(
                        m.random.state(),
                        expected.random.state(),
                        "cramming and water, seed {seed}"
                    );
                    landed[0] += usize::from(m.position != Vec3::new(8.5, 64.0, 8.5));
                }

                // A mod part's queued hurt, then freezing in powder snow.
                let mut m = enderman(types, seed);
                m.ticks_frozen = crate::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE;
                m.age = 40;
                let mut expected = m.clone();
                for (amount, kind) in [(2.0, DamageKind::Generic), (1.0, DamageKind::Freeze)] {
                    hurt(&mut expected, w, &mut Vec::new(), amount, Damage::of(kind));
                }
                // `isInPowderSnow`, as the tick's block walk sets it before the hurts.
                m.in_powder_snow = true;
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                {
                    let mut level_guard = level.level();
                    let mut effects = mod_api::InsideEffects::default();
                    effects.advance_step(0);
                    effects.run_after(mod_api::InsideEffect::Freeze, |view| {
                        view.hurt("minecraft:generic", 2.0);
                    });
                    let mut access = MobInsideAccess {
                        m: &mut m,
                        w,
                        level: &mut level_guard,
                        events: &mut events,
                    };
                    effects.apply_and_clear(&mut access);
                }
                freeze_tick(&mut m, w, &mut level.level(), &mut events);
                assert_eq!(
                    m.position, expected.position,
                    "mod hurt and freezing, seed {seed}"
                );
                assert_eq!(
                    m.random.state(),
                    expected.random.state(),
                    "mod hurt and freezing, seed {seed}"
                );
                landed[1] += usize::from(m.position != Vec3::new(0.5, 64.0, 0.5));
            }
            assert!(landed.iter().all(|n| *n > 0), "both pairs land: {landed:?}");
        }),
        "vanilla data is missing"
    );
}

/// #1697: a cactus's hurt (`CactusBlock.entityInside`, in the block walk)
/// teleports inside the hurt, as magma's does; and a shulker crammed below half
/// health makes `Shulker.hurtServer`'s low-health teleport inside the hurt.
#[test]
#[ignore = "daily"]
fn a_cactus_hurt_and_a_crammed_shulker_teleport_inside_the_hurt() {
    use crate::mobs::Seen;
    use sunrise_core::random::RandomSource as _;
    assert!(
        with_built_test_world(
            &[],
            build("minecraft:sand", "minecraft:cactus"),
            |w, types| {
                let mut landed = [0; 2];
                for seed in 0..40 {
                    let mut m = enderman(types, seed);
                    let mut expected = m.clone();
                    hurt(
                        &mut expected,
                        w,
                        &mut Vec::new(),
                        1.0,
                        Damage::of(DamageKind::Cactus),
                    );
                    let mut events = Vec::new();
                    effects_from_blocks(
                        &mut m,
                        w,
                        &mut crate::health::TestLevel::new(0).level(),
                        &mut events,
                    );
                    assert_eq!(m.last_damage.map(|(d, _)| d.kind), Some(DamageKind::Cactus));
                    assert_eq!(m.position, expected.position, "cactus, seed {seed}");
                    assert_eq!(
                        m.random.state(),
                        expected.random.state(),
                        "cactus, seed {seed}"
                    );
                    landed[0] += usize::from(m.position != Vec3::new(0.5, 64.0, 0.5));

                    let shulker = |id: u32| {
                        let kind = std::sync::Arc::new(types.types["minecraft:shulker"].clone());
                        let mut s = Mob::new(id, kind, Vec3::new(8.5, 64.0, 8.5), 0.0);
                        register(&mut s);
                        s.random = sunrise_core::random::LegacyRandom::new(seed * 104_729 + 7);
                        s.health = 10.0;
                        s
                    };
                    let pig = |id: u32, x: f64| {
                        let kind = std::sync::Arc::new(types.types["minecraft:pig"].clone());
                        let mut p = Mob::new(id, kind, Vec3::new(x, 64.0, 8.5), 0.0);
                        p.no_ai = true;
                        p.no_gravity = true;
                        p
                    };
                    let crowd = [pig(11, 8.6), pig(12, 8.7), pig(13, 8.8)];
                    let seen: Vec<Seen> = crowd
                        .iter()
                        .map(|o| o.seen(None, &crate::items::Items::empty()))
                        .collect();
                    let crowded = MobWorld {
                        side: crate::motion::dry_side(),
                        others: &seen,
                        max_entity_cramming: 3,
                        others_grid: None,
                        ..w.clone()
                    };
                    let mut s = shulker(1);
                    let mut expected = s.clone();
                    if expected.random.next_int_bound(4).unwrap() != 0 {
                        continue;
                    }
                    hurt(
                        &mut expected,
                        &crowded,
                        &mut Vec::new(),
                        6.0,
                        Damage::of(DamageKind::Cramming),
                    );
                    push_entities(
                        &mut s,
                        &crowded,
                        &mut crate::health::TestLevel::new(0).level(),
                        &mut Vec::new(),
                    );
                    assert_eq!(
                        s.last_damage.map(|(d, _)| d.kind),
                        Some(DamageKind::Cramming)
                    );
                    assert_eq!(s.position, expected.position, "shulker, seed {seed}");
                    assert_eq!(
                        s.random.state(),
                        expected.random.state(),
                        "shulker, seed {seed}"
                    );
                    landed[1] += usize::from(s.position != Vec3::new(8.5, 64.0, 8.5));
                }
                assert!(landed.iter().all(|n| *n > 0), "both land: {landed:?}");
            },
        ),
        "vanilla data is missing"
    );
}

/// The state `name` with `set` over its default properties.
fn state_with(
    registry: &BlockRegistry,
    name: &str,
    set: &[(&str, &str)],
) -> sunrise_world::registry::BlockStateId {
    let base = registry.default_state(name).unwrap();
    let mut properties: std::collections::BTreeMap<String, String> = registry
        .state(base)
        .unwrap()
        .properties
        .clone()
        .into_iter()
        .collect();
    for (k, v) in set {
        properties.insert((*k).to_owned(), (*v).to_owned());
    }
    registry
        .resolve(&sunrise_world::chunk::BlockState {
            name: name.to_owned(),
            properties: properties.into_iter().collect(),
        })
        .unwrap()
}

/// #1697: a lit campfire's and a grown sweet berry bush's hurts
/// (`CampfireBlock.entityInside`, `SweetBerryBushBlock.entityInside`, in the
/// block walk) teleport inside the hurt, as the cactus's and magma's do.
#[test]
#[ignore = "daily"]
fn a_campfire_and_a_berry_bush_hurt_teleport_inside_the_hurt() {
    for (block, set, kind, amount) in [
        (
            "minecraft:campfire",
            &[("lit", "true")][..],
            DamageKind::Campfire,
            1.0,
        ),
        (
            "minecraft:sweet_berry_bush",
            &[("age", "1")][..],
            DamageKind::SweetBerryBush,
            1.0,
        ),
    ] {
        let world = move |world: &mut crate::world::World, registry: &BlockRegistry| {
            build("minecraft:grass_block", "minecraft:air")(world, registry);
            world
                .set(0, 64, 0, state_with(registry, block, set))
                .unwrap();
        };
        assert!(
            with_built_test_world(&[], world, |w, types| {
                let mut landed = 0;
                for seed in 0..40 {
                    let mut m = enderman(types, seed);
                    // A bush hurts a mob that moved this tick (`getKnownMovement`).
                    m.previous = Vec3::new(0.49, 64.0, 0.5);
                    let mut expected = m.clone();
                    hurt(&mut expected, w, &mut Vec::new(), amount, Damage::of(kind));
                    let mut events = Vec::new();
                    effects_from_blocks(
                        &mut m,
                        w,
                        &mut crate::health::TestLevel::new(0).level(),
                        &mut events,
                    );
                    assert_eq!(m.last_damage.map(|(d, _)| d.kind), Some(kind), "{block}");
                    assert_eq!(m.position, expected.position, "{block}, seed {seed}");
                    assert_eq!(
                        m.random.state(),
                        expected.random.state(),
                        "{block}, seed {seed}"
                    );
                    landed += usize::from(m.position != Vec3::new(0.5, 64.0, 0.5));
                }
                assert!(landed > 0, "{block}: some seed lands");
            }),
            "vanilla data is missing"
        );
    }
}

/// #1697, through the block walk and freezing (`walk_and_freeze`): an enderman
/// fully frozen on the 40-tick beat whose box takes in a cactus and powder snow
/// is hurt by the cactus during the block walk, which may teleport it out of
/// the snow. `isInPowderSnow`, set by
/// the walk's FREEZE effect, still holds: it stays fully frozen, and the
/// freeze hurt (refused by the cooldown) still draws its teleport chance, as
/// the two hurts dealt one after the other.
#[test]
#[ignore = "daily"]
fn the_walks_powder_snow_outlasts_a_teleport_out_of_it() {
    let world = |world: &mut crate::world::World, registry: &BlockRegistry| {
        build("minecraft:sand", "minecraft:cactus")(world, registry);
        let snow = registry.default_state("minecraft:powder_snow").unwrap();
        world.set(1, 64, 0, snow).unwrap();
    };
    assert!(
        with_built_test_world(&[], world, |w, types| {
            let mut moved = 0;
            for seed in 0..40 {
                let mut m = enderman(types, seed);
                // Its box (0.6 wide) spans the cactus at x 0 and the snow at x 1.
                m.position = Vec3::new(1.0, 64.0, 0.5);
                m.previous = m.position;
                m.ticks_frozen = crate::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE;
                m.age = 40;
                let start = m.random.clone();
                let mut expected = m.clone();
                for kind in [DamageKind::Cactus, DamageKind::Freeze] {
                    hurt(&mut expected, w, &mut Vec::new(), 1.0, Damage::of(kind));
                }
                walk_and_freeze(
                    &mut m,
                    w,
                    &mut crate::health::TestLevel::new(0).level(),
                    &mut Vec::new(),
                );
                assert_eq!(m.last_damage.map(|(d, _)| d.kind), Some(DamageKind::Cactus));
                assert_eq!(
                    m.ticks_frozen,
                    crate::motion::PlayerMotion::TICKS_REQUIRED_TO_FREEZE,
                    "seed {seed}: still in the walk's powder snow"
                );
                assert_eq!(m.position, expected.position, "seed {seed}");
                assert_eq!(
                    steps(&start, &m.random),
                    steps(&start, &expected.random),
                    "seed {seed}: both hurts' draws"
                );
                moved += usize::from(m.position != Vec3::new(1.0, 64.0, 0.5));
            }
            assert!(moved > 0, "some seed teleports out of the snow");
        }),
        "vanilla data is missing"
    );
}

/// `Entity.baseTick` clears `isInPowderSnow` (kept as `wasInPowderSnow`)
/// before the block walk: a mob that was in powder snow last tick and stands
/// on stone now thaws by 2.
#[test]
#[ignore = "daily"]
fn powder_snow_from_last_tick_does_not_carry_over() {
    assert!(
        with_built_test_world(
            &[],
            build("minecraft:stone", "minecraft:air"),
            |w, types| {
                let mut m = enderman(types, 0);
                m.previous = m.position;
                m.in_powder_snow = true;
                m.ticks_frozen = 100;
                let mut level = crate::health::TestLevel::new(0);
                base_tick(&mut m, w, &mut level.level(), &mut Vec::new());
                assert!(m.was_in_powder_snow, "kept as wasInPowderSnow");
                walk_and_freeze(&mut m, w, &mut level.level(), &mut Vec::new());
                assert!(!m.in_powder_snow);
                assert_eq!(m.ticks_frozen, 98);
            },
        ),
        "vanilla data is missing"
    );
}

/// For every mob, not only an enderman: `isInPowderSnow` follows the tick's
/// path, as the block walk's FREEZE effect sets it. A zombie whose move this
/// tick starts in powder snow and ends on stone is in powder snow this tick,
/// so it freezes by one, where reading where it ended would thaw it.
#[test]
#[ignore = "daily"]
fn a_move_through_powder_snow_freezes_a_zombie_that_ends_outside_it() {
    let world = |world: &mut crate::world::World, registry: &BlockRegistry| {
        build("minecraft:stone", "minecraft:air")(world, registry);
        let snow = registry.default_state("minecraft:powder_snow").unwrap();
        world.set(1, 64, 0, snow).unwrap();
    };
    assert!(
        with_built_test_world(&[], world, |w, types| {
            let kind = Arc::new(types.types["minecraft:zombie"].clone());
            let mut m = Mob::new(5, kind, Vec3::new(3.5, 64.0, 0.5), 0.0);
            m.previous = Vec3::new(1.5, 64.0, 0.5);
            m.no_gravity = true;
            walk_and_freeze(
                &mut m,
                w,
                &mut crate::health::TestLevel::new(0).level(),
                &mut Vec::new(),
            );
            assert!(m.in_powder_snow, "the move crossed powder snow");
            assert_eq!(m.ticks_frozen, 1);
        }),
        "vanilla data is missing"
    );
}
