use super::*;
use crate::motion::{MotionWorld, PlayerMotion};
use crate::pose::{Pose, PoseFlags, desired_pose};
use sunrise_core::geometry::Vec3;

/// The poses `Player.getDesiredPose` reads: each flag alone, a spin and a glide while
/// swimming (swimming comes first), a crouch while flying (none) and in bed.
const CASES: [PoseFlags; 8] = [
    PoseFlags {
        sleeping: false,
        swimming: false,
        fall_flying: false,
        auto_spin_attack: false,
        shift_key_down: false,
        flying: false,
    },
    PoseFlags {
        sleeping: true,
        swimming: false,
        fall_flying: false,
        auto_spin_attack: false,
        shift_key_down: true,
        flying: false,
    },
    PoseFlags {
        sleeping: false,
        swimming: true,
        fall_flying: true,
        auto_spin_attack: true,
        shift_key_down: false,
        flying: false,
    },
    PoseFlags {
        sleeping: false,
        swimming: false,
        fall_flying: true,
        auto_spin_attack: true,
        shift_key_down: false,
        flying: false,
    },
    PoseFlags {
        sleeping: false,
        swimming: false,
        fall_flying: false,
        auto_spin_attack: true,
        shift_key_down: true,
        flying: false,
    },
    PoseFlags {
        sleeping: false,
        swimming: false,
        fall_flying: false,
        auto_spin_attack: false,
        shift_key_down: true,
        flying: false,
    },
    PoseFlags {
        sleeping: false,
        swimming: false,
        fall_flying: false,
        auto_spin_attack: false,
        shift_key_down: true,
        flying: true,
    },
    PoseFlags {
        sleeping: false,
        swimming: false,
        fall_flying: false,
        auto_spin_attack: false,
        shift_key_down: false,
        flying: true,
    },
];

/// The pose `updatePlayerPose` leaves `player` in for `flags`, under an open sky where every
/// pose fits: in the client's prediction (`update_player_pose`) or in the server's copy of a
/// player (`server_player_pose_tick`).
fn updated_pose(
    v: &crate::test_fixtures::Vanilla,
    scene: &mut crate::world::World,
    player: i32,
    flags: PoseFlags,
    server: bool,
) -> Pose {
    let (assets, binding, resources) = (&v.assets, &v.collision, &v.motion);
    let mut body = PlayerMotion::new(resources, Vec3::new(20.5, 10.0, 20.5)).unwrap();
    body.entity_id = player;
    body.sleeping = flags.sleeping;
    body.swimming = flags.swimming;
    body.fall_flying = flags.fall_flying;
    body.auto_spin_attack_ticks = if flags.auto_spin_attack { 2 } else { 0 };
    body.shift_key_down = flags.shift_key_down;
    body.flying = flags.flying;
    let mut w = MotionWorld {
        side: crate::motion::dry_side(),
        tag_items: None,
        block_registries: None,
        loop_context: crate::test_resources::server_loop_context(),
        damage_hooks: &crate::health::DamageHooks::NONE,
        entities: &[],
        player: &mut body,
        scene,
        assets,
        collision_binding: binding,
        resources,
        health: &resources.health,
    };
    if server {
        w.server_player_pose_tick().unwrap();
    } else {
        w.update_player_pose().unwrap();
    }
    body.pose
}

