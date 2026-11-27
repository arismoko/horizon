//! Dispenser behaviours: `DispenserBlock.getDispenseMethod(..).dispense` with
//! every `DispenseItemBehavior` registration of vanilla that Sunrise has the
//! entities for. A behaviour returns what is left in the slot and the level
//! event it plays (`DefaultDispenseItemBehavior.dispense` then plays that
//! event and the smoke animation, event 2000).
#[allow(unused_imports)]
use super::{Dim, item_abilities::ability};
#[allow(unused_imports)]
use mod_api::{LevelRef, LevelWriteError, SoundSource};
#[allow(unused_imports)]
use std::ops::ControlFlow;
#[allow(unused_imports)]
use sunrise_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
    random::RandomSource,
};
#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay::dispense_rules::{
    CLICK, Dispensed, FAIL, Pos, Slot, body_equipment_species, equip_sound, mob_set_slot,
    mob_slot_item, offset, tipped_effect,
};
#[allow(unused_imports)]
use sunrise_gameplay::dispense_rules::{
    LAUNCH, block_box, data_value, mob_can_dispenser_equip, mob_can_use_slot, overlaps,
    section_order, shrink, step, y_rot,
};
#[allow(unused_imports)]
use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEnginePreservingCount};
#[allow(unused_imports)]
use sunrise_gameplay_common::game::level_sounds_common::LevelSound;
use sunrise_gameplay_common::game::spill_common;
use sunrise_gameplay_common::game::tool_modification::ToolModificationGameExt1 as _;
#[allow(unused_imports)]
use sunrise_world::{
    fluid::FluidKind,
    level::{PlaceContext, PlacementSource},
    registry::BlockStateId,
};
#[allow(unused_imports)]
use {
    sunrise_game_data::Result, sunrise_items::items::ItemStack, sunrise_items::items::Items,
    sunrise_level::block_entities::BlockEntityIdentity, sunrise_level::collisions::SceneCollisions,
    sunrise_level::containers::BlockEntity, sunrise_level::particle_options::ParticleOptions,
    sunrise_level::particle_options::ParticleType, sunrise_sim::mobs::Species,
    sunrise_sim::mobs::equip::equippable, sunrise_sim::mobs::sulfur_cube,
    sunrise_sim::projectiles::Projectile, sunrise_sim::projectiles::ProjectileKind,
};

/// A behaviour returns a detached stack; the dispenser applies the slot limit afterwards.
pub fn mod_dispensed(
    (stack, event): mod_api::Dispensed,
    items: &Items,
) -> std::result::Result<Dispensed, mod_api::StackWriteError> {
    let stack = stack
        .as_ref()
        .map(|stack| stack.try_into_engine_preserving_count(items))
        .transpose()?;
    Ok((stack, event))
}

