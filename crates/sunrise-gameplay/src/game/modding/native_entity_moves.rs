//! Moving and turning an entity for native server mods, through an
//! `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387):
//! teleport, collided moves, rotation, head and body yaw, facing, size and pushes. Writes move the
//! entity as `/tp` and `/rotate` do, so clients see it as they see those.
//! Reads return `None`, writes false, for a stale handle or on a client.
use super::entity::{Entity as _, EntityHandle, with_entity_mut};
use super::{
    Dim,
    commands::{Target, entity_pitch},
    vehicle_teleport::TeleportMotion,
};
use mod_api::{EntityRef, MoveCause};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::{collision_source::Mover, move_gate::MoveWho};
use sunrise_world::entity_move::MoverType;

/// `Level.isInSpawnableBounds` and `/tp`'s limits: 30,000,000 blocks out,
/// 20,000,000 up or down.
pub(in crate::game) fn in_bounds([x, y, z]: [f64; 3]) -> bool {
    (-30_000_000.0..30_000_000.0).contains(&x)
        && (-30_000_000.0..30_000_000.0).contains(&z)
        && (-20_000_000.0..20_000_000.0).contains(&y)
}

/// `Direction.fromYRot`: the horizontal direction a yaw faces.
pub(in crate::game::server_side) fn facing(yaw: f32) -> &'static str {
    const BY_2D: [&str; 4] = ["south", "west", "north", "east"];
    BY_2D[((f64::from(yaw) / 90.0 + 0.5).floor() as i32 & 3) as usize]
}

