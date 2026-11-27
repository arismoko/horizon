//! Beehives and bee nests in the game (`crate::beehive` holds their logic):
//! their server tick, bees going in, and the bees let out, angry, when a
//! player breaks the hive or fire reaches it.
use super::item_abilities::ability;
use sunrise_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use sunrise_game_data::Result;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::{
    beehive::{self, Beehive, BeehiveExt, Release},
    mobs::{Who, bee},
};
#[allow(unused_imports)]
use {sunrise_core::nbt::Tag, sunrise_world::registry::BlockStateId};

pub(in crate::game) struct GameHive<'g, 'a> {
    pub(in crate::game) game: &'g mut crate::game::ServerGame<'a>,
    /// `bees_stay_in_hive`, once asked: the time and weather it reads hold
    /// for the hive's turn, and every occupant whose time is up asks.
    pub(in crate::game) stay: std::cell::Cell<Option<bool>>,
}
impl<'g, 'a> GameHive<'g, 'a> {
    pub(in crate::game::server_side) fn new(game: &'g mut crate::game::ServerGame<'a>) -> Self {
        Self {
            game,
            stay: Default::default(),
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// Route a target assigned outside the mob tick through the same listener
    /// as AI, before changing the player's binding or the mob's state.
    pub(in crate::game) fn set_event_ai_target(
        &mut self,
        id: u32,
        target: Option<Who>,
        player: Option<u32>,
    ) -> bool {
        if matches!(target, Some(Who::Player))
            && (self.server.lan_acting_fake
                || player.is_some_and(|conn| {
                    self.server
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(conn))
                        .is_some_and(|guest| guest.is_fake())
                }))
        {
            return false;
        }
        let is_brain = self
            .common
            .mobs
            .iter()
            .find(|mob| mob.id == id)
            .is_some_and(|mob| sunrise_sim::mobs::ai::brain_target(mob).is_some());
        let mut chosen = target;
        let mut player_id = (target == Some(Who::Player))
            .then(|| self.player_entity_id_by_connection(player))
            .flatten()
            .and_then(|id| u32::try_from(id).ok());
        let mut redirected = false;
        if self.native_authoritative()
            && mod_api::__private::listened(mod_api::HookId::LivingChangeTarget)
        {
            let Some(epoch) = self.common.native_lifecycle.epoch() else {
                return false;
            };
            let mob = mod_api::MobRef::new(mod_api::EntityRef::new(epoch, id));
            let original = match target {
                None => None,
                Some(Who::Mob(target)) => Some(mod_api::LivingRef::new(mod_api::EntityRef::new(
                    epoch, target,
                ))),
                Some(Who::Player) => {
                    let Some(target) = player_id else {
                        return false;
                    };
                    Some(mod_api::LivingRef::new(mod_api::EntityRef::new(
                        epoch, target,
                    )))
                }
            };
            let Some(result) = self.native_mob_target_event(mob, original, is_brain) else {
                return false;
            };
            if result != original {
                let Some(bound) = self.native_mob_bind_target(id, result) else {
                    return false;
                };
                chosen = bound;
                player_id = result.map(|t| t.entity().id());
                redirected = true;
            }
        }
        if !redirected
            && chosen == Some(Who::Player)
            && let Some(host) = self.server.lan.as_mut()
        {
            match player {
                Some(conn) => {
                    host.mob_players.insert(id, conn);
                }
                None => {
                    host.mob_players.remove(&id);
                }
            }
        }
        let Some(mob) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        if !is_brain || !sunrise_sim::mobs::ai::set_brain_target(mob, chosen, player_id) {
            mob.set_target(chosen);
        }
        true
    }
    pub(in crate::game) fn hive_with_honey(
        &self,
        state: BlockStateId,
        level: i32,
    ) -> Option<BlockStateId> {
        let registry = self.common.pack.assets.registry;
        let mut s = registry.state(state)?.clone();
        s.properties
            .insert("honey_level".to_owned(), level.to_string());
        registry.resolve(&s).ok()
    }
    /// `BeehiveBlockEntity.serverTick` of the hive at `key`, after an
    /// emergency exit if fire is beside it (`BeehiveBlock.updateShape`).
    pub fn tick_beehive(&mut self, key: (i32, i32, i32)) {
        let at = [key.0, key.1, key.2];
        let Some((honey, facing)) = self.common.hive_state(at) else {
            return;
        };
        let Some((lease, BlockEntity::Beehive(mut hive))) =
            self.common.world.block_entities.take(&key)
        else {
            return;
        };
        let fire = [
            [1, 0, 0],
            [-1, 0, 0],
            [0, 1, 0],
            [0, -1, 0],
            [0, 0, 1],
            [0, 0, -1],
        ]
        .iter()
        .any(|d| {
            let state = self
                .common
                .world
                .block(at[0] + d[0], at[1] + d[1], at[2] + d[2]);
            &*self.common.res.facts().get(state).name == "minecraft:fire"
        });
        let mut host = GameHive::new(self);
        if fire && !hive.bees.is_empty() {
            hive.release_all(at, facing, honey, Release::Emergency, &mut host);
        }
        hive.server_tick(at, facing, honey, &mut host);
        self.common
            .world
            .block_entities
            .put(lease, BlockEntity::Beehive(hive));
    }
    /// `BeehiveBlockEntity.addOccupant` for a bee its `BeeEnterHiveGoal`
    /// sent in: off its lead, its tag stored, the bee gone.
    pub fn bee_enter_hive(&mut self, mob: u32, at: [i32; 3]) {
        let key = (at[0], at[1], at[2]);
        if !self
            .common
            .mobs
            .iter()
            .any(|m| m.id == mob && m.alive() && !m.removed)
        {
            return;
        };
        if !matches!(self.world.block_entities.get(&key), Some(BlockEntity::Beehive(h)) if !h.full())
        {
            return;
        }
        // The hive stops the bee riding and ejects its passengers before
        // storing one occupant. Any vetoed edge is asked again at discard.
        self.dismount_mob(mob);
        if let Some(entity) = self.entity(mob) {
            self.entity_eject_passengers(entity);
        }
        self.break_leash(mob, true, false);
        let Some(i) = self
            .common
            .mobs
            .iter()
            .position(|m| m.id == mob && !m.removed)
        else {
            return;
        };
        let tag = self.mob_own_nbt(i);
        let flower = bee::get(&self.common.mobs[i]).and_then(|b| b.flower);
        let kind = self.common.mobs[i].kind.id.clone();
        let Some((lease, BlockEntity::Beehive(mut hive))) =
            self.common.world.block_entities.take(&key)
        else {
            return;
        };
        let stored = hive.add_occupant(at, tag, flower, &mut GameHive::new(self));
        self.common
            .world
            .block_entities
            .put(lease, BlockEntity::Beehive(hive));
        if stored {
            let state = self.common.world.block(at[0], at[1], at[2]);
            self.post_block_game_event(
                "minecraft:block_change",
                at,
                Some(Who::Mob(mob)),
                Some(state),
            );
            let block = self
                .common
                .res
                .facts()
                .get(self.common.world.block(at[0], at[1], at[2]))
                .name
                .clone();
            self.remove_mob_now(mob, false);
            self.native_entity_enter_block(mob, &kind, at, &block);
        }
    }
    /// `BeehiveBlockEntity.emptyAllLivingFromHive` with no player about.
    pub(in crate::game::server_side) fn empty_hive(
        &mut self,
        at: [i32; 3],
        facing: [i32; 2],
        status: Release,
    ) {
        let key = (at[0], at[1], at[2]);
        let Some((lease, BlockEntity::Beehive(mut hive))) =
            self.common.world.block_entities.take(&key)
        else {
            return;
        };
        hive.release_all(at, facing, None, status, &mut GameHive::new(self));
        self.common
            .world
            .block_entities
            .put(lease, BlockEntity::Beehive(hive));
    }
    /// `BeehiveBlock.playerDestroy` without silk touch: the bees come out
    /// angry at the player unless smoke calms them, then
    /// `angerNearbyBees`.
    pub(in crate::game::server_side) fn hive_broken_by_player(
        &mut self,
        at: [i32; 3],
        mut hive: Beehive,
        honey: Option<i32>,
        facing: [i32; 2],
    ) {
        let released = hive.release_all(
            at,
            facing,
            honey,
            Release::Emergency,
            &mut GameHive::new(self),
        );
        let sedated = beehive::smokey(&self.mob_world_base(&[]), at);
        if let Some(p) = self.player_view().map(|p| p.position) {
            let ids: Vec<u32> = self
                .common
                .mobs
                .iter()
                .filter(|m| released.contains(&m.id) && m.distance_sqr(p) <= 16.0)
                .map(|m| m.id)
                .collect();
            for id in ids {
                if sedated {
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                        bee::set_stay_out_of_hive_countdown(m);
                    }
                } else {
                    self.set_event_ai_target(id, Some(Who::Player), self.server.lan_acting);
                }
            }
        }
        self.anger_nearby_bees(at);
    }
    /// `BeehiveBlock.useItemOn` with `held` in the acting hand: shears take
    /// the honeycombs and a glass bottle a honey bottle from a full hive
    /// (honey level 5), then the honey level resets and, unless a campfire
    /// smokes the hive, the bees come out angry. True when it took the click.
    pub fn use_beehive(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> Result<bool> {
        let Some(held) = held else { return Ok(false) };
        let shears = self.stack_shears(held, ability::SHEARS_HARVEST);
        if !shears && &*held.item != "minecraft:glass_bottle" {
            return Ok(false);
        }
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if !matches!(
            &*self.res.facts().get(state).name,
            "minecraft:beehive" | "minecraft:bee_nest"
        ) {
            return Ok(false);
        }
        let prop = |key: &str| {
            self.common
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get(key).cloned())
        };
        if prop("honey_level").and_then(|v| v.parse::<i32>().ok()) != Some(5) {
            return Ok(false);
        }
        let facing = beehive::facing_step(prop("facing").as_deref());
        let at = self
            .common
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        if shears {
            // dropHoneycomb (harvest/beehive), the shear sound, hurtAndBreak.
            let actor = self.interacting_loot_entity();
            for stack in self.interact_loot_at(
                "minecraft:harvest/beehive",
                pos,
                state,
                Some(actor),
                Some(held),
            ) {
                self.pop_resource(pos, stack);
            }
            if let Some(at) = at {
                self.server_play_sound("minecraft:block.beehive.shear", at, 1.0, 1.0);
            };
            self.damage_held(1);
            self.post_block_game_event("minecraft:shear", pos, self.sculk_actor(), None);
        } else {
            // shrink(1), then the honey bottle in the hand, the inventory or
            // on the ground.
            if let Some(at) = at {
                self.server_play_sound("minecraft:item.bottle.fill", at, 1.0, 1.0);
            };
            self.consume_player_slot(self.common.held_slot(), 1);
            let bottle = ItemStack::new("minecraft:honey_bottle", 1);
            if self.common.held().is_none() {
                *self.common.held_mut() = Some(bottle);
            } else if let Some(rest) = self.add_stack_to_inventory(bottle)
                && let Some(eye) = self.common.eye_position()
            {
                // `player.drop(stack, false, PREDICTED)`: thrown from below
                // the eyes along the view, with no thrower.
                self.throw_from_eyes(rest, eye, false, false);
            }
            self.post_block_game_event("minecraft:fluid_pickup", pos, self.sculk_actor(), None);
        }
        self.award_stat("used", &held.item, 1);
        let smokey = beehive::smokey(&self.mob_world_base(&[]), pos);
        let key = (pos[0], pos[1], pos[2]);
        if !smokey
            && matches!(self.world.block_entities.get(&key), Some(BlockEntity::Beehive(h)) if !h.bees.is_empty())
        {
            self.anger_nearby_bees(pos);
        }
        // resetHoneyLevel.
        if let Some(reset) = self.common.with_props(state, &[("honey_level", "0")]) {
            self.level_set(pos, reset)?;
        }
        if !smokey
            && let Some((lease, BlockEntity::Beehive(mut hive))) =
                self.common.world.block_entities.take(&key)
        {
            // emptyAllLivingFromHive: the bees within 4 blocks of the player
            // go for them.
            let released = hive.release_all(
                pos,
                facing,
                None,
                Release::Emergency,
                &mut GameHive::new(self),
            );
            self.common
                .world
                .block_entities
                .put(lease, BlockEntity::Beehive(hive));
            if let Some(p) = self.player_view().map(|p| p.position) {
                let ids: Vec<u32> = self
                    .common
                    .mobs
                    .iter()
                    .filter(|m| released.contains(&m.id) && m.distance_sqr(p) <= 16.0)
                    .map(|m| m.id)
                    .collect();
                for id in ids {
                    self.set_event_ai_target(id, Some(Who::Player), self.server.lan_acting);
                }
            }
        }
        // ServerPlayerGameMode.useItemOn: the action was consumed.
        self.trigger_block("minecraft:item_used_on_block", pos, held);
        Ok(true)
    }
    /// `BeehiveBlock.angerNearbyBees`: bees within 8 blocks (6 up and down)
    /// without a target go for a random player in the same box, the window's
    /// own or a guest (`None` is the window's player).
    pub(in crate::game::server_side) fn anger_nearby_bees(&mut self, at: [i32; 3]) {
        let min = Vec3::new(
            f64::from(at[0]) - 8.0,
            f64::from(at[1]) - 6.0,
            f64::from(at[2]) - 8.0,
        );
        let area = Aabb::new(min, Vec3::new(min.x + 17.0, min.y + 13.0, min.z + 17.0));
        let mut players: Vec<Option<u32>> = Vec::new();
        if self
            .player_view()
            .is_some_and(|p| p.bounds.intersects(area))
        {
            players.push(None);
        }
        if let Some(host) = self.server.lan.as_ref() {
            players.extend(
                host.hub
                    .guests
                    .iter()
                    .filter(|g| host.hub.here(g) && g.loaded && !g.dead && !g.is_fake())
                    .filter(|g| super::lan_mobs::guest_bounds(g).intersects(area))
                    .map(|g| Some(g.conn)),
            );
        }
        if players.is_empty() {
            return;
        }
        let mut bees: Vec<usize> = (0..self.common.mobs.len())
            .filter(|&i| {
                let m = &self.common.mobs[i];
                m.species() == sunrise_sim::mobs::Species::Bee
                    && !m.removed
                    && m.bounds().intersects(area)
            })
            .collect();
        bees.retain(|&i| self.common.mobs[i].ai.target.is_none());
        for i in bees {
            // Util.getRandom over the players.
            let pick = self
                .common
                .world
                .random
                .next_int_bound(players.len() as i32)
                .unwrap_or(0) as usize;
            self.set_event_ai_target(self.common.mobs[i].id, Some(Who::Player), players[pick]);
        }
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_abilities::ItemAbilitiesGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
