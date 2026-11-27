//! The native travel decision affects the server copy without sending extra corrections.
use super::*;
use mod_api::{EntityMoveEvent, Flow, Hooks, LivingTravelEvent, LoopContext, MoveCause};
use std::sync::atomic::{AtomicUsize, Ordering};
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use sunrise_gameplay_common::game::server_side::native_item_places::tests::guest as online_guest;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::play;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_sim::lan::{NetConfig, Out, hub::HubConfig};
use {
    sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
};

pub(crate) const START: [f64; 3] = [3.0, 64.0, 5.0];
pub(crate) const ACCEPTED: [f64; 3] = [3.25, 64.0, 5.0];
pub(crate) const NEXT: [f64; 3] = [3.5, 64.0, 5.0];

#[derive(Clone, Copy)]
pub(crate) enum Reply {
    Continue,
    Veto,
    Velocity,
    TinyInput,
    OverflowingInputLength,
    Input,
    LargeInput,
    LargeVelocity,
    HugeVelocity,
    NonFiniteVelocityWithInput,
    NonFiniteInputWithVelocity,
    NonFiniteNoOp,
    VetoWithNonFiniteVelocity,
}

#[mod_api::__subscriptions]
impl Hooks for Reply {
    fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
        assert_eq!(event.kind, "minecraft:player");
        match self {
            Self::Continue => {}
            Self::Veto => return Flow::Cancel,
            Self::Velocity => *event.velocity = [0.5, 0.0, 0.0],
            Self::TinyInput => *event.input = [0.000001, 0.0, 0.0],
            Self::OverflowingInputLength => *event.input = [f64::MAX, 0.0, 0.0],
            Self::Input => *event.input = [1.0, 0.0, 0.0],
            Self::LargeInput => *event.input = [2.0, 0.0, 0.0],
            Self::LargeVelocity => *event.velocity = [70.0, 0.0, 0.0],
            Self::HugeVelocity => *event.velocity = [1.0e9, 0.0, 1.0e9],
            Self::NonFiniteVelocityWithInput => {
                *event.velocity = [f64::NAN, 0.0, 0.0];
                *event.input = [1.0, 0.0, 0.0];
            }
            Self::NonFiniteInputWithVelocity => {
                *event.input = [f64::INFINITY, 0.0, 0.0];
                *event.velocity = [0.5, 0.0, 0.0];
            }
            Self::NonFiniteNoOp => {
                *event.input = [f64::NAN, 0.0, 0.0];
                *event.velocity = [f64::INFINITY, 0.0, 0.0];
            }
            Self::VetoWithNonFiniteVelocity => {
                *event.velocity = [f64::NAN, 0.0, 0.0];
                return Flow::Cancel;
            }
        }
        Flow::Continue
    }
}

pub(crate) fn with_player(
    integrated: bool,
    run: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, mpsc::Receiver<Out>),
) {
    let role = if integrated {
        ExecutionRole::IntegratedServer
    } else {
        ExecutionRole::DedicatedServer
    };
    with_game_role(fixture_root(), role, |game| {
        game.lan_host(
            sunrise_gameplay::game::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            },
            NetConfig {
                listen: false,
                announce: false,
                ..NetConfig::loopback(0, "living-travel".into(), 2)
            },
            HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: "minecraft:overworld".into(),
                allow_flight: false,
                seed: 0,
                max_players: 2,
                integrated,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        {
            let (common, server) =
                sunrise_gameplay_common::game::SideGame::common_and_state_mut(&mut *game);
            server
                .streaming
                .insert_remote(&mut common.world, Chunk::empty(0, 0));
        }
        let mut guest = online_guest(1);
        guest.loaded = true;
        guest.load_timeout = 0;
        guest.position = START;
        guest.first_good = START;
        guest.last_reported = (START, [0.0, 0.0]);
        guest.motion = Some(Box::new(
            PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 5.0)).unwrap(),
        ));
        let (tx, rx) = mpsc::channel();
        guest.sender = PlayerSender::new(tx, Arc::default());
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.ticks = 10;
        hub.singleplayer_owner = integrated.then_some(guest.uuid);
        hub.guests.push(guest);
        run(game, rx);
    });
}

