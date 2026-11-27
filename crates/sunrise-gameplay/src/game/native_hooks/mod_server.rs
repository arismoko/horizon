use self::block_entity_tick::ResidentBlockEntityAccess;
use crate::game::ServerGame;
use mod_api::BlockPlacement;
pub use sunrise_gameplay_common::game::native_hooks::mob_kind;
pub use sunrise_gameplay_common::game::native_hooks::*;
use sunrise_level::containers::ModBlockEntity as EngineBlockEntity;
use sunrise_sim::block_contact::Pending;
use sunrise_world::level::PlaceContext;
use {
    sunrise_level::containers::ModContainer, sunrise_level::containers::ModEnergyBuffer,
    sunrise_level::containers::ModFluidTanks, sunrise_sim::merchant::MerchantOffer,
};

impl mod_api::PackDiscoveryContext for ServerGame<'_> {
    fn live_host(&mut self) -> Option<mod_api::HostSide<'_>> {
        Some(mod_api::HostSide::Server(self))
    }

    fn client_text(&self) -> Option<&dyn mod_api::prelude::ClientTextClientHost> {
        None
    }
}

#[path = "base_server.rs"]
pub mod base;

#[path = "block_entity_on_load_server.rs"]
pub mod block_entity_on_load;

#[path = "block_entity_save_additional_tests_server.rs"]
pub mod block_entity_save_additional_tests;

#[path = "block_entity_tick_server.rs"]
pub mod block_entity_tick;

#[path = "build_creative_mode_tab_contents_server.rs"]
pub mod build_creative_mode_tab_contents;

#[path = "can_start_swimming_server.rs"]
pub mod can_start_swimming;

#[path = "contents_changed_server.rs"]
pub(in crate::game) mod contents_changed;

#[path = "enchanted_block_loot_server.rs"]
pub mod enchanted_block_loot;

#[path = "exp_drop_server.rs"]
pub mod exp_drop;

#[path = "explosion_notice_containment_server.rs"]
pub mod explosion_notice_containment;

#[path = "general_entity_tick_server.rs"]
pub mod general_entity_tick;

#[path = "incoming_tests_server.rs"]
pub mod incoming_tests;

#[path = "item_attack_server.rs"]
pub mod item_attack;

#[path = "level_world_server.rs"]
pub mod level_world;

#[path = "living_death_server.rs"]
pub mod living_death;

#[path = "mod_mismatch_server.rs"]
pub mod mod_mismatch;

#[path = "on_chunk_unloaded_server.rs"]
pub mod on_chunk_unloaded;

#[path = "on_entity_item_update_server.rs"]
pub mod on_entity_item_update;

#[path = "player_bed_leave_server.rs"]
pub mod player_bed_leave;

#[path = "player_leave_server.rs"]
pub mod player_leave;

pub mod register_enchantment_effects;
pub mod register_mob_parts;

#[path = "shared/item_fallthrough_server.rs"]
pub mod item_fallthrough;

#[path = "shared/loot_server.rs"]
pub mod loot;

