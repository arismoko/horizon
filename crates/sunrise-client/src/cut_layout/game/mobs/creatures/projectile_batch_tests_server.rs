use crate::game::creatures::projectile_batch_tests::WindContinuationObserved;
use crate::game::creatures::projectile_batch_tests::WindContinuationObserver;
use crate::game::creatures::projectile_batch_tests::assert_boat_matches_witness;
use crate::game::creatures::projectile_batch_tests::assert_vec3_bits;
use crate::game::creatures::projectile_batch_tests::load_wind_continuation_witness_26_3;
use crate::game::creatures::projectile_batch_tests::wind_continuation_witness_26_3;
use std::sync::{Arc, atomic::Ordering};
use sunrise_core::geometry::Vec3;
use sunrise_level::chunk::Chunk;
use sunrise_sim::projectiles::Projectile;
use sunrise_sim::projectiles::ProjectileKind;
use {
    crate::game::server_side::native_projectiles::impact_probe::Action,
    crate::game::server_side::native_projectiles::impact_probe::with_probe,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

#[test]
#[ignore = "daily"]
fn a_first_shot_listener_can_clear_the_eggs_captured_block_before_landing() {
    if run_fresh_cases(
        "cut_layout::game::server_side::creatures_server::projectile_batch_tests::a_first_shot_listener_can_clear_the_eggs_captured_block_before_landing",
        "SUNRISE_PROJECTILE_BATCH_CASE",
        &["clear-on-shoot"],
    ) {
        return;
    }
    with_probe(
        Action::ClearBlockOnShoot([2, 64, 0]),
        true,
        |game, observed| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(2, 64, 0, stone).unwrap();
            let mut egg = Projectile::new(ProjectileKind::Egg, Vec3::new(0.5, 64.5, 0.5), None);
            egg.id = 71;
            egg.velocity = Vec3::new(3.0, 0.0, 0.0);
            game.creatures.projectiles.push(egg);
            let handle = game.entity(71).unwrap();
            let join = game.creatures.projectiles[0].join;
            game.tick_projectiles().unwrap();
            let hits = observed.hits.lock().unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].block, [2, 64, 0]);
            assert_ne!(game.world.block(2, 64, 0), stone);
            assert_eq!(observed.lands.load(Ordering::Relaxed), 0);
            assert!(game.server.lan_entity_events.is_empty());
            assert_eq!(game.entity(71), Some(handle));
            assert_eq!(game.creatures.projectiles[0].join, join);
            assert_eq!(
                game.creatures.projectiles[0].position.components(),
                hits[0].at
            );
        },
    );
}