impl crate::game::ServerGame<'_> {
    /// Moves the entity to `pos` in its level (`Entity.teleportTo`), as
    /// `/tp` does: a player is placed there and told, a rider gets off. With
    /// `rotation` (yaw, pitch in degrees) it also turns. False for a position
    /// outside the world's bounds, for wall-mounted entities or dragon parts, or when the
    /// requested rotation cannot be stored.
    pub fn entity_teleport_to(
        &mut self,
        entity: EntityRef,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool {
        let Some(t) = self.teleport_target(entity, pos, rotation) else {
            return false;
        };
        self.teleport_to(t, pos, rotation, true);
        self.teleport_stops(t);
        true
    }
    /// Sends the entity to `pos` in the level `dimension` (`Entity.teleport`
    /// with a portal's transition, `TeleportMotion::Keep`): the level opens if
    /// it isn't, the entity keeps its motion, a player arrives after the
    /// current tick, a vehicle takes its passengers along. In the entity's own
    /// level it is `entity_teleport_to` with the motion kept. False for what
    /// `entity_teleport_to` refuses, a dimension that isn't registered, and
    /// kinds that cannot change level here (`teleport_target_across`).
    pub fn entity_teleport_to_level(
        &mut self,
        entity: EntityRef,
        dimension: &str,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool {
        let Some(t) = self.teleport_target(entity, pos, rotation) else {
            return false;
        };
        if !Dim::exists(dimension) {
            return false;
        }
        self.teleport_to_level(t, Dim::from_id(dimension), pos, rotation)
    }
    /// A mod sends `t` to `pos` in `level` (`Entity.teleport` with a
    /// `TeleportTransition` that keeps the motion, `TeleportMotion::Keep`): in
    /// its own level when that is `level`, else as `/tp` into another level
    /// does. False for a kind that only moves within its level (armor stands,
    /// falling blocks, ...; `teleport_target_across`), a player who is not a
    /// server guest, or a transfer the game could not make. A fake player
    /// stays, and the call is true.
    pub fn teleport_to_level(
        &mut self,
        t: Target,
        level: Dim,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool {
        if self.target_level(t) == level {
            let motion = self.transition_motion(t);
            self.teleport_to(t, pos, rotation, true);
            if let Some(motion) = motion {
                self.restore_transition_motion(t, motion);
            }
            return true;
        }
        self.teleport_across(t, level, pos, rotation, true, TeleportMotion::Keep)
            .unwrap_or(false)
    }
    /// What a teleport of `entity` to `pos` moves, when `/tp` could place it there.
    fn teleport_target(
        &self,
        entity: EntityRef,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> Option<Target> {
        let t = self.native_entity_target(entity)?;
        if !in_bounds(pos)
            || matches!(
                t,
                Target::Frame(_) | Target::Painting(_) | Target::DragonPart(_)
            )
            || rotation.is_some_and(|r| {
                !r.iter().all(|v| v.is_finite())
                    || !matches!(
                        t,
                        Target::Player
                            | Target::Guest(_)
                            | Target::Mob(_)
                            | Target::Other(_)
                            | Target::General(_)
                    )
            })
        {
            return None;
        }
        Some(t)
    }
    /// Moves the entity by `delta` as `Entity.move(cause, delta)` does: collided, stepping,
    /// through the `entity_move` hook. How far it moved; None for a stale handle, on a client,
    /// for an entity outside the entity walk, and for an unknown cause and for a delta that is
    /// not finite or beyond 64 blocks on an axis. A player is moved by `move_player_by`.
    pub fn entity_move_by(
        &mut self,
        entity: EntityRef,
        delta: [f64; 3],
        cause: MoveCause,
    ) -> Option<[f64; 3]> {
        if !self.common.is_logical_server() || !sunrise_sim::move_gate::valid_mod_move_delta(delta)
        {
            return None;
        }
        let mover = sunrise_sim::move_gate::mover_of(cause)?;
        let t = self.native_entity_target(entity)?;
        if matches!(t, Target::Player | Target::Guest(_)) {
            let conn = match t {
                Target::Player => self.server.lan_acting?,
                Target::Guest(conn) => conn,
                _ => return None,
            };
            // A teleport is made in the player's level, with that level's moving colliders.
            return self.in_target_level(t, |game| {
                game.in_this_levels_colliders(|game| game.move_player_by(t, conn, mover, delta))
            })?;
        }
        let handle = self.common.native_walk_entity(entity.id())?;
        let [x, y, z] = delta;
        let moved = self.move_listed_entity(handle, mover, Vec3::new(x, y, z));
        Some(moved.components())
    }
    /// Turns the entity to `yaw` and `pitch` in degrees (`Entity.setYRot`,
    /// `setXRot`): the pitch wraps and is clamped to -90..90. False for kinds
    /// whose rotation Sunrise does not fully store, or an unavailable player.
    /// Does not teleport or dismount.
    pub(in crate::game) fn entity_set_rotation(
        &mut self,
        entity: EntityRef,
        yaw: f32,
        pitch: f32,
    ) -> bool {
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        if !yaw.is_finite() || !pitch.is_finite() {
            return false;
        }
        let pitch = entity_pitch(pitch);
        match t {
            Target::Player | Target::Guest(_) => {
                self.for_player(t, |g| {
                    if g.player.is_none() {
                        return false;
                    }
                    g.yaw = yaw;
                    g.pitch = pitch;
                    true
                }) == Some(true)
            }
            Target::Mob(id) => {
                let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
                    return false;
                };
                m.yaw = yaw;
                m.head_pitch = pitch;
                m.needs_sync = true;
                true
            }
            Target::Other(id) => {
                let Some(i) = self.server.others.0.iter().position(|o| o.id == id) else {
                    return false;
                };
                self.server.others.0.update(i, |o| {
                    o.yaw = yaw;
                    o.pitch = pitch;
                });
                true
            }
            Target::General(id) => {
                let Some(row) = self.common.resident_entities.general.row_mut(id) else {
                    return false;
                };
                row.rotation = [yaw, pitch];
                true
            }
            _ => false,
        }
    }
    /// The yaw of the entity's head (`Entity.getYHeadRot`): a mob's or a
    /// player's own, zero for the kinds without a head.
    pub(in crate::game) fn entity_head_yaw(&self, entity: EntityRef) -> Option<f32> {
        let t = self.native_entity_target(entity)?;
        if let Some(g) = self.native_target_guest(t) {
            return Some(g.head_yaw);
        }
        match t {
            Target::Mob(id) => Some(self.common.mobs.iter().find(|m| m.id == id)?.head_yaw),
            Target::Stand(id) => Some(self.armor_stands.iter().find(|a| a.id == id)?.head_yaw),
            _ => Some(0.0),
        }
    }
    /// `LivingEntity.yBodyRot`: the living entity's authoritative body on a server.
    pub(in crate::game) fn entity_body_yaw(&self, entity: EntityRef) -> Option<f32> {
        match self.native_entity_target(entity)? {
            Target::Mob(id) => Some(self.common.mobs.iter().find(|m| m.id == id)?.body_yaw),
            Target::Stand(id) => Some(self.armor_stands.iter().find(|a| a.id == id)?.body_yaw),
            Target::Dragon(_) => Some(self.common.dragon.as_ref()?.body_yaw),
            t @ (Target::Player | Target::Guest(_)) => Some(self.native_player_body(t)?.body_yaw),
            _ => None,
        }
    }

    /// `LivingEntity.setYBodyRot`, including the vanilla subclass overrides.
    pub(in crate::game) fn entity_set_body_yaw(
        &mut self,
        entity: EntityRef,
        yaw: f32,
    ) -> Result<bool, mod_api::LevelWriteError> {
        Ok(match self.native_entity_target(entity) {
            Some(Target::Mob(id)) => {
                let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
                    return Ok(false);
                };
                use sunrise_sim::mobs::Species;
                // Silverfish and endermites call Entity.setYRot too. Only that
                // side effect rejects non-finite values, not the body assignment.
                if matches!(m.species(), Species::Silverfish | Species::Endermite)
                    && yaw.is_finite()
                {
                    m.yaw = yaw;
                }
                m.body_yaw = yaw;
                true
            }
            Some(Target::Stand(id)) => {
                let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id) else {
                    return Ok(false);
                };
                a.previous_yaw = yaw;
                a.previous_body_yaw = yaw;
                a.head_yaw = yaw;
                a.previous_head_yaw = yaw;
                true
            }
            Some(Target::Dragon(_)) => self.common.dragon.as_mut().is_some_and(|d| {
                d.body_yaw = yaw;
                true
            }),
            Some(t @ (Target::Player | Target::Guest(_))) => {
                self.native_player_body_mut(t).is_some_and(|p| {
                    p.body_yaw = yaw;
                    true
                })
            }
            _ => false,
        })
    }
    /// Turns a mob's head to `yaw` (`Entity.setYHeadRot`); its body follows
    /// as its AI turns it. Players' heads can turn too; false for other kinds
    /// or an unavailable player.
    pub fn entity_set_head_yaw(&mut self, entity: EntityRef, yaw: f32) -> bool {
        if !yaw.is_finite() {
            return false;
        }
        match self.native_entity_target(entity) {
            Some(Target::Mob(id)) => match self.common.mobs.iter_mut().find(|m| m.id == id) {
                Some(m) => {
                    m.head_yaw = yaw;
                    m.needs_sync = true;
                    true
                }
                None => false,
            },
            Some(t @ (Target::Player | Target::Guest(_))) => {
                let conn = match t {
                    Target::Player => self.server.lan_acting,
                    Target::Guest(conn) => Some(conn),
                    _ => None,
                };
                conn.and_then(|conn| {
                    self.server
                        .lan
                        .as_mut()?
                        .hub
                        .update_guest(conn, |g| g.head_yaw = yaw)
                })
                .is_some()
            }
            _ => false,
        }
    }
    /// The horizontal direction the entity faces (`Entity.getDirection`):
    /// `north`, `south`, `east` or `west`.
    pub(in crate::game) fn entity_direction(&self, entity: EntityRef) -> Option<&'static str> {
        let t = self.native_entity_target(entity)?;
        Some(facing(self.target_rotation(t)[0]))
    }
    /// The entity's width and height (`Entity.getBbWidth`, `getBbHeight`):
    /// its box's, so a baby's or a scaled entity's own.
    pub(in crate::game) fn entity_dimensions(&self, entity: EntityRef) -> Option<[f64; 2]> {
        let [min, max] = self.entity_bounding_box(entity)?;
        Some([max[0] - min[0], max[1] - min[1]])
    }
    /// Adds `delta` to the entity's velocity (`Entity.push`,
    /// `addDeltaMovement`). False for an unavailable player, an entity
    /// Sunrise doesn't simulate, or a value that isn't finite.
    pub(in crate::game) fn entity_push(&mut self, entity: EntityRef, delta: [f64; 3]) -> bool {
        if !delta.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        if matches!(target, Target::Player | Target::Guest(_)) {
            let [x, y, z] = delta;
            return self.write_player_velocity(target, |before| {
                let velocity = before.add_vector(Vec3::new(x, y, z));
                velocity.is_finite().then_some(velocity)
            });
        }
        let Some(h) = self.common.native_walk_entity(entity.id()) else {
            return false;
        };
        let [x, y, z] = delta;
        with_entity_mut!(self, h, e => {
            let v = e.velocity_mut();
            *v = Vec3::new(v.x + x, v.y + y, v.z + z);
        });
        if let EntityHandle::Mob(i) = h {
            self.common.mobs[i].needs_sync = true;
        }
        true
    }
    /// Update the resolved player's body, in place for its own turn or in its guest record.
    /// The resolver already excludes displaced outer players. Send to the owning client only.
    pub(in crate::game::server_side) fn write_player_velocity(
        &mut self,
        target: Target,
        change: impl FnOnce(Vec3) -> Option<Vec3>,
    ) -> bool {
        let Some((conn, motion)) = self.player_motion_mut(target) else {
            return false;
        };
        let Some(velocity) = change(motion.velocity) else {
            return false;
        };
        motion.velocity = velocity;
        self.send_guest_motion(conn, Some(velocity));
        true
    }

    /// Put back a player's own random after a copy of it drew for them.
    pub(in crate::game::server_side) fn write_player_random(
        &mut self,
        target: Target,
        random: sunrise_core::random::EntityRandom,
    ) {
        if let Some((_, motion)) = self.player_motion_mut(target) {
            motion.random = random;
        }
    }

    /// The body of the acting player or a guest, with its connection, during or outside a turn.
    fn player_motion_mut(
        &mut self,
        target: Target,
    ) -> Option<(u32, &mut sunrise_sim::motion::PlayerMotion)> {
        let conn = match target {
            Target::Player => self.server.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => None,
        }?;
        let host = self.server.lan.as_mut()?;
        let motion = if target == Target::Player {
            host.hub.guest(conn).and(self.common.player.as_mut())
        } else {
            host.hub.guests.motion_mut(conn)
        }?;
        Some((conn, motion))
    }

    /// Moves a player by `delta` through the level (`Entity.move`) after the `entity_move` hook
    /// has heard it, and tells its client with a relative teleport (`Hub::displace`). The
    /// displacement made, or `None` (see the host trait for what is refused).
    fn move_player_by(
        &mut self,
        t: Target,
        conn: u32,
        mover: MoverType,
        delta: [f64; 3],
    ) -> Option<[f64; 3]> {
        let acting = t == Target::Player;
        {
            let guest = self.server.lan.as_ref()?.hub.guest(conn)?;
            if guest.vehicle.is_some() || guest.sleeping {
                return None;
            }
        }
        // The hook hears the move once, as `Entity.move` begins; a cancel is a move of nothing.
        let who = if acting {
            MoveWho::Player
        } else {
            MoveWho::Guest(conn)
        };
        let gate = self.entity_move(who, mover, Vec3::new(delta[0], delta[1], delta[2]));
        if gate.skipped() {
            return Some([0.0; 3]);
        }
        let delta = gate.delta().components();
        let host = self.server.lan.as_ref()?;
        let guest = host.hub.guest(conn)?;
        let body = if acting {
            self.common.player.as_ref()
        } else {
            guest.motion.as_deref()
        }?;
        let from = [body.position.x, body.position.y, body.position.z];
        let to = [from[0] + delta[0], from[1] + delta[1], from[2] + delta[2]];
        if !in_bounds(to) {
            return None;
        }
        // A spectator goes through everything, as its moves are not checked.
        let reached = if guest.spectator() {
            to
        } else {
            // The solid entities (boats, shulkers, a still happy ghast) the player may meet.
            let entities = self.entity_collisions(Mover::PlayerAt { feet_y: from[1] });
            self.server.lan.as_ref()?.hub.settle_conn(
                conn,
                from,
                to,
                &self.common.world,
                self.common.collision,
                &entities,
            )?
        };
        let target = Vec3::new(reached[0], reached[1], reached[2]);
        if acting {
            self.common.player.as_mut()?.position = target;
        } else {
            let host = self.server.lan.as_mut()?;
            host.hub.update_guest(conn, |g| {
                g.motion.as_deref_mut().map(|m| m.position = target)
            })??;
        }
        let by = [
            reached[0] - from[0],
            reached[1] - from[1],
            reached[2] - from[2],
        ];
        self.server.lan.as_mut()?.hub.displace(conn, reached, by);
        Some(by)
    }
}

