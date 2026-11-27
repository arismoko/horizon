//! An entity's state flags for native server mods, through an `EntityRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): sneaking and
//! sprinting, water and lava, fire and air, silence, invisibility and pose.
//! Players and mobs have them all; the other kinds have the ones Sunrise
//! keeps for them (an armor stand's fire and invisibility, an item's fire),
//! and `None` for the rest, as for a stale handle. Secondary use also reads
//! the local client's live semantic sneak state; fire and underwater reads also
//! use the client's own cached state.
//! Ticks lived (`Entity.tickCount`) are read for players, mobs, items, orbs and minecarts.
use super::commands::Target;
use mod_api::EntityRef;
use sunrise_gameplay_common::game::native_entity_state_common::Body;
use sunrise_sim::mobs::sync;
use sunrise_sim::pose::Pose;

/// `Pose`'s serialized names by id (`Pose.STANDING` is 0).
pub(in crate::game) const POSES: [&str; 18] = [
    "standing",
    "fall_flying",
    "sleeping",
    "swimming",
    "spin_attack",
    "crouching",
    "long_jumping",
    "dying",
    "croaking",
    "using_tongue",
    "sitting",
    "roaring",
    "sniffing",
    "emerging",
    "digging",
    "sliding",
    "shooting",
    "inhaling",
];

/// A player's pose as `Pose` names it.
fn player_pose(pose: Pose) -> &'static str {
    match pose {
        Pose::Standing => "standing",
        Pose::Crouching => "crouching",
        Pose::Swimming => "swimming",
        Pose::FallFlying => "fall_flying",
        Pose::SpinAttack => "spin_attack",
        Pose::Sleeping => "sleeping",
    }
}