/// The pose the others are sent for a player (`player_state`'s `DATA_POSE`): none while it
/// is standing.
fn sent_pose(hub: &Hub, player: i32, flags: PoseFlags, body: BodyFlags) -> Option<Value> {
    let state = hub.player_state(
        player,
        0,
        [0.0; 3],
        (0.0, 0.0, 0.0),
        true,
        (flags.shift_key_down, false, flags.sleeping),
        body,
        0,
        None,
        &hub.guests[0].info,
        20.0,
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    state
        .meta
        .iter()
        .find_map(|(index, _, value)| (*index == 6).then_some(value.clone()))
}

/// A body before its first tick, as `guest_body` reads it for `flags`.
fn no_body(flags: PoseFlags) -> BodyFlags {
    BodyFlags {
        swimming: flags.swimming,
        gliding: flags.fall_flying,
        spin_attack: flags.auto_spin_attack,
        flying: flags.flying,
        ..Default::default()
    }
}

/// `Player.getDesiredPose` is answered in one place,
/// [`desired_pose`](crate::pose::desired_pose): `updatePlayerPose` takes its answer in the
/// client's prediction and in the server's copy of a player, in vanilla's order. The others
/// are sent the pose the body took (`getPose`), whatever the flags say, and before the body
/// exists the pose it would ask for: a crouch while flying is no crouch.
#[test]
#[ignore = "daily"]
fn update_player_pose_takes_the_desired_pose_and_the_others_see_the_bodys() {
    let v = crate::test_fixtures::vanilla().expect("prepared resources required");
    let mut scene = crate::world::World::test_scene_with_placements(&v.assets, &[], 2).unwrap();
    let (hub, _rx) = hub_with_a_guest();
    let entity = hub.guests[0].entity_id;
    let mut seen = Vec::new();
    for flags in CASES {
        let desired = desired_pose(entity, flags);
        seen.push(desired);
        for server in [false, true] {
            let pose = updated_pose(v, &mut scene, entity, flags, server);
            assert_eq!(pose, desired, "{flags:?}, on the server: {server}");
        }
        let body = BodyFlags {
            pose: Some(Pose::Swimming),
            ..no_body(flags)
        };
        assert_eq!(
            sent_pose(&hub, entity, flags, body),
            Some(Value::Id(Pose::Swimming.id())),
            "the body's pose, {flags:?}"
        );
    }
    assert_eq!(
        seen,
        [
            Pose::Standing,
            Pose::Sleeping,
            Pose::Swimming,
            Pose::FallFlying,
            Pose::SpinAttack,
            Pose::Crouching,
            Pose::Standing,
            Pose::Standing,
        ]
    );
    let crouch = |flying| PoseFlags {
        shift_key_down: true,
        flying,
        ..PoseFlags::default()
    };
    assert_eq!(
        sent_pose(&hub, entity, crouch(false), no_body(crouch(false))),
        Some(Value::Id(Pose::Crouching.id()))
    );
    assert_eq!(
        sent_pose(&hub, entity, crouch(true), no_body(crouch(true))),
        None,
        "a flying player holding shift stands"
    );
}

/// A mod's row that changes `getDesiredPose`'s answer changes the pose everywhere it is
/// taken: with the answer replaced on this thread (`rule_replacements::DESIRED_POSE`) by
/// swimming, a standing player crawls under an open sky in the client's prediction and on the
/// server, and the others are sent it before its body exists.
#[test]
#[ignore = "daily"]
fn a_replaced_desired_pose_reaches_every_pose_update() {
    use crate::rule_replacements::{DESIRED_POSE, replaced};
    let v = crate::test_fixtures::vanilla().expect("prepared resources required");
    let mut scene = crate::world::World::test_scene_with_placements(&v.assets, &[], 2).unwrap();
    let (hub, _rx) = hub_with_a_guest();
    let entity = hub.guests[0].entity_id;
    let standing = PoseFlags::default();
    let poses = replaced(
        &DESIRED_POSE,
        |_, _| Pose::Swimming,
        || {
            [
                updated_pose(v, &mut scene, entity, standing, false),
                updated_pose(v, &mut scene, entity, standing, true),
            ]
        },
    );
    assert_eq!(poses, [Pose::Swimming; 2]);
    let sent = replaced(
        &DESIRED_POSE,
        |_, _| Pose::Swimming,
        || sent_pose(&hub, entity, standing, no_body(standing)),
    );
    assert_eq!(sent, Some(Value::Id(Pose::Swimming.id())));
    assert_eq!(
        updated_pose(v, &mut scene, entity, standing, false),
        Pose::Standing
    );
}