thread_local! {
    /// `OptionalDispenseItemBehavior.success` of the registered behaviours
    /// that leave it unset on some paths (brush, TNT, honeycomb, chest).
    /// Vanilla keeps one behaviour instance per item for the whole run, so
    /// there the flag carries over from that item's previous dispense.
    pub(in crate::game) static SUCCESS: std::cell::RefCell<std::collections::HashMap<&'static str, bool>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}
pub(in crate::game) fn set_success(behaviour: &'static str, success: bool) {
    SUCCESS.with(|s| s.borrow_mut().insert(behaviour, success));
}
/// The level event `OptionalDispenseItemBehavior.playSound` plays for the flag.
pub(in crate::game) fn optional_event(behaviour: &'static str) -> i32 {
    if SUCCESS.with(|s| s.borrow().get(behaviour).copied().unwrap_or(true)) {
        CLICK
    } else {
        FAIL
    }
}

/// Entities `getEntitiesOfClass(.., AABB(pos))` can find.
#[derive(Clone, Copy)]
pub(in crate::game) enum Target {
    Player,
    /// A server player, by connection: the integrated owner and every guest.
    Guest(u32),
    Mob(usize),
    Stand(usize),
}

#[derive(Clone, Copy)]
pub(in crate::game) struct DispenseSource {
    pub(in crate::game) dim: Dim,
    pub(in crate::game) pos: Pos,
    pub(in crate::game) holder: Option<BlockEntityIdentity>,
}

pub(in crate::game) struct DispenseScope<'a, 'g> {
    pub(in crate::game) game: &'a mut crate::game::ServerGame<'g>,
    pub(in crate::game) outer: Option<DispenseSource>,
}

impl Drop for DispenseScope<'_, '_> {
    fn drop(&mut self) {
        self.game.server.dispense_source = self.outer.take();
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn block_at(&self, p: Pos) -> BlockStateId {
        self.common.world.block(p.0, p.1, p.2)
    }
    pub(in crate::game::server_side) fn water_at(&self, p: Pos) -> bool {
        matches!(
            self.res.behaviours().fluid_kind(self.block_at(p)),
            FluidKind::Water | FluidKind::FlowingWater
        )
    }
    pub(in crate::game) fn center(p: Pos) -> [f64; 3] {
        [
            f64::from(p.0) + 0.5,
            f64::from(p.1) + 0.5,
            f64::from(p.2) + 0.5,
        ]
    }
    pub(in crate::game) fn pos_of(p: Pos) -> BlockPos {
        BlockPos::new(p.0, p.1, p.2)
    }

    /// Publish dispenser feedback after commit so the client plays its sound and smoke once.
    pub(in crate::game) fn dispenser_event(&mut self, id: i32, pos: Pos, data: i32) {
        if self.has_level_transaction() {
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::DispenseEvent(id, pos, data),
            );
            return;
        }
        self.server_level_event_at(id, [pos.0, pos.1, pos.2], data);
    }

    // ---- DefaultDispenseItemBehavior ----

    /// `DispenserBlock.getDispensePosition(source, scale, offset)`; vanilla adds
    /// `scale * step + offset` first, then adds that sum to the block centre.
    pub(in crate::game::server_side) fn dispense_position(
        pos: Pos,
        facing: Direction,
        scale: f64,
        offset: [f64; 3],
    ) -> Vec3 {
        let s = step(facing);
        let c = Self::center(pos);
        Vec3::new(
            c[0] + (scale * f64::from(s[0]) + offset[0]),
            c[1] + (scale * f64::from(s[1]) + offset[1]),
            c[2] + (scale * f64::from(s[2]) + offset[2]),
        )
    }
    /// `DefaultDispenseItemBehavior.spawnItem` at `getDispensePosition`.
    pub(in crate::game::server_side) fn prepared_dispensed(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> spill_common::Spawn {
        let s = step(facing);
        let p = Self::dispense_position(pos, facing, 0.7, [0.0; 3]);
        let mut at = [p.x, p.y, p.z];
        at[1] -= if s[1] != 0 { 0.125 } else { 0.156_25 };
        // Vanilla constructs the entity and samples the source level's RNG before adding it.
        // Keep those draws in order even when publication is deferred by the journal.
        let random = spill_common::prepare_item_random(None);
        let r = &mut self.common.world.random;
        let pow = r.next_double() * 0.1 + 0.2;
        let spread = 0.017_227_5 * 6.0;
        let v = [
            r.triangle(f64::from(s[0]) * pow, spread),
            r.triangle(0.2, spread),
            r.triangle(f64::from(s[2]) * pow, spread),
        ];
        spill_common::Spawn {
            random: Some(random),
            stack,
            pos: at,
            motion: Some(v),
            delay: 0,
        }
    }

    pub(in crate::game::server_side) fn spawn_dispensed(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) {
        let spawn = self.prepared_dispensed(pos, facing, stack);
        self.commit_dispensed(pos, facing, spawn, false);
    }

    pub(in crate::game::server_side) fn stage_dispensed(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
        play_events: bool,
    ) {
        let spawn = self.prepared_dispensed(pos, facing, stack);
        if self.has_level_transaction() {
            self.after_commit(
                sunrise_gameplay_common::game::transactions::After::DispenserDrop {
                    pos,
                    facing,
                    spawn: Box::new(spawn),
                    play_events,
                },
            );
        } else {
            self.commit_dispensed(pos, facing, spawn, play_events);
        }
    }

    pub(in crate::game) fn commit_dispensed(
        &mut self,
        pos: Pos,
        facing: Direction,
        spawn: spill_common::Spawn,
        play_events: bool,
    ) {
        self.add_item_entity_prepared(
            spawn.stack,
            spawn.pos,
            spawn.motion,
            spawn.delay,
            spawn.random,
        );
        if play_events {
            self.commit_dispense_feedback(pos, facing, CLICK);
        }
    }

    pub(in crate::game) fn commit_dispense_feedback(
        &mut self,
        pos: Pos,
        facing: Direction,
        event: i32,
    ) {
        self.dispenser_event(event, pos, 0);
        self.dispenser_event(2000, pos, data_value(facing));
    }

    /// `DefaultDispenseItemBehavior.execute`: drop one item.
    pub(in crate::game) fn default_execute(
        &mut self,
        pos: Pos,
        facing: Direction,
        mut stack: ItemStack,
    ) -> Option<ItemStack> {
        let one = ItemStack {
            count: 1,
            ..stack.clone()
        };
        stack.count -= 1;
        self.stage_dispensed(pos, facing, one, false);
        (stack.count > 0).then_some(stack)
    }

    /// Sunrise represents a DispenserBlockEntity as the dispenser state and a slot holder.
    pub(super) fn dispenser_source(&self, pos: Pos) -> Option<u64> {
        if !self.world.loaded(pos.0, pos.2)
            || self.dispenser_block_name(pos).as_ref() != "minecraft:dispenser"
            || !matches!(self.world.block_entities.get(&pos), Some(holder @ BlockEntity::Chest(_)) if holder.fits("minecraft:dispenser"))
        {
            return None;
        }
        self.world.block_entities.generation(&pos)
    }
    pub(in crate::game) fn level_dispense_default(
        &mut self,
        level: LevelRef,
        [x, y, z]: [i32; 3],
        facing: Direction,
        stack: mod_api::ItemStack,
    ) -> std::result::Result<Option<mod_api::ItemStack>, LevelWriteError> {
        let stack = stack
            .try_into_engine_preserving_count(self.items())
            .map_err(LevelWriteError::InvalidStack)?;
        let left = self.with_server_level_mut(
            level,
            LevelWriteError::Unavailable,
            LevelWriteError::Stale,
            Some(LevelWriteError::InTransaction),
            |game| {
                let pos = (x, y, z);
                game.dispenser_source(pos)
                    .ok_or(LevelWriteError::InvalidDispenser)?;
                Ok((stack.count != 0)
                    .then(|| game.default_execute(pos, facing, stack))
                    .flatten())
            },
        )?;
        left.try_into_api()
            .map_err(|error| LevelWriteError::InvalidStack(error.into_write_error()))
    }

    pub(in crate::game) fn level_dispense_with_remainder(
        &mut self,
        level: LevelRef,
        [x, y, z]: [i32; 3],
        facing: Direction,
        stack: mod_api::ItemStack,
        remainder: mod_api::ItemStack,
    ) -> std::result::Result<Option<mod_api::ItemStack>, LevelWriteError> {
        let stack = stack
            .try_into_engine_preserving_count(self.items())
            .map_err(LevelWriteError::InvalidStack)?;
        let remainder = remainder
            .try_into_engine_preserving_count(self.items())
            .map_err(LevelWriteError::InvalidStack)?;
        let left = self.with_server_level_mut(
            level,
            LevelWriteError::Unavailable,
            LevelWriteError::Stale,
            Some(LevelWriteError::InTransaction),
            |game| {
                let pos = (x, y, z);
                game.dispenser_source(pos)
                    .ok_or(LevelWriteError::InvalidDispenser)?;
                if stack.count == 0 {
                    return Ok(None);
                }
                Ok(game.consume_with_remainder(pos, facing, stack, remainder))
            },
        )?;
        left.try_into_api()
            .map_err(|error| LevelWriteError::InvalidStack(error.into_write_error()))
    }
    /// `DefaultDispenseItemBehavior.addToInventoryOrDispense` for a mod's dispense helper, in the
    /// level in place: refuses a client, another dimension or an unloaded chunk.
    pub(in crate::game) fn dispense_add_remainder(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        facing: Direction,
        remainder: ItemStack,
    ) -> std::result::Result<(), mod_api::LevelWriteError> {
        if !(self.server.serving.is_some() || self.common.options.dedicated.is_some())
            || dimension != self.common.dim.id()
            || !self
                .common
                .world
                .chunks
                .contains_key(&(pos[0] >> 4, pos[2] >> 4))
        {
            return Err(mod_api::LevelWriteError::Unavailable);
        }
        self.add_to_inventory_or_dispense((pos[0], pos[1], pos[2]), facing, remainder);
        Ok(())
    }
    /// `new DefaultDispenseItemBehavior().dispense(..)` including its events.
    pub(in crate::game) fn dispense_default_in_level(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Option<ItemStack> {
        let left = self.default_execute(pos, facing, stack);
        self.after_commit(
            sunrise_gameplay_common::game::transactions::After::DispenserFeedback {
                pos,
                facing,
                event: CLICK,
            },
        );
        left
    }
    /// `DefaultDispenseItemBehavior.addToInventoryOrDispense`: inserts `remainder` into the
    /// dispenser (`insertItem`) or drops what does not fit with the dispenser click and smoke.
    /// Journals each slot write and defers dropped items and level events while a storage
    /// transaction is open.
    pub fn add_to_inventory_or_dispense(
        &mut self,
        pos: Pos,
        facing: Direction,
        mut rest: ItemStack,
    ) {
        if rest.count == 0 {
            return;
        }
        let max = self.containers().max_stack(&rest);
        let resident = self.common.world.block_entities.identity(pos);
        // Like BlockSource, a running behaviour retains its original dispenser even if
        // the transfer or its commit listeners replace the block. A stand-alone helper
        // call has no BlockSource and uses the currently resident container.
        let holder = match self.server.dispense_source {
            Some(source) if source.dim == self.common.dim && source.pos == pos => source.holder,
            _ => resident,
        };
        let slots = self
            .common
            .world
            .block_entities
            .get(&pos)
            .filter(|_| holder == resident)
            .map_or(0, |entity| entity.slots().len());
        let mut set_item = false;
        for slot in 0..slots {
            let room = self
                .common
                .world
                .block_entities
                .get(&pos)
                .and_then(|entity| entity.slots().get(slot))
                .map_or(0, |existing| match existing {
                    None => max,
                    Some(item)
                        if item.same_item_same_components_with_prototype(&rest, self.items()) =>
                    {
                        max.saturating_sub(item.count)
                    }
                    Some(_) => 0,
                });
            let n = rest.count.min(room);
            if n == 0 {
                continue;
            }
            self.journal_block_entity_slot(pos, slot);
            if let Some(cell) = self
                .common
                .world
                .block_entities
                .get_mut(&pos)
                .and_then(|entity| entity.slot_mut(slot))
            {
                match cell {
                    None => {
                        *cell = Some(ItemStack {
                            count: n,
                            ..rest.clone()
                        });
                        set_item = true;
                    }
                    Some(e) => {
                        e.count += n;
                    }
                }
                rest.count -= n;
            }
            if rest.count == 0 {
                break;
            }
        }
        // DispenserBlockEntity.insertItem calls setItem only for an empty slot; an in-place
        // merge is covered by dispenseFrom's normal final setItem on the active slot.
        if set_item && let Some(holder) = holder {
            self.after_holder_changed(holder);
        }
        if rest.count > 0 {
            self.stage_dispensed(pos, facing, rest, true);
        }
    }
    /// `DefaultDispenseItemBehavior.consumeWithRemainder`: the remainder goes
    /// into the dispenser (`insertItem`) or is dropped.
    pub(in crate::game) fn consume_with_remainder(
        &mut self,
        pos: Pos,
        facing: Direction,
        mut stack: ItemStack,
        remainder: ItemStack,
    ) -> Option<ItemStack> {
        stack.count -= 1;
        if stack.count == 0 {
            return Some(remainder);
        }
        self.add_to_inventory_or_dispense(pos, facing, remainder);
        Some(stack)
    }
    /// `ItemStack.hurtAndBreak(amount)` without an entity.
    pub fn hurt_and_break(&mut self, stack: ItemStack, amount: u32) -> Option<ItemStack> {
        self.damage_stack(&stack, amount.min(i32::MAX as u32) as i32, false)
            .0
    }

    // ---- the registry ----

    /// `DispenserBlock.getDispenseMethod(..).dispense(source, stack)`: the replacement
    /// for the slot, or `Break` when the slot must not be written at all.
    pub fn dispense_item(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<ControlFlow<(), Option<ItemStack>>> {
        let outer = self.server.dispense_source.replace(DispenseSource {
            dim: self.common.dim,
            pos,
            holder: self.common.world.block_entities.identity(pos),
        });
        let scope = DispenseScope { game: self, outer };
        scope.game.dispense_item_from_source(pos, facing, stack)
    }

    pub(in crate::game::server_side) fn dispense_item_from_source(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<ControlFlow<(), Option<ItemStack>>> {
        let mut answer = None;
        let block_hooks_active =
            self.server.serving.is_some() || self.common.options.dedicated.is_some();
        let block_reply_state = if block_hooks_active {
            // The block the dispenser faces has the first say (`dispense_method`), then the
            // item's own part (`dispense_item`); the first answer wins.
            let front = self.block_at(offset(pos, facing));
            let mut inert = false;
            self.native_dispense_method(
                self.common.dim.id(),
                [pos.0, pos.1, pos.2],
                facing,
                front,
                &stack,
                &mut answer,
                &mut inert,
            );
            if inert {
                return Ok(ControlFlow::Break(()));
            }
            if answer.is_none() {
                self.native_dispense_item(
                    self.common.dim.id(),
                    [pos.0, pos.1, pos.2],
                    facing,
                    &stack,
                    &mut answer,
                );
            }
            Some(front)
        } else {
            None
        };
        if let Some(answer) = answer {
            let (left, event) = match mod_dispensed(answer, self.items()) {
                Ok(dispensed) => dispensed,
                Err(error) => {
                    // The chosen behaviour may have committed effects already. Do not run a
                    // second behaviour on the original stack if its remainder is refused.
                    log::warn!("mod dispense remainder refused at {pos:?}: {error:?}");
                    return Ok(ControlFlow::Break(()));
                }
            };
            if event == FAIL {
                self.native_block_failed_dispense(
                    self.common.dim.id(),
                    [pos.0, pos.1, pos.2],
                    &stack.item,
                );
            }
            // Feedback follows the behaviour's effects. Event 0 still replaces the slot,
            // but plays neither sound nor smoke.
            if event != 0 {
                self.after_commit(
                    sunrise_gameplay_common::game::transactions::After::DispenserFeedback {
                        pos,
                        facing,
                        event,
                    },
                );
            }
            return Ok(ControlFlow::Continue(left));
        }
        // A refused reply may have stopped dispatch before the disabled block owner.
        if let Some(front) = block_reply_state
            && mod_api::__private::any_mod_disabled()
            && front.try_into_api().is_ok_and(|state| {
                mod_api::__private::block_hook_inert(mod_api::HookId::DispenseMethod, state)
            })
        {
            return Ok(ControlFlow::Break(()));
        }
        let item = stack.item.to_string();
        // A stack able to `shears_harvest` is dispensed as shears are, whatever its item.
        let shears = self.stack_shears(&stack, ability::SHEARS_HARVEST);
        let (left, event) = match item.as_str() {
            _ if shears => self.dispense_shears(pos, facing, stack)?,

            "minecraft:arrow"
            | "minecraft:tipped_arrow"
            | "minecraft:spectral_arrow"
            | "minecraft:egg"
            | "minecraft:blue_egg"
            | "minecraft:brown_egg"
            | "minecraft:snowball"
            | "minecraft:experience_bottle"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:firework_rocket"
            | "minecraft:fire_charge"
            | "minecraft:wind_charge" => self.dispense_projectile(pos, facing, stack),
            "minecraft:armor_stand" => self.dispense_armor_stand(pos, facing, stack),
            "minecraft:chest" => self.dispense_chest(pos, facing, stack),
            "minecraft:lava_bucket"
            | "minecraft:water_bucket"
            | "minecraft:powder_snow_bucket"
            | "minecraft:salmon_bucket"
            | "minecraft:cod_bucket"
            | "minecraft:pufferfish_bucket"
            | "minecraft:tropical_fish_bucket"
            | "minecraft:axolotl_bucket"
            | "minecraft:tadpole_bucket"
            | "minecraft:sulfur_cube_bucket" => self.dispense_filled_bucket(pos, facing, stack)?,
            "minecraft:bucket" => self.dispense_empty_bucket(pos, facing, stack)?,
            "minecraft:flint_and_steel" => self.dispense_flint_and_steel(pos, facing, stack)?,
            "minecraft:bone_meal" => self.dispense_bone_meal(pos, facing, stack)?,
            "minecraft:tnt" => self.dispense_tnt(pos, facing, stack),
            "minecraft:wither_skeleton_skull" => self.dispense_wither_skull(pos, facing, stack)?,
            "minecraft:carved_pumpkin" => self.dispense_carved_pumpkin(pos, facing, stack)?,
            "minecraft:glass_bottle" => self.dispense_glass_bottle(pos, facing, stack)?,
            "minecraft:glowstone" => self.dispense_glowstone(pos, facing, stack)?,
            "minecraft:brush" => self.dispense_brush(pos, facing, stack),
            "minecraft:honeycomb" => self.dispense_honeycomb(pos, facing, stack)?,
            "minecraft:potion" => self.dispense_potion(pos, facing, stack)?,
            _ if item.ends_with("shulker_box") => self.dispense_shulker_box(pos, facing, stack)?,
            _ if self.dispenses_as_filled_bucket(&item) => {
                self.dispense_filled_bucket(pos, facing, stack)?
            }
            _ if sunrise_gameplay_common::game::boats_common::any_boat_wood(&item).is_some() => {
                self.dispense_boat(pos, facing, stack)
            }
            _ if super::minecarts::CartKind::from_id(&item).is_some() => {
                self.dispense_minecart(pos, facing, stack)
            }
            _ => self.dispense_default_method(pos, facing, stack)?,
        };
        if event == FAIL {
            self.native_block_failed_dispense(self.common.dim.id(), [pos.0, pos.1, pos.2], &item);
        }
        self.after_commit(
            sunrise_gameplay_common::game::transactions::After::DispenserFeedback {
                pos,
                facing,
                event,
            },
        );
        Ok(ControlFlow::Continue(left))
    }
    /// `DispenserBlock.getDefaultDispenseMethod`: equipment, spawn eggs, or a drop.
    pub fn dispense_default_method(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        // `ItemStack.has(DataComponents.EQUIPPABLE)`: the stack's own, a removed one included.
        if equippable(self.items(), &stack).is_some() {
            let mut stack = stack;
            if self.dispense_equipment(pos, facing, &mut stack) {
                return Ok((Some(stack).filter(|s| s.count > 0), CLICK));
            }
            return Ok((self.default_execute(pos, facing, stack), CLICK));
        }
        let swallowable = self
            .common
            .res
            .items()
            .item_tags
            .get("minecraft:sulfur_cube_swallowable")
            .is_some_and(|t| t.contains(&*stack.item));
        if swallowable {
            let front = offset(pos, facing);
            if self.sulfur_cube_swallow(front, &stack) {
                return Ok((shrink(stack), CLICK));
            }
            return Ok((self.default_execute(pos, facing, stack), CLICK));
        }
        if stack.item.ends_with("_spawn_egg") {
            return self.dispense_spawn_egg(pos, facing, stack);
        }
        Ok((self.default_execute(pos, facing, stack), CLICK))
    }

    // ---- entities ----

    /// Entities whose box overlaps the block at `p`, in `getEntitiesOfClass`
    /// order ([`section_order`]): players are entities of the level like any
    /// other, so they sort with the mobs and stands.
    pub(in crate::game) fn targets_at(&self, p: Pos) -> Vec<Target> {
        let block = block_box(p);
        let key = |at: Vec3, id: u32| (section_order(at), id);
        let mut found: Vec<(_, Target)> = self
            .common
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.removed && overlaps(m.bounds(), block))
            .map(|(i, m)| (key(m.position, m.id), Target::Mob(i)))
            .collect();
        found.extend(
            self.common
                .armor_stands
                .iter()
                .enumerate()
                .filter(|(_, s)| overlaps(s.bounds(), block))
                .map(|(i, s)| (key(s.position, s.id), Target::Stand(i))),
        );
        // `ServerLevel.getEntities`: every player placed in this level, the
        // integrated owner included (a server has no player of its own), its
        // client loaded or not (it joins the level at placeNewPlayer/respawn).
        if let Some(host) = &self.server.lan {
            found.extend(
                (host.hub.guests.iter())
                    .filter(|g| host.hub.here(g) && !g.dead && !g.spectator())
                    .filter(|g| overlaps(g.bounds(), block))
                    .map(|g| {
                        (
                            key(
                                Vec3::new(g.position[0], g.position[1], g.position[2]),
                                g.entity_id as u32,
                            ),
                            Target::Guest(g.conn),
                        )
                    }),
            );
        }
        if let Some(v) = self.player_view()
            && v.alive
            && !self.common.spectator()
            && overlaps(v.bounds, block)
        {
            // The local player joins before the chunks around it load their
            // entities (prepareLevels loads only saved tickets): the lowest id,
            // 0 as `own_entity_id` has it.
            found.push((key(v.position, 0), Target::Player));
        }
        found.sort_by_key(|(k, _)| *k);
        found.into_iter().map(|(_, t)| t).collect()
    }
    /// `EntityType.getYOffset(level, pos, false, box)`: [`Self::drop_offset_from`] without
    /// `movedUp`.
    pub(in crate::game::server_side) fn drop_offset(&self, width: f64, height: f64, p: Pos) -> f64 {
        self.drop_offset_from(width, height, p, false)
    }
    /// `EntityType.getYOffset(level, pos, movedUp, box)`: the entity box set on
    /// top of the block (`setPos(x + 0.5, y + 1, z + 0.5)`) falls one block
    /// against the collision shapes overlapping that block, or, `moved_up`, two
    /// blocks against those of the block and the one under it.
    pub(in crate::game::server_side) fn drop_offset_from(
        &self,
        width: f64,
        height: f64,
        p: Pos,
        moved_up: bool,
    ) -> f64 {
        use sunrise_world::{
            block_collision_scan::BlockCollisionScan, collision_context::CollisionContext,
        };
        let h = width / 2.0;
        let (x, z) = (f64::from(p.0) + 0.5, f64::from(p.2) + 0.5);
        let y = f64::from(p.1) + 1.0;
        let entity = Aabb::new(
            Vec3::new(x - h, y, z - h),
            Vec3::new(x + h, y + height, z + h),
        );
        let cell = Aabb::new(
            Vec3::new(
                f64::from(p.0),
                f64::from(p.1) - if moved_up { 1.0 } else { 0.0 },
                f64::from(p.2),
            ),
            Vec3::new(
                f64::from(p.0) + 1.0,
                f64::from(p.1) + 1.0,
                f64::from(p.2) + 1.0,
            ),
        );
        let mut scene = SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        let mut amount = if moved_up { -2.0 } else { -1.0 };
        // Level.getCollisions(null, aabb): CollisionContext.empty().
        if let Ok(scan) = BlockCollisionScan::new(
            &mut scene,
            CollisionContext::empty(false),
            cell,
            false,
            1_000_000,
        ) {
            for hit in scan.flatten() {
                amount = hit
                    .shape
                    .collide(sunrise_core::geometry::Axis::Y, entity, amount);
            }
        }
        1.0 + amount
    }
    /// `EntityType.spawn(.., pos, reason, tryMoveDown, false)` for a mob.
    pub fn spawn_mob_at(&mut self, kind: &str, p: Pos, try_move_down: bool) -> Option<u32> {
        let name = if kind.contains(':') {
            kind.to_owned()
        } else {
            format!("minecraft:{kind}")
        };
        // Ender Dragons are simulated separately from `Mob` and therefore do
        // not have a mob-type registry entry for the generic dispenser path.
        if name == "minecraft:ender_dragon" {
            // `EntityType.getYOffset` uses the created entity's full 16 by 8
            // bounding box when the dispenser asks to move it down.
            let y_off = if try_move_down {
                self.drop_offset(16.0, 8.0, p)
            } else {
                0.0
            };
            let at = [
                f64::from(p.0) + 0.5,
                f64::from(p.1) + y_off,
                f64::from(p.2) + 0.5,
            ];
            return self.spawn_dragon_spawned(
                at,
                sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                    sunrise_sim::mobs::spawn_rules::Reason::Dispenser,
                ),
            );
        }
        let t = self.common.res.mob_types().get(&name)?.clone();
        let y_off = if try_move_down {
            self.drop_offset(t.width, t.height, p)
        } else {
            0.0
        };
        let at = [
            f64::from(p.0) + 0.5,
            f64::from(p.1) + y_off,
            f64::from(p.2) + 0.5,
        ];
        let id = self.spawn_mob(
            &name,
            at,
            true,
            sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                sunrise_sim::mobs::spawn_rules::Reason::Dispenser,
            ),
        )?;
        Some(id)
    }
    /// `Player.setItemSlot` of a dispensed piece for the player in place:
    /// `onEquipItem` plays the sound and the event; the modifiers follow.
    pub(in crate::game) fn put_on_player(&mut self, slot: Slot, one: &ItemStack) {
        let name = match slot.armor_index() {
            Some(i) => {
                self.common.armor[i] = Some(one.clone());
                ["head", "chest", "legs", "feet"][i]
            }
            None => {
                self.common.offhand = Some(one.clone());
                "offhand"
            }
        };
        self.player_equipped(name, Some(one));
        self.apply_equipment();
    }
    /// After `EntityType.spawn` / `MobBucketItem.spawn`: `mob.playAmbientSound()`.
    pub(in crate::game::server_side) fn mob_ambient(&mut self, id: u32) -> Result<()> {
        if self.dragon_spawn_ambient(id) {
            return Ok(());
        }
        let Some(i) = self.common.mobs.iter().position(|m| m.id == id) else {
            return Ok(());
        };
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        let mut m = self.common.mobs.extract(self.common.mobs[i].join).unwrap();
        let mut events = Vec::new();
        {
            let w = self.mob_world_base(&[]);
            sunrise_sim::mobs::ai::ambient_sound(&mut m, &w, &mut events);
        }
        self.common.mobs.put_back(i, m);
        self.handle_mob_events(events)
    }
    /// A mob a bucket emptied (`MobBucketItem.spawn`): the bucket's components
    /// and entity data (`loadFromBucketTag`), then `setFromBucket(true)`, which
    /// `requiresCustomPersistence` reads, then it calls out. Only the bucket's
    /// data can make it persistent, not the spawn: `spawn_mob_at` creates a
    /// persistent mob. A tadpole's setter does nothing: its `fromBucket` is
    /// always true.
    pub fn mob_placed(&mut self, id: u32, stack: &ItemStack) -> Result<()> {
        let variants = world_variants!(self);
        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
            m.persistent = false;
            sunrise_sim::entity_nbt::apply_item_components(m, stack);
            sunrise_sim::entity_nbt::apply_item_variant_components(m, stack, variants);
            match m.species() {
                Species::Cod
                | Species::Salmon
                | Species::Pufferfish
                | Species::TropicalFish
                | Species::Axolotl => m.extra.from_bucket = true,
                Species::SulfurCube => {
                    // SulfurCube.loadFromBucketTag: its own lowercase keys, 0
                    // and false when absent, whatever `Age` the data holds.
                    let data = stack
                        .data()
                        .and_then(|c| c.other("minecraft:bucket_entity_data"));
                    let age = data.and_then(|d| sunrise_sim::save::number(d.get("age")));
                    m.extra.age = age.unwrap_or(0) as i32;
                    m.extra.age_locked =
                        data.is_some_and(|d| sunrise_sim::entity_nbt::flag(d, "age_locked"));
                    m.extra.scratch[3] = 1;
                }
                _ => {}
            }
        }
        self.mob_ambient(id)
    }
    /// `SpawnEggItemBehavior.execute`: spawn the type selected by the stack's entity data.
    pub(in crate::game::server_side) fn dispense_spawn_egg(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let items = self.common.res.shared_items();
        let Some(data) = sunrise_sim::block_entity_item::item_entity_data(&stack, &items) else {
            // Without effective ENTITY_DATA, DispenserBlock selects the ordinary drop.
            return Ok((self.default_execute(pos, facing, stack), CLICK));
        };
        if !self.common.spawn_egg_allowed(&data.kind) {
            return Ok((Some(stack), CLICK));
        }
        let front = offset(pos, facing);
        let Some(id) = self.spawn_mob_at(&data.kind, front, facing != Direction::Up) else {
            return Ok((Some(stack), CLICK));
        };
        // EntityType.create: `finalizeSpawn` first, the egg's components after.
        let variants = world_variants!(self);
        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
            // SpawnEggItemBehavior's `EntityType.spawn` never calls
            // `setPersistenceRequired`, as a spawn egg used by hand doesn't;
            // only the elder guardian's and copper golem's constructors do.
            m.persistent = matches!(m.species(), Species::ElderGuardian | Species::CopperGolem);
            sunrise_sim::entity_nbt::apply_item_components(m, &stack);
            sunrise_sim::entity_nbt::apply_item_variant_components(m, &stack, variants);
        }
        // EntityType.updateCustomEntityTag: a configured egg
        // (`/give @s pig_spawn_egg[minecraft:entity_data={…}]`) spawns the mob
        // it describes from a dispenser too, not only from a player's hand.
        self.apply_spawn_egg_entity_data(id, &data);
        self.mob_ambient(id)?;
        // SpawnEggItemBehavior: the entity was placed by the dispenser.
        self.post_block_game_event("minecraft:entity_place", [pos.0, pos.1, pos.2], None, None);
        Ok((shrink(stack), CLICK))
    }

    // ---- ProjectileDispenseBehavior ----

    pub(in crate::game::server_side) fn dispense_projectile(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Dispensed {
        let item = stack.item.to_string();
        let s = step(facing);
        let dir = [f64::from(s[0]), f64::from(s[1]), f64::from(s[2])];
        // ProjectileItem.DispenseConfig per item: position, uncertainty, power, event.
        let (at, uncertainty, power, event) = match item.as_str() {
            "minecraft:firework_rocket" => (
                Self::dispense_position(pos, facing, 0.500_009_999_999_747_4, [0.0; 3]),
                1.0,
                0.5,
                1004,
            ),
            "minecraft:fire_charge" => (
                Self::dispense_position(pos, facing, 1.0, [0.0; 3]),
                6.666_666_5,
                1.0,
                1018,
            ),
            "minecraft:wind_charge" => (
                Self::dispense_position(pos, facing, 1.0, [0.0; 3]),
                6.666_666_5,
                1.0,
                1051,
            ),
            "minecraft:experience_bottle"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion" => (
                Self::dispense_position(pos, facing, 0.7, [0.0, 0.1, 0.0]),
                3.0,
                1.375,
                LAUNCH,
            ),
            _ => (
                Self::dispense_position(pos, facing, 0.7, [0.0, 0.1, 0.0]),
                6.0,
                1.1,
                LAUNCH,
            ),
        };
        self.shoot_projectile_item(at, dir, &stack, power, uncertainty);
        (shrink(stack), event)
    }

    /// `ProjectileItem.asProjectile` and `Projectile.spawnProjectileUsingShoot`:
    /// one of `stack` fired from `at` along `dir`.
    pub fn shoot_projectile_item(
        &mut self,
        at: Vec3,
        dir: [f64; 3],
        stack: &ItemStack,
        power: f32,
        uncertainty: f32,
    ) {
        let item = stack.item.to_string();
        let one = ItemStack {
            count: 1,
            ..stack.clone()
        };
        // A rocket makes its own random in `launch_angled_rocket`: the constructor's draws
        // and then the shot's spread come from that one.
        if item == "minecraft:firework_rocket" {
            self.launch_angled_rocket(at, &one, None, |own| {
                let mut shot = Projectile::new(ProjectileKind::Snowball, at, None);
                shot.shoot(own, dir, power, uncertainty);
                shot.velocity
            });
            return;
        }
        // Projectile.shoot spreads the shot with the projectile's own random.
        match item.as_str() {
            "minecraft:experience_bottle"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion" => {
                let mut shot =
                    crate::game::server_side::potions_server::thrown_projectile(one, at, false);
                shot.shoot_own(dir, power, uncertainty);
                self.add_fresh_projectile(shot);
            }
            _ => {
                let kind = match item.as_str() {
                    "minecraft:arrow" => ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    "minecraft:spectral_arrow" => ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: Some(("minecraft:glowing", 200)),
                    },
                    "minecraft:tipped_arrow" => ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: tipped_effect(stack),
                    },
                    "minecraft:fire_charge" | "minecraft:wind_charge" => {
                        // asProjectile aims with the level's random first; the
                        // shot along the facing then replaces that motion.
                        let r = &mut self.common.world.random;
                        let _ = (
                            r.triangle(dir[0], 0.114_85),
                            r.triangle(dir[1], 0.114_85),
                            r.triangle(dir[2], 0.114_85),
                        );
                        if item == "minecraft:wind_charge" {
                            ProjectileKind::WindCharge
                        } else {
                            ProjectileKind::SmallFireball
                        }
                    }
                    "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" => {
                        ProjectileKind::Egg
                    }
                    _ => ProjectileKind::Snowball,
                };
                let mut shot = Projectile::new(kind, at, None);
                shot.set_egg_item(&one);
                // ArrowItem/SpectralArrowItem.asProjectile: dispensed arrows
                // may be picked up.
                if let Some(a) = shot.arrow.as_mut() {
                    a.set_item(&one);
                    a.pickup = sunrise_sim::entities::arrows::Pickup::Allowed;
                }
                shot.shoot_own(dir, power, uncertainty);
                self.spawn_projectile(shot, &one);
            }
        }
    }

    // ---- entities placed by items ----

    pub(in crate::game::server_side) fn dispense_armor_stand(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Dispensed {
        let front = offset(pos, facing);
        let foot = Vec3::new(
            f64::from(front.0) + 0.5,
            f64::from(front.1),
            f64::from(front.2) + 0.5,
        );
        // EntityType.create rolls a yaw from the level random before the stack config turns it.
        let _ = self.common.world.random.next_float();
        self.server.next_entity += 1;
        let (stand, uuid) = sunrise_sim::entities::ArmorStand::new_with_uuid(
            self.server.next_entity,
            foot,
            y_rot(facing),
        );
        self.common
            .persist
            .uuids
            .insert(self.server.next_entity, uuid);
        self.common
            .world
            .modified
            .insert((front.0 >> 4, front.2 >> 4));
        self.common.armor_stands.push(stand);
        // EntityType.appendDefaultStackConfig: the dispenser's yaw first, then
        // the item's createDefaultStackConfig with no user.
        self.apply_item_stack_config(
            super::commands::Target::Stand(self.server.next_entity),
            "minecraft:armor_stand",
            &stack,
            false,
        );
        (shrink(stack), CLICK)
    }
    /// A tame donkey, mule or llama in front without a chest gets it.
    pub(in crate::game::server_side) fn dispense_chest(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Dispensed {
        let front = offset(pos, facing);
        for target in self.targets_at(front) {
            let Target::Mob(i) = target else { continue };
            let m = &mut self.common.mobs[i];
            if matches!(
                m.species(),
                Species::Donkey | Species::Mule | Species::Llama | Species::TraderLlama
            ) && m.alive()
                && m.extra.tame
                && !m.extra.chested
            {
                m.extra.chested = true;
                set_success("chest", true);
                return (shrink(stack), CLICK);
            }
        }
        (
            self.default_execute(pos, facing, stack),
            optional_event("chest"),
        )
    }
    /// `BoatDispenseItemBehavior`.
    pub(in crate::game::server_side) fn dispense_boat(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Dispensed {
        let Some((wood, chest)) = sunrise_gameplay_common::game::boats_common::registered_boat_item(
            &stack.item,
            self.common.res.items(),
        ) else {
            return (self.dispense_default_in_level(pos, facing, stack), CLICK);
        };
        let front = offset(pos, facing);
        let s = step(facing);
        let c = Self::center(pos);
        // Boat width 1.375.
        let outside = 0.5625 + 1.375 / 2.0;
        let (x, y, z) = (
            c[0] + f64::from(s[0]) * outside,
            c[1] + f64::from(s[1]) * f64::from(1.125f32),
            c[2] + f64::from(s[2]) * outside,
        );
        let below = (front.0, front.1 - 1, front.2);
        let y_offset = if self.water_at(front) {
            1.0
        } else if self.common.res.facts().is_air(self.block_at(front)) && self.water_at(below) {
            0.0
        } else {
            return (self.dispense_default_in_level(pos, facing, stack), CLICK);
        };
        self.server.next_entity += 1;
        let mut boat = sunrise_gameplay_common::game::boats_common::Boat::new(
            self.server.next_entity,
            wood.into(),
            Vec3::new(x, y + y_offset, z),
            y_rot(facing),
        );
        if chest {
            boat = boat.with_chest();
        }
        self.common
            .world
            .modified
            .insert((front.0 >> 4, front.2 >> 4));
        let (id, kind, yaw) = (boat.id, boat.entity_type(), boat.yaw);
        self.common.boats.push(boat);
        // BoatDispenseItemBehavior: createDefaultStackConfig with no user,
        // then setYRot(direction.toYRot()).
        self.apply_item_stack_config(super::commands::Target::Boat(id), &kind, &stack, false);
        self.set_boat_yaw(id, yaw);
        if self.server.serving.is_some() || self.common.options.dedicated.is_some() {
            let boat = self.common.boats.last().unwrap();
            self.native_vehicle_create(
                boat.entity_type().as_str(),
                [boat.position.x, boat.position.y, boat.position.z],
            );
        }
        (shrink(stack), CLICK)
    }
    /// `MinecartDispenseItemBehavior`.
    pub(in crate::game::server_side) fn dispense_minecart(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Dispensed {
        let Some(kind) = super::minecarts::CartKind::from_id(&stack.item) else {
            return (self.dispense_default_in_level(pos, facing, stack), CLICK);
        };
        let s = step(facing);
        let c = Self::center(pos);
        let (x, base_y, z) = (
            c[0] + f64::from(s[0]) * 1.125,
            c[1].floor() + f64::from(s[1]),
            c[2] + f64::from(s[2]) * 1.125,
        );
        let front = offset(pos, facing);
        let y_offset = if let Some(rail) = self.rail(None, front.0, front.1, front.2) {
            if rail.shape.is_slope() { 0.6 } else { 0.1 }
        } else {
            if !self.common.res.facts().is_air(self.block_at(front)) {
                return (self.dispense_default_in_level(pos, facing, stack), CLICK);
            }
            let below = (front.0, front.1 - 1, front.2);
            let Some(rail) = self.rail(None, below.0, below.1, below.2) else {
                return (self.dispense_default_in_level(pos, facing, stack), CLICK);
            };
            if facing != Direction::Down && rail.shape.is_slope() {
                -0.4
            } else {
                -0.9
            }
        };
        self.server.next_entity += 1;
        let cart = super::minecarts::Minecart::new(
            self.server.next_entity,
            kind,
            Vec3::new(x, base_y + y_offset, z),
        );
        self.common
            .world
            .modified
            .insert((front.0 >> 4, front.2 >> 4));
        self.common.carts.push(cart);
        // AbstractMinecart.createMinecart: createDefaultStackConfig with no user.
        let id = self.server.next_entity;
        self.apply_item_stack_config(super::commands::Target::Cart(id), kind.id(), &stack, false);
        if self.server.serving.is_some() || self.common.options.dedicated.is_some() {
            let cart = self.common.carts.last().unwrap();
            self.native_vehicle_create(
                cart.kind.id(),
                [cart.position.x, cart.position.y, cart.position.z],
            );
        }
        (shrink(stack), CLICK)
    }

    // ---- buckets ----

    /// The empty bucket: `BucketPickup.pickupBlock`.
    pub(in crate::game::server_side) fn dispense_empty_bucket(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = Self::pos_of(offset(pos, facing));
        let picked =
            self.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, target, false));
        self.server_process_level_events()?;
        match picked {
            Some(filled) => {
                self.post_block_game_event(
                    "minecraft:fluid_pickup",
                    [target.x, target.y, target.z],
                    None,
                    None,
                );
                let filled = ItemStack::new(filled, 1);
                Ok((
                    self.consume_with_remainder(pos, facing, stack, filled),
                    CLICK,
                ))
            }
            None => Ok((self.default_execute(pos, facing, stack), CLICK)),
        }
    }

    // ---- fire, bone meal and TNT ----

    pub(in crate::game::server_side) fn dispense_flint_and_steel(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let target_pos = Self::pos_of(target);
        let name = self.dispenser_block_name(target);
        let mut success = true;
        // tryIgniteExplosiveEntities: the first sulfur cube that can explode is primed.
        let cube = self.targets_at(target).into_iter().find_map(|t| match t {
            Target::Mob(i) => {
                let m = &self.common.mobs[i];
                (m.species() == Species::SulfurCube
                    && m.alive()
                    && !sulfur_cube::primed(m)
                    && sulfur_cube::archetype(m).is_some_and(|a| a.explosion.is_some()))
                .then_some(i)
            }
            _ => None,
        });
        if let Some(i) = cube {
            // primeTime(false): the fuse is lit only while TNT explodes; the
            // ignition counts either way.
            if self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
                && sulfur_cube::prime(&mut self.common.mobs[i], false)
            {
                let m = &mut self.common.mobs[i];
                let (at, id) = (m.position, m.id);
                let mut events = Vec::new();
                m.voice("entity.tnt.primed", &mut events);
                self.handle_mob_events(events)?;
                let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Mob(id)), None);
                self.post_game_event("minecraft:prime_fuse", [at.x, at.y, at.z], ctx);
            }
            return Ok((self.hurt_and_break(stack, 1), CLICK));
        }
        // getToolModifiedState(FIRESTARTER_LIGHT) on a hit at the target's centre, from the
        // dispenser's side, with no player, asked only where no fire can be placed.
        let hit = super::Hit {
            pos: [target.0, target.1, target.2],
            face: facing.opposite().normal(),
            point: [
                f64::from(target.0) + 0.5,
                f64::from(target.1) + 0.5,
                f64::from(target.2) + 0.5,
            ],
            inside: false,
        };
        if self
            .with_level(|b, level, ticks| b.fire_can_be_placed_at(level, ticks, target_pos, facing))
        {
            if !self.native_block_ignite_allowed(target_pos, "flint_and_steel") {
                return Ok((Some(stack), CLICK));
            }
            let fire =
                self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, target_pos));
            self.level_set([target.0, target.1, target.2], fire)?;
            self.post_block_game_event(
                "minecraft:block_place",
                [target.0, target.1, target.2],
                None,
                None,
            );
        } else if let Some(lit) = self.fire_starter_lit(&hit, &stack, false)? {
            self.level_set([target.0, target.1, target.2], lit)?;
            self.post_block_game_event(
                "minecraft:block_change",
                [target.0, target.1, target.2],
                None,
                None,
            );
        } else if &*name == "minecraft:tnt" {
            if self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
                && self.tnt_prime_allowed(target_pos)
            {
                self.spawn_primed_tnt(target_pos, false, None, None);
                self.with_level(|b, level, ticks| b.remove_block(level, ticks, target_pos, false));
                self.server_process_level_events()?;
            } else {
                success = false;
            }
        }
        let event = if success { CLICK } else { FAIL };
        if success {
            return Ok((self.hurt_and_break(stack, 1), event));
        }
        Ok((Some(stack), event))
    }
    pub(in crate::game::server_side) fn dispense_bone_meal(
        &mut self,
        pos: Pos,
        facing: Direction,
        mut stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let target_pos = Self::pos_of(target);
        let ask = self.bonemeal(target_pos, false, &mut stack);
        let grown = self.with_level(|b, level, ticks| {
            ask.crop
                .unwrap_or_else(|| b.grow_crop(level, ticks, target_pos, ask.check))
                || b.grow_water_plant(level, ticks, target_pos, None)
        });
        if grown {
            self.common
                .level_events
                .push(sunrise_world::level::LevelEvent::LevelEvent {
                    id: 1505,
                    pos: target_pos,
                    data: 15,
                    source: None,
                });
        }
        self.server_process_level_events()?;
        // A canceled `BonemealEvent` that succeeded uses none of the meal.
        let left = if grown && ask.crop != Some(true) {
            shrink(stack)
        } else {
            (stack.count > 0).then_some(stack)
        };
        Ok((left, if grown { CLICK } else { FAIL }))
    }

    // ---- blocks placed by dispensers ----

    pub(in crate::game::server_side) fn dispense_wither_skull(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let free = self.common.res.facts().is_air(self.block_at(target))
            && target.1 >= self.common.world.bottom_y() + 2
            && self.common.ui.difficulty != 0
            && self.can_build_wither(target);
        if free {
            let rotation = match facing {
                Direction::Up | Direction::Down => 0,
                other => {
                    // RotationSegment.convertToSegment: the 2D data value * 4.
                    (match other {
                        Direction::South => 0,
                        Direction::West => 1,
                        Direction::North => 2,
                        _ => 3,
                    }) * 4
                }
            };
            let skull = self
                .common
                .pack
                .assets
                .registry
                .default_state("minecraft:wither_skeleton_skull")
                .and_then(|s| {
                    self.common
                        .with_props(s, &[("rotation", &rotation.to_string())])
                });
            if let Some(skull) = skull {
                self.level_set([target.0, target.1, target.2], skull)?;
                if let Some(entity) = BlockEntity::for_block("minecraft:wither_skeleton_skull") {
                    self.common.world.block_entities.insert(target, entity);
                }
                self.post_block_game_event(
                    "minecraft:block_place",
                    [target.0, target.1, target.2],
                    None,
                    None,
                );
                self.check_wither_spawn([target.0, target.1, target.2])?;
                return Ok((shrink(stack), CLICK));
            }
        }
        let mut stack = stack;
        let equipped = self.dispense_equipment(pos, facing, &mut stack);
        Ok((
            Some(stack).filter(|s| s.count > 0),
            if equipped { CLICK } else { FAIL },
        ))
    }
    /// `WitherSkullBlock.getOrCreateWitherBase().find(level, pos)`: a soul
    /// sand T for the skull row, in any orientation.
    pub(in crate::game::server_side) fn can_build_wither(&self, head: Pos) -> bool {
        self.find_wither_pattern(
            sunrise_sim::block_pattern::WITHER_BASE,
            [head.0, head.1, head.2],
            self.common.world.block(head.0, head.1, head.2),
        )
        .is_some()
    }
    pub(in crate::game::server_side) fn dispense_carved_pumpkin(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        if self.common.res.facts().is_air(self.block_at(target))
            && self.can_spawn_golem([target.0, target.1, target.2])
        {
            if let Some(pumpkin) = self
                .common
                .pack
                .assets
                .registry
                .default_state("minecraft:carved_pumpkin")
            {
                let head = [target.0, target.1, target.2];
                self.set_golem_head(head, pumpkin)?;
                self.post_block_game_event("minecraft:block_place", head, None, None);
            }
            return Ok((shrink(stack), CLICK));
        }
        let mut stack = stack;
        let equipped = self.dispense_equipment(pos, facing, &mut stack);
        Ok((
            Some(stack).filter(|s| s.count > 0),
            if equipped { CLICK } else { FAIL },
        ))
    }
    /// `ShulkerBoxDispenseBehavior`: place it like a `DirectionalPlaceContext`.
    pub(in crate::game::server_side) fn shulker_box_place_context(
        front: Pos,
        facing: Direction,
        below_empty: bool,
        item: String,
    ) -> PlaceContext {
        let clicked_face = if below_empty { facing } else { Direction::Up };
        PlaceContext {
            clicked_pos: Self::pos_of(front),
            face: clicked_face,
            hit: Vec3::new(
                f64::from(front.0) + 0.5,
                f64::from(front.1),
                f64::from(front.2) + 0.5,
            ),
            inside: false,
            source: PlacementSource::Directional { direction: facing },
            item,
            can_use_game_master_blocks: false,
        }
    }
    /// `ShulkerBoxDispenseBehavior`: place it like a `DirectionalPlaceContext`.
    pub(in crate::game::server_side) fn dispense_shulker_box(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let front = offset(pos, facing);
        let below_empty =
            self.common
                .res
                .facts()
                .is_air(self.block_at((front.0, front.1 - 1, front.2)));
        let ctx =
            Self::shulker_box_place_context(front, facing, below_empty, stack.item.to_string());
        // DirectionalPlaceContext places at the front block or nowhere. A shulker box is
        // neither a slab nor snow, so `canPlace` always has an answer here.
        if self.with_level(|b, level, ticks| b.directional_can_place(level, ticks, &ctx))
            != Ok(true)
        {
            return Ok((Some(stack), FAIL));
        }
        if self.place_item_block(&ctx, false, &stack)?.is_none() {
            self.server_process_level_events()?;
            return Ok((Some(stack), FAIL));
        }
        self.server_process_level_events()?;
        Ok((shrink(stack), CLICK))
    }

    // ---- bottles, glowstone, shears, brush, honeycomb ----

    pub(in crate::game::server_side) fn dispense_glass_bottle(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let state = self.block_at(target);
        let name = self.dispenser_block_name(target);
        let honey = self
            .common
            .prop(state, "honey_level")
            .and_then(|v| v.parse::<i32>().ok());
        if self.in_block_tag("minecraft:beehives", &name)
            && let Some(level) = honey
            && level >= 5
        {
            // releaseBeesAndResetHoneyLevel.
            if let Some(reset) = self.common.with_props(state, &[("honey_level", "0")]) {
                self.level_set([target.0, target.1, target.2], reset)?;
            }
            self.dispenser_empty_hive(target, state);
            let bottle = ItemStack::new("minecraft:honey_bottle", 1);
            self.post_block_game_event("minecraft:fluid_pickup", [pos.0, pos.1, pos.2], None, None);
            return Ok((
                self.consume_with_remainder(pos, facing, stack, bottle),
                CLICK,
            ));
        }
        if self.water_at(target) {
            let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
            self.post_block_game_event("minecraft:fluid_pickup", [pos.0, pos.1, pos.2], None, None);
            return Ok((
                self.consume_with_remainder(pos, facing, stack, water),
                CLICK,
            ));
        }
        Ok((self.default_execute(pos, facing, stack), FAIL))
    }
    pub(in crate::game::server_side) fn dispense_shears(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let sheared = self.shear_beehive(target, &stack)? || self.shear_entity(target, &stack)?;
        if sheared {
            return Ok((self.hurt_and_break(stack, 1), CLICK));
        }
        Ok((Some(stack), FAIL))
    }
    /// The bees of a hive a dispenser harvested come out (`BEE_RELEASED`:
    /// not at night nor in the rain), through the front of its old state.
    pub(in crate::game) fn dispenser_empty_hive(&mut self, target: Pos, old: BlockStateId) {
        let facing = self.common.prop(old, "facing");
        self.empty_hive(
            [target.0, target.1, target.2],
            sunrise_sim::beehive::facing_step(facing.as_deref()),
            sunrise_sim::beehive::Release::BeeReleased,
        );
    }
    /// `LootTable.getRandomItems` of `table` in the context of `parent`: the
    /// draws come from `parent`'s server random sequence when it names one (a
    /// nested `loot_table` entry shares its parent's context), else the level's.
    pub(in crate::game) fn sequence_loot(
        &mut self,
        parent: &str,
        table: &str,
        held: Option<&ItemStack>,
    ) -> Vec<ItemStack> {
        match server_world_items!(self).loot_sequence(parent) {
            Some(sequence) => {
                let random = self
                    .server
                    .rand_sequences
                    .get(&sequence, self.common.level.seed);
                server_world_items!(self).loot_table_from(table, false, held, random)
            }
            None => server_world_items!(self).loot_table(
                table,
                false,
                held,
                &mut self.common.world.random,
            ),
        }
    }
    /// `Entity.spawnAtLocation(level, stack, offset)`: an item at the entity's
    /// feet plus the offset, moving as `new ItemEntity` makes it.
    pub fn spawn_at_location(&mut self, at: Vec3, y_offset: f64, stack: ItemStack) -> [f64; 3] {
        self.spawn_item_own(stack, [at.x, at.y + y_offset, at.z], 10)
    }
    /// `MushroomCow.shear`: `player` is who sheared it, none for a dispenser.
    pub(in crate::game) fn shear_mooshroom(
        &mut self,
        index: usize,
        player: Option<u128>,
        tool: &ItemStack,
    ) {
        let old = self.common.mobs[index].clone();
        let at = old.position;
        let height = f64::from(old.height());
        let table = if old.extra.variant == 1 {
            "minecraft:shearing/mooshroom/brown"
        } else {
            "minecraft:shearing/mooshroom/red"
        };
        // shear: level.playSound(null, this, MOOSHROOM_SHEAR, source, 1, 1),
        // PLAYERS for a player's shears and BLOCKS for a dispenser's.
        let source = if player.is_some() {
            SoundSource::Players
        } else {
            SoundSource::Blocks
        };
        let shear = LevelSound::new(
            "minecraft:entity.mooshroom.shear",
            [at.x, at.y, at.z],
            1.0,
            1.0,
        )
        .in_source(source);
        self.play_bound_sound(shear, old.id, None);
        self.convert_to(
            &old,
            "minecraft:cow",
            sunrise_sim::mobs::conversion::Params {
                keep_equipment: false,
                preserve_can_pick_up_loot: false,
            },
            false,
            |game, _| {
                game.mooshroom_shear_puff([at.x, at.y, at.z], height);
                let mut drops =
                    game.sequence_loot("minecraft:shearing/mooshroom", table, Some(tool));
                game.native_use_drops(
                    table,
                    [at.x, at.y, at.z],
                    None,
                    Some((old.id, "minecraft:mooshroom")),
                    player,
                    Some(tool),
                    &mut drops,
                );
                for stack in drops {
                    for _ in 0..stack.count {
                        game.spawn_item_own(
                            ItemStack {
                                count: 1,
                                ..stack.clone()
                            },
                            [at.x, at.y + height, at.z],
                            0,
                        );
                    }
                }
            },
        );
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use sunrise_sim::mobs::Mob;

    #[cfg(test)]
    #[cfg(any())]
    #[test]
    #[ignore = "daily"]
    fn a_dispenser_keeps_the_egg_if_its_type_cannot_spawn_in_peaceful() {
        use {
            sunrise_gameplay_common::game::execution_role::ExecutionRole,
            sunrise_gameplay_common::game::server_side::commands::peaceful_tests,
        };

        peaceful_tests::with_types(ExecutionRole::IntegratedServer, |game| {
            let egg = peaceful_tests::trader_egg();
            let dispense = |game: &mut crate::game::ServerGame<'_>| {
                game.dispense_spawn_egg((4, 64, 4), Direction::Up, egg.clone())
                    .unwrap()
                    .0
            };
            assert_eq!(dispense(game).unwrap().count, 1);
            assert!(game.mobs.is_empty());
            game.set_difficulty_forced(2);
            assert!(dispense(game).is_none());
            assert_eq!(game.mobs.len(), 1);
            assert_eq!(game.mobs[0].kind.id, peaceful_tests::TRADER);
        });
    }

    /// One scene per dispense, three blocks apart: a dispenser at (5, 64, 2 + 3 row) facing east,
    /// and in front a zombie that picks up loot (id 10 + row) and an armour stand (id 20 + row),
    /// the zombie first. Dispenses `stack` and answers whether the dispenser equipped it, the
    /// zombie's slot that took it, whether the stand took it, and where the two stand.
    pub fn dress(
        game: &mut crate::game::ServerGame<'_>,
        row: u32,
        stack: ItemStack,
    ) -> (bool, Option<Slot>, bool, [f64; 3]) {
        use sunrise_core::geometry::Vec3;
        (game.world.chunks).insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let z = 2 + 3 * row as i32;
        let at = Vec3::new(6.5, 64.0, f64::from(z) + 0.5);
        let mut zombie = sunrise_sim::mobs::Mob::new(10 + row, kind, at, 0.0);
        zombie.extra.pick_up_loot = true;
        game.mobs.push(zombie);
        game.armor_stands
            .push(sunrise_sim::entities::ArmorStand::new(20 + row, at, 0.0));
        let mut stack = stack;
        let done = game.dispense_equipment((5, 64, z), Direction::East, &mut stack);
        let zombie = game.mobs.iter().find(|m| m.id == 10 + row).unwrap();
        let worn = [Slot::Head, Slot::Chest, Slot::Legs, Slot::Feet]
            .into_iter()
            .find(|slot| mob_slot_item(zombie, *slot).is_some());
        let stand = game.armor_stands.iter().find(|s| s.id == 20 + row).unwrap();
        (
            done,
            worn,
            stand.equipment.iter().any(Option::is_some),
            [at.x, at.y, at.z],
        )
    }

    #[allow(unused_imports)]
    use std::sync::{Arc, atomic::AtomicI32, mpsc};
    #[allow(unused_imports)]
    use sunrise_protocol::packets::configuration::ClientInformation;
    #[allow(unused_imports)]
    use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};

    pub fn dispense_at_guest(game: &mut crate::game::ServerGame<'_>, item: &str) -> (bool, u32) {
        let mut stack = ItemStack::new(item, 2);
        // The dispenser sits one block over the guest's feet, facing down.
        let done = game.dispense_equipment((8, 201, 8), Direction::Down, &mut stack);
        (done, stack.count)
    }

    #[cfg(test)]
    /// `SectionPos.asLong` packs z (22 bits) over y (20 bits) under a fixed x,
    /// and the section set is sorted as longs: z -1 packs as 0x3FFFFF << 20,
    /// above z 0's 0; y -1 at z 0 packs as 0xFFFFF, below z 1's 1 << 20.
    #[test]
    fn sections_sort_as_their_packed_keys() {
        use sunrise_core::geometry::Vec3;
        let at = |x: f64, y: f64, z: f64| super::section_order(Vec3::new(x, y, z));
        assert!(at(0.0, 0.0, -1.0) > at(0.0, 0.0, 0.0));
        assert!(at(0.0, -1.0, 0.0) < at(0.0, 0.0, 16.0));
        assert!(at(0.0, -1.0, 0.0) > at(0.0, 0.0, 0.0));
        // x is the outer loop, ascending as a signed coordinate.
        assert!(at(-1.0, 0.0, 0.0) < at(0.0, 0.0, 0.0));
        assert!(at(15.9, 0.0, 0.0) < at(16.0, 0.0, 0.0));
    }

    #[cfg(test)]
    #[test]
    fn shulker_box_dispenser_builds_directional_place_context() {
        use sunrise_core::{geometry::Vec3, position::BlockPos};
        for facing in Direction::ALL {
            let over_air = crate::game::ServerGame::shulker_box_place_context(
                (10, 64, 5),
                facing,
                true,
                "minecraft:shulker_box".into(),
            );
            assert_eq!(over_air.clicked_pos, BlockPos::new(10, 64, 5));
            assert_eq!(over_air.face, facing);
            assert_eq!(over_air.hit, Vec3::new(10.5, 64.0, 5.5));
            assert!(!over_air.inside);
            assert_eq!(
                over_air.source,
                super::PlacementSource::Directional { direction: facing }
            );
            assert_eq!(over_air.item, "minecraft:shulker_box");
            assert!(!over_air.can_use_game_master_blocks);

            let over_solid = crate::game::ServerGame::shulker_box_place_context(
                (10, 64, 5),
                facing,
                false,
                "minecraft:shulker_box".into(),
            );
            assert_eq!(over_solid.face, Direction::Up);
            assert_eq!(
                over_solid.source,
                super::PlacementSource::Directional { direction: facing }
            );
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// The brush behaviour: the first armadillo in front that `brushOffScute`s.
    pub(in crate::game::server_side) fn dispense_brush(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Dispensed {
        let target = offset(pos, facing);
        let armadillos: Vec<usize> = self
            .targets_at(target)
            .into_iter()
            .filter_map(|t| match t {
                Target::Mob(i) if self.common.mobs[i].species() == Species::Armadillo => Some(i),
                _ => None,
            })
            .collect();
        for i in armadillos {
            if self.common.mobs[i].is_baby() {
                continue;
            }
            let at = self.common.mobs[i].position;
            let drops = self.sequence_loot(
                "minecraft:brush/armadillo",
                "minecraft:brush/armadillo",
                Some(&stack),
            );
            for d in drops {
                self.spawn_at_location(at, 0.0, d);
            }
            self.server_play_sound(
                "minecraft:entity.armadillo.brush",
                [at.x, at.y, at.z],
                1.0,
                1.0,
            );
            let who = sunrise_sim::mobs::Who::Mob(self.common.mobs[i].id);
            let ctx = self.who_context(Some(who), None);
            self.post_game_event("minecraft:entity_interact", [at.x, at.y, at.z], ctx);
            return (self.hurt_and_break(stack, 16), optional_event("brush"));
        }
        set_success("brush", false);
        (Some(stack), FAIL)
    }
    /// `EquipmentDispenseItemBehavior.dispenseEquipment`: the first living
    /// entity in front that `canEquipWithDispenser` gets one of the item.
    pub fn dispense_equipment(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: &mut ItemStack,
    ) -> bool {
        // The stack's own `equippable`: its slot and allowed entities, and `dispensable`.
        let Some(equippable) = equippable(self.items(), stack) else {
            return false;
        };
        let default = (self.items().get(&stack.item)).is_some_and(|info| info.equip_dispensable);
        let dispensable = sunrise_sim::item_sync::dispensable(stack, default);
        let Some(slot) = Slot::from_name(equippable.slot).filter(|_| dispensable) else {
            return false;
        };
        // Equippable.equipSound defaults to the generic armour sound.
        let sound = (self.items().get(&stack.item))
            .and_then(|info| info.equip_sound.clone())
            .or_else(|| Some("minecraft:item.armor.equip_generic".into()));
        let front = offset(pos, facing);
        let one = ItemStack {
            count: 1,
            ..stack.clone()
        };
        // `Equippable.canBeEquippedBy`, then the item's parts (`canEquip`) with the stack converted
        // once, for the entity in front.
        let targets = self.targets_at(front);
        let prepared = self.common.equip_stack(stack);
        let allows = |entity: u32, id: &str| {
            let data = equippable.allows(id);
            self.can_equip(&prepared, equippable.slot, entity, id, data)
        };
        // LivingEntity.canEquipWithDispenser: alive, the slot usable, the
        // item's allowed entities, the slot empty and the dispenser rule.
        let target = targets.into_iter().find(|t| match *t {
            // Player.canUseSlot: every slot; the armour and the off hand are
            // the ones an equippable item can reach.
            Target::Player => {
                allows(self.own_entity_id() as u32, "minecraft:player")
                    && match slot {
                        Slot::OffHand => self.common.offhand.is_none(),
                        _ => slot
                            .armor_index()
                            .is_some_and(|i| self.common.armor[i].is_none()),
                    }
            }
            Target::Guest(conn) => self
                .server
                .lan
                .as_ref()
                .and_then(|h| h.hub.guest(conn))
                .is_some_and(|g| {
                    allows(g.entity_id as u32, "minecraft:player")
                        && match slot {
                            Slot::OffHand => g.offhand.is_none(),
                            _ => slot.armor_index().is_some_and(|i| g.armor[i].is_none()),
                        }
                }),
            // ArmorStand.canUseSlot: not a locked slot, and a hand needs arms.
            Target::Stand(i) => slot.stand_index().is_some_and(|s| {
                let stand = &self.common.armor_stands[i];
                allows(stand.id, "minecraft:armor_stand")
                    && !stand.is_disabled(s)
                    && stand.equipment[s].is_none()
            }),
            Target::Mob(i) => {
                let m = &self.common.mobs[i];
                m.alive()
                    && mob_can_use_slot(m, slot)
                    && allows(m.id, &m.kind.id)
                    && mob_slot_item(m, slot).is_none()
                    && mob_can_dispenser_equip(m, slot)
            }
        });
        let Some(target) = target else {
            return false;
        };
        stack.count -= 1;
        // setItemSlot: onEquipItem plays the equip sound and the equip event.
        let (at, sound) = match target {
            Target::Guest(conn) => {
                // The guest's own turn: their state in place, so the slot, the
                // sound, the game event and the equipment sync run as for the host.
                let done = self
                    .for_player(super::commands::Target::Guest(conn), |g| {
                        g.put_on_player(slot, &one)
                    })
                    .is_some();
                if !done {
                    stack.count += 1;
                    return false;
                }
                self.native_block_dispense_armor(
                    self.common.dim.id(),
                    [pos.0, pos.1, pos.2],
                    &stack.item,
                    "player",
                );
                return true;
            }
            Target::Player => {
                self.put_on_player(slot, &one);
                self.native_block_dispense_armor(
                    self.common.dim.id(),
                    [pos.0, pos.1, pos.2],
                    &stack.item,
                    "player",
                );
                return true;
            }
            Target::Stand(i) => {
                let stand = &mut self.common.armor_stands[i];
                if let Some(s) = slot.stand_index() {
                    stand.equipment[s] = Some(one);
                }
                (Some(stand.position), sound)
            }
            Target::Mob(i) => {
                let m = &mut self.common.mobs[i];
                mob_set_slot(m, slot, one);
                // setGuaranteedDrop / setPersistenceRequired.
                m.persistent = true;
                let sound = if m.silent {
                    None
                } else {
                    equip_sound(m, slot, sound)
                };
                (Some(m.position), sound)
            }
        };
        if let Some(at) = at {
            if let Some(sound) = sound {
                self.server_play_sound(&sound, [at.x, at.y, at.z], 1.0, 1.0);
            }
            let who = match target {
                Target::Mob(i) => Some(sunrise_sim::mobs::Who::Mob(self.common.mobs[i].id)),
                Target::Player | Target::Guest(_) | Target::Stand(_) => None,
            };
            let ctx = self.who_context(who, None);
            self.post_game_event("minecraft:equip", [at.x, at.y, at.z], ctx);
        }
        self.native_block_dispense_armor(
            self.common.dim.id(),
            [pos.0, pos.1, pos.2],
            &stack.item,
            match target {
                Target::Stand(_) => "armor_stand",
                Target::Mob(_) => "mob",
                Target::Player | Target::Guest(_) => "player",
            },
        );
        true
    }
    pub(in crate::game::server_side) fn dispense_glowstone(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let state = self.block_at(target);
        if &*self.dispenser_block_name(target) == "minecraft:respawn_anchor" {
            let charge = self
                .common
                .prop(state, "charges")
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(0);
            if charge != 4 {
                if let Some(next) = self
                    .common
                    .with_props(state, &[("charges", &(charge + 1).to_string())])
                {
                    self.level_set([target.0, target.1, target.2], next)?;
                    // RespawnAnchorBlock.charge: Context.of(null, newState).
                    self.post_block_game_event(
                        "minecraft:block_change",
                        [target.0, target.1, target.2],
                        None,
                        Some(next),
                    );
                }
                self.server_play_sound(
                    "minecraft:block.respawn_anchor.charge",
                    Self::center(target),
                    1.0,
                    1.0,
                );
                return Ok((shrink(stack), CLICK));
            }
            return Ok((Some(stack), FAIL));
        }
        Ok((self.default_execute(pos, facing, stack), CLICK))
    }
    pub(in crate::game::server_side) fn dispense_honeycomb(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let state = self.block_at(target);
        // HoneycombItem.getWaxed, as the player's honeycomb waxes.
        let Some(waxed) = self.common.honeycomb_waxed(state) else {
            return Ok((
                self.default_execute(pos, facing, stack),
                optional_event("honeycomb"),
            ));
        };
        set_success("honeycomb", true);
        self.level_set([target.0, target.1, target.2], waxed)?;
        self.common
            .level_events
            .push(sunrise_world::level::LevelEvent::LevelEvent {
                id: 3003,
                pos: Self::pos_of(target),
                data: 0,
                source: None,
            });
        self.server_process_level_events()?;
        self.server_play_sound(
            "minecraft:item.honeycomb.wax_on",
            Self::center(target),
            1.0,
            1.0,
        );
        Ok((shrink(stack), CLICK))
    }
    pub(in crate::game::server_side) fn dispense_potion(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Result<Dispensed> {
        let target = offset(pos, facing);
        let water = stack.potion() == Some("minecraft:water");
        let name = self.dispenser_block_name(target);
        if !water || !self.in_block_tag("minecraft:convertible_to_mud", &name) {
            // The inner default behaviour dispenses, then the potion's own
            // events play on top.
            return Ok((self.dispense_default_in_level(pos, facing, stack), CLICK));
        }
        // The server picks the splash positions with the level random.
        for _ in 0..5 {
            let r = &mut self.common.world.random;
            let p = [
                f64::from(pos.0) + r.next_double(),
                f64::from(pos.1 + 1),
                f64::from(pos.2) + r.next_double(),
            ];
            self.server_particle_at(
                sunrise_sim::lan::server_effects::ServerParticle::simple(ParticleType::Splash),
                p,
                [0.0; 3],
            );
        }
        self.server_play_sound("minecraft:item.bottle.empty", Self::center(pos), 1.0, 1.0);
        self.post_block_game_event("minecraft:fluid_place", [pos.0, pos.1, pos.2], None, None);
        if let Some(mud) = self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:mud")
        {
            self.level_set([target.0, target.1, target.2], mud)?;
        }
        let bottle = ItemStack::new("minecraft:glass_bottle", 1);
        Ok((
            self.consume_with_remainder(pos, facing, stack, bottle),
            CLICK,
        ))
    }
    pub(in crate::game::server_side) fn dispense_tnt(
        &mut self,
        pos: Pos,
        facing: Direction,
        stack: ItemStack,
    ) -> Dispensed {
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::TNT_EXPLODES)
        {
            set_success("tnt", false);
            return (Some(stack), FAIL);
        }
        let front = offset(pos, facing);
        if self.sulfur_cube_swallow(front, &stack) {
            return (shrink(stack), optional_event("tnt"));
        }
        let target = Self::pos_of(front);
        if !self.tnt_prime_allowed(target) {
            set_success("tnt", false);
            return (Some(stack), FAIL);
        }
        set_success("tnt", true);
        // new PrimedTnt(level, x + 0.5, y, z + 0.5, null), its sound and ENTITY_PLACE.
        let rot = self.common.world.random.next_double() * f64::from(std::f32::consts::TAU);
        self.add_primed_tnt(target, rot, 80, None, None);
        self.server_play_sound(
            "minecraft:entity.tnt.primed",
            [
                f64::from(target.x) + 0.5,
                f64::from(target.y),
                f64::from(target.z) + 0.5,
            ],
            1.0,
            1.0,
        );
        self.post_block_game_event(
            "minecraft:entity_place",
            [front.0, front.1, front.2],
            None,
            None,
        );
        (shrink(stack), CLICK)
    }

    pub(in crate::game::server_side) fn shear_beehive(
        &mut self,
        target: Pos,
        tool: &ItemStack,
    ) -> Result<bool> {
        let state = self.block_at(target);
        let name = self.dispenser_block_name(target);
        let level = self
            .common
            .prop(state, "honey_level")
            .and_then(|v| v.parse::<i32>().ok());
        if !self.in_block_tag("minecraft:beehives", &name) || level.is_none_or(|l| l < 5) {
            return Ok(false);
        }
        self.server_play_sound(
            "minecraft:block.beehive.shear",
            Self::center(target),
            1.0,
            1.0,
        );
        // BeehiveBlock.dropHoneycomb (harvest/beehive), then
        // releaseBeesAndResetHoneyLevel and the shear event.
        let p = [target.0, target.1, target.2];
        for stack in self.interact_loot_at("minecraft:harvest/beehive", p, state, None, Some(tool))
        {
            self.pop_resource(p, stack);
        }
        if let Some(reset) = self.common.with_props(state, &[("honey_level", "0")]) {
            self.level_set(p, reset)?;
        }
        self.dispenser_empty_hive(target, state);
        self.post_block_game_event("minecraft:shear", p, None, None);
        Ok(true)
    }
    /// `ShearsDispenseItemBehavior.tryShearEntity`: the first live, ready
    /// `Shearable` in the block is sheared (an entity part's `IShearable` answer comes first).
    pub fn shear_entity(&mut self, target: Pos, shears: &ItemStack) -> Result<bool> {
        for t in self.targets_at(target) {
            // Entity.shearOffAllLeashConnections comes first, for any entity.
            if self.shear_off_leashes(t) {
                return Ok(true);
            }
            let Target::Mob(i) = t else { continue };
            let part = self.part_shearable(i, None, shears);
            let m = &self.common.mobs[i];
            if !m.alive() || part.is_some_and(|(_, shearable)| !shearable) {
                continue;
            }
            let (at, entity_id, kind) = (m.position, m.id, m.kind.id.clone());
            // shear(level, BLOCKS, tool): level.playSound(null, this, sound, BLOCKS, 1, 1).
            let shear_sound = |name: &'static str| {
                LevelSound::new(name, [at.x, at.y, at.z], 1.0, 1.0).in_source(SoundSource::Blocks)
            };
            match m.species() {
                _ if let Some((part, _)) = part => {
                    if self.part_shear(i, part, None, shears).is_err() {
                        continue;
                    }
                }
                Species::SulfurCube if sulfur_cube::has_body(m) => {
                    // SulfurCube.shear: the swallowed block comes out at the
                    // passenger attachment, and nothing is swallowed for 5 s.
                    let height = f64::from(m.height());
                    let id = m.id;
                    if let Some(body) = self.common.mobs[i].take_body_equipment() {
                        self.spawn_at_location(at, height, body);
                    }
                    // setItemSlot(BODY, EMPTY): an unequip.
                    let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Mob(id)), None);
                    self.post_game_event("minecraft:unequip", [at.x, at.y, at.z], ctx);
                    self.common.mobs[i].extra.ints[3] = sulfur_cube::PICKUP_TIMER;
                    self.server_play_sound(
                        "minecraft:entity.sulfur_cube.eject",
                        [at.x, at.y, at.z],
                        1.0,
                        1.0,
                    );
                }
                Species::Sheep if !m.extra.sheared && !m.is_baby() => {
                    let color = sunrise_level::dyes::DYE_NAMES[m.extra.color as usize & 15];
                    self.play_bound_sound(
                        shear_sound("minecraft:entity.sheep.shear"),
                        entity_id,
                        None,
                    );
                    let drops = self.sequence_loot(
                        "minecraft:shearing/sheep",
                        &format!("minecraft:shearing/sheep/{color}"),
                        Some(shears),
                    );
                    for stack in drops {
                        for _ in 0..stack.count {
                            let one = ItemStack {
                                count: 1,
                                ..stack.clone()
                            };
                            self.spawn_at_location(at, 1.0, one);
                            // Sheep.shear nudges each item with the sheep's own random.
                            let r = &mut self.common.mobs[i].random;
                            let push = [
                                f64::from((r.next_float() - r.next_float()) * 0.1),
                                f64::from(r.next_float() * 0.05),
                                f64::from((r.next_float() - r.next_float()) * 0.1),
                            ];
                            if let Some(item) = self.just_spawned_item() {
                                item.velocity = Vec3::new(
                                    item.velocity.x + push[0],
                                    item.velocity.y + push[1],
                                    item.velocity.z + push[2],
                                );
                            }
                        }
                    }
                    self.common.mobs[i].extra.sheared = true;
                }
                Species::Bogged if !m.extra.sheared => {
                    let height = f64::from(m.height());
                    self.play_bound_sound(
                        shear_sound("minecraft:entity.bogged.shear"),
                        entity_id,
                        None,
                    );
                    let drops = self.sequence_loot(
                        "minecraft:shearing/bogged",
                        "minecraft:shearing/bogged",
                        Some(shears),
                    );
                    for stack in drops {
                        self.spawn_at_location(at, height, stack);
                    }
                    self.common.mobs[i].extra.sheared = true;
                }
                Species::SnowGolem if !m.extra.sheared => {
                    let eye = m.eye_height();
                    self.play_bound_sound(
                        shear_sound("minecraft:entity.snow_golem.shear"),
                        entity_id,
                        None,
                    );
                    self.common.mobs[i].extra.sheared = true;
                    let drops = self.sequence_loot(
                        "minecraft:shearing/snow_golem",
                        "minecraft:shearing/snow_golem",
                        Some(shears),
                    );
                    for stack in drops {
                        self.spawn_at_location(at, eye, stack);
                    }
                }
                // CopperGolem.shear: the antenna's flower drops 1.5 up.
                Species::CopperGolem if self.common.golem_shearable(i) => {
                    self.play_bound_sound(
                        shear_sound("minecraft:entity.copper_golem.shear"),
                        entity_id,
                        None,
                    );
                    let antenna = self.common.mobs[i].saddle_item();
                    let id = self.common.mobs[i].id;
                    // setItemSlot(EQUIPMENT_SLOT_ANTENNA, EMPTY): an unequip.
                    self.common.mobs[i].set_saddle(None);
                    let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Mob(id)), None);
                    self.post_game_event("minecraft:unequip", [at.x, at.y, at.z], ctx);
                    if let Some(antenna) = antenna {
                        self.spawn_at_location(at, 1.5, antenna);
                    }
                }
                Species::Mooshroom if !m.is_baby() => {
                    self.shear_mooshroom(i, None, shears);
                }
                _ => continue,
            }
            self.native_block_shear_entity(
                self.common.dim.id(),
                [target.0, target.1, target.2],
                entity_id,
                &kind,
            );
            self.post_block_game_event(
                "minecraft:shear",
                [target.0, target.1, target.2],
                None,
                None,
            );
            return Ok(true);
        }
        Ok(false)
    }
    /// `Entity.shearOffAllLeashConnections(null)`: the entity's own lead and
    /// every lead held by it (within a 32 block box around its centre) drop;
    /// a shear game event at the entity and a snip at its block follow.
    pub(in crate::game) fn shear_off_leashes(&mut self, t: Target) -> bool {
        use sunrise_sim::mobs::{Who, leash::Holder};
        let (holder, centre, own) = match t {
            Target::Mob(i) => {
                let m = &self.common.mobs[i];
                let b = m.bounds();
                (Holder::Mob(m.id), b.center(), Some(i))
            }
            Target::Player => {
                let Some(v) = self.player_view() else {
                    return false;
                };
                (Holder::Player, v.bounds.center(), None)
            }
            Target::Guest(_) | Target::Stand(_) => return false,
        };
        let area = Aabb::new(centre, centre).inflate(Vec3::new(16.0, 16.0, 16.0));
        let mut leashed: Vec<(u32, usize)> = self
            .common
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                !m.removed && m.extra.leash == Some(holder) && overlaps(m.bounds(), area)
            })
            .map(|(j, m)| (m.id, j))
            .collect();
        leashed.sort_unstable();
        let own_leashed = own.is_some_and(|i| self.common.mobs[i].extra.leash.is_some());
        if leashed.is_empty() && !own_leashed {
            return false;
        }
        // Leashable.dropLeash: a lead item at the leashed entity's feet.
        let drop = |game: &mut Self, j: usize| {
            if sunrise_sim::mobs::leash::remove(&mut game.mobs[j]).is_some() {
                let at = game.mobs[j].position;
                game.spawn_at_location(at, 0.0, ItemStack::new("minecraft:lead", 1));
            }
        };
        if let Some(i) = own.filter(|_| own_leashed) {
            drop(self, i);
        }
        for (_, j) in leashed {
            drop(self, j);
        }
        let (at, who) = match t {
            Target::Mob(i) => {
                let m = &self.common.mobs[i];
                (m.position, Who::Mob(m.id))
            }
            _ => (
                self.common
                    .player
                    .as_ref()
                    .map_or(Vec3::ZERO, |p| p.position),
                Who::Player,
            ),
        };
        let ctx = self.who_context(Some(who), None);
        self.post_game_event("minecraft:shear", [at.x, at.y, at.z], ctx);
        let block = [at.x.floor() + 0.5, at.y.floor() + 0.5, at.z.floor() + 0.5];
        self.server_play_sound("minecraft:item.shears.snip", block, 1.0, 1.0);
        true
    }
    /// `SulfurCubeBlockDispenseItemBehavior.dispenseBlock`: the first sulfur
    /// cube in the block that `equipItem`s the stack swallows one.
    pub fn sulfur_cube_swallow(&mut self, target: Pos, stack: &ItemStack) -> bool {
        let cubes: Vec<usize> = self
            .targets_at(target)
            .into_iter()
            .filter_map(|t| match t {
                Target::Mob(i) if self.common.mobs[i].species() == Species::SulfurCube => Some(i),
                _ => None,
            })
            .collect();
        for i in cubes {
            let m = &self.common.mobs[i];
            if m.is_baby() || m.extra.body_id() == Some(&*stack.item) {
                continue;
            }
            let (at, height) = (m.position, f64::from(m.height()));
            // The swallowed block comes out on top (the passenger attachment).
            if let Some(old) = self.common.mobs[i].take_body_equipment() {
                self.spawn_at_location(at, height, old);
            }
            let m = &mut self.common.mobs[i];
            m.extra.set_body_stack(Some(ItemStack {
                count: 1,
                ..stack.clone()
            }));
            m.persistent = true;
            let (at, id) = (m.position, m.id);
            // setItemSlot: a body item without `equippable` posts UNEQUIP.
            let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Mob(id)), None);
            self.post_game_event("minecraft:unequip", [at.x, at.y, at.z], ctx);
            self.server_play_sound(
                "minecraft:entity.sulfur_cube.absorb",
                [at.x, at.y, at.z],
                1.0,
                1.0,
            );
            return true;
        }
        false
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::cart_rails::CartRailsGameExt1 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::equip_answers::EquipAnswersGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_abilities::ItemAbilitiesGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::player_equip::PlayerEquipGameExt2 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
