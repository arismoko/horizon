//! Real rocket tick coverage for impact payloads and canceled explosions.
use crate::game::server_side::native_projectiles::impact_probe::{Action, point, with_probe};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
use sunrise_gameplay_common::game::fireworks::Rocket;
use sunrise_items::firework::FireworkExplosion;
use sunrise_sim::mobs::Mob;
use sunrise_sim::mobs::Who;

pub(crate) fn launch(game: &mut sunrise_gameplay::game::ServerGame<'_>, lifetime: i32) {
    let mut r = Rocket::remote(71, Vec3::new(0.5, 64.5, 0.5), Vec3::new(4.0, 0.0, 0.0));
    r.angled = true;
    r.owner = Some(Who::Player);
    r.left_owner = true;
    r.lifetime = lifetime;
    std::sync::Arc::make_mut(&mut r.explosions).push(FireworkExplosion::default());
    game.rockets.push(r);
}

#[test]
#[ignore = "daily"]
fn rocket_hits_report_geometry_and_cancellation_skips_the_explosion() {
    for entity in [false, true] {
        for action in [Action::Allow, Action::Cancel] {
            with_probe(action, true, |game, observed| {
                if entity {
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    game.mobs
                        .push(Mob::new(72, kind, Vec3::new(3.0, 64.0, 0.5), 0.0));
                } else {
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(2, 64, 0, stone).unwrap();
                }
                launch(game, 100);
                game.tick_rockets();
                let hits = observed.hits.lock().unwrap();
                assert_eq!(hits.len(), 1);
                let hit = &hits[0];
                assert_eq!(hit.projectile, 71);
                assert_eq!(hit.kind, "minecraft:firework_rocket");
                assert_eq!(hit.owner, 7);
                assert_eq!(hit.dimension, "minecraft:overworld");
                if entity {
                    assert_eq!(hit.entity, Some(72));
                    assert_eq!(hit.entity_kind, "minecraft:pig");
                    point(hit.at, [2.55, 64.5, 0.5]);
                } else {
                    assert_eq!(hit.entity, None);
                    assert_eq!(hit.block, [2, 64, 0]);
                    assert_eq!(hit.face, [-1, 0, 0]);
                    point(hit.at, [2.0, 64.5, 0.5]);
                }
                if matches!(action, Action::Cancel) {
                    assert_eq!(game.rockets.len(), 1);
                    assert!(game.entity(71).is_some());
                    assert_eq!(game.rockets[0].life, 1);
                    if entity {
                        assert_eq!(game.mobs[0].health, 10.0);
                    }
                } else {
                    assert!(game.rockets.is_empty());
                    if entity {
                        assert!(game.mobs[0].health < 10.0);
                    }
                }
            });
        }
    }
}

#[test]
#[ignore = "daily"]
fn rockets_skip_misses_deflections_and_a_closed_gate_but_keep_lifetime_expiry() {
    for case in 0..5 {
        let action = if case == 4 {
            Action::Allow
        } else {
            Action::Cancel
        };
        with_probe(action, case != 2, |game, observed| {
            if case == 1 {
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:breeze")
                    .unwrap()
                    .clone();
                game.mobs
                    .push(Mob::new(72, kind, Vec3::new(3.0, 64.0, 0.5), 0.0));
            } else if case >= 2 {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(2, 64, 0, stone).unwrap();
            }
            launch(game, if case >= 3 { 0 } else { 100 });
            if case == 4 {
                // A plain rocket still reports its block hit, but does not
                // explode on that hit even when it is allowed.
                game.rockets[0].explosions = Default::default();
                game.rockets[0].lifetime = 100;
            }
            game.tick_rockets();
            assert_eq!(observed.hits.lock().unwrap().len(), usize::from(case >= 3));
            match case {
                1 => {
                    assert_eq!(game.rockets.len(), 1);
                    assert_eq!(game.rockets[0].velocity.x, -2.0);
                }
                2 | 3 => assert!(game.rockets.is_empty()),
                _ => assert_eq!(game.rockets.len(), 1),
            }
        });
    }
}

