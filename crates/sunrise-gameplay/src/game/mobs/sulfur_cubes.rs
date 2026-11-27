//! Sulfur cubes on the game side (`SulfurCube.mobInteract`, `pickUpItem`,
//! `remove`): feeding babies, swallowing a held block, shearing it back
//! out, lighting an explosive one, swallowing blocks from the ground and
//! splitting into two babies.
use crate::game::ServerGame;
use sunrise_core::position::BlockPos;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::item_abilities::ability;
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    mobs::{Species, Who, sulfur_cube},
    projectiles::Projectile,
};

impl ServerGame<'_> {
    /// Spit the swallowed block out on top (`EntityAttachment.PASSENGER`).
    pub(in crate::game::server_side) fn eject_sulfur_block(&mut self, index: usize) {
        let m = &mut self.common.mobs[index];
        let Some(item) = m.take_body_equipment() else {
            return;
        };
        let at = [
            m.position.x,
            m.position.y + f64::from(m.height()),
            m.position.z,
        ];
        self.spawn_item(item, at, [0.0, 0.2, 0.0], 10);
    }
    /// `SulfurCube.mobInteract`. Returns whether the click was used.
    pub fn interact_sulfur_cube(&mut self, index: usize, item: &str) -> bool {
        let m = &self.common.mobs[index];
        if m.species() != Species::SulfurCube || !m.alive() {
            return false;
        }
        if m.is_baby() {
            // Slime balls grow a baby up by a tenth (`canAgeUp`: not while an
            // age lock holds it).
            if !m.kind.food.iter().any(|f| f == item) || !sunrise_sim::mobs::ai::can_age_up(m) {
                return false;
            }
            let m = &mut self.common.mobs[index];
            let secs = (-m.extra.age / 20) as f32 * 0.1;
            // AgeableMob.ageUp: a grown cube is size 2 (SulfurCube.ageBoundaryReached).
            sunrise_sim::mobs::ai::age_up(m, secs as i32, false);
            self.consume_held();
            self.sulfur_sound(index, "entity.small_sulfur_cube.eat");
            return true;
        }
        if sulfur_cube::primed(m) {
            return false;
        }
        let explosive = sulfur_cube::archetype(m).is_some_and(|a| a.explosion.is_some());
        if explosive && matches!(item, "minecraft:flint_and_steel" | "minecraft:fire_charge") {
            if !self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
            {
                let message = Text::translate("block.minecraft.tnt.disabled", &[]);
                self.server_send_overlay_message(message);
                return false;
            }
            sulfur_cube::prime(&mut self.common.mobs[index], false);
            if item == "minecraft:flint_and_steel" {
                self.damage_held_tool();
            } else {
                self.consume_held();
            }
            self.award_stat("used", item, 1);
            return true;
        }
        if self.held_shears(ability::SHEARS_REMOVE_ARMOR) && sulfur_cube::has_body(m) {
            // shear: out comes the block, and nothing is swallowed for 5 s.
            let sheared = self.common.mobs[index]
                .extra
                .body_stack()
                .map(|s| s.item.clone());
            self.eject_sulfur_block(index);
            self.common.mobs[index].extra.ints[3] = sulfur_cube::PICKUP_TIMER;
            self.sulfur_sound(index, "entity.sulfur_cube.eject");
            self.damage_held_tool();
            // gameEvent(SHEAR, player).
            let at = self.common.mobs[index].position;
            let ctx = self.who_context(Some(Who::Player), None);
            self.post_game_event("minecraft:shear", [at.x, at.y, at.z], ctx);
            // CriteriaTriggers.PLAYER_SHEARED_EQUIPMENT with the body block.
            if let Some(body) = sheared {
                let target = self.mob_entity(&self.common.mobs[index]);
                self.trigger_item_entity(
                    "minecraft:player_sheared_equipment",
                    &ItemStack::new(&body, 1),
                    &target,
                );
            }
            return true;
        }
        if sulfur_cube::swallowable(m, item) {
            // equipItem: not the same block again; the old one comes out.
            if m.extra.body_id() == Some(item) {
                return false;
            }
            self.eject_sulfur_block(index);
            let held = self.common.inventory[self.common.selected]
                .clone()
                .map(|s| ItemStack { count: 1, ..s });
            self.common.mobs[index].extra.set_body_stack(held);
            self.consume_held();
            self.sulfur_sound(index, "entity.sulfur_cube.absorb");
            // Entity.gameEvent(ENTITY_INTERACT): the cube is the source.
            let (id, at) = (self.common.mobs[index].id, self.common.mobs[index].position);
            let ctx = self.who_context(Some(Who::Mob(id)), None);
            self.post_game_event("minecraft:entity_interact", [at.x, at.y, at.z], ctx);
            return true;
        }
        if item == "minecraft:bucket" {
            self.bucket_sulfur_cube(index);
            return true;
        }
        false
    }

    /// `Bucketable.bucketMobPickup` for a sulfur cube
    /// (`SulfurCube.canBePickedUpWithBucket`: an empty bucket), as the water
    /// bucket's mobs are picked up in `interact_mob`.
    pub(in crate::game::server_side) fn bucket_sulfur_cube(&mut self, index: usize) {
        let held = self.common.held().cloned();
        let at = self.common.mobs[index].center();
        self.server_play_sound("minecraft:item.bucket.fill_sulfur_cube", at, 1.0, 1.0);
        // getBucketItemStack, then saveToBucketTag.
        let mut filled = ItemStack::new("minecraft:sulfur_cube_bucket", 1);
        filled.extra = Some(Box::new(sunrise_sim::entity_nbt::bucket_components(
            &self.common.mobs[index],
        )));
        self.trigger_item("minecraft:filled_bucket", &filled);
        // PlayerInteractTrigger: the bucket as it was, and the cube picked up.
        if let Some(stack) = &held {
            let target = self.mob_entity(&self.common.mobs[index]);
            self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
        }
        let id = self.common.mobs[index].id;
        // Leashable.dropLeash, then discard.
        self.break_leash(id, true, false);
        self.remove_mob_now(id, false);
        // ItemUtils.createFilledResult.
        self.filled_result(filled);
        if let (Some(player), Some(name)) = (
            self.server.lan_acting_uuid,
            self.server.lan_acting_name.clone(),
        ) {
            self.native_player_bucket_entity(player, &name, id as i32, "minecraft:sulfur_cube");
        }
    }
    /// The resident sulfur cubes whose adopted body (`adopted_body`, the
    /// body `collectEquipmentChanges` last took) explodes, with its fuse:
    /// their cached `explosionData`, which `primeWhenOnPoweredPosition` may
    /// use this tick although the body has changed since.
    pub fn sulfur_cubes_adopted_explosive(&self) -> Vec<(u32, i32)> {
        self.common
            .mobs
            .iter()
            .filter(|m| m.species() == Species::SulfurCube)
            .filter_map(|m| Some((m.id, sulfur_cube::adopted_explosion(m)?.0)))
            .collect()
    }
    /// `SulfurCube.primeWhenOnPoweredPosition`: an explosive cube standing in
    /// or next to redstone power lights its fuse (`getBestOwnOrNeighbourSignal`;
    /// no block has an own signal, so the neighbour signal is all there is).
    /// Only the cubes `primable` names (an adopted explosive body when the
    /// tick started) can: `canExplode` and `primeTime` both read the cached
    /// `explosionData`, so the fuse is that body's, not the live one's.
    pub fn sulfur_cube_power_priming(&mut self, primable: &[(u32, i32)]) {
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
        {
            return;
        }
        // Vanilla calls primeWhenOnPoweredPosition from SulfurCube.tick,
        // not from a block-change callback. Frozen descendants never poll power.
        let blocked = self.mobs_below_off_level();
        for i in 0..self.common.mobs.len() {
            let m = &self.common.mobs[i];
            if blocked.contains(&m.id)
                || m.species() != Species::SulfurCube
                || !m.alive()
                || sulfur_cube::primed(m)
            {
                continue;
            }
            let Some(&(_, fuse)) = primable.iter().find(|(id, _)| *id == m.id) else {
                continue;
            };
            let at = BlockPos::new(
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            );
            let signal =
                self.with_level(|b, level, ticks| b.best_neighbor_signal(level, ticks, at));
            if signal != 0 {
                sulfur_cube::prime_with_fuse(&mut self.common.mobs[i], false, fuse);
            }
        }
    }
    /// `SulfurCube.hurtServer`: an arrow or a trident on fire that hits an
    /// explosive cube with a block lights its fuse (with `tnt_explodes`, as
    /// `primeTime` asks), called after the hit's hurt, which the priming
    /// leaves as it is, so its knock and the fuse both go ahead.
    pub(in crate::game::server_side) fn burning_arrow_primes(&mut self, p: &Projectile, who: Who) {
        if !p.arrow.as_ref().is_some_and(|a| a.on_fire())
            || !self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
        {
            return;
        }
        if let Who::Mob(id) = who
            && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id)
            && m.species() == Species::SulfurCube
        {
            sulfur_cube::burning_arrow_hit(m);
        }
    }
    /// `SulfurCubeSearchForItemsGoal` and `pickUpItem`: an adult with nothing
    /// inside hops to a block it can swallow within 8 and takes it in.
    pub fn sulfur_cube_pickups(&mut self) {
        let griefing = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING);
        for i in 0..self.common.mobs.len() {
            let m = &self.common.mobs[i];
            if m.species() != Species::SulfurCube || !self.common.creatures.ticked.contains(&m.id) {
                continue;
            }
            if !m.alive() || m.is_baby() || sulfur_cube::has_body(m) || m.extra.ints[3] > 0 {
                self.common.mobs[i].extra.point = None;
                continue;
            }
            let reach = m
                .bounds()
                .inflate(sunrise_core::geometry::Vec3::new(8.0, 8.0, 8.0));
            let pos = m.position;
            let best = self
                .common
                .dropped
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.pickup_delay == 0
                        && reach.contains(e.position)
                        && sulfur_cube::swallowable(m, &e.stack.item)
                })
                .map(|(j, e)| (j, e.position.distance_squared(pos)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((j, _)) = best else {
                self.common.mobs[i].extra.point = None;
                continue;
            };
            // Mob.aiStep: items within the box grown by (1, 0, 1).
            let touch = m
                .bounds()
                .inflate(sunrise_core::geometry::Vec3::new(1.0, 0.0, 1.0));
            let e = &self.common.dropped[j];
            let close = touch.contains(e.position);
            self.common.mobs[i].extra.point = Some([e.position.x, e.position.y, e.position.z]);
            if !griefing || !close {
                continue;
            }
            let thrown_by_player = e
                .thrower
                .is_some_and(|uuid| Some(uuid) == self.common.level.player_uuid);
            let mut swallowed = e.stack.clone();
            swallowed.count = 1;
            let (taken_item, id) = (e.clone(), self.common.mobs[i].id);
            self.server_mob_take_item(&taken_item, id);
            let e = &mut self.common.dropped[j];
            e.stack.count -= 1;
            if e.stack.count == 0 {
                self.remove_dropped(j);
            }
            let m = &mut self.common.mobs[i];
            m.extra.set_body_stack(Some(swallowed.clone()));
            m.extra.point = None;
            self.sulfur_sound(i, "entity.sulfur_cube.absorb");
            if thrown_by_player {
                let target = self.mob_entity(&self.common.mobs[i]);
                self.trigger_item_entity(
                    "minecraft:thrown_item_picked_up_by_entity",
                    &swallowed,
                    &target,
                );
            }
        }
    }
}

impl ServerGame<'_> {
    pub(in crate::game::server_side) fn sulfur_sound(&mut self, index: usize, sound: &str) {
        let at = self.common.mobs[index].center();
        self.server_play_sound(&format!("minecraft:{sound}"), at, 1.0, 1.0);
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::powder_snow::FilledResultGameExt as _;
