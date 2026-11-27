//! Potent sulfur geysers: the tickers `PotentSulfurBlock.getTicker` gives
//! `PotentSulfurBlockEntity` by the block's state. The server's make living
//! entities near the gas nauseous, count a dormant or erupting geyser down to
//! its next change of state and push the entities an eruption throws up. The
//! client's draw the gas clouds and the eruption's plume, play its sound and
//! push the local player, who moves on its own client and not on the server.
use crate::game::ServerGame;
use sunrise_core::{geometry::Aabb, position::BlockPos, random::RandomSource};
use sunrise_game_data::Result;
#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::game::potent_sulfur_common::{
    GEYSER_CONTINUOUS_ACTIVE, GEYSER_ERUPTION_ACTIVE, PassMobs,
};
use sunrise_gameplay_common::potent_sulfur::{
    EFFECT_APPLICATION_FREQUENCY_TICKS, EFFECT_DURATION_IN_TICKS, Geyser, geyser_positional,
    launch_area, nearby_living_area,
};
use sunrise_level::potent_sulfur_level::{
    SulfurLevel, can_be_reached_by_noxious_gas, unobstructed_block_count,
};
use sunrise_level::{block_entities::Pos, containers::BlockEntity, effects::EffectInstance};
use sunrise_world::{level::flags, registry::BlockStateId};

