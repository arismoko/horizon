//! The writers `/tp` and `/rotate` share with a vehicle's riders: a vehicle
//! that is teleported takes its passengers along in the same operation, by the
//! vehicle's offset and the angles it was placed at (`Entity.teleport` ->
//! `teleportSameDimension` -> `calculatePassengerTransition`), and `/rotate`
//! turns an entity without teleporting it (`RotateCommand` ->
//! `Entity.forceSetRotation`).
use super::commands::Target;
use super::entity::{Entity as _, with_entity, with_entity_mut};
use crate::game::ServerGame;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
pub use sunrise_gameplay::teleport_transition::TeleportMotion;
pub(in crate::game) use sunrise_gameplay::teleport_transition::{Pose, Transition};
use sunrise_gameplay::teleport_transition::{absolute_pitch, is_horse};
pub(in crate::game) use sunrise_gameplay::teleport_transition::{
    cart_known_movement, passenger_destination, passenger_rotation,
};
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// `Entity.teleportCrossDimension`'s transitions for the general rows of one tree that left their level: the root takes `transition`, each rider its own vehicle's.
pub(in crate::game::server_side) fn transfer_general_tree(
    rows: &mut [super::general::GeneralRow],
    root: u32,
    transition: Transition,
    turn_riders: bool,
) {
    let mut work = vec![(root, transition)];
    while let Some((vehicle, transition)) = work.pop() {
        let Some(at) = rows.iter().position(|row| row.id == vehicle) else {
            continue;
        };
        let from = Pose {
            position: rows[at].position,
            rotation: rows[at].rotation,
        };
        let riders: Vec<_> = rows
            .iter()
            .filter(|row| row.mount == Some(vehicle))
            .map(|row| {
                let pose = Pose {
                    position: row.position,
                    rotation: row.rotation,
                };
                let mut next = transition.for_passenger(from, pose);
                next.rotation = next.rotation.filter(|_| turn_riders);
                (row.id, next)
            })
            .collect();
        let row = &mut rows[at];
        row.position = transition.position_for(row.position);
        row.previous = row.position;
        row.velocity = transition.velocity_for(row.velocity);
        if let Some(rotation) = transition.rotation_for(row.rotation) {
            row.rotation = rotation;
        }
        row.previous_rotation = row.rotation;
        work.extend(riders);
    }
}

/// The entity id a vehicle's riders point at, for the targets that carry some.
fn vehicle_id(t: Target) -> Option<u32> {
    match t {
        Target::Boat(id)
        | Target::Cart(id)
        | Target::Mob(id)
        | Target::General(id)
        | Target::Stand(id)
        | Target::Other(id) => Some(id),
        _ => None,
    }
}