pub(crate) fn with_reply(reply: Option<Reply>, run: impl FnOnce()) {
    let mods = reply
        .map(|reply| mod_api::ModEntry::new("travel-test", Box::new(reply)))
        .into_iter()
        .collect::<Vec<_>>();
    mod_platform::testing::with_mods(mods.into(), run);
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::lan_actions_server::guest_body::tests::travel::send;

pub(crate) fn report(game: &mut sunrise_gameplay::game::ServerGame<'_>, [x, y, z]: [f64; 3]) {
    send(
        game,
        S::MovePlayerPos(play::MovePlayerPos { x, y, z, flags: 0 }),
    );
}

#[test]
#[ignore = "daily"]
fn a_connected_players_report_never_exposes_stale_collision_flags() {
    for integrated in [true, false] {
        with_player(integrated, |game, _rx| {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(1, |g| {
                    g.motion.as_mut().unwrap().collision.horizontal = true;
                })
                .unwrap();
            let player = game.player(guest(game).uuid).unwrap();
            report(game, ACCEPTED);
            assert_eq!(guest(game).position, ACCEPTED);
            assert_eq!(player.collision_flags(game), Ok(None));
            game.as_guest(1, |game| {
                assert_eq!(player.collision_flags(game), Ok(None));
            })
            .unwrap();
        });
    }
}

pub(crate) fn guest<'a>(game: &'a sunrise_gameplay::game::ServerGame<'_>) -> &'a Guest {
    game.server.lan.as_ref().unwrap().hub.guest(1).unwrap()
}

pub(crate) fn packets(rx: &mpsc::Receiver<Out>) -> Vec<(i32, Vec<u8>)> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            _ => Vec::new(),
        })
        .map(|body| {
            let mut r = sunrise_protocol::buf::Reader::new(&body);
            (r.varint().unwrap(), r.rest().to_vec())
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn a_player_travel_veto_skips_server_travel_without_clamping_reports() {
    for integrated in [true, false] {
        with_reply(Some(Reply::Veto), || {
            with_player(integrated, |game, rx| {
                report(game, ACCEPTED);
                assert_eq!(guest(game).position, ACCEPTED);
                game.server.lan.as_mut().unwrap().hub.update_guest(1, |g| {
                    g.motion.as_mut().unwrap().velocity = Vec3::new(0.4, 0.0, 0.0);
                });
                game.guest_server_tick(1, true);
                assert_eq!(guest(game).position, ACCEPTED);
                assert_eq!(
                    guest(game).motion.as_ref().unwrap().velocity,
                    Vec3::new(0.4, 0.0, 0.0)
                );
                assert!(guest(game).awaiting_teleport.is_none());
                assert!(!packets(&rx).into_iter().any(|(id, body)| matches!(
                    C::decode(id, &body).unwrap(),
                    C::PlayerPosition(_) | C::SetEntityMotion(_)
                )));

                report(game, NEXT);
                assert_eq!(guest(game).position, NEXT);
                assert!(guest(game).awaiting_teleport.is_none());
                assert!(!packets(&rx).into_iter().any(|(id, body)| matches!(
                    C::decode(id, &body).unwrap(),
                    C::PlayerPosition(_) | C::SetEntityMotion(_)
                )));
            })
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_player_travel_edit_updates_server_velocity_without_correction() {
    for integrated in [true, false] {
        for reply in [Reply::Velocity, Reply::Input] {
            with_reply(Some(reply), || {
                with_player(integrated, |game, rx| {
                    game.guest_server_tick(1, true);
                    let displacement_x = match reply {
                        Reply::Velocity => 0.5,
                        // The default off-ground movement speed is the widened vanilla 0.02f.
                        _ => f64::from(0.02_f32),
                    };
                    let expected = Vec3::new(
                        displacement_x * f64::from(0.91_f32),
                        -0.08 * f64::from(0.98_f32),
                        0.0,
                    );
                    assert_eq!(guest(game).position, START);
                    assert!(guest(game).awaiting_teleport.is_none());
                    assert_eq!(guest(game).motion.as_ref().unwrap().velocity, expected);
                    assert!(!packets(&rx).into_iter().any(|(id, body)| matches!(
                        C::decode(id, &body).unwrap(),
                        C::PlayerPosition(_) | C::SetEntityMotion(_)
                    )));
                })
            });
        }
    }
}

#[test]
#[ignore = "daily"]
fn an_unchanged_player_travel_outcome_keeps_vanilla_reports_and_packets() {
    if run_fresh_cases(
        "cut_layout::game::lan_actions::guest_body::tests::travel::an_unchanged_player_travel_outcome_keeps_vanilla_reports_and_packets",
        "SUNRISE_PLAYER_TRAVEL_CHILD",
        &["travel"],
    ) {
        return;
    }
    for integrated in [true, false] {
        let mut baseline = None;
        for reply in [
            None,
            Some(Reply::Continue),
            Some(Reply::TinyInput),
            // The vector is finite, but its norm is infinite, so Vec3.normalize yields zero.
            Some(Reply::OverflowingInputLength),
        ] {
            with_reply(reply, || {
                with_player(integrated, |game, rx| {
                    report(game, ACCEPTED);
                    game.guest_server_tick(1, true);
                    report(game, NEXT);
                    let guest = guest(game);
                    assert_eq!(guest.position, NEXT);
                    assert!(guest.awaiting_teleport.is_none());
                    let sent = packets(&rx);
                    assert!(!sent.iter().any(|(id, body)| matches!(
                        C::decode(*id, body).unwrap(),
                        C::PlayerPosition(_) | C::SetEntityMotion(_)
                    )));
                    let velocity = guest.motion.as_ref().unwrap().velocity;
                    let outcome = (velocity.components().map(f64::to_bits), sent);
                    if let Some(baseline) = &baseline {
                        assert_eq!(&outcome, baseline);
                    } else {
                        baseline = Some(outcome);
                    }
                })
            });
        }
    }
}

pub(crate) struct MoveVeto {
    pub(crate) travel: Reply,
    pub(crate) moves: Arc<AtomicUsize>,
}

#[mod_api::__subscriptions]
impl Hooks for MoveVeto {
    fn living_travel(&self, context: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
        self.travel.living_travel(context, event)
    }

    fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
        if event.kind == "minecraft:player" && event.cause == MoveCause::SelfMovement {
            self.moves.fetch_add(1, Ordering::Relaxed);
            return Flow::Cancel;
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn a_tiny_travel_edit_and_move_veto_leave_the_player_outcome_unchanged() {
    for travel in [Reply::Continue, Reply::TinyInput] {
        let moves = Arc::new(AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "travel-veto-test",
            Box::new(MoveVeto {
                travel,
                moves: moves.clone(),
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_player(false, |game, rx| {
                game.server.lan.as_mut().unwrap().hub.update_guest(1, |g| {
                    g.motion.as_mut().unwrap().velocity.x = 0.5;
                });
                game.guest_server_tick(1, true);
                let g = guest(game);
                let body = g.motion.as_ref().unwrap();
                assert_eq!(
                    body.position.components().map(f64::to_bits),
                    START.map(f64::to_bits)
                );
                assert_eq!(g.position.map(f64::to_bits), START.map(f64::to_bits));
                assert_eq!(
                    body.velocity.components().map(f64::to_bits),
                    [0.5 * f64::from(0.91_f32), -0.08 * f64::from(0.98_f32), 0.0].map(f64::to_bits)
                );
                assert_eq!(moves.load(Ordering::Relaxed), 1);
                assert!(!packets(&rx).into_iter().any(|(id, body)| matches!(
                    C::decode(id, &body).unwrap(),
                    C::PlayerPosition(_) | C::SetEntityMotion(_)
                )));
            });
        });
    }
}

#[test]
#[ignore = "daily"]
fn vanilla_travel_operations_apply_large_and_non_finite_edits_independently() {
    if run_fresh_cases(
        "cut_layout::game::lan_actions::guest_body::tests::travel::vanilla_travel_operations_apply_large_and_non_finite_edits_independently",
        "SUNRISE_PLAYER_TRAVEL_CHILD",
        &["travel"],
    ) {
        return;
    }
    let cases = [
        (Reply::LargeInput, f64::from(0.02_f32)),
        (Reply::LargeVelocity, 70.0),
        (Reply::NonFiniteVelocityWithInput, f64::from(0.02_f32)),
        (Reply::NonFiniteInputWithVelocity, 0.5),
    ];
    for (reply, displacement_x) in cases {
        with_reply(Some(reply), || {
            with_player(false, |game, rx| {
                for cx in 1..=5 {
                    let (common, server) =
                        sunrise_gameplay_common::game::SideGame::common_and_state_mut(&mut *game);
                    server
                        .streaming
                        .insert_remote(&mut common.world, Chunk::empty(cx, 0));
                }
                game.guest_server_tick(1, true);
                let expected = Vec3::new(
                    displacement_x * f64::from(0.91_f32),
                    -0.08 * f64::from(0.98_f32),
                    0.0,
                );
                assert_eq!(guest(game).position, START);
                assert_eq!(guest(game).motion.as_ref().unwrap().velocity, expected);
                assert!(!packets(&rx).into_iter().any(|(id, body)| matches!(
                    C::decode(id, &body).unwrap(),
                    C::PlayerPosition(_) | C::SetEntityMotion(_)
                )));
            });
        });
    }

    // A finite velocity whose swept box the collision scan cannot cover fails the travel. None of
    // its work is published: the velocity stands as it was and no correction packet is sent.
    with_reply(Some(Reply::HugeVelocity), || {
        with_player(false, |game, rx| {
            let before = guest(game).motion.as_ref().unwrap().velocity;
            game.guest_server_tick(1, true);
            assert_eq!(guest(game).position, START);
            assert_eq!(guest(game).motion.as_ref().unwrap().velocity, before);
            assert!(!packets(&rx).into_iter().any(|(id, body)| matches!(
                C::decode(id, &body).unwrap(),
                C::PlayerPosition(_) | C::SetEntityMotion(_)
            )));
        });
    });

    with_reply(Some(Reply::NonFiniteNoOp), || {
        with_player(false, |game, rx| {
            report(game, ACCEPTED);
            game.guest_server_tick(1, true);
            report(game, NEXT);
            let guest = guest(game);
            assert_eq!(guest.position, NEXT);
            assert!(guest.awaiting_teleport.is_none());
            assert!(guest.motion.as_ref().unwrap().velocity.is_finite());
            assert!(!packets(&rx).into_iter().any(|(id, body)| matches!(
                C::decode(id, &body).unwrap(),
                C::PlayerPosition(_) | C::SetEntityMotion(_)
            )));
        });
    });

    with_reply(Some(Reply::VetoWithNonFiniteVelocity), || {
        with_player(false, |game, rx| {
            report(game, ACCEPTED);
            game.guest_server_tick(1, true);
            report(game, NEXT);
            assert_eq!(guest(game).position, NEXT);
            assert!(guest(game).awaiting_teleport.is_none());
            assert!(!packets(&rx).into_iter().any(|(id, body)| matches!(
                C::decode(id, &body).unwrap(),
                C::PlayerPosition(_) | C::SetEntityMotion(_)
            )));
        });
    });
}

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
