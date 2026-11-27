//! Primed TNT (`PrimedTnt`): 80-tick fuse, gravity and drag, explosion of
//! power 4, `TntRenderer` swell and white flash, and `TntBlock.useItemOn`
//! (flint and steel / fire charge).
use crate::game::ServerGame;
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_game_data::{Result, text::Text};
use sunrise_gameplay_common::game::{
    item_abilities::ItemAbilitiesGameExt1 as _, tnt_common::PrimedTnt,
};
use sunrise_items::items::ItemStack;
use sunrise_sim::lan::playerdata::uuid_ints;
use sunrise_sim::mobs::Who;
use sunrise_world::level::flags as level_flags;

impl ServerGame<'_> {
    /// Asks the mods whether the TNT at `pos` may be primed
    /// (`native_tnt_primed`); the acting guest, if any, is its player.
    pub(in crate::game) fn tnt_prime_allowed(&mut self, pos: BlockPos) -> bool {
        let mut cancel = false;
        self.native_tnt_primed(
            self.server.lan_acting_uuid,
            [pos.x, pos.y, pos.z],
            &mut cancel,
        );
        // `TntBlock.prime` leaves the block alone with `tnt_explodes` off, for
        // redstone, fire and an unstable block breaking alike.
        !cancel
            && self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
    }
    /// Primes TNT a level event announced, or, when the mods refuse, puts
    /// back the block the fire or signal had already taken out.
    pub(in crate::game::server_side) fn prime_tnt_event(&mut self, pos: BlockPos, rot: f64) {
        if self.tnt_prime_allowed(pos) {
            self.add_primed_tnt(pos, rot, 80, None, None);
            return;
        }
        // Refused: the level queued the fuse sound and the game event right
        // after the prime; they are dropped as they come up.
        self.server.tnt_refused = Some(pos);
        let Some(tnt) = self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:tnt")
        else {
            return;
        };
        if self.common.world.block(pos.x, pos.y, pos.z) == self.common.world.air {
            let flags = level_flags::UPDATE_CLIENTS | level_flags::UPDATE_SKIP_ON_PLACE;
            self.with_level(|b, level, ticks| b.set_block(level, ticks, pos, tnt, flags));
        }
    }
    /// Adds primed TNT at the block's bottom centre, pushed along `rot`.
    pub fn add_primed_tnt(
        &mut self,
        pos: BlockPos,
        rot: f64,
        fuse: i32,
        owner: Option<Who>,
        guest: Option<u32>,
    ) {
        let position = Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y),
            f64::from(pos.z) + 0.5,
        );
        // An id from the entity counter at once, so a command can name it.
        self.server.next_entity += 1;
        // Entity: the UUID is the first two draws of the TNT's own random.
        let uuid = sunrise_sim::save::insecure_uuid(&mut sunrise_level::random_source::entity());
        self.common
            .persist
            .uuids
            .insert(self.server.next_entity, uuid);
        self.common.tnts.push(PrimedTnt {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: self.server.next_entity,
            position,
            previous: position,
            velocity: Vec3::new(-rot.sin() * 0.02, f64::from(0.2f32), -rot.cos() * 0.02),
            fuse,
            ticks: 0,
            ground: Default::default(),
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            in_water: false,
            owner,
            guest,
            // PrimedTnt's constructor: 4 when nothing sets it.
            power: 4.0,
            used_portal: false,
            block_state: None,
        });
    }
    /// The end of a piston's `Entity.move(MoverType.PISTON, …)` for primed TNT
    /// `i`, after restitution, for a step of `moved` from `from` (which it
    /// skipped if tiny): its fall and the block speed factor, then
    /// `moveEntityByPiston`'s `applyEffectsFromBlocks(previousPos, position)`,
    /// so powder snow it was pushed into makes it stuck.
    pub(in crate::game) fn finish_tnt_piston_move(&mut self, i: usize, from: Vec3) {
        let t = &mut self.common.tnts[i];
        let factor = sunrise_sim::motion::block_speed_factor_at(
            &self.common.world,
            &self.common.res.motion().blocks,
            &self.common.loop_context,
            t.position,
            None,
        );
        t.velocity = sunrise_world::entity_move::scale_by_block_speed(t.velocity, factor);
        self.tnt_piston_block_effects(i, from);
    }
    /// `moveEntityByPiston`'s `applyEffectsFromBlocks(previousPos, position)`
    /// for primed TNT `i`, also run when `limitPistonMovement` left no push.
    pub(in crate::game) fn tnt_piston_block_effects(&mut self, i: usize, from: Vec3) {
        let t = &self.common.tnts[i];
        let (id, footprint, ground, mut velocity) =
            (t.id, (t.position, t.bounds()), t.ground, t.velocity);
        let writes = self.body_step_on((id, "minecraft:tnt"), footprint, &ground, &mut velocity);
        let side = server_game_side!(self);
        let t = &mut self.common.tnts[i];
        let known = t.velocity;
        t.velocity = velocity;
        super::entity::apply_contact_writes(
            writes,
            &mut t.velocity,
            Some(&mut t.fall_distance),
            Some(&mut t.stuck),
            None,
        );
        let tops = t.apply_effects_from_blocks(
            sunrise_sim::motion::BodyMotion {
                world: &self.common.world,
                facts: self.common.res.facts(),
                collision: self.common.collision,
                context: &self.common.loop_context,
                blocks: Some(&self.common.motion.blocks),
                side: Some(&side),
            },
            self.common.pack.assets.registry,
            from,
            None,
        );
        let (to, bounds) = (t.position, t.bounds());
        self.entity_powder_snow_particles(from, to, bounds, known, false);
        for pos in tops {
            self.bubble_column_particles(pos);
        }
    }
    pub(in crate::game::server_side) fn take_removed_tnt(&mut self, id: u32) -> Option<PrimedTnt> {
        self.with_destroying_vehicle(super::commands::Target::Tnt(id), |game| {
            let i = game.tnts.iter().position(|t| t.id == id)?;
            Some(game.tnts.remove(i))
        })?
    }

    /// `PrimedTnt.tick` for every primed TNT, in order; an expired fuse
    /// explodes immediately (later TNT in the list feel its knockback this
    /// tick). TNT primed by an explosion starts ticking next tick.
    /// One primed TNT's `tick`: it moves, and explodes when its fuse is out.
    /// `i` is its place in the list; the caller has checked it is ticking.
    /// `entities` are the boxes that stop its move (`Game::entity_collisions` with `Mover::Passive`).
    pub fn tick_one_tnt(&mut self, i: usize, entities: &[Aabb]) -> Result<()> {
        let t = &mut self.common.tnts[i];
        let motion = sunrise_sim::motion::BodyMotion {
            world: &self.common.world,
            collision: self.common.collision,
            facts: self.common.res.facts(),
            blocks: Some(&self.common.res.motion().blocks),
            context: &self.common.loop_context,
            side: None,
        };
        let (movement, recorded) =
            t.move_before_fall(motion, Some(self.common.border.geometry()), entities);
        let (id, position, bounds) = (t.id, t.position, t.bounds());
        if t.check_fall_damage(movement.moved.y) {
            self.nonliving_fall_on(id, position, movement.effect);
            self.post_landing_event(position, bounds, false);
        }
        let Some(i) = super::native_ticks::place(&self.common.tnts, i, |t| t.id == id) else {
            return Ok(());
        };
        let response = self.common.tnts[i]
            .move_restitution(self.common.body_motion(&game_side!(self)), movement);
        let mut velocity = self.finish_body_restitution((id, "minecraft:tnt"), position, response);
        let Some(i) = super::native_ticks::place(&self.common.tnts, i, |t| t.id == id) else {
            return Ok(());
        };
        self.common
            .body_motion(&game_side!(self))
            .apply_block_speed(position, &mut velocity);
        let ground = self.common.tnts[i].ground;
        let writes = self.body_step_on(
            (id, "minecraft:tnt"),
            (position, bounds),
            &ground,
            &mut velocity,
        );
        let side = server_game_side!(self);
        let t = &mut self.common.tnts[i];
        t.velocity = velocity;
        super::entity::apply_contact_writes(
            writes,
            &mut t.velocity,
            Some(&mut t.fall_distance),
            Some(&mut t.stuck),
            None,
        );
        let tops = t.finish_tick(
            sunrise_sim::motion::BodyMotion {
                world: &self.common.world,
                collision: self.common.collision,
                facts: self.common.res.facts(),
                blocks: Some(&self.common.res.motion().blocks),
                context: &self.common.loop_context,
                side: Some(&side),
            },
            &self.common.pack.assets,
            &self.common.res.motion().fluids,
            recorded.then_some(movement.wanted),
        );
        let snow = (t.previous, t.position, t.bounds(), t.velocity);
        self.entity_powder_snow_particles(snow.0, snow.1, snow.2, snow.3, false);
        // Its move is filed at once, also for one about to explode.
        let (from, to) = (snow.0, snow.1);
        self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
        for pos in tops {
            self.bubble_column_particles(pos);
        }
        let Some(i) = super::native_ticks::place(&self.common.tnts, i, |t| t.id == id) else {
            return Ok(());
        };
        let t = &self.common.tnts[i];
        if t.fuse <= 0 {
            let center = [
                t.position.x,
                t.position.y + PrimedTnt::SIZE * 0.0625,
                t.position.z,
            ];
            let bounds = t.bounds();
            let (id, owner, guest, power) = (t.id, t.owner, t.guest, t.power);
            let used_portal = t.used_portal;
            let silent = self.entity_silent(id);
            let (owner, guest) = self.tnt_owner(id, owner, guest);
            let in_water = self.bounds_in_water(bounds);
            if self.take_removed_tnt(id).is_none() {
                return Ok(());
            }
            // PrimedTnt.tick: with `tnt_explodes` off it goes away without exploding.
            if !self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
            {
                return Ok(());
            }
            // ExplosionInteraction.TNT: the tnt_explosion_drop_decay rule.
            let interaction =
                self.destroy_type(sunrise_game_data::gamerules::vanilla::TNT_EXPLOSION_DROP_DECAY);
            self.server.combat.0.explosion_in_water = in_water;
            self.server.combat.0.explosion_direct = Some((id, "minecraft:tnt"));
            self.set_blast_owner(owner, guest);
            let result = self.with_hive_emptying_blast(|g| {
                g.server_explode_from(
                    center,
                    power,
                    interaction,
                    false,
                    super::explosion::ExplosionSource::default()
                        .with_silence(silent)
                        .with_portal_proof(used_portal)
                        .with_entity(
                            id,
                            "minecraft:tnt",
                            sunrise_sim::health::DirectEntity::Other,
                        ),
                )
            });
            self.server.combat.0.blast_owner = None;
            self.server.combat.0.blast_owner_player = false;
            self.server.combat.0.blast_guest = None;
            self.server.combat.0.explosion_in_water = false;
            self.server.combat.0.explosion_direct = None;
            result?;
        }
        Ok(())
    }
    /// The saved compound of primed TNT `i`: chunk saves, `/data get`, portals.
    pub fn tnt_nbt(&mut self, i: usize) -> sunrise_core::nbt::Tag {
        let id = self.common.tnts[i].id;
        let uuid = self.uuid_for(id);
        let owner = self.tnt_owner_uuid(self.common.tnts[i].owner, self.common.tnts[i].guest);
        let t = &self.common.tnts[i];
        sunrise_sim::entity_nbt::tnt_to_nbt(
            t.position,
            t.velocity,
            t.ground.on_ground,
            t.fall_distance,
            t.fuse,
            t.power,
            t.block_state_nbt(self.common.pack.assets.registry),
            t.rotation,
            uuid,
            owner,
            self.common.persist.raw.get(&id),
        )
    }
    /// `EntityReference.of(owner)`: the UUID of the player or mob that lit it.
    pub(in crate::game::server_side) fn tnt_owner_uuid(
        &mut self,
        owner: Option<Who>,
        guest: Option<u32>,
    ) -> Option<[i32; 4]> {
        match owner? {
            Who::Player => match guest {
                Some(conn) => self
                    .server
                    .lan
                    .as_ref()?
                    .hub
                    .guest(conn)
                    .map(|g| uuid_ints(g.uuid)),
                None => self.common.level.player_uuid,
            },
            Who::Mob(id) if self.common.mobs.iter().any(|m| m.id == id) => Some(self.uuid_for(id)),
            Who::Mob(id) => self.common.persist.uuids.get(&id).copied(),
        }
    }
    /// `PrimedTnt.getOwner` for the TNT `id` whose own owner is `owner`: else
    /// the one it was loaded with.
    pub(in crate::game::server_side) fn tnt_owner(
        &self,
        id: u32,
        owner: Option<Who>,
        guest: Option<u32>,
    ) -> (Option<Who>, Option<u32>) {
        match owner {
            Some(_) => (owner, guest),
            None => self
                .loaded_tnt_owner(id)
                .map_or((None, None), |(w, g)| (Some(w), g)),
        }
    }
    /// `Explosion.getIndirectSourceEntity` of the primed TNT `id` that a
    /// blast names as its source: its owner, as `set_blast_owner` sets it.
    pub(in crate::game::server_side) fn set_tnt_blast_owner(&mut self, id: u32) {
        let (owner, guest) = self
            .common
            .tnts
            .iter()
            .find(|t| t.id == id)
            .map_or((None, None), |t| (t.owner, t.guest));
        let (owner, guest) = self.tnt_owner(id, owner, guest);
        self.set_blast_owner(owner, guest);
    }
    /// `Explosion.getIndirectSourceEntity`: a blast whose cause is `owner`
    /// (a TNT's igniter, a player's projectile, a player) names it, which
    /// `blast_attacker`, the damage type and kill credit read. A player is the
    /// one on `guest`, the player in place without one.
    pub(in crate::game::server_side) fn set_blast_owner(
        &mut self,
        owner: Option<Who>,
        guest: Option<u32>,
    ) {
        self.server.combat.0.blast_guest = guest;
        self.server.combat.0.blast_owner = tnt_blast_owner(
            owner,
            || match guest {
                Some(_) => self.player_attacker(guest),
                None => self
                    .common
                    .player
                    .as_ref()
                    .map(|p| (p.entity_id as u32, self.player_display_name())),
            },
            |id| self.mob_display_name(id),
        );
        self.server.combat.0.blast_owner_player =
            owner == Some(Who::Player) && self.server.combat.0.blast_owner.is_some();
    }
    /// The attack a player-owned blast's hits land in: its causing player's,
    /// so a mob it kills is that player's kill.
    pub(in crate::game::server_side) fn blast_owner_attack(&self) -> sunrise_sim::health::Attack {
        let id = self
            .server
            .combat
            .0
            .blast_owner
            .as_ref()
            .map_or(0, |(id, _)| *id as i32);
        let mut attack = self.attack_of(id);
        attack.player_uuid = self.player_uuid_of(self.server.combat.0.blast_guest);
        attack
    }
    /// `PrimedTnt.getOwner` for TNT loaded with an `owner`: looked up when
    /// asked, so a player who joined or a mob loaded since is found.
    pub(in crate::game::server_side) fn loaded_tnt_owner(
        &self,
        id: u32,
    ) -> Option<(Who, Option<u32>)> {
        let uuid = sunrise_sim::entity_nbt::tnt_owner(self.common.persist.raw.get(&id)?)?;
        let guest = self.server.lan.as_ref().and_then(|host| {
            let mut guests = host.hub.guests.iter();
            guests.find(|g| uuid_ints(g.uuid) == uuid).map(|g| g.conn)
        });
        if guest.is_some() {
            return Some((Who::Player, guest));
        }
        if self.common.level.player_uuid == Some(uuid) {
            return Some((Who::Player, None));
        }
        self.mob_by_uuid(uuid).map(|m| (Who::Mob(m), None))
    }
    /// `TntBlock.useItemOn`: flint and steel or a fire charge primes TNT.
    pub(in crate::game::server_side) fn ignite_tnt(&mut self, stack: &ItemStack) -> Result<bool> {
        let item = &*stack.item;
        let starter = matches!(item, "minecraft:flint_and_steel" | "minecraft:fire_charge");
        if !self.stack_can_perform(
            stack,
            super::item_abilities::ability::FIRESTARTER_LIGHT,
            starter,
        ) {
            return Ok(false);
        }
        let Some(hit) = self.pick() else {
            return Ok(false);
        };
        let state = self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if &*self.common.res.facts().get(state).name != "minecraft:tnt" {
            return Ok(false);
        }
        let pos = BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
        {
            // TntBlock.useItemOn: the rule is off, the acting player's overlay
            // says so, and PASS lets the item's own useOn light a fire there.
            let message = Text::translate("block.minecraft.tnt.disabled", &[]);
            self.server_send_overlay_message(message);
            return Ok(false);
        }
        // `TntBlock.prime`: an adventure player lights TNT only with a stack
        // whose `can_break` matches it. The refused click is still SUCCESS: the
        // arm swings, and neither the block, the stack nor the fuse changes.
        if !self.may_build() && !self.can_break_in_adventure(hit.pos) {
            self.swing_arm();
            return Ok(true);
        }
        if !self.tnt_prime_allowed(pos) {
            return Ok(true);
        }
        let guest = self.server.lan_acting;
        self.spawn_primed_tnt(pos, false, Some(Who::Player), guest);
        let air = self.common.world.air;
        self.with_level(|b, level, ticks| {
            b.set_block(level, ticks, pos, air, level_flags::UPDATE_ALL_IMMEDIATE)
        });
        self.server_process_level_events()?;
        self.swing_arm();
        if item == "minecraft:flint_and_steel" {
            self.damage_held(1);
        } else {
            self.consume_held();
        }
        self.award_stat("used", item, 1);
        Ok(true)
    }
}

