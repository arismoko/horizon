//! Mob and player equipment-change dispatch and server mob actions.
use crate::game::ServerGame;
use mod_api::{EquipmentSlot, HookId, ItemStackView, LivingRef, WorldEpoch};
use sunrise_gameplay_common::game::api_values::{
    StackReadMetadata, project_stack, stack_read_metadata,
};
use sunrise_items::equipment::{BODY, Equipment, SADDLE};
use sunrise_items::items::ItemStack;
use sunrise_sim::lan::guest::Guest;

/// `LivingEntity.equipmentHasChanged` negated (`ItemStack.matches`).
pub(in crate::game) fn equipment_stack_matches(
    now: Option<&ItemStack>,
    prev: Option<&ItemStack>,
) -> bool {
    match (now, prev) {
        (None, None) => true,
        (Some(now), Some(prev)) => now.matches(prev),
        _ => false,
    }
}

pub(in crate::game) fn guest_equipment_slot(g: &Guest, slot: EquipmentSlot) -> Option<&ItemStack> {
    match slot {
        EquipmentSlot::MainHand => g.inventory.get(g.selected)?.as_ref(),
        EquipmentSlot::OffHand => g.offhand.as_ref(),
        EquipmentSlot::Feet => g.armor[3].as_ref(),
        EquipmentSlot::Legs => g.armor[2].as_ref(),
        EquipmentSlot::Chest => g.armor[1].as_ref(),
        EquipmentSlot::Head => g.armor[0].as_ref(),
        EquipmentSlot::Body => g.equipment[BODY].as_ref(),
        EquipmentSlot::Saddle => g.equipment[SADDLE].as_ref(),
    }
}

pub(in crate::game) fn guest_equipment_changed(g: &Guest, last: Option<&Equipment>) -> bool {
    EquipmentSlot::ALL.into_iter().any(|slot| {
        let current = guest_equipment_slot(g, slot);
        let prev = last.and_then(|last| last[slot as usize].as_ref());
        !equipment_stack_matches(current, prev)
    })
}

/// Retain only the slots needed for vanilla's `lastEquipmentItems` comparison,
/// freeing the boxed table once every slot is empty again.
pub(in crate::game) fn store_equipment_history(
    history: &mut Option<Box<Equipment>>,
    index: usize,
    to: Option<ItemStack>,
) {
    match to {
        Some(stack) => {
            history.get_or_insert_with(Default::default)[index] = Some(stack);
        }
        None => {
            if let Some(slots) = history.as_deref_mut() {
                slots[index] = None;
                if slots.is_empty() {
                    *history = None;
                }
            }
        }
    }
}

fn equipment_view_metadata(stack: Option<&ItemStack>) -> Option<StackReadMetadata> {
    let stack = stack.filter(|s| s.count > 0 && &*s.item != "minecraft:air")?;
    match stack_read_metadata(stack) {
        Ok(metadata) => Some(metadata),
        Err(error) => {
            log::warn!(
                "living_equipment_change: omitting unrepresentable stack '{}' (count {}): {error:?}",
                stack.item,
                stack.count,
            );
            None
        }
    }
}

impl ServerGame<'_> {
    /// The active server epoch when an enabled mod subscribes to equipment changes.
    pub(in crate::game) fn living_equipment_notify_epoch(&self) -> Option<WorldEpoch> {
        if !self.common.is_logical_server()
            || !mod_api::__private::enabled_listened(HookId::LivingEquipmentChange)
        {
            return None;
        }
        self.common.native_lifecycle.epoch()
    }

    /// Lend `from` and `to` lazily to `living_equipment_change` without eagerly
    /// cloning stacks or encoding component patches.
    pub(in crate::game) fn dispatch_living_equipment_change(
        &mut self,
        entity: LivingRef,
        slot: EquipmentSlot,
        from: Option<&ItemStack>,
        to: Option<&ItemStack>,
    ) {
        let from_metadata = equipment_view_metadata(from);
        let to_metadata = equipment_view_metadata(to);
        let project_from = |metadata| from.and_then(|stack| project_stack(stack, metadata).ok());
        let project_to = |metadata| to.and_then(|stack| project_stack(stack, metadata).ok());
        let from_view =
            from_metadata.map(|metadata| ItemStackView::resident(metadata, &project_from));
        let to_view = to_metadata.map(|metadata| ItemStackView::resident(metadata, &project_to));
        self.native_living_equipment_change(mod_api::__private::LivingEquipmentChangeCall {
            entity,
            slot,
            from: from_view,
            to: to_view,
        });
    }
}

fn finite(v: [f64; 3]) -> bool {
    v.into_iter().all(f64::is_finite)
}

/// `/summon`'s world bounds (`Level.isInSpawnableBounds`).
pub(in crate::game) fn spawnable(pos: [f64; 3]) -> bool {
    finite(pos)
        && (-30_000_000.0..30_000_000.0).contains(&pos[0])
        && (-30_000_000.0..30_000_000.0).contains(&pos[2])
        && (-20_000_000.0..20_000_000.0).contains(&pos[1].floor())
}