#[path = "shared/world_helpers_server.rs"]
pub mod world_helpers;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// `AnimalTameEvent` for the mob at `index`, whose taming roll by the
    /// player `tamer` (`tamer_entity_id`) succeeded: true when a mod cancels
    /// it, and the taming fails.
    pub(in crate::game) fn animal_tame_cancelled(
        &mut self,
        index: usize,
        tamer: u128,
        tamer_entity_id: i32,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::AnimalTame) {
            return false;
        }
        let m = &self.common.mobs[index];
        let (entity, kind) = (m.id, mob_kind(m));
        let mut cancel = false;
        self.native_animal_tame(tamer, tamer_entity_id, entity, &kind, &mut cancel);
        cancel
    }
    /// The [`Attack`] of `attacker` (an entity id, 0 for none) for the level a hit lands in
    /// ([`DamageLevel::attacked_by`]). Record the attacker's hand state only if a
    /// subscribed hit, death or loot hook may read it, or when the main hand carries
    /// `minecraft:armor_effectiveness`, which the hit reads as its weapon. Capture before
    /// the level borrows the game so nested damage can't substitute a different attacker's
    /// hands.
    pub fn attack_of(&self, attacker: i32) -> Attack {
        let listens = [
            mod_api::HookId::LivingIncomingDamage,
            mod_api::HookId::LivingKnockback,
            mod_api::HookId::LivingDeath,
            mod_api::HookId::LootDrops,
        ]
        .into_iter()
        .any(mod_api::__private::listened);
        let enchantments = &server_world_stations!(self).enchantments;
        let main = |stack: Option<&sunrise_items::items::ItemStack>| {
            stack
                .filter(|stack| {
                    listens || enchantments.has_component(stack, "minecraft:armor_effectiveness")
                })
                .cloned()
        };
        let held = if self
            .common
            .player
            .as_ref()
            .is_some_and(|p| p.entity_id == attacker)
        {
            [
                main(self.common.stack_in(self.common.selected)),
                listens.then(|| self.common.offhand.clone()).flatten(),
            ]
        } else if let Some(index) = self
            .common
            .mob_index
            .find_ref(&self.common.mobs, attacker as u32)
        {
            let mob = &self.common.mobs[index];
            [main(mob.extra.main_hand_stack()), None]
        } else if let Some(guest) = self
            .server
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|g| g.entity_id == attacker))
        {
            [
                main(guest.inventory.get(guest.selected).and_then(Option::as_ref)),
                listens.then(|| guest.offhand.clone()).flatten(),
            ]
        } else {
            [None, None]
        };
        let player_uuid = if self
            .common
            .player
            .as_ref()
            .is_some_and(|p| p.entity_id == attacker)
        {
            self.player_uuid_of(self.server.lan_acting)
        } else {
            self.server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().find(|g| g.entity_id == attacker))
                .map(|g| sunrise_sim::lan::playerdata::uuid_ints(g.uuid))
        };
        Attack {
            attacker,
            held,
            projectile: None,
            projectile_kind: sunrise_sim::health::DirectEntity::Other,
            projectile_type: None,
            direct: None,
            direct_kind: None,
            direct_type: None,
            position: None,
            projectile_weapon: None,
            attacker_uuid: (player_uuid.is_none() && attacker != 0)
                .then(|| self.known_attacker_uuid(attacker as u32))
                .flatten(),
            player_uuid,
            enchantment_attacker: self.enchantment_actor(attacker),
            enchantment_direct: None,
        }
    }
    /// Finish a lethal or totem-protected hit only once the owning level has settled it.
    pub(crate) fn finish_damage_post(
        &mut self,
        entity: i32,
        kind: &str,
        post: std::sync::Arc<sunrise_sim::health::DamagePost>,
    ) {
        use sunrise_gameplay_common::game::transactions::entities::EntityRemovalGameExt as _;
        use sunrise_sim::health::{HitLend, LivingBody, PlayerBody, PlayerHealthRead};
        let (source, state, original_lend) = post.view();
        // Check removal before borrowing the victim and the lent attacker.
        let dragon_victim = self
            .common
            .dragon
            .as_ref()
            .filter(|d| d.id as i32 == entity)
            .is_some_and(|d| !self.target_removed(&crate::game::commands::Target::Dragon(d.id)));
        let victim_mob = self.common.mobs.iter().position(|m| m.id as i32 == entity);
        // The Post record exists only for a Post subscriber, and names an attacker only if the hit
        // lent one: it carries the setup's decision.
        let lend_mob = original_lend
            .and_then(|id| self.common.mobs.iter().position(|m| m.id as i32 == id))
            .filter(|&index| Some(index) != victim_mob);
        // A parked guest the hit lent, lent again if its reads still answer from its record.
        let parked = original_lend
            .filter(|_| lend_mob.is_none())
            .and_then(|id| self.native_parked_causer(id));
        let (victim_mob, lend_mob) = match (victim_mob, lend_mob) {
            (Some(v), Some(l)) => {
                let [v, l] = self
                    .common
                    .mobs
                    .get_disjoint_mut([v, l])
                    .expect("two different mobs");
                (Some(&*v), Some(l))
            }
            (None, Some(l)) => (None, Some(&mut self.common.mobs[l])),
            (Some(v), None) => (Some(&self.common.mobs[v]), None),
            (None, None) => (None, None),
        };
        let player_victim = self
            .common
            .player
            .as_ref()
            .is_some_and(|p| p.entity_id == entity);
        let hub = self.server.lan.as_ref().map(|lan| &lan.hub);
        let parked = move || HitLend::parked_guest(hub, parked?);
        let (victim, lend) = if player_victim {
            let p = self.common.player.as_ref().expect("the player victim");
            let body = LivingBody::Player(PlayerBody {
                id: p.entity_id,
                uuid: None,
                health: PlayerHealthRead::Synced(&p.health.data, p.health.ids),
                attributes: Some(&p.attributes),
                effects: Some(&self.common.potions.effects),
                equipment: acting_equipment!(self.common),
                attachments: None,
            });
            (Some(body), lend_mob.and_then(HitLend::mob).or_else(parked))
        } else {
            let lend = match lend_mob {
                Some(mob) => HitLend::mob(mob),
                None => original_lend
                    .and_then(|id| {
                        HitLend::attacking_player(
                            self.common.player.as_mut(),
                            id,
                            &self.common.potions.effects,
                            acting_equipment!(self.common),
                        )
                    })
                    .or_else(parked),
            };
            let victim = victim_mob.map(LivingBody::Mob).or_else(|| {
                dragon_victim
                    .then(|| self.common.dragon.as_ref())
                    .flatten()
                    .map(sunrise_gameplay_common::game::native_living_body::dragon_body)
            });
            (victim, lend)
        };
        let level = crate::game::server_side::entity_server::damage_level_of!(
            self,
            &sunrise_sim::health::NO_ATTACK,
            no_push,
            lend
        );
        let level = match victim {
            Some(victim) => level.reborrow().with_victim(victim),
            None => level.reborrow(),
        };
        let live = LiveDamageRead(state);
        HitLevel(level.caused_by(source.causing)).native_living_damage_post(
            entity,
            kind,
            &source,
            mod_api::DamageContainerView::new(&live),
        );
    }
    /// `MobEvent::PlaceBlock`: a mob sets `state` at `pos` unless a mod
    /// cancels `entity_place`. Enderman attempts use `enderman_place_block`.
    pub(in crate::game) fn mob_place_block(
        &mut self,
        mob: u32,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> sunrise_game_data::Result<()> {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let against = self.common.world.block(pos[0], pos[1] - 1, pos[2]);
        let kind = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .map(|m| m.kind.id.clone());
        if self.place_canceled(mob, at, state, against, None) {
            return Ok(());
        }
        let facts = self.common.res.facts();
        let from = facts
            .get(self.common.world.block(pos[0], pos[1], pos[2]))
            .name
            .clone();
        let to = facts.get(state).name.clone();
        self.level_set(pos, state)?;
        if let Some(kind) = kind {
            self.native_entity_change_block(mob, &kind, pos, &from, &to);
        }
        Ok(())
    }
    /// Menu button `id` on a mod menu (`clickMenuButton`), with the acting
    /// player's cursor stack (`carried`, read and written). A block-backed
    /// menu lends its live resident block entity through `BlockEntityScope`;
    /// a blockless menu lends no view while keeping `host()` and
    /// `transaction(...)` usable.
    pub fn mod_menu_button(
        &mut self,
        menu: &str,
        context: mod_api::item::menu::MenuButtonContext,
        block: Option<&str>,
        backing: Option<(i32, i32, i32)>,
        carried: &mut Option<ItemStack>,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuButton) {
            return;
        }
        let Ok(mut api_cursor) = carried.try_into_api() else {
            return;
        };
        let original_api = api_cursor.clone();
        let target_pos = backing.filter(|&pos| self.common.mod_menu_at(pos) == Some(menu));
        let target = target_pos.and_then(|pos| self.resident_block_entity_target(pos));
        let items_changed = {
            let mut access = match target {
                Some((handle, state)) => ResidentBlockEntityAccess::for_handle(self, handle, state),
                None => ResidentBlockEntityAccess::blockless(self),
            };
            sunrise_gameplay_common::game::native_hooks::native_menu_button(
                menu,
                block,
                &context,
                mod_api::BlockEntityScope::new(&mut access),
                &mut api_cursor,
            );
            access.items_changed()
        };
        if items_changed
            && let Some((handle, _)) = target
            && let Some(pos) = self.native_block_entity_write_key(handle)
        {
            self.after_commit(sunrise_gameplay_common::game::transactions::After::Changed(
                pos,
            ));
        }
        apply_menu_button_cursor(self.items(), &original_api, &api_cursor, carried);
    }
    /// The slot a player's carried stack is in, by `native_inventory_tick`'s
    /// numbers: none for another number.
    pub(in crate::game) fn native_carried_slot(
        &mut self,
        slot: i32,
    ) -> Option<&mut Option<ItemStack>> {
        match slot {
            0.. if (slot as usize) < self.common.inventory.len() => {
                self.common.inventory.get_mut(slot as usize)
            }
            100..=103 => self.common.armor.get_mut(103 - slot as usize),
            -106 => Some(&mut self.common.offhand),
            _ => None,
        }
    }
    pub(in crate::game) fn server_acting_id(&mut self) -> u128 {
        sunrise_sim::lan::playerdata::ints_uuid(self.acting_player_uuid())
    }
    pub(in crate::game) fn server_clicker(&mut self) -> (u128, i32) {
        let entity = self
            .server
            .lan_acting_entity
            .map_or_else(|| self.own_entity_id(), |id| id as i32);
        (self.server_acting_id(), entity)
    }
    pub(in crate::game) fn server_acting_or_local_player(&self) -> Option<PlayerRef> {
        self.native_acting_player()
    }
    pub(in crate::game) fn server_chunks_loaded(&mut self) {
        self.fire_pending_block_entity_data_loads();
        let loaded = std::mem::take(&mut self.common.world.loaded_chunks);
        let dimension = self.common.dim.id();
        for (x, z, new) in loaded {
            self.native_chunk_load(dimension, [x, z], new);
        }
    }
    pub(in crate::game) fn server_chunks_unloaded(
        &mut self,
        unloaded: &[std::sync::Arc<sunrise_level::chunk::Chunk>],
    ) {
        let dimension = self.common.dim.id();
        for chunk in unloaded {
            self.native_chunk_and_block_entities_unload(dimension, [chunk.x, chunk.z]);
        }
    }
    pub(in crate::game) fn server_apply_after_use_components(&mut self, stack: &ItemStack) {
        self.apply_after_use_components(stack);
    }
    /// Apply one already-validated engine write, at the block the entity landed on.
    /// The original-state guard runs once before the ordered batch, not between writes.
    pub(in crate::game) fn apply_landing_write(
        &mut self,
        level: mod_api::LevelRef,
        pos: BlockPos,
        write: sunrise_world::level::LandingWrite,
    ) {
        let at = [pos.x, pos.y, pos.z];
        match write {
            // `Level.setBlockAndUpdate`: flag 3, the neighbours and the clients.
            sunrise_world::level::LandingWrite::SetBlock(state) => {
                let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
                let _ = self.level_set_block(level, at, state, flags);
            }
            sunrise_world::level::LandingWrite::DestroyBlock { drop } => {
                self.level_destroy_block(level, at, drop, None, None, false);
            }
            // A write a later API adds that this game does not know is not applied.
            _ => {}
        }
    }
    /// [`Self::attack_of`] for the attacker a damage source names: the player, a mob, or (none)
    /// the attack the enclosing [`Self::with_attack`] lends.
    pub fn attack_of_who(&self, attacker: Option<Who>) -> Attack {
        match attacker {
            Some(Who::Player) => {
                self.attack_of(self.common.player.as_ref().map_or(0, |p| p.entity_id))
            }
            Some(Who::Mob(id)) => self.attack_of(id as i32),
            None => self.current_attack(),
        }
    }
    /// Deliver what the batch just ticked recorded, in order: `minecart_pass`
    /// to the parts, and the writes a landing (`Block.fallOn`) or step (`Block.stepOn`)
    /// asked for to the level. A write is skipped when the block at its position is no
    /// longer the state it touched, as vanilla never sees a stale one.
    pub fn drain_entity_inside(&mut self) {
        for contact in sunrise_sim::block_contact::take_pending() {
            let (state, pos) = match &contact {
                Pending::Landed(landed) => (landed.state, landed.position),
                Pending::MinecartPass(pass) => (pass.state, pass.position),
            };
            let Some(level) = self.level(self.common.dim.id()) else {
                continue;
            };
            if self.level_block_state(level, [pos.x, pos.y, pos.z]) != Some(state) {
                continue;
            }
            match contact {
                Pending::MinecartPass(pass) => {
                    if let Some(entity) = u32::try_from(pass.entity)
                        .ok()
                        .and_then(|id| self.entity(id))
                    {
                        self.native_minecart_pass(state, pos, entity);
                    }
                }
                Pending::Landed(landed) => {
                    for write in landed.writes {
                        self.apply_landing_write(level, pos, write);
                    }
                }
            }
        }
    }
    /// The mobs from index `from` on are new in the world.
    pub fn admit_new_mobs(&mut self, from: usize) {
        // Preserve saved or constructed UUIDs. Insertion paths without an
        // identity still need one before their join listeners run.
        for mob in self.common.mobs.iter().skip(from) {
            self.common
                .persist
                .uuids
                .get_or_insert_with(mob.id, sunrise_sim::save::random_uuid);
        }
        self.flush_pending_mob_parts();
        self.join_spawn_tags(from);
        self.native_joins_flush(false, 0);
    }
    pub(in crate::game) fn placement_canceled(
        &mut self,
        placer: Option<EntityRef>,
        pos: BlockPos,
        state: BlockStateId,
        placed_against: BlockStateId,
        extra: impl IntoIterator<Item = (BlockPos, BlockStateId)>,
    ) -> bool {
        let mut extra = extra.into_iter();
        let placement = if let Some(first) = extra.next() {
            let Some(writes) = std::iter::once((pos, state))
                .chain(std::iter::once(first))
                .chain(extra)
                .map(|(at, state)| Some(BlockPlacement::new(at, state.try_into_api().ok()?)))
                .collect()
            else {
                return false;
            };
            Placement::Multi(writes)
        } else {
            Placement::Single
        };
        let mut canceled = false;
        self.native_entity_place(placer, pos, state, placed_against, placement, &mut canceled);
        canceled
    }
    /// Let `entity_place` veto exactly the decision the world will apply. If the
    /// callback changed the inputs, do not choose another target behind its back.
    pub(in crate::game) fn place_item_block(
        &mut self,
        ctx: &PlaceContext,
        by_player: bool,
        stack: &ItemStack,
    ) -> sunrise_game_data::Result<Option<sunrise_world::level::AppliedPlacement>> {
        let registry = self.common.pack.assets.registry;
        let game_master = ctx.can_use_game_master_blocks;
        let stack_state = |state| {
            sunrise_gameplay_common::game::block_items::item_block_state(
                registry,
                state,
                stack,
                game_master,
            )
        };
        let has_part =
            mod_api::__private::item_listens(mod_api::HookId::PlacementContext, &stack.item);
        let placer = (has_part && by_player)
            .then(|| self.placement_placer())
            .flatten();
        let Some((decision, preview)) = self.with_level(|b, level, ticks| {
            b.decide_placement(level, ticks, ctx, stack_state, |level, context| {
                has_part
                    .then(|| level.rewrite_placement_context(stack, context, placer))
                    .flatten()
            })
        }) else {
            return Ok(None);
        };
        let veto = mod_api::__private::listened(mod_api::HookId::EntityPlace);
        if veto {
            let against_pos = decision.placed_against_pos();
            let block = |game: &crate::game::ServerGame<'_>, pos: BlockPos| {
                game.world.block(pos.x, pos.y, pos.z)
            };
            let against = block(self, against_pos);
            let target_before = block(self, preview.pos);
            // One proposed state per affected cell. A companion at the pre-clear
            // position supersedes the intermediate air or water write.
            let extra = preview.companion.into_iter().chain(
                preview
                    .pre_clear
                    .filter(|(pos, _)| preview.companion.is_none_or(|(at, _)| at != *pos)),
            );
            let hand = if by_player {
                self.common.held().cloned()
            } else {
                None
            };
            let placer = by_player
                .then(|| self.native_clicker().1.cast_unsigned())
                .and_then(|entity| self.entity(entity));
            // The block revision and journal epoch cover supporting blocks and
            // block-entity shapes without tracing ordinary reads.
            let level = (
                self.common.dim,
                self.common.world.revision,
                self.common.transactions.writes(),
            );
            if self.placement_canceled(placer, preview.pos, preview.state, against, extra)
                || level
                    != (
                        self.common.dim,
                        self.common.world.revision,
                        self.common.transactions.writes(),
                    )
                || block(self, preview.pos) != target_before
                || block(self, against_pos) != against
                || (by_player && self.common.held() != hand.as_ref())
            {
                return Ok(None);
            }
        }
        let Some(placed) = self
            .with_level(|b, level, ticks| b.apply_placement(level, ticks, decision, stack_state))
        else {
            return Ok(None);
        };
        if placed.matches_candidate {
            let pos = placed.context.pos;
            let target = [pos.x, pos.y, pos.z];
            self.apply_block_item_data(target, placed.state, stack, game_master);
            self.with_level(|b, level, ticks| b.placement_set_placed_by(level, ticks, &placed));
            let placer = by_player.then(|| self.native_acting_player()).flatten();
            self.set_placed_by(target, placer, Some(stack))?;
            self.block_item_placed(stack, placer, &placed.context);
            if by_player {
                self.trigger_block("minecraft:placed_block", target, stack);
            }
        }
        let placer = by_player.then(|| self.native_clicker().1.cast_unsigned());
        self.with_level(|b, level, ticks| b.report_placement(level, ticks, &placed, placer));
        Ok(Some(placed))
    }
    /// `GameRuleChangedEvent` once the server set the rule `name` to `value`
    /// (`GameRules.set` with a server).
    pub(in crate::game) fn game_rule_changed(
        &mut self,
        name: &str,
        value: GameRuleValue,
        actor: Option<PlayerRef>,
    ) {
        // MinecraftServer.onGameRuleChanged refreshes the chunk source for this
        // rule, but not when spawn_mobs itself changes.
        if name == "minecraft:spawn_monsters" {
            self.update_mob_spawning_flags();
        }
        if mod_api::__private::listened(mod_api::HookId::GameRuleChanged) {
            self.native_game_rule_changed(name, value, actor);
        }
    }
    /// The players still here leave with the server (`PlayerList.removeAll`):
    /// the mods hear it once, whichever path closes it.
    pub fn native_players_leaving(&mut self) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        if std::mem::replace(&mut host.players_told_left, true) {
            return;
        }
        let players: Vec<(u128, String)> = host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed && g.participates_online())
            .map(|g| (g.uuid, g.name.clone()))
            .collect();
        for (player, name) in players {
            self.native_player_logged_out(player, &name);
        }
    }
    /// The containers of the mod blocks' block entities, by block state
    /// (`native_block_entity_container`).
    pub fn native_containers(&self) -> FxHashMap<u32, Arc<ModContainer>> {
        let registry = self.common.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModContainer>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(sunrise_world::registry::BlockStateId(i)) else {
                continue;
            };
            if !mod_api::__private::is_content_block(&state.name) {
                continue;
            }
            let container = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut container = None;
                self.native_block_entity_container(&state.name, &mut container);
                container.map(|mut c| {
                    let n = c.slots;
                    c.menu = c
                        .menu
                        .filter(|m| mod_menu(m).is_some_and(|(_, size)| size == n));
                    Arc::new(c)
                })
            });
            if let Some(c) = container {
                out.insert(i, Arc::clone(c));
            }
        }
        out
    }
    /// The energy buffers of the mod blocks' block entities, by block state
    /// (`native_block_entity_energy`).
    pub fn native_energy_tables(&self) -> FxHashMap<u32, Arc<ModEnergyBuffer>> {
        let registry = self.common.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModEnergyBuffer>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(sunrise_world::registry::BlockStateId(i)) else {
                continue;
            };
            if !mod_api::__private::is_content_block(&state.name) {
                continue;
            }
            let buffer = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut energy = None;
                self.native_block_entity_energy(&state.name, &mut energy);
                energy.map(Arc::new)
            });
            if let Some(b) = buffer {
                out.insert(i, Arc::clone(b));
            }
        }
        out
    }
    /// The fluid tanks of the mod blocks' block entities, by block state
    /// (`native_block_entity_fluids`).
    pub fn native_fluid_tables(&self) -> FxHashMap<u32, Arc<ModFluidTanks>> {
        let registry = self.common.pack.assets.registry;
        let mut by_block: FxHashMap<String, Option<Arc<ModFluidTanks>>> = FxHashMap::default();
        let mut out = FxHashMap::default();
        for i in 0..registry.len() as u32 {
            let Some(state) = registry.state(sunrise_world::registry::BlockStateId(i)) else {
                continue;
            };
            if !mod_api::__private::is_content_block(&state.name) {
                continue;
            }
            let tanks = by_block.entry(state.name.clone()).or_insert_with(|| {
                let mut fluids = None;
                self.native_block_entity_fluids(&state.name, &mut fluids);
                fluids.map(Arc::new)
            });
            if let Some(t) = tanks {
                out.insert(i, Arc::clone(t));
            }
        }
        out
    }
    /// The acting player's hit landed on mob `index`, holding `stack`.
    pub(in crate::game) fn native_hurt_enemy_event(&mut self, stack: &ItemStack, index: usize) {
        if !mod_api::__private::item_listens(mod_api::HookId::HurtEnemy, &stack.item) {
            return;
        }
        let mob = &self.common.mobs[index];
        let entity = mob.id;
        // MobTypes retains the canonical registry id for vanilla and content types alike.
        let kind = mob.kind.clone();
        sunrise_gameplay_common::game::api_values::with_stack_view(Some(stack), |view| {
            if let Some(view) = view {
                self.native_hurt_enemy(view, entity, &kind.id);
            }
        });
    }
    /// `Inventory.tick` for the acting player: each carried stack goes through
    /// `native_inventory_tick`.
    pub fn native_inventory_ticks(&mut self) {
        if self.common.player.is_none()
            || !mod_api::__private::has_part_hook(mod_api::HookId::InventoryTick)
        {
            return;
        }
        for i in 0..self.common.inventory.len() {
            self.native_slot_tick(i as i32, i == self.common.selected);
        }
        // Head first here; `Inventory` saves feet as 100 and head as 103.
        for i in 0..self.common.armor.len() {
            self.native_slot_tick(103 - i as i32, false);
        }
        self.native_slot_tick(-106, false);
    }
    pub(in crate::game) fn native_slot_tick(&mut self, slot: i32, selected: bool) {
        let Some(stack) = self.native_carried_slot(slot).and_then(|s| s.as_ref()) else {
            return;
        };
        if stack.count > 0
            && mod_api::__private::item_listens(mod_api::HookId::InventoryTick, &stack.item)
            && let Some(item) = mod_api::__private::item_key(&stack.item)
        {
            let place = mod_api::ItemPlace::PlayerSlot {
                player: self.native_acting_id(),
                slot,
            };
            self.native_inventory_tick(item, &place, slot, selected);
        }
        if let Some(stack) = self.native_carried_slot(slot)
            && stack.as_ref().is_some_and(|s| s.count == 0)
        {
            *stack = None;
        }
    }
    /// `entity_tame` for the pet at `index`, just tamed by its owner.
    pub(in crate::game) fn pet_tamed(&mut self, index: usize) {
        let m = &self.common.mobs[index];
        let (entity, kind) = (m.id, mob_kind(m));
        let owner = m.extra.owner.map(sunrise_sim::lan::playerdata::ints_uuid);
        self.native_entity_tame(entity, &kind, owner);
    }
    /// Whether a mod canceled `placer` writing `state` at `pos` (`entity_place`):
    /// `placer` is an entity id, `placed_against` the block it goes against and
    /// `companion` the second half of a bed, door or tall plant.
    pub fn place_canceled(
        &mut self,
        placer: u32,
        pos: BlockPos,
        state: BlockStateId,
        placed_against: BlockStateId,
        companion: Option<(BlockPos, BlockStateId)>,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::EntityPlace) {
            return false;
        }
        self.placement_canceled(self.entity(placer), pos, state, placed_against, companion)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl crate::game::ServerGame<'_> {
    pub fn test_save(&self) -> &std::sync::Arc<sunrise_sim::save::SaveDir> {
        &self.server.save.0
    }
}

#[cfg(test)]
#[path = "shared/item_fall_on_tests_server.rs"]
pub mod item_fall_on_tests;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::block_entity_load_additional::BlockEntityLoadAdditionalGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::placement_context::PlacementContextGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
