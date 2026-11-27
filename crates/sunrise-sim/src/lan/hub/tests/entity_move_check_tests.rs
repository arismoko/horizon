//! #2223: the server's moved-wrongly check resolves a reported move against the level's solid
//! entities (`Entity.collide`'s entity collisions), so a client that walks through a shulker is
//! told it moved wrongly, and one that stops at its side is not.
use super::*;
use crate::entity_solids::{Candidate, SolidKind};
use sunrise_core::geometry::{Aabb, Vec3};

/// A stone floor with its top at y 5 and air above it.
fn floor(fixture: &crate::test_fixtures::Vanilla) -> World {
    let assets = &fixture.assets;
    let mut scene = World::test_scene_with_placements(assets, &[], 2).unwrap();
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    let air = assets.registry.default_state("minecraft:air").unwrap();
    for x in 10..=30 {
        for z in 10..=30 {
            scene.set(x, 4, z, stone).unwrap();
            for y in 5..=12 {
                scene.set(x, y, z, air).unwrap();
            }
        }
    }
    scene
}

/// A shulker with its west face at x 22, standing on the floor: a full block high, so the
/// player's 0.6 step does not carry it over.
fn shulker() -> Candidate {
    Candidate::fixed(
        SolidKind::Shulker,
        500,
        Aabb::new(Vec3::new(22.0, 5.0, 20.0), Vec3::new(23.0, 6.0, 21.0)),
    )
}

#[test]
#[ignore = "daily"]
fn a_reported_move_through_a_shulker_is_moved_wrongly_and_one_that_stops_at_its_side_is_not() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let mut world = floor(fixture);
    let (mut hub, _rx) = hub_with_a_guest();
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [20.5, 5.0, 20.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: Some(&fixture.collision),
        border: &border,
        streaming: None,
    };
    hub.guests.update(0, |g| {
        g.position = [20.5, 5.0, 20.5];
        g.on_ground = true;
    });
    // The player's half width is 0.3: stopped by the boat, it stands at x 21.7.
    let through = [23.0, 5.0, 20.5];
    let beside = [21.7, 5.0, 20.5];

    // No entity: the move east is free, whatever the report.
    let (wrongly, refused, _) = hub.collide(0, through, &ctx);
    assert!(!wrongly && !refused, "no boat: {wrongly} {refused}");

    hub.move_solids = vec![shulker()];
    let (wrongly, refused, _) = hub.collide(0, through, &ctx);
    assert!(
        wrongly && refused,
        "through the entity: {wrongly} {refused}"
    );
    let (wrongly, refused, _) = hub.collide(0, beside, &ctx);
    assert!(
        !wrongly && !refused,
        "beside the entity: {wrongly} {refused}"
    );

    // An entity the move does not reach changes nothing.
    hub.move_solids = vec![Candidate::fixed(
        SolidKind::Shulker,
        501,
        Aabb::new(Vec3::new(26.0, 5.0, 20.0), Vec3::new(27.0, 6.0, 21.0)),
    )];
    let (wrongly, refused, _) = hub.collide(0, [22.5, 5.0, 20.5], &ctx);
    assert!(!wrongly && !refused, "a far entity: {wrongly} {refused}");
}

/// Runs `test` with a hub whose guest 0 stands at (20.5, 5, 20.5) on the floor, and its tick context.
fn with_rig(test: impl FnOnce(&mut Hub, &mut TickCtx)) {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let mut world = floor(fixture);
    let (mut hub, _rx) = hub_with_a_guest();
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let mut ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [20.5, 5.0, 20.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: Some(&fixture.collision),
        border: &border,
        streaming: None,
    };
    hub.guests.update(0, |g| {
        g.position = [20.5, 5.0, 20.5];
        g.on_ground = true;
    });
    test(&mut hub, &mut ctx);
}