pub(crate) fn run_continuing_wind_charge_keeps_tick_order_and_skips_relaunch_in_batched_pass() {
    let witness = load_wind_continuation_witness_26_3();
    const CASE: &str = "SUNRISE_PROJECTILE_BATCH_CASE";
    if run_fresh_cases(
        "cut_layout::game::creatures::projectile_batch_tests::continuing_wind_charge_keeps_tick_order_and_skips_relaunch_in_batched_pass",
        CASE,
        &["main", "ctrl4", "reversed", "mixed-batch"],
    ) {
        return;
    }
    let case = std::env::var(CASE).unwrap();
    let observed = Arc::new(WindContinuationObserved::default());
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "wind-continuation-observer",
        Box::new(WindContinuationObserver {
            spawn_on_shoot: case == "mixed-batch",
            observed: observed.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for cz in 0..=2 {
                game.world
                    .chunks
                    .insert((0, cz), Arc::new(Chunk::empty(0, cz)));
                game.block_ticks
                    .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, cz))
                    .unwrap();
            }
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("forceload add 0 0 15 47"));
            game.server.next_entity = 100;
            let (z, breakers, boat_tag, w6_tag, a2_tag) = match case.as_str() {
                "ctrl4" => (24.5, 4_u32, "boat_ctrl4", "w6_ctrl4", None),
                "reversed" => (40.5, 5_u32, "boat_rev", "w6_rev", Some("a2_rev")),
                _ => (8.5, 5_u32, "boat_main", "w6_main", Some("a2_main")),
            };
            game.boats
                .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                    71,
                    Arc::from("oak"),
                    Vec3::new(4.5, -60.0, z),
                    0.0,
                ));
            for i in 0..breakers {
                let mut p =
                    Projectile::new(ProjectileKind::WindCharge, Vec3::new(4.5, -59.2, z), None);
                p.id = 80 + i;
                p.velocity = Vec3::new(0.0, -0.5, 0.0);
                p.acceleration = 0.0;
                game.creatures.projectiles.push(p);
            }
            let make_w6 = || {
                let mut w6 = Projectile::new(
                    ProjectileKind::BreezeWindCharge,
                    Vec3::new(3.5, -59.7, z),
                    None,
                );
                w6.id = 85;
                w6.velocity = Vec3::new(1.5, 0.0, 0.0);
                w6.acceleration = 0.0;
                w6
            };
            let make_a2 = || {
                let mut a2 = Projectile::new(
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    Vec3::new(8.0, -58.6, z),
                    None,
                );
                a2.id = 86;
                a2.velocity = Vec3::new(0.0, -0.2, 0.0);
                a2
            };
            let (w6_join, a2_join) = if case == "reversed" {
                game.creatures.projectiles.push(make_a2());
                let a2_join = game.creatures.projectiles.last().unwrap().join;
                game.creatures.projectiles.push(make_w6());
                let w6_join = game.creatures.projectiles.last().unwrap().join;
                (w6_join, Some(a2_join))
            } else {
                game.creatures.projectiles.push(make_w6());
                let w6_join = game.creatures.projectiles.last().unwrap().join;
                let a2_join = a2_tag.map(|_| {
                    game.creatures.projectiles.push(make_a2());
                    game.creatures.projectiles.last().unwrap().join
                });
                (w6_join, a2_join)
            };
            game.native_joins_skip();
            assert_boat_matches_witness(&witness, game, 0, boat_tag);
            game.server.ticked_entities_through = u32::MAX;
            game.tick_projectiles().unwrap();
            if case != "reversed" {
                assert!(
                    observed.alone_ticks.lock().unwrap().contains(&85),
                    "w6 must fire entity_tick_post on its own tick"
                );
            }
            if case == "ctrl4" {
                assert!(wind_continuation_witness_26_3(&witness, 1, w6_tag).is_none());
                assert!(game.creatures.projectiles.is_empty());
                assert_boat_matches_witness(&witness, game, 1, boat_tag);
                for tick in 2..=4 {
                    game.server.ticked_entities_through = u32::MAX;
                    game.tick_projectiles().unwrap();
                    assert!(wind_continuation_witness_26_3(&witness, tick, w6_tag).is_none());
                    assert!(game.creatures.projectiles.is_empty());
                    assert_boat_matches_witness(&witness, game, tick, boat_tag);
                }
                assert!(observed.launches.lock().unwrap().is_empty());
                return;
            }
            let spawned = (case == "mixed-batch").then(|| {
                assert!(*observed.buffered.lock().unwrap());
                game.creatures
                    .projectiles
                    .iter()
                    .find(|p| p.position.x == 12.5)
                    .expect("nested snowball restored ahead of survivors")
                    .id
            });
            let expected_ids: Vec<u32> = match (case.as_str(), spawned) {
                ("reversed", None) => vec![86, 85],
                (_, Some(id)) => vec![id, 85, 86],
                (_, None) => vec![85, 86],
            };
            let actual_ids: Vec<u32> = game.creatures.projectiles.iter().map(|p| p.id).collect();
            assert_eq!(actual_ids, expected_ids);
            let expected_launches: Vec<u32> = spawned.into_iter().collect();
            assert_eq!(*observed.launches.lock().unwrap(), expected_launches);
            let w6_live = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == 85)
                .unwrap();
            let a2_live = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == 86)
                .unwrap();
            assert_eq!(w6_live.join, w6_join);
            assert_eq!(Some(a2_live.join), a2_join);
            let a2_tag = a2_tag.unwrap();
            let check_tick = |game: &sunrise_gameplay::game::ServerGame<'_>, tick: usize| {
                assert_boat_matches_witness(&witness, game, tick, boat_tag);
                let (w6_pos, w6_mot) =
                    wind_continuation_witness_26_3(&witness, tick, w6_tag).expect("w6 alive");
                let (a2_pos, a2_mot) =
                    wind_continuation_witness_26_3(&witness, tick, a2_tag).expect("a2 alive");
                let w6 = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == 85)
                    .unwrap();
                let a2 = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == 86)
                    .unwrap();
                assert_vec3_bits(w6.position, w6_pos, "w6 pos");
                assert_vec3_bits(w6.velocity, w6_mot, "w6 motion");
                assert_vec3_bits(a2.position, a2_pos, "a2 pos");
                assert_vec3_bits(a2.velocity, a2_mot, "a2 motion");
            };
            check_tick(game, 1);
            for tick in 2..=4 {
                game.server.ticked_entities_through = u32::MAX;
                game.tick_projectiles().unwrap();
                check_tick(game, tick);
            }
            assert_eq!(*observed.launches.lock().unwrap(), expected_launches);
        });
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