impl ServerGame<'_> {
    /// `PotentSulfurBlock.getTicker` on the server, for the block entity at
    /// `pos`: a wet block gives nausea; a dormant one counts down, then gives
    /// nausea; an erupting one pushes entities, then counts down; a continuous
    /// one pushes entities. A dry block has no ticker.
    pub(in crate::game) fn tick_potent_sulfur(&mut self, pos: Pos) -> Result<()> {
        let Some((state, geyser)) = self.common.potent_sulfur_state(pos) else {
            return Ok(());
        };
        match geyser {
            Geyser::Dry => {}
            Geyser::Wet => self.server_nausea_effect_ticker(pos),
            Geyser::Dormant => {
                self.server_waiting_countdown_ticker(pos, state, geyser)?;
                self.server_nausea_effect_ticker(pos);
            }
            Geyser::Erupting => {
                self.launch_entity_ticker(pos);
                self.server_waiting_countdown_ticker(pos, state, geyser)?;
            }
            Geyser::Continuous => self.launch_entity_ticker(pos),
        }
        Ok(())
    }

    /// `PotentSulfurBlockEntity.SERVER_NAUSEA_EFFECT_TICKER`: every 10 ticks,
    /// the living entities near the source block that its gas reaches (at
    /// their eyes) get nausea for 4 seconds.
    pub fn server_nausea_effect_ticker(&mut self, pos: Pos) {
        if self.common.world.game_time % EFFECT_APPLICATION_FREQUENCY_TICKS != 0 {
            return;
        }
        let Some(source) = self.common.potent_sulfur_source(pos) else {
            return;
        };
        let area = nearby_living_area(source);
        for player in self
            .fight_players()
            .into_iter()
            .filter(|p| p.alive && p.bounds.intersects(area))
        {
            self.as_fight_player(player.who, |g| g.apply_nausea_effect_to_player(source));
        }
        let mut near = Vec::new();
        if !self.mobs_in_pass(area, &mut near) {
            near.extend(0..self.common.mobs.len() as u32);
        }
        let level = SulfurLevel::new(
            &self.common.world,
            self.common.res.facts(),
            self.common.collision,
        );
        // The gas reads blocks only, so the mobs it reaches can be found
        // before any gets its effect.
        near.retain(|&j| {
            let mob = &self.common.mobs[j as usize];
            mob.alive()
                && mob.bounds().intersects(area)
                && can_be_reached_by_noxious_gas(
                    &level,
                    source,
                    [mob.position.x, mob.eye_y(), mob.position.z],
                )
        });
        if near.is_empty() {
            return;
        }
        let reached: Vec<_> = near
            .into_iter()
            .map(|j| self.common.mobs[j as usize].id)
            .collect();
        for id in reached {
            let mut nausea = EffectInstance::new("nausea", EFFECT_DURATION_IN_TICKS, 0);
            nausea.ambient = true;
            self.add_mob_effect(id, nausea, None);
        }
        // Applicability listeners can move or remove mobs: never acknowledge their
        // writes against the pre-hook grid.
        if let Some(pass) = self.server.pass_mobs.as_mut() {
            pass.clear();
        }
    }

    /// The slots of the mobs whose box may meet `area`, from a grid kept for
    /// the block entity pass; false outside the pass: scan them all.
    pub fn mobs_in_pass(&mut self, area: Aabb, out: &mut Vec<u32>) -> bool {
        let Some(pass) = self.server.pass_mobs.as_mut() else {
            return false;
        };
        pass.grid_for(&self.common.mobs).candidates(area, out)
    }

    /// `PotentSulfurBlockEntity.applyNauseaEffect` on the player acting (a
    /// guest is swapped in as the player) when the gas reaches their eyes;
    /// spectators are not affected.
    pub(in crate::game::server_side) fn apply_nausea_effect_to_player(&mut self, source: [i32; 3]) {
        if self.common.spectator() {
            return;
        }
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let eye = [
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        ];
        let reached = {
            let level = SulfurLevel::new(
                &self.common.world,
                self.common.res.facts(),
                self.common.collision,
            );
            can_be_reached_by_noxious_gas(&level, source, eye)
        };
        if reached {
            // MobEffectInstance(NAUSEA, 80, 0, true, true).
            let mut nausea = EffectInstance::new("nausea", EFFECT_DURATION_IN_TICKS, 0);
            nausea.ambient = true;
            self.add_player_effect(nausea);
        }
    }

    /// `PotentSulfurBlockEntity.SERVER_WAITING_COUNTDOWN_TICKER`: every second
    /// the countdown runs down; a dormant geyser waits `10 * (water - 1)` plus
    /// 15 to 30, an erupting one `water - 1` plus 1 or 2, and at zero the
    /// block turns erupting (dormant) or dormant (erupting). `state` is the
    /// block state the tick began with.
    pub(in crate::game::server_side) fn server_waiting_countdown_ticker(
        &mut self,
        pos: Pos,
        state: BlockStateId,
        geyser: Geyser,
    ) -> Result<()> {
        if self.common.world.game_time % 20 != 0 {
            return Ok(());
        }
        let Some(source) = self.common.potent_sulfur_source(pos) else {
            return Ok(());
        };
        let Some(BlockEntity::PotentSulfur(sulfur)) =
            self.common.world.block_entities.get_mut(&pos)
        else {
            return Ok(());
        };
        if sulfur.waiting_countdown <= 0 {
            let water_blocks = source[1] - pos.1 - 1;
            let mut positional = geyser_positional(self.common.level.seed, pos);
            sulfur.waiting_countdown = if geyser == Geyser::Dormant {
                10 * (water_blocks - 1)
                    + positional.next_int_between_inclusive(15, 30).unwrap_or(15)
            } else {
                positional.next_int();
                water_blocks - 1 + positional.next_int_between_inclusive(1, 2).unwrap_or(1)
            };
        }
        if sulfur.waiting_countdown > 0 {
            sulfur.waiting_countdown -= 1;
        }
        if sulfur.waiting_countdown != 0 {
            return Ok(());
        }
        let to_set = if geyser == Geyser::Dormant {
            "erupting"
        } else {
            "dormant"
        };
        let registry = self.common.pack.assets.registry;
        let Some(mut next) = registry.state(state).cloned() else {
            return Ok(());
        };
        next.properties
            .insert("potent_sulfur_state".into(), to_set.into());
        let Some(next) = registry.id(&next) else {
            return Ok(());
        };
        let at = BlockPos::new(pos.0, pos.1, pos.2);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, at, next, flags::UPDATE_ALL));
        self.common.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
        if to_set == "dormant" {
            self.post_block_game_event(
                "minecraft:block_deactivate",
                [pos.0, pos.1, pos.2],
                None,
                Some(state),
            );
        }
        self.server_process_level_events()
    }

    /// `PotentSulfurBlockEntity.LAUNCH_ENTITY_TICKER` on the server: mobs, items,
    /// primed TNT, projectiles, boats and minecarts in the geyser's column with less than
    /// `0.3 + 0.1 * water` upward speed get 0.2 more, unless they ride
    /// something or are of a type geysers do not move. Players are moved by
    /// their own client.
    pub(in crate::game::server_side) fn launch_entity_ticker(&mut self, pos: Pos) {
        let Some(source) = self.common.potent_sulfur_source(pos) else {
            return;
        };
        let water_blocks = source[1] - pos.1 - 1;
        let level = SulfurLevel::new(
            &self.common.world,
            self.common.res.facts(),
            self.common.collision,
        );
        let height = unobstructed_block_count(&level, [pos.0, pos.1 + 1, pos.2], water_blocks);
        let area = launch_area(pos, height);
        let limit = f64::from(0.3f32) + f64::from(water_blocks) * 0.1;
        let push = f64::from(0.2f32);
        let mut near = Vec::new();
        if !self.mobs_in_pass(area, &mut near) {
            near.extend(0..self.common.mobs.len() as u32);
        }
        near.retain(|&j| {
            let m = &self.common.mobs[j as usize];
            m.alive() && m.bounds().intersects(area)
        });
        let tags = self.items().entity_type_tags.clone();
        for &j in &near {
            let mob = &mut self.common.mobs[j as usize];
            // Entity.checkFallDistanceAccumulation.
            if mob.velocity.y > -0.5 && mob.fall_distance > 1.0 {
                mob.fall_distance = 1.0;
            }
            if !mob.extra.passenger
                && !tags.has_bit(
                    mob.kind.flags.entity_tag_slot,
                    sunrise_items::items::EntityTypeTags::NOT_AFFECTED_BY_GEYSERS,
                )
                && mob.velocity.y < limit
            {
                mob.velocity.y += push;
            }
        }
        // A push changes velocities, not boxes: the grid still holds.
        if !near.is_empty()
            && let Some(pass) = self.server.pass_mobs.as_mut()
        {
            pass.wrote(&self.common.mobs);
        }
        for item in self
            .common
            .dropped
            .iter_mut()
            .filter(|i| i.bounds().intersects(area))
        {
            if item.velocity.y < limit {
                item.velocity.y += push;
            }
        }
        for tnt in self
            .common
            .tnts
            .iter_mut()
            .filter(|t| t.bounds().intersects(area))
        {
            if tnt.velocity.y < limit {
                tnt.velocity.y += push;
            }
        }
        // Every entity in the column is launched (`getEntitiesOfClass(Entity.class)`):
        // a projectile in flight or at rest, a boat and a minecart too.
        self.common.creatures.projectiles.for_each_mut(|p| {
            if p.bounds().intersects(area) && p.velocity.y < limit {
                p.velocity.y += push;
                p.needs_sync = true;
            }
        });
        self.common.boats.for_each_mut(|boat| {
            if boat.bounds().intersects(area) && boat.velocity.y < limit {
                boat.velocity.y += push;
            }
        });
        self.common.carts.for_each_mut(|cart| {
            if cart.bounds().intersects(area) && cart.velocity.y < limit {
                cart.velocity.y += push;
            }
        });
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;