/// Disabling the last `projectile_impact` subscriber during one rocket's impact stops a second
/// hitting rocket in the same `tick_rockets` pass from preparing or dispatching an impact.
#[test]
#[ignore = "daily"]
fn rocket_impact_rechecks_subscriptions_per_impact_moment() {
    use mod_api::{Flow, Hooks, ModEntry, ProjectileImpactEvent, ServerHost};
    use std::sync::{Arc, Mutex};
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay::game::ROCKET_IMPACT_PREPARES;
    use sunrise_gameplay::game::native_entity_refs::tests::{
        bind_local_player, run_fresh_cases, with_game,
    };
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::motion::PlayerMotion;

    if run_fresh_cases(
        "cut_layout::game::fireworks::impact::tests::rocket_impact_rechecks_subscriptions_per_impact_moment",
        "SUNRISE_ROCKET_IMPACT_SUBSCRIPTION_CASE",
        &["disable"],
    ) {
        return;
    }

    struct DisableOnFirst(Arc<Mutex<Vec<u32>>>);
    #[mod_api::__subscriptions]
    impl Hooks for DisableOnFirst {
        fn projectile_impact(
            &self,
            _: &mut dyn ServerHost,
            event: &mut ProjectileImpactEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push(event.projectile);
            assert!(mod_api::__private::set_mod_enabled(
                "disable_on_first_rocket",
                false
            ));
            Flow::Continue
        }
    }

    let seen: Arc<Mutex<Vec<u32>>> = Arc::default();
    let mods: mod_api::Mods = vec![ModEntry::new(
        "disable_on_first_rocket",
        Box::new(DisableOnFirst(seen.clone())),
    )]
    .into();
    let mut prepares = 0;
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("forceload add 0 0"));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 8.5)).unwrap());
            bind_local_player(game, 7, 7);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(2, 64, 0, stone).unwrap();
            for id in [71u32, 72] {
                let mut r = Rocket::remote(id, Vec3::new(0.5, 64.5, 0.5), Vec3::new(4.0, 0.0, 0.0));
                r.angled = true;
                r.owner = Some(Who::Player);
                r.left_owner = true;
                r.lifetime = 100;
                std::sync::Arc::make_mut(&mut r.explosions).push(FireworkExplosion::default());
                game.rockets.push(r);
            }
            ROCKET_IMPACT_PREPARES.with(|count| count.set(0));
            game.tick_rockets();
            prepares = ROCKET_IMPACT_PREPARES.with(std::cell::Cell::get);
            assert!(game.rockets.is_empty());
        });
    });
    assert_eq!(*seen.lock().unwrap(), [71]);
    assert_eq!(
        prepares, 1,
        "second rocket checked native_hears_impacts at its own impact moment and skipped preparation"
    );
}

#[test]
#[ignore = "daily"]
fn a_crowd_of_rockets_ticks_in_place_keeping_order_and_identity() {
    with_probe(Action::Allow, true, |game, _| {
        for id in 0..6 {
            let mut r = Rocket::remote(
                100 + id,
                Vec3::new(0.5 + f64::from(id) * 4.0, 90.5, 0.5),
                Vec3::new(0.0, 0.2, 0.0),
            );
            r.angled = true;
            r.left_owner = true;
            r.lifetime = 100;
            game.rockets.push(r);
        }
        let joins: Vec<_> = game.rockets.iter().map(|r| (r.id, r.join)).collect();
        for tick in 1..=3 {
            game.tick_rockets();
            // Continuing flights keep every resident's identity and order.
            assert_eq!(
                game.rockets
                    .iter()
                    .map(|r| (r.id, r.join))
                    .collect::<Vec<_>>(),
                joins
            );
            assert!(game.rockets.iter().all(|r| r.life == tick));
            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
        }
        // Expired rockets permanently leave the list, the others stay put.
        game.rockets[2].lifetime = 0;
        game.tick_rockets();
        assert_eq!(game.rockets.len(), 5);
        assert!(game.rockets.iter().all(|r| r.id != 102));
        assert_eq!(
            game.rockets.iter().map(|r| r.id).collect::<Vec<_>>(),
            [100, 101, 103, 104, 105]
        );
    });
}

#[test]
#[ignore = "daily"]
fn rocket_section_listener_reads_the_advanced_position() {
    with_probe(Action::ReadSectionPosition, false, |game, observed| {
        launch(game, 100);
        game.rockets[0].position = Vec3::new(0.5, 79.5, 0.5);
        game.rockets[0].velocity = Vec3::new(0.0, 1.0, 0.0);
        game.tick_rockets();

        let pos = game.rockets[0].position;
        assert!(pos.y >= 80.0);
        assert_eq!(
            *observed.section_positions.lock().unwrap(),
            [Some([pos.x, pos.y, pos.z])]
        );
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