use sunrise_gameplay_common::game::collision_source::CollisionSourceGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::move_gate::MoveGateGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_motion_values::NativeEntityMotionValuesGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    #[test]
    #[ignore = "daily"]
    fn moves_and_pushes_a_live_mob() {
        use crate::game::native_entity_refs::tests::{fixture_resident_column, with_game};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;

        with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut mob = Mob::new(12, kind, Vec3::ZERO, 0.0);
            mob.ai.nav.path = Some(sunrise_sim::mobs::path::Path {
                nodes: Vec::new(),
                next: 0,
                target: [1, 64, 1],
                dist_to_target: 0.0,
                reached: true,
            });
            mob.extra.mount = Some(42);
            mob.extra.passenger = true;
            mob.fall_distance = 5.0;
            mob.velocity.y = -2.0;
            fixture_resident_column(&mut game.world, (0, 0));
            game.mobs.push(mob);
            let entity = game.entity(12).unwrap();
            fixture_resident_column(&mut game.world, (0, -1));
            assert!(entity.teleport_to(game, [5.0, 70.0, -3.0], Some([90.0, 120.0])));
            let mob = &game.mobs[0];
            assert!(mob.ai.nav.path.is_none());
            assert!(mob.extra.mount.is_none());
            assert!(!mob.extra.passenger);
            assert_eq!(mob.fall_distance, 5.0);
            assert_eq!(mob.velocity.y, 0.0);
            assert_eq!(entity.position(game), Some([5.0, 70.0, -3.0]));
            assert_eq!(entity.rotation(game), Some([90.0, 90.0]));
            assert_eq!(entity.head_yaw(game), Some(90.0));
            let before = entity.velocity(game).unwrap();
            assert!(entity.set_rotation(game, 15.0, 360.0));
            assert_eq!(entity.rotation(game), Some([15.0, 0.0]));
            assert_eq!(entity.head_yaw(game), Some(90.0));
            assert_eq!(entity.position(game), Some([5.0, 70.0, -3.0]));
            assert_eq!(entity.velocity(game), Some(before));
            assert!(entity.teleport_to(game, [5.0, 70.0, -3.0], Some([0.0, 450.0])));
            assert_eq!(entity.rotation(game), Some([0.0, 90.0]));
            assert!(entity.push(game, [1.0, 2.0, -1.0]));
            assert_eq!(
                entity.velocity(game),
                Some([before[0] + 1.0, before[1] + 2.0, before[2] - 1.0])
            );
            assert!(!entity.teleport_to(game, [f64::NAN, 0.0, 0.0], None));
            assert!(!entity.push(game, [0.0, f64::INFINITY, 0.0]));
            game.armor_stands
                .push(sunrise_sim::entities::ArmorStand::new(13, Vec3::ZERO, 0.0));
            let stand = game.entity(13).unwrap();
            assert!(!stand.set_rotation(game, 30.0, 10.0));
            game.server
                .others
                .0
                .push(super::super::commands::OtherEntity {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    knot_check: None,
                    eye: None,
                    join: Default::default(),
                    display: None,
                    interaction: None,
                    id: 14,
                    kind: "minecraft:marker".into(),
                    position: Vec3::ZERO,
                    yaw: 60.0,
                    pitch: 0.0,
                });
            let marker = game.entity(14).unwrap();
            assert_eq!(marker.head_yaw(game), Some(0.0));
            assert!(!marker.set_head_yaw(game, 10.0));
            game.paintings.push(sunrise_sim::entities::Painting {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 15,
                pos: [0, 64, 0],
                facing: 0,
                variant: "minecraft:kebab".into(),
                asset: None,
                width: 1,
                height: 1,
                check_interval: 0,
                ticks: 0,
            });
            let painting = game.entity(15).unwrap();
            assert!(!painting.teleport_to(game, [1.0, 64.0, 0.0], None));
            assert!(!painting.set_rotation(game, 30.0, 10.0));
        });
    }

    #[test]
    fn a_yaw_faces_vanillas_direction() {
        assert_eq!(super::facing(0.0), "south");
        assert_eq!(super::facing(90.0), "west");
        assert_eq!(super::facing(180.0), "north");
        assert_eq!(super::facing(-90.0), "east");
        assert_eq!(super::facing(-45.0), "south");
        assert_eq!(super::facing(-46.0), "east");
        assert_eq!(super::facing(720.0 + 170.0), "north");
    }

    #[test]
    fn positions_stay_inside_the_worlds_bounds() {
        assert!(super::in_bounds([29_999_999.0, -64.0, 0.0]));
        assert!(super::in_bounds([
            -30_000_000.0,
            -20_000_000.0,
            -30_000_000.0
        ]));
        assert!(!super::in_bounds([30_000_000.0, 64.0, 0.0]));
        assert!(!super::in_bounds([0.0, 20_000_001.0, 0.0]));
        assert!(!super::in_bounds([f64::NAN, 64.0, 0.0]));
    }
    /// Independent vanilla outcomes for spawn bounds and the yaw-to-direction conversion.
    #[test]
    fn move_boundaries_match_vanilla() {
        run_move_boundaries_match_vanilla();
    }

    pub(in crate::game::server_side) fn run_move_boundaries_match_vanilla() {
        use serde_json::Value;
        let cases = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/native_entity_moves.jsonl"
        ));
        let expected = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/native_entity_moves_expected.jsonl"
        ));
        let rows = |text: &str| {
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>()
        };
        let (cases, expected) = (rows(cases), rows(expected));
        assert_eq!(cases.len(), expected.len());
        assert!(cases.len() >= 20);
        for (input, expected) in cases.iter().zip(&expected) {
            assert_eq!(input["name"], expected["name"]);
            let pos = std::array::from_fn(|i| input["pos"][i].as_f64().unwrap());
            let yaw = input["yaw"].as_f64().unwrap() as f32;
            assert_eq!(
                super::in_bounds(pos),
                expected["spawnable"].as_bool().unwrap(),
                "{}",
                input["name"]
            );
            assert_eq!(
                super::facing(yaw),
                expected["direction"].as_str().unwrap(),
                "{}",
                input["name"]
            );
        }
    }
}
