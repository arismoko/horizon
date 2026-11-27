// Shared state, helpers and mixed tests stay at the original LAN action scope.

pub const RECIPE_CHECK_COST: u32 = 1;
pub const RECIPE_CHECK_LIMIT: u32 = 10;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuOpen {
    Opened,
    Cancelled,
    Refused,
    NoProvider,
    Stale,
}

/// A guest's command waiting for the hub (`LanHost::commands`).
pub struct GuestCommand {
    pub conn: u32,
    pub command: String,
    pub permission: u8,
    /// An integrated server's: no level teleports (yet).
    pub integrated: bool,
    /// A mod's quiet command (game/network/lan_quiet.rs): its output suppressed, as
    /// `CommandSourceStack.withSuppressedOutput` does.
    pub quiet: bool,
}

/// Why a guest's turn didn't start (`as_guest_at`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnRefused {
    /// No guest has that `conn` at that place in the list, or it has no body.
    Gone,
    /// The guest is already taking a turn: its record holds an outer player's state.
    Nested,
}

/// `Player.isWithinEntityInteractionRange(entity, buffer)` for a guest: the
/// eyes closer than the entity interaction range (3, 5 in creative) plus
/// `buffer` to the entity's box.
pub fn guest_within_entity_range(
    g: &sunrise_sim::lan::guest::Guest,
    bounds: &sunrise_core::geometry::Aabb,
    buffer: f64,
) -> bool {
    let eye = [g.position[0], g.position[1] + g.eye_height(), g.position[2]];
    let (lo, hi) = (bounds.min_position(), bounds.max_position());
    let d = [
        eye[0] - eye[0].clamp(lo.x, hi.x),
        eye[1] - eye[1].clamp(lo.y, hi.y),
        eye[2] - eye[2].clamp(lo.z, hi.z),
    ];
    let range = if g.creative() { 5.0 } else { 3.0 } + buffer;
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < range * range
}
/// Kept at LAN action scope to preserve its published internals target path.
impl crate::game::CommonGame<'_> {
    pub fn vehicle_has_other_viewers(
        container: Option<(i32, i32, i32)>,
        guests: &[sunrise_sim::lan::guest::Guest],
        closing_conn: Option<u32>,
        key: (i32, i32, i32),
    ) -> bool {
        container == Some(key)
            || guests.iter().any(|g| {
                Some(g.conn) != closing_conn
                    && g.menu
                        .as_ref()
                        .is_some_and(|m| m.slots == Slots::Block(key))
            })
    }
}

/// ClientboundSwingAnimationPacket: the hand and the animation (its type and
/// base duration; each client applies the swinger's haste and fatigue).
pub fn swing_packet(
    id: i32,
    hand: sunrise_sim::first_person_sim::Hand,
    animation: sunrise_sim::first_person_sim::SwingAnimation,
) -> C {
    C::SwingAnimation(play::SwingAnimation {
        id,
        hand: i32::from(hand == sunrise_sim::first_person_sim::Hand::Off),
        kind: i32::from(animation.kind),
        duration: animation.duration,
    })
}

/// The block columns `Entity.isInWall` looks at: `AABB.ofSize` around the
/// eyes, 0.8 of the body's width across (`BlockPos.betweenClosedStream`).
pub fn eye_columns(at: [f64; 3], body: Dimensions) -> [std::ops::RangeInclusive<i32>; 2] {
    let half = f64::from(body.width * 0.8) / 2.0;
    let range = |v: f64| (v - half).floor() as i32..=(v + half).floor() as i32;
    [range(at[0]), range(at[2])]
}

/// ServerPlayer.swingAndResetAttackStrength resets only when a use starts a swing.
pub fn reset_use_attack_strength(ticker: &mut i32, before: u32, after: u32) {
    if before != after {
        *ticker = 0;
    }
}

/// A `ServerboundSelectBundleItemPacket` slot as the hub's `select_bundle_item` reads it.
/// With a crafting table or a station open (`own` slots of its own first), its
/// inventory part is the inventory menu's main inventory and hotbar from slot 9, in
/// the same order; its own slots hold no bundle the hub can reach.
pub fn bundle_inventory_slot(slot: usize, own: Option<usize>) -> Option<usize> {
    match own {
        Some(own) => slot.checked_sub(own).filter(|s| *s < 36).map(|s| s + 9),
        None => Some(slot),
    }
}

