//! `AbstractThrownPotion`: what a splash or lingering potion does around where
//! it lands besides its effects. A potion in `#minecraft:douses_fire` puts out
//! the fire, candles and campfires next to the block it hit; one in
//! `#minecraft:hurts_water_sensitive_entities` hurts the water-sensitive mobs
//! near it and one in `#minecraft:extinguishes_entities` puts out burning
//! entities. Only plain water is in those tags in vanilla.
use crate::game::ServerGame;
use sunrise_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
};
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::{Damage, Species, Who};
use sunrise_world::{chunk::BlockState, registry::BlockStateId};

impl ServerGame<'_> {
    /// `PotionContents.is`: the potion of `stack` is in the potion tag `tag`
    /// (`minecraft:douses_fire` and the others, read from the data pack's
    /// `tags/potion`) and the stack has no custom effects of its own
    /// (`isPotionWithoutCustomEffects`). A stack with no potion is in no tag.
    pub(in crate::game::server_side) fn potion_in_tag(&self, stack: &ItemStack, tag: &str) -> bool {
        stack.custom_potion_effects().is_empty()
            && stack.potion().is_some_and(|potion| {
                self.common
                    .res
                    .stations()
                    .trades
                    .potion_tags
                    .get(tag)
                    .is_some_and(|members| members.iter().any(|m| m.as_str() == potion))
            })
    }

    /// `AbstractThrownPotion.onHitBlock` on the server: a potion (the stack's
    /// `potion_contents`) in `#douses_fire` douses the cell in front of the
    /// face it hit, the cell behind that (the block it hit) and the four
    /// horizontal neighbours of the cell in front. `hit` is the block and
    /// `face` the face it was hit on.
    pub(in crate::game::server_side) fn potion_hit_block(
        &mut self,
        stack: &ItemStack,
        hit: [i32; 3],
        face: Direction,
        owner: Option<Who>,
    ) {
        // `PotionContents.is(PotionTags.DOUSES_FIRE)` on the stack's `potion_contents`.
        if !self.potion_in_tag(stack, "minecraft:douses_fire") {
            return;
        }
        let relative = |pos: [i32; 3], direction: Direction| {
            let n = direction.normal();
            [pos[0] + n[0], pos[1] + n[1], pos[2] + n[2]]
        };
        let effect = relative(hit, face);
        self.douse_fire(effect, owner);
        self.douse_fire(relative(effect, face.opposite()), owner);
        // Direction.Plane.HORIZONTAL.
        for direction in [
            Direction::North,
            Direction::East,
            Direction::South,
            Direction::West,
        ] {
            self.douse_fire(relative(effect, direction), owner);
        }
    }

    /// `AbstractThrownPotion.douseFire`: a fire block is destroyed without
    /// drops, a lit candle or candle cake goes out, and a lit campfire goes
    /// out after level event 1009 and its block-change game event.
    pub fn douse_fire(&mut self, pos: [i32; 3], owner: Option<Who>) {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let name = self.common.res.facts().get(state).name.clone();
        let fire = self.named_block_in_tag("minecraft:fire", &name);
        if fire {
            // Level.destroyBlock(pos, false, this).
            self.with_level(|b, level, ticks| b.destroy_block(level, ticks, at, false));
        }
        let candle =
            !fire && self.with_level(|b, level, ticks| b.extinguish_candle(level, ticks, at));
        if !fire
            && !candle
            && let Some(mut lit) = self.lit_campfire(state, &name)
        {
            self.server_level_event_at(1009, pos, 0);
            // `CampfireBlock.douse(entity, ...)`: the block-change game event,
            // sourced from the potion's owner as vanilla passes `getOwner()`.
            self.post_block_game_event("minecraft:block_change", pos, owner, None);
            lit.properties.insert("lit".into(), "false".into());
            if let Ok(unlit) = self.common.pack.assets.registry.resolve(&lit) {
                let _ = self.level_set(pos, unlit);
            }
        }
        let _ = self.server_process_level_events();
    }

    /// `CampfireBlock.isLitCampfire`: the state of a lit block in
    /// `#minecraft:campfires`.
    pub fn lit_campfire(&self, state: BlockStateId, name: &str) -> Option<BlockState> {
        if !self.named_block_in_tag("minecraft:campfires", name) {
            return None;
        }
        self.common
            .pack
            .assets
            .registry
            .state(state)
            .filter(|s| s.properties.get("lit").map(String::as_str) == Some("true"))
            .cloned()
    }

    /// `AbstractThrownPotion.affectEntitiesAround`: the living entities whose
    /// box meets the potion's box grown by (4, 2, 4) and that are less than 4
    /// blocks from it. A potion in `#hurts_water_sensitive_entities` hurts
    /// the water-sensitive ones for 1 (`indirectMagic`, the owner being a
    /// player when `by_player`), an Enderman then trying to teleport
    /// (`Enderman.hurtServer`); one in `#extinguishes_entities` puts out
    /// those that burn; one in `#rehydrates_axolotls` refills the air of
    /// the axolotls in the box.
    pub fn affect_entities_around(&mut self, stack: &ItemStack, at: Vec3, by_player: bool) {
        let hurts = self.potion_in_tag(stack, "minecraft:hurts_water_sensitive_entities");
        let extinguishes = self.potion_in_tag(stack, "minecraft:extinguishes_entities");
        // The potion's box is 0.25 wide and high.
        let reach = Aabb::new(
            Vec3::new(at.x - 0.125, at.y, at.z - 0.125),
            Vec3::new(at.x + 0.125, at.y + 0.25, at.z + 0.125),
        )
        .inflate(Vec3::new(4.0, 2.0, 4.0));
        let near = |p: Vec3| {
            let (dx, dy, dz) = (p.x - at.x, p.y - at.y, p.z - at.z);
            dx * dx + dy * dy + dz * dz < 16.0
        };
        if hurts || extinguishes {
            let potion = if &*stack.item == "minecraft:lingering_potion" {
                "minecraft:lingering_potion"
            } else {
                "minecraft:splash_potion"
            };
            let source = Damage::indirect_magic(
                potion,
                by_player.then_some(Who::Player),
                [at.x, at.y, at.z],
            );
            let attacker = by_player.then_some(Who::Player);
            let attack = self.attack_of_who(attacker);
            for index in 0..self.common.mobs.len() {
                let m = &self.common.mobs[index];
                // WATER_SENSITIVE_OR_ON_FIRE.
                let sensitive = m.kind.flags.water_sensitive;
                if m.removed
                    || !(sensitive || m.is_on_fire())
                    || !reach.intersects(m.bounds())
                    || !near(m.position)
                {
                    continue;
                }
                if hurts && sensitive {
                    let mob_id = m.id;
                    let invulnerable = m.invulnerable_flag;
                    let plan = self.prepare_knockback_credit(mob_id as i32, attacker, &attack);
                    let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                    let m = &mut self.common.mobs[index];
                    m.damage(
                        crate::game::server_side::entity_server::damage_level_of!(
                            self,
                            &attack,
                            &mut credit,
                            None
                        ),
                        1.0,
                        source,
                    );
                    self.complete_mob_hurt(index, false);
                    let Some(m) = self.common.mobs.get_mut(index) else {
                        continue;
                    };
                    // `Enderman.hurtServer` for a thrown potion: the hit is
                    // `Enderman.hurtWithCleanWater`, then
                    // `Enderman.repeatedlyTryToTeleport`, which does nothing
                    // while it rides or once it is dead (`teleport` needs
                    // `isAlive`) and which the AI step runs.
                    if m.species() == Species::Enderman
                        && !invulnerable
                        && m.alive()
                        && !m.extra.passenger
                    {
                        m.extra.counter2 = 64;
                    }
                    self.finish_mob_damage_at(index);
                }
                let m = &mut self.common.mobs[index];
                if extinguishes && m.is_on_fire() && m.alive() {
                    m.extinguish_fire();
                }
            }
        }
        if extinguishes {
            self.for_each_player(|g| g.extinguish_player_in_splash(reach, at));
            // Armor stands are living entities too.
            let mut put_out = Vec::new();
            for stand in self.common.armor_stands.iter_mut() {
                if stand.fire_ticks > 0
                    && stand.health > 0.5
                    && reach.intersects(stand.bounds())
                    && near(stand.position)
                {
                    stand.fire_ticks = 0;
                    put_out.push([stand.position.x, stand.position.y, stand.position.z]);
                }
            }
            for stand_at in put_out {
                self.extinguish_sound(stand_at);
            }
        }
        if self.potion_in_tag(stack, "minecraft:rehydrates_axolotls") {
            // Axolotl.rehydrate: 1800 more air, up to the maximum.
            let (lo, hi) = (
                [at.x - 0.125 - 4.0, at.y - 2.0, at.z - 0.125 - 4.0],
                [at.x + 0.125 + 4.0, at.y + 0.25 + 2.0, at.z + 0.125 + 4.0],
            );
            for m in self
                .common
                .mobs
                .iter_mut()
                .filter(|m| m.species() == Species::Axolotl && !m.removed)
            {
                let (w, h, _) = m.dims();
                let half = w / 2.0;
                let (x, y, z) = (m.position.x, m.position.y, m.position.z);
                if x - half < hi[0]
                    && x + half > lo[0]
                    && y < hi[1]
                    && y + h > lo[1]
                    && z - half < hi[2]
                    && z + half > lo[2]
                {
                    m.air = (m.air + 1800).min(sunrise_sim::mobs::axolotl::MAX_AIR);
                }
            }
        }
    }

    /// `Entity.extinguishFire` for the player in place (the host's or a
    /// guest's) when it burns, is alive and is in a splash's reach: the
    /// extinguish sound, then the fire out.
    fn extinguish_player_in_splash(&mut self, reach: Aabb, at: Vec3) {
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let w = f64::from(p.dimensions.width) / 2.0;
        let bounds = Aabb::new(
            Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
            Vec3::new(
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ),
        );
        let (dx, dy, dz) = (
            p.position.x - at.x,
            p.position.y - at.y,
            p.position.z - at.z,
        );
        if p.fire_ticks <= 0
            || p.health.dead_or_dying()
            || !reach.intersects(bounds)
            || dx * dx + dy * dy + dz * dz >= 16.0
        {
            return;
        }
        let feet = [p.position.x, p.position.y, p.position.z];
        self.extinguish_sound(feet);
        if let Some(p) = self.common.player.as_mut() {
            let ticks = p.fire_ticks.min(0);
            p.set_remaining_fire_ticks(ticks);
        }
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;
