#![cfg(test)]
use core::ops::ControlFlow;
use mod_api::DimensionKey;
use mod_api::LevelReadError;
use mod_api::LevelRef;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_living_body::NativeLivingBodyGameExt6 as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::position::ChunkPos;
    use sunrise_sim::{join_seq::Joined, mobs::Mob};
    use {
        sunrise_gameplay::game::native_entity_refs::tests::collect_entities_in,
        sunrise_gameplay::game::native_entity_refs::tests::collect_game_all_entities,
        sunrise_gameplay::game::native_entity_refs::tests::collect_game_entities_in,
        sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood,
        sunrise_gameplay::game::native_entity_refs::tests::with_resident_game,
        sunrise_gameplay_common::game::dragon_common::Dragon,
    };

    #[test]
    #[ignore = "daily"]
    fn entity_uuid_lookup_includes_the_dragon_and_excludes_its_parts() {
        use mod_api::{EntityUuid, ServerHost};
        use sunrise_gameplay_common::game::creatures::SpawnSite;
        use sunrise_sim::lan::playerdata::uuid_ints;

        with_resident_game(|game| {
            let id = game
                .spawn_dragon_spawned([0.0, 80.0, 0.0], SpawnSite::Plain)
                .unwrap();
            game.persist.uuids.insert(id, [19, 23, 29, 31]);
            let dragon = game.entity(id).unwrap();
            let part = game.entity(id + 1).unwrap();
            let part_uuid = part.uuid(game).expect("dragon parts have UUIDs");
            // A part can also have saved UUID metadata when it holds attachments.
            game.persist.uuids.insert(part.id(), uuid_ints(part_uuid));

            let level = game.native_current_level().unwrap();
            let host: &dyn ServerHost = game;
            let dragon_uuid = EntityUuid(0x0000_0013_0000_0017_0000_001d_0000_001f);
            let part_uuid = EntityUuid(part_uuid);
            assert_eq!(level.entity_by_uuid(host, dragon_uuid), Some(dragon));
            assert_eq!(level.entity_by_uuid(host, part_uuid), None);

            #[cfg(not(any(feature = "system-allocator", feature = "alloc-census")))]
            {
                let (found, allocations) = sunrise_threads::allocation_count::measure(|| {
                    (
                        level.entity_by_uuid(host, dragon_uuid),
                        level.entity_by_uuid(host, part_uuid),
                    )
                });
                assert_eq!(found, (Some(dragon), None));
                assert_eq!(allocations, 0);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn all_entities_uses_lookup_order_without_bounds_or_parts() {
        with_resident_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(
                level.visit_all_entities(game, &mut |_| panic!("empty level invoked the visitor")),
                Ok(ControlFlow::Continue(()))
            );
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let distant_x = 1_000_000;
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(distant_x >> 4, 0));
            let distant_position = Vec3::new(f64::from(distant_x), 64.0, 0.0);
            game.mobs
                .push(Mob::new(90, kind.clone(), distant_position, 0.0));
            let mut dragon = Dragon::new(Vec3::new(0.0, 80.0, 0.0), 0.0, 0);
            dragon.id = 20;
            game.dragon.replace(dragon);
            game.mobs.push(Mob::new(3, kind, Vec3::ZERO, 0.0));
            let ids: Vec<_> = collect_game_all_entities(game, level)
                .unwrap()
                .into_iter()
                .map(|e| e.id())
                .filter(|id| [90, 20, 3].contains(id) || (21..=28).contains(id))
                .collect();
            assert_eq!(ids, vec![90, 20, 3]);

            // Extracting an early member skips it cleanly and visits remaining members in order.
            let first_join = game.mobs[0].join_seq();
            let first = game.mobs.extract(first_join).unwrap();
            let remaining: Vec<_> = collect_game_all_entities(game, level)
                .unwrap()
                .into_iter()
                .map(|e| e.id())
                .filter(|id| [90, 20, 3].contains(id) || (21..=28).contains(id))
                .collect();
            assert_eq!(remaining, vec![20, 3]);
            game.mobs.put_back(0, first);

            // Extracting a later member does not block a lazy Break(()) or the earlier members.
            let join = game.mobs[1].join_seq();
            let extracted = game.mobs.extract(join).unwrap();
            let mut visited = Vec::new();
            assert_eq!(
                level.visit_all_entities(game, &mut |entity| {
                    visited.push(entity.id());
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(visited, vec![90]);
            let remaining: Vec<_> = collect_game_all_entities(game, level)
                .unwrap()
                .into_iter()
                .map(|e| e.id())
                .filter(|id| [90, 20, 3].contains(id) || (21..=28).contains(id))
                .collect();
            assert_eq!(remaining, vec![90, 20]);
            game.mobs.put_back(1, extracted);

            let mut visits = 0;
            let host: &dyn mod_api::Host = game;
            assert_eq!(
                level.visit_all_entities(host, &mut |entity| {
                    visits += 1;
                    assert_eq!(entity.id(), 90);
                    // This read consults residency again, which must not still be borrowed.
                    assert_eq!(
                        entity.position(host),
                        Some([f64::from(distant_x), 64.0, 0.0])
                    );
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(visits, 1);

            assert_eq!(
                collect_game_all_entities(
                    game,
                    LevelRef::new(
                        mod_api::WorldEpoch::new(level.epoch().get() + 1),
                        level.dimension()
                    )
                ),
                Err(LevelReadError::Stale)
            );
            assert_eq!(
                collect_game_all_entities(game, LevelRef::new(level.epoch(), DimensionKey::NETHER)),
                Err(LevelReadError::Unavailable)
            );
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            assert_eq!(
                collect_game_all_entities(game, level),
                Err(LevelReadError::Unavailable)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn ordinary_entities_precede_dragon_parts() {
        with_resident_game(|game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let mut dragon = Dragon::new(Vec3::new(0.0, 80.0, 0.0), 0.0, 0);
            dragon.id = 20;
            game.dragon.replace(dragon);
            let level = game.level(game.dim.id()).unwrap();
            // Parts start at the origin, away from the dragon's own body.
            let ids =
                collect_game_entities_in(game, level, [-10.0, -1.0, -10.0], [10.0, 10.0, 10.0])
                    .unwrap()
                    .into_iter()
                    .map(|e| e.id())
                    .collect::<Vec<_>>();
            assert_eq!(ids.first(), Some(&12));
            // This test pins only the ordinary/parts boundary, not part order.
            let mut parts = ids[1..].to_vec();
            parts.sort_unstable();
            assert_eq!(parts, (21..=28).collect::<Vec<_>>());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn spatial_visit_stops_before_later_matches() {
        with_resident_game(|game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            game.item_frames.push(sunrise_sim::entities::ItemFrame {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 13,
                pos: [0, 0, 0],
                facing: 3,
                glow: false,
                item: None,
                rotation: 0,
                drop_chance: 1.0,
                fixed: false,
                invisible: false,
                custom_name: None,
                random: sunrise_core::random::LegacyRandom::new(0),
                check_interval: 0,
                ticks: 0,
            });
            let mut dragon = Dragon::new(Vec3::new(0.0, 80.0, 0.0), 0.0, 0);
            dragon.id = 20;
            game.dragon.replace(dragon);

            let level = game.level(game.dim.id()).unwrap();
            let host: &dyn mod_api::Host = game;
            let min = [-10.0; 3];
            let max = [10.0; 3];
            let mut calls = 0;
            assert_eq!(
                level.visit_entities_in(host, min, max, &mut |entity| {
                    calls += 1;
                    assert_eq!(entity.id(), 12);
                    assert_eq!(entity.position(host), Some([0.0; 3]));
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(calls, 1);
            let ids: Vec<_> = collect_entities_in(host, level, min, max)
                .unwrap()
                .into_iter()
                .map(|entity| entity.id())
                .collect();
            assert_eq!(ids, [12, 13, 25, 27, 28, 24, 26, 23, 22, 21]);
            assert_eq!(
                level.visit_entities_in(host, [100.0; 3], [110.0; 3], &mut |_| {
                    panic!("empty box invoked the visitor")
                }),
                Ok(ControlFlow::Continue(()))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn nearest_ray_rejects_closer_hits_and_keeps_first_tie() {
        with_resident_game(|game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            // Far first, then a closer rejected hit, then two accepted hits at the same distance.
            // The excluded pig is skipped before `accept`; the last pig encloses the ray's start,
            // so `accept` runs on its `EntityRef` before clipping finds no entry.
            for (id, x) in [
                (90, 9.0),
                (60, 3.0),
                (40, 6.0),
                (20, 6.0),
                (10, 1.5),
                (30, 0.0),
            ] {
                game.mobs
                    .push(Mob::new(id, kind.clone(), Vec3::new(x, 64.0, 0.0), 0.0));
            }
            let level = game.level(game.dim.id()).unwrap();
            let exclude = Some(game.entity(10).unwrap());
            let host: &dyn mod_api::Host = game;
            let from = [0.0, 64.5, 0.0];
            let to = [12.0, 64.5, 0.0];
            let mut seen = Vec::new();
            let nearest = level
                .ray_trace_nearest_entity(host, from, to, 0.25, exclude, &mut |entity| {
                    assert!(entity.position(host).is_some());
                    seen.push(entity.id());
                    entity.id() != 60
                })
                .unwrap();
            assert_eq!(seen, [90, 60, 40, 20, 30]);
            assert_eq!(nearest.map(|hit| hit.entity.id()), Some(40));

            let hits = level
                .ray_trace_entities(host, from, to, 0.25, exclude)
                .unwrap();
            let ordered: Vec<_> = hits.into_iter().map(|hit| hit.entity.id()).collect();
            assert_eq!(ordered, [60, 40, 20, 90]);

            assert_eq!(
                level.ray_trace_nearest_entity(host, from, to, 0.25, exclude, &mut |_| false),
                Ok(None),
            );
            // Negative margins deflate candidate boxes like vanilla's `AABB.inflate`.
            // A pig is 0.9 wide (z in [-0.45, 0.45]): grazing at z = 0.35 hits at 0.0 and
            // misses at -0.2; z = 0.5 misses at 0.0 and hits when -1.0 swaps inverted bounds.
            let grazing_from = [0.0, 64.5, 0.35];
            let grazing_to = [12.0, 64.5, 0.35];
            assert_eq!(
                level
                    .ray_trace_nearest_entity(
                        host,
                        grazing_from,
                        grazing_to,
                        0.0,
                        exclude,
                        &mut |_| true,
                    )
                    .unwrap()
                    .map(|hit| hit.entity.id()),
                Some(60),
            );
            assert_eq!(
                level.ray_trace_nearest_entity(
                    host,
                    grazing_from,
                    grazing_to,
                    -0.2,
                    exclude,
                    &mut |_| true,
                ),
                Ok(None),
            );
            assert_eq!(
                level.ray_trace_entities(host, grazing_from, grazing_to, -0.2, exclude),
                Some(Vec::new()),
            );
            let over_from = [0.0, 64.5, 0.5];
            let over_to = [12.0, 64.5, 0.5];
            assert_eq!(
                level.ray_trace_nearest_entity(host, over_from, over_to, 0.0, exclude, &mut |_| {
                    true
                },),
                Ok(None),
            );
            assert_eq!(
                level
                    .ray_trace_nearest_entity(host, over_from, over_to, -1.0, exclude, &mut |_| {
                        true
                    },)
                    .unwrap()
                    .map(|hit| hit.entity.id()),
                Some(60),
            );

            let empty_from = [0.0, 80.5, 0.0];
            let empty_to = [12.0, 80.5, 0.0];
            assert_eq!(
                level.ray_trace_entities(host, empty_from, empty_to, 0.25, exclude),
                Some(Vec::new()),
            );
            let stale = LevelRef::new(
                mod_api::WorldEpoch::new(level.epoch().get() + 1),
                level.dimension(),
            );
            for (level, margin, error) in [
                (level, f64::NAN, LevelReadError::Unavailable),
                (level, f64::NEG_INFINITY, LevelReadError::Unavailable),
                (stale, 0.25, LevelReadError::Stale),
            ] {
                assert_eq!(
                    level.ray_trace_entities(host, from, to, margin, exclude),
                    None
                );
                assert_eq!(
                    level.ray_trace_nearest_entity(host, from, to, margin, exclude, &mut |_| {
                        panic!("a refused query must not invoke the predicate")
                    }),
                    Err(error),
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn entities_in_includes_dragon_and_separate_parts() {
        with_resident_game(|game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs
                .push(Mob::new(12, kind, Vec3::new(5.0, 64.0, 5.0), 0.0));
            let level = game.level(game.dim.id()).unwrap();
            let query = |game: &sunrise_gameplay::game::ServerGame<'_>, lo, hi| {
                collect_game_entities_in(game, level, lo, hi)
                    .unwrap()
                    .into_iter()
                    .map(|e| e.id())
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                query(game, [-10.0, 60.0, -10.0], [10.0, 100.0, 10.0]),
                vec![12]
            );
            let mut dragon = Dragon::new(Vec3::new(0.0, 80.0, 0.0), 0.0, 0);
            dragon.id = 20;
            game.dragon.replace(dragon);
            assert_eq!(
                query(game, [-10.0, 60.0, -10.0], [10.0, 100.0, 10.0]),
                vec![12, 20]
            );
            assert_eq!(
                query(game, [20.0, 60.0, 20.0], [30.0, 70.0, 30.0]),
                Vec::<u32>::new()
            );
            assert_eq!(query(game, [4.0, 63.0, 4.0], [6.0, 66.0, 6.0]), vec![12]);
            let entity = game.entity(20).unwrap();
            assert_eq!(
                game.living_health(game.living(entity).unwrap()),
                Some(200.0)
            );
            assert_eq!(
                query(game, [-100.0, -10.0, -100.0], [100.0, 200.0, 100.0]),
                vec![12, 20, 25, 27, 28, 24, 26, 23, 22, 21]
            );
            // Vanilla parts begin at the origin until the first dragon tick.
            assert_eq!(
                query(game, [-10.0, -1.0, -10.0], [10.0, 10.0, 10.0]),
                vec![25, 27, 28, 24, 26, 23, 22, 21]
            );
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_client_lists_the_dragon_and_its_parts_in_a_box() {
        if super::super::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_entity_refs::dragon_query::tests::a_client_lists_the_dragon_and_its_parts_in_a_box",
            "SUNRISE_CLIENT_ENTITY_BOX_QUERY_CHILD",
            &["client"],
        ) {
            return;
        }
        use mod_api::Host;
        use sunrise_protocol::connection::Pipe;
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "EntitiesInTest").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            remote.profile = 42;
            let mut local =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            local.entity_id = 7;
            game.player = Some(local);
            game.spawn_entity(15, "minecraft:pig", [2.0, 0.0, 0.0], [0.0; 3]);
            game.pick_section_sync_id(15);
            game.bind_local_player_residency();
            // The server tracks the dragon alone; the client places its parts around it.
            game.spawn_entity(20, "minecraft:ender_dragon", [0.0, 80.0, 0.0], [0.0; 3]);
            // The dragon's own box moves to y 200 (a move whose part placement has not run
            // yet), so a box near y 80 holds only parts.
            game.dragon.as_mut().unwrap().position = Vec3::new(0.0, 200.0, 0.0);
            game.pick_section_sync_id(20);
            let level = game.level("minecraft:overworld").unwrap();
            let ids = |host: &dyn Host, min, max| {
                collect_entities_in(host, level, min, max)
                    .map(|found| found.into_iter().map(|e| e.id()).collect::<Vec<_>>())
            };
            // Order: the pig and the local player in the same section, the dragon in
            // its higher section, then the eight parts in the parts table's order.
            let parts = [25, 27, 28, 24, 26, 23, 22, 21];
            assert_eq!(
                ids(game, [-100.0, -10.0, -100.0], [100.0, 300.0, 100.0]),
                Ok([vec![15, 7, 20], parts.to_vec()].concat())
            );
            // Only the dragon's own 16x8 box: the dragon alone, with no part.
            assert_eq!(
                ids(game, [-10.0, 199.0, -10.0], [10.0, 210.0, 10.0]),
                Ok(vec![20])
            );
            // Only parts: every part is listed and the dragon is not.
            assert_eq!(
                ids(game, [-100.0, 40.0, -100.0], [100.0, 120.0, 100.0]),
                Ok(parts.to_vec())
            );
            // A spectating window leaves only its own player out.
            game.set_local_mode(sunrise_command::args::GameMode::Spectator);
            assert_eq!(
                ids(game, [-100.0, -10.0, -100.0], [100.0, 300.0, 100.0]),
                Ok([vec![15, 20], parts.to_vec()].concat())
            );
        });
    }
}
