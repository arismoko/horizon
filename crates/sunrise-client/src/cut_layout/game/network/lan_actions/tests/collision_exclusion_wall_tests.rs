//! #3717: a block sharing volume with one of a player's collision exclusions is no wall: the
//! server's `isInWall` doesn't count it, and the client doesn't push its player out of it.
use std::sync::Arc;
use sunrise_core::geometry::Vec3;

/// A player-sized body at (3.5, 64, 5.5): stone in its column (3, 64..=65, 5) and on its west,
/// north and south sides, so east is the one free side.
fn walled(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
    chunk.lit = true;
    game.world.chunks.insert((0, 0), Arc::new(chunk));
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for y in 64..=65 {
        for (x, z) in [(3, 5), (2, 5), (3, 4), (3, 6)] {
            game.world.set(x, y, z, stone).unwrap();
        }
    }
}

fn exclude(game: &mut sunrise_gameplay::game::ServerGame<'_>, boxes: &[[f64; 6]]) {
    let hub = &mut game.server.lan.as_mut().unwrap().hub;
    hub.update_guest(12, |g| g.collision_exclusions.set(boxes))
        .unwrap()
        .unwrap();
}

/// The opening over the body's own column.
const OPENING: [[f64; 6]; 1] = [[3.0, 64.0, 5.0, 4.0, 66.0, 6.0]];

#[test]
#[ignore = "daily"]
fn a_block_in_a_players_exclusions_does_not_suffocate_it() {
    sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
        walled(game);
        let at = [3.5, 64.0, 5.5];
        let body = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(12)
            .unwrap()
            .dimensions();
        assert!(
            game.in_wall(12, at, body),
            "its eyes are in the stone at y 65"
        );
        exclude(game, &[[10.0, 64.0, 10.0, 11.0, 66.0, 11.0]]);
        assert!(
            game.in_wall(12, at, body),
            "a box elsewhere changes nothing"
        );
        exclude(game, &OPENING);
        assert!(!game.in_wall(12, at, body));
    });
}

#[test]
#[ignore = "daily"]
fn a_block_in_a_players_exclusions_does_not_push_it_out() {
    sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
        walled(game);
        let mut body =
            sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(3.5, 64.0, 5.5)).unwrap();
        let nudge = |game: &sunrise_gameplay::game::ServerGame<'_>,
                     body: &sunrise_sim::motion::PlayerMotion| {
            sunrise_gameplay_common::game::closest_space_nudge(
                body,
                &game.world,
                game.collision,
                game.level_suffocation(),
            )
        };
        // Every corner is in the stone; east is the nearest free side: 0.1 a tick east.
        assert_eq!(nudge(game, &body), [Some(0.1), None]);
        body.collision_exclusions.set(&OPENING).unwrap();
        assert_eq!(nudge(game, &body), [None, None]);
    });
}

/// A box of a moving collider at a guest's eyes is a wall, in the level as it stands now: the
/// guest's committed position is in the current frame, so a carried guest is not tested against the
/// collider where its client saw it some ticks ago.
#[test]
#[ignore = "daily"]
fn a_carried_guests_eyes_are_tested_against_the_collider_as_it_stands_now() {
    use sunrise_core::geometry::carried::Carried;
    use sunrise_core::geometry::oriented::{ColliderBoxes, Frame, OrientedCollider};
    use sunrise_world::moving_colliders::{History, HistoryScope, Placed, Prepared, in_level};
    const COLLIDER: u32 = 9_999;
    let placed_at = |x: f64| {
        let boxes = ColliderBoxes::new(vec![sunrise_core::geometry::Aabb::new(
            Vec3::new(-1.0, 65.0, -1.0),
            Vec3::new(1.0, 66.5, 1.0),
        )]);
        Prepared::new(vec![Placed {
            id: COLLIDER,
            collider: OrientedCollider::new(
                Frame::translation(Vec3::new(x, 0.0, 5.5)),
                Arc::new(boxes),
            ),
            exempt: Vec::new(),
        }])
    };
    sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
        let at = [3.5, 64.0, 5.5];
        let body = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(12)
            .unwrap()
            .dimensions();
        // Four ticks ago the collider was 30 blocks away; now it is around the guest's eyes.
        let mut history = History::default();
        for _ in 0..4 {
            history.record(&placed_at(33.5));
        }
        let now = placed_at(3.5);
        history.record(&now);
        let history = std::rc::Rc::new(std::cell::RefCell::new(history));
        let local = Vec3::new(0.0, 0.0, 0.0);
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.update_guest(12, |g| {
            g.on_collider = Some(Carried {
                collider: COLLIDER,
                age: 4.0,
                local,
                reported: local,
                committed: local,
            });
        })
        .unwrap();
        let _scope = HistoryScope::enter(history);
        let inside = in_level(now, || game.in_wall(12, at, body));
        assert!(inside, "the collider is at the eyes now");
        let away = in_level(placed_at(33.5), || game.in_wall(12, at, body));
        assert!(!away, "and not there when it is elsewhere now");
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;