impl crate::game::ServerGame<'_> {
    pub fn entity_is_sitting(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Mob(mob) => mob.sitting(),
            Body::Player(_) | Body::Other(_) => None,
        }
    }
    /// The player's body or the mob a handle names, or its target.
    fn native_entity_body(&self, entity: EntityRef) -> Option<Body<'_>> {
        self.native_target_body(self.native_entity_target(entity)?)
    }

    pub(in crate::game) fn server_entity_is_shift_key_down(
        &self,
        entity: EntityRef,
    ) -> Option<bool> {
        Some(match self.native_entity_body(entity)? {
            Body::Player(p) => p.shift_key_down,
            _ => false,
        })
    }

    pub(in crate::game) fn server_entity_main_arm(
        &self,
        entity: EntityRef,
    ) -> Option<mod_api::MainHand> {
        let (target, location) = self.native_entity_target_with_location(entity)?;
        let left = match target {
            Target::Player => return self.guest_main_arm(self.server.lan_acting?),
            Target::Guest(conn) => return self.guest_main_arm(conn),
            Target::Mob(_) => {
                self.common.mobs[self.common.mobs.locate(location?.join)?].main_arm_left()
            }
            Target::Stand(_) | Target::Dragon(_) => false,
            _ => return None,
        };
        Some(if left {
            mod_api::MainHand::Left
        } else {
            mod_api::MainHand::Right
        })
    }
    /// A server's player's main arm, from the Main Hand its client last reported.
    fn guest_main_arm(&self, conn: u32) -> Option<mod_api::MainHand> {
        Some(super::lan::main_hand(
            self.command_guest(conn)?.info.main_hand,
        ))
    }
    /// Whether the entity crouches (`Entity.isCrouching`: the crouching
    /// pose).
    pub(in crate::game) fn entity_is_crouching(&self, entity: EntityRef) -> Option<bool> {
        if !self.common.is_logical_server()
            && self.native_entity_read_target(entity)? == Target::Player
        {
            return Some(self.common.player.as_ref()?.local_crouching);
        }
        Some(self.entity_pose(entity)? == "crouching")
    }
    /// Whether the entity sprints (`Entity.isSprinting`): a player, or a
    /// cat or ocelot dashing after its target (their shared sprinting flag);
    /// false for the other kinds.
    pub fn entity_is_sprinting(&self, entity: EntityRef) -> Option<bool> {
        Some(
            match self.native_target_body(self.native_entity_read_target(entity)?)? {
                Body::Player(p) => p.sprinting,
                Body::Mob(m) => m.extra.sprinting,
                _ => false,
            },
        )
    }
    /// `Entity.setSprinting` for a player (the flag and the speed modifier) or a mob (the
    /// flag, sent to the clients). False for other kinds, a stale handle and a client.
    pub fn entity_set_sprinting(&mut self, entity: EntityRef, sprinting: bool) -> bool {
        if !self.common.is_logical_server() {
            return false;
        }
        match self.native_entity_target(entity) {
            // The acting body: a guest's turn writes its flag back from the guest row, so the
            // row (`Guest::sprinting`, what metadata publishes) is set with it.
            Some(Target::Player) => {
                let Some(player) = self.common.player.as_mut() else {
                    return false;
                };
                if player.set_sprinting(sprinting).is_err() {
                    return false;
                }
                if let (Some(&conn), Some(lan)) =
                    (self.server.lan_guest_swaps.last(), self.server.lan.as_mut())
                {
                    lan.hub
                        .update_guest(conn, |guest| guest.sprinting = sprinting);
                }
                true
            }
            // A parked guest: its row and, with it, the body's flag and speed modifier.
            Some(Target::Guest(conn)) => self
                .server
                .lan
                .as_mut()
                .and_then(|lan| {
                    lan.hub.update_guest(conn, |guest| {
                        let body = guest.motion.as_mut()?;
                        body.set_sprinting(sprinting).ok()?;
                        guest.sprinting = sprinting;
                        Some(())
                    })
                })
                .flatten()
                .is_some(),
            Some(Target::Mob(id)) => self
                .common
                .mobs
                .iter_mut()
                .find(|m| m.id == id)
                .is_some_and(|m| {
                    m.extra.sprinting = sprinting;
                    m.needs_sync = true;
                    true
                }),
            _ => false,
        }
    }
    /// Whether the entity is in lava (`Entity.isInLava`): players and mobs.
    pub(in crate::game) fn entity_is_in_lava(&self, entity: EntityRef) -> Option<bool> {
        match self.native_target_body(self.native_entity_read_target(entity)?)? {
            Body::Player(p) => Some(p.is_in_lava()),
            Body::Mob(m) => Some(m.fluids.in_lava),
            Body::Other(Target::General(id)) => Some(
                self.common
                    .resident_entities
                    .general
                    .row(id)?
                    .fluids
                    .in_lava,
            ),
            Body::Other(_) => None,
        }
    }

    pub(in crate::game) fn entity_set_fire_immunity(
        &mut self,
        entity: EntityRef,
        immune: bool,
    ) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.set_fire_immunity_for(target, immune))
    }

    fn set_fire_immunity_for(&mut self, target: Target, immune: bool) -> bool {
        let server = self.common.is_logical_server();
        let (grant, ticks) = match target {
            Target::Mob(id) => {
                let Some(mob) = self
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == id && !m.removed)
                else {
                    return false;
                };
                mob.needs_sync |= server && mob.mod_fire_immune != immune;
                (&mut mob.mod_fire_immune, &mut mob.fire_ticks)
            }
            Target::Stand(id) => {
                let Some(stand) = self.common.armor_stands.iter_mut().find(|s| s.id == id) else {
                    return false;
                };
                (&mut stand.mod_fire_immune, &mut stand.fire_ticks)
            }
            Target::Player | Target::Guest(_) => {
                let Some(player) = self.native_player_body_mut(target) else {
                    return false;
                };
                (&mut player.mod_fire_immune, &mut player.fire_ticks)
            }
            _ => return false,
        };
        *grant = immune;
        // Remote mobs and stands keep the server's fire bit in this field. Mask it
        // while rendering, but keep the copy so revocation needs no new packet.
        if immune && (server || matches!(target, Target::Player)) {
            *ticks = (*ticks).min(0);
        }
        true
    }

    /// The ticks the entity burns for (`Entity.getRemainingFireTicks`), for every kind.
    pub(in crate::game) fn entity_fire_ticks(&self, entity: EntityRef) -> Option<i32> {
        let target = self.native_entity_target(entity)?;
        Some(self.read_entity_base(target)?.base.fire_ticks)
    }
    /// `Entity.getPortalCooldown`: a player's own portal state, the falling
    /// block's, a pearl's End gateway state, or the level's for the other
    /// kinds its portal walk carries (zero when it keeps no state for one).
    pub(in crate::game) fn entity_portal_cooldown(&self, entity: EntityRef) -> Option<i32> {
        let id = match self.native_entity_target(entity)? {
            // The acting guest's state is in place for its turn (the entry of `guest_turn`).
            Target::Player => return Some(self.common.portal_player.cooldown),
            Target::Guest(conn) => {
                let play = self.server.lan.as_ref()?.play.get(&conn);
                return Some(play.map_or(0, |p| p.portal.cooldown));
            }
            Target::Falling(id) => {
                return Some(
                    self.common
                        .falling
                        .iter()
                        .find(|f| f.id == id)?
                        .portal
                        .cooldown,
                );
            }
            Target::Projectile(id) => {
                let p = self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)?;
                if matches!(p.kind, sunrise_sim::projectiles::ProjectileKind::EnderPearl) {
                    return Some(self.pearl_portal_cooldown(id));
                }
                if !super::nether_portal::is_arrow(p)
                    && !super::nether_portal::is_portal_projectile(p)
                {
                    return None;
                }
                id
            }
            Target::Mob(id)
            | Target::Item(id)
            | Target::Orb(id)
            | Target::Boat(id)
            | Target::Cart(id)
            | Target::Tnt(id)
            | Target::Rocket(id)
            | Target::Trident(id) => id,
            _ => return None,
        };
        Some(
            self.server
                .portals
                .entities
                .get(&id)
                .map_or(0, |p| p.cooldown),
        )
    }
    /// `Entity.setPortalCooldown`: writes the value `entity_portal_cooldown`
    /// reads, where that read keeps it — the level's portal state for the
    /// kinds its portal walk carries (mobs, items, experience orbs, boats,
    /// minecarts, primed TNT, fireworks, tridents, arrows and the thrown
    /// items and fireballs a chunk saves). Server-only; a negative value or a
    /// client-side write is refused. A positive value blocks portal entry on
    /// the next tick (and is refilled while the entity stands in a portal);
    /// 0 lets it enter at once. The write refuses a player, an acting guest,
    /// a falling block and an ender pearl: their cooldown lives in a store of
    /// its own (the player and guest portal states, the falling block's saved
    /// field, an End gateway traveller) that this write does not own, so it
    /// needs its own owner decision before it can reach them. False for a kind
    /// the read refuses or a stale handle.
    pub(in crate::game) fn entity_set_portal_cooldown(
        &mut self,
        entity: EntityRef,
        ticks: i32,
    ) -> bool {
        if ticks < 0 || !self.common.is_logical_server() {
            return false;
        }
        match self.native_entity_target(entity) {
            Some(Target::Projectile(id)) => {
                let Some(projectile) = self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)
                else {
                    return false;
                };
                if matches!(
                    projectile.kind,
                    sunrise_sim::projectiles::ProjectileKind::EnderPearl
                ) || (!super::nether_portal::is_arrow(projectile)
                    && !super::nether_portal::is_portal_projectile(projectile))
                {
                    return false;
                }
                self.server.portals.entities.entry(id).or_default().cooldown = ticks;
                true
            }
            Some(Target::Mob(id))
            | Some(Target::Item(id))
            | Some(Target::Orb(id))
            | Some(Target::Boat(id))
            | Some(Target::Cart(id))
            | Some(Target::Tnt(id))
            | Some(Target::Rocket(id))
            | Some(Target::Trident(id)) => {
                self.server.portals.entities.entry(id).or_default().cooldown = ticks;
                true
            }
            _ => false,
        }
    }
    /// Sets the remaining fire ticks (`Entity.setRemainingFireTicks`), for every kind.
    pub(in crate::game) fn entity_set_remaining_fire_ticks(
        &mut self,
        entity: EntityRef,
        ticks: i32,
    ) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.update_entity_base(target, |b| b.fire_ticks = ticks))
    }
    /// Clears positive fire ticks while retaining fire immunity (`Entity.clearFire`).
    pub(in crate::game) fn entity_clear_fire(&mut self, entity: EntityRef) -> bool {
        let Some(ticks) = self.entity_fire_ticks(entity) else {
            return false;
        };
        self.entity_set_remaining_fire_ticks(entity, ticks.min(0))
    }
    /// Sets the entity's air (`Entity.setAirSupply`).
    pub(in crate::game) fn entity_set_air_supply(&mut self, entity: EntityRef, air: i32) -> bool {
        match self.native_entity_target(entity) {
            Some(Target::Player) => match self.common.player.as_mut() {
                Some(player) => {
                    player.air = air;
                    true
                }
                None => false,
            },
            Some(Target::Guest(conn)) => match self
                .server
                .lan
                .as_mut()
                .and_then(|lan| lan.hub.guests.motion_mut(conn))
            {
                Some(player) => {
                    player.air = air;
                    true
                }
                None => false,
            },
            Some(Target::Mob(id)) => match self.common.mobs.iter_mut().find(|mob| mob.id == id) {
                Some(mob) => {
                    mob.air = air;
                    mob.needs_sync = true;
                    true
                }
                None => false,
            },
            _ => false,
        }
    }
    /// Whether the entity makes no sounds (`Entity.isSilent`, its `Silent` tag), for every kind.
    pub(in crate::game) fn entity_is_silent(&self, entity: EntityRef) -> Option<bool> {
        let target = self.native_entity_target(entity)?;
        Some(self.read_entity_base(target)?.base.silent)
    }
    /// Silences the entity or lets it make sounds again (`Entity.setSilent`), for every kind.
    pub(in crate::game) fn entity_set_silent(&mut self, entity: EntityRef, silent: bool) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.update_entity_base(target, |b| b.silent = silent))
    }
    /// Whether the entity's model is hidden (`Entity.isInvisible`): the
    /// invisibility effect on a player or a mob, a mob's or an armor stand's
    /// flag.
    pub fn entity_is_invisible(&self, entity: EntityRef) -> Option<bool> {
        match self.native_entity_target(entity)? {
            Target::Player => Some(self.common.potions.effects.get("invisibility").is_some()),
            Target::Guest(conn) => Some(
                self.command_guest(conn)?
                    .effects
                    .get("invisibility")
                    .is_some(),
            ),
            Target::Mob(id) => {
                let m = self.common.mobs.iter().find(|m| m.id == id)?;
                Some(m.is_invisible())
            }
            Target::Stand(id) => Some(
                self.common
                    .armor_stands
                    .iter()
                    .find(|s| s.id == id)?
                    .invisible,
            ),
            Target::Frame(id) => Some(
                self.common
                    .item_frames
                    .iter()
                    .find(|f| f.id == id)?
                    .invisible,
            ),
            _ => None,
        }
    }
    /// Sets the entity's frozen ticks (`Entity.setTicksFrozen`).
    pub(in crate::game) fn entity_set_ticks_frozen(
        &mut self,
        entity: EntityRef,
        ticks: i32,
    ) -> bool {
        match self.native_entity_target(entity) {
            Some(Target::Player) => self.common.player.as_mut().is_some_and(|p| {
                p.ticks_frozen = ticks;
                true
            }),
            Some(Target::Guest(conn)) => self
                .server
                .lan
                .as_mut()
                .and_then(|lan| lan.hub.guests.motion_mut(conn))
                .is_some_and(|p| {
                    p.ticks_frozen = ticks;
                    true
                }),
            Some(Target::Mob(id)) => self
                .common
                .mobs
                .iter_mut()
                .find(|m| m.id == id)
                .is_some_and(|m| {
                    m.ticks_frozen = ticks;
                    m.needs_sync = true;
                    true
                }),
            _ => false,
        }
    }
    /// The persistent glowing tag (`Entity.hasGlowingTag`), for every kind.
    pub(in crate::game) fn entity_glowing(&self, entity: EntityRef) -> Option<bool> {
        let target = self.native_entity_target(entity)?;
        Some(self.read_entity_base(target)?.base.glowing)
    }
    /// The no-gravity flag (`Entity.isNoGravity`), for every kind.
    pub(in crate::game) fn entity_no_gravity(&self, entity: EntityRef) -> Option<bool> {
        let target = self.native_entity_target(entity)?;
        Some(self.read_entity_base(target)?.base.no_gravity)
    }
    /// Returns the entity's frozen ticks (`Entity.getTicksFrozen`).
    pub(in crate::game) fn entity_ticks_frozen(&self, entity: EntityRef) -> Option<i32> {
        match self.native_entity_body(entity)? {
            Body::Player(p) => Some(p.ticks_frozen),
            Body::Mob(m) => Some(m.ticks_frozen),
            _ => None,
        }
    }
    /// Sets the persistent glowing tag (`Entity.setGlowingTag`), for every kind.
    pub(in crate::game) fn entity_set_glowing(&mut self, entity: EntityRef, glowing: bool) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.update_entity_base(target, |b| b.glowing = glowing))
    }
    /// Sets the no-gravity flag (`Entity.setNoGravity`), for every kind.
    pub(in crate::game) fn entity_set_no_gravity(
        &mut self,
        entity: EntityRef,
        no_gravity: bool,
    ) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.update_entity_base(target, |b| b.no_gravity = no_gravity))
    }
    /// Sets the entity's shared invisibility flag (`Entity.setInvisible`) on a mob, an armor
    /// stand or an item frame. Returns false for players, guests and other kinds.
    pub fn entity_set_invisible(&mut self, entity: EntityRef, invisible: bool) -> bool {
        match self.native_entity_target(entity) {
            Some(Target::Mob(id)) => self
                .common
                .mobs
                .iter_mut()
                .find(|m| m.id == id)
                .is_some_and(|m| {
                    m.invisible = invisible;
                    m.needs_sync = true;
                    true
                }),
            Some(Target::Stand(id)) => self
                .common
                .armor_stands
                .iter_mut()
                .find(|s| s.id == id)
                .is_some_and(|s| {
                    s.invisible = invisible;
                    true
                }),
            // `ItemFrame` saves the flag with its chunk.
            Some(Target::Frame(id)) => {
                let Some(frame) = self.common.item_frames.iter_mut().find(|f| f.id == id) else {
                    return false;
                };
                frame.invisible = invisible;
                let pos = frame.pos;
                self.common
                    .world
                    .modified
                    .insert((pos[0] >> 4, pos[2] >> 4));
                true
            }
            _ => false,
        }
    }
    /// The entity's pose (`Entity.getPose`) as `Pose` names it: `standing`,
    /// `crouching`, `swimming`, `sleeping`, a warden's `roaring`... Kinds
    /// with no pose stand.
    pub(in crate::game) fn entity_pose(&self, entity: EntityRef) -> Option<&'static str> {
        Some(
            match self.native_target_body(self.native_entity_read_target(entity)?)? {
                Body::Player(p) => player_pose(p.pose),
                Body::Mob(m) if &*m.kind.id == "minecraft:mannequin" => {
                    player_pose(m.extra.avatar_pose)
                }
                Body::Mob(m) => POSES.get(usize::from(sync::pose_id(m))).copied()?,
                Body::Other(_) => POSES[0],
            },
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use super::*;
    #[cfg(test)]
    use std::ops::ControlFlow;
    #[cfg(test)]
    use sunrise_gameplay_common::game::native_entity_state_common::{
        eye_fluid, max_air, visit_touched_fluids,
    };
    #[cfg(test)]
    use sunrise_sim::mobs::{Mob, Species};
    use sunrise_sim::motion::PlayerMotion;

    #[cfg(test)]
    /// The reads' order: highest first, water before lava in a tie, an untouched fluid left
    /// out even with a height, and nothing for an entity in neither.
    #[test]
    fn fluid_reads_list_the_touched_fluids_highest_first() {
        let (water, lava) = ("minecraft:water", "minecraft:lava");
        let touched_fluids = |water, lava| {
            let mut fluids = Vec::new();
            let result = visit_touched_fluids(water, lava, &mut |id, depth| {
                fluids.push((id.to_owned(), depth));
                ControlFlow::Continue(())
            });
            assert_eq!(result, ControlFlow::Continue(()));
            fluids
        };
        assert_eq!(
            touched_fluids((0.5, true), (1.25, true)),
            [(lava.to_owned(), 1.25), (water.to_owned(), 0.5)]
        );
        assert_eq!(
            touched_fluids((0.75, true), (0.75, true)),
            [(water.to_owned(), 0.75), (lava.to_owned(), 0.75)]
        );
        assert_eq!(
            touched_fluids((0.4, true), (0.2, false)),
            [(water.to_owned(), 0.4)]
        );
        assert!(touched_fluids((0.0, false), (0.0, false)).is_empty());

        // A break visits only the first contact. Total order treats +0.0 as higher than -0.0.
        let mut first = Vec::new();
        assert_eq!(
            visit_touched_fluids((-0.0, true), (0.0, true), &mut |id, depth| {
                first.push((id.to_owned(), depth.to_bits()));
                ControlFlow::Break(())
            }),
            ControlFlow::Break(())
        );
        assert_eq!(first, [(lava.to_owned(), 0.0_f64.to_bits())]);
        assert_eq!(eye_fluid(true, true), Some(water));
        assert_eq!(eye_fluid(false, true), Some(lava));
        assert_eq!(eye_fluid(false, false), None);
    }

    #[cfg(test)]
    #[test]
    fn poses_keep_vanillas_ids() {
        assert_eq!(POSES[usize::from(sync::SITTING)], "sitting");
        assert_eq!(POSES[usize::from(sync::INHALING)], "inhaling");
        for pose in [
            Pose::Standing,
            Pose::Crouching,
            Pose::Swimming,
            Pose::FallFlying,
            Pose::SpinAttack,
            Pose::Sleeping,
        ] {
            let id = POSES.iter().position(|p| *p == player_pose(pose));
            assert_eq!(id.map(|i| Pose::from_id(i as i32)), Some(pose));
        }
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn mob_setters_update_stored_state() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_core::geometry::Vec3;
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(42, kind, Vec3::ZERO, 0.0));
            let entity = game.entity(42).unwrap();
            assert!(entity.set_air_supply(game, -7));
            assert_eq!(game.mobs[0].air, -7);
            assert!(entity.set_remaining_fire_ticks(game, 37));
            assert_eq!(game.mobs[0].fire_ticks, 37);
            assert!(entity.clear_fire(game));
            assert_eq!(game.mobs[0].fire_ticks, 0);
            assert!(game.entity_set_remaining_fire_ticks(entity, -12));
            assert!(game.entity_clear_fire(entity));
            assert_eq!(game.mobs[0].fire_ticks, -12);
            assert!(entity.set_ticks_frozen(game, 31));
            assert_eq!(game.mobs[0].ticks_frozen, 31);
            assert!(entity.set_invisible(game, true));
            assert!(game.mobs[0].invisible);
            game.mobs[0].add_effect("minecraft:invisibility", 100, 0);
            assert!(entity.set_invisible(game, false));
            assert_eq!(entity.is_invisible(game), Some(false));
            assert!(entity.set_glowing(game, true));
            assert!(game.mobs[0].glowing);
            assert!(entity.set_no_gravity(game, true));
            assert!(game.mobs[0].no_gravity);
            assert!(game.mobs[0].needs_sync);
        });
    }

    #[cfg(test)]
    /// `Entity.setNoGravity` and `Entity.isNoGravity` reach a live dropped item, not only a
    /// mob; a kind that is neither still refuses both, as does a stale handle (#5011, #3938
    /// item 47).
    #[test]
    #[ignore = "daily"]
    fn no_gravity_reads_and_writes_a_live_dropped_item() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use mod_api::WorldEpoch;
        use sunrise_core::geometry::Vec3;
        use sunrise_items::items::ItemStack;
        use sunrise_sim::entities::ArmorStand;

        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            let item = game.entity(game.dropped[0].id).unwrap();
            assert_eq!(item.no_gravity(game), Some(false));

            assert!(item.set_no_gravity(game, true));
            assert!(game.dropped[0].no_gravity);
            assert!(game.dropped[0].needs_sync);
            assert_eq!(item.no_gravity(game), Some(true));

            // Every kind keeps the flag (#5082): a stand too.
            game.armor_stands.push(ArmorStand::new(43, Vec3::ZERO, 0.0));
            let stand = game.entity(43).unwrap();
            assert_eq!(stand.no_gravity(game), Some(false));
            assert!(stand.set_no_gravity(game, true));
            assert_eq!(stand.no_gravity(game), Some(true));

            let stale = EntityRef::new(WorldEpoch::new(item.epoch().get() + 1), item.id());
            assert_eq!(stale.no_gravity(game), None);
            assert!(!stale.set_no_gravity(game, true));
        });
    }

    pub fn with_player(test: impl FnOnce(&mut crate::game::ServerGame<'_>, EntityRef)) {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_core::geometry::Vec3;
        with_resident_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            test(game, player);
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn a_granted_player_does_not_burn_or_take_damage_in_lava() {
        use std::sync::Arc;
        use sunrise_core::geometry::Vec3;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::motion::MotionWorld;

        with_player(|game, player| {
            // `EntityFluidInteraction.update` needs the chunks around the box loaded as well.
            for chunk in [(-1, -1), (-1, 0), (0, -1), (0, 0)] {
                game.world
                    .chunks
                    .insert(chunk, Arc::new(Chunk::empty(chunk.0, chunk.1)));
            }
            let lava = game
                .pack
                .assets
                .registry
                .default_state("minecraft:lava")
                .unwrap();
            assert!(game.world.set_stored(0, 64, 0, lava));
            game.player.as_mut().unwrap().position = Vec3::new(0.5, 64.0, 0.5);
            let tick_lava = |game: &mut crate::game::ServerGame<'_>| {
                let side = game_side!(game);
                let mut world = MotionWorld {
                    side: &side,
                    entities: &[],
                    player: game.common.player.as_mut().unwrap(),
                    scene: &mut game.common.world,
                    assets: &game.common.pack.assets,
                    collision_binding: game.common.collision,
                    resources: game.common.motion,
                    health: &game.common.health_resources,
                    tag_items: None,
                    loop_context: &game.common.loop_context,
                    damage_hooks: &game.common.damage_hooks,
                    block_registries: None,
                };
                world.fluid_base_tick().unwrap();
                assert!(world.player.is_in_lava());
                world.tick_environment_damage().unwrap();
            };
            let health =
                |game: &crate::game::ServerGame<'_>| game.player.as_ref().unwrap().health.health();
            let before = health(game);
            assert_eq!(player.fire_immune(game), Some(false));
            assert!(player.set_fire_immunity(game, true));
            assert_eq!(player.fire_immune(game), Some(true));
            game.hurt_player_by("minecraft:on_fire", 2.0);
            assert_eq!(health(game), before, "direct fire damage is also refused");
            tick_lava(game);
            assert_eq!(player.is_on_fire(game), Some(false));
            assert_eq!(health(game), before);

            assert!(player.set_fire_immunity(game, false));
            tick_lava(game);
            assert_eq!(player.is_on_fire(game), Some(true));
            assert!(health(game) < before);
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn fire_ticks_reads_every_kind_and_refuses_a_stale_handle() {
        use mod_api::WorldEpoch;
        use sunrise_core::geometry::Vec3;
        use sunrise_items::items::ItemStack;
        use sunrise_sim::entities::ArmorStand;

        with_player(|game, player| {
            game.player.as_mut().unwrap().fire_ticks = 0;
            assert_eq!(player.fire_ticks(game), Some(0));
            game.player.as_mut().unwrap().fire_ticks = -12;
            assert_eq!(player.fire_ticks(game), Some(-12));

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(42, kind, Vec3::ZERO, 0.0));
            game.mobs[0].fire_ticks = 37;
            let mob = game.entity(42).unwrap();
            assert_eq!(mob.fire_ticks(game), Some(37));

            game.armor_stands.push(ArmorStand::new(43, Vec3::ZERO, 0.0));
            game.armor_stands[0].fire_ticks = 23;
            let stand = game.entity(43).unwrap();
            assert_eq!(stand.fire_ticks(game), Some(23));

            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            game.dropped[0].fire_ticks = 19;
            let item = game.entity(game.dropped[0].id).unwrap();
            assert_eq!(item.fire_ticks(game), Some(19));

            let mut snowball = sunrise_sim::projectiles::Projectile::new(
                sunrise_sim::projectiles::ProjectileKind::Snowball,
                Vec3::ZERO,
                None,
            );
            snowball.id = 45;
            snowball.fire_ticks = 159;
            game.creatures.projectiles.push(snowball);
            let proj = game.entity(45).unwrap();
            assert_eq!(proj.fire_ticks(game), Some(159));
            assert_eq!(proj.is_on_fire(game), Some(true));
            assert!(proj.set_remaining_fire_ticks(game, 40));
            assert_eq!(proj.fire_ticks(game), Some(40));

            let mut arrow = sunrise_sim::projectiles::Projectile::new(
                sunrise_sim::projectiles::ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::ZERO,
                None,
            );
            arrow.id = 46;
            arrow.arrow.as_mut().unwrap().fire_ticks = 80;
            game.creatures.projectiles.push(arrow);
            let arrow_ref = game.entity(46).unwrap();
            assert_eq!(arrow_ref.fire_ticks(game), Some(80));
            assert_eq!(arrow_ref.is_on_fire(game), Some(true));
            assert!(arrow_ref.clear_fire(game));
            assert_eq!(arrow_ref.fire_ticks(game), Some(0));

            game.boats.push(super::super::boats_common::Boat::new(
                44,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            ));
            let boat = game.entity(44).unwrap();
            // A boat keeps the base state every `Entity` has (#5082).
            assert_eq!(boat.fire_ticks(game), Some(0));
            assert!(boat.set_remaining_fire_ticks(game, 5));
            assert_eq!(boat.fire_ticks(game), Some(5));
            let stale = EntityRef::new(WorldEpoch::new(mob.epoch().get() + 1), mob.id());
            assert_eq!(stale.fire_ticks(game), None);
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_blocks_entry_and_counts_down() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use std::sync::Arc;
        use sunrise_core::geometry::Vec3;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::motion::PlayerMotion;

        with_resident_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.server.settings.simulation_distance = 4;
            game.server.settings.render_distance = 12;
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let portal = registry.default_state("minecraft:nether_portal").unwrap();
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5)).unwrap());
            for z in [8, 12] {
                game.world.set(0, 63, z, stone).unwrap();
            }
            game.world.set(0, 64, 8, portal).unwrap();
            game.world.set(0, 65, 8, portal).unwrap();
            game.mobs
                .push(Mob::new(40, zombie, Vec3::new(0.5, 64.0, 8.5), 0.0));
            use sunrise_gameplay_common::game::GameGameExt1 as _;
            game.load_ticking_neighbourhood(0, 0);
            game.update_chunk_tickets();
            assert!(
                game.entity_ticking_area()(0, 8),
                "the mob's chunk ticks entities"
            );
            let mob = game.entity(40).unwrap();

            // A written positive value blocks the portal: the mob stays, and
            // standing in the portal refills the countdown.
            assert!(mob.set_portal_cooldown(game, 40));
            assert_eq!(mob.portal_cooldown(game), Some(40));
            game.find_entity_portals();
            game.handle_entity_portals();
            assert!(
                game.mobs.iter().any(|m| m.id == 40),
                "the mob stays in its portal"
            );
            let held = mob.portal_cooldown(game).unwrap();
            assert_eq!(
                held,
                sunrise_sim::nether_portal::ENTITY_DELAY - 1,
                "standing in the portal refills the cooldown before the tick"
            );

            // Out of the portal, the value counts down one per tick.
            game.mobs.iter_mut().find(|m| m.id == 40).unwrap().position =
                Vec3::new(0.5, 64.0, 12.5);
            game.handle_entity_portals();
            assert_eq!(mob.portal_cooldown(game), Some(held - 1));

            // A negative value is refused and leaves the value alone.
            assert!(!mob.set_portal_cooldown(game, -1));
            assert_eq!(mob.portal_cooldown(game), Some(held - 1));

            // 0 lets it enter on the next tick.
            game.mobs.iter_mut().find(|m| m.id == 40).unwrap().position = Vec3::new(0.5, 64.0, 8.5);
            assert!(mob.set_portal_cooldown(game, 0));
            assert_eq!(mob.portal_cooldown(game), Some(0));
            game.find_entity_portals();
            game.handle_entity_portals();
            assert!(
                !game.mobs.iter().any(|m| m.id == 40),
                "0 lets the mob through"
            );
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_is_refused_for_a_player() {
        with_player(|game, player| {
            game.portal_player.cooldown = 7;
            assert!(!player.set_portal_cooldown(game, 40));
            assert_eq!(game.portal_player.cooldown, 7);
            assert_eq!(player.portal_cooldown(game), Some(7));
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_is_refused_for_a_falling_block() {
        use super::super::FallingBlock;
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_core::{geometry::Vec3, position::BlockPos};
        use sunrise_sim::nether_portal::PortalState;

        with_resident_game(|game| {
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:sand")
                .unwrap();
            game.server.next_entity += 1;
            let id = game.server.next_entity;
            game.falling.push(FallingBlock {
                size_answer: None,
                ground: Default::default(),
                fire_ticks: 0,
                sync_velocity: false,
                attachment_lifetime: Default::default(),
                rotation: [0.0, 0.0],
                join: Default::default(),
                id,
                state,
                blocks_building: true,
                start_pos: BlockPos::default(),
                position: Vec3::ZERO,
                previous: Vec3::ZERO,
                velocity: Vec3::ZERO,
                age: 0,
                no_gravity: false,
                portal: PortalState {
                    cooldown: 25,
                    process: None,
                },
                drop_item: false,
                fall_distance: 0.0,
                stuck: Vec3::ZERO,
                damage: sunrise_sim::falling_block::Damage::falling("minecraft:sand"),
                saved: None,
            });
            let falling = game.entity(id).unwrap();
            assert!(!falling.set_portal_cooldown(game, 40));
            assert_eq!(
                game.falling
                    .iter()
                    .find(|f| f.id == id)
                    .unwrap()
                    .portal
                    .cooldown,
                25
            );
            assert_eq!(falling.portal_cooldown(game), Some(25));
        });
    }

    // `portal_cooldown_write_is_refused_on_a_client` is now enforced at compile time
    // (`ClientGame` does not implement `ServerHost`).

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_reaches_an_item() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_items::items::ItemStack;

        with_resident_game(|game| {
            game.spawn_item(
                ItemStack {
                    item: "minecraft:stone".into(),
                    count: 1,
                    damage: 0,
                    extra: None,
                },
                [0.0; 3],
                [0.0; 3],
                0,
            );
            let item = game.entity(game.dropped[0].id).unwrap();
            assert!(item.set_portal_cooldown(game, 40));
            assert_eq!(item.portal_cooldown(game), Some(40));
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_reaches_an_arrow() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::projectiles::{Projectile, ProjectileKind};

        with_resident_game(|game| {
            game.add_projectile(Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::ZERO,
                None,
            ));
            let arrow = game.entity(game.creatures.projectiles[0].id).unwrap();
            assert!(arrow.set_portal_cooldown(game, 40));
            assert_eq!(arrow.portal_cooldown(game), Some(40));
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn portal_cooldown_write_is_refused_for_a_non_portal_projectile() {
        use super::super::native_entity_refs::tests::with_resident_game;
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::projectiles::{Projectile, ProjectileKind};

        with_resident_game(|game| {
            game.add_projectile(Projectile::new(
                ProjectileKind::WindCharge,
                Vec3::ZERO,
                None,
            ));
            let charge = game.entity(game.creatures.projectiles[0].id).unwrap();
            assert!(!charge.set_portal_cooldown(game, 40));
            assert_eq!(charge.portal_cooldown(game), None);
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn player_invisibility_write_is_refused() {
        with_player(|game, player| assert!(!player.set_invisible(game, true)));
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn player_glowing_write_is_kept_in_its_saved_data() {
        with_player(|game, player| {
            assert!(player.set_glowing(game, true));
            assert_eq!(player.glowing(game), Some(true));
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn player_no_gravity_write_is_kept_in_its_saved_data() {
        with_player(|game, player| {
            assert!(player.set_no_gravity(game, true));
            assert_eq!(player.no_gravity(game), Some(true));
        });
    }

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn player_frozen_write_updates_server_state() {
        with_player(|game, player| {
            assert!(player.set_ticks_frozen(game, 53));
            assert_eq!(game.player.as_ref().unwrap().ticks_frozen, 53);
        });
    }

    #[cfg(test)]
    #[test]
    fn max_air_follows_the_species() {
        assert_eq!(max_air(None), 300);
        assert_eq!(max_air(Some(Species::Dolphin)), 4800);
        assert_eq!(max_air(Some(Species::Axolotl)), 6000);
        assert_eq!(max_air(Some(Species::Zombie)), 300);
    }

    // `EntityRef::main_arm` (`LivingEntity.getMainArm`) on a client and on a server.
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_entity_state_common::NativeEntityStateCommonGameExt1 as _;