/// `PrimedTnt.getOwner` as the blast's causing entity: the player, or a mob
/// that is still in the level (null once it is gone).
pub(in crate::game::server_side) fn tnt_blast_owner(
    owner: Option<Who>,
    player: impl FnOnce() -> Option<(u32, Text)>,
    mob_name: impl Fn(u32) -> Option<Text>,
) -> Option<(u32, Text)> {
    match owner? {
        Who::Player => player(),
        Who::Mob(id) => mob_name(id).map(|name| (id, name)),
    }
}

impl ServerGame<'_> {
    /// `new PrimedTnt(level, x + 0.5, y, z + 0.5, owner)`; `short` applies
    /// `wasExploded`'s random short fuse. Plays `entity.tnt.primed` for primes.
    pub fn spawn_primed_tnt(
        &mut self,
        pos: BlockPos,
        short: bool,
        owner: Option<Who>,
        guest: Option<u32>,
    ) {
        // PrimedTnt: the fuse is lit.
        let who = self.sculk_actor();
        self.post_block_game_event("minecraft:prime_fuse", [pos.x, pos.y, pos.z], who, None);
        let r = &mut self.common.world.random;
        let rot = r.next_double() * f64::from(std::f32::consts::TAU);
        let fuse = if short {
            // PrimedTnt.getRandomShortFuse
            r.next_int_bound(20).unwrap_or(0) + 10
        } else {
            self.server_play_sound(
                "minecraft:entity.tnt.primed",
                [
                    f64::from(pos.x) + 0.5,
                    f64::from(pos.y),
                    f64::from(pos.z) + 0.5,
                ],
                1.0,
                1.0,
            );
            80
        };
        self.add_primed_tnt(pos, rot, fuse, owner, guest);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    #[cfg(test)]
    use super::*;

    #[cfg(test)]
    #[test]
    fn saved_block_state_follows_the_model_instead_of_raw_input() {
        use {
            sunrise_core::nbt::Tag,
            sunrise_world::registry::{BlockRegistry, BlockStateId},
        };
        let unstable = Tag::compound([
            ("id", Tag::String("minecraft:tnt".into())),
            (
                "properties",
                Tag::compound([("unstable", Tag::String("true".into()))]),
            ),
        ]);
        let registry = BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            (
                "states",
                Tag::List(vec![
                    Tag::String("minecraft:tnt".into()),
                    Tag::String("minecraft:stone".into()),
                    unstable.clone(),
                ]),
            ),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:tnt", Tag::Int(0)),
                    ("minecraft:stone", Tag::Int(1)),
                ]),
            ),
        ]))
        .unwrap();
        let mut tnt = PrimedTnt {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            id: 1,
            position: Vec3::ZERO,
            previous: Vec3::ZERO,
            velocity: Vec3::ZERO,
            fuse: 400,
            ticks: 0,
            ground: Default::default(),
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            in_water: false,
            owner: None,
            guest: None,
            no_gravity: true,
            power: 4.0,
            used_portal: false,
            block_state: Some(BlockStateId(1)),
        };
        let raw = Tag::compound([("block_state", Tag::String("audit_missing:block".into()))]);
        for (state, expected) in [
            (Some(BlockStateId(1)), Tag::String("minecraft:stone".into())),
            (Some(BlockStateId(2)), unstable),
            (None, Tag::String("minecraft:tnt".into())),
        ] {
            tnt.block_state = state;
            let saved = sunrise_sim::entity_nbt::tnt_to_nbt(
                tnt.position,
                tnt.velocity,
                tnt.ground.on_ground,
                tnt.fall_distance,
                tnt.fuse,
                tnt.power,
                tnt.block_state_nbt(&registry),
                [0.0, 0.0],
                [221, 0, 0, 1],
                None,
                Some(&raw),
            );
            assert_eq!(saved.get("block_state"), Some(&expected));
        }
    }

    #[cfg(test)]
    #[test]
    fn a_tnt_lit_by_a_skeletons_arrow_names_the_skeleton_as_its_cause() {
        let name = || Text::translate("entity.minecraft.skeleton", &[]);
        let by_mob = tnt_blast_owner(Some(Who::Mob(7)), || None, |id| (id == 7).then(name));
        assert_eq!(by_mob, Some((7, name())));
        // The skeleton died before the blast: `getOwner` is null.
        assert_eq!(tnt_blast_owner(Some(Who::Mob(7)), || None, |_| None), None);
        let player = Text::literal("Steve");
        let by_player = tnt_blast_owner(Some(Who::Player), || Some((1, player.clone())), |_| None);
        assert_eq!(by_player, Some((1, player)));
        assert_eq!(tnt_blast_owner(None, || None, |_| None), None);
    }

    #[cfg(test)]
    #[test]
    fn a_rider_sits_on_top_of_the_tnt_less_its_own_attachment() {
        // 64 + 0.98F (the box's top, `EntityAttachment.PASSENGER` at its
        // height) - 0.14 (a rider's vehicle attachment).
        let seat =
            sunrise_gameplay_common::game::tnt_common::tnt_seat(Vec3::new(1.5, 64.0, -2.5), 0.14);
        assert_eq!((seat.x, seat.z), (1.5, -2.5));
        assert!((seat.y - (64.0 + f64::from(0.98_f32) - 0.14)).abs() < 1e-9);
    }

    #[cfg(test)]
    #[test]
    fn a_piston_slime_bounce_launches_primed_tnt() {
        use sunrise_core::geometry::Direction;
        use sunrise_world::piston_push::bounce_velocity;
        let mut tnt = PrimedTnt {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 0,
            position: Vec3::new(0.5, 1.0, 0.5),
            previous: Vec3::new(0.5, 1.0, 0.5),
            velocity: Vec3::ZERO,
            fuse: 80,
            ticks: 0,
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            in_water: false,
            owner: None,
            guest: None,
            power: 4.0,
            used_portal: false,
            block_state: None,
        };
        tnt.velocity = bounce_velocity(Direction::Up, tnt.velocity);
        tnt.ground.on_ground = false;
        assert_eq!(tnt.velocity.y, 1.0);
        assert!(!tnt.ground.on_ground);
    }

    pub fn particle_count(game: &crate::game::ServerGame<'_>) -> usize {
        game.server.pending_explosions.len()
            + game
                .server
                .lan_events
                .iter()
                .filter(|e| matches!(e.kind, 2002 | 2007))
                .count()
    }

    pub fn sound_count(game: &crate::game::ServerGame<'_>, event: &str) -> u32 {
        let sounds = game
            .server
            .pending_sounds
            .iter()
            .filter(|s| s.event == event)
            .count() as u32;
        let level_events = if event == "minecraft:entity.splash_potion.break" {
            game.server
                .lan_events
                .iter()
                .filter(|e| matches!(e.kind, 1053 | 1054))
                .count() as u32
        } else {
            0
        };
        sounds + level_events
    }

    pub fn set_silent(game: &mut crate::game::ServerGame<'_>, id: u32, silent: bool) {
        game.remember_tags(
            id,
            &sunrise_core::nbt::Tag::Compound(
                [(
                    "Silent".into(),
                    sunrise_core::nbt::Tag::Byte(i8::from(silent)),
                )]
                .into_iter()
                .collect(),
            ),
        );
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
