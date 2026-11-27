#![cfg(test)]

#[cfg(test)]
mod tests {

    use mod_api::{ClientHost, EntityRef, HostSide, LevelRef};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use sunrise_gameplay_common::game::piston_push::PistonPushGameExt1;
    use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1;
    use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
    use sunrise_protocol::packets::play;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
    };

    /// Writes a velocity and a ground flag before the local player's tick, and records what
    /// the tick left of them.
    struct Boots {
        velocity: [f64; 3],
        saw: Arc<Mutex<Vec<(Vec<bool>, [f64; 3])>>>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Boots {
        fn player_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
            let HostSide::Client(host) = host else {
                return;
            };
            if host.local_player() != Some(event.player) {
                return;
            }
            let wrote = vec![
                host.set_local_player_velocity(self.velocity),
                host.set_local_player_on_ground(true),
                host.set_local_player_velocity([f64::NAN, 0.0, 0.0]),
            ];
            self.saw.lock().unwrap().push((wrote, self.velocity));
        }
    }

    /// A client of a server whose local player is loaded, in the air over an empty chunk.
    /// Returns the server end's reader, what the client sends.
    fn loaded_client(
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    ) -> sunrise_protocol::connection::ConnReader {
        game.options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "writes").unwrap();
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.profile = 456;
        remote.entity_id = 42;
        remote.loaded_sent = true;
        let mut player = sunrise_sim::motion::PlayerMotion::new(
            game.common.motion,
            sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
        )
        .unwrap();
        player.entity_id = remote.entity_id;
        game.player = Some(player);
        game.client.streaming.insert_remote(
            &mut game.common.world,
            sunrise_level::chunk::Chunk::empty(0, 0),
        );
        sunrise_protocol::connection::Connection::local(server)
            .split()
            .0
    }

    #[test]
    #[ignore = "daily"]
    fn client_tick_body_yaw_writes_feed_local_living_render_state() {
        if run_fresh_cases(
            "cut_layout::game::client_side::native_local_player_writes::tests::client_tick_body_yaw_writes_feed_local_living_render_state",
            "SUNRISE_CLIENT_BODY_YAW_CASE",
            &["frame"],
        ) {
            return;
        }
        struct Turns(Arc<Mutex<Vec<f32>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Turns {
            fn client_tick_post(
                &self,
                host: &mut dyn ClientHost,
                _: &mut mod_api::ClientTickPostEvent,
            ) {
                let player = host.local_player().unwrap();
                let stale = mod_api::__private::game::player_ref(
                    EntityRef::new(
                        mod_api::WorldEpoch::new(player.entity().epoch().get() + 1),
                        player.entity().id(),
                    ),
                    player.uuid(),
                );
                assert_eq!(stale.set_body_yaw(host, -100.0), Ok(false));
                assert_eq!(stale.entity().body_yaw(host), None);
                assert_eq!(player.entity().body_yaw(host), Some(0.0));
                assert_eq!(player.set_body_yaw(host, 75.0), Ok(true));
                assert_eq!(player.entity().body_yaw(host), Some(75.0));
            }
            fn render_living_pre(
                &self,
                _: &dyn ClientHost,
                event: &mut mod_api::RenderLivingPreEvent<'_>,
            ) -> mod_api::Flow {
                self.0.lock().unwrap().push(event.entity.body_yaw);
                mod_api::Flow::Continue
            }
        }
        let turns = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "yaw-frame",
            Box::new(Turns(turns.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                loaded_client(game);
                game.client.camera_type = sunrise_level::camera_math::CameraType::ThirdPersonBack;
                game.player = None;
                game.remote_teleport(&sunrise_protocol::packets::play::PlayerPosition {
                    teleport: 1,
                    position: [0.5, 64.0, 0.5],
                    delta: [0.0; 3],
                    yaw: 15.0,
                    pitch: 0.0,
                    relatives: 0,
                })
                .unwrap();
                let old = game.local_player().unwrap();
                assert_eq!(old.entity().body_yaw(game), Some(0.0));
                assert_eq!(
                    game.client.player_model.as_ref().unwrap().previous_body_yaw,
                    0.0
                );
                game.previous_position = game.player.as_ref().unwrap().position;
                game.tick_player_model();
                game.native_client_tick_post();
                game.prepare_entity_culling();
                let camera = glam::DVec3::new(0.5, 64.0, -4.5);
                game.player_model_geometry(camera, 0.5, &mut Vec::new());
                assert_eq!(*turns.lock().unwrap(), [37.5]);
                let model = game.client.player_model.as_ref().unwrap();
                assert_eq!(
                    (game.yaw, model.yaw, model.head_yaw, model.previous_body_yaw),
                    (15.0, 15.0, 15.0, 0.0)
                );

                use sunrise_protocol::packets::play::{AddEntity, Clientbound, SetPassengers};
                let set_passengers = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                                      vehicle,
                                      passengers| {
                    game.handle_packet(Clientbound::SetPassengers(SetPassengers {
                        vehicle,
                        passengers,
                    }))
                    .unwrap();
                };
                game.spawn_entity(43, "minecraft:pig", [0.5, 64.0, 0.5], [55.0, 0.0, 55.0]);
                let index = game
                    .common
                    .mobs
                    .iter()
                    .position(|mob| mob.id == 43)
                    .unwrap();
                let mount = &mut game.common.mobs[index];
                mount.previous_body_yaw = 35.0;
                mount.body_yaw = 55.0;
                set_passengers(game, 43, vec![42]);
                // The packet changed the vehicle, but the player hasn't ticked.
                game.prepare_entity_culling();
                game.player_model_geometry(camera, 0.5, &mut Vec::new());
                assert_eq!(old.entity().body_yaw(game), Some(75.0));
                assert_eq!(*turns.lock().unwrap(), [37.5, 45.0]);
                game.tick_remote_player(false, false).unwrap();

                assert_eq!(old.set_body_yaw(game, 75.0), Ok(true));
                let before = game.player.as_ref().unwrap().position.z;
                let index = game
                    .common
                    .mobs
                    .iter()
                    .position(|mob| mob.id == 43)
                    .unwrap();
                game.common.mobs[index].position.z += 3.0;
                game.tick_remote_player(false, false).unwrap();
                assert_eq!(game.player.as_ref().unwrap().position.z, before + 3.0);
                // No horizontal movement in the rider's own tick. A turn toward the
                // seat delta would instead blend toward its south-facing course.
                assert_eq!(old.entity().body_yaw(game), Some(65.0));
                set_passengers(game, 43, Vec::new());
                game.prepare_entity_culling();
                game.player_model_geometry(camera, 1.0, &mut Vec::new());
                assert_eq!(*turns.lock().unwrap(), [37.5, 45.0, 65.0]);

                let stand_type = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id("minecraft:armor_stand")
                    .unwrap();
                game.handle_packet(Clientbound::AddEntity(AddEntity {
                    id: 45,
                    uuid: 45,
                    kind: stand_type,
                    position: [0.5, 64.0, 3.5],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 32, // 45 degrees
                    data: 0,
                }))
                .unwrap();
                // The first frame uses the head-derived body samples, not either
                // view sample; no tick has synchronized the stand's angles yet.
                set_passengers(game, 45, vec![42]);
                game.prepare_entity_culling();
                game.player_model_geometry(camera, 0.5, &mut Vec::new());
                assert_eq!(*turns.lock().unwrap(), [37.5, 45.0, 65.0, 45.0]);
                let stand = game
                    .armor_stands
                    .iter()
                    .find(|stand| stand.id == 45)
                    .unwrap();
                assert_eq!((stand.yaw, stand.previous_yaw), (0.0, 0.0));

                let at = game.player.as_ref().unwrap().position;
                game.boats
                    .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                        44,
                        "oak".into(),
                        at,
                        385.0,
                    ));
                game.riding = Some(44);
                game.tick_player_model();
                assert_eq!(old.entity().body_yaw(game), Some(385.0));
                game.prepare_entity_culling();
                game.player_model_geometry(camera, 1.0, &mut Vec::new());
                assert_eq!(*turns.lock().unwrap(), [37.5, 45.0, 65.0, 45.0, 25.0]);
                assert_eq!(old.set_body_yaw(game, 50.0), Ok(true));
                game.turn_player(1.0, 0.0);
                assert_eq!(old.entity().body_yaw(game), Some(385.0));
                game.remove_local_player_body();
                assert_eq!(old.entity().body_yaw(game), None);
                assert_eq!(old.set_body_yaw(game, 90.0), Ok(false));
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn frame_tick_rate_reads_the_displayed_levels_configured_rate() {
        use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
        use sunrise_protocol::{
            connection::Pipe,
            packets::play::{self, Clientbound as C},
        };

        if run_fresh_cases(
            "cut_layout::game::client_side::native_local_player_writes::tests::frame_tick_rate_reads_the_displayed_levels_configured_rate",
            "SUNRISE_CLIENT_TICK_RATE_CASE",
            &["level"],
        ) {
            return;
        }
        type Observation = (Option<f32>, Option<f32>);
        #[derive(Default)]
        struct Rates {
            original: Option<LevelRef>,
            pre: Vec<Observation>,
            post: Vec<Observation>,
        }
        impl Rates {
            fn read(&self, host: &dyn ClientHost) -> Observation {
                let read = |level: Option<LevelRef>| {
                    level.and_then(|level| level.tick_rate(host).unwrap())
                };
                let level = host
                    .level("minecraft:overworld")
                    .or_else(|| host.level("minecraft:the_nether"));
                (read(level), read(self.original))
            }
        }
        struct RateFrames(Arc<Mutex<Rates>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for RateFrames {
            fn render_frame_pre(
                &self,
                host: &mut dyn ClientHost,
                _: &mut mod_api::RenderFramePreEvent,
            ) {
                let mut state = self.0.lock().unwrap();
                let observation = state.read(host);
                state.pre.push(observation);
            }
            fn render_frame_post(
                &self,
                host: &mut dyn ClientHost,
                _: &mut mod_api::RenderFramePostEvent,
            ) {
                let mut state = self.0.lock().unwrap();
                let observation = state.read(host);
                state.post.push(observation);
            }
        }
        let saw = Arc::new(Mutex::new(Rates::default()));
        let mods = vec![mod_api::ModEntry::new(
            "rate-frame",
            Box::new(RateFrames(saw.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "rate").unwrap();
                let spawn = play::SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 0,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: false,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                };
                game.handle_packet(C::Login(play::Login {
                    player_id: 7,
                    hardcore: false,
                    levels: vec!["minecraft:overworld".into(), "minecraft:the_nether".into()],
                    max_players: 1,
                    chunk_radius: 2,
                    simulation_distance: 2,
                    reduced_debug_info: false,
                    show_death_screen: true,
                    limited_crafting: false,
                    spawn: spawn.clone(),
                    online_mode: false,
                    enforces_secure_chat: false,
                }))
                .unwrap();
                saw.lock().unwrap().original = game.native_client_level();
                let frame = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                    game.native_render_frame_pre(Duration::ZERO, 0.5, 0.5);
                    game.finish_render_frame(Duration::ZERO);
                };
                frame(game);
                for (rate, frozen) in [(37.5, false), (11.25, true)] {
                    game.handle_packet(C::TickingState(play::TickingState { rate, frozen }))
                        .unwrap();
                    frame(game);
                }
                game.handle_packet(C::Respawn(play::Respawn {
                    spawn: play::SpawnInfo {
                        dimension: "minecraft:the_nether".into(),
                        ..spawn
                    },
                    keep: 0,
                }))
                .unwrap();
                frame(game);
                game.leave_server(None).unwrap();
                assert!(game.client.title.0.is_some());
                frame(game);
            });
        });
        let expected = [
            (Some(20.0), Some(20.0)),
            (Some(37.5), Some(37.5)),
            (Some(11.25), Some(11.25)),
            (Some(20.0), None),
            (None, None),
        ];
        let saw = saw.lock().unwrap();
        assert_eq!(saw.pre, expected);
        assert_eq!(saw.post, expected);
    }

    #[test]
    #[ignore = "daily"]
    fn a_velocity_written_before_the_tick_is_what_the_tick_moves_by() {
        let saw = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "boots",
            Box::new(Boots {
                velocity: [0.25, 0.0, 0.0],
                saw: Arc::clone(&saw),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                loaded_client(game);
                // The write itself sets the flag, either way, before any tick can overwrite it.
                for flag in [true, false, true] {
                    game.player.as_mut().unwrap().ground.on_ground = !flag;
                    assert!(game.set_local_player_on_ground(flag));
                    assert_eq!(game.player.as_ref().unwrap().ground.on_ground, flag);
                }
                let before = game.player.as_ref().unwrap().position;
                game.tick_remote().unwrap();
                let body = game.player.as_ref().unwrap();
                let heard = saw.lock().unwrap();
                assert_eq!(heard.len(), 1, "the hook fires once a tick");
                assert_eq!(
                    heard[0].0,
                    [true, true, false],
                    "a finite velocity and the flag are written, a NaN is refused"
                );
                let moved = body.position.x - before.x;
                assert!((moved - 0.25).abs() < 1.0e-9, "moved {moved}");
                assert!(!body.ground.on_ground, "the move set the flag from the air");
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_local_displacement_stops_at_a_wall_and_reports_how_far_it_moved() {
        struct Push(Arc<Mutex<Option<[f64; 3]>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Push {
            fn player_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
                if let HostSide::Client(host) = host
                    && host.local_player() == Some(event.player)
                {
                    *self.0.lock().unwrap() = host.move_local_player_by([3.0, 0.0, 0.0]).unwrap();
                }
            }
        }

        let moved = Arc::new(Mutex::new(None));
        let mods = vec![mod_api::ModEntry::new(
            "push",
            Box::new(Push(Arc::clone(&moved))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                loaded_client(game);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for y in 63..67 {
                    game.world.set(2, y, 0, stone).unwrap();
                }
                game.tick_remote().unwrap();
                // Standing players are 0.6 blocks wide. Their right edge stops at x = 2.
                let stopped_at = 2.0 - f64::from(0.6_f32 / 2.0);
                assert_eq!(*moved.lock().unwrap(), Some([stopped_at - 0.5, 0.0, 0.0]));
                assert_eq!(game.player.as_ref().unwrap().position.x, stopped_at);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn resetting_a_local_fall_discards_the_distance_before_landing() {
        use crate::game::server_side::native_entity_refs::tests::with_client_game;
        use mod_api::ClientHost;
        use sunrise_sim::{
            landing::{FallEvent, SceneSound},
            motion::MotionEvent,
        };

        with_client_game(|game| {
            loaded_client(game);
            let local = game.local_player().unwrap();
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    mod_api::WorldEpoch::new(local.entity().epoch().get() + 1),
                    local.entity().id(),
                ),
                local.uuid(),
            );
            assert_eq!(stale.reset_fall_distance(game), Ok(false));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 57, 0, stone).unwrap();

            for reset in [true, false] {
                let player = game.player.as_mut().unwrap();
                player.client_side = true;
                player.position.y = 64.0;
                player.ground.on_ground = false;
                player.fall_distance = 0.0;
                player.events.clear();
                let health = player.health.health();
                let host: &mut dyn ClientHost = game;
                assert_eq!(
                    host.move_local_player_by([0.0, -5.0, 0.0]),
                    Ok(Some([0.0, -5.0, 0.0]))
                );
                assert_eq!(game.player.as_ref().unwrap().fall_distance, 5.0);
                if reset {
                    assert_eq!(local.reset_fall_distance(game), Ok(true));
                    assert_eq!(game.player.as_ref().unwrap().fall_distance, 0.0);
                }
                let host: &mut dyn ClientHost = game;
                assert_eq!(
                    host.move_local_player_by([0.0, -2.0, 0.0]),
                    Ok(Some([0.0, -1.0, 0.0]))
                );
                let player = game.player.as_ref().unwrap();
                assert!(player.ground.on_ground);
                assert_eq!(player.health.health(), health);
                if reset {
                    assert!(!player.events.iter().any(|event| {
                        matches!(
                            event,
                            MotionEvent::Fall(
                                FallEvent::Sound { .. } | FallEvent::BlockFallSound { .. }
                            )
                        )
                    }));
                } else {
                    assert!(player.events.iter().any(|event| {
                        matches!(
                            event,
                            MotionEvent::Fall(FallEvent::Sound {
                                sound: SceneSound::SmallFall,
                                ..
                            })
                        )
                    }));
                }
            }
            game.player = None;
            assert_eq!(local.reset_fall_distance(game), Ok(false));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_local_displacement_respects_collider_exemptions_when_moving_and_sneaking() {
        use sunrise_core::geometry::{
            Aabb, Vec3,
            oriented::{ColliderBoxes, Frame, OrientedCollider},
        };
        use sunrise_world::moving_colliders::{Placed, Prepared, Scope};

        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            loaded_client(game);
            let start = game.player.as_ref().unwrap().position;
            let player_id = game.own_entity_id() as u32;
            let level = game.client_collider_level().unwrap();
            let mut sweep = |min: Vec3, size: Vec3, sneaking: bool, exempt: bool| {
                let player = game.player.as_mut().unwrap();
                player.position = start;
                player.shift_key_down = sneaking;
                player.ground.on_ground = sneaking;
                let _scope = Scope::enter_level(
                    Some(level),
                    Prepared::new(vec![Placed {
                        id: 900,
                        collider: OrientedCollider::new(
                            Frame::translation(min),
                            Arc::new(ColliderBoxes::new(vec![Aabb::new(Vec3::ZERO, size)])),
                        ),
                        exempt: if exempt {
                            vec![900, player_id]
                        } else {
                            vec![900]
                        },
                    }]),
                );
                game.move_local_player_by([2.0, 0.0, 0.0]).unwrap().unwrap()
            };

            let wall = Vec3::new(1.5, 63.0, 0.0);
            let wall_size = Vec3::new(1.0, 4.0, 1.0);
            assert_eq!(sweep(wall, wall_size, false, true), [2.0, 0.0, 0.0]);
            let to_wall = 1.5 - 0.5 - f64::from(0.6_f32 / 2.0);
            assert_eq!(sweep(wall, wall_size, false, false), [to_wall, 0.0, 0.0]);

            // Only the collider below the destination could provide support. Sneaking
            // leaves the player in place when it doesn't count for this player.
            let platform = Vec3::new(2.0, 63.0, 0.0);
            let platform_size = Vec3::new(1.0, 0.875, 1.0);
            assert_eq!(sweep(platform, platform_size, true, true), [0.0; 3]);
            assert_eq!(sweep(platform, platform_size, true, false), [2.0, 0.0, 0.0]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn local_displacements_out_and_back_notify_both_section_crossings() {
        type Crossing = ([i32; 3], [i32; 3], [f64; 3], Option<bool>);
        #[derive(Default)]
        struct Walk(Arc<Mutex<Vec<Crossing>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Walk {
            fn player_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
                if let HostSide::Client(host) = host
                    && host.local_player() == Some(event.player)
                {
                    assert!(host.set_local_player_on_ground(true));
                    assert_eq!(
                        host.move_local_player_by([16.0, 0.0, 0.0]),
                        Ok(Some([16.0, 0.0, 0.0]))
                    );
                    assert_eq!(self.0.lock().unwrap().len(), 1);
                    assert_eq!(event.player.entity().on_ground(host), Some(false));
                    assert_eq!(
                        host.move_local_player_by([-16.0, 0.0, 0.0]),
                        Ok(Some([-16.0, 0.0, 0.0]))
                    );
                    assert_eq!(self.0.lock().unwrap().len(), 2);
                }
            }

            fn entity_entering_section(
                &self,
                host: HostSide<'_>,
                event: &mut mod_api::EntityEnteringSectionEvent,
            ) {
                if let HostSide::Client(host) = host
                    && let Some(player) = host.local_player()
                    && player.entity() == event.entity
                {
                    assert_eq!(
                        event.entity.residency(host),
                        mod_api::EntityResidency::InPlace
                    );
                    assert_eq!(
                        player.reset_fall_distance(host),
                        Err(mod_api::LevelWriteError::Unavailable),
                    );
                    assert!(host.client_executor().is_none());
                    self.0.lock().unwrap().push((
                        event.old_section,
                        event.new_section,
                        event.entity.position(host).unwrap(),
                        event.entity.on_ground(host),
                    ));
                }
            }
        }

        let walk = Walk::default();
        let crossings = Arc::clone(&walk.0);
        let mods = vec![mod_api::ModEntry::new("walk", Box::new(walk))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                loaded_client(game);
                // Keep the tick's normal gravity away from a section boundary.
                game.player.as_mut().unwrap().position.y = 64.5;
                game.client.streaming.insert_remote(
                    &mut game.common.world,
                    sunrise_level::chunk::Chunk::empty(1, 0),
                );
                game.tick_remote().unwrap();
                assert_eq!(
                    *crossings.lock().unwrap(),
                    [
                        ([0, 4, 0], [1, 4, 0], [16.5, 64.5, 0.5], Some(true)),
                        ([1, 4, 0], [0, 4, 0], [0.5, 64.5, 0.5], Some(false))
                    ],
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn with_no_player_of_its_own_both_writes_answer_false() {
        with_client_game(|game| {
            game.player = None;
            assert!(!game.set_local_player_velocity([0.0; 3]));
            assert!(!game.set_local_player_on_ground(true));
            assert!(!game.start_local_player_gliding());
            assert_eq!(game.move_local_player_by([1.0, 0.0, 0.0]), Ok(None));
        });
    }

    /// A client whose connection completed login and configuration, with its player loaded.
    fn loaded_menu_client(
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    ) -> sunrise_protocol::connection::ConnReader {
        let (peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
        crate::game::remote::tests::spawn_menu_test(game);
        let remote = game.client.remote.as_mut().unwrap();
        remote.loaded_sent = true;
        let mut player = sunrise_sim::motion::PlayerMotion::new(
            game.common.motion,
            sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
        )
        .unwrap();
        player.entity_id = remote.entity_id;
        game.player = Some(player);
        game.client.streaming.insert_remote(
            &mut game.common.world,
            sunrise_level::chunk::Chunk::empty(0, 0),
        );
        peer
    }

    #[test]
    #[ignore = "daily"]
    fn start_local_player_gliding_from_the_tick_pre_hook_reports_the_start() {
        struct Start;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Start {
            fn player_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
                if let HostSide::Client(host) = host
                    && host.local_player() == Some(event.player)
                {
                    assert!(host.start_local_player_gliding());
                }
            }
        }
        let mods = vec![mod_api::ModEntry::new("start", Box::new(Start))].into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let mut peer = loaded_menu_client(game);
                game.player.as_mut().unwrap().fall_flying = false;
                game.tick_remote().unwrap();
                assert!(game.player.as_ref().unwrap().fall_flying);
                peer.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut seen = 0;
                loop {
                    let (id, body) = peer.recv().unwrap_or_else(|e| {
                        panic!("no START_FALL_FLYING after {seen} packets: {e}")
                    });
                    seen += 1;
                    if let Ok(play::Serverbound::PlayerCommand(command)) =
                        play::Serverbound::decode(id, &body)
                        && command.action == 6
                    {
                        assert_eq!(command.data, 0);
                        break;
                    }
                }
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_collision_flags_distinguish_a_ceiling_from_the_floor_and_a_wall() {
        use sunrise_core::geometry::Vec3;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::motion::PlayerMotion;
        use sunrise_world::entity_move::MoverType;
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
        };

        fn check<'g, G: sunrise_gameplay_common::game::SideGame<'g> + mod_api::Host>(game: &mut G) {
            let player =
                sunrise_gameplay_common::game::native_server::NativeServerGameExt1::player(
                    game, 456,
                )
                .unwrap();
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 66, 0, stone).unwrap();
            game.world.set(2, 64, 0, stone).unwrap();

            let mut move_and_read = |movement| {
                game.move_local_player(MoverType::SelfMovement, movement, false)
                    .unwrap();
                player.collision_flags(game).unwrap().unwrap()
            };
            let ceiling = move_and_read(Vec3::new(0.0, 1.0, 0.0));
            assert!(ceiling.vertical);
            assert!(!ceiling.vertical_below);
            assert!(!ceiling.horizontal);

            let floor = move_and_read(Vec3::new(0.0, -1.0, 0.0));
            assert!(floor.vertical);
            assert!(floor.vertical_below);
            assert!(!floor.horizontal);

            let wall = move_and_read(Vec3::new(3.0, 0.0, 0.0));
            assert!(wall.horizontal);
            assert!(!wall.minor_horizontal);
        }

        with_client_game(|game| {
            loaded_client(game);
            check(game);
        });
        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            bind_local_player(game, 42, 456);
            game.server
                .streaming
                .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
            check(game);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn local_displacements_refuse_bad_deltas_and_players_held_in_place() {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            loaded_client(game);
            let before = game.player.as_ref().unwrap().position;
            for x in [f64::NAN, f64::INFINITY, -f64::INFINITY, 64.5, -64.5] {
                assert_eq!(game.move_local_player_by([x, 0.0, 0.0]), Ok(None));
            }
            game.riding = Some(43);
            assert_eq!(game.move_local_player_by([1.0, 0.0, 0.0]), Ok(None));
            game.riding = None;
            game.sleeping = Some([0, 64, 0]);
            assert_eq!(game.move_local_player_by([1.0, 0.0, 0.0]), Ok(None));
            game.sleeping = None;
            game.client.screen = sunrise_gameplay_common::game::Screen::Dead;
            assert_eq!(game.move_local_player_by([1.0, 0.0, 0.0]), Ok(None));
            assert_eq!(game.player.as_ref().unwrap().position, before);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn creative_give_adds_to_local_player_and_reports_remainder() {
        use mod_api::CreativeGiveError;
        use sunrise_items::items::ItemStack;

        with_client_game(|game| {
            loaded_client(game);
            let local = game.local_player().unwrap();
            let diamond = mod_api::__private::item_key("minecraft:diamond").unwrap();
            let diamonds = |count| mod_api::ItemStack::new(diamond, count).unwrap();
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    mod_api::WorldEpoch::new(local.entity().epoch().get().wrapping_add(1)),
                    local.entity().id(),
                ),
                local.uuid(),
            );

            game.creative = false;
            {
                let host: &mut dyn ClientHost = game;
                assert_eq!(
                    local.give_creative(host, &diamonds(3)),
                    Err(CreativeGiveError::NotCreative)
                );
                assert_eq!(
                    stale.give_creative(host, &diamonds(3)),
                    Err(CreativeGiveError::Stale)
                );
            }

            game.creative = true;
            game.inventory.fill(None);
            game.offhand = None;
            {
                let host: &mut dyn ClientHost = game;
                assert_eq!(local.give_creative(host, &diamonds(3)), Ok(None));
            }
            assert_eq!(
                game.inventory[game.selected]
                    .as_ref()
                    .map(|s| (s.item.as_ref(), s.count)),
                Some(("minecraft:diamond", 3))
            );

            // Fill every slot with full stone stacks so nothing fits.
            game.inventory
                .fill(Some(ItemStack::new("minecraft:stone", 64)));
            game.offhand = Some(ItemStack::new("minecraft:stone", 64));
            let selected = game.selected;
            assert_eq!(
                local.give_creative(game, &diamonds(1)),
                Ok(Some(diamonds(1)))
            );
            game.inventory[selected] = Some(ItemStack::new("minecraft:diamond", 63));
            assert_eq!(
                local.give_creative(game, &diamonds(2)),
                Ok(Some(diamonds(1)))
            );
            assert_eq!(game.inventory[selected].as_ref().unwrap().count, 64);
        });
    }
}