impl ServerGame<'_> {
    /// `AbstractMinecart.getDismountLocationForPassenger` for a standing mob.
    pub(in crate::game::server_side) fn cart_dismount_point(
        &self,
        vehicle: u32,
        rider: u32,
    ) -> Option<Vec3> {
        self.cart_dismount_point_from(self.cart_object(vehicle)?, self.common.mob_object(rider)?)
    }

    pub(in crate::game::server_side) fn cart_dismount_mob(
        &self,
        vehicle: u32,
        rider: &sunrise_sim::mobs::Mob,
    ) -> Option<Vec3> {
        self.cart_dismount_point_from(self.cart_object(vehicle)?, rider)
    }

    /// The live tick snapshot may have moved beyond the cart's owning slot.
    pub(in crate::game) fn cart_dismount_point_from(
        &self,
        cart: &super::minecarts::Minecart,
        rider: &sunrise_sim::mobs::Mob,
    ) -> Option<Vec3> {
        use sunrise_core::geometry::Aabb;
        let direction =
            (f64::from(cart.yaw) / 90.0 + 0.5).floor() as i32 + if cart.flipped { 3 } else { 1 };
        let (fx, fz) = [(0, 1), (-1, 0), (0, -1), (1, 0)][(direction & 3) as usize];
        let (rx, rz) = (-fz, fx);
        let offsets = [
            (rx, rz),
            (-rx, -rz),
            (rx - fx, rz - fz),
            (-rx - fx, -rz - fz),
            (fx + rx, fz + rz),
            (fx - rx, fz - rz),
            (-fx, -fz),
            (fx, fz),
        ];
        let [bx, by, bz] =
            [cart.position.x, cart.position.y, cart.position.z].map(|v| v.floor() as i32);
        let half = f64::from(rider.width().min(1.0) / 2.0);
        for dy in [0, 1, -1] {
            for (dx, dz) in offsets {
                let p = [bx + dx, by + dy, bz + dz];
                let shape = self.block_collision_shape(p);
                let below = self.block_collision_shape([p[0], p[1] - 1, p[2]]);
                let floor = sunrise_core::geometry::block_floor_height(&shape, || &below);
                if !floor.is_infinite() && floor < 1.0 {
                    let at = Vec3::new(
                        f64::from(p[0]) + 0.5,
                        f64::from(p[1]) + floor,
                        f64::from(p[2]) + 0.5,
                    );
                    let bounds = Aabb::new(
                        Vec3::new(at.x - half, at.y, at.z - half),
                        Vec3::new(at.x + half, at.y + f64::from(rider.height()), at.z + half),
                    );
                    if self.can_dismount_to(bounds) {
                        return Some(at);
                    }
                }
            }
        }
        Some(Vec3::new(
            cart.position.x,
            cart.position.y + super::minecarts::HEIGHT,
            cart.position.z,
        ))
    }

    /// `AbstractHorse.getDismountLocationForPassenger`: try either side
    /// through the mount's height, then fall back to its feet.
    pub(in crate::game::server_side) fn horse_dismount_point(
        &self,
        vehicle: u32,
        rider: u32,
    ) -> Option<Vec3> {
        self.horse_dismount_mobs(
            self.common.mob_object(vehicle)?,
            self.common.mob_object(rider)?,
        )
    }

    /// The horse search for a mount and rider currently lent out of the mob list.
    pub(in crate::game::server_side) fn horse_dismount_mobs(
        &self,
        mount: &sunrise_sim::mobs::Mob,
        rider: &sunrise_sim::mobs::Mob,
    ) -> Option<Vec3> {
        use sunrise_core::geometry::{Aabb, trig};
        if !is_horse(mount.species()) {
            return None;
        }
        let turn = if rider.main_arm_left() { -90.0 } else { 90.0 };
        let reach =
            (f64::from(mount.width()) + f64::from(rider.width()) + f64::from(1e-5_f32)) / 2.0;
        let limit = mount.bounds().max_position().y + 0.75;
        for turn in [turn, -turn] {
            let radians = f64::from((mount.yaw + turn) * 0.017_453_292);
            let (dx, dz) = (-trig::sin(radians), trig::cos(radians));
            let scale = f64::from(dx.abs().max(dz.abs()));
            let x = mount.position.x + f64::from(dx) * reach / scale;
            let z = mount.position.z + f64::from(dz) * reach / scale;
            let mut y = mount.position.y.floor() as i32;
            loop {
                let p = [x.floor() as i32, y, z.floor() as i32];
                let shape = self.block_collision_shape(p);
                let below = self.block_collision_shape([p[0], p[1] - 1, p[2]]);
                let floor = sunrise_core::geometry::block_floor_height(&shape, || &below);
                if f64::from(y) + floor > limit {
                    break;
                }
                if !floor.is_infinite() && floor < 1.0 {
                    let at = Vec3::new(x, f64::from(y) + floor, z);
                    let half = f64::from(rider.width()) / 2.0;
                    let bounds = Aabb::new(
                        Vec3::new(x - half, at.y, z - half),
                        Vec3::new(x + half, at.y + f64::from(rider.height()), z + half),
                    );
                    if self.can_dismount_to(bounds) {
                        return Some(at);
                    }
                }
                y += 1;
                if f64::from(y) >= limit {
                    break;
                }
            }
        }
        Some(mount.position)
    }

    /// `Entity.teleportCrossDimension` ejects the riders before calculating
    /// their transitions. Dismounting also seats their own passengers again.
    fn prepare_vehicle_transition(&mut self, root: u32) {
        let riders = self.mob_passenger_ids(root);
        for &rider in riders.iter().rev() {
            self.dismount_mob(rider);
        }
        for rider in riders {
            self.prepare_vehicle_transition(rider);
            if let Some(m) = self.mob_object_mut(rider) {
                // Preserve the tree for serialization after all transitions
                // have used the positions left by ejectPassengers.
                m.extra.mount = Some(root);
                m.extra.passenger = true;
            }
        }
    }

    /// `Entity.teleportPassengers` on a mob dismounted during a transition.
    pub(in crate::game) fn snap_mob_passengers(&mut self, root: u32) {
        use sunrise_sim::mobs::riders;
        let Some(vehicle) = self.common.mob_object(root) else {
            return;
        };
        let (position, yaw, width, vehicle_box) = (
            vehicle.position,
            vehicle.yaw,
            vehicle.width(),
            vehicle.bounds(),
        );
        let children = self.seated_passengers(root);
        for (seat, &(_, id, player)) in children.iter().enumerate() {
            if player {
                continue;
            }
            let Some(m) = self.common.mob_object(id) else {
                continue;
            };
            let attachment = sunrise_gameplay_common::game::CommonGame::mob_passenger_attachment(
                self.common.mob_object(root).unwrap(),
                sunrise_sim::mobs::drops::is_animal(m.species()),
                seat,
                children.len(),
            );
            let offset = riders::vehicle_attachment(m, width);
            let m = self.mob_object_mut(id).unwrap();
            m.seated(root, offset, vehicle_box);
            let at = riders::seat(attachment, offset, position, yaw);
            sunrise_sim::mobs::living::teleports::snap_to(m, at);
        }
        for (_, child, player) in children {
            if !player {
                self.snap_mob_passengers(child);
            }
        }
    }

    /// `PositionMoveRotation.of(entity).deltaMovement()`: the motion a
    /// transition that keeps it (`TeleportMotion::Keep`) carries over, a
    /// minecart's clamped (`cart_known_movement`). None for a player, whose
    /// client holds its motion, and an entity no level holds.
    pub(in crate::game::server_side) fn transition_motion(&mut self, t: Target) -> Option<Vec3> {
        let id = sunrise_gameplay_common::game::commands_common::target_id(t)?;
        let motion = self
            .in_target_level(t, |g| {
                let h = g.native_walk_entity(id)?;
                Some(with_entity!(g, h, e => e.velocity()))
            })
            .flatten()?;
        Some(if matches!(t, Target::Cart(_)) {
            cart_known_movement(motion)
        } else {
            motion
        })
    }
    /// After a teleport placed `t` in its level: `motion` is its motion again
    /// (`Entity.teleportSetPosition` with `Relative.DELTA`).
    pub(in crate::game::server_side) fn restore_transition_motion(
        &mut self,
        t: Target,
        motion: Vec3,
    ) {
        let Some(id) = sunrise_gameplay_common::game::commands_common::target_id(t) else {
            return;
        };
        let _ = self.in_target_level(t, |g| {
            if let Some(h) = g.native_walk_entity(id) {
                with_entity_mut!(g, h, e => *e.velocity_mut() = motion);
            }
        });
    }

    /// The cross-level teleport of a command's or a mod's target: a general
    /// entity tree carries itself across (identity kept, join identity new)
    /// and always answers; an entity that changes level alone answers; a
    /// vehicle tree answers only when it moved. `None` leaves the target to
    /// the caller's player path.
    pub fn teleport_target_across(
        &mut self,
        target: Target,
        destination: super::Dim,
        at: [f64; 3],
        rotation: Option<[f32; 2]>,
        turn_riders: bool,
        motion: TeleportMotion,
    ) -> sunrise_game_data::Result<Option<bool>> {
        if let Target::General(id) = target {
            // Entity.teleport's stopRiding(): a targeted rider leaves its vehicle first.
            if self.common.dim != destination
                && self
                    .common
                    .general_row(id)
                    .is_some_and(|row| row.mount.is_some())
                && let Some(entity) = self.entity(id)
            {
                self.entity_stop_riding(entity);
            }
            let tree = self.vehicle_tree_ids(id);
            if tree.iter().all(|&id| {
                self.common.general_row(id).is_some()
                    && self.common.riding != Some(id)
                    && !self.ridden_by_guest(id)
            }) {
                return Ok(Some(self.teleport_general_tree_across(
                    id,
                    destination,
                    at,
                    rotation,
                    turn_riders,
                    motion,
                )?));
            }
            return self
                .teleport_vehicle_across(target, destination, at, rotation, turn_riders, motion)
                .map(Some);
        }
        if let Target::Stand(id) | Target::Other(id) = target
            && self.common.general_passengers(id).next().is_some()
        {
            return self
                .teleport_vehicle_across(target, destination, at, rotation, turn_riders, motion)
                .map(Some);
        }
        if let Some(moved) = self.teleport_lone_across(target, destination, at, rotation, motion)? {
            return Ok(Some(moved));
        }
        Ok(self
            .teleport_vehicle_across(target, destination, at, rotation, turn_riders, motion)?
            .then_some(true))
    }

    /// `Entity.teleportCrossDimension` for an entity that changes level alone
    /// (`lone_traveller_box`: an item, a primed TNT, a trident, a firework, an
    /// experience orb, an arrow or another saved projectile): saved, removed
    /// from its level, and recreated at `at` in `destination` under a new id,
    /// its stack, owner, thrower and portal cooldown kept (`restoreFrom`), its
    /// motion as `motion` says, turned to `rotation` when there is one. None
    /// for a target of another kind; Some(false) for one no level holds, one
    /// already in `destination`, one with no saved form, or one carrying mobs;
    /// Some(true) when it moved, or a mod canceled the travel
    /// (`entity_travel_to_dimension`) and it stayed.
    fn teleport_lone_across(
        &mut self,
        target: Target,
        destination: super::Dim,
        at: [f64; 3],
        rotation: Option<[f32; 2]>,
        motion: TeleportMotion,
    ) -> sunrise_game_data::Result<Option<bool>> {
        let (Target::Item(id)
        | Target::Tnt(id)
        | Target::Trident(id)
        | Target::Rocket(id)
        | Target::Orb(id)
        | Target::Projectile(id)
        | Target::Stand(id)
        | Target::Frame(id)
        | Target::Painting(id)
        | Target::Falling(id)
        | Target::Cushion(id)
        | Target::Other(id)
        | Target::Fangs(id)
        | Target::Crystal(id)
        | Target::Dragon(id)
        | Target::Spawner(id)
        | Target::Cloud(id)
        | Target::Bolt(id)) = target
        else {
            return Ok(None);
        };
        self.in_target_level(target, |game| {
            let source = game.dim;
            // Entity.teleportCrossDimension carries the riders; nothing rides
            // these kinds but a mob a server seats on a primed TNT.
            if source == destination
                || game.lone_traveller_box(id).is_none()
                || game.mobs.iter().any(|m| m.extra.mount == Some(id))
            {
                return Ok(Some(false));
            }
            if !game.entity_may_travel(id, destination, super::server_travel::TravelCause::Teleport)
            {
                return Ok(Some(true));
            }
            // Open the destination before removing the entity.
            game.enter_level(destination)?;
            game.enter_level(source)?;
            let cooldown = game
                .server
                .portals
                .entities
                .get(&id)
                .map_or(0, |s| s.cooldown);
            let Some((mut tag, state)) = game.take_lone_traveller(id) else {
                return Ok(Some(false));
            };
            game.native_leaves_flush();
            let [yaw, pitch] = sunrise_sim::entity_nbt::rotation(&tag);
            // PositionMoveRotation.calculateAbsolute: an axis the teleport
            // does not set keeps the entity's own, the pitch is clamped.
            let [yaw, pitch] = rotation.map_or([yaw, pitch], |[y, p]| {
                [
                    if y.is_finite() { y } else { yaw },
                    absolute_pitch(p).unwrap_or(pitch),
                ]
            });
            let kept = match motion {
                TeleportMotion::Keep => sunrise_sim::entity_nbt::motion(&tag),
                TeleportMotion::Stop => Vec3::ZERO,
            };
            let to = Vec3::new(at[0], at[1], at[2]);
            super::nether_portal::set_arrival_pose(&mut tag, to, kept, [yaw, pitch], 0.0);
            if matches!(
                target,
                Target::Frame(_) | Target::Painting(_) | Target::Cushion(_)
            ) && let Tag::Compound(c) = &mut tag
            {
                c.insert(
                    "block_pos".into(),
                    Tag::IntArray(at.map(|v| v.floor() as i32).to_vec()),
                );
            }
            if let Tag::Compound(c) = &mut tag {
                c.insert("PortalCooldown".into(), Tag::Int(cooldown));
            }
            // ServerLevel.addDuringTeleport: a fresh add, not loaded from disk.
            super::persist::mark_fresh(&mut tag);
            game.enter_level(destination)?;
            game.spawn_teleported_entity(Self::chunk_of(to), tag.into(), Some(state));
            game.enter_level(source)?;
            Ok(Some(true))
        })
        .unwrap_or(Ok(Some(false)))
    }

    /// `Entity.teleportCrossDimension`: carry a command's entire vehicle tree
    /// into the destination level, retaining UUIDs and the passengers' mounts,
    /// the root's motion as `motion` says.
    pub fn teleport_vehicle_across(
        &mut self,
        target: Target,
        destination: super::Dim,
        at: [f64; 3],
        rotation: Option<[f32; 2]>,
        turn_riders: bool,
        motion: TeleportMotion,
    ) -> sunrise_game_data::Result<bool> {
        let Some(root) = vehicle_id(target) else {
            return Ok(false);
        };
        self.in_target_level(target, |game| {
            let source = game.dim;
            if source == destination {
                return Ok(false);
            }
            // Refused passengers are dismounted and remain in the source level.
            if !game.tree_may_travel(root, destination) {
                return Ok(true);
            }
            // Open the destination before changing or removing any entity.
            game.enter_level(destination)?;
            game.enter_level(source)?;
            if game.general_transfer_conflicts(root, destination) {
                return Err(
                    "a travelling general entity UUID is already in the destination".into(),
                );
            }
            if game
                .mobs
                .iter()
                .any(|m| m.id == root && m.extra.mount.is_some())
            {
                game.dismount_mob(root);
            }
            if let Some(mount) = game.general_row(root).and_then(|row| row.mount) {
                game.dismount_general(root, mount);
                if game.general_row(root).is_none_or(|row| row.mount.is_some()) {
                    return Ok(false);
                }
            }
            game.prepare_vehicle_transition(root);
            let kept = match motion {
                TeleportMotion::Keep => game.transition_motion(target),
                TeleportMotion::Stop => None,
            };
            let from = if let Target::General(id) = target {
                let to = Vec3::new(at[0], at[1], at[2]);
                game.teleport_riders_first(
                    target,
                    to,
                    [false; 3],
                    rotation.filter(|_| turn_riders),
                );
                let row = game
                    .resident_entities
                    .general
                    .rows
                    .iter_mut()
                    .find(|row| row.id == id)
                    .ok_or("missing general vehicle")?;
                let before = row.position;
                row.position = to;
                row.previous = to;
                if let Some([yaw, pitch]) = rotation {
                    if yaw.is_finite() {
                        row.rotation[0] = yaw;
                    }
                    if let Some(pitch) = absolute_pitch(pitch) {
                        row.rotation[1] = pitch;
                    }
                }
                if kept.is_none() {
                    row.velocity = Vec3::ZERO;
                }
                Some([before.x, before.y, before.z])
            } else {
                game.place_teleport_in_level(target, at, rotation, [false; 3], turn_riders)
            };
            if let Some(from) = from
                && game.options.dedicated.is_some()
            {
                game.native_entity_teleport(game.dim.id(), root, from, at);
            }
            match kept {
                Some(kept) => game.restore_transition_motion(target, kept),
                None => game.teleport_stops(target),
            }
            let tree = game.vehicle_tree_ids(root);
            let tag = game
                .vehicle_tree_nbt_for(root, true)
                .ok_or("missing vehicle tree")?;
            let state = game
                .capture_vehicle_tree_state(root)
                .ok_or("missing vehicle object state")?;
            let mut players = Vec::new();
            // A command runs with its source player swapped out of the hub.
            if game.riding.is_some_and(|id| tree.contains(&id))
                && let (Some(conn), Some(player)) = (game.server.lan_acting, game.player.as_ref())
            {
                players.push((
                    conn,
                    [player.position.x, player.position.y, player.position.z],
                    [game.yaw, game.pitch],
                    game.riding.unwrap(),
                ));
            }
            if let Some(host) = game.server.lan.as_ref() {
                for guest in &host.hub.guests {
                    if Some(guest.conn) != game.server.lan_acting
                        && guest.vehicle.is_some_and(|id| tree.contains(&id))
                    {
                        players.push((
                            guest.conn,
                            guest.position,
                            [guest.yaw, guest.pitch],
                            guest.vehicle.unwrap(),
                        ));
                    }
                }
            }
            game.remove_vehicle_rows(&tree);
            for id in tree {
                game.detach_entity(id);
            }
            game.native_leaves_flush();
            game.enter_level(destination)?;
            // restoreFrom reloads saved state into fresh entities, rather than
            // retaining navigation paths or goals from the old level.
            let restored_objects = game
                .restore_vehicle_tree_state(tag, state)
                .ok_or("could not restore vehicle tree")?;
            // AbstractHorse.addPassenger snaps a reattached passenger to the
            // new horse's getViewYRot(0) / getViewXRot(0). A fresh living
            // entity's previous head yaw is still zero, before its first tick.
            let new_root = restored_objects.root;
            let mounts: Vec<_> = game.mob_objects().map(|m| (m.id, m.extra.mount)).collect();
            let restored = super::persist::vehicle_tree(new_root, &mounts);
            let horses: Vec<_> = game
                .mob_objects()
                .filter(|m| restored.contains(&m.id) && is_horse(m.species()))
                .map(|m| (m.id, m.head_pitch))
                .collect();
            for (horse, pitch) in horses {
                for id in game.mob_passenger_ids(horse) {
                    let Some(rider) = game.mob_object_mut(id) else {
                        continue;
                    };
                    rider.yaw = 0.0;
                    rider.previous_yaw = 0.0;
                    rider.head_pitch = pitch;
                    rider.previous_head_pitch = pitch;
                }
            }
            for (conn, at, rotation, mount) in players {
                game.server
                    .level_teleports
                    .push(super::server_travel::LevelTeleport {
                        conn,
                        dim: destination,
                        at,
                        rotation: Some(rotation),
                        vehicle: restored_objects.by_source.get(&mount).copied(),
                    });
            }
            game.native_joins_flush(false, 0);
            game.enter_level(source)?;
            Ok(true)
        })
        .unwrap_or(Ok(false))
    }

    /// Whether the vehicle `root` and the server players riding it (or what it
    /// carries) may all travel to `destination` by a teleport
    /// (`entity_travel_to_dimension`).
    fn tree_may_travel(&mut self, root: u32, destination: super::Dim) -> bool {
        self.entity_may_travel(
            root,
            destination,
            super::server_travel::TravelCause::Teleport,
        )
    }

    /// `BlockAttachedEntity.setPos`: a frame or painting moves to the block
    /// holding the position; its bounds follow from it.
    pub(in crate::game) fn teleport_hanging(&mut self, t: Target, v: Vec3) {
        let pos = [v.x.floor() as i32, v.y.floor() as i32, v.z.floor() as i32];
        match t {
            Target::Frame(id) => {
                if let Some(f) = self.common.item_frames.iter_mut().find(|f| f.id == id) {
                    f.pos = pos;
                }
            }
            Target::Painting(id) => {
                if let Some(p) = self.common.paintings.iter_mut().find(|p| p.id == id) {
                    p.pos = pos;
                }
            }
            _ => {}
        }
    }

    /// The riders of `t`, about to be placed at `to`, teleported before it; returns the riding players' connections.
    pub(in crate::game) fn teleport_riders_first(
        &mut self,
        t: Target,
        to: Vec3,
        relative: [bool; 3],
        rotation: Option<[f32; 2]>,
    ) -> Vec<u32> {
        let (Some(id), Some(at)) = (vehicle_id(t), self.target_position(t)) else {
            return Vec::new();
        };
        let from = Pose {
            position: Vec3::new(at[0], at[1], at[2]),
            rotation: self.target_rotation(t),
        };
        self.teleport_passengers(
            id,
            from,
            Transition::of_command(from, to, relative, rotation),
        )
    }

    /// `Entity.teleportSameDimension`'s walk of `vehicle`'s riders, deepest first; returns the players' connections.
    pub(in crate::game::server_side) fn teleport_passengers(
        &mut self,
        vehicle: u32,
        from: Pose,
        transition: Transition,
    ) -> Vec<u32> {
        struct Frame {
            id: u32,
            from: Pose,
            transition: Transition,
            riders: std::vec::IntoIter<u32>,
        }
        let mut riding = Vec::new();
        let mut stack = vec![Frame {
            id: vehicle,
            from,
            transition,
            riders: self.vehicle_passenger_ids(vehicle).into_iter(),
        }];
        while let Some(frame) = stack.last_mut() {
            if let Some(id) = frame.riders.next() {
                let Some(pose) = self.rider_pose(id) else {
                    continue;
                };
                let transition = frame.transition.for_passenger(frame.from, pose);
                stack.push(Frame {
                    id,
                    from: pose,
                    transition,
                    riders: self.vehicle_passenger_ids(id).into_iter(),
                });
                continue;
            }
            let Some(done) = stack.pop() else { break };
            self.teleport_riding_players(done.id, done.from, done.transition, &mut riding);
            if !stack.is_empty() {
                self.commit_rider(done.id, done.transition);
            }
        }
        riding
    }

    fn rider_pose(&self, id: u32) -> Option<Pose> {
        if let Some(m) = self.common.mob_object(id) {
            return Some(Pose {
                position: m.position,
                rotation: [m.yaw, m.head_pitch],
            });
        }
        let row = self
            .common
            .resident_entities
            .general
            .rows
            .iter()
            .find(|r| r.id == id)?;
        Some(Pose {
            position: row.position,
            rotation: row.rotation,
        })
    }

    /// `Entity.teleportSetPosition` of one non-player rider, then its section step.
    fn commit_rider(&mut self, id: u32, transition: Transition) {
        let mut carried = None;
        if let Some(m) = self.mob_object_mut(id) {
            let from = m.position;
            sunrise_sim::mobs::living::teleports::snap_to(m, transition.position_for(from));
            m.velocity = transition.velocity_for(m.velocity);
            carried = Some((from, m.position));
            if let Some([yaw, pitch]) = transition.rotation_for([m.yaw, m.head_pitch]) {
                m.yaw = yaw;
                m.body_yaw = yaw;
                m.head_yaw = yaw;
                m.head_pitch = pitch;
                m.previous_yaw = yaw;
                m.previous_head_pitch = pitch;
            }
        } else if let Some(row) = self
            .common
            .resident_entities
            .general
            .rows
            .iter_mut()
            .find(|row| row.id == id)
        {
            let from = row.position;
            row.position = transition.position_for(from);
            row.previous = row.position;
            row.velocity = transition.velocity_for(row.velocity);
            if let Some(rotation) = transition.rotation_for(row.rotation) {
                row.rotation = rotation;
            }
            row.previous_rotation = row.rotation;
            carried = Some((from, row.position));
        }
        if let Some((from, to)) = carried {
            self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
        }
    }

    /// The players seated on `vehicle` take `transition`: the acting body here (the put-back at the end of `guest_turn` publishes it), the others through the hub.
    fn teleport_riding_players(
        &mut self,
        vehicle: u32,
        from: Pose,
        transition: Transition,
        riding: &mut Vec<u32>,
    ) {
        let acting = self
            .server
            .lan_acting
            .filter(|_| self.common.player.is_some());
        let seated = self.common.riding;
        if seated == Some(vehicle)
            && let Some(p) = self.common.player.as_mut()
        {
            let before = p.position;
            let transition = transition.for_passenger(
                from,
                Pose {
                    position: before,
                    rotation: [self.common.yaw, self.common.pitch],
                },
            );
            p.position = transition.position_for(before);
            p.velocity = transition.velocity_for(p.velocity);
            p.teleported = true;
            p.teleported_delta = transition.delta;
            self.common.previous_position = p.position;
            let (to, own) = (p.position, self.own_entity_id());
            if let Ok(own) = u32::try_from(own) {
                self.entity_section_step(own, [before.x, before.y, before.z], [to.x, to.y, to.z]);
            }
            if let Some([yaw, pitch]) =
                transition.rotation_for([self.common.yaw, self.common.pitch])
            {
                self.common.yaw = yaw;
                self.common.pitch = pitch;
            }
        }
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let mut steps = Vec::new();
        for i in 0..host.hub.guests.len() {
            let conn = host.hub.guests[i].conn;
            let rides = if acting == Some(conn) {
                seated
            } else {
                host.hub.guests[i].vehicle
            };
            if rides != Some(vehicle) {
                continue;
            }
            riding.push(conn);
            if acting == Some(conn) {
                continue;
            }
            let [x, y, z] = host.hub.guests[i].position;
            let before = Vec3::new(x, y, z);
            let transition = transition.for_passenger(
                from,
                Pose {
                    position: before,
                    rotation: [host.hub.guests[i].yaw, host.hub.guests[i].pitch],
                },
            );
            if transition.rotation.is_some() {
                host.hub.guests.update(i, |g| {
                    if let Some([yaw, pitch]) = transition.rotation_for([g.yaw, g.pitch]) {
                        g.yaw = yaw;
                        g.head_yaw = yaw;
                        g.pitch = pitch;
                    }
                });
            }
            let id = host.hub.guests[i].entity_id as u32;
            let at = transition.position_for(before);
            host.hub.teleport_rider(
                i,
                [at.x, at.y, at.z],
                transition.delta,
                transition.rotation.is_none(),
            );
            host.hub.guests.update(i, |g| {
                if let Some(m) = g.motion.as_mut() {
                    m.velocity = transition.velocity_for(m.velocity);
                }
            });
            steps.push((id, before, at));
        }
        for (id, from, to) in steps {
            self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
        }
    }

    /// `Entity.sendTeleportTransitionToPlayers`: every player riding the tree hears the vehicle's written pose and motion.
    pub(in crate::game) fn announce_vehicle_teleport(&mut self, t: Target, riding: &[u32]) {
        let Some(id) = vehicle_id(t) else {
            return;
        };
        let (Some(at), [yaw, pitch]) = (self.target_position(t), self.target_rotation(t)) else {
            return;
        };
        let delta = self.transition_motion(t).unwrap_or(Vec3::ZERO);
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        for &conn in riding {
            if let Some(guest) = host.hub.guest(conn) {
                guest.send(&C::TeleportEntity(play::TeleportEntity {
                    id: id as i32,
                    position: at,
                    delta: [delta.x, delta.y, delta.z],
                    yaw,
                    pitch,
                    relatives: 0,
                    on_ground: false,
                }));
            }
        }
    }

    /// `Entity.forceSetRotation`: yaw, head yaw, pitch and their old values
    /// only. Position, motion, the vehicle and the riders stay.
    pub(in crate::game::server_side) fn set_rotation(&mut self, t: Target, rotation: [f32; 2]) {
        let _ = self.in_target_level(t, |g| g.set_rotation_in_level(t, rotation));
    }
    fn set_rotation_in_level(&mut self, t: Target, [yaw, pitch]: [f32; 2]) {
        let pitch = absolute_pitch(pitch);
        match t {
            Target::Guest(_) => {
                self.for_player(t, |g| {
                    g.set_rotation_in_level(Target::Player, [yaw, pitch.unwrap_or(f32::NAN)])
                });
            }
            Target::Player => {
                if yaw.is_finite() {
                    self.common.yaw = yaw;
                }
                if let Some(pitch) = pitch {
                    self.common.pitch = pitch;
                }
                // A server's player hears the turn alone when its turn ends.
                self.server.rotation_forced = self.server.lan_acting.is_some();
            }
            Target::Mob(id) => {
                if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                    if yaw.is_finite() {
                        m.yaw = yaw;
                        m.body_yaw = yaw;
                        m.head_yaw = yaw;
                    }
                    if let Some(pitch) = pitch {
                        m.head_pitch = pitch;
                    }
                    m.previous_yaw = m.yaw;
                    m.previous_head_pitch = m.head_pitch;
                }
            }
            Target::Other(id) => {
                if let Some(i) = self.server.others.0.iter().position(|e| e.id == id) {
                    self.server.others.0.update(i, |o| {
                        if yaw.is_finite() {
                            o.yaw = yaw;
                        }
                        if let Some(pitch) = pitch {
                            o.pitch = pitch;
                        }
                    });
                }
            }
            Target::Stand(id) => {
                if let Some(a) = self.common.armor_stands.iter_mut().find(|a| a.id == id) {
                    a.set_rotation([yaw, pitch.unwrap_or(a.pitch)]);
                }
            }
            Target::Boat(id) if yaw.is_finite() => {
                if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
                    self.common.boats.update(i, |b| {
                        b.yaw = yaw;
                        if let Some(pitch) = pitch {
                            b.pitch = pitch;
                        }
                        // forceSetRotation ends with setOldRot.
                        b.previous_yaw = b.yaw;
                    });
                }
            }
            Target::Cart(id) if yaw.is_finite() => {
                if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
                    self.common.carts.update(i, |c| {
                        c.yaw = yaw;
                        if let Some(pitch) = pitch {
                            c.pitch = pitch;
                        }
                        // forceSetRotation ends with setOldRot.
                        c.previous_yaw = c.yaw;
                        c.previous_pitch = c.pitch;
                    });
                }
            }
            Target::Fangs(id) if yaw.is_finite() => {
                if let Some(i) = self.common.fangs.list.iter().position(|e| e.id == id) {
                    self.common.fangs.list.update(i, |f| {
                        f.yaw = yaw;
                    });
                }
            }
            Target::Bolt(id) => {
                if let Some(i) = self.common.bolts.iter().position(|e| e.id == id) {
                    self.common.bolts.update(i, |b| {
                        if yaw.is_finite() {
                            b.rotation[0] = yaw;
                        }
                        if let Some(pitch) = pitch {
                            b.rotation[1] = pitch;
                        }
                    });
                }
            }
            _ => {}
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
