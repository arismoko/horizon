#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use crate::game::server_side::native_entity_refs::tests::collect_entities_in;
    use crate::game::server_side::native_entity_refs::tests::run_fresh_cases;
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use core::ops::ControlFlow;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1;

    #[test]
    #[ignore = "daily"]
    fn client_box_queries_use_live_display_positions_and_section_order() {
        use mod_api::Host;
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;
        use sunrise_protocol::{
            connection::Pipe,
            entity_data::Value,
            packets::play::{self, Clientbound as C},
        };

        const CASE: &str = "SUNRISE_DISPLAY_BOX_QUERY_CHILD";
        if run_fresh_cases(
            "cut_layout::game::client_side::native_entity_refs_client::tests::client_box_queries_use_live_display_positions_and_section_order",
            CASE,
            &["client"],
        ) {
            return;
        }

        const LO: [f64; 3] = [13.0, 65.0, 2.0];
        const HI: [f64; 3] = [19.0, 66.0, 4.0];
        const ADMISSIONS: [(i32, &str, [f64; 3]); 4] = [
            (31, "minecraft:item_display", [17.0, 65.5, 3.0]),
            (92, "minecraft:block_display", [15.5, 65.5, 3.0]),
            (55, "minecraft:pig", [14.5, 65.5, 3.0]),
            (11, "minecraft:text_display", [13.5, 65.5, 3.0]),
        ];
        let chunks = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
            for x in 0..=1 {
                game.world
                    .chunks
                    .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
            }
        };
        let ids = |host: &dyn Host, level, min, max| {
            collect_entities_in(host, level, min, max)
                .unwrap()
                .into_iter()
                .map(|e| e.id())
                .collect::<Vec<_>>()
        };
        match std::env::var(CASE).unwrap().as_str() {
            "client" => with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "DisplayBoxTest").unwrap();
                let remote = game.client.remote.as_mut().unwrap();
                remote.logged_in = true;
                remote.entity_id = 7;
                chunks(game);
                for (id, name, position) in ADMISSIONS {
                    let kind = game
                        .client
                        .remote
                        .as_ref()
                        .unwrap()
                        .builtin
                        .get("minecraft:entity_type")
                        .unwrap()
                        .id(name)
                        .unwrap();
                    game.handle_packet(C::AddEntity(play::AddEntity {
                        id,
                        uuid: id as u128,
                        kind,
                        position,
                        velocity: [0.0; 3],
                        yaw: 0,
                        pitch: 0,
                        head_yaw: 0,
                        data: 0,
                    }))
                    .unwrap();
                }
                let level = game.level("minecraft:overworld").unwrap();
                assert_eq!(ids(game, level, HI, LO), [92, 55, 11, 31]);
                let mut seen = Vec::new();
                assert_eq!(
                    level.visit_entities_in(game, LO, HI, &mut |entity| {
                        seen.push(entity.id());
                        let (_, kind, at) = ADMISSIONS
                            .into_iter()
                            .find(|(id, _, _)| *id as u32 == entity.id())
                            .unwrap();
                        assert_eq!(
                            entity.kind(game).and_then(|k| k.id(game)).as_deref(),
                            Some(kind)
                        );
                        assert_eq!(entity.position(game), Some(at));
                        if entity.id() != 55 {
                            assert_eq!(entity.bounding_box(game), Some([at; 2]));
                        }
                        ControlFlow::Continue(())
                    }),
                    Ok(ControlFlow::Continue(()))
                );
                assert_eq!(seen, [92, 55, 11, 31]);

                let block = game.entity(92).unwrap();
                game.client
                    .text_displays
                    .0
                    .get_mut(&92)
                    .unwrap()
                    .metadata(&[
                        (10, 1, Value::Int(3)),
                        (20, 2, Value::Float(4.0)),
                        (21, 2, Value::Float(4.0)),
                    ]);
                game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
                    id: 92,
                    path: play::PositionPath::Linear([18.5, 65.5, 3.0]),
                    yaw: 0.0,
                    pitch: 0.0,
                    on_ground: false,
                }))
                .unwrap();
                assert_eq!(block.position(game), Some([15.5, 65.5, 3.0]));
                assert_eq!(block.bounding_box(game), Some([[15.5, 65.5, 3.0]; 2]));
                assert_eq!(ids(game, level, LO, HI), [92, 55, 11, 31]);

                game.tick_text_displays(true);
                assert_eq!(ids(game, level, LO, HI), [55, 11, 31, 92]);
                let tight_lo = [16.25, 65.0, 2.5];
                let tight_hi = [16.75, 66.0, 3.5];
                let mut seen = Vec::new();
                let stopped = level.visit_entities_in(game, LO, HI, &mut |entity| {
                    seen.push(entity.id());
                    assert_eq!(ids(game, level, tight_lo, tight_hi), [92]);
                    ControlFlow::Break(())
                });
                assert_eq!(stopped, Ok(ControlFlow::Break(())));
                assert_eq!(seen, [55]);
                assert_eq!(block.position(game), Some([16.5, 65.5, 3.0]));
                assert_eq!(block.bounding_box(game), Some([[16.5, 65.5, 3.0]; 2]));
                assert!(ids(game, level, [16.5, 65.0, 2.5], tight_hi).is_empty());
                let culling_only_lo = [16.5, 67.0, 2.5];
                let culling_only_hi = [16.75, 68.0, 3.5];
                assert!(ids(game, level, culling_only_lo, culling_only_hi).is_empty());
                let (lo, hi) = game.client.text_displays.0[&92].interpolated_bounding_box(1.0);
                let culling = sunrise_gameplay_common::game::pick_entity_common::aabb_extents((
                    lo.to_array(),
                    hi.to_array(),
                ));
                let culling_only = sunrise_gameplay_common::game::pick_entity_common::aabb_extents(
                    (culling_only_lo, culling_only_hi),
                );
                assert!(culling.intersects(culling_only));
                let mut picked = Vec::new();
                game.for_each_pickable_in_bounds(
                    sunrise_gameplay_common::game::pick_entity_common::aabb_extents((LO, HI)),
                    |id, _, _| picked.push(id),
                );
                assert_eq!(picked, [55]);

                game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![92] }))
                    .unwrap();
                assert!(game.entity(92).is_none());
                assert_eq!(block.position(game), None);
                assert_eq!(block.bounding_box(game), None);
                assert_eq!(ids(game, level, LO, HI), [55, 11, 31]);
            }),
            _ => unreachable!("unexpected display query case"),
        }
    }

    /// Crossings in one normal client tick land in `ClientLevel.tickEntities` order whichever
    /// Sunrise loop moved them and whether or not a read-only tick listener splits the displays
    /// off; a tracked player is filed before its `player_tick_post`; a rider snapped with its
    /// display is filed at its seat before it ticks.
    #[test]
    #[ignore = "daily"]
    fn client_crossings_land_in_tick_order_and_before_hooks_read_them() {
        use mod_api::{Hooks, Host, HostSide, ModEntry, PlayerTickPostEvent};
        use std::sync::{Arc, Mutex};
        use sunrise_core::geometry::Vec3;
        use sunrise_level::chunk::Chunk;
        use sunrise_protocol::{
            connection::Pipe,
            entity_data::Value,
            packets::play::{self, Clientbound as C},
        };

        const CASE: &str = "SUNRISE_CLIENT_CROSSING_CHILD";
        if run_fresh_cases(
            "cut_layout::game::client_side::native_entity_refs_client::tests::client_crossings_land_in_tick_order_and_before_hooks_read_them",
            CASE,
            &["order", "order_listened", "tracked_player", "rider"],
        ) {
            return;
        }

        /// Asks for each block display's tick alone, reading nothing; answers the box around
        /// player 18 from its `player_tick_post`.
        struct Probe(Arc<Mutex<Vec<Vec<u32>>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Probe {
            fn player_tick_post(&self, host: HostSide<'_>, event: &mut PlayerTickPostEvent) {
                let HostSide::Client(host) = host else { return };
                if event.player.entity().id() != 18 {
                    return;
                }
                let level = host.level("minecraft:overworld").unwrap();
                let found = collect_entities_in(&*host, level, [30.0, 0.0, -0.5], [31.0, 1.8, 0.5])
                    .unwrap()
                    .into_iter()
                    .map(|e| e.id())
                    .collect();
                self.0.lock().unwrap().push(found);
            }
        }

        let ids = |host: &dyn Host, min, max| {
            let level = host.level("minecraft:overworld").unwrap();
            collect_entities_in(host, level, min, max)
                .unwrap()
                .into_iter()
                .map(|e| e.id())
                .collect::<Vec<_>>()
        };
        let connect = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
            let (client, server) = Pipe::pair();
            std::mem::forget(server);
            game.connect_local(client, "CrossingTest").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.entity_id = 7;
            for x in -1..=4 {
                game.world
                    .chunks
                    .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
            }
        };
        let add = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                   id: i32,
                   name: &str,
                   position: [f64; 3]| {
            let kind = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id(name)
                .unwrap();
            game.handle_packet(C::AddEntity(play::AddEntity {
                id,
                uuid: id as u128,
                kind,
                position,
                velocity: [0.0; 3],
                yaw: 0,
                pitch: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
        };
        let sync =
            |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, id: i32, to: [f64; 3]| {
                game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
                    id,
                    path: play::PositionPath::Linear(to),
                    yaw: 0.0,
                    pitch: 0.0,
                    on_ground: false,
                }))
                .unwrap();
            };
        let heard = Arc::new(Mutex::new(Vec::new()));
        let case = std::env::var(CASE).unwrap();
        let mods = if case == "order" {
            Vec::new()
        } else {
            vec![ModEntry::new("probe", Box::new(Probe(heard.clone())))]
        };
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                connect(game);
                match case.as_str() {
                    "order" | "order_listened" => {
                        // The pig joined first, so vanilla moves it first: both enter section 1
                        // on this tick, the pig ahead of the display the display loop moved.
                        add(game, 70, "minecraft:pig", [14.5, 65.5, 3.0]);
                        add(game, 71, "minecraft:block_display", [13.5, 65.5, 3.0]);
                        game.client
                            .text_displays
                            .0
                            .get_mut(&71)
                            .unwrap()
                            .metadata(&[(10, 1, Value::Int(3))]);
                        sync(game, 70, [20.5, 65.5, 3.0]);
                        sync(game, 71, [22.5, 65.5, 3.0]);
                        game.tick_remote_entities(true);
                        assert_eq!(
                            game.entity(71).unwrap().position(game),
                            Some([16.5, 65.5, 3.0])
                        );
                        assert_eq!(ids(game, [16.0, 65.0, 2.5], [17.0, 66.0, 3.5]), [70, 71]);
                    }
                    "tracked_player" => {
                        let player = game
                            .client
                            .remote
                            .as_ref()
                            .unwrap()
                            .builtin
                            .get("minecraft:entity_type")
                            .unwrap()
                            .id("minecraft:player")
                            .unwrap();
                        game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                            actions: 1,
                            entries: vec![play::PlayerInfo {
                                uuid: 9,
                                name: Some("Player18".into()),
                                ..Default::default()
                            }],
                        }))
                        .unwrap();
                        game.handle_packet(C::AddEntity(play::AddEntity {
                            id: 18,
                            uuid: 9,
                            kind: player,
                            position: [0.5, 0.0, 0.0],
                            velocity: [0.0; 3],
                            pitch: 0,
                            yaw: 0,
                            head_yaw: 0,
                            data: 0,
                        }))
                        .unwrap();
                        // A tracked player's interpolation carries it halfway on this step, to
                        // x 30.5: section 1, beyond the padded range of a box around its new
                        // position.
                        sync(game, 18, [60.5, 0.0, 0.0]);
                        game.tick_remote_entities(true);
                        assert_eq!(*heard.lock().unwrap(), [vec![18]]);
                    }
                    "rider" => {
                        let mut local =
                            sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO)
                                .unwrap();
                        local.entity_id = 7;
                        local.position = Vec3::new(15.5, 65.5, 3.0);
                        game.player = Some(local);
                        game.bind_local_player_residency();
                        add(game, 92, "minecraft:block_display", [15.5, 65.5, 3.0]);
                        game.handle_packet(C::SetPassengers(play::SetPassengers {
                            vehicle: 92,
                            passengers: vec![7],
                        }))
                        .unwrap();
                        let near = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                            ids(game, [39.5, 60.0, 2.0], [41.5, 70.0, 4.0])
                        };
                        assert!(near(game).is_empty());
                        // No teleport duration: the display snaps, and its rider with it.
                        sync(game, 92, [40.5, 65.5, 3.0]);
                        let seat = game.player.as_ref().unwrap().position;
                        use sunrise_sim::join_seq::residency::table::SectionKey;
                        assert_eq!(
                            SectionKey::at(seat.components()),
                            SectionKey::at([40.5, 65.5, 3.0])
                        );
                        assert_eq!(near(game), [92, 7]);
                    }
                    _ => unreachable!("unexpected crossing case"),
                }
            });
        });
    }
}