/// Where the server aims a guest's attack or interact on a mob. The packet names only the
/// entity, so the server picked the mob's centre; a slab, trapdoor, stair or fence between the
/// eyes and that centre then hid a mob the guest's own crosshair hit through the gap (#5772).
/// This returns the centre when it is visible, else the visible point of the box nearest to it (the one nearer the eyes when two are equally near)
/// (`occluded` tells whether a block outline hides a point from the eyes); the centre again
/// when the whole box is hidden.
pub fn visible_aim_point(
    eye: [f64; 3],
    lo: [f64; 3],
    hi: [f64; 3],
    occluded: impl Fn([f64; 3]) -> bool,
) -> [f64; 3] {
    let centre = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
    if !occluded(centre) {
        return centre;
    }
    let margin = [0, 1, 2].map(|k| ((hi[k] - lo[k]) / 2.0).min(0.02));
    let across = |k: usize, steps: usize| {
        (0..=steps)
            .map(|i| {
                lo[k] + margin[k] + (hi[k] - lo[k] - 2.0 * margin[k]) * i as f64 / steps as f64
            })
            .collect::<Vec<f64>>()
    };
    let (xs, ys, zs) = (across(0, 2), across(1, 4), across(2, 2));
    let mut candidates = Vec::with_capacity(xs.len() * ys.len() * zs.len());
    for &x in &xs {
        for &y in &ys {
            for &z in &zs {
                candidates.push([x, y, z]);
            }
        }
    }
    let nearness = |p: &[f64; 3]| (0..3).map(|k| (p[k] - centre[k]).powi(2)).sum::<f64>();
    let from_eye = |p: &[f64; 3]| (0..3).map(|k| (p[k] - eye[k]).powi(2)).sum::<f64>();
    candidates.sort_by(|a, b| {
        nearness(a)
            .total_cmp(&nearness(b))
            .then(from_eye(a).total_cmp(&from_eye(b)))
            .then(a[1].total_cmp(&b[1]))
            .then(a[0].total_cmp(&b[0]))
            .then(a[2].total_cmp(&b[2]))
    });
    candidates
        .into_iter()
        .find(|p| !occluded(*p))
        .unwrap_or(centre)
}

/// The first target `(id, removed, bounds)` whose box meets the swept box,
/// skipping removed ones and the vehicle the player rides.
pub fn first_spin_target(
    swept: Aabb,
    riding: Option<u32>,
    mut targets: impl Iterator<Item = (u32, bool, Aabb)>,
) -> Option<usize> {
    targets.position(|(id, removed, b)| !removed && Some(id) != riding && b.intersects(swept))
}

#[cfg(test)]
pub(in crate::game) mod respawn_world_point_tests {}

/// `Entity.touchingUnloadedChunk`: the box at `at` inflated by one block
/// reaches a chunk that is not loaded (`Level.hasChunksAt` over the floor of
/// its minimum and the ceiling of its maximum, as section coordinates).
pub fn touching_unloaded_chunk(
    at: [f64; 3],
    body: Dimensions,
    loaded: impl Fn(i32, i32) -> bool,
) -> bool {
    let half = f64::from(body.width) / 2.0 + 1.0;
    let section = |v: f64| (v as i32) >> 4;
    let (x0, x1) = (
        section((at[0] - half).floor()),
        section((at[0] + half).ceil()),
    );
    let (z0, z1) = (
        section((at[2] - half).floor()),
        section((at[2] + half).ceil()),
    );
    !(x0..=x1).all(|x| (z0..=z1).all(|z| loaded(x, z)))
}

#[cfg(any())]
#[cfg(any(test, feature = "test-support"))]
pub mod guest_close_listener_tests {
    use super::*;
    use sunrise_level::{containers::BlockEntity, sculk::Info};

    pub fn selected(game: &crate::game::ServerGame<'_>) -> Option<Info> {
        let Some(BlockEntity::Sculk(sensor)) = game.world.block_entities.get(&(9, 100, 4)) else {
            panic!("actual command-created sensor must exist");
        };
        sensor.vibration.selector.current.map(|(info, _)| info)
    }
    pub fn reset_listener(game: &mut crate::game::ServerGame<'_>) {
        assert!(game.run_command("setblock 9 100 4 air strict"));
        assert!(game.run_command("setblock 9 100 4 sculk_sensor strict"));
        game.sculk_prepare();
        game.sculk_sync_index();
        assert!(selected(game).is_none());
    }
    pub fn open(game: &mut crate::game::ServerGame<'_>, conn: u32, sequence: i32) -> i32 {
        game.lan_guest_packet(
            conn,
            S::UseItemOn(play::UseItemOn {
                hand: 0,
                pos: [6, 100, 4],
                face: 1,
                cursor: [0.5, 1.0, 0.5],
                inside: false,
                world_border: false,
                sequence,
            }),
        );
        let guest = game.side.lan.as_mut().unwrap().hub.guest(conn).unwrap();
        assert!(guest.ack_up_to.get() >= sequence);
        assert_eq!(guest.health, 20.0);
        assert_eq!(guest.position, [4.5, 100.0, 4.5]);
        let menu = guest
            .menu
            .as_ref()
            .expect("actual packet must open the chest menu");
        assert_eq!(menu.pos, Some((6, 100, 4)));
        assert_eq!(menu.size, 27);
        menu.window
    }
    pub fn close(game: &mut crate::game::ServerGame<'_>, conn: u32, window: i32) {
        game.lan_guest_packet(
            conn,
            S::ContainerClose(play::ContainerClose { container: window }),
        );
    }
}

#[cfg(test)]
#[path = "tests/aim_point_tests.rs"]
pub(in crate::game) mod aim_point_tests;

#[cfg(test)]
#[path = "tests/bundle_slot_tests.rs"]
pub(in crate::game) mod bundle_slot_tests;

#[cfg(test)]
#[path = "tests/spin_target_tests.rs"]
pub(in crate::game) mod spin_target_tests;

#[cfg(test)]
#[path = "tests/unloaded_chunk_tests.rs"]
pub(in crate::game) mod unloaded_chunk_tests;