#[test]
#[ignore = "daily"]
fn an_accepted_move_still_yields_when_it_logs_moved_wrongly() {
    // Unlike a clean old box, an already obstructed start can accept a move
    // logging "moved wrongly" when the destination is clear.
    with_rig(|hub, ctx| {
        hub.dispatch_move_effects = true;
        hub.move_solids = vec![
            Candidate::fixed(
                SolidKind::Shulker,
                501,
                Aabb::new(Vec3::new(20.0, 5.0, 20.0), Vec3::new(21.0, 6.0, 21.0)),
            ),
            shulker(),
        ];
        hub.guests.update(0, |g| {
            g.load_timeout = 0;
            g.inbox.extend([
                InboxItem::Packet(
                    S::MovePlayerPos(play::MovePlayerPos {
                        x: 24.5,
                        y: 5.0,
                        z: 20.5,
                        flags: 3,
                    }),
                    None,
                ),
                InboxItem::Packet(
                    S::MovePlayerRot(play::MovePlayerRot {
                        yaw: 25.0,
                        pitch: 0.0,
                        flags: 1,
                    }),
                    None,
                ),
            ]);
        });
        let mut events = Vec::new();
        let mut held = false;
        assert!(hub.process_inbox(0, ctx, &mut events, true, &mut held, &mut None));
        assert!(
            matches!(&events[..], [HostEvent::Warn(_), HostEvent::AcceptedMove(_, movement), HostEvent::InputCredits(_)] if movement.horizontal_collision)
        );
        let guest = &hub.guests[0];
        assert_eq!(guest.position, [24.5, 5.0, 20.5]);
        assert!(matches!(
            guest.inbox.front(),
            Some(InboxItem::Packet(S::MovePlayerRot(_), _))
        ));
    });
}

/// `isEntityCollidingWithAnythingNew`: a report that ends inside a solid entity the old box did
/// not overlap is refused even when it is too small to be moved wrongly, and for a creative
/// guest, which skips the distance test; a guest already overlapping it may move within it.
#[test]
#[ignore = "daily"]
fn a_report_ending_inside_a_new_solid_entity_is_refused_however_small_or_creative() {
    with_rig(|hub, ctx| {
        hub.move_solids = vec![shulker()];
        // 0.2 into the shulker's west face: under the distance threshold.
        let (wrongly, refused, _) = hub.collide(0, [21.9, 5.0, 20.5], ctx);
        assert!(
            !wrongly && refused,
            "a small penetration: {wrongly} {refused}"
        );

        hub.guests
            .update(0, |g| g.game_mode = crate::lan::guest::CREATIVE);
        let (wrongly, refused, _) = hub.collide(0, [23.0, 5.0, 20.5], ctx);
        assert!(!wrongly && refused, "creative: {wrongly} {refused}");
        hub.guests
            .update(0, |g| g.game_mode = crate::lan::guest::SURVIVAL);

        // Already overlapping it: a small move within it adds nothing new.
        hub.guests.update(0, |g| g.position = [22.3, 5.0, 20.5]);
        let (wrongly, refused, _) = hub.collide(0, [22.4, 5.0, 20.5], ctx);
        assert!(!wrongly && !refused, "already inside: {wrongly} {refused}");
    });
}

/// `HappyGhast.canBeCollidedWith`: on the server only an adult held still is solid to a player;
/// the feet-over-the-top landing is the client's.
#[test]
#[ignore = "daily"]
fn a_happy_ghast_is_solid_to_a_reported_move_only_while_held_still() {
    with_rig(|hub, ctx| {
        let ghast = |still| Candidate {
            still,
            ..Candidate::fixed(
                SolidKind::HappyGhast,
                502,
                Aabb::new(Vec3::new(22.0, 5.0, 19.0), Vec3::new(26.0, 9.0, 22.0)),
            )
        };
        // From over its top, straight down into it: the client's own walk lands on it, the
        // server lets a ghast that is not held still be fallen through.
        let above = [23.0, 9.0, 20.5];
        let into = [23.0, 5.0, 20.5];
        hub.guests.update(0, |g| {
            g.position = above;
            g.on_ground = false;
        });
        hub.move_solids = vec![ghast(false)];
        let (wrongly, refused, _) = hub.collide(0, into, ctx);
        assert!(!wrongly && !refused, "not held still: {wrongly} {refused}");
        hub.move_solids = vec![ghast(true)];
        let (_, refused, _) = hub.collide(0, into, ctx);
        assert!(refused, "held still, dropped into: {refused}");
        // From the side, held still, through it.
        hub.guests.update(0, |g| g.position = [20.5, 5.0, 20.5]);
        let (wrongly, refused, _) = hub.collide(0, [23.0, 5.0, 20.5], ctx);
        assert!(wrongly && refused, "held still: {wrongly} {refused}");
    });
}
