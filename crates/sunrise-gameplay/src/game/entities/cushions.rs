//! Cushions: `CushionItem.useOn` placement on the top face of a block,
//! `Cushion.canBePlacedAt`/`survives`, `Cushion.interact` (sit),
//! `BlockAttachedEntity.hurtServer` (break, dropping the matching colour),
//! `BlockAttachedEntity.tick` and `CushionRenderer`.
//!
//! A cushion is a `BlockAttachedEntity`, not a block: it has no block state,
//! only an entity whose `Pos` is the click point's height. Placement uses
//! outline bounds, collision shapes and suffocation predicates.
use super::{Hit, InteractTarget, Keys};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay_common::game::transactions::entities::EntityRemovalGameExt as _;
use sunrise_items::items::ItemStack;
use sunrise_sim::cushion::blocks_in;
use sunrise_sim::entities::{Cushion, dye_index};

pub(in crate::game::server_side) fn overlaps(a: Aabb, b: Aabb) -> bool {
    a.intersects(b)
}
use sunrise_gameplay_common::game::{
    cushions_common::{aabb, snap_yaw},
    entity_common::PlacementWorld,
};

impl crate::game::ServerGame<'_> {
    /// The cushion under the crosshair, its distance and the hit point.
    pub(in crate::game::server_side) fn cushion_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.common.entity_reach;
        let (from, to) = super::pick_ray::pick_ray_segment(eye, dir, reach);
        let mut pick = super::pick_ray::PickAccumulator::new(reach * reach);
        let mut best: Option<(usize, f64, [f64; 3])> = None;
        for (i, c) in self.common.cushions.iter().enumerate() {
            if pick.consider_box(self, c.id, from, to, aabb(c.bounds())) {
                let hit = pick.hovered.unwrap();
                best = Some((i, hit.distance_sq.sqrt(), hit.point));
            }
        }
        best
    }

    pub(in crate::game::server_side) fn cushion_survives(&self, c: &Cushion) -> bool {
        sunrise_sim::cushion::survives(
            &PlacementWorld(self, std::marker::PhantomData),
            aabb(c.bounds()),
        )
    }

    /// `Cushion.destroyIfInFire`: a fire-tagged block inside the deflated box.
    pub(in crate::game::server_side) fn cushion_in_fire(&self, c: &Cushion) -> bool {
        let bounds = aabb(c.bounds()).next_deflated();
        blocks_in(bounds).any(|p| {
            let facts = self
                .common
                .res
                .facts()
                .get(self.common.world.block(p[0], p[1], p[2]));
            self.items().block_in_tag("minecraft:fire", &facts.name)
        })
    }

    /// `CushionItem.useOn` up to `Cushion.canBePlacedAt`, which the client
    /// checks too: on the top face only, at the click's height above the
    /// clicked block (or the replaceable block itself). The position and
    /// spawn box of a cushion that may go there.
    /// `CushionItem.useOn`: spawn the cushion where `cushion_spawn` allows,
    /// unless another cushion is in the way.
    pub fn place_cushion(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        let Some(info) = self.items().get(&stack.item).cloned() else {
            return false;
        };
        let Some(default_color) = info.cushion_color.as_deref() else {
            return false;
        };
        let color_override = stack
            .data()
            .and_then(|d| d.other("minecraft:cushion/color"))
            .and_then(|tag| match tag {
                sunrise_core::nbt::Tag::String(s) => Some(s.to_string_lossy()),
                _ => None,
            });
        let color = dye_index(color_override.as_deref().unwrap_or(default_color));
        let Some((position, bounds)) = self.cushion_spawn(hit) else {
            return false;
        };
        // `getEntitiesOfClass(Cushion.class, spawnAABB).isEmpty()`.
        if self
            .common
            .cushions
            .iter()
            .any(|c| overlaps(aabb(c.bounds()), bounds))
        {
            return false;
        }
        self.server.next_entity += 1;
        let id = self.server.next_entity;
        // `BlockAttachedEntity.setPos`: `BlockPos.containing(position)`.
        let attached = [
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ];
        self.common.cushions.push(Cushion {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            position,
            pos: attached,
            yaw: snap_yaw(self.common.yaw),
            color,
            check_interval: 0,
            // `EntityType.createDefaultStackConfig`: `Entity.applyImplicitComponents`
            // copies the stack's `CUSTOM_NAME` onto the placed cushion.
            custom_name: stack.data().and_then(|d| d.custom_name.clone()),
        });
        self.common
            .world
            .modified
            .insert((attached[0] >> 4, attached[2] >> 4));
        // `destroyIfInFire`: placed into fire, it breaks at once.
        let placed = self.common.cushions.len() - 1;
        if self.cushion_in_fire(&self.common.cushions[placed]) {
            self.break_cushion(placed, false);
        }
        self.server_play_sound(
            "minecraft:entity.cushion.place",
            [position.x, position.y, position.z],
            0.75,
            0.8,
        );
        let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Player), None);
        self.post_game_event(
            "minecraft:entity_place",
            [position.x, position.y, position.z],
            ctx,
        );
        if !self.common.creative {
            self.consume_held();
        }
        // `CushionItem.useOn` returns SUCCESS: the hand swings.
        self.swing_arm();
        true
    }

    /// `Cushion.hurtServer` from a player: killed, then the item dropped.
    pub(in crate::game::server_side) fn attack_cushion(&mut self) -> bool {
        let Some((i, d, _)) = self.cushion_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        // `Cushion.hurtServer` refuses a player who may not build, so
        // skipAttackInteraction is false and Player.attack goes on to onAttack.
        if !self.may_build() {
            self.server.attack_ticks = 0;
            return true;
        }
        self.break_cushion(i, true);
        true
    }

    /// `BlockAttachedEntity.hurtServer`: `kill` (the `entity_die` event), then
    /// `Cushion.dropItem`.
    pub fn break_cushion(&mut self, index: usize, by_player: bool) {
        self.break_cushion_by(index, by_player.then_some(super::commands::Target::Player));
    }
    pub fn break_cushion_by(
        &mut self,
        index: usize,
        source: Option<super::commands::Target>,
    ) -> bool {
        self.remove_cushion(index, source, true)
    }
    /// `BlockAttachedEntity.tick` on lost support: `discard()` and then
    /// `dropItem(level, null)`, which posts no game event.
    pub(in crate::game::server_side) fn discard_cushion(&mut self, index: usize) {
        self.remove_cushion(index, None, false);
    }
    /// `Player.hasInfiniteMaterials` of a hurt's cause: the mode of the body in
    /// place, or of the guest the blast names.
    pub(in crate::game::server_side) fn infinite_materials(
        &self,
        source: Option<&super::commands::Target>,
    ) -> bool {
        use super::commands::Target;
        match source {
            Some(Target::Player) => self.common.creative,
            // The guest whose turn it is is in place already (the hub is out of
            // `self.lan` while it acts), as `sculk::entity_event_by` reads it.
            Some(Target::Guest(conn)) if self.server.lan_acting == Some(*conn) => {
                self.common.creative
            }
            Some(Target::Guest(conn)) => self.command_guest(*conn).is_some_and(|g| g.creative()),
            _ => false,
        }
    }

    /// `Entity.kill` of a cushion (`/kill`): gone with no drop, its rider let go.
    pub(in crate::game::server_side) fn take_removed_cushion(
        &mut self,
        id: u32,
    ) -> Option<Cushion> {
        self.with_destroying_vehicle(super::commands::Target::Cushion(id), |game| {
            let i = game.cushions.iter().position(|c| c.id == id)?;
            let c = game.cushions.remove(i);
            game.world.modified.insert((c.pos[0] >> 4, c.pos[2] >> 4));
            Some(c)
        })?
    }

    #[inline(always)]
    pub(in crate::game::server_side) fn cushion_break_particles_server(
        &mut self,
        state: sunrise_world::registry::BlockStateId,
        particle_at: [f64; 3],
    ) {
        self.server.lan_cushion_breaks.push((state, particle_at));
    }

    /// `Cushion.showBreakingParticles`: ten of the matching wool block.
    pub(in crate::game::server_side) fn cushion_break_particles(&mut self, c: &Cushion) {
        let name = format!("minecraft:{}_wool", c.color_name());
        let Some(state) = self.common.pack.assets.registry.default_state(&name) else {
            return;
        };
        let p = c.position;
        // Entity.getY(2/3) is relative to the cushion's quarter-block height.
        let particle_at = [p.x, p.y + 0.25 * (2.0 / 3.0), p.z];
        // Guest actions temporarily take self.lan; flush once the hub is back.
        // Queue before resolving a sprite: a dedicated server has no renderer.
        if self.server.audio_capture_active {
            self.cushion_break_particles_server(state, particle_at);
        }
    }

    /// `LivingEntity.stopRiding` off a cushion: `dismountVehicle` beside it,
    /// and `Cushion.removePassenger`'s get-up sound for a rider that left
    /// rather than one whose cushion was killed.
    pub(in crate::game::server_side) fn get_up_from_cushion(&mut self, c: &Cushion) {
        if self.common.riding == Some(c.id) && !self.native_may_leave(c.id) {
            return;
        }
        let at = c.dismount();
        self.common.riding = None;
        if let Some(p) = self.common.player.as_mut() {
            self.common.previous_position = p.position;
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
        self.server_play_sound(
            "minecraft:entity.cushion.get_up",
            [c.position.x, c.position.y, c.position.z],
            1.0,
            1.0,
        );
    }

    /// `Cushion.interact`: sit down unless sneaking or already occupied. The
    /// cushion under the crosshair, or the one a guest's interact packet named.
    pub fn interact_cushion(&mut self, target: Option<InteractTarget>) -> bool {
        let Some((i, d, _)) = (match target {
            Some(t) => sunrise_gameplay_common::game::item_actions::named_entity(
                &self.common.cushions,
                t.id,
                |c| c.id,
            )
            .map(|i| (i, 0.0, [0.0; 3])),
            None => self.cushion_hit(),
        }) else {
            return false;
        };
        // Only a client's own pick is stopped by a block: the range check passed.
        if target.is_none() && !self.before_block(d) {
            return false;
        }
        // isSecondaryUseActive.
        if self.server_acting_sneak() {
            return false;
        }
        let id = self.common.cushions[i].id;
        let occupied = self.common.riding == Some(id)
            || self
                .server
                .lan
                .as_ref()
                .is_some_and(|h| h.hub.guests.iter().any(|g| g.vehicle == Some(id)));
        // `Cushion.interact`: an occupied cushion (`isVehicle`) passes.
        if occupied {
            return false;
        }
        // `Entity.startRiding`: a passenger first gets off what it rides
        // (`stopRiding`), so a seated player moves to this cushion.
        if !self.native_may_board(id) {
            return true;
        }
        // After each hook the cushion is found again by its id: a listener
        // may have removed it, and the click is then spent.
        if sunrise_gameplay_common::game::item_actions::named_entity(
            &self.common.cushions,
            id as i32,
            |c| c.id,
        )
        .is_none()
        {
            return true;
        }
        self.stop_riding();
        // startRiding stops when stopRiding left the passenger seated (a mod
        // canceled the dismount).
        if self.common.riding.is_some() {
            return true;
        }
        let Some(i) = sunrise_gameplay_common::game::item_actions::named_entity(
            &self.common.cushions,
            id as i32,
            |c| c.id,
        ) else {
            return true;
        };
        self.start_riding(id);
        let p = self.common.cushions[i].position;
        self.server_play_sound("minecraft:entity.cushion.sit", [p.x, p.y, p.z], 1.0, 1.0);
        self.swing_arm();
        true
    }

    /// The removal both paths share; `killed` posts `BlockAttachedEntity.kill`'s
    /// `entity_die` event before `dropItem`'s sound and drop.
    pub(in crate::game::server_side) fn remove_cushion(
        &mut self,
        index: usize,
        source: Option<super::commands::Target>,
        killed: bool,
    ) -> bool {
        let id = self.common.cushions[index].id;
        if self.target_removed(&super::commands::Target::Cushion(id)) {
            return false;
        }
        let Some(c) = self.take_removed_cushion(id) else {
            return false;
        };
        let p = c.position;
        if killed {
            // BlockAttachedEntity.kill, before dropItem.
            self.entity_killed_event_by([p.x, p.y, p.z], source);
        }
        self.server_play_sound("minecraft:entity.cushion.break", [p.x, p.y, p.z], 1.0, 1.0);
        self.cushion_break_particles(&c);
        // `Cushion.dropItem`: the item unless drops are off or the cause is a
        // player with infinite materials (the TNT's owner for a blast).
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS)
            || self.infinite_materials(source.as_ref())
        {
            return true;
        }
        let stack = c.item_stack_with_data();
        self.spawn_item(stack, [p.x, p.y, p.z], [0.0; 3], 10);
        true
    }

    /// `BlockAttachedEntity.tick`: survive and burn checks after the post-increment reaches 100.
    pub fn tick_cushions(&mut self) {
        let ticking = self.entity_ticking_area();
        let mut i = 0;
        while i < self.common.cushions.len() {
            let p = self.common.cushions[i].position;
            if !ticking(p.x.floor() as i32, p.z.floor() as i32) {
                i += 1;
                continue;
            }
            let previous = self.common.cushions[i].check_interval;
            self.common.cushions[i].check_interval += 1;
            if previous < 100 {
                i += 1;
                continue;
            }
            self.common.cushions[i].check_interval = 0;
            if !self.check_cushion(i) {
                i += 1;
            }
        }
    }

    /// `Cushion.tickAtCheckInterval` plus the survive check; true when the cushion broke.
    pub(in crate::game::server_side) fn check_cushion(&mut self, i: usize) -> bool {
        let c = self.common.cushions[i].clone();
        let p = c.position;
        let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        // collidedWithFluid: the fluid box of the cushion's own block against its box.
        if let Some((height, _)) =
            self.common
                .fluid_height_of(x, y, z, Some(sunrise_items::blocks::Fluid::Lava))
        {
            let fluid = Aabb::new(
                Vec3::new(f64::from(x), f64::from(y), f64::from(z)),
                Vec3::new(
                    f64::from(x) + 1.0,
                    f64::from(y) + f64::from(height),
                    f64::from(z) + 1.0,
                ),
            );
            if aabb(c.bounds())
                .deflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7))
                .intersects(fluid)
            {
                // LAVA_IGNITE's lavaHurt deals 4 damage, which breaks the cushion.
                self.break_cushion(i, false);
                self.lava_hurt_sound([p.x, p.y, p.z]);
                return true;
            }
        }
        // `destroyIfInFire` hurts the cushion (`kill`), so it is removed before
        // the survive check below.
        if self.cushion_in_fire(&c) {
            self.break_cushion(i, false);
            return true;
        }
        // BlockAttachedEntity.tick: `discard()` then `dropItem(level, null)`.
        if !self.cushion_survives(&c) {
            self.discard_cushion(i);
            return true;
        }
        false
    }

    /// `LivingEntity.dismountVehicle` from a live vehicle at `vehicle` whose
    /// block (`blockPosition`) is in `#minecraft:portals`: the rider leaves by
    /// the same free-position branch as from a removed vehicle, not beside it.
    /// True when it did.
    pub(in crate::game) fn dismount_in_portal(&mut self, vehicle: Vec3) -> bool {
        let pos = [
            vehicle.x.floor() as i32,
            vehicle.y.floor() as i32,
            vehicle.z.floor() as i32,
        ];
        if !self.named_block_in_tag("minecraft:portals", &self.block_name(pos)) {
            return false;
        }
        self.common.dismount_removed_vehicle(vehicle.y);
        true
    }

    /// `Player.rideTick` on a cushion: hold the seat, and get up on sneak.
    pub(in crate::game::server_side) fn tick_cushion_riding(&mut self, keys: Keys) -> bool {
        let Some(c) = self
            .common
            .cushions
            .iter()
            .find(|c| Some(c.id) == self.common.riding)
            .cloned()
        else {
            return false;
        };
        if keys.shift {
            self.get_up_from_cushion(&c);
            return true;
        }
        let seat = c.seat();
        if let Some(p) = self.common.player.as_mut() {
            self.common.previous_position = p.position;
            p.position = seat;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
        true
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use sunrise_core::nbt::Tag;

    /// `Entity.saveWithoutId` writes `OnGround` and `fall_distance`; a
    /// cushion never moves, so a save keeps what it was loaded with.
    #[test]
    fn a_cushion_save_keeps_its_loaded_ground_state() {
        use sunrise_sim::entity_nbt::{cushion_from_nbt, cushion_to_nbt};
        let d = |v: f64| Tag::Double(v.to_bits());
        let pos = Tag::List(vec![d(0.5), d(64.0), d(0.5)]);
        let raw = Tag::compound([
            ("Pos", pos),
            ("block_pos", Tag::IntArray(vec![0, 64, 0])),
            ("OnGround", Tag::Byte(1)),
            ("fall_distance", d(20.0)),
        ]);
        let c = cushion_from_nbt(&raw, 1).unwrap();
        let saved = cushion_to_nbt(&c, [1, 2, 3, 4], Some(&raw));
        assert_eq!(saved.get("OnGround"), Some(&Tag::Byte(1)));
        assert_eq!(saved.get("fall_distance"), Some(&d(20.0)));
        let fresh = cushion_to_nbt(&c, [1, 2, 3, 4], None);
        assert_eq!(fresh.get("OnGround"), Some(&Tag::Byte(0)));
        assert_eq!(fresh.get("fall_distance"), Some(&d(0.0)));
    }

    /// `Cushion.getCushionItemStackWithData`: a named cushion drops named.
    #[test]
    fn a_named_cushion_drops_with_its_name() {
        let c = Cushion {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 1,
            position: Vec3::ZERO,
            pos: [0, 64, 0],
            yaw: 0.0,
            color: 1,
            check_interval: 0,
            custom_name: Some(Tag::String("Seat".into())),
        };
        let stack = c.item_stack_with_data();
        assert_eq!(&*stack.item, "minecraft:orange_cushion");
        assert!(stack.data().is_some_and(|d| d.custom_name.is_some()));
        // An unnamed one gains no component.
        let c = Cushion {
            custom_name: None,
            ..c
        };
        assert!(c.item_stack_with_data().data().is_none());
    }
}
